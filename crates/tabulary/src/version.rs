//! One version policy expansion shared by standalone and transaction-ready writes.
use crate::{
    BuildError, Condition, DecodeContext, FieldAccess, FieldDescriptor, Item, Record, Update,
    Value,
    condition::{ConditionNode, Operand, Operator},
    update::{Action, ActionNode, SetNode},
};

#[derive(Default)]
pub(crate) enum Policy {
    #[default]
    Missing,
    New,
    Expected(u64),
    Unchecked,
    Conflicting,
}
pub(crate) enum Write<'a, R> {
    Put(&'a Item),
    Update(&'a mut Update<R>),
    Delete,
}
impl Policy {
    pub(crate) fn select(&mut self, policy: Self) {
        *self = if matches!(self, Self::Missing) {
            policy
        } else {
            Self::Conflicting
        };
    }
    pub(crate) fn expand<R: Record>(
        self,
        write: Write<'_, R>,
        condition: Option<Condition<R>>,
    ) -> Result<Option<Condition<R>>, BuildError> {
        let mut versions = R::SCHEMA
            .fields
            .iter()
            .enumerate()
            .filter(|(_, field)| field.access == FieldAccess::VersionOnly);
        let field = versions.next();
        if versions.next().is_some() {
            return Err(BuildError::VersionPolicy);
        }
        let field = field
            .map(|(index, field)| {
                let attr = crate::__private::field::<
                    R,
                    u64,
                    crate::Required,
                    crate::VersionOnly,
                    crate::Native<u64>,
                >(field.name, index);
                let field = attr.validate()?;
                if field.index == R::SCHEMA.primary.partition
                    || Some(field.index) == R::SCHEMA.primary.sort
                    || field.name.is_empty()
                {
                    return Err(BuildError::VersionPolicy);
                }
                Ok(field)
            })
            .transpose()?;
        let guard = match (self, field) {
            (Self::Missing, None) | (Self::Unchecked, Some(_)) => return Ok(condition),
            (Self::New, field) if matches!(write, Write::Put(_)) => {
                if let (Some(field), Write::Put(item)) = (field, &write) {
                    check_proposed::<R>(item, field, 1)?;
                }
                let key = R::SCHEMA
                    .fields
                    .get(R::SCHEMA.primary.partition)
                    .filter(|field| {
                        field.index == R::SCHEMA.primary.partition && !field.name.is_empty()
                    })
                    .copied()
                    .ok_or(BuildError::VersionPolicy)?;
                Condition::new(move || {
                    Ok(ConditionNode::Predicate {
                        operator: Operator::NotExists,
                        operands: vec![Operand::Attribute(key)],
                    })
                })
            }
            (Self::Expected(expected), Some(field)) => {
                if !matches!(write, Write::Delete) {
                    let next = expected.checked_add(1).ok_or(BuildError::VersionPolicy)?;
                    match write {
                        Write::Put(item) => check_proposed::<R>(item, field, next)?,
                        Write::Update(changes) => {
                            let action = Action::new(move || {
                                Ok(ActionNode::Set {
                                    target: field,
                                    value: SetNode::Operand(Operand::Value(
                                        crate::Value::encode_value(
                                            &next,
                                            &mut crate::EncodeContext::new(R::SCHEMA.name).at(
                                                crate::PathSegment::Attribute(field.name.into()),
                                            ),
                                        )?,
                                    )),
                                })
                            });
                            *changes = std::mem::take(changes).and(action);
                        }
                        Write::Delete => unreachable!(),
                    }
                }
                crate::__private::field::<
                    R,
                    u64,
                    crate::Required,
                    crate::VersionOnly,
                    crate::Native<u64>,
                >(field.name, field.index)
                .eq(expected)
            }
            _ => return Err(BuildError::VersionPolicy),
        };
        Ok(Some(match condition {
            Some(condition) => condition.and(guard),
            None => guard,
        }))
    }
}
fn check_proposed<R: Record>(
    item: &Item,
    field: FieldDescriptor,
    expected: u64,
) -> Result<(), BuildError> {
    let value = item.get(field.name).ok_or(BuildError::VersionPolicy)?;
    let actual = u64::decode_value(value, &mut DecodeContext::new(R::SCHEMA.name))
        .map_err(|_| BuildError::VersionPolicy)?;
    if actual != expected {
        return Err(BuildError::VersionPolicy);
    }
    Ok(())
}
