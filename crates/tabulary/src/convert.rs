//! Explicit field representations. Expression constants use the same converter.
use std::marker::PhantomData;

use aws_sdk_dynamodb::types::AttributeValue;

use crate::{DecodeContext, DecodeError, EncodeContext, EncodeError, Value, WireType};

/// A field codec whose wire capabilities are independent of its logical Rust type.
///
/// Implementations must use the supplied context for structured errors and must
/// never put rejected values or arbitrary error text into diagnostics.
pub trait Converter<T> {
    type Wire: WireType;
    fn encode(value: &T, cx: &mut EncodeContext) -> Result<AttributeValue, EncodeError>;
    fn decode(value: &AttributeValue, cx: &mut DecodeContext) -> Result<T, DecodeError>;
}

/// Adapt the native [`Value`] codec for field and expression encoding.
pub struct Native<T>(PhantomData<fn() -> T>);

impl<T: Value> Converter<T> for Native<T> {
    type Wire = T::Wire;
    fn encode(value: &T, cx: &mut EncodeContext) -> Result<AttributeValue, EncodeError> {
        value.encode_value(cx)
    }
    fn decode(value: &AttributeValue, cx: &mut DecodeContext) -> Result<T, DecodeError> {
        T::decode_value(value, cx)
    }
}

/// Full-range unsigned integers stored as canonical decimal strings.
pub mod u128_string {
    use super::*;
    use crate::{DecodeFault, DynamoType, value::wrong_type};

    /// String storage with equality only, not string ordering or numeric operators.
    pub struct Wire;

    impl WireType for Wire {}

    pub struct Codec;

    impl Converter<u128> for Codec {
        type Wire = Wire;
        fn encode(value: &u128, _: &mut EncodeContext) -> Result<AttributeValue, EncodeError> {
            Ok(AttributeValue::S(value.to_string()))
        }
        fn decode(value: &AttributeValue, cx: &mut DecodeContext) -> Result<u128, DecodeError> {
            let AttributeValue::S(text) = value else {
                return Err(wrong_type(value, DynamoType::S, cx));
            };
            if text.is_empty()
                || !text.bytes().all(|b| b.is_ascii_digit())
                || (text.len() > 1 && text.starts_with('0'))
            {
                return Err(cx.error(DecodeFault::Custom {
                    code: "invalid_u128_string",
                }));
            }
            text.parse().map_err(|_| {
                cx.error(DecodeFault::Custom {
                    code: "invalid_u128_string",
                })
            })
        }
    }
}
