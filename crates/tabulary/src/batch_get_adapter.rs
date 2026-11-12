//! Explicit, caller-driven BatchGet rounds. No client, transport, or retries.
use crate::{
    BatchBudget, BatchGetChunk, BatchGetComplete, BatchGetPartial, BatchGetProgress,
    BatchStateError, BuildError, Capacity, DecodeContext, DecodeError, Document, Item,
    PreparedRequest, ReadConsistency, Record, RecordKey, RenderedExpressions, Table,
};
use aws_sdk_dynamodb::{
    error::SdkError,
    operation::batch_get_item::{
        BatchGetItemError, BatchGetItemInput, BatchGetItemOutput,
        builders::BatchGetItemFluentBuilder,
    },
    types::{ConsumedCapacity, KeysAndAttributes},
};
use std::{error::Error, fmt};

#[non_exhaustive]
pub enum BatchGetFailure {
    Aborted(BatchRoundAbort),
    State(BatchStateError),
    Sdk(Box<SdkError<BatchGetItemError>>),
    Decode {
        position: usize,
        source: DecodeError,
    },
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BatchRoundAbort {
    Cancelled,
    Deadline,
}
pub struct BatchGetError<K, P> {
    pub kind: BatchGetFailure,
    pub partial: Box<BatchGetPartial<K, P>>,
}
impl fmt::Display for BatchGetFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Aborted(cause) => write!(f, "batch round aborted: {cause:?}"),
            Self::State(error) => error.fmt(f),
            Self::Sdk(_) => f.write_str("SDK batch read failed"),
            Self::Decode { position, source } => write!(f, "item {position}: {source}"),
        }
    }
}
impl Error for BatchGetFailure {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Aborted(_) => None,
            Self::State(error) => Some(error),
            Self::Sdk(error) => Some(error.as_ref()),
            Self::Decode { source, .. } => Some(source),
        }
    }
}
impl<K, P> fmt::Display for BatchGetError<K, P> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.kind.fmt(f)
    }
}
impl<K, P> Error for BatchGetError<K, P> {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(&self.kind)
    }
}

