//! Caller-driven BatchWriteItem requests; no I/O, retries or implicit splitting.
use std::{error::Error, fmt, sync::Arc};

use aws_sdk_dynamodb::{
    error::SdkError,
    operation::batch_write_item::{
        BatchWriteItemError, BatchWriteItemInput, BatchWriteItemOutput,
        builders::BatchWriteItemFluentBuilder,
    },
    types::{ConsumedCapacity, DeleteRequest, PutRequest, WriteRequest},
};

use crate::{
    BatchAction, BatchBudget, BatchStateError, BatchWriteChunk, BatchWritePartial,
    BatchWriteProgress, BuildError, Capacity, Record, RenderedExpressions, Table,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum BatchWriteAbort {
    Cancelled,
    Deadline,
}

#[non_exhaustive]
pub enum BatchWriteErrorKind {
    State(BatchStateError),
    Sdk(Box<SdkError<BatchWriteItemError>>),
    Aborted(BatchWriteAbort),
}
/// Every original action and its state, plus one unchanged optional vector per issued call.
pub struct BatchWriteError<R> {
    pub kind: BatchWriteErrorKind,
    pub partial: BatchWritePartial<R>,
    pub capacity: Vec<Option<Vec<ConsumedCapacity>>>,
}
impl<R> BatchWriteError<R> {
    pub fn sdk_error(&self) -> Option<&SdkError<BatchWriteItemError>> {
        match &self.kind {
            BatchWriteErrorKind::Sdk(error) => Some(error),
            _ => None,
        }
    }
}
impl<R> fmt::Display for BatchWriteError<R> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.kind {
            BatchWriteErrorKind::State(state) => state.fmt(f),
            BatchWriteErrorKind::Aborted(cause) => write!(f, "BatchWrite aborted: {cause:?}"),
            BatchWriteErrorKind::Sdk(_) => f.write_str("BatchWrite SDK error"),
        }
    }
}
impl<R> Error for BatchWriteError<R> {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        self.sdk_error().map(|error| error as &dyn Error)
    }
}
pub struct BatchWriteResult<R> {
    pub actions: Vec<BatchAction<R>>,
    pub capacity: Vec<Option<Vec<ConsumedCapacity>>>,
}
struct Round {
    identity: Arc<()>,
    call: u32,
    chunk: BatchWriteChunk,
    capacity: Option<Vec<ConsumedCapacity>>,
}
/// Immutable input and round identity; decode consumes the request, never sending it.
pub struct PreparedBatchWrite {
    input: BatchWriteItemInput,
    expressions: RenderedExpressions,
    round: Round,
}
pub struct BatchWriteRound {
    round: Round,
    pending: Vec<usize>,
}
pub struct BatchWriteRoundError {
    round: Round,
    kind: BatchWriteErrorKind,
}
impl fmt::Display for BatchWriteRoundError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("BatchWrite round error; pass it to batch.accept")
    }
}
impl Error for BatchWriteRoundError {}
impl PreparedBatchWrite {
    pub fn input(&self) -> &BatchWriteItemInput {
        &self.input
    }
    pub fn expressions(&self) -> &RenderedExpressions {
        &self.expressions
    }
    pub fn positions(&self) -> &[usize] {
        self.round.chunk.positions()
    }
    /// Replace all modeled fields, including absent fields, without touching transport.
    pub fn apply(&self, builder: BatchWriteItemFluentBuilder) -> BatchWriteItemFluentBuilder {
        builder
            .set_request_items(self.input.request_items.clone())
            .set_return_consumed_capacity(self.input.return_consumed_capacity.clone())
            .set_return_item_collection_metrics(self.input.return_item_collection_metrics.clone())
    }
    pub fn decode(
        mut self,
        result: Result<BatchWriteItemOutput, SdkError<BatchWriteItemError>>,
    ) -> Result<BatchWriteRound, BatchWriteRoundError> {
        let output = match result {
            Ok(output) => output,
            Err(error) => {
                return Err(BatchWriteRoundError {
                    round: self.round,
                    kind: BatchWriteErrorKind::Sdk(Box::new(error)),
                });
            }
        };
        self.round.capacity = output.consumed_capacity;
        let mut pending = Vec::new();
        for (table, writes) in output.unprocessed_items.unwrap_or_default() {
            let issued = self
                .input
                .request_items
                .as_ref()
                .and_then(|items| items.get(&table));
            for write in writes {
                let position = issued
                    .and_then(|issued| issued.iter().position(|w| *w == write))
                    .map(|i| self.positions()[i]);
                match position {
                    Some(position) if !pending.contains(&position) => pending.push(position),
                    _ => {
                        return Err(BatchWriteRoundError {
                            round: self.round,
                            kind: BatchWriteErrorKind::State(BatchStateError::InvalidResponse),
                        });
                    }
                }
            }
            if issued.is_none() {
                return Err(BatchWriteRoundError {
                    round: self.round,
                    kind: BatchWriteErrorKind::State(BatchStateError::InvalidResponse),
                });
            }
        }
        Ok(BatchWriteRound {
            round: self.round,
            pending,
        })
    }
}

