//! Shared immutable input, expression bindings, and item decoder storage.
use std::fmt;

use aws_sdk_dynamodb::types::ReturnConsumedCapacity;

use crate::{DecodeError, Item, RenderedExpressions};

/// Requested capacity detail. None omits ReturnConsumedCapacity entirely.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Capacity {
    #[default]
    None,
    Total,
    Indexes,
}
impl Capacity {
    pub(crate) fn sdk(self) -> Option<ReturnConsumedCapacity> {
        match self {
            Self::None => None,
            Self::Total => Some(ReturnConsumedCapacity::Total),
            Self::Indexes => Some(ReturnConsumedCapacity::Indexes),
        }
    }
}

/// An immutable operation input with its typed output decoder. Performs no I/O.
/// Operation aliases provide the corresponding SDK apply/decode methods.
pub struct PreparedRequest<I, P> {
    pub(crate) input: I,
    pub(crate) expressions: RenderedExpressions,
    pub(crate) decode_item: fn(&Item) -> Result<P, DecodeError>,
}
impl<I, P> PreparedRequest<I, P> {
    /// Explicit raw SDK access; formatting this input may reveal values.
    pub fn input(&self) -> &I {
        &self.input
    }
    pub fn expressions(&self) -> &RenderedExpressions {
        &self.expressions
    }
}
impl<I, P> fmt::Debug for PreparedRequest<I, P> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PreparedRequest")
            .field("input", &"[redacted]")
            .field("expressions", &self.expressions)
            .finish()
    }
}
