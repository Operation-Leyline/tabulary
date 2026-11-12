//! Owned item and key codecs. The metadata-only `Record` trait remains independent.
use aws_sdk_dynamodb::types::AttributeValue;

use crate::{
    DecodeContext, DecodeError, DecodeFault, EncodeContext, EncodeError, EncodeFault,
    KeyFormatFault, Record, Value,
};

pub type Item = std::collections::HashMap<String, AttributeValue>;

/// Encode only physical primary keys; decode accepts a full item's extra fields.
pub trait RecordKey<R: Record>: Sized {
    fn encode_key(&self) -> Result<Item, EncodeError>;
    fn decode_key(item: &Item) -> Result<Self, DecodeError>;
}

pub(crate) fn validate_key(value: &AttributeValue, max: usize) -> Result<(), EncodeFault> {
    let size = match value {
        AttributeValue::S(value) => value.len(),
        AttributeValue::B(value) => value.as_ref().len(),
        AttributeValue::N(value) => {
            return value
                .parse::<crate::Decimal>()
                .map(|_| ())
                .map_err(|_| EncodeFault::NumberRange);
        }
        _ => {
            return Err(EncodeFault::Custom {
                code: "non_scalar_key",
            });
        }
    };
    if size == 0 {
        return Err(EncodeFault::EmptyKey);
    }
    if size > max {
        return Err(EncodeFault::KeySize { max, actual: size });
    }
    Ok(())
}

pub fn encode_key<T: Value>(
    value: &T,
    cx: &mut EncodeContext,
    max: usize,
) -> Result<AttributeValue, EncodeError> {
    let value = value.encode_value(cx)?;
    validate_key(&value, max).map_err(|fault| cx.error(fault))?;
    Ok(value)
}

pub fn decode_key<T: Value>(
    value: Option<&AttributeValue>,
    cx: &mut DecodeContext,
    max: usize,
) -> Result<T, DecodeError> {
    let value = value.ok_or_else(|| cx.error(DecodeFault::Missing))?;
    let decoded = T::decode_value(value, cx)?;
    validate_key(value, max)
        .map_err(|_| cx.error(DecodeFault::KeyFormat(KeyFormatFault::InvalidLength)))?;
    Ok(decoded)
}
