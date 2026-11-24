//! One explicit BatchGetItem round across several table bindings. No client, transport, or retries.
use std::{error::Error, fmt, sync::Arc};

use aws_sdk_dynamodb::{
    error::SdkError,
    operation::batch_get_item::{
        BatchGetItemError, BatchGetItemInput, BatchGetItemOutput,
        builders::BatchGetItemFluentBuilder,
    },
    types::{ConsumedCapacity, KeysAndAttributes},
};

use crate::{
    BatchGetState, BuildError, Capacity, DecodeContext, DecodeError, Document, Item, Projection,
    ReadConsistency, Record, RecordKey, RenderedExpressions, Table,
};

type ResponseKey = fn(&Item, bool) -> Result<Item, BuildError>;

fn constraint(code: &'static str) -> BuildError {
    BuildError::OperationConstraint { code }
}

struct TableEntry {
    table: String,
    attributes: KeysAndAttributes,
    expressions: RenderedExpressions,
    key: ResponseKey,
}

/// One table's reads in a [`MultiBatchGet`]. It decodes only its own key and
/// record or projection types and is accepted only by its own request's responses.
pub struct BatchGetSlot<K, P> {
    identity: Arc<()>,
    table: usize,
    decode_item: fn(&Item) -> Result<P, DecodeError>,
    decode_key: fn(&Item) -> Result<K, DecodeError>,
}
impl<K, P> Clone for BatchGetSlot<K, P> {
    fn clone(&self) -> Self {
        Self {
            identity: Arc::clone(&self.identity),
            table: self.table,
            decode_item: self.decode_item,
            decode_key: self.decode_key,
        }
    }
}
impl<K, P> fmt::Debug for BatchGetSlot<K, P> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BatchGetSlot")
            .field("table", &self.table)
            .finish_non_exhaustive()
    }
}

/// Collects keyed reads from several distinct tables into one BatchGetItem request.
/// Every fallible step is reported by `build`; adding a table never fails.
/// It retains at most 100 keys and the first fault, whatever the caller adds.
pub struct MultiBatchGet {
    identity: Arc<()>,
    tables: Vec<TableEntry>,
    /// The first fault in insertion order; later tables are not prepared.
    fault: Option<BuildError>,
    /// Tables added so far, including any that were not prepared.
    added: usize,
    /// Keys supplied so far, including tables that were not prepared.
    requested: usize,
    capacity: Capacity,
}
impl Default for MultiBatchGet {
    fn default() -> Self {
        Self::new()
    }
}
impl MultiBatchGet {
    pub fn new() -> Self {
        Self {
            identity: Arc::new(()),
            tables: Vec::new(),
            fault: None,
            added: 0,
            requested: 0,
            capacity: Capacity::None,
        }
    }

    /// Read complete records from one table with its own explicit consistency.
    pub fn get<R: Record + Document, K: RecordKey<R>>(
        &mut self,
        table: &Table<R>,
        keys: Vec<K>,
        consistency: ReadConsistency,
    ) -> BatchGetSlot<K, R> {
        self.push(table, &keys, consistency, None, |item| {
            R::decode_map(item, &mut DecodeContext::new(R::NAME))
        })
    }

