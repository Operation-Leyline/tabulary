//! Typed TransactGetItems: positional slots bound to one request; execution belongs to the caller.
use std::{error::Error, fmt, sync::Arc};

use aws_sdk_dynamodb::{
    error::SdkError,
    operation::transact_get_items::{
        TransactGetItemsError, TransactGetItemsInput, TransactGetItemsOutput,
        builders::TransactGetItemsFluentBuilder,
    },
    types::{CancellationReason, ConsumedCapacity, Get, TransactGetItem},
};

use crate::{
    BuildError, Capacity, DecodeContext, DecodeError, Document, FailureKind, Item, Projection,
    ReasonCompleteness, Record, RecordKey, RenderedExpressions, Table,
};

type ResponseKey = fn(&Item, bool) -> Result<Item, BuildError>;

fn constraint(code: &'static str) -> BuildError {
    BuildError::OperationConstraint { code }
}

struct Entry {
    item: TransactGetItem,
    expressions: RenderedExpressions,
    target: (String, Item),
    key: ResponseKey,
}

/// One read position in a [`TransactGet`]. It decodes only its own record or
/// projection type and is accepted only by responses of the request that issued it.
pub struct GetSlot<P> {
    identity: Arc<()>,
    position: usize,
    decode: fn(&Item) -> Result<P, DecodeError>,
}
impl<P> GetSlot<P> {
    /// Zero-based position in the request's `TransactItems`.
    pub fn position(&self) -> usize {
        self.position
    }
}
impl<P> Clone for GetSlot<P> {
    fn clone(&self) -> Self {
        Self {
            identity: Arc::clone(&self.identity),
            position: self.position,
            decode: self.decode,
        }
    }
}
impl<P> fmt::Debug for GetSlot<P> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GetSlot")
            .field("position", &self.position)
            .finish_non_exhaustive()
    }
}

/// Collects keyed reads from any record types and table bindings.
/// Every fallible step is reported by `build`; adding a read never fails.
/// It retains at most 100 entries and the first fault, whatever the caller adds.
pub struct TransactGet {
    identity: Arc<()>,
    entries: Vec<Entry>,
    /// The first fault in request order; later reads are not prepared.
    fault: Option<BuildError>,
    /// Reads added so far, including any that were not prepared.
    requested: usize,
    capacity: Capacity,
}
impl Default for TransactGet {
    fn default() -> Self {
        Self::new()
    }
}
impl TransactGet {
    pub fn new() -> Self {
        Self {
            identity: Arc::new(()),
            entries: Vec::new(),
            fault: None,
            requested: 0,
            capacity: Capacity::None,
        }
    }

    /// Read one complete record.
    pub fn get<R: Record + Document, K: RecordKey<R>>(
        &mut self,
        table: &Table<R>,
        key: K,
    ) -> GetSlot<R> {
        self.push(table, &key, None, |item| {
            R::decode_map(item, &mut DecodeContext::new(R::NAME))
        })
    }

    /// Read one typed projection. The request also selects any omitted
    /// physical primary-key attributes so the response can be verified.
    pub fn get_projected<V, K>(&mut self, table: &Table<V::Source>, key: K) -> GetSlot<V>
    where
        V: Projection,
        V::Source: Record,
        K: RecordKey<V::Source>,
    {
        self.push(
            table,
            &key,
            Some(crate::batch_get_adapter::projection_for::<V>),
            V::from_item,
        )
    }

    /// Request-wide ReturnConsumedCapacity; `Capacity::None` omits it.
    pub fn set_capacity(&mut self, capacity: Capacity) {
        self.capacity = capacity;
    }

    fn push<R: Record, K: RecordKey<R>, P>(
        &mut self,
        table: &Table<R>,
        key: &K,
        selection: Option<fn() -> Result<RenderedExpressions, BuildError>>,
        decode: fn(&Item) -> Result<P, DecodeError>,
    ) -> GetSlot<P> {
        let position = self.requested;
        self.requested = self.requested.saturating_add(1);
        let slot = GetSlot {
            identity: Arc::clone(&self.identity),
            position,
            decode,
        };
        // Build refuses an over-cap or already faulty request before any entry:
        // retain and prepare nothing more for it.
        if self.requested > 100 || self.fault.is_some() {
            return slot;
        }
        let entry = (|| {
            let key = crate::batch_get::canonical_key::<R>(key.encode_key()?)?;
            let expressions = selection
                .map(|render| render())
                .transpose()?
                .unwrap_or_default();
            let names = (!expressions.names().is_empty()).then(|| expressions.names().clone());
            let get = Get::builder()
                .table_name(table.name.clone())
                .set_key(Some(key.clone()))
                .set_projection_expression(expressions.projection.clone())
                .set_expression_attribute_names(names)
                .build()
                .map_err(|_| constraint("invalid_transact_get_input"))?;
            Ok(Entry {
                item: TransactGetItem::builder().get(get).build(),
                expressions,
                target: (table.name.clone(), key),
                key: crate::batch_get_adapter::response_key::<R>,
            })
        })();
        match entry {
            Ok(entry) => self.entries.push(entry),
            Err(fault) => self.fault = Some(fault),
        }
        slot
    }

