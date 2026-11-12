//! Pure record validation diagnostics built exclusively from typed handles.
use std::{error::Error, fmt, marker::PhantomData};

use crate::{
    Attr, AttributePath, DecodeContext, DecodeError, DecodeFault, Document, EncodeContext,
    EncodeError, EncodeFault, Native, PathSegment,
};

type Invariant<T> = PhantomData<fn(T) -> T>;

/// A path relative to the document being validated. It contains no field values.
/// Construct from an attribute handle, then descend through native documents.
pub struct ValidationPath<R, T, C = Native<T>> {
    path: AttributePath,
    types: Invariant<(R, T, C)>,
}
impl<R, T, P, A, C> From<Attr<R, T, P, A, C>> for ValidationPath<R, T, C> {
    fn from(handle: Attr<R, T, P, A, C>) -> Self {
        Self {
            path: vec![PathSegment::Attribute(handle.name().into())].into(),
            types: PhantomData,
        }
    }
}
impl<R, T: Document> ValidationPath<R, T> {
    pub fn field<U, P, A, C>(self, handle: Attr<T, U, P, A, C>) -> ValidationPath<R, U, C> {
        let mut segments = self.path.segments().to_vec();
        segments.push(PathSegment::Attribute(handle.name().into()));
        ValidationPath {
            path: segments.into(),
            types: PhantomData,
        }
    }
}

/// A structured, value-free refusal. Codes must be static identifiers, never values.
/// All handles must be relative to the document whose hook returns this fault.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ValidationFault {
    path: AttributePath,
    code: &'static str,
    additional_paths: Vec<AttributePath>,
}
impl ValidationFault {
    pub fn new<R, T, C>(handle: impl Into<ValidationPath<R, T, C>>, code: &'static str) -> Self {
        Self {
            path: handle.into().path,
            code,
            additional_paths: Vec::new(),
        }
    }
    pub fn with_path<R, T, C>(mut self, handle: impl Into<ValidationPath<R, T, C>>) -> Self {
        self.additional_paths.push(handle.into().path);
        self
    }
    pub fn path(&self) -> &AttributePath {
        &self.path
    }
    pub fn code(&self) -> &'static str {
        self.code
    }
    pub fn additional_paths(&self) -> &[AttributePath] {
        &self.additional_paths
    }
}
impl fmt::Display for ValidationFault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "validation {} at {}", self.code, self.path)
    }
}
impl Error for ValidationFault {}

macro_rules! conversion {
    ($method:ident, $cx:ident, $error:ident, $kind:ident) => {
        impl ValidationFault {
            #[doc(hidden)]
            pub fn $method(self, cx: &$cx) -> $error {
                let at = |path: &AttributePath| {
                    path.segments()
                        .iter()
                        .fold(cx.clone(), |cx, segment| cx.at(segment.clone()))
                };
                let mut error = at(&self.path).error($kind::Validation { code: self.code });
                error.additional_paths = self
                    .additional_paths
                    .iter()
                    .map(|path| at(path).error($kind::Validation { code: self.code }).path)
                    .collect();
                error
            }
        }
    };
}
conversion!(into_encode_error, EncodeContext, EncodeError, EncodeFault);
conversion!(into_decode_error, DecodeContext, DecodeError, DecodeFault);