    /// Read a typed projection from one table. The request also selects any
    /// omitted physical primary-key attributes so returned items can be correlated.
    pub fn get_projected<V, K>(
        &mut self,
        table: &Table<V::Source>,
        keys: Vec<K>,
        consistency: ReadConsistency,
    ) -> BatchGetSlot<K, V>
    where
        V: Projection,
        V::Source: Record,
        K: RecordKey<V::Source>,
    {
        self.push(
            table,
            &keys,
            consistency,
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
        keys: &[K],
        consistency: ReadConsistency,
        selection: Option<fn() -> Result<RenderedExpressions, BuildError>>,
        decode_item: fn(&Item) -> Result<P, DecodeError>,
    ) -> BatchGetSlot<K, P> {
        let position = self.added;
        self.added = self.added.saturating_add(1);
        self.requested = self.requested.saturating_add(keys.len());
        let slot = BatchGetSlot {
            identity: Arc::clone(&self.identity),
            table: position,
            decode_item,
            decode_key: K::decode_key,
        };
        // Build refuses an over-cap or already faulty request before any table:
        // never encode, compare or retain keys it can never send.
        if self.requested > 100 || self.fault.is_some() {
            return slot;
        }
        let entry = (|| {
            if keys.is_empty() {
                return Err(BuildError::EmptyOperation);
            }
            let mut encoded = Vec::with_capacity(keys.len());
            for key in keys {
                let key = crate::batch_get::canonical_key::<R>(key.encode_key()?)?;
                if encoded.contains(&key) {
                    return Err(constraint("duplicate_batch_key"));
                }
                encoded.push(key);
            }
            let expressions = selection
                .map(|render| render())
                .transpose()?
                .unwrap_or_default();
            let names = (!expressions.names().is_empty()).then(|| expressions.names().clone());
            let attributes = KeysAndAttributes::builder()
                .set_keys(Some(encoded))
                .consistent_read(consistency == ReadConsistency::Strong)
                .set_projection_expression(expressions.projection.clone())
                .set_expression_attribute_names(names)
                .build()
                .map_err(|_| constraint("invalid_batch_get_input"))?;
            Ok(TableEntry {
                table: table.name.clone(),
                attributes,
                expressions,
                key: crate::batch_get_adapter::response_key::<R>,
            })
        })();
        match entry {
            Ok(entry) => self.tables.push(entry),
            Err(fault) => self.fault = Some(fault),
        }
        slot
    }

    /// Require at least one table and at most 100 keys in total, then validate
    /// every table's keys and projection in insertion order, then distinct tables.
    pub fn build(self) -> Result<PreparedMultiBatchGet, BuildError> {
        if self.added == 0 {
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
        let tables = self.tables;
        let mut location = None;
        for (i, table) in tables.iter().enumerate() {
            // One regional endpoint serves the request; accounts may differ.
            if let (Some((partition, region, _)), _) =
                crate::transaction::table_identity(&table.table)
            {
                if location.is_some_and(|previous| previous != (partition, region)) {
                    return Err(constraint("batch_table_location"));
                }
                location = Some((partition, region));
            }
            if tables[..i]
                .iter()
                .any(|other| same_table(&other.table, &table.table))
            {
                return Err(constraint("duplicate_batch_table"));
            }
        }
        let mut builder =
            BatchGetItemInput::builder().set_return_consumed_capacity(self.capacity.sdk());
        let (mut prepared, mut expressions) = (Vec::new(), Vec::new());
        for entry in tables {
            prepared.push(PreparedTable {
                keys: entry.attributes.keys.clone(),
                table: entry.table.clone(),
                key: entry.key,
            });
            builder = builder.request_items(entry.table, entry.attributes);
            expressions.push(entry.expressions);
        }
        let input = builder
            .build()
            .map_err(|_| constraint("invalid_batch_get_input"))?;
        Ok(PreparedMultiBatchGet {
            input,
            tables: prepared,
            expressions,
            identity: self.identity,
        })
    }
}
/// Two explicit ARNs name one table only in the same partition, region and account;
/// a bare name is an alias of any ARN with that table name.
fn same_table(a: &str, b: &str) -> bool {
    let (a_scope, a_name) = crate::transaction::table_identity(a);
    let (b_scope, b_name) = crate::transaction::table_identity(b);
    a_name == b_name && (a_scope.is_none() || b_scope.is_none() || a_scope == b_scope)
}
impl fmt::Debug for MultiBatchGet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("MultiBatchGet([redacted])")
    }
}

struct PreparedTable {
    table: String,
    keys: Vec<Item>,
    key: ResponseKey,
}

