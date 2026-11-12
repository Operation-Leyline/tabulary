use std::{collections::HashMap, fmt};

use aws_sdk_dynamodb::types::AttributeValue;

use crate::{
    BuildError,
    aliases::AliasAllocator,
    request_size::{add, ensure},
};

/// Expression text and its exact SDK bindings. Debug omits all operand values.
#[derive(Default)]
pub struct RenderedExpressions {
    pub(crate) condition: Option<String>,
    pub(crate) update: Option<String>,
    pub(crate) key_condition: Option<String>,
    pub(crate) filter: Option<String>,
    pub(crate) projection: Option<String>,
    pub(crate) aliases: AliasAllocator,
}
impl RenderedExpressions {
    /// Check rendered service limits using the complete shared binding maps.
    /// Operation builders call this after all guards and projections are added.
    /// Binding bytes are a conservative JSON bound, not an SDK body measurement.
    pub fn validate(&self) -> Result<(), BuildError> {
        for (text, update) in [
            (self.condition(), false),
            (self.update(), true),
            (self.key_condition(), false),
            (self.filter(), false),
            (self.projection(), false),
        ] {
            if let Some(text) = text {
                expression(text, update)?;
            }
        }
        for alias in self.names().keys().chain(self.values().keys()) {
            ensure(255, alias.len())?;
        }
        ensure(
            2 * 1024 * 1024,
            crate::request_size::bindings(self.names(), self.values())?,
        )
    }
    pub fn condition(&self) -> Option<&str> {
        self.condition.as_deref()
    }
    pub fn update(&self) -> Option<&str> {
        self.update.as_deref()
    }
    pub fn key_condition(&self) -> Option<&str> {
        self.key_condition.as_deref()
    }
    pub fn filter(&self) -> Option<&str> {
        self.filter.as_deref()
    }
    pub fn projection(&self) -> Option<&str> {
        self.projection.as_deref()
    }
    pub fn names(&self) -> &HashMap<String, String> {
        &self.aliases.names
    }
    /// Explicit access to raw SDK values, including secrets.
    pub fn values(&self) -> &HashMap<String, AttributeValue> {
        &self.aliases.values
    }
}

fn expression(text: &str, update: bool) -> Result<(), BuildError> {
    ensure(4096, text.len())?;
    let mut operators = 0;
    let mut between = false;
    // Only renderer-owned grammar reaches here: all names and constants are
    // aliases. Literal dots, brackets and operator words never enter the text.
    for token in text.split(|c: char| c.is_ascii_whitespace() || "(),".contains(c)) {
        ensure(
            32,
            token.bytes().filter(|b| matches!(b, b'.' | b'[')).count(),
        )?;
        let operator = match token {
            "=" => !update,
            "BETWEEN" => {
                between = true;
                true
            }
            "AND" if between => {
                between = false;
                false
            }
            "<>"
            | "<"
            | "<="
            | ">"
            | ">="
            | "AND"
            | "OR"
            | "NOT"
            | "IN"
            | "attribute_exists"
            | "attribute_not_exists"
            | "begins_with"
            | "contains"
            | "size"
            | "if_not_exists"
            | "list_append"
            | "+"
            | "-" => true,
            _ => false,
        };
        operators = add(operators, usize::from(operator))?;
    }
    ensure(300, operators)
}
impl fmt::Debug for RenderedExpressions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RenderedExpressions")
            .field("condition", &self.condition)
            .field("update", &self.update)
            .field("key_condition", &self.key_condition)
            .field("filter", &self.filter)
            .field("projection", &self.projection)
            .field("names", self.names())
            .field("values", &"[redacted]")
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expression_byte_path_and_operator_boundaries() {
        // Legal whitespace pins the exact byte boundary independently of aliases.
        assert!(expression(&format!("#n0{}", " ".repeat(4093)), false).is_ok());
        assert_eq!(
            expression(&format!("#n0{}", " ".repeat(4094)), false),
            Err(BuildError::RequestLimit {
                max: 4096,
                actual: 4097
            })
        );
        for depth in [32, 33] {
            for path in [
                format!("#n0{}", ".#n1".repeat(depth)),
                format!("#n0{}", "[0]".repeat(depth)),
            ] {
                assert_eq!(
                    expression(&format!("SET #n2 = if_not_exists({path}, :v0)"), true).is_ok(),
                    depth == 32
                );
            }
            for target in [false, true] {
                let root: crate::AttributePath =
                    vec![crate::PathSegment::Attribute("literal.[0]".into())].into();
                let mut nested = root.segments().to_vec();
                nested.extend((0..depth).map(crate::PathSegment::Index));
                let nested: crate::AttributePath = nested.into();
                let action = crate::update::Action::<()>::new(move || {
                    Ok(crate::update::ActionNode::NestedSet {
                        target: if target { nested.clone() } else { root.clone() },
                        value: crate::update::SetNode::Path(if target { root } else { nested }),
                    })
                });
                let rendered = crate::Update::new().and(action).build().unwrap().render();
                assert_eq!(rendered.validate().is_ok(), depth == 32);
            }
        }
        assert!(expression(&"#n0 = :v0 ".repeat(300), false).is_ok());
        assert!(matches!(
            expression(&"#n0 = :v0 ".repeat(301), false),
            Err(BuildError::RequestLimit {
                max: 300,
                actual: 301
            })
        ));
        // SET assignment does not count; functions and arithmetic do.
        assert!(expression(&"#n0 = :v0 ".repeat(301), true).is_ok());
        let between = "(#n0 BETWEEN :v0 AND :v1) AND (#n0 BETWEEN :v2 AND :v3)";
        assert!(expression(&format!("{}{between}", "NOT ".repeat(297)), false).is_ok());
        assert_eq!(
            expression(&format!("{}{between}", "NOT ".repeat(298)), false),
            Err(BuildError::RequestLimit {
                max: 300,
                actual: 301
            })
        );
        for token in [
            "size",
            "attribute_exists",
            "attribute_not_exists",
            "begins_with",
            "contains",
            "if_not_exists",
            "list_append",
            "+",
            "-",
            "NOT",
            "AND",
            "OR",
            "BETWEEN",
            "IN",
            "<>",
            "<",
            "<=",
            ">",
            ">=",
        ] {
            let boundary = format!("{}{token}", "+ ".repeat(299));
            assert!(expression(&boundary, false).is_ok());
            assert_eq!(
                expression(&format!("{boundary} +"), false),
                Err(BuildError::RequestLimit {
                    max: 300,
                    actual: 301
                })
            );
        }
    }

    #[test]
    fn binding_bound_is_shared_checked_and_counts_json_expansion() {
        let mut rendered = RenderedExpressions::default();
        let alias = rendered.aliases.value(&AttributeValue::S(String::new()));
        let overhead = crate::request_size::bindings(rendered.names(), rendered.values()).unwrap();
        let payload = 2 * 1024 * 1024 - overhead;
        rendered
            .aliases
            .values
            .insert(alias.clone(), AttributeValue::S("x".repeat(payload)));
        assert!(rendered.validate().is_ok());
        rendered
            .aliases
            .values
            .insert(alias, AttributeValue::S("x".repeat(payload + 1)));
        assert_eq!(
            rendered.validate(),
            Err(BuildError::RequestLimit {
                max: 2_097_152,
                actual: 2_097_153
            })
        );
        rendered.aliases.values.clear();
        rendered.aliases.name("x");
        rendered.aliases.names.insert("#".repeat(256), "x".into());
        assert_eq!(
            rendered.validate(),
            Err(BuildError::RequestLimit {
                max: 255,
                actual: 256
            })
        );
    }
}
