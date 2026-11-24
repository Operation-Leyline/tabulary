//! Request-local bindings shared by expression renderers. Segments are never parsed.
use std::collections::HashMap;

use aws_sdk_dynamodb::types::AttributeValue;

use crate::{AttributePath, PathSegment, condition::Operand};

#[derive(Default)]
pub(crate) struct AliasAllocator {
    by_segment: HashMap<String, String>,
    pub(crate) names: HashMap<String, String>,
    pub(crate) values: HashMap<String, AttributeValue>,
}
impl AliasAllocator {
    pub(crate) fn name(&mut self, segment: &str) -> String {
        if let Some(alias) = self.by_segment.get(segment) {
            return alias.clone();
        }
        let alias = format!("#n{}", self.names.len());
        self.by_segment.insert(segment.into(), alias.clone());
        self.names.insert(alias.clone(), segment.into());
        alias
    }
    /// Alias every attribute segment; list indexes stay literal `[n]` dereferences.
    pub(crate) fn path(&mut self, path: &AttributePath) -> String {
        let mut text = String::new();
        for segment in path.segments() {
            match segment {
                PathSegment::Attribute(name) => {
                    if !text.is_empty() {
                        text.push('.');
                    }
                    text.push_str(&self.name(name));
                }
                PathSegment::Index(index) => text.push_str(&format!("[{index}]")),
                PathSegment::Component(_) => {
                    unreachable!("expression paths have no key components")
                }
            }
        }
        text
    }
    /// Render one validated operand, allocating names before values in traversal order.
    pub(crate) fn operand(&mut self, operand: &Operand) -> String {
        match operand {
            Operand::Attribute(field) => self.name(field.name),
            Operand::Size(field) => format!("size({})", self.name(field.name)),
            Operand::Path(path) => self.path(path),
            Operand::PathSize(path) => format!("size({})", self.path(path)),
            Operand::Value(value) => self.value(value),
        }
    }
    pub(crate) fn value(&mut self, value: &AttributeValue) -> String {
        let alias = format!(":v{}", self.values.len());
        self.values.insert(alias.clone(), value.clone());
        alias
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fragments_share_bindings_and_fresh_requests_restart_numbering() {
        let mut aliases = AliasAllocator::default();
        let value = AttributeValue::N("7".into());
        let first = format!("{} = {}", aliases.name("quantity"), aliases.value(&value));
        let second = format!("{} = {}", aliases.name("quantity"), aliases.value(&value));
        assert_eq!(first, "#n0 = :v0");
        assert_eq!(second, "#n0 = :v1");
        assert_eq!(aliases.name("gift.note"), "#n1");
        assert_eq!(aliases.names["#n1"], "gift.note");
        assert_eq!(aliases.values.len(), 2);
        let mut fresh = AliasAllocator::default();
        assert_eq!(fresh.name("gift.note"), "#n0");
        assert_eq!(fresh.value(&value), ":v0");
    }
}
