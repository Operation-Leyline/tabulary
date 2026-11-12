//! Structured diagnostics. Payload values and arbitrary converter errors are never stored.
use std::{error::Error, fmt};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PathSegment {
    Attribute(String),
    Index(usize),
    Component(&'static str),
}

/// Physical attribute names, list indexes, and composite-key components.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AttributePath(Vec<PathSegment>);
impl From<Vec<PathSegment>> for AttributePath {
    fn from(segments: Vec<PathSegment>) -> Self {
        Self(segments)
    }
}
impl AttributePath {
    pub fn segments(&self) -> &[PathSegment] {
        &self.0
    }
    pub fn attribute(&self) -> Option<&str> {
        self.0.iter().find_map(|segment| match segment {
            PathSegment::Attribute(name) => Some(name.as_str()),
            _ => None,
        })
    }
}
impl fmt::Display for AttributePath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for segment in &self.0 {
            match segment {
                PathSegment::Attribute(name) => write!(f, "[{name:?}]")?,
                PathSegment::Index(index) => write!(f, "[{index}]")?,
                PathSegment::Component(name) => write!(f, "[component:{name:?}]")?,
            }
        }
        Ok(())
    }
}

macro_rules! faults {
    ($(pub enum $name:ident { $($variants:tt)* })*) => {$ (
        #[non_exhaustive]
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum $name { $($variants)* }
    )*};
}
faults! {
    pub enum DynamoType { S, N, B, Bool, Null, L, M, Ss, Ns, Bs, Unknown }
    pub enum SetFault { Empty, Duplicate }
    pub enum KeyFormatFault { InvalidPrefix, InvalidHex, InvalidLength, InvalidUtf8, InvalidNumber, InvalidSeparator }
    pub enum DecodeFault {
        Missing,
        WrongType { expected: DynamoType, actual: DynamoType },
        InvalidNumber, NumberRange, InexactNumber,
        InvalidLength { expected: usize, actual: usize },
        InvalidUtf8, InvalidSet(SetFault), UnknownVariant, InactiveVariantAttribute,
        UnknownAttribute, KeyFormat(KeyFormatFault), KeyMismatch, IndexMismatch,
        Validation { code: &'static str }, Custom { code: &'static str },
        DepthLimit, UnsupportedType,
    }
    pub enum EncodeFault {
        NumberRange, NonFiniteFloat, NegativeZero, InvalidSet(SetFault),
        KeyFormat(KeyFormatFault), EmptyKey,
        KeySize { max: usize, actual: usize },
        Validation { code: &'static str }, Custom { code: &'static str },
        DepthLimit, SizeOverflow,
    }
}

impl fmt::Display for DecodeFault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self, f)
    }
}
impl Error for DecodeFault {}

