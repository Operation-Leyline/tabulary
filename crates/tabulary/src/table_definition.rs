//! Inspectable table inputs. No client, activation polling, or migrations.
use std::{collections::BTreeMap, marker::PhantomData};

use crate::{
    BuildError, DynamoType, IndexKind, IndexProjection, KeyDefinition, Presence, Record,
    RecordSchema,
};
use aws_sdk_dynamodb::{
    operation::create_table::CreateTableInput,
    types::{
        AttributeDefinition, BillingMode, GlobalSecondaryIndex, KeySchemaElement, KeyType,
        LocalSecondaryIndex, Projection, ProjectionType, ProvisionedThroughput,
        ScalarAttributeType,
    },
};

/// Explicit read/write capacity units; both must be positive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Throughput {
    pub read: i64,
    pub write: i64,
}
impl Throughput {
    fn sdk(self) -> Result<ProvisionedThroughput, BuildError> {
        if self.read <= 0 || self.write <= 0 {
            return Err(constraint("invalid_throughput"));
        }
        ProvisionedThroughput::builder()
            .read_capacity_units(self.read)
            .write_capacity_units(self.write)
            .build()
            .map_err(|_| constraint("invalid_throughput"))
    }
}

/// No implicit billing mode or capacity defaults.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Billing {
    OnDemand,
    Provisioned {
        table: Throughput,
        global_indexes: BTreeMap<String, Throughput>,
    },
}

/// Static record schemas combined into one physical table definition.
pub struct TableDefinition<R: Record> {
    schema: RecordSchema,
    marker: PhantomData<fn() -> R>,
}
impl<R: Record> TableDefinition<R> {
    pub(crate) fn new(schema: RecordSchema) -> Self {
        Self {
            schema,
            marker: PhantomData,
        }
    }
    /// Construct a complete SDK input. Only primary/index keys define attributes.
    pub fn create_input(
        &self,
        table: impl Into<String>,
        billing: Billing,
    ) -> Result<CreateTableInput, BuildError> {
        let table = table.into();
        if !identifier(&table) {
            return Err(constraint("invalid_table_name"));
        }
        let shape = self.shape()?;
        let (mode, throughput, mut global_units) = match billing {
            Billing::OnDemand => (BillingMode::PayPerRequest, None, BTreeMap::new()),
            Billing::Provisioned {
                table,
                global_indexes,
            } => {
                let names: Vec<_> = shape
                    .indexes
                    .iter()
                    .filter(|(_, i)| i.kind == IndexKind::Global)
                    .map(|(n, _)| *n)
                    .collect();
                if names.len() != global_indexes.len()
                    || names.iter().any(|n| !global_indexes.contains_key(*n))
                {
                    return Err(constraint("gsi_throughput_mismatch"));
                }
                let units: Result<BTreeMap<_, _>, BuildError> = global_indexes
                    .into_iter()
                    .map(|(name, units)| Ok((name, units.sdk()?)))
                    .collect();
                (BillingMode::Provisioned, Some(table.sdk()?), units?)
            }
        };
        let mut global = Vec::new();
        let mut local = Vec::new();
        for (name, index) in shape.indexes {
            match index.kind {
                IndexKind::Global => global.push(
                    GlobalSecondaryIndex::builder()
                        .index_name(name)
                        .set_key_schema(Some(index.keys))
                        .projection(index.projection)
                        .set_provisioned_throughput(global_units.remove(name))
                        .build()
                        .map_err(|_| incompatible())?,
                ),
                IndexKind::Local => local.push(
                    LocalSecondaryIndex::builder()
                        .index_name(name)
                        .set_key_schema(Some(index.keys))
                        .projection(index.projection)
                        .build()
                        .map_err(|_| incompatible())?,
                ),
            }
        }
        let attributes: Result<Vec<_>, _> = shape
            .attributes
            .into_iter()
            .map(|(name, wire)| {
                AttributeDefinition::builder()
                    .attribute_name(name)
                    .attribute_type(wire)
                    .build()
                    .map_err(|_| incompatible())
            })
            .collect();
        CreateTableInput::builder()
            .table_name(table)
            .billing_mode(mode)
            .set_attribute_definitions(Some(attributes?))
            .set_key_schema(Some(shape.primary))
            .set_global_secondary_indexes((!global.is_empty()).then_some(global))
            .set_local_secondary_indexes((!local.is_empty()).then_some(local))
            .set_provisioned_throughput(throughput)
            .build()
            .map_err(|_| incompatible())
    }
    fn shape(&self) -> Result<Shape, BuildError> {
        let shape = Shape::from_schema(self.schema)?;
        let globals = shape
            .indexes
            .values()
            .filter(|i| i.kind == IndexKind::Global)
            .count();
        let includes: usize = shape
            .indexes
            .values()
            .map(|i| i.projection.non_key_attributes().len())
            .sum();
        if globals > 20 || shape.indexes.len() - globals > 5 || includes > 100 {
            return Err(incompatible());
        }
        Ok(shape)
    }
}

