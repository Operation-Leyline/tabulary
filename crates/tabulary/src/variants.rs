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

impl<R: crate::Record, T: Value + 'static> crate::Attr<R, T, crate::Required, crate::DerivedOnly> {
    /// Compare a read-only discriminator with one of its typed tags.
    pub fn is(self, tag: T) -> crate::Condition<R> {
        self.eq(tag)
    }
}

/// Compile-time namespace check used by the derives, including across crates.
#[doc(hidden)]
pub const fn assert_disjoint(common: &[&str], variants: &[&str]) {
    let mut i = 0;
    while i < common.len() {
        let mut j = 0;
        while j < variants.len() {
            assert!(
                !same(common[i].as_bytes(), variants[j].as_bytes()),
                "flattened variant attribute overlaps a common field"
            );
            j += 1;
        }
        i += 1;
    }
}

const fn same(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut i = 0;
    while i < a.len() {
        if a[i] != b[i] {
            return false;
        }
        i += 1;
    }
    true
}
