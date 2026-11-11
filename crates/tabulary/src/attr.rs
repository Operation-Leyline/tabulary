//! Typed field handles. Constructors are reserved macro helpers.
use std::marker::PhantomData;

use crate::{BuildError, Converter, FieldAccess, FieldDescriptor, Native, Presence, Record};

mod sealed {
    pub trait Sealed {}
}

/// A sealed field-presence marker.
pub trait PresenceMarker: sealed::Sealed {
    const PRESENCE: Presence;
}

/// A sealed mutation-access marker.
pub trait AccessMarker: sealed::Sealed {
    const ACCESS: FieldAccess;
}

macro_rules! markers {
    ($trait:ident, $constant:ident, $kind:ident; $($name:ident),+ $(,)?) => {$(
        #[doc = concat!("The `", stringify!($name), "` field marker.")]
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub struct $name;
        impl sealed::Sealed for $name {}
        impl $trait for $name {
            const $constant: $kind = $kind::$name;
        }
    )+};
}
markers!(PresenceMarker, PRESENCE, Presence; Required, Optional);
markers!(AccessMarker, ACCESS, FieldAccess; Mutable, KeyOnly, VersionOnly, IndexSourceOnly, DerivedOnly);

/// A copyable handle with no owned record, logical value, or codec.
/// The owner is invariant, including when it contains lifetime parameters.
///
/// Handles from another record cannot be supplied to an owner's builder:
/// ```compile_fail
/// use tabulary::Attr;
/// struct Order;
/// struct Invoice;
/// fn order_field(_: Attr<Order, u64>) {}
/// fn foreign(field: Attr<Invoice, u64>) { order_field(field); }
/// ```
pub struct Attr<R, T, P = Required, A = Mutable, C = Native<T>> {
    pub(crate) descriptor: FieldDescriptor,
    pub(crate) owner: PhantomData<fn(R) -> R>,
    pub(crate) types: PhantomData<fn(T, P, A, C)>,
}

impl<R, T, P, A, C> Copy for Attr<R, T, P, A, C> {}
impl<R, T, P, A, C> Clone for Attr<R, T, P, A, C> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<R, T, P, A, C> Attr<R, T, P, A, C> {
    pub const fn name(self) -> &'static str {
        self.descriptor.name
    }
    pub const fn descriptor(self) -> FieldDescriptor {
        self.descriptor
    }
}

impl<R: Record, T, P: PresenceMarker, A: AccessMarker, C: Converter<T>> Attr<R, T, P, A, C> {
    /// Builders must check a handle against the owner's schema before using it.
    pub fn validate(self) -> Result<FieldDescriptor, BuildError> {
        R::SCHEMA
            .fields
            .get(self.descriptor.index)
            .filter(|field| **field == self.descriptor)
            .copied()
            .ok_or(BuildError::OperationConstraint {
                code: "invalid_attribute_descriptor",
            })
    }
}

impl<R: Record, T, P: PresenceMarker, C: Converter<T>> Attr<R, T, P, Mutable, C> {
    /// Validate an ordinary mutation target, including physical key protection.
    /// Dedicated access modes never acquire this capability:
    /// ```compile_fail
    /// use tabulary::{Attr, KeyOnly, Record, Required};
    /// fn mutate<R: Record>(field: Attr<R, u64, Required, KeyOnly>) {
    ///     field.validate_mutation();
    /// }
    /// ```
    /// ```compile_fail
    /// use tabulary::{Attr, Record, Required, VersionOnly};
    /// fn mutate<R: Record>(field: Attr<R, u64, Required, VersionOnly>) {
    ///     field.validate_mutation();
    /// }
    /// ```
    /// ```compile_fail
    /// use tabulary::{Attr, IndexSourceOnly, Record, Required};
    /// fn mutate<R: Record>(field: Attr<R, u64, Required, IndexSourceOnly>) {
    ///     field.validate_mutation();
    /// }
    /// ```
    /// ```compile_fail
    /// use tabulary::{Attr, DerivedOnly, Record, Required};
    /// fn mutate<R: Record>(field: Attr<R, u64, Required, DerivedOnly>) {
    ///     field.validate_mutation();
    /// }
    /// ```
    pub fn validate_mutation(self) -> Result<FieldDescriptor, BuildError> {
        let field = self.validate()?;
        if field.index == R::SCHEMA.primary.partition || Some(field.index) == R::SCHEMA.primary.sort
        {
            return Err(BuildError::OperationConstraint {
                code: "immutable_key",
            });
        }
        Ok(field)
    }
}
