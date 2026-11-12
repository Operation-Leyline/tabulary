//! Continuation identity and versioned, bounded byte encoding.
use std::{collections::BTreeMap, fmt, marker::PhantomData};

use aws_sdk_dynamodb::types::AttributeValue;

use crate::condition::{ConditionNode, Operand, Operator};
use crate::{
    BuildError, BuiltCondition, BuiltKeyCondition, Decimal, FieldDescriptor, IndexDefinition, Item,
    ReadConsistency, Record, RecordSchema, Table,
};

/// Physical sort-key traversal order for a Query.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QueryOrder {
    Ascending,
    Descending,
}

/// An owned continuation, retaining the complete service key rather than an item.
/// Matching a cursor does not authenticate or authorize its use.
pub struct Cursor<R, I> {
    key: Item,
    binding: Identity<R, I>,
}
enum Identity<R, I> {
    Memory(Box<CursorBinding<R, I>>),
    Encoded(Vec<u8>),
}
impl<R, I> Cursor<R, I> {
    /// Explicit raw access; keys can contain sensitive values.
    pub fn last_evaluated_key(&self) -> &Item {
        &self.key
    }
}
impl<R, I> Clone for Cursor<R, I> {
    fn clone(&self) -> Self {
        Self {
            key: self.key.clone(),
            binding: match &self.binding {
                Identity::Memory(binding) => Identity::Memory(binding.clone()),
                Identity::Encoded(bytes) => Identity::Encoded(bytes.clone()),
            },
        }
    }
}
impl<R, I> fmt::Debug for Cursor<R, I> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Cursor([redacted])")
    }
}

#[derive(Clone, PartialEq)]
enum Operation {
    Query(CanonicalNode, QueryOrder),
    Scan(Option<(u32, u32)>),
}

/// No limit or capacity field: these alter the page, not its continuation identity.
type Invariant<R, I> = PhantomData<fn(R, I) -> (R, I)>;
pub(crate) struct CursorBinding<R, I> {
    table: String,
    schema: RecordSchema,
    index: Option<IndexDefinition>,
    operation: Operation,
    filter: Option<CanonicalNode>,
    projection: Option<Vec<&'static str>>,
    consistency: ReadConsistency,
    marker: Invariant<R, I>,
}
impl<R, I> Clone for CursorBinding<R, I> {
    fn clone(&self) -> Self {
        Self {
            table: self.table.clone(),
            schema: self.schema,
            index: self.index,
            operation: self.operation.clone(),
            filter: self.filter.clone(),
            projection: self.projection.clone(),
            consistency: self.consistency,
            marker: PhantomData,
        }
    }
}
impl<R, I> fmt::Debug for CursorBinding<R, I> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("CursorBinding([redacted])")
    }
}
fn projection(names: Option<Vec<&'static str>>) -> Option<Vec<&'static str>> {
    names.map(|mut names| {
        names.sort_unstable();
        names.dedup();
        names
    })
}
impl<R: Record, I> CursorBinding<R, I> {
    pub(crate) fn query(
        table: &Table<R>,
        index: Option<IndexDefinition>,
        key: &BuiltKeyCondition<R, I>,
        filter: Option<&BuiltCondition<R>>,
        selection: Option<Vec<&'static str>>,
        consistency: ReadConsistency,
        order: QueryOrder,
    ) -> Result<Self, BuildError> {
        Self::new(
            table,
            index,
            Operation::Query(CanonicalNode::new(key.root())?, order),
            filter,
            selection,
            consistency,
        )
    }
    pub(crate) fn scan(
        table: &Table<R>,
        index: Option<IndexDefinition>,
        filter: Option<&BuiltCondition<R>>,
        selection: Option<Vec<&'static str>>,
        consistency: ReadConsistency,
        segment: Option<(u32, u32)>,
    ) -> Result<Self, BuildError> {
        Self::new(
            table,
            index,
            Operation::Scan(segment),
            filter,
            selection,
            consistency,
        )
    }
    fn new(
        table: &Table<R>,
        index: Option<IndexDefinition>,
        operation: Operation,
        filter: Option<&BuiltCondition<R>>,
        selection: Option<Vec<&'static str>>,
        consistency: ReadConsistency,
    ) -> Result<Self, BuildError> {
        if index.is_some_and(|index| !R::SCHEMA.indexes.contains(&index)) {
            return Err(BuildError::IncompatibleIndex);
        }
        let binding = Self {
            table: table.name().into(),
            schema: R::SCHEMA,
            index,
            operation,
            filter: filter.map(|f| CanonicalNode::new(f.root())).transpose()?,
            projection: projection(selection),
            consistency,
            marker: PhantomData,
        };
        // Count without allocating the output; reserve room for version and key count.
        bytes::binding_size(&binding)?;
        Ok(binding)
    }
    /// Only absence or an empty map means end; no record decoder is involved.
    pub(crate) fn cursor(&self, key: Option<Item>) -> Result<Option<Cursor<R, I>>, BuildError> {
        let Some(key) = key.filter(|key| !key.is_empty()) else {
            return Ok(None);
        };
        for (name, value) in &key {
            if name.is_empty()
                || matches!(value, AttributeValue::N(n) if n.len() > bytes::MAX_NUMBER_KEY_BYTES)
                || ![
                    Some(self.schema.primary.partition),
                    self.schema.primary.sort,
                    self.index.map(|i| i.keys.partition),
                    self.index.and_then(|i| i.keys.sort),
                ]
                .into_iter()
                .flatten()
                .any(|p| self.schema.fields.get(p).is_some_and(|f| f.name == name))
                || crate::record::validate_key(value, 2048).is_err()
            {
                return Err(BuildError::InvalidCursor);
            }
        }
        for keys in std::iter::once(self.schema.primary).chain(self.index.map(|index| index.keys)) {
            for (position, max) in [(Some(keys.partition), 2048), (keys.sort, 1024)] {
                let Some(position) = position else {
                    continue;
                };
                let field = self
                    .schema
                    .fields
                    .get(position)
                    .ok_or(BuildError::InvalidCursor)?;
                let value = key.get(field.name).ok_or(BuildError::InvalidCursor)?;
                let wire_matches = matches!(
                    (value, field.wire),
                    (AttributeValue::S(_), crate::DynamoType::S)
                        | (AttributeValue::N(_), crate::DynamoType::N)
                        | (AttributeValue::B(_), crate::DynamoType::B)
                );
                if !wire_matches || crate::record::validate_key(value, max).is_err() {
                    return Err(BuildError::InvalidCursor);
                }
            }
        }
        Ok(Some(Cursor {
            key,
            binding: Identity::Memory(Box::new(self.clone())),
        }))
    }
    pub(crate) fn resume<'a>(&self, cursor: &'a Cursor<R, I>) -> Result<&'a Item, BuildError> {
        let other = match &cursor.binding {
            Identity::Memory(other) => other,
            Identity::Encoded(encoded) => {
                if bytes::binding_bytes(self)? != *encoded {
                    return Err(BuildError::CursorMismatch);
                }
                return Ok(&cursor.key);
            }
        };
        if self.table != other.table
            || self.schema != other.schema
            || self.index != other.index
            || self.operation != other.operation
            || self.filter != other.filter
            || self.projection != other.projection
            || self.consistency != other.consistency
        {
            return Err(BuildError::CursorMismatch);
        }
        Ok(&cursor.key)
    }
}

