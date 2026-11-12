//! Complete index inputs expanded into the ordinary update AST.
use std::collections::BTreeMap;

use crate::{
    BuildError, EncodeError, FieldAccess, FieldDescriptor, IndexSchema, Item, Record,
    condition::Operand,
    update::{ActionNode, SetNode},
};

/// Generated binding between an index, its complete inputs, and dependency metadata.
/// Derives describe every index in record schema order. Construction hooks are reserved.
pub trait ReindexSchema<R: Record>: IndexSchema<R> {
    type Inputs;
    const INPUTS: &'static [usize];
    const SYNTHETIC: &'static [usize];
    const DEPENDENCIES: &'static [&'static [usize]];
    const SYNTHETIC_DEPENDENCIES: &'static [&'static [usize]];
    fn encode(inputs: &Self::Inputs) -> Result<Item, EncodeError>;
    /// Encode immutable logical inputs from the operation's physical key.
    fn primary(key: &Item) -> Result<Item, BuildError>;
}

type Values = BTreeMap<usize, Option<aws_sdk_dynamodb::types::AttributeValue>>;
type Expand = dyn FnOnce(Option<&Item>) -> Result<Expansion, BuildError>;
pub(crate) struct Request(Box<Expand>);
struct Expansion {
    index: usize,
    fields: &'static [FieldDescriptor],
    dependencies: &'static [&'static [usize]],
    synthetic_dependencies: &'static [&'static [usize]],
    sources: Values,
    keys: Values,
}
fn invalid() -> BuildError {
    BuildError::OperationConstraint {
        code: "invalid_reindex_schema",
    }
}
pub(crate) fn request<R: Record, I: ReindexSchema<R>>(inputs: I::Inputs) -> Request
where
    I::Inputs: 'static,
{
    Request(Box::new(move |key| {
        let schema = R::SCHEMA;
        let index = schema
            .indexes
            .iter()
            .position(|d| *d == I::DEFINITION)
            .ok_or_else(invalid)?;
        if I::DEPENDENCIES.len() != schema.indexes.len()
            || I::SYNTHETIC_DEPENDENCIES.len() != schema.indexes.len()
        {
            return Err(invalid());
        }
        let mut item = I::encode(&inputs)?;
        let mut sources = Values::new();
        let mut keys = Values::new();
        let mut primary = None;
        for &i in I::INPUTS {
            let field = schema.fields.get(i).ok_or_else(invalid)?;
            let value = item.remove(field.name);
            if field.access == FieldAccess::KeyOnly
                || i == schema.primary.partition
                || Some(i) == schema.primary.sort
            {
                if primary.is_none() {
                    primary = Some(I::primary(key.ok_or(BuildError::OperationConstraint {
                        code: "reindex_requires_key",
                    })?)?);
                }
                if value.is_none()
                    || primary.as_ref().and_then(|p| p.get(field.name)) != value.as_ref()
                {
                    return Err(BuildError::OperationConstraint {
                        code: "primary_source_mutation",
                    });
                }
            } else if matches!(
                field.access,
                FieldAccess::Mutable | FieldAccess::IndexSourceOnly
            ) {
                if field.presence == crate::Presence::Required && value.is_none() {
                    return Err(invalid());
                }
                sources.insert(i, value);
            } else {
                return Err(invalid());
            }
        }
        for &i in I::SYNTHETIC {
            let field = schema.fields.get(i).ok_or_else(invalid)?;
            if field.access != FieldAccess::DerivedOnly
                || i == schema.primary.partition
                || Some(i) == schema.primary.sort
            {
                return Err(invalid());
            }
            keys.insert(i, item.remove(field.name));
        }
        if !item.is_empty() {
            return Err(invalid());
        }
        Ok(Expansion {
            index,
            fields: schema.fields,
            dependencies: I::DEPENDENCIES,
            synthetic_dependencies: I::SYNTHETIC_DEPENDENCIES,
            sources,
            keys,
        })
    }))
}

pub(crate) fn expand(
    requests: Vec<Request>,
    key: Option<&Item>,
) -> Result<Vec<ActionNode>, BuildError> {
    let parts = requests
        .into_iter()
        .map(|r| (r.0)(key))
        .collect::<Result<Vec<_>, _>>()?;
    let Some(first) = parts.first() else {
        return Ok(vec![]);
    };
    let (mut sources, mut keys) = (Values::new(), Values::new());
    for (position, part) in parts.iter().enumerate() {
        if parts[..position].iter().any(|p| p.index == part.index) {
            return Err(BuildError::ConflictingReindex);
        }
        for (&i, value) in &part.sources {
            if sources.get(&i).is_some_and(|previous| previous != value) {
                return Err(BuildError::ConflictingReindex);
            }
            sources.insert(i, value.clone());
        }
        for (&i, value) in &part.keys {
            if let Some(Some(previous)) = keys.get(&i)
                && value.as_ref().is_some_and(|value| value != previous)
            {
                return Err(BuildError::ConflictingReindex);
            }
            if !keys.contains_key(&i) || value.is_some() {
                keys.insert(i, value.clone());
            }
        }
    }
    for (index, inputs) in first.dependencies.iter().enumerate() {
        let affected = inputs.iter().any(|i| sources.contains_key(i))
            || first.synthetic_dependencies[index]
                .iter()
                .any(|i| keys.contains_key(i));
        if affected && !parts.iter().any(|p| p.index == index) {
            return Err(BuildError::IncompleteReindex);
        }
    }
    sources.extend(keys);
    sources
        .into_iter()
        .map(|(i, value)| {
            let target = *first.fields.get(i).ok_or_else(invalid)?;
            Ok(match value {
                Some(value) => ActionNode::Set {
                    target,
                    value: SetNode::Operand(Operand::Value(value)),
                },
                None => ActionNode::Remove { target },
            })
        })
        .collect()
}
