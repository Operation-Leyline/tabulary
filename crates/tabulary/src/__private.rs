//! Reserved derive helpers; these carry no semver guarantee.
use std::marker::PhantomData;

pub use crate::record::{decode_key, encode_key};
pub use crate::value::wrong_type;
pub use aws_sdk_dynamodb::types::AttributeValue;

use crate::{
    AccessMarker, Attr, Converter, FieldDescriptor, KeyDefinition, PresenceMarker, RecordSchema,
    WireType,
};

pub const fn field<R, T, P: PresenceMarker, A: AccessMarker, C: Converter<T>>(
    name: &'static str,
    index: usize,
) -> Attr<R, T, P, A, C> {
    Attr {
        descriptor: FieldDescriptor {
            name,
            index,
            presence: P::PRESENCE,
            access: A::ACCESS,
            wire: C::Wire::DYNAMO_TYPE,
        },
        owner: PhantomData,
        types: PhantomData,
    }
}

pub const fn keys(partition: usize, sort: Option<usize>) -> KeyDefinition {
    KeyDefinition { partition, sort }
}

pub const fn schema(
    name: &'static str,
    fields: &'static [FieldDescriptor],
    primary: KeyDefinition,
) -> RecordSchema {
    RecordSchema {
        name,
        fields,
        primary,
    }
}
