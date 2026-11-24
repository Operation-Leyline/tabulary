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
    count: bool,
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
        // A count returns no attributes; its filter must still stay in the index.
        if !count
            && selection
                .as_ref()
                .is_none_or(|names| names.iter().any(|name| !available.contains(name)))
        {
            return Err(BuildError::OperationConstraint {
                code: "projection_not_available",
            });
        }
        if let Some(filter) = filter {
            visit(filter.root(), &mut |root, _| {
                if available.contains(&root) {
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
/// Visit each attribute operand's top-level name, with its schema position when
/// the operand is a top-level field. A nested path reports its root attribute.
pub(crate) fn visit(
    node: &ConditionNode,
    check: &mut impl FnMut(&str, Option<usize>) -> Result<(), BuildError>,
) -> Result<(), BuildError> {
    match node {
        ConditionNode::Predicate { operands, .. } => {
            for operand in operands {
                match operand {
                    Operand::Attribute(field) | Operand::Size(field) => {
                        check(field.name, Some(field.index))?
                    }
                    Operand::Path(path) | Operand::PathSize(path) => {
                        // Typed nested paths always start at a named attribute.
                        check(path.attribute().unwrap_or_default(), None)?
                    }
                    Operand::Value(_) => {}
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
