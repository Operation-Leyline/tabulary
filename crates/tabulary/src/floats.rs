use crate::{
    Decimal, DecodeContext, DecodeError, DecodeFault, DynamoType, EncodeContext, EncodeError,
    EncodeFault, Value, value::wrong_type, wire,
};
use aws_sdk_dynamodb::types::AttributeValue;

impl Value for f64 {
    type Wire = wire::Number;

    fn encode_value(&self, cx: &mut EncodeContext) -> Result<AttributeValue, EncodeError> {
        if !self.is_finite() {
            return Err(cx.error(EncodeFault::NonFiniteFloat));
        }
        if *self == 0.0 && self.is_sign_negative() {
            return Err(cx.error(EncodeFault::NegativeZero));
        }
        // Display supplies the shortest round-trip digits; Decimal validates
        // their service domain and selects the canonical positional/exponent form.
        let decimal: Decimal = self
            .to_string()
            .parse()
            .map_err(|_| cx.error(EncodeFault::NumberRange))?;
        Ok(AttributeValue::N(decimal.to_string()))
    }

    fn decode_value(value: &AttributeValue, cx: &mut DecodeContext) -> Result<Self, DecodeError> {
        let AttributeValue::N(text) = value else {
            return Err(wrong_type(value, DynamoType::N, cx));
        };
        let decimal: Decimal = text.parse().map_err(|kind| cx.error(kind))?;
        // Parse the bounded canonical form, including when input zero has an
        // arbitrarily large exponent. Retain the original sign check below.
        let number: f64 = decimal
            .to_string()
            .parse()
            .map_err(|_| cx.error(DecodeFault::InvalidNumber))?;
        if !number.is_finite() || (number == 0.0 && text.starts_with('-')) {
            return Err(cx.error(DecodeFault::NumberRange));
        }
        let shortest: Decimal = number
            .to_string()
            .parse()
            .map_err(|_| cx.error(DecodeFault::InexactNumber))?;
        if shortest != decimal {
            return Err(cx.error(DecodeFault::InexactNumber));
        }
        Ok(number)
    }
}
