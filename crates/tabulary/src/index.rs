//! Secondary key schemas and storage projections, without a client or table.
use std::marker::PhantomData;

use aws_sdk_dynamodb::types::{AttributeDefinition, ScalarAttributeType};

use crate::{BuildError, Converter, DynamoType, KeyDefinition, Native, Record, RecordSchema};

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

impl RecordSchema {
    #[doc(hidden)]
    pub const fn with_indexes(mut self, indexes: &'static [IndexDefinition]) -> Self {
        self.indexes = indexes;
        self
    }
    /// Only physical primary/index keys are DynamoDB AttributeDefinitions.
    /// Included and ordinary attributes are deliberately excluded.
    pub fn attribute_definitions(self) -> Result<Vec<AttributeDefinition>, BuildError> {
        let positions: std::collections::BTreeSet<_> = std::iter::once(self.primary)
            .chain(self.indexes.iter().map(|i| i.keys))
            .flat_map(|k| std::iter::once(k.partition).chain(k.sort))
            .collect();
        let mut names = std::collections::BTreeSet::new();
        positions
            .into_iter()
            .map(|position| {
                let field = self
                    .fields
                    .get(position)
                    .ok_or(BuildError::IncompatibleIndex)?;
                if !names.insert(field.name) {
                    return Err(BuildError::IncompatibleIndex);
                }
                let wire = match field.wire {
                    DynamoType::S => ScalarAttributeType::S,
                    DynamoType::N => ScalarAttributeType::N,
                    DynamoType::B => ScalarAttributeType::B,
                    _ => return Err(BuildError::IncompatibleIndex),
                };
                AttributeDefinition::builder()
                    .attribute_name(field.name)
                    .attribute_type(wire)
                    .build()
                    .map_err(|_| BuildError::IncompatibleIndex)
            })
            .collect()
    }
}
