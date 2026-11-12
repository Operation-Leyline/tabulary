use std::fmt;

use crate::{DecodeContext, DecodeError, EncodeContext, EncodeError, Value};
use aws_sdk_dynamodb::types::AttributeValue;

/// Redaction, not encryption or guaranteed memory zeroization. Debug writes nothing.
/// Raw SDK values returned by Value are explicit access and are not redacted.
///
/// No implicit dereference:
/// ```compile_fail
/// let secret = tabulary::Secret::new(String::from("note"));
/// let _: &String = &*secret;
/// ```
/// No Display:
/// ```compile_fail
/// let _ = format!("{}", tabulary::Secret::new("note"));
/// ```
/// No public inner field:
/// ```compile_fail
/// let _ = tabulary::Secret::new("note").0;
/// ```
pub struct Secret<T>(T);

impl<T> Secret<T> {
    pub const fn new(value: T) -> Self {
        Self(value)
    }
    pub fn expose_secret(&self) -> &T {
        &self.0
    }
    pub fn into_inner(self) -> T {
        self.0
    }
}

impl<T> fmt::Debug for Secret<T> {
    fn fmt(&self, _: &mut fmt::Formatter<'_>) -> fmt::Result {
        Ok(())
    }
}

impl<T: Value> Value for Secret<T> {
    type Wire = T::Wire;
    fn encode_value(&self, cx: &mut EncodeContext) -> Result<AttributeValue, EncodeError> {
        self.0.encode_value(cx)
    }
    fn decode_value(value: &AttributeValue, cx: &mut DecodeContext) -> Result<Self, DecodeError> {
        T::decode_value(value, cx).map(Self)
    }
}
