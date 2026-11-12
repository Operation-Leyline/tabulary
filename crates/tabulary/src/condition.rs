//! Owned conditions and opt-in codec capabilities. No expression strings are accepted.
use std::{fmt, marker::PhantomData};

use aws_sdk_dynamodb::types::AttributeValue;

use crate::{
    AccessMarker, Attr, BinarySet, BuildError, Converter, EncodeContext, FieldDescriptor, List,
    Native, NumberSet, PathSegment, PresenceMarker, Record, StringSet, Value, WireType, wire,
};

/// Wire categories whose encoded order preserves the logical order.
pub trait Ordered: WireType {}
/// Wire categories supporting a typed prefix.
pub trait Prefix: WireType {}
/// Wire categories supporting DynamoDB's numeric size function.
pub trait SizedWire: WireType {}
macro_rules! capabilities {
    ($cap:ident: $($wire:ident),+) => {$(impl $cap for wire::$wire {})+};
}
capabilities!(Ordered: Number, String, Binary);
capabilities!(Prefix: String, Binary);
capabilities!(SizedWire: String, Binary, List, Map, StringSet, NumberSet, BinarySet);

/// A codec explicitly declares both the logical element and its encoding.
pub trait Contains<T>: Converter<T> {
    type Element: 'static;
    type ElementCodec: Converter<Self::Element> + 'static;
}
macro_rules! contains {
    ($container:ty, $element:ty) => {
        impl Contains<$container> for Native<$container> {
            type Element = $element;
            type ElementCodec = Native<$element>;
        }
    };
}
contains!(String, String);
contains!(StringSet, String);
contains!(BinarySet, Vec<u8>);
impl<T: Value + 'static> Contains<List<T>> for Native<List<T>> {
    type Element = T;
    type ElementCodec = Native<T>;
}
impl<T: Value<Wire = wire::Number> + 'static> Contains<NumberSet<T>> for Native<NumberSet<T>> {
    type Element = T;
    type ElementCodec = Native<T>;
}

/// Predicate grammar, separate from its later textual rendering.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Operator {
    Exists,
    NotExists,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    Between,
    BeginsWith,
    Contains,
    In,
}

/// Built operands can be inspected explicitly; Debug always hides constants.
#[derive(Clone, PartialEq)]
pub enum Operand {
    Attribute(FieldDescriptor),
    Size(FieldDescriptor),
    Value(AttributeValue),
}
impl fmt::Debug for Operand {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Attribute(field) => f.debug_tuple("Attribute").field(field).finish(),
            Self::Size(field) => f.debug_tuple("Size").field(field).finish(),
            Self::Value(_) => f.write_str("Value([redacted])"),
        }
    }
}

/// A read-only result tree. Constructing a node cannot bypass typed builders.
#[derive(Debug, PartialEq)]
pub enum ConditionNode {
    Predicate {
        operator: Operator,
        operands: Vec<Operand>,
    },
    And(Box<Self>, Box<Self>),
    Or(Box<Self>, Box<Self>),
    Not(Box<Self>),
}

/// A validated tree with encoded constants, owned by exactly one record type.
pub struct BuiltCondition<R> {
    root: ConditionNode,
    owner: PhantomData<fn(R) -> R>,
}
impl<R> BuiltCondition<R> {
    pub fn root(&self) -> &ConditionNode {
        &self.root
    }
    pub fn into_root(self) -> ConditionNode {
        self.root
    }
}
impl<R> fmt::Debug for BuiltCondition<R> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.root.fmt(f)
    }
}

/// An owned condition. Constants are encoded only by `build`, using their field codec.
///
/// Owners, logical types and codecs must agree:
/// ```compile_fail
/// use tabulary::{Attr, Record};
/// fn foreign<R: Record, S: Record>(a: Attr<R, u64>, b: Attr<S, u64>) { a.eq_path(b); }
/// ```
/// ```compile_fail
/// use tabulary::{Attr, Record};
/// fn wrong<R: Record>(a: Attr<R, u64>) { a.eq("3".to_owned()); }
/// ```
/// ```compile_fail
/// use tabulary::{Attr, Converter, Mutable, Record, Required, wire};
/// fn codec<R: Record, C: Converter<u64, Wire = wire::Number> + 'static>(
///     a: Attr<R, u64>, b: Attr<R, u64, Required, Mutable, C>,
/// ) { a.eq_path(b); }
/// ```
/// ```compile_fail
/// use tabulary::{Attr, Record};
/// fn grouping<R: Record, S: Record>(a: Attr<R, u64>, b: Attr<S, u64>) { a.eq(1).and(b.eq(1)); }
/// ```
/// Capabilities follow the codec, including equality-only string-encoded integers:
/// ```compile_fail
/// use tabulary::{Attr, Mutable, Record, Required, convert::u128_string::Codec};
/// fn ordering<R: Record>(a: Attr<R, u128, Required, Mutable, Codec>) { a.lt(3); }
/// ```
/// ```compile_fail
/// use tabulary::{Attr, Record};
/// fn prefix<R: Record>(a: Attr<R, u64>) { a.begins_with(3); }
/// ```
/// ```compile_fail
/// use tabulary::{Attr, Record};
/// fn size<R: Record>(a: Attr<R, bool>) { a.size(); }
/// ```
/// ```compile_fail
/// use tabulary::{Attr, List, Record};
/// fn element<R: Record>(a: Attr<R, List<u64>>) { a.contains("3".to_owned()); }
/// ```
/// ```compile_fail
/// use tabulary::{Attr, Optional, Record};
/// fn absent<R: Record>(a: Attr<R, u64, Optional>) { a.eq(None); }
/// ```
pub struct Condition<R> {
    build: Box<dyn FnOnce() -> Result<ConditionNode, BuildError>>,
    owner: PhantomData<fn(R) -> R>,
}
impl<R> fmt::Debug for Condition<R> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Condition([unbuilt])")
    }
}
impl<R> Condition<R> {
    pub(crate) fn new(build: impl FnOnce() -> Result<ConditionNode, BuildError> + 'static) -> Self {
        Self {
            build: Box::new(build),
            owner: PhantomData,
        }
    }
    pub fn build(self) -> Result<BuiltCondition<R>, BuildError> {
        Ok(BuiltCondition {
            root: (self.build)()?,
            owner: PhantomData,
        })
    }
    pub fn and(self, other: Self) -> Self {
        self.join(other, ConditionNode::And)
    }
    pub fn or(self, other: Self) -> Self {
        self.join(other, ConditionNode::Or)
    }
    fn join(
        self,
        other: Self,
        join: fn(Box<ConditionNode>, Box<ConditionNode>) -> ConditionNode,
    ) -> Self {
        let (left, right) = (self.build, other.build);
        Self::new(move || Ok(join(Box::new(left()?), Box::new(right()?))))
    }
    #[allow(clippy::should_implement_trait)]
    pub fn not(self) -> Self {
        let build = self.build;
        Self::new(move || Ok(ConditionNode::Not(Box::new(build()?))))
    }
}