    /// Validate 1..=100 reads, then every key and projection in request order,
    /// then same-location tables and distinct physical targets.
    pub fn build(self) -> Result<PreparedTransactGet, BuildError> {
        if self.requested == 0 {
            return Err(BuildError::EmptyOperation);
        }
        if self.requested > 100 {
            return Err(BuildError::RequestLimit {
                max: 100,
                actual: self.requested,
            });
        }
        if let Some(fault) = self.fault {
            return Err(fault);
        }
        let entries = self.entries;
        let targets: Vec<_> = entries.iter().map(|entry| entry.target.clone()).collect();
        crate::transaction::validate_targets(&targets)?;
        let (mut items, mut expressions, mut keys) = (Vec::new(), Vec::new(), Vec::new());
        for entry in entries {
            items.push(entry.item);
            expressions.push(entry.expressions);
            keys.push(entry.key);
        }
        let input = TransactGetItemsInput::builder()
            .set_transact_items(Some(items))
            .set_return_consumed_capacity(self.capacity.sdk())
            .build()
            .map_err(|_| constraint("invalid_transact_get_input"))?;
        Ok(PreparedTransactGet {
            input,
            expressions,
            keys,
            identity: self.identity,
        })
    }
}
impl fmt::Debug for TransactGet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("TransactGet([redacted])")
    }
}

/// One immutable TransactGetItems request; never split.
pub struct PreparedTransactGet {
    input: TransactGetItemsInput,
    expressions: Vec<RenderedExpressions>,
    keys: Vec<ResponseKey>,
    identity: Arc<()>,
}
impl PreparedTransactGet {
    /// Explicit raw SDK access; formatting this input may reveal key values.
    pub fn input(&self) -> &TransactGetItemsInput {
        &self.input
    }
    /// One projection binding per read, in request order (empty for full records).
    pub fn expressions(&self) -> &[RenderedExpressions] {
        &self.expressions
    }
    /// Replace every modeled request field; transport configuration is untouched.
    pub fn apply(&self, builder: TransactGetItemsFluentBuilder) -> TransactGetItemsFluentBuilder {
        builder
            .set_transact_items(self.input.transact_items.clone())
            .set_return_consumed_capacity(self.input.return_consumed_capacity.clone())
    }
    /// Decode a caller-supplied SDK result without I/O, retries, or message parsing.
    /// The response must align one-to-one with the request, and every returned
    /// item must carry exactly the requested physical key; otherwise nothing is trusted.
    pub fn decode(
        &self,
        result: Result<TransactGetItemsOutput, SdkError<TransactGetItemsError>>,
    ) -> Result<TransactGetResponse, TransactGetError> {
        let output = match result {
            Ok(output) => output,
            Err(source) => {
                let Some(TransactGetItemsError::TransactionCanceledException(error)) =
                    source.as_service_error()
                else {
                    return Err(TransactGetError::Sdk(Box::new(source)));
                };
                let expected = self.keys.len();
                let (completeness, raw) =
                    crate::tx_decode::completeness(error.cancellation_reasons.as_deref(), expected);
                let reasons = (0..expected)
                    .map(|i| crate::tx_decode::failure_kind(raw.get(i)))
                    .collect();
                return Err(TransactGetError::Cancelled(TransactGetCancellation {
                    reasons,
                    completeness,
                    source: Box::new(source),
                    identity: Arc::clone(&self.identity),
                }));
            }
        };
        let capacity = output.consumed_capacity;
        let expected = self.keys.len();
        let responses = match output.responses {
            Some(responses) if responses.len() == expected => responses,
            responses => {
                return Err(TransactGetError::ResponseCount {
                    expected,
                    actual: responses.map(|responses| responses.len()),
                    capacity,
                });
            }
        };
        let mut items = Vec::with_capacity(expected);
        for (position, response) in responses.into_iter().enumerate() {
            if let Some(raw) = &response.item {
                let requested = self
                    .input
                    .transact_items
                    .as_deref()
                    .and_then(|items| items[position].get.as_ref().map(|get| &get.key));
                let returned = (self.keys[position])(raw, false).ok();
                if returned.as_ref() != requested {
                    return Err(TransactGetError::KeyMismatch { position, capacity });
                }
            }
            items.push(response.item);
        }
        Ok(TransactGetResponse {
            items,
            capacity,
            identity: Arc::clone(&self.identity),
        })
    }
}
impl fmt::Debug for PreparedTransactGet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PreparedTransactGet")
            .field("input", &"[redacted]")
            .field("expressions", &self.expressions)
            .finish()
    }
}

