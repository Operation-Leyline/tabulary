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
    /// Select the payload namespace of a single variant in a flattened group.
    pub fn variant<S>(self, tag: S) -> S::Payload
    where
        R: VariantRecord,
        R::Group: Variants<Tag = T>,
        S: VariantScope<R, R::Group>,
    {
        tag.scope(self)
    }

    /// Compare a read-only discriminator with one of its typed tags.
    pub fn is(self, tag: impl Into<T>) -> crate::Condition<R> {
        self.eq(tag.into())
    }
}

/// Compile-time namespace check used by the derives, including across crates.
#[doc(hidden)]
pub const fn disjoint(common: &[&str], variants: &[&str]) -> bool {
    let mut i = 0;
    while i < common.len() {
        let mut j = 0;
        while j < variants.len() {
            if same(common[i].as_bytes(), variants[j].as_bytes()) {
                return false;
            }
            j += 1;
        }
        i += 1;
    }
    true
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

/// Implemented by generated singleton tags, binding a payload to its group.
pub trait VariantScope<R, V: Variants> {
    type Payload;
    #[doc(hidden)]
    fn scope(
        self,
        group: crate::Attr<R, V::Tag, crate::Required, crate::DerivedOnly>,
    ) -> Self::Payload;
}

/// The single flattened group whose payloads may be scoped by a record.
pub trait VariantRecord: crate::Record {
    type Group: Variants;
}
