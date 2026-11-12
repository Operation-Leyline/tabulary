use crate::aliases::AliasAllocator;
use crate::condition::{ConditionNode, Operand, Operator};
use crate::{BuiltCondition, RenderedExpressions};

impl<R> BuiltCondition<R> {
    /// Render with fresh request-local aliases. Repeated rendering is deterministic.
    /// Request-wide expression limits are checked by the owning operation builder.
    pub fn render(&self) -> RenderedExpressions {
        let mut aliases = AliasAllocator::default();
        let condition = Some(render_condition(self.root(), &mut aliases));
        RenderedExpressions {
            condition,
            aliases,
            ..RenderedExpressions::default()
        }
    }
}

/// Other request expressions may allocate bindings before or after this traversal.
pub(crate) fn render_condition(node: &ConditionNode, aliases: &mut AliasAllocator) -> String {
    match node {
        ConditionNode::And(left, right) | ConditionNode::Or(left, right) => {
            let left = render_condition(left, aliases);
            let right = render_condition(right, aliases);
            let op = if matches!(node, ConditionNode::And(..)) {
                "AND"
            } else {
                "OR"
            };
            format!("({left} {op} {right})")
        }
        ConditionNode::Not(inner) => format!("NOT ({})", render_condition(inner, aliases)),
        ConditionNode::Predicate { operator, operands } => {
            // Only validated BuiltCondition trees reach this renderer; their arity is fixed.
            let operands: Vec<_> = operands
                .iter()
                .map(|operand| match operand {
                    Operand::Attribute(field) => aliases.name(field.name),
                    Operand::Size(field) => format!("size({})", aliases.name(field.name)),
                    Operand::Value(value) => aliases.value(value),
                })
                .collect();
            let path = &operands[0];
            match operator {
                Operator::Exists => format!("attribute_exists({path})"),
                Operator::NotExists => format!("attribute_not_exists({path})"),
                Operator::BeginsWith => format!("begins_with({path}, {})", operands[1]),
                Operator::Contains => format!("contains({path}, {})", operands[1]),
                Operator::Between => format!("{path} BETWEEN {} AND {}", operands[1], operands[2]),
                Operator::In => format!("{path} IN ({})", operands[1..].join(", ")),
                op => {
                    let op = match op {
                        Operator::Eq => "=",
                        Operator::Ne => "<>",
                        Operator::Lt => "<",
                        Operator::Le => "<=",
                        Operator::Gt => ">",
                        Operator::Ge => ">=",
                        _ => unreachable!("non-comparison operators handled above"),
                    };
                    format!("{path} {op} {}", operands[1])
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Attr;
    use aws_sdk_dynamodb::types::AttributeValue;

    #[test]
    fn condition_rendering_shares_an_existing_request_allocator() {
        let mut aliases = AliasAllocator::default();
        let value = AttributeValue::N("7".into());
        // Simulate an earlier expression fragment without assuming its grammar.
        assert_eq!(aliases.name("quantity"), "#n0");
        assert_eq!(aliases.value(&value), ":v0");
        let field: Attr<(), u64> = crate::__private::field("quantity", 0);
        let node = ConditionNode::Predicate {
            operator: Operator::Eq,
            operands: vec![
                Operand::Attribute(field.descriptor()),
                Operand::Value(value.clone()),
            ],
        };
        assert_eq!(render_condition(&node, &mut aliases), "#n0 = :v1");
        assert_eq!(aliases.names.len(), 1);
        assert_eq!(aliases.values.len(), 2);
        assert_eq!(aliases.values[":v0"], value);
        assert_eq!(aliases.values[":v1"], value);
    }
}