/// One immutable BatchGetItem request; never split and never resubmitted by tabulary.
pub struct PreparedMultiBatchGet {
    input: BatchGetItemInput,
    tables: Vec<PreparedTable>,
    expressions: Vec<RenderedExpressions>,
    identity: Arc<()>,
}
impl PreparedMultiBatchGet {
    /// Explicit raw SDK access; formatting this input may reveal key values.
    pub fn input(&self) -> &BatchGetItemInput {
        &self.input
    }
    /// One projection binding per table, in insertion order (empty for full records).
    pub fn expressions(&self) -> &[RenderedExpressions] {
        &self.expressions
    }
    /// Replace every modeled request field; transport configuration is untouched.
    pub fn apply(&self, builder: BatchGetItemFluentBuilder) -> BatchGetItemFluentBuilder {
        builder
            .set_request_items(self.input.request_items.clone())
            .set_return_consumed_capacity(self.input.return_consumed_capacity.clone())
    }
    /// Correlate a caller-supplied SDK result by full physical keys, independent of
    /// response order. A foreign table, unexpected or malformed key, duplicate, or
    /// key both returned and unprocessed rejects the whole response.
    pub fn decode(
        &self,
        result: Result<BatchGetItemOutput, SdkError<BatchGetItemError>>,
    ) -> Result<MultiBatchGetResponse, MultiBatchGetError> {
        let output = result.map_err(|error| MultiBatchGetError::Sdk(Box::new(error)))?;
        let capacity = output.consumed_capacity;
        let mut responses = output.responses.unwrap_or_default();
        let mut unprocessed = output.unprocessed_keys.unwrap_or_default();
        if responses
            .keys()
            .chain(unprocessed.keys())
            .any(|name| !self.tables.iter().any(|table| &table.table == name))
        {
            return Err(MultiBatchGetError::InvalidResponse { capacity });
        }
        let mut tables = Vec::with_capacity(self.tables.len());
        for table in &self.tables {
            let pending = unprocessed
                .remove(&table.table)
                .map(|attributes| attributes.keys)
                .unwrap_or_default();
            let found = responses.remove(&table.table).unwrap_or_default();
            match table.correlate(pending, found) {
                Some(states) => tables.push(states),
                None => return Err(MultiBatchGetError::InvalidResponse { capacity }),
            }
        }
        Ok(MultiBatchGetResponse {
            tables: self
                .tables
                .iter()
                .zip(tables)
                .map(|(table, states)| ResponseTable {
                    keys: table.keys.clone(),
                    states,
                })
                .collect(),
            capacity,
            identity: Arc::clone(&self.identity),
        })
    }
}
impl PreparedTable {
    fn correlate(&self, pending: Vec<Item>, found: Vec<Item>) -> Option<Vec<BatchGetState<Item>>> {
        let mut states: Vec<_> = self.keys.iter().map(|_| BatchGetState::Missing).collect();
        let index = |raw: &Item, key_only, states: &[BatchGetState<Item>]| {
            let key = (self.key)(raw, key_only).ok()?;
            let index = self.keys.iter().position(|issued| issued == &key)?;
            matches!(states[index], BatchGetState::Missing).then_some(index)
        };
        for key in pending {
            let i = index(&key, true, &states)?;
            states[i] = BatchGetState::Pending;
        }
        for item in found {
            let i = index(&item, false, &states)?;
            states[i] = BatchGetState::Found(item);
        }
        Some(states)
    }
}
impl fmt::Debug for PreparedMultiBatchGet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PreparedMultiBatchGet")
            .field("input", &"[redacted]")
            .field("expressions", &self.expressions)
            .finish()
    }
}

struct ResponseTable {
    keys: Vec<Item>,
    states: Vec<BatchGetState<Item>>,
}

