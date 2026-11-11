use crate::{
    Decimal, DecodeContext, DecodeError, DecodeFault, DynamoType, EncodeContext, EncodeError,
    EncodeFault, Value, value::wrong_type, wire,
};
use aws_sdk_dynamodb::types::AttributeValue;

impl Value for Decimal {
    type Wire = wire::Number;

    fn encode_value(&self, _cx: &mut EncodeContext) -> Result<AttributeValue, EncodeError> {
        Ok(AttributeValue::N(self.to_string()))
    }

    fn decode_value(value: &AttributeValue, cx: &mut DecodeContext) -> Result<Self, DecodeError> {
        match value {
            AttributeValue::N(text) => text.parse().map_err(|kind| cx.error(kind)),
            _ => Err(wrong_type(value, DynamoType::N, cx)),
        }
    }
}

fn integer<T: TryFrom<u128> + TryFrom<i128>>(number: Decimal) -> Result<T, DecodeFault> {
    let (negative, magnitude) = number.integer()?;
    if negative {
        i128::try_from(magnitude)
            .ok()
            .and_then(|n| T::try_from(-n).ok())
    } else {
        T::try_from(magnitude).ok()
    }
    .ok_or(DecodeFault::NumberRange)
}

macro_rules! integers {
    ($($ty:ty),* $(,)?) => {$ (
        impl Value for $ty {
            type Wire = wire::Number;

            fn encode_value(&self, cx: &mut EncodeContext) -> Result<AttributeValue, EncodeError> {
                let text = self.to_string();
                text.parse::<Decimal>()
                    .map_err(|_| cx.error(EncodeFault::NumberRange))?;
                Ok(AttributeValue::N(text))
            }

            fn decode_value(value: &AttributeValue, cx: &mut DecodeContext) -> Result<Self, DecodeError> {
                integer(Decimal::decode_value(value, cx)?).map_err(|kind| cx.error(kind))
            }
        }
    )*};
}

integers!(u64, i64, u128);
