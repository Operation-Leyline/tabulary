//! Explicit null values, independent of optional field presence.
use std::marker::PhantomData;

use aws_sdk_dynamodb::types::AttributeValue;

use crate::{
    DecodeContext, DecodeError, DecodeFault, DynamoType, EncodeContext, EncodeError, EncodeFault,
    Value, WireType,
};

/// A present `NULL(true)` or a strictly decoded value. Missing fields still fail.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Nullable<T> {
    Null,
    Value(T),
}

/// A nullable wire category; it does not inherit scalar expression capabilities.
pub struct NullableWire<W>(PhantomData<W>);

impl<W: WireType> WireType for NullableWire<W> {
    // NULL or W is a union, not one statically known DynamoDB category.
    const DYNAMO_TYPE: DynamoType = DynamoType::Unknown;
}

impl<T: Value> Value for Nullable<T> {
    type Wire = NullableWire<T::Wire>;
    fn encode_value(&self, cx: &mut EncodeContext) -> Result<AttributeValue, EncodeError> {
        match self {
            Self::Null => Ok(AttributeValue::Null(true)),
            Self::Value(value) => match value.encode_value(cx)? {
                AttributeValue::Null(_) => Err(cx.error(EncodeFault::Custom {
                    code: "null_inner_value",
                })),
                value => Ok(value),
            },
        }
    }
    fn decode_value(value: &AttributeValue, cx: &mut DecodeContext) -> Result<Self, DecodeError> {
        match value {
            AttributeValue::Null(true) => Ok(Self::Null),
            AttributeValue::Null(false) => Err(cx.error(DecodeFault::Custom {
                code: "invalid_null",
            })),
            value => T::decode_value(value, cx).map(Self::Value),
        }
    }
}