/// A correlated response. Each table decodes on access through its slot.
pub struct MultiBatchGetResponse {
    tables: Vec<ResponseTable>,
    identity: Arc<()>,
    /// The SDK's capacity vector, unchanged.
    pub capacity: Option<Vec<ConsumedCapacity>>,
}
impl MultiBatchGetResponse {
    /// Strictly decode one table's returned items and its missing and unprocessed keys.
    /// Any malformed item or key fails the whole table, listing every failed position.
    pub fn get<K, P>(
        &self,
        slot: &BatchGetSlot<K, P>,
    ) -> Result<BatchGetTableResult<K, P>, BatchGetSlotError> {
        if !Arc::ptr_eq(&self.identity, &slot.identity) {
            return Err(BatchGetSlotError::Foreign);
        }
        let table = self
            .tables
            .get(slot.table)
            .ok_or(BatchGetSlotError::Foreign)?;
        let mut result = BatchGetTableResult {
            found: Vec::new(),
            missing: Vec::new(),
            unprocessed: Vec::new(),
        };
        let mut failures = Vec::new();
        for (position, (key, state)) in table.keys.iter().zip(&table.states).enumerate() {
            let outcome = match state {
                BatchGetState::Found(item) => {
                    (slot.decode_item)(item).map(|v| result.found.push(v))
                }
                BatchGetState::Missing => (slot.decode_key)(key).map(|k| result.missing.push(k)),
                BatchGetState::Pending => {
                    (slot.decode_key)(key).map(|k| result.unprocessed.push(k))
                }
            };
            if let Err(error) = outcome {
                failures.push((position, error));
            }
        }
        if !failures.is_empty() {
            return Err(BatchGetSlotError::Decode { failures });
        }
        Ok(result)
    }
}
impl fmt::Debug for MultiBatchGetResponse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MultiBatchGetResponse")
            .field("tables", &"[redacted]")
            .field("capacity", &self.capacity)
            .finish()
    }
}

/// One table's typed outcome, each list in the table's original key order.
/// Unprocessed keys are never missing; the caller decides whether to issue them again.
pub struct BatchGetTableResult<K, P> {
    pub found: Vec<P>,
    pub missing: Vec<K>,
    pub unprocessed: Vec<K>,
}
impl<K, P> fmt::Debug for BatchGetTableResult<K, P> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BatchGetTableResult")
            .field("found", &self.found.len())
            .field("missing", &self.missing.len())
            .field("unprocessed", &self.unprocessed.len())
            .finish()
    }
}

/// A table-specific failure; other tables of the same response are unaffected.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BatchGetSlotError {
    /// The slot was issued by a different request.
    Foreign,
    /// Every malformed item or key, as its position in the table's key list.
    Decode { failures: Vec<(usize, DecodeError)> },
}
impl fmt::Display for BatchGetSlotError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Foreign => f.write_str("slot belongs to a different request"),
            Self::Decode { failures } => {
                f.write_str("batch table decode failed")?;
                for (position, source) in failures {
                    write!(f, "; key {position}: {source}")?;
                }
                Ok(())
            }
        }
    }
}
impl Error for BatchGetSlotError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Foreign => None,
            Self::Decode { failures } => failures.first().map(|(_, source)| source as &dyn Error),
        }
    }
}

/// Whole-request failures. A protocol fault retains the received capacity.
#[non_exhaustive]
pub enum MultiBatchGetError {
    Sdk(Box<SdkError<BatchGetItemError>>),
    InvalidResponse {
        capacity: Option<Vec<ConsumedCapacity>>,
    },
}
impl MultiBatchGetError {
    /// Explicit SDK access; formatting the original error may reveal payloads.
    pub fn sdk_error(&self) -> Option<&SdkError<BatchGetItemError>> {
        match self {
            Self::Sdk(error) => Some(error),
            Self::InvalidResponse { .. } => None,
        }
    }
}
impl fmt::Debug for MultiBatchGetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Sdk(_) => f.write_str("Sdk([redacted])"),
            Self::InvalidResponse { capacity } => f
                .debug_struct("InvalidResponse")
                .field("capacity", capacity)
                .finish(),
        }
    }
}
impl fmt::Display for MultiBatchGetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Sdk(_) => f.write_str("SDK batch read failed"),
            Self::InvalidResponse { .. } => {
                f.write_str("batch response does not match its request")
            }
        }
    }
}
impl Error for MultiBatchGetError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Sdk(error) => Some(error.as_ref()),
            Self::InvalidResponse { .. } => None,
        }
    }
}
