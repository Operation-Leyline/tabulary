//! Explicit containers; byte vectors remain binary values.
use std::collections::{BTreeMap, BTreeSet};

use aws_sdk_dynamodb::{primitives::Blob, types::AttributeValue};

use crate::{
    Decimal, DecodeContext, DecodeError, DecodeFault, DynamoType, EncodeContext, EncodeError,
    EncodeFault, PathSegment, SetFault, Value, value::wrong_type, wire,
};

/// An ordered DynamoDB list, including an empty list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct List<T>(pub Vec<T>);
impl<T> From<Vec<T>> for List<T> {
    fn from(values: Vec<T>) -> Self {
        Self(values)
    }
}
impl<T: Value> Value for List<T> {
    type Wire = wire::List;
    fn encode_value(&self, cx: &mut EncodeContext) -> Result<AttributeValue, EncodeError> {
        let cx = cx.enter_container()?;
        self.0
            .iter()
            .enumerate()
            .map(|(i, value)| value.encode_value(&mut cx.at(PathSegment::Index(i))))
            .collect::<Result<_, _>>()
            .map(AttributeValue::L)
    }
    fn decode_value(value: &AttributeValue, cx: &mut DecodeContext) -> Result<Self, DecodeError> {
        let AttributeValue::L(values) = value else {
            return Err(wrong_type(value, DynamoType::L, cx));
        };
        let cx = cx.enter_container()?;
        values
            .iter()
            .enumerate()
            .map(|(i, value)| T::decode_value(value, &mut cx.at(PathSegment::Index(i))))
            .collect::<Result<_, _>>()
            .map(Self)
    }
}

/// Dynamic data keys, traversed in sorted order and never parsed as dotted paths.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Map<T>(pub BTreeMap<String, T>);
impl<T> From<BTreeMap<String, T>> for Map<T> {
    fn from(values: BTreeMap<String, T>) -> Self {
        Self(values)
    }
}
impl<T: Value> Value for Map<T> {
    type Wire = wire::Map;
    fn encode_value(&self, cx: &mut EncodeContext) -> Result<AttributeValue, EncodeError> {
        let cx = cx.enter_container()?;
        self.0
            .iter()
            .map(|(key, value)| {
                value
                    .encode_value(&mut cx.at(PathSegment::Attribute(key.clone())))
                    .map(|value| (key.clone(), value))
            })
            .collect::<Result<_, _>>()
            .map(AttributeValue::M)
    }
    fn decode_value(value: &AttributeValue, cx: &mut DecodeContext) -> Result<Self, DecodeError> {
        let AttributeValue::M(values) = value else {
            return Err(wrong_type(value, DynamoType::M, cx));
        };
        let cx = cx.enter_container()?;
        values
            .iter()
            .collect::<BTreeMap<_, _>>()
            .into_iter()
            .map(|(key, value)| {
                T::decode_value(value, &mut cx.at(PathSegment::Attribute(key.clone())))
                    .map(|value| (key.clone(), value))
            })
            .collect::<Result<_, _>>()
            .map(Self)
    }
}

macro_rules! set_codec {
    ($name:ident $(<$t:ident>)?, $member:ty, $scalar:ident, $tag:ident, $wire:ident, $normalize:expr, $raw:expr) => {
        /// A nonempty set with unique normalized members. Encoding sorts canonical members.
        #[derive(Clone, Debug, PartialEq, Eq)]
        pub struct $name $(<$t>)? (Vec<$member>);
        impl $(<$t: Value<Wire = wire::Number>>)? $name $(<$t>)? {
            pub fn as_slice(&self) -> &[$member] { &self.0 }
            pub fn into_inner(self) -> Vec<$member> { self.0 }
        }
        impl $(<$t: Value<Wire = wire::Number>>)? TryFrom<Vec<$member>> for $name $(<$t>)? {
            type Error = EncodeError;
            fn try_from(values: Vec<$member>) -> Result<Self, Self::Error> {
                let set = Self(values);
                set.encode_value(&mut EncodeContext::new(stringify!($name)))?;
                Ok(set)
            }
        }
        impl $(<$t: Value<Wire = wire::Number>>)? Value for $name $(<$t>)? {
            type Wire = wire::$wire;
            fn encode_value(&self, cx: &mut EncodeContext) -> Result<AttributeValue, EncodeError> {
                if self.0.is_empty() {
                    return Err(cx.error(EncodeFault::InvalidSet(SetFault::Empty)));
                }
                let mut sorted = BTreeSet::new();
                for (i, member) in self.0.iter().enumerate() {
                    let mut child = cx.at(PathSegment::Index(i));
                    let AttributeValue::$scalar(raw) = member.encode_value(&mut child)? else {
                        return Err(child.error(EncodeFault::Custom { code: "set_member_type" }));
                    };
                    let normalized = ($normalize)(&raw).map_err(|_| child.error(EncodeFault::NumberRange))?;
                    if !sorted.insert(normalized) {
                        return Err(child.error(EncodeFault::InvalidSet(SetFault::Duplicate)));
                    }
                }
                Ok(AttributeValue::$tag(sorted.into_iter().map($raw).collect()))
            }
            fn decode_value(value: &AttributeValue, cx: &mut DecodeContext) -> Result<Self, DecodeError> {
                let AttributeValue::$tag(values) = value else {
                    return Err(wrong_type(value, DynamoType::$tag, cx));
                };
                if values.is_empty() {
                    return Err(cx.error(DecodeFault::InvalidSet(SetFault::Empty)));
                }
                let mut seen = BTreeSet::new();
                let mut members = Vec::with_capacity(values.len());
                for (i, raw) in values.iter().enumerate() {
                    let mut child = cx.at(PathSegment::Index(i));
                    let normalized = ($normalize)(raw).map_err(|kind| child.error(kind))?;
                    if !seen.insert(normalized) {
                        return Err(child.error(DecodeFault::InvalidSet(SetFault::Duplicate)));
                    }
                    members.push(<$member>::decode_value(&AttributeValue::$scalar(raw.clone()), &mut child)?);
                }
                Ok(Self(members))
            }
        }
    };
}

set_codec!(
    StringSet,
    String,
    S,
    Ss,
    StringSet,
    |s: &String| Ok::<_, DecodeFault>(s.clone()),
    |s| s
);
set_codec!(
    NumberSet<T>,
    T,
    N,
    Ns,
    NumberSet,
    |s: &String| s.parse::<Decimal>().map(|n| n.to_string()),
    |s| s
);
set_codec!(
    BinarySet,
    Vec<u8>,
    B,
    Bs,
    BinarySet,
    |b: &Blob| Ok::<_, DecodeFault>(b.as_ref().to_vec()),
    Blob::new
);
