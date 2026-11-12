//! Secondary key schemas and storage projections, without a client or table.
use std::marker::PhantomData;

use crate::{BuildError, Converter, KeyDefinition, Native, Record, RecordSchema};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IndexKind {
    Global,
    Local,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IndexProjection {
    All,
    KeysOnly,
    Include(&'static [&'static str]),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IndexDefinition {
    pub name: &'static str,
    pub kind: IndexKind,
    pub keys: KeyDefinition,
    pub projection: IndexProjection,
}

/// A hash-only schema has no sort predicates.
pub struct NoSort;
/// Logical sort value and its physical codec.
pub struct SortKey<T, C = Native<T>>(PhantomData<fn() -> (T, C)>);

/// Implemented by a distinct generated marker for each record's index.
pub trait IndexSchema<R: Record> {
    type Partition;
    type PartitionCodec: Converter<Self::Partition>;
    type Sort;
    const DEFINITION: IndexDefinition;
}

pub(crate) fn validate_set<R: Record>(
    target: crate::FieldDescriptor,
    node: &crate::update::SetNode,
) -> Result<(), BuildError> {
    use crate::{condition::Operand::Value, update::SetNode};
    let value = match node {
        SetNode::Operand(Value(v))
        | SetNode::IfNotExists {
            default: Value(v), ..
        }
        | SetNode::IfPathNotExists {
            default: Value(v), ..
        } => v,
        _ => return Ok(()), // The result of a stored-value expression is unknown locally.
    };
    let cx = crate::EncodeContext::new(R::SCHEMA.name)
        .at(crate::PathSegment::Attribute(target.name.into()));
    for index in R::SCHEMA.indexes {
        for (position, limit) in [(Some(index.keys.partition), 2048), (index.keys.sort, 1024)] {
            if position == Some(target.index) {
                crate::record::validate_key(value, limit).map_err(|fault| cx.error(fault))?;
            }
        }
    }
    Ok(())
}

impl RecordSchema {
    #[doc(hidden)]
    pub const fn with_indexes(mut self, indexes: &'static [IndexDefinition]) -> Self {
        self.indexes = indexes;
        self
    }
}
