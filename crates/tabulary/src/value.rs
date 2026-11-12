use aws_sdk_dynamodb::types::AttributeValue;

use crate::{
    DecodeContext, DecodeError, DecodeFault, DynamoType, EncodeContext, EncodeError, WireType,
};

/// Strict conversion between one Rust value and one DynamoDB attribute value.
/// Field presence and key validation belong to their enclosing codecs.
pub trait Value: Sized {
    type Wire: WireType;

    fn encode_value(&self, cx: &mut EncodeContext) -> Result<AttributeValue, EncodeError>;

    fn decode_value(value: &AttributeValue, cx: &mut DecodeContext) -> Result<Self, DecodeError>;
}

pub fn wrong_type(value: &AttributeValue, expected: DynamoType, cx: &DecodeContext) -> DecodeError {
    let actual = match value {
        AttributeValue::B(_) => DynamoType::B,
        AttributeValue::Bool(_) => DynamoType::Bool,
        AttributeValue::Bs(_) => DynamoType::Bs,
        AttributeValue::L(_) => DynamoType::L,
        AttributeValue::M(_) => DynamoType::M,
        AttributeValue::N(_) => DynamoType::N,
        AttributeValue::Ns(_) => DynamoType::Ns,
        AttributeValue::Null(_) => DynamoType::Null,
        AttributeValue::S(_) => DynamoType::S,
        AttributeValue::Ss(_) => DynamoType::Ss,
        _ => return cx.error(DecodeFault::UnsupportedType),
    };
    cx.error(DecodeFault::WrongType { expected, actual })
}
