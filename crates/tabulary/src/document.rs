//! Owned maps, independent of table and key metadata.
use crate::{DecodeContext, DecodeError, EncodeContext, EncodeError, Item};

/// A strict named-field map codec. The top-level map does not consume nesting depth.
pub trait Document: Sized {
    const NAME: &'static str;
    /// Physical fields available to typed nested paths. Manual codecs opt in explicitly.
    const FIELDS: &'static [crate::FieldDescriptor] = &[];
    fn encode_map(&self, cx: &mut EncodeContext) -> Result<Item, EncodeError>;
    fn decode_map(item: &Item, cx: &mut DecodeContext) -> Result<Self, DecodeError>;
}
