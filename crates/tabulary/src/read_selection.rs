//! Shared index-read rules for the one-page Query and Scan adapters.
use crate::condition::{ConditionNode, Operand};
use crate::{
    BuildError, BuiltCondition, IndexDefinition, IndexKind, IndexProjection, ReadConsistency,
    Record,
};

pub(crate) fn validate_consistency(
    index: Option<IndexDefinition>,
    consistency: ReadConsistency,
) -> Result<(), BuildError> {
    if consistency == ReadConsistency::Strong
        && index.is_some_and(|index| index.kind == IndexKind::Global)
    {
        return Err(BuildError::UnsupportedConsistency);
    }
    Ok(())
}

pub(crate) fn validate_selection<R: Record>(
    index: Option<IndexDefinition>,
    selection: Option<&[&str]>,
    filter: Option<&BuiltCondition<R>>,
) -> Result<(), BuildError> {
    if selection.is_some_and(|names| names.is_empty()) {
        return Err(BuildError::EmptyOperation);
    }
    if let Some(index) = index.filter(|index| index.projection != IndexProjection::All) {
        let mut available = Vec::new();
        for keys in [R::SCHEMA.primary, index.keys] {
            for position in [Some(keys.partition), keys.sort].into_iter().flatten() {
                available.push(
                    R::SCHEMA
                        .fields
                        .get(position)
                        .ok_or(BuildError::IncompatibleIndex)?
                        .name,
                );
            }
        }
        if let IndexProjection::Include(names) = index.projection {
            available.extend_from_slice(names);
        }
        if selection
            .as_ref()
            .is_none_or(|names| names.iter().any(|name| !available.contains(name)))
        {
            return Err(BuildError::OperationConstraint {
                code: "projection_not_available",
            });
        }
        if let Some(filter) = filter {
            visit(filter.root(), &mut |field| {
                if available.contains(&field.name) {
                    Ok(())
                } else {
                    Err(BuildError::OperationConstraint {
                        code: "projection_not_available",
                    })
                }
            })?;
        }
    }
    Ok(())
}
pub(crate) fn visit(
    node: &ConditionNode,
    check: &mut impl FnMut(&crate::FieldDescriptor) -> Result<(), BuildError>,
) -> Result<(), BuildError> {
    match node {
        ConditionNode::Predicate { operands, .. } => {
            for operand in operands {
                if let Operand::Attribute(field) | Operand::Size(field) = operand {
                    check(field)?;
                }
            }
            Ok(())
        }
        ConditionNode::And(a, b) | ConditionNode::Or(a, b) => {
            visit(a, check)?;
            visit(b, check)
        }
        ConditionNode::Not(a) => visit(a, check),
    }
}
