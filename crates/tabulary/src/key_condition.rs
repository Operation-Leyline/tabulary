//! Schema-owned key conditions; partition equality is the only entry point.
use std::{fmt, marker::PhantomData};

use aws_sdk_dynamodb::types::AttributeValue;

use crate::condition::{ConditionNode, Operand, Operator, Ordered, Prefix};
use crate::{
    BuildError, BuiltCondition, Condition, Converter, DynamoType, EncodeContext, EncodeError,
    EncodeFault, KeyDefinition, Native, PathSegment, Record, RenderedExpressions, SortKey, Value,
    WireType,
};

/// Query-facing contract shared by primary and secondary schema markers.
pub trait KeySchema<R: Record> {
    type Partition;
    type PartitionCodec: Converter<Self::Partition>;
    type Sort;
    const KEYS: KeyDefinition;
}

/// A primary schema inherits its logical types and physical keys from `KeySchema`.
pub trait PrimarySchema<R: Record>: KeySchema<R> {}

/// A prefix encoder may accept a different logical input from a full sort key.
/// Generated composite codecs accept only their private complete-boundary carrier.
pub trait PrefixCodec<T>: Converter<T> {
    type Prefix;
    fn encode_prefix(
        value: &Self::Prefix,
        cx: &mut EncodeContext,
    ) -> Result<AttributeValue, EncodeError>;
}
impl<T: Value> PrefixCodec<T> for Native<T>
where
    T::Wire: Prefix,
{
    type Prefix = T;
    fn encode_prefix(value: &T, cx: &mut EncodeContext) -> Result<AttributeValue, EncodeError> {
        Self::encode(value, cx)
    }
}

/// The single sort slot has been consumed. There are no sort methods in this state.
pub struct SortComplete;

type Invariant<T> = PhantomData<fn(T) -> T>;

/// An owned condition that always includes partition equality, bound to one schema.
pub struct KeyCondition<R: Record, I: KeySchema<R>, S = <I as KeySchema<R>>::Sort> {
    condition: Condition<R>,
    marker: Invariant<(I, S)>,
}

/// A validated key condition. Debug hides all encoded values.
pub struct BuiltKeyCondition<R, I> {
    condition: BuiltCondition<R>,
    marker: PhantomData<fn(I) -> I>,
}
impl<R: Record, I: KeySchema<R>, S> KeyCondition<R, I, S> {
    pub fn build(self) -> Result<BuiltKeyCondition<R, I>, BuildError> {
        Ok(BuiltKeyCondition {
            condition: self.condition.build()?,
            marker: PhantomData,
        })
    }
}
impl<R, I> BuiltKeyCondition<R, I> {
    pub fn root(&self) -> &ConditionNode {
        self.condition.root()
    }
    pub fn render(&self) -> RenderedExpressions {
        let mut rendered = RenderedExpressions::default();
        rendered.key_condition = Some(crate::condition_renderer::render_condition(
            self.root(),
            &mut rendered.aliases,
        ));
        rendered
    }
}
impl<R: Record, I: KeySchema<R>, S> fmt::Debug for KeyCondition<R, I, S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("KeyCondition([unbuilt])")
    }
}
impl<R, I> fmt::Debug for BuiltKeyCondition<R, I> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.condition.fmt(f)
    }
}

fn predicate<R: Record, I: KeySchema<R>, T: 'static>(
    sort: bool,
    wire: DynamoType,
    operator: Operator,
    values: Result<Vec<T>, EncodeError>,
    encode: fn(&T, &mut EncodeContext) -> Result<AttributeValue, EncodeError>,
) -> Condition<R> {
    Condition::new(move || {
        let invalid = || BuildError::OperationConstraint {
            code: "invalid_key_schema",
        };
        let keys = I::KEYS;
        if keys != R::SCHEMA.primary && !R::SCHEMA.indexes.iter().any(|i| i.keys == keys) {
            return Err(invalid());
        }
        let position = if sort {
            keys.sort.ok_or_else(invalid)?
        } else {
            keys.partition
        };
        let field = *R::SCHEMA.fields.get(position).ok_or_else(invalid)?;
        if field.index != position
            || field.name.is_empty()
            || field.wire != wire
            || !matches!(wire, DynamoType::S | DynamoType::N | DynamoType::B)
            || keys.sort == Some(keys.partition)
        {
            return Err(invalid());
        }
        let mut cx =
            EncodeContext::new(R::SCHEMA.name).at(PathSegment::Attribute(field.name.into()));
        let mut operands = vec![Operand::Attribute(field)];
        for value in values? {
            let value = encode(&value, &mut cx)?;
            if !matches!(
                (wire, &value),
                (DynamoType::S, AttributeValue::S(_))
                    | (DynamoType::N, AttributeValue::N(_))
                    | (DynamoType::B, AttributeValue::B(_))
            ) {
                return Err(cx
                    .error(EncodeFault::Custom {
                        code: "key_codec_wire_mismatch",
                    })
                    .into());
            }
            crate::record::validate_key(&value, if sort { 1024 } else { 2048 })
                .map_err(|fault| cx.error(fault))?;
            operands.push(Operand::Value(value));
        }
        Ok(ConditionNode::Predicate { operator, operands })
    })
}

