use aws_sdk_dynamodb::{primitives::Blob, types::AttributeValue};

use crate::{
    DecodeContext, DecodeError, DecodeFault, DynamoType, EncodeContext, EncodeError, Value,
    value::wrong_type, wire,
};

impl Value for String {
    type Wire = wire::String;

    fn encode_value(&self, _cx: &mut EncodeContext) -> Result<AttributeValue, EncodeError> {
        Ok(AttributeValue::S(self.clone()))
    }

    fn decode_value(value: &AttributeValue, cx: &mut DecodeContext) -> Result<Self, DecodeError> {
        match value {
            AttributeValue::S(value) => Ok(value.clone()),
            _ => Err(wrong_type(value, DynamoType::S, cx)),
        }
    }
}

impl Value for bool {
    type Wire = wire::Bool;

    fn encode_value(&self, _cx: &mut EncodeContext) -> Result<AttributeValue, EncodeError> {
        Ok(AttributeValue::Bool(*self))
    }

    fn decode_value(value: &AttributeValue, cx: &mut DecodeContext) -> Result<Self, DecodeError> {
        match value {
            AttributeValue::Bool(value) => Ok(*value),
            _ => Err(wrong_type(value, DynamoType::Bool, cx)),
        }
    }
}

impl Value for Vec<u8> {
    type Wire = wire::Binary;

    fn encode_value(&self, _cx: &mut EncodeContext) -> Result<AttributeValue, EncodeError> {
        Ok(AttributeValue::B(Blob::new(self.as_slice())))
    }

    fn decode_value(value: &AttributeValue, cx: &mut DecodeContext) -> Result<Self, DecodeError> {
        match value {
            AttributeValue::B(value) => Ok(value.as_ref().to_vec()),
            _ => Err(wrong_type(value, DynamoType::B, cx)),
        }
    }
}

impl<const N: usize> Value for [u8; N] {
    type Wire = wire::Binary;

    fn encode_value(&self, _cx: &mut EncodeContext) -> Result<AttributeValue, EncodeError> {
        Ok(AttributeValue::B(Blob::new(self.as_slice())))
    }

    fn decode_value(value: &AttributeValue, cx: &mut DecodeContext) -> Result<Self, DecodeError> {
        match value {
            AttributeValue::B(value) => value.as_ref().try_into().map_err(|_| {
                cx.error(DecodeFault::InvalidLength {
                    expected: N,
                    actual: value.as_ref().len(),
                })
            }),
            _ => Err(wrong_type(value, DynamoType::B, cx)),
        }
    }
}
