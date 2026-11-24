//! Reserved derive helpers; these carry no semver guarantee.
use std::marker::PhantomData;

pub use crate::key_condition::partition;
pub use crate::record::{decode_key, decode_key_with, encode_key, encode_key_with};
pub use crate::scoped_variant::scoped;
pub use crate::value::wrong_type;
pub use crate::variants::disjoint;
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
        indexes: &[],
        ttl: None,
    }
}

/// One shared key descriptor of a multi-object schema, typed from a member.
/// A missing member key yields `Unknown`, which the schema checks reject.
pub const fn multi_key<R: crate::Record>(
    name: &'static str,
    index: usize,
    sort: bool,
) -> FieldDescriptor {
    let primary = R::SCHEMA.primary;
    let position = if sort {
        primary.sort
    } else {
        Some(primary.partition)
    };
    let wire = match position {
        Some(position) if position < R::SCHEMA.fields.len() => R::SCHEMA.fields[position].wire,
        _ => crate::DynamoType::Unknown,
    };
    FieldDescriptor {
        name,
        index,
        presence: crate::Presence::Required,
        access: crate::FieldAccess::KeyOnly,
        wire,
    }
}
