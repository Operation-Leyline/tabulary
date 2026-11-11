#![forbid(unsafe_code)]

#[doc(hidden)]
pub mod __private;
mod attr;
pub mod convert;
mod decimal;
mod errors;
pub mod field;
mod nullable;
mod numbers;
mod scalar;
mod schema;
mod value;
pub mod wire;

pub use attr::AccessMarker;
pub use attr::Attr;
pub use attr::DerivedOnly;
pub use attr::IndexSourceOnly;
pub use attr::KeyOnly;
pub use attr::Mutable;
pub use attr::Optional;
pub use attr::PresenceMarker;
pub use attr::Required;
pub use attr::VersionOnly;
pub use convert::{Converter, Native};
pub use decimal::Decimal;
pub use errors::*;
pub use nullable::{Nullable, NullableWire};
pub use schema::{FieldAccess, FieldDescriptor, KeyDefinition, Presence, Record, RecordSchema};
pub use value::Value;
pub use wire::WireType;
