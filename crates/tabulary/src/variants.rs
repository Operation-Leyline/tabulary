//! Strict discriminator-selected maps, also usable as flattened field groups.
use crate::{DecodeContext, DecodeError, Document, Value, wire};

/// A map codec with a declared discriminator and a closed payload namespace.
/// Flattening borrows the containing item and leaves its common attributes alone.
pub trait Variants: Document {
    type Tag: Value<Wire = wire::String>;
    const TAG: &'static str;
    const ATTRIBUTES: &'static [&'static str];
    fn decode_flattened(item: &crate::Item, cx: &mut DecodeContext) -> Result<Self, DecodeError>;
}
