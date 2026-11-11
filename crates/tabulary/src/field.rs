//! Field presence is separate from value encoding; `Option<T>` is not a `Value`.
//!
//! Pass a context already positioned at the physical field. These helpers never
//! substitute defaults for missing required fields or malformed present values.
//!
//! ```compile_fail
//! fn value<T: tabulary::Value>() {}
//! value::<Option<String>>();
//! ```
use aws_sdk_dynamodb::types::AttributeValue;

use crate::{Converter, DecodeContext, DecodeError, DecodeFault, EncodeContext, EncodeError};

pub fn encode_optional<T, C: Converter<T>>(
    value: &Option<T>,
    cx: &mut EncodeContext,
) -> Result<Option<AttributeValue>, EncodeError> {
    value.as_ref().map(|value| C::encode(value, cx)).transpose()
}

pub fn decode_optional<T, C: Converter<T>>(
    value: Option<&AttributeValue>,
    cx: &mut DecodeContext,
) -> Result<Option<T>, DecodeError> {
    value.map(|value| C::decode(value, cx)).transpose()
}

pub fn decode_required<T, C: Converter<T>>(
    value: Option<&AttributeValue>,
    cx: &mut DecodeContext,
) -> Result<T, DecodeError> {
    let value = value.ok_or_else(|| cx.error(DecodeFault::Missing))?;
    C::decode(value, cx)
}
