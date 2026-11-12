//! Shared single-write outcomes. Classification never inspects SDK messages.
use std::{error::Error, fmt};

use aws_sdk_dynamodb::{
    error::SdkError,
    operation::{
        delete_item::DeleteItemError, put_item::PutItemError, update_item::UpdateItemError,
    },
    types::{ConsumedCapacity, ItemCollectionMetrics},
};

use crate::{DecodeContext, DecodeError, Document, Item, Record, Redacted};

/// Shared vocabulary for single-write and transaction refusals.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FailureKind {
    ConditionFailed,
    TransactionConflict,
    ReplicatedWriteConflict,
    ProvisionedThroughputExceeded,
    Throttled,
    RequestLimitExceeded,
    ItemCollectionSizeLimitExceeded,
    Validation,
    NoFailureReported,
    Unavailable,
    UnknownServiceCode(String),
}

/// Whether to request ALL_OLD on a failed condition. None omits the SDK field.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum OldItemMode {
    #[default]
    None,
    All,
}

pub enum OldItem<R> {
    NotRequested,
    Unavailable,
    Decoded(R),
    Invalid {
        error: DecodeError,
        raw: Redacted<Item>,
    },
}
impl<R: Record + Document> OldItem<R> {
    /// Strictly decode the complete record, independently of the refusal kind.
    /// An unsolicited item is retained in the SDK source, not decoded here.
    pub fn decode(mode: OldItemMode, item: Option<Item>) -> Self {
        if mode == OldItemMode::None {
            return Self::NotRequested;
        }
        let Some(raw) = item else {
            return Self::Unavailable;
        };
        match R::decode_map(&raw, &mut DecodeContext::new(R::NAME)) {
            Ok(record) => Self::Decoded(record),
            Err(error) => Self::Invalid {
                error,
                raw: Redacted::new(raw),
            },
        }
    }
}
impl<R> fmt::Debug for OldItem<R> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotRequested => f.write_str("NotRequested"),
            Self::Unavailable => f.write_str("Unavailable"),
            Self::Decoded(_) => f.write_str("Decoded([redacted])"),
            Self::Invalid { error, raw } => f
                .debug_struct("Invalid")
                .field("error", error)
                .field("raw", raw)
                .finish(),
        }
    }
}

pub struct Failure<R> {
    pub kind: FailureKind,
    pub old: OldItem<R>,
}
impl<R> fmt::Debug for Failure<R> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Failure")
            .field("kind", &self.kind)
            .field("old", &self.old)
            .finish()
    }
}

/// The write committed, but its returned full record could not be decoded.
/// All modeled output fields remain accessible, including accounting details.
pub struct WriteDecodeError {
    pub source: DecodeError,
    pub raw: Redacted<Item>,
    pub capacity: Option<ConsumedCapacity>,
    pub item_collection_metrics: Option<ItemCollectionMetrics>,
}
impl fmt::Debug for WriteDecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WriteDecodeError")
            .field("source", &self.source)
            .field("raw", &self.raw)
            .field("capacity", &"[redacted]")
            .field("item_collection_metrics", &"[redacted]")
            .finish()
    }
}
impl fmt::Display for WriteDecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "write committed; returned item: {}", self.source)
    }
}
impl Error for WriteDecodeError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(&self.source)
    }
}

#[non_exhaustive]
pub enum WriteError<R, E> {
    Refused {
        failure: Failure<R>,
        source: Box<SdkError<E>>,
    },
    Sdk(Box<SdkError<E>>),
    Decode(Box<WriteDecodeError>),
}
impl<R, E> WriteError<R, E> {
    /// Explicit SDK access; the original error may contain sensitive values.
    pub fn sdk_error(&self) -> Option<&SdkError<E>> {
        match self {
            Self::Refused { source, .. } | Self::Sdk(source) => Some(source),
            Self::Decode(_) => None,
        }
    }
}
impl<R, E> fmt::Debug for WriteError<R, E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Refused { failure, .. } => f
                .debug_struct("Refused")
                .field("kind", &failure.kind)
                .field("old", &failure.old)
                .field("source", &"[redacted]")
                .finish(),
            Self::Sdk(_) => f.write_str("Sdk([redacted])"),
            Self::Decode(error) => f.debug_tuple("Decode").field(error).finish(),
        }
    }
}
impl<R, E> fmt::Display for WriteError<R, E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Refused { failure, .. } => write!(f, "write refused: {:?}", failure.kind),
            Self::Sdk(_) => f.write_str("SDK write failed"),
            Self::Decode(error) => fmt::Display::fmt(error, f),
        }
    }
}
impl<R, E: Error + Send + Sync + 'static> Error for WriteError<R, E> {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Refused { source, .. } | Self::Sdk(source) => Some(source.as_ref()),
            Self::Decode(error) => Some(error.as_ref()),
        }
    }
}

// All three operations expose the same modeled refusal variants. Keep one
// classifier definition while retaining their exact SDK error types.
macro_rules! classify {
    ($($error:ident),+ $(,)?) => {$ (
        impl<R: Record + Document> WriteError<R, $error> {
            /// Classify only modeled condition, conflict, and throttle refusals.
            /// Performs no I/O or retries; unknown and transport errors stay SDK errors.
            pub fn from_sdk(error: SdkError<$error>, mode: OldItemMode) -> Self {
                let refusal = error.as_service_error().and_then(|service| {
                    let (kind, raw) = match service {
                        $error::ConditionalCheckFailedException(condition) =>
                            (FailureKind::ConditionFailed, condition.item()),
                        $error::TransactionConflictException(_) =>
                            (FailureKind::TransactionConflict, None),
                        $error::ReplicatedWriteConflictException(_) =>
                            (FailureKind::ReplicatedWriteConflict, None),
                        $error::ProvisionedThroughputExceededException(_) =>
                            (FailureKind::ProvisionedThroughputExceeded, None),
                        $error::ThrottlingException(_) => (FailureKind::Throttled, None),
                        $error::RequestLimitExceeded(_) => (FailureKind::RequestLimitExceeded, None),
                        _ => return None,
                    };
                    let old = OldItem::decode(mode, raw.filter(|_| mode == OldItemMode::All).cloned());
                    Some(Failure { kind, old })
                });
                match refusal {
                    Some(failure) => Self::Refused { failure, source: Box::new(error) },
                    None => Self::Sdk(Box::new(error)),
                }
            }
        }
    )+};
}
classify!(PutItemError, DeleteItemError, UpdateItemError);