/// A verified, positionally aligned response. Items decode on access through their slots.
pub struct TransactGetResponse {
    items: Vec<Option<Item>>,
    identity: Arc<()>,
    /// The SDK's capacity vector, unchanged.
    pub capacity: Option<Vec<ConsumedCapacity>>,
}
impl TransactGetResponse {
    /// `None` means the service found no item. A present item is decoded strictly.
    pub fn get<P>(&self, slot: &GetSlot<P>) -> Result<Option<P>, GetSlotError> {
        if !Arc::ptr_eq(&self.identity, &slot.identity) {
            return Err(GetSlotError::Foreign);
        }
        let item = self.items.get(slot.position).ok_or(GetSlotError::Foreign)?;
        item.as_ref()
            .map(slot.decode)
            .transpose()
            .map_err(GetSlotError::Decode)
    }
    /// Explicit raw payload access in request order, including any secret attributes.
    pub fn raw_items(&self) -> &[Option<Item>] {
        &self.items
    }
}
impl fmt::Debug for TransactGetResponse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TransactGetResponse")
            .field("items", &"[redacted]")
            .field("capacity", &self.capacity)
            .finish()
    }
}

/// A slot-specific failure; other slots of the same response are unaffected.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GetSlotError {
    /// The slot was issued by a different request.
    Foreign,
    Decode(DecodeError),
}
impl fmt::Display for GetSlotError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Foreign => f.write_str("slot belongs to a different request"),
            Self::Decode(error) => fmt::Display::fmt(error, f),
        }
    }
}
impl Error for GetSlotError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Foreign => None,
            Self::Decode(error) => Some(error),
        }
    }
}

/// A cancelled read transaction with one positional reason per slot.
pub struct TransactGetCancellation {
    reasons: Vec<FailureKind>,
    pub completeness: ReasonCompleteness,
    /// Explicit SDK access may reveal messages and response data.
    pub source: Box<SdkError<TransactGetItemsError>>,
    identity: Arc<()>,
}
impl TransactGetCancellation {
    /// This slot's reason; `Unavailable` unless the reason vector was complete.
    pub fn reason<P>(&self, slot: &GetSlot<P>) -> Result<&FailureKind, GetSlotError> {
        if !Arc::ptr_eq(&self.identity, &slot.identity) {
            return Err(GetSlotError::Foreign);
        }
        self.reasons.get(slot.position).ok_or(GetSlotError::Foreign)
    }
    /// Every reason in request order.
    pub fn reasons(&self) -> &[FailureKind] {
        &self.reasons
    }
    /// Preserve absent versus present-empty vectors without guessing alignment.
    pub fn raw_reasons(&self) -> Option<&[CancellationReason]> {
        match self.source.as_service_error() {
            Some(TransactGetItemsError::TransactionCanceledException(error)) => {
                error.cancellation_reasons.as_deref()
            }
            _ => None,
        }
    }
}
impl fmt::Debug for TransactGetCancellation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TransactGetCancellation")
            .field("completeness", &self.completeness)
            .field("reasons", &self.reasons)
            .field("source", &"[redacted]")
            .finish()
    }
}
impl fmt::Display for TransactGetCancellation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "read transaction cancelled: {:?}", self.completeness)
    }
}
impl Error for TransactGetCancellation {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(self.source.as_ref())
    }
}

/// Whole-request failures. Protocol faults retain the received capacity.
#[non_exhaustive]
pub enum TransactGetError {
    Cancelled(TransactGetCancellation),
    Sdk(Box<SdkError<TransactGetItemsError>>),
    /// The response vector is absent or does not align with the request.
    ResponseCount {
        expected: usize,
        actual: Option<usize>,
        capacity: Option<Vec<ConsumedCapacity>>,
    },
    /// A returned item's physical key is absent, malformed, or not the requested key.
    KeyMismatch {
        position: usize,
        capacity: Option<Vec<ConsumedCapacity>>,
    },
}
impl TransactGetError {
    /// Explicit SDK access; formatting the original error may reveal payloads.
    pub fn sdk_error(&self) -> Option<&SdkError<TransactGetItemsError>> {
        match self {
            Self::Cancelled(error) => Some(&error.source),
            Self::Sdk(error) => Some(error),
            Self::ResponseCount { .. } | Self::KeyMismatch { .. } => None,
        }
    }
}
impl fmt::Debug for TransactGetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cancelled(error) => f.debug_tuple("Cancelled").field(error).finish(),
            Self::Sdk(_) => f.write_str("Sdk([redacted])"),
            Self::ResponseCount {
                expected,
                actual,
                capacity,
            } => f
                .debug_struct("ResponseCount")
                .field("expected", expected)
                .field("actual", actual)
                .field("capacity", capacity)
                .finish(),
            Self::KeyMismatch { position, capacity } => f
                .debug_struct("KeyMismatch")
                .field("position", position)
                .field("capacity", capacity)
                .finish(),
        }
    }
}
impl fmt::Display for TransactGetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cancelled(error) => fmt::Display::fmt(error, f),
            Self::Sdk(_) => f.write_str("SDK read transaction failed"),
            Self::ResponseCount {
                expected, actual, ..
            } => write!(f, "expected {expected} item responses, received {actual:?}"),
            Self::KeyMismatch { position, .. } => {
                write!(
                    f,
                    "item response {position} does not match its requested key"
                )
            }
        }
    }
}
impl Error for TransactGetError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Cancelled(error) => Some(error),
            Self::Sdk(error) => Some(error.as_ref()),
            Self::ResponseCount { .. } | Self::KeyMismatch { .. } => None,
        }
    }
}