#[derive(PartialEq)]
struct PhysicalIndex {
    kind: IndexKind,
    keys: Vec<KeySchemaElement>,
    projection: Projection,
}
struct Shape {
    attributes: BTreeMap<&'static str, ScalarAttributeType>,
    primary: Vec<KeySchemaElement>,
    indexes: BTreeMap<&'static str, PhysicalIndex>,
}
impl Shape {
    fn from_schema(schema: RecordSchema) -> Result<Self, BuildError> {
        let mut names = std::collections::BTreeSet::new();
        if schema
            .fields
            .iter()
            .enumerate()
            .any(|(i, f)| f.index != i || !names.insert(f.name))
        {
            return Err(incompatible());
        }
        let primary = keys(schema, schema.primary)?;
        for i in std::iter::once(schema.primary.partition).chain(schema.primary.sort) {
            if schema.fields[i].presence != Presence::Required {
                return Err(incompatible());
            }
        }
        let mut result = Self {
            attributes: BTreeMap::new(),
            primary,
            indexes: BTreeMap::new(),
        };
        for attr in schema.attribute_definitions()? {
            result.attributes.insert(
                schema
                    .fields
                    .iter()
                    .find(|f| f.name == attr.attribute_name())
                    .ok_or_else(incompatible)?
                    .name,
                attr.attribute_type().clone(),
            );
        }
        for index in schema.indexes {
            let index_keys = keys(schema, index.keys)?;
            if !identifier(index.name)
                || (index.kind == IndexKind::Local
                    && (result.primary.len() != 2
                        || index_keys.len() != 2
                        || index_keys[0] != result.primary[0]
                        || index_keys[1] == result.primary[1]))
            {
                return Err(incompatible());
            }
            let projection = match index.projection {
                IndexProjection::All => Projection::builder()
                    .projection_type(ProjectionType::All)
                    .build(),
                IndexProjection::KeysOnly => Projection::builder()
                    .projection_type(ProjectionType::KeysOnly)
                    .build(),
                IndexProjection::Include(names) => {
                    let mut sorted = names.to_vec();
                    sorted.sort_unstable();
                    if names.is_empty()
                        || names.len() > 20
                        || sorted.windows(2).any(|w| w[0] == w[1])
                        || names.iter().any(|n| {
                            !attribute_name(n)
                                || !schema.fields.iter().any(|f| f.name == *n)
                                || result
                                    .primary
                                    .iter()
                                    .chain(&index_keys)
                                    .any(|k| k.attribute_name() == *n)
                        })
                    {
                        return Err(incompatible());
                    }
                    Projection::builder()
                        .projection_type(ProjectionType::Include)
                        .set_non_key_attributes(Some(
                            sorted.into_iter().map(str::to_owned).collect(),
                        ))
                        .build()
                }
            };
            if result
                .indexes
                .insert(
                    index.name,
                    PhysicalIndex {
                        kind: index.kind,
                        keys: index_keys,
                        projection,
                    },
                )
                .is_some()
            {
                return Err(incompatible());
            }
        }
        Ok(result)
    }
}
fn keys(schema: RecordSchema, keys: KeyDefinition) -> Result<Vec<KeySchemaElement>, BuildError> {
    if keys.sort == Some(keys.partition) {
        return Err(incompatible());
    }
    std::iter::once((keys.partition, KeyType::Hash))
        .chain(keys.sort.map(|i| (i, KeyType::Range)))
        .map(|(i, kind)| {
            let field = schema.fields.get(i).ok_or_else(incompatible)?;
            if !attribute_name(field.name) || scalar(field.wire).is_none() {
                return Err(incompatible());
            }
            KeySchemaElement::builder()
                .attribute_name(field.name)
                .key_type(kind)
                .build()
                .map_err(|_| incompatible())
        })
        .collect()
}
fn scalar(wire: DynamoType) -> Option<ScalarAttributeType> {
    match wire {
        DynamoType::S => Some(ScalarAttributeType::S),
        DynamoType::N => Some(ScalarAttributeType::N),
        DynamoType::B => Some(ScalarAttributeType::B),
        _ => None,
    }
}
fn identifier(name: &str) -> bool {
    (3..=255).contains(&name.len())
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b))
}
fn attribute_name(name: &str) -> bool {
    (1..=255).contains(&name.len())
}
fn incompatible() -> BuildError {
    BuildError::IncompatibleIndex
}
fn constraint(code: &'static str) -> BuildError {
    BuildError::OperationConstraint { code }
}

#[cfg(test)]
mod tests;
