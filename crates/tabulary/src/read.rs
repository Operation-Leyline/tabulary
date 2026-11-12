//! Read outcomes preserve SDK accounting even when a returned item is corrupt.
use std::{error::Error, fmt};

use aws_sdk_dynamodb::{error::SdkError, types::ConsumedCapacity};

use crate::{DecodeError, Item};

/// Every read must explicitly choose its consistency; there is no default.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReadConsistency {
    Strong,
    Eventual,
}

#[derive(Debug)]
pub struct GetResult<P> {
    pub item: Option<P>,
    pub capacity: Option<ConsumedCapacity>,
}

/// A strict item decode failure with the unmodified response accounting.
pub struct ReadDecodeError {
    /// Zero-based response position (always zero for GetItem).
    pub position: usize,
    pub source: DecodeError,
    pub capacity: Option<ConsumedCapacity>,
    pub(crate) raw_items: Vec<Item>,
}
impl ReadDecodeError {
    /// Explicit raw payload access, including any secret attributes.
    pub fn raw_items(&self) -> &[Item] {
        &self.raw_items
    }
}
impl fmt::Debug for ReadDecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ReadDecodeError")
            .field("position", &self.position)
            .field("source", &self.source)
            .field("capacity", &self.capacity)
            .field("raw_items", &"[redacted]")
            .finish()
    }
}
impl fmt::Display for ReadDecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "item {}: {}", self.position, self.source)
    }
}
impl Error for ReadDecodeError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(&self.source)
    }
}

/// SDK failures remain typed and are never classified using their messages.
#[non_exhaustive]
pub enum ReadError<E> {
    Sdk(Box<SdkError<E>>),
    Decode(Box<ReadDecodeError>),
}
impl<E> ReadError<E> {
    /// Explicit raw SDK access; the SDK error can contain sensitive values.
    pub fn sdk_error(&self) -> Option<&SdkError<E>> {
        match self {
            Self::Sdk(error) => Some(error),
            Self::Decode(_) => None,
        }
    }
}
impl<E> fmt::Debug for ReadError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Sdk(_) => f.write_str("Sdk([redacted])"),
            Self::Decode(error) => f.debug_tuple("Decode").field(error).finish(),
        }
    }
}
impl<E> fmt::Display for ReadError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Sdk(_) => f.write_str("SDK read failed"),
            Self::Decode(error) => fmt::Display::fmt(error, f),
        }
    }
}
impl<E: Error + Send + Sync + 'static> Error for ReadError<E> {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Sdk(error) => Some(error.as_ref()),
            Self::Decode(error) => Some(error.as_ref()),
        }
    }
}
