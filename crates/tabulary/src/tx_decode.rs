//! Typed transaction outcomes; only machine reason codes determine refusal kinds.
use std::{collections::HashMap, error::Error, fmt};

use crate::{Failure, FailureKind, OldItem, PreparedTransaction, Redacted, TxAction, TxActions};
use aws_sdk_dynamodb::{
    error::SdkError,
    operation::transact_write_items::{TransactWriteItemsError, TransactWriteItemsOutput},
    types::{
        CancellationReason, ConsumedCapacity, ItemCollectionMetrics,
        ReturnValuesOnConditionCheckFailure,
    },
};

/// Whether the endpoint supplied a positionally trustworthy reason vector.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReasonCompleteness {
    Complete,
    Missing,
    MalformedLength { expected: usize, actual: usize },
}

pub struct Cancellation<C> {
    pub reasons: C,
    pub completeness: ReasonCompleteness,
    /// Explicit SDK access may reveal messages, raw items, and response data.
    pub source: Box<SdkError<TransactWriteItemsError>>,
}
impl<C> Cancellation<C> {
    /// Preserve absent versus present-empty vectors without guessing alignment.
    pub fn raw_reasons(&self) -> Option<&[CancellationReason]> {
        match self.source.as_service_error() {
            Some(TransactWriteItemsError::TransactionCanceledException(error)) => {
                error.cancellation_reasons.as_deref()
            }
            _ => None,
        }
    }
}
impl<C> fmt::Debug for Cancellation<C> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Cancellation")
            .field("completeness", &self.completeness)
            .field("reasons", &"[redacted]")
            .field("source", &"[redacted]")
            .finish()
    }
}
impl<C> fmt::Display for Cancellation<C> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "transaction cancelled: {:?}", self.completeness)
    }
}
impl<C> Error for Cancellation<C> {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(self.source.as_ref())
    }
}

#[non_exhaustive]
pub enum TxError<C> {
    Cancelled(Cancellation<C>),
    /// Includes transport failures whose commit outcome can be indeterminate.
    Sdk(Box<SdkError<TransactWriteItemsError>>),
}
impl<C> TxError<C> {
    /// Explicit SDK access; formatting the original error may reveal payloads.
    pub fn sdk_error(&self) -> &SdkError<TransactWriteItemsError> {
        match self {
            Self::Cancelled(error) => &error.source,
            Self::Sdk(error) => error,
        }
    }
}
impl<C> fmt::Debug for TxError<C> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cancelled(error) => f.debug_tuple("Cancelled").field(error).finish(),
            Self::Sdk(_) => f.write_str("Sdk([redacted])"),
        }
    }
}
impl<C> fmt::Display for TxError<C> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cancelled(error) => fmt::Display::fmt(error, f),
            Self::Sdk(_) => f.write_str("SDK transaction failed"),
        }
    }
}
impl<C> Error for TxError<C> {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(self.sdk_error())
    }
}

/// A committed transaction's accounting, unchanged from the SDK output.
pub struct TxResult {
    pub capacity: Option<Vec<ConsumedCapacity>>,
    pub item_collection_metrics: Option<HashMap<String, Vec<ItemCollectionMetrics>>>,
}
impl fmt::Debug for TxResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TxResult")
            .field("capacity", &"[redacted]")
            .field("item_collection_metrics", &"[redacted]")
            .finish()
    }
}

impl<A: TxActions> PreparedTransaction<A> {
    /// Decode a caller-supplied SDK result without I/O, retries, or message parsing.
    pub fn decode(
        &self,
        result: Result<TransactWriteItemsOutput, SdkError<TransactWriteItemsError>>,
    ) -> Result<TxResult, TxError<A::Reasons>> {
        match result {
            Ok(output) => Ok(TxResult {
                capacity: output.consumed_capacity,
                item_collection_metrics: output.item_collection_metrics,
            }),
            Err(source) => {
                let Some(TransactWriteItemsError::TransactionCanceledException(error)) =
                    source.as_service_error()
                else {
                    return Err(TxError::Sdk(Box::new(source)));
                };
                let expected = self.input().transact_items().len();
                let (completeness, reasons) = match error.cancellation_reasons.as_deref() {
                    None => (ReasonCompleteness::Missing, &[][..]),
                    Some(raw) if raw.len() != expected => (
                        ReasonCompleteness::MalformedLength {
                            expected,
                            actual: raw.len(),
                        },
                        &[][..],
                    ),
                    Some(raw) => (ReasonCompleteness::Complete, raw),
                };
                let reasons = self.actions().decode_reasons(&mut reasons.iter());
                Err(TxError::Cancelled(Cancellation {
                    reasons,
                    completeness,
                    source: Box::new(source),
                }))
            }
        }
    }
}

impl<R> TxAction<R> {
    pub(crate) fn decode_reason(&self, reason: Option<&CancellationReason>) -> Failure<R> {
        let kind = match reason.map(|reason| reason.code()) {
            None => FailureKind::Unavailable,
            Some(None | Some("None")) => FailureKind::NoFailureReported,
            Some(Some("ConditionalCheckFailed")) => FailureKind::ConditionFailed,
            Some(Some("TransactionConflict")) => FailureKind::TransactionConflict,
            Some(Some("ProvisionedThroughputExceeded")) => {
                FailureKind::ProvisionedThroughputExceeded
            }
            Some(Some("ThrottlingError")) => FailureKind::Throttled,
            Some(Some("ItemCollectionSizeLimitExceeded")) => {
                FailureKind::ItemCollectionSizeLimitExceeded
            }
            Some(Some("ValidationError")) => FailureKind::Validation,
            Some(Some(code)) => FailureKind::UnknownServiceCode(code.into()),
        };
        let input = self.input();
        let requested = [
            input
                .put
                .as_ref()
                .and_then(|a| a.return_values_on_condition_check_failure.as_ref()),
            input
                .update
                .as_ref()
                .and_then(|a| a.return_values_on_condition_check_failure.as_ref()),
            input
                .delete
                .as_ref()
                .and_then(|a| a.return_values_on_condition_check_failure.as_ref()),
            input
                .condition_check
                .as_ref()
                .and_then(|a| a.return_values_on_condition_check_failure.as_ref()),
        ]
        .contains(&Some(&ReturnValuesOnConditionCheckFailure::AllOld));
        let old = if !requested {
            OldItem::NotRequested
        } else if let Some(raw) = reason.and_then(|reason| reason.item.as_ref()) {
            match (self.prepared.decode_item)(raw) {
                Ok(record) => OldItem::Decoded(record),
                Err(error) => OldItem::Invalid {
                    error,
                    raw: Redacted::new(raw.clone()),
                },
            }
        } else {
            OldItem::Unavailable
        };
        Failure { kind, old }
    }
}