#[doc(hidden)]
pub fn partition<R: Record, I: KeySchema<R>, T: 'static, C: Converter<T>>(
    value: Result<T, EncodeError>,
) -> KeyCondition<R, I> {
    KeyCondition {
        condition: predicate::<R, I, T>(
            false,
            C::Wire::DYNAMO_TYPE,
            Operator::Eq,
            value.map(|v| vec![v]),
            C::encode,
        ),
        marker: PhantomData,
    }
}

impl<R: Record, I: KeySchema<R>, T: 'static, C: Converter<T>> KeyCondition<R, I, SortKey<T, C>> {
    fn sort(self, operator: Operator, values: Vec<T>) -> KeyCondition<R, I, SortComplete> {
        self.with_sort(predicate::<R, I, T>(
            true,
            C::Wire::DYNAMO_TYPE,
            operator,
            Ok(values),
            C::encode,
        ))
    }
    fn with_sort(self, condition: Condition<R>) -> KeyCondition<R, I, SortComplete> {
        KeyCondition {
            condition: self.condition.and(condition),
            marker: PhantomData,
        }
    }
    pub fn sort_eq(self, value: T) -> KeyCondition<R, I, SortComplete> {
        self.sort(Operator::Eq, vec![value])
    }
    pub fn sort_begins_with(self, value: C::Prefix) -> KeyCondition<R, I, SortComplete>
    where
        C: PrefixCodec<T>,
        C::Prefix: 'static,
        C::Wire: Prefix,
    {
        self.with_sort(predicate::<R, I, C::Prefix>(
            true,
            C::Wire::DYNAMO_TYPE,
            Operator::BeginsWith,
            Ok(vec![value]),
            C::encode_prefix,
        ))
    }
}
macro_rules! ordered {
    ($($method:ident => $op:ident),+) => {$(
        pub fn $method(self, value: T) -> KeyCondition<R, I, SortComplete> { self.sort(Operator::$op, vec![value]) }
    )+};
}
impl<R: Record, I: KeySchema<R>, T: 'static, C: Converter<T>> KeyCondition<R, I, SortKey<T, C>>
where
    C::Wire: Ordered,
{
    ordered!(sort_lt => Lt, sort_le => Le, sort_gt => Gt, sort_ge => Ge);
    pub fn sort_between(self, lo: T, hi: T) -> KeyCondition<R, I, SortComplete> {
        self.sort(Operator::Between, vec![lo, hi])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{__private as p, aliases::AliasAllocator, condition_renderer::render_condition};
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Row;
    impl Record for Row {
        const SCHEMA: crate::RecordSchema = p::schema(
            "Row",
            &[
                p::field::<Row, String, crate::Required, crate::KeyOnly, Native<String>>("pk", 0)
                    .descriptor(),
                p::field::<Row, String, crate::Required, crate::KeyOnly, Native<String>>("sk", 1)
                    .descriptor(),
            ],
            p::keys(0, Some(1)),
        );
    }
    struct Schema<const P: usize>;
    impl<const P: usize> KeySchema<Row> for Schema<P> {
        type Partition = String;
        type PartitionCodec = Upper;
        type Sort = SortKey<String, Upper>;
        const KEYS: KeyDefinition = p::keys(P, Some(1));
    }
    static CALLS: AtomicUsize = AtomicUsize::new(0);
    struct Upper;
    impl Converter<String> for Upper {
        type Wire = crate::wire::String;
        fn encode(value: &String, cx: &mut EncodeContext) -> Result<AttributeValue, EncodeError> {
            CALLS.fetch_add(1, Ordering::Relaxed);
            if value == "bad" {
                return Ok(AttributeValue::Bool(true));
            }
            Native::<String>::encode(&value.to_uppercase(), cx)
        }
        fn decode(
            value: &AttributeValue,
            cx: &mut crate::DecodeContext,
        ) -> Result<String, crate::DecodeError> {
            Native::<String>::decode(value, cx)
        }
    }
    #[test]
    fn manual_codecs_validate_once_and_share_the_request_allocator() {
        let condition = partition::<Row, Schema<0>, _, Upper>(Ok("a".into())).sort_eq("b".into());
        assert_eq!(CALLS.load(Ordering::Relaxed), 0);
        let built = condition.build().unwrap();
        assert_eq!(CALLS.load(Ordering::Relaxed), 2);
        let mut aliases = AliasAllocator::default();
        assert_eq!(aliases.name("sk"), "#n0");
        aliases.value(&AttributeValue::S("earlier".into()));
        assert_eq!(
            render_condition(built.root(), &mut aliases),
            "(#n1 = :v1 AND #n0 = :v2)"
        );
        assert_eq!(aliases.values[":v1"], AttributeValue::S("A".into()));
        assert_eq!(aliases.values[":v2"], AttributeValue::S("B".into()));
        assert_eq!(CALLS.load(Ordering::Relaxed), 2);
        assert!(matches!(
            partition::<Row, Schema<2>, _, Upper>(Ok("a".into())).build(),
            Err(BuildError::OperationConstraint {
                code: "invalid_key_schema"
            })
        ));
        assert!(matches!(
            partition::<Row, Schema<0>, _, Native<u64>>(Ok(1)).build(),
            Err(BuildError::OperationConstraint {
                code: "invalid_key_schema"
            })
        ));
        assert!(
            matches!(partition::<Row, Schema<0>, _, Upper>(Ok("bad".into())).build(), Err(BuildError::Encode(e)) if e.kind == EncodeFault::Custom { code: "key_codec_wire_mismatch" })
        );
    }
}
