//! Physical key definitions derived from static schema metadata.
use aws_sdk_dynamodb::types::{AttributeDefinition, ScalarAttributeType};
use crate::{BuildError, DynamoType, RecordSchema};
impl RecordSchema {
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