/// Drives milestone 44's pure core. Arc errors retain owned progress both here and
/// in the caller, without requiring records/actions to be Clone. Finish returns
/// the same error after a terminal failure; dropping the batch releases its copy.
pub struct BatchWriteBatch<R> {
    core: Option<BatchWriteProgress<R>>,
    table: String,
    identity: Arc<()>,
    outstanding: Option<BatchWriteChunk>,
    capacity_choice: Capacity,
    capacity: Vec<Option<Vec<ConsumedCapacity>>>,
    failure: Option<Arc<BatchWriteError<R>>>,
}
impl<R: Record> Table<R> {
    pub fn batch_write(
        &self,
        actions: Vec<BatchAction<R>>,
        budget: BatchBudget,
    ) -> Result<BatchWriteBatch<R>, BuildError> {
        let core = BatchWriteProgress::new(actions, budget)?;
        Ok(BatchWriteBatch {
            core: Some(core),
            table: self.name.clone(),
            identity: Arc::new(()),
            outstanding: None,
            capacity_choice: Capacity::None,
            capacity: vec![],
            failure: None,
        })
    }
}
impl<R> BatchWriteBatch<R> {
    pub fn set_capacity(&mut self, capacity: Capacity) -> Result<(), BuildError> {
        if self
            .core
            .as_ref()
            .is_none_or(|core| core.calls_issued() != 0)
        {
            return Err(BuildError::OperationConstraint {
                code: "already_started",
            });
        }
        self.capacity_choice = capacity;
        Ok(())
    }
    pub fn next_request(&mut self) -> Result<Option<PreparedBatchWrite>, BatchStateError> {
        let core = self.core.as_mut().ok_or(BatchStateError::Indeterminate)?;
        let Some(chunk) = core.next_round()? else {
            return Ok(None);
        };
        self.outstanding = Some(chunk.clone());
        let mut writes = Vec::new();
        for &position in chunk.positions() {
            let action = &core.entries()[position].action;
            // Required maps are frozen/validated by BatchAction. SDK builders
            // have no additional fallible fields here.
            let write = match action.item() {
                Some(item) => WriteRequest::builder().put_request(
                    PutRequest::builder()
                        .set_item(Some(item.clone()))
                        .build()
                        .expect("validated item map"),
                ),
                None => WriteRequest::builder().delete_request(
                    DeleteRequest::builder()
                        .set_key(Some(action.key().clone()))
                        .build()
                        .expect("validated key map"),
                ),
            };
            writes.push(write.build());
        }
        let input = BatchWriteItemInput::builder()
            .request_items(&self.table, writes)
            .set_return_consumed_capacity(self.capacity_choice.sdk())
            .build()
            .expect("validated batch maps");
        Ok(Some(PreparedBatchWrite {
            input,
            expressions: RenderedExpressions::default(),
            round: Round {
                identity: Arc::clone(&self.identity),
                call: core.calls_issued(),
                chunk,
                capacity: None,
            },
        }))
    }
    pub fn accept(
        &mut self,
        result: Result<BatchWriteRound, BatchWriteRoundError>,
    ) -> Result<(), Arc<BatchWriteError<R>>> {
        if let Some(error) = &self.failure {
            return Err(Arc::clone(error));
        }
        let (round, pending) = match result {
            Ok(success) => (success.round, Ok(success.pending)),
            Err(error) => (error.round, Err(error.kind)),
        };
        let core = self.core.as_mut().expect("active core");
        if !Arc::ptr_eq(&round.identity, &self.identity)
            || round.call != core.calls_issued()
            || self.outstanding.is_none()
        {
            return Err(self.fail(
                BatchWriteErrorKind::State(BatchStateError::WrongRound),
                None,
            ));
        }
        let pending = match pending {
            Ok(pending) => pending,
            Err(kind) => return Err(self.fail(kind, round.capacity)),
        };
        if let Err(kind) = core.accept_round(&round.chunk, pending) {
            return Err(self.fail(BatchWriteErrorKind::State(kind), round.capacity));
        }
        self.outstanding = None;
        self.capacity.push(round.capacity);
        Ok(())
    }
    fn fail(
        &mut self,
        kind: BatchWriteErrorKind,
        capacity: Option<Vec<ConsumedCapacity>>,
    ) -> Arc<BatchWriteError<R>> {
        let mut core = self.core.take().expect("active core");
        if let Some(chunk) = self.outstanding.take() {
            core.fail_round(&chunk).expect("current outstanding round");
            self.capacity.push(capacity);
        }
        let error = Arc::new(BatchWriteError {
            kind,
            partial: core.into_partial(),
            capacity: std::mem::take(&mut self.capacity),
        });
        self.failure = Some(Arc::clone(&error));
        error
    }
    pub fn abort_round(&mut self, cause: BatchWriteAbort) -> Result<(), BatchStateError> {
        if self.failure.is_some() {
            return Err(BatchStateError::Indeterminate);
        }
        if self.outstanding.is_none() {
            return Err(BatchStateError::WrongRound);
        }
        self.fail(BatchWriteErrorKind::Aborted(cause), None);
        Ok(())
    }
    pub fn finish(mut self) -> Result<BatchWriteResult<R>, Arc<BatchWriteError<R>>> {
        if let Some(error) = self.failure {
            return Err(error);
        }
        if self.outstanding.is_some() {
            self.capacity.push(None);
        }
        match self.core.take().expect("active core").finish() {
            Ok(complete) => Ok(BatchWriteResult {
                actions: complete.actions,
                capacity: self.capacity,
            }),
            Err(error) => Err(Arc::new(BatchWriteError {
                kind: BatchWriteErrorKind::State(error.kind),
                partial: error.partial,
                capacity: self.capacity,
            })),
        }
    }
}
macro_rules! redacted_debug {
    ($name:ident $(<$generic:ident>)?) => {
        impl$(<$generic>)? fmt::Debug for $name$(<$generic>)? {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(concat!(stringify!($name), "([redacted])"))
            }
        }
    };
}
redacted_debug!(BatchWriteErrorKind);
redacted_debug!(BatchWriteError<R>);
redacted_debug!(BatchWriteResult<R>);
redacted_debug!(PreparedBatchWrite);
redacted_debug!(BatchWriteRound);
redacted_debug!(BatchWriteRoundError);
redacted_debug!(BatchWriteBatch<R>);