macro_rules! diagnostics {
    ($error:ident, $context:ident, $fault:ident) => {
        #[derive(Clone, PartialEq, Eq)]
        pub struct $error {
            pub record: &'static str,
            pub document: &'static str,
            pub path: AttributePath,
            pub kind: $fault,
            /// Other physical paths implicated by a validation fault.
            pub additional_paths: Vec<AttributePath>,
        }
        impl $error {
            pub fn attribute(&self) -> Option<&str> {
                self.path.attribute()
            }
        }
        impl fmt::Display for $error {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(
                    f,
                    "{} ({} at {}): {:?}",
                    self.record, self.document, self.path, self.kind
                )
            }
        }
        impl fmt::Debug for $error {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                let mut debug = f.debug_struct(stringify!($error));
                debug
                    .field("record", &self.record)
                    .field("document", &self.document)
                    .field("path", &self.path)
                    .field("kind", &self.kind);
                if !self.additional_paths.is_empty() {
                    debug.field("additional_paths", &self.additional_paths);
                }
                debug.finish()
            }
        }
        impl Error for $error {}

        /// An owned traversal snapshot; child contexts never change their parent.
        #[derive(Clone, Debug)]
        pub struct $context {
            record: &'static str,
            document: &'static str,
            path: AttributePath,
            depth: usize,
        }
        impl $context {
            pub fn new(record: &'static str) -> Self {
                Self {
                    record,
                    document: record,
                    path: AttributePath::default(),
                    depth: 0,
                }
            }
            /// Add exactly one segment; names are never parsed as dotted paths.
            pub fn at(&self, segment: PathSegment) -> Self {
                let mut child = self.clone();
                child.path.0.push(segment);
                child
            }
            /// Change the current document while retaining the outer record and path.
            pub fn in_document(&self, document: &'static str) -> Self {
                Self {
                    document,
                    ..self.clone()
                }
            }
            /// Enter one map or list; the top-level item is depth zero.
            pub fn enter_container(&self) -> Result<Self, $error> {
                if self.depth >= 32 {
                    return Err(self.error($fault::DepthLimit));
                }
                Ok(Self {
                    depth: self.depth + 1,
                    ..self.clone()
                })
            }
            pub fn error(&self, kind: $fault) -> $error {
                $error {
                    record: self.record,
                    document: self.document,
                    path: self.path.clone(),
                    kind,
                    additional_paths: Vec::new(),
                }
            }
        }
    };
}
diagnostics!(DecodeError, DecodeContext, DecodeFault);
diagnostics!(EncodeError, EncodeContext, EncodeFault);

/// Local preparation failures; encoding retains its complete diagnostic as a source.
#[non_exhaustive]
#[derive(Clone, PartialEq, Eq)]
pub enum BuildError {
    Encode(EncodeError),
    OperationConstraint {
        code: &'static str,
    },
    ContradictoryUpdate,
    OverlappingUpdate {
        first: AttributePath,
        second: AttributePath,
    },
    InvalidCursor,
    CursorMismatch,
    CursorTooLarge,
    IncompatibleIndex,
    UnsupportedConsistency,
    IncompleteReindex,
    ConflictingReindex,
    VersionPolicy,
    EmptyOperation,
    RequestLimit {
        max: usize,
        actual: usize,
    },
    SizeOverflow,
}
impl From<EncodeError> for BuildError {
    fn from(error: EncodeError) -> Self {
        Self::Encode(error)
    }
}
impl Error for BuildError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Encode(error) => Some(error),
            _ => None,
        }
    }
}
impl fmt::Display for BuildError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self, f)
    }
}
impl fmt::Debug for BuildError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Encode(error) => f.debug_tuple("Encode").field(error).finish(),
            Self::OperationConstraint { code } => f
                .debug_struct("OperationConstraint")
                .field("code", code)
                .finish(),
            Self::OverlappingUpdate { first, second } => f
                .debug_struct("OverlappingUpdate")
                .field("first", first)
                .field("second", second)
                .finish(),
            Self::RequestLimit { max, actual } => f
                .debug_struct("RequestLimit")
                .field("max", max)
                .field("actual", actual)
                .finish(),
            Self::ContradictoryUpdate => f.write_str("ContradictoryUpdate"),
            Self::InvalidCursor => f.write_str("InvalidCursor"),
            Self::CursorMismatch => f.write_str("CursorMismatch"),
            Self::CursorTooLarge => f.write_str("CursorTooLarge"),
            Self::IncompatibleIndex => f.write_str("IncompatibleIndex"),
            Self::UnsupportedConsistency => f.write_str("UnsupportedConsistency"),
            Self::IncompleteReindex => f.write_str("IncompleteReindex"),
            Self::ConflictingReindex => f.write_str("ConflictingReindex"),
            Self::VersionPolicy => f.write_str("VersionPolicy"),
            Self::EmptyOperation => f.write_str("EmptyOperation"),
            Self::SizeOverflow => f.write_str("SizeOverflow"),
        }
    }
}
