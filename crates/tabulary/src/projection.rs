//! Read-only selected-attribute decoding, independent of request execution.
use crate::{
    Attr, BuildError, DecodeContext, DecodeError, DecodeFault, Document, DynamoType, EncodeContext,
    FieldDescriptor, Item, Optional, PathSegment, Required,
};
use aws_sdk_dynamodb::types::AttributeValue;

/// A partial read with the source's field codecs, presence, and explicit defaults.
/// Extra response attributes belong to the caller's transport validation.
pub trait Projection: Sized {
    type Source: Document;
    fn attributes() -> Result<Vec<&'static str>, BuildError>;
    fn decode_map(item: &Item, cx: &mut DecodeContext) -> Result<Self, DecodeError>;
    fn from_item(item: &Item) -> Result<Self, DecodeError> {
        Self::decode_map(item, &mut DecodeContext::new(Self::Source::NAME))
    }
    /// Check a non-ALL index's complete stored namespace, including transport keys.
    /// Callers supply primary/index key names as well as INCLUDE attributes.
    fn check_available(available: &[&str], transport: &[&str]) -> Result<(), BuildError> {
        if Self::attributes()?
            .iter()
            .chain(transport)
            .any(|name| !available.contains(name))
        {
            return Err(BuildError::OperationConstraint {
                code: "projection_not_available",
            });
        }
        Ok(())
    }
}

/// Reserved source-field hooks emitted by Document and Record derives.
#[doc(hidden)]
pub trait SourceField<const I: usize>: Document {
    type Handle: Handle;
    type Output;
    type Presence;
    const ATTRIBUTES: &'static [&'static str];
    fn decode(item: &Item, cx: &mut DecodeContext) -> Result<Self::Output, DecodeError>;
}

#[doc(hidden)]
pub trait Handle {
    fn descriptor(self) -> FieldDescriptor;
}
impl<R, T, P, A, C> Handle for Attr<R, T, P, A, C> {
    fn descriptor(self) -> FieldDescriptor {
        self.descriptor()
    }
}

#[doc(hidden)]
pub fn select<S: SourceField<I>, const I: usize>(
    handle: S::Handle,
) -> Result<&'static [&'static str], BuildError> {
    if S::FIELDS.get(I) != Some(&handle.descriptor()) {
        return Err(BuildError::OperationConstraint {
            code: "invalid_projection_attribute",
        });
    }
    Ok(S::ATTRIBUTES)
}

#[doc(hidden)]
pub fn append(names: &mut Vec<&'static str>, selected: &[&'static str]) -> Result<(), BuildError> {
    for name in selected {
        if names.contains(name) {
            return Err(BuildError::OperationConstraint {
                code: "duplicate_projection_attribute",
            });
        }
        names.push(name);
    }
    Ok(())
}

/// Presence-specific nested decoding keeps Option out of the Value codec.
#[doc(hidden)]
pub trait NestedValue<P: Projection, Presence>: Sized {
    type Output;
    fn present(value: &AttributeValue, cx: &mut DecodeContext)
    -> Result<Self::Output, DecodeError>;
    fn default_view(self, cx: &mut DecodeContext) -> Result<Self::Output, DecodeError>;
}
#[doc(hidden)]
pub fn nested_field<R, T: Document + crate::Value<Wire = crate::wire::Map>, P, A>(
    _: Attr<R, T, P, A>,
) {
}
fn nested<P: Projection>(value: &AttributeValue, cx: &mut DecodeContext) -> Result<P, DecodeError> {
    let cx = cx.in_document(P::Source::NAME);
    let AttributeValue::M(item) = value else {
        return Err(crate::__private::wrong_type(value, DynamoType::M, &cx));
    };
    P::decode_map(item, &mut cx.enter_container()?)
}
impl<T: Document, P: Projection<Source = T>> NestedValue<P, Required> for T {
    type Output = P;
    fn present(value: &AttributeValue, cx: &mut DecodeContext) -> Result<P, DecodeError> {
        nested::<P>(value, cx)
    }
    fn default_view(self, cx: &mut DecodeContext) -> Result<P, DecodeError> {
        let mut cx = cx.in_document(T::NAME).enter_container()?;
        let item = self
            .encode_map(&mut EncodeContext::new(T::NAME))
            .map_err(|_| {
                cx.error(DecodeFault::Custom {
                    code: "invalid_projection_default",
                })
            })?;
        P::decode_map(&item, &mut cx)
    }
}
impl<T: Document, P: Projection<Source = T>> NestedValue<P, Optional> for Option<T> {
    type Output = Option<P>;
    fn present(
        value: &AttributeValue,
        cx: &mut DecodeContext,
    ) -> Result<Self::Output, DecodeError> {
        nested::<P>(value, cx).map(Some)
    }
    fn default_view(self, cx: &mut DecodeContext) -> Result<Self::Output, DecodeError> {
        self.map(|value| <T as NestedValue<P, Required>>::default_view(value, cx))
            .transpose()
    }
}
#[doc(hidden)]
pub fn decode_nested<S: SourceField<I>, const I: usize, P: Projection>(
    item: &Item,
    cx: &mut DecodeContext,
) -> Result<<S::Output as NestedValue<P, S::Presence>>::Output, DecodeError>
where
    S::Output: NestedValue<P, S::Presence>,
{
    let name = S::ATTRIBUTES[0];
    let mut child = cx.at(PathSegment::Attribute(name.into()));
    match item.get(name) {
        Some(value) => <S::Output as NestedValue<P, S::Presence>>::present(value, &mut child),
        None => S::decode(item, cx)?.default_view(&mut child),
    }
}