// Structural canonical data, not a hash, Rust identity, Debug string, or wire format.
#[derive(Clone, PartialEq)]
enum CanonicalNode {
    Predicate(Operator, Vec<CanonicalOperand>),
    And(Box<Self>, Box<Self>),
    Or(Box<Self>, Box<Self>),
    Not(Box<Self>),
}
#[derive(Clone, PartialEq)]
enum CanonicalOperand {
    Attribute(FieldDescriptor),
    Size(FieldDescriptor),
    Value(CanonicalValue),
}
impl CanonicalNode {
    fn new(node: &ConditionNode) -> Result<Self, BuildError> {
        Self::at_depth(node, 0)
    }
    fn at_depth(node: &ConditionNode, depth: usize) -> Result<Self, BuildError> {
        if depth > 32 {
            return Err(BuildError::InvalidCursor);
        }
        Ok(match node {
            ConditionNode::Predicate { operator, operands } => Self::Predicate(
                *operator,
                operands
                    .iter()
                    .map(|operand| {
                        Ok(match operand {
                            Operand::Attribute(field) => CanonicalOperand::Attribute(*field),
                            Operand::Size(field) => CanonicalOperand::Size(*field),
                            Operand::Value(value) => {
                                CanonicalOperand::Value(CanonicalValue::new(value, 0)?)
                            }
                        })
                    })
                    .collect::<Result<_, BuildError>>()?,
            ),
            ConditionNode::And(a, b) => Self::And(
                Box::new(Self::at_depth(a, depth + 1)?),
                Box::new(Self::at_depth(b, depth + 1)?),
            ),
            ConditionNode::Or(a, b) => Self::Or(
                Box::new(Self::at_depth(a, depth + 1)?),
                Box::new(Self::at_depth(b, depth + 1)?),
            ),
            ConditionNode::Not(a) => Self::Not(Box::new(Self::at_depth(a, depth + 1)?)),
        })
    }
}
#[derive(Clone, PartialEq)]
enum CanonicalValue {
    String(String),
    Number(Decimal),
    Binary(Vec<u8>),
    Bool(bool),
    Null,
    List(Vec<Self>),
    Map(BTreeMap<String, Self>),
    Strings(Vec<String>),
    Numbers(Vec<Decimal>),
    Binaries(Vec<Vec<u8>>),
}
fn set<T: Ord>(mut values: Vec<T>) -> Result<Vec<T>, BuildError> {
    values.sort_unstable();
    if values.is_empty() || values.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err(BuildError::InvalidCursor);
    }
    Ok(values)
}
impl CanonicalValue {
    fn new(value: &AttributeValue, depth: usize) -> Result<Self, BuildError> {
        use AttributeValue as A;
        let number = |n: &str| n.parse::<Decimal>().map_err(|_| BuildError::InvalidCursor);
        Ok(match value {
            A::S(v) => Self::String(v.clone()),
            A::N(v) => Self::Number(number(v)?),
            A::B(v) => Self::Binary(v.as_ref().to_vec()),
            A::Bool(v) => Self::Bool(*v),
            A::Null(true) => Self::Null,
            A::L(v) if depth < 32 => Self::List(
                v.iter()
                    .map(|v| Self::new(v, depth + 1))
                    .collect::<Result<_, _>>()?,
            ),
            A::M(v) if depth < 32 => Self::Map(
                v.iter()
                    .map(|(k, v)| Ok((k.clone(), Self::new(v, depth + 1)?)))
                    .collect::<Result<_, BuildError>>()?,
            ),
            A::Ss(v) => Self::Strings(set(v.clone())?),
            A::Ns(v) => Self::Numbers(set(v
                .iter()
                .map(|v| number(v))
                .collect::<Result<_, _>>()?)?),
            A::Bs(v) => Self::Binaries(set(v.iter().map(|v| v.as_ref().to_vec()).collect())?),
            _ => return Err(BuildError::InvalidCursor),
        })
    }
}

mod bytes;
#[cfg(test)]
mod tests;
pub use bytes::CursorError;