pub struct BatchGet<R, K, P = R> {
    progress: Option<BatchGetProgress<R, K, P>>,
    abort: Option<BatchRoundAbort>,
    table: String,
    template: KeysAndAttributes,
    capacity: Capacity,
    capacities: Vec<Vec<ConsumedCapacity>>,
    expressions: RenderedExpressions,
    decode_item: fn(&Item) -> Result<P, DecodeError>,
    key: fn(&Item, bool) -> Result<Item, BuildError>,
}
pub struct PreparedBatchGet<P> {
    request: PreparedRequest<BatchGetItemInput, P>,
    chunk: BatchGetChunk,
    table: String,
    key: fn(&Item, bool) -> Result<Item, BuildError>,
}
/// Opaque round identity and decoded outcome; only its originating batch may accept it.
pub struct BatchGetRound<P> {
    chunk: BatchGetChunk,
    capacity: Vec<ConsumedCapacity>,
    result: Result<DecodedRound<P>, BatchGetFailure>,
}
struct DecodedRound<P> {
    found: Vec<(usize, P)>,
    pending: Vec<usize>,
    fault: Option<BatchGetFailure>,
}
impl<R: Record + Document> Table<R> {
    pub fn batch_get<K: RecordKey<R>>(
        &self,
        keys: Vec<K>,
        consistency: ReadConsistency,
        budget: BatchBudget,
    ) -> Result<BatchGet<R, K>, BuildError> {
        Ok(BatchGet {
            progress: Some(BatchGetProgress::new(keys, budget)?),
            abort: None,
            table: self.name.clone(),
            template: KeysAndAttributes::builder()
                .set_keys(Some(Vec::new()))
                .consistent_read(consistency == ReadConsistency::Strong)
                .build()
                .map_err(|_| BuildError::EmptyOperation)?,
            capacity: Capacity::None,
            capacities: Vec::new(),
            expressions: RenderedExpressions::default(),
            decode_item: |item| R::decode_map(item, &mut DecodeContext::new(R::NAME)),
            key: |item, key_only| {
                let mut key = Item::new();
                for position in [Some(R::SCHEMA.primary.partition), R::SCHEMA.primary.sort]
                    .into_iter()
                    .flatten()
                {
                    let field = R::SCHEMA
                        .fields
                        .get(position)
                        .ok_or(BuildError::EmptyOperation)?;
                    if let Some(value) = item.get(field.name) {
                        key.insert(field.name.into(), value.clone());
                    }
                }
                if key_only && key.len() != item.len() {
                    return Err(BuildError::EmptyOperation);
                }
                crate::batch_get::canonical_key::<R>(key)
            },
        })
    }
}
impl<R, K, P> BatchGet<R, K, P> {
    /// Freeze an outstanding call without making its unread keys Missing.
    pub fn abort_round(&mut self, cause: BatchRoundAbort) -> Result<(), BatchStateError> {
        self.active()?;
        if !self
            .progress
            .as_ref()
            .expect("active batch")
            .has_outstanding()
        {
            return Err(BatchStateError::WrongRound);
        }
        self.abort = Some(cause);
        Ok(())
    }
    fn active(&self) -> Result<(), BatchStateError> {
        if self.abort.is_some() || self.progress.is_none() {
            return Err(BatchStateError::Frozen);
        }
        Ok(())
    }
    pub fn set_capacity(&mut self, capacity: Capacity) -> Result<(), BatchStateError> {
        self.active()?;
        let progress = self.progress.as_ref().ok_or(BatchStateError::Frozen)?;
        if progress.calls_issued() != 0 {
            return Err(BatchStateError::AlreadyStarted);
        }
        self.capacity = capacity;
        Ok(())
    }
    pub fn next_request(&mut self) -> Result<Option<PreparedBatchGet<P>>, BatchStateError> {
        self.active()?;
        let Some(chunk) = self
            .progress
            .as_mut()
            .ok_or(BatchStateError::Frozen)?
            .next_round()?
        else {
            return Ok(None);
        };
        let mut attributes = self.template.clone();
        attributes.keys = chunk.keys().to_vec();
        let input = BatchGetItemInput::builder()
            .request_items(self.table.clone(), attributes)
            .set_return_consumed_capacity(self.capacity.sdk())
            .build()
            .expect("one validated table");
        let mut expressions = RenderedExpressions::default();
        expressions.projection = self.expressions.projection.clone();
        expressions.aliases.names = self.expressions.names().clone();
        let request = PreparedRequest {
            input,
            expressions,
            decode_item: self.decode_item,
        };
        Ok(Some(PreparedBatchGet {
            request,
            chunk,
            table: self.table.clone(),
            key: self.key,
        }))
    }
    pub fn accept(&mut self, round: BatchGetRound<P>) -> Result<(), BatchGetError<K, P>> {
        if let Some(cause) = self.abort {
            return Err(self.fail(BatchGetFailure::Aborted(cause)));
        }
        let Some(progress) = self.progress.as_ref() else {
            return Err(self.fail(BatchGetFailure::State(BatchStateError::Frozen)));
        };
        if let Err(error) = progress.validate_round(&round.chunk) {
            return Err(self.fail(BatchGetFailure::State(error)));
        }
        self.capacities.push(round.capacity);
        let data = round.result.map_err(|error| self.fail(error))?;
        self.progress
            .as_mut()
            .expect("checked above")
            .accept_round(&round.chunk, data.found, data.pending)
            .map_err(|error| self.fail(BatchGetFailure::State(error)))?;
        if let Some(error) = data.fault {
            return Err(self.fail(error));
        }
        Ok(())
    }
    fn fail(&mut self, kind: BatchGetFailure) -> BatchGetError<K, P> {
        self.abort = None;
        let mut partial = self
            .progress
            .take()
            .map(BatchGetProgress::into_partial)
            .unwrap_or(BatchGetPartial {
                entries: Vec::new(),
                calls_issued: 0,
                capacity: Vec::new(),
            });
        partial.capacity = std::mem::take(&mut self.capacities);
        BatchGetError {
            kind,
            partial: Box::new(partial),
        }
    }
    pub fn finish(mut self) -> Result<BatchGetComplete<P>, BatchGetError<K, P>> {
        if let Some(cause) = self.abort {
            return Err(self.fail(BatchGetFailure::Aborted(cause)));
        }
        let Some(progress) = self.progress.take() else {
            return Err(self.fail(BatchGetFailure::State(BatchStateError::Frozen)));
        };
        match progress.finish() {
            Ok(mut complete) => {
                complete.capacity = self.capacities;
                Ok(complete)
            }
            Err(mut error) => {
                error.partial.capacity = self.capacities;
                Err(BatchGetError {
                    kind: BatchGetFailure::State(error.kind),
                    partial: Box::new(error.partial),
                })
            }
        }
    }
}
impl<P> PreparedBatchGet<P> {
    pub fn input(&self) -> &BatchGetItemInput {
        self.request.input()
    }
    pub fn expressions(&self) -> &RenderedExpressions {
        self.request.expressions()
    }
    pub fn apply(&self, builder: BatchGetItemFluentBuilder) -> BatchGetItemFluentBuilder {
        builder
            .set_request_items(self.input().request_items.clone())
            .set_return_consumed_capacity(self.input().return_consumed_capacity.clone())
    }
    pub fn decode(
        &self,
        result: Result<BatchGetItemOutput, SdkError<BatchGetItemError>>,
    ) -> BatchGetRound<P> {
        let (capacity, result) = match result {
            Err(error) => (Vec::new(), Err(BatchGetFailure::Sdk(Box::new(error)))),
            Ok(mut output) => (
                output.consumed_capacity.take().unwrap_or_default(),
                self.decode_output(output),
            ),
        };
        BatchGetRound {
            chunk: self.chunk.clone(),
            capacity,
            result,
        }
    }
    fn decode_output(
        &self,
        output: BatchGetItemOutput,
    ) -> Result<DecodedRound<P>, BatchGetFailure> {
        let invalid = || BatchGetFailure::State(BatchStateError::InvalidResponse);
        let responses = output.responses.unwrap_or_default();
        let unprocessed = output.unprocessed_keys.unwrap_or_default();
        if responses
            .keys()
            .chain(unprocessed.keys())
            .any(|table| table != &self.table)
        {
            return Err(invalid());
        }
        let mut seen = vec![false; self.chunk.keys().len()];
        let mut correlate = |raw: &Item, key_only| {
            let key = (self.key)(raw, key_only).map_err(|_| invalid())?;
            let index = self
                .chunk
                .keys()
                .iter()
                .position(|issued| issued == &key)
                .ok_or_else(invalid)?;
            if std::mem::replace(&mut seen[index], true) {
                return Err(invalid());
            }
            Ok(self.chunk.positions()[index])
        };
        let mut pending = Vec::new();
        if let Some(attributes) = unprocessed.get(&self.table) {
            for key in &attributes.keys {
                pending.push(correlate(key, true)?);
            }
        }
        let mut correlated = Vec::new();
        if let Some(items) = responses.get(&self.table) {
            for item in items {
                correlated.push((correlate(item, false)?, item));
            }
        }
        let mut found = Vec::new();
        let mut fault = None;
        for (position, raw) in correlated {
            match (self.request.decode_item)(raw) {
                Ok(value) => found.push((position, value)),
                Err(source) => {
                    pending.push(position);
                    if fault.is_none() {
                        fault = Some(BatchGetFailure::Decode { position, source });
                    }
                }
            }
        }
        Ok(DecodedRound {
            found,
            pending,
            fault,
        })
    }
}
macro_rules! redacted_debug {
    ($name:ident $(<$($generic:ident),+>)?) => {
        impl$(<$($generic),+>)? fmt::Debug for $name$(<$($generic),+>)? {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { f.write_str(concat!(stringify!($name), "([redacted])")) }
        }
    };
}
redacted_debug!(BatchGetFailure);
redacted_debug!(BatchGetError<K, P>);
redacted_debug!(BatchGet<R, K, P>);
redacted_debug!(PreparedBatchGet<P>);
redacted_debug!(BatchGetRound<P>);