fn predicate<R: Record, T: 'static, C: Converter<T> + 'static>(
    field: Result<FieldDescriptor, BuildError>,
    size: bool,
    operator: Operator,
    values: Vec<T>,
) -> Condition<R> {
    Condition::new(move || {
        let field = field?;
        if operator == Operator::In {
            if values.is_empty() {
                return Err(BuildError::EmptyOperation);
            }
            if values.len() > 100 {
                return Err(BuildError::RequestLimit {
                    max: 100,
                    actual: values.len(),
                });
            }
        }
        let mut operands = vec![if size {
            Operand::Size(field)
        } else {
            Operand::Attribute(field)
        }];
        let mut cx =
            EncodeContext::new(R::SCHEMA.name).at(PathSegment::Attribute(field.name.into()));
        for value in values {
            operands.push(Operand::Value(C::encode(&value, &mut cx)?));
        }
        Ok(ConditionNode::Predicate { operator, operands })
    })
}

macro_rules! comparisons {
    ($t:ty; $($method:ident => $op:ident),+) => {$(
        pub fn $method(self, value: $t) -> Condition<R> { self.predicate(Operator::$op, vec![value]) }
    )+};
}
macro_rules! ordered_comparisons {
    ($t:ty) => {
        comparisons!($t; lt => Lt, le => Le, gt => Gt, ge => Ge);
        pub fn between(self, lo: $t, hi: $t) -> Condition<R> { self.predicate(Operator::Between, vec![lo, hi]) }
    };
}
impl<R: Record, T: 'static, P: PresenceMarker, A: AccessMarker, C: Converter<T> + 'static>
    Attr<R, T, P, A, C>
{
    fn predicate(self, operator: Operator, values: Vec<T>) -> Condition<R> {
        predicate::<R, T, C>(self.validate(), false, operator, values)
    }
    comparisons!(T; eq => Eq, ne => Ne);
    pub fn exists(self) -> Condition<R> {
        self.predicate(Operator::Exists, vec![])
    }
    pub fn not_exists(self) -> Condition<R> {
        self.predicate(Operator::NotExists, vec![])
    }
    pub fn in_values(self, values: Vec<T>) -> Condition<R> {
        self.predicate(Operator::In, values)
    }
    pub fn eq_path<P2: PresenceMarker, A2: AccessMarker>(
        self,
        other: Attr<R, T, P2, A2, C>,
    ) -> Condition<R> {
        let (left, right) = (self.validate(), other.validate());
        Condition::new(move || {
            Ok(ConditionNode::Predicate {
                operator: Operator::Eq,
                operands: vec![Operand::Attribute(left?), Operand::Attribute(right?)],
            })
        })
    }
    pub fn begins_with(self, value: T) -> Condition<R>
    where
        C::Wire: Prefix,
    {
        self.predicate(Operator::BeginsWith, vec![value])
    }
    pub fn size(self) -> Size<R>
    where
        C::Wire: SizedWire,
    {
        Size {
            field: self.validate(),
            owner: PhantomData,
        }
    }
    pub fn contains(self, value: C::Element) -> Condition<R>
    where
        C: Contains<T>,
    {
        predicate::<R, C::Element, C::ElementCodec>(
            self.validate(),
            false,
            Operator::Contains,
            vec![value],
        )
    }
}
impl<R: Record, T: 'static, P: PresenceMarker, A: AccessMarker, C: Converter<T> + 'static>
    Attr<R, T, P, A, C>
where
    C::Wire: Ordered,
{
    ordered_comparisons!(T);
}

/// A numeric size operand. It retains its source's record and validated descriptor.
pub struct Size<R> {
    field: Result<FieldDescriptor, BuildError>,
    owner: PhantomData<fn(R) -> R>,
}
impl<R: Record> Size<R> {
    fn predicate(self, operator: Operator, values: Vec<u64>) -> Condition<R> {
        predicate::<R, u64, Native<u64>>(self.field, true, operator, values)
    }
    pub fn in_values(self, values: Vec<u64>) -> Condition<R> {
        self.predicate(Operator::In, values)
    }
    comparisons!(u64; eq => Eq, ne => Ne);
    ordered_comparisons!(u64);
}
