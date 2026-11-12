use std::{collections::HashMap, fmt};

use aws_sdk_dynamodb::types::AttributeValue;

use crate::aliases::AliasAllocator;

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
