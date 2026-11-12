//! Request-local bindings shared by expression renderers. Segments are never parsed.
use std::collections::HashMap;

use aws_sdk_dynamodb::types::AttributeValue;

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
