//! Pure BatchGet progress; SDK preparation and key correlation belong to the adapter.
use std::{error::Error, fmt, marker::PhantomData, sync::Arc};

use aws_sdk_dynamodb::types::AttributeValue;

use crate::{
    BatchBudget, BuildError, Decimal, Document, EncodeContext, EncodeFault, Item, PathSegment,
    Record, RecordKey,
};

/// State of one original input position. Unprocessed keys are never Missing.
pub enum BatchGetState<P> {
    Found(P),
    Missing,
    Pending,
}

/// The original owned key and its current state; no Clone or Eq bound is needed.
pub struct BatchGetEntry<K, P> {
    pub key: K,
    pub state: BatchGetState<P>,
}

/// Ordered progress retained when completion is impossible.
pub struct BatchGetPartial<K, P> {
    pub entries: Vec<BatchGetEntry<K, P>>,
    pub calls_issued: u32,
}

/// Every position was processed; None represents a missing item.
pub struct BatchGetComplete<P> {
    pub records: Vec<Option<P>>,
}

/// Local round lifecycle and correlation faults, without any payload values.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BatchStateError {
    OutstandingRound,
    WrongRound,
    InvalidResponse,
    BudgetExhausted,
    PendingWork,
}
impl fmt::Display for BatchStateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self, f)
    }
}
impl Error for BatchStateError {}

pub struct BatchGetIncomplete<K, P> {
    pub kind: BatchStateError,
    pub partial: BatchGetPartial<K, P>,
}
impl<K, P> fmt::Display for BatchGetIncomplete<K, P> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.kind.fmt(f)
    }
}
impl<K, P> Error for BatchGetIncomplete<K, P> {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(&self.kind)
    }
}

/// An immutable issued round. Positions and encoded keys have the same order.
/// Clones retain the same identity; acceptance still succeeds at most once.
#[derive(Clone)]
pub struct BatchGetChunk {
    identity: Arc<()>,
    call: u32,
    positions: Vec<usize>,
    keys: Vec<Item>,
}
impl BatchGetChunk {
    pub fn positions(&self) -> &[usize] {
        &self.positions
    }

    /// Explicit raw key access; formatting the chunk itself redacts keys.
    pub fn keys(&self) -> &[Item] {
        &self.keys
    }
}

/// The state machine used by the BatchGet adapter, with no SDK request or I/O.
/// R and K follow GetItem's Record + Document and RecordKey contracts.
pub struct BatchGetProgress<R, K, P = R> {
    entries: Vec<BatchGetEntry<K, P>>,
    keys: Vec<Item>,
    identity: Arc<()>,
    outstanding: Option<u32>,
    budget: BatchBudget,
    calls_issued: u32,
    marker: PhantomData<fn() -> R>,
}
impl<R: Record + Document, K: RecordKey<R>, P> BatchGetProgress<R, K, P> {
    pub fn new(keys: Vec<K>, budget: BatchBudget) -> Result<Self, BuildError> {
        if keys.is_empty() {
            return Err(BuildError::EmptyOperation);
        }
        if keys.len() > 100 {
            return Err(BuildError::RequestLimit {
                max: 100,
                actual: keys.len(),
            });
        }
        let mut encoded = Vec::with_capacity(keys.len());
        for key in &keys {
            let key = canonical_key::<R>(key.encode_key()?)?;
            if encoded.contains(&key) {
                return Err(BuildError::OperationConstraint {
                    code: "duplicate_batch_key",
                });
            }
            encoded.push(key);
        }
        Ok(Self {
            entries: keys
                .into_iter()
                .map(|key| BatchGetEntry {
                    key,
                    state: BatchGetState::Pending,
                })
                .collect(),
            keys: encoded,
            identity: Arc::new(()),
            outstanding: None,
            budget,
            calls_issued: 0,
            marker: PhantomData,
        })
    }
}
impl<R, K, P> BatchGetProgress<R, K, P> {
    pub fn entries(&self) -> &[BatchGetEntry<K, P>] {
        &self.entries
    }

    pub fn calls_issued(&self) -> u32 {
        self.calls_issued
    }

    /// Reserve one explicit call; even an abandoned round consumes its slot.
    /// Only keys still Pending are issued, with no splitting or reordering.
    pub fn next_round(&mut self) -> Result<Option<BatchGetChunk>, BatchStateError> {
        if self.outstanding.is_some() {
            return Err(BatchStateError::OutstandingRound);
        }
        let positions: Vec<_> = self
            .entries
            .iter()
            .enumerate()
            .filter_map(|(i, entry)| matches!(entry.state, BatchGetState::Pending).then_some(i))
            .collect();
        if positions.is_empty() || self.calls_issued == self.budget.max_calls() {
            return Ok(None);
        }
        self.calls_issued += 1;
        self.outstanding = Some(self.calls_issued);
        Ok(Some(BatchGetChunk {
            identity: Arc::clone(&self.identity),
            call: self.calls_issued,
            keys: positions.iter().map(|&i| self.keys[i].clone()).collect(),
            positions,
        }))
    }

    /// Feed already decoded/correlated values at original input positions.
    /// Validate the entire round before changing progress. Invalid or stale
    /// submissions leave the outstanding round and accumulated state unchanged.
    /// The SDK adapter owns decoding and transfers partial state on failures.
    pub fn accept_round(
        &mut self,
        chunk: &BatchGetChunk,
        found: Vec<(usize, P)>,
        pending: Vec<usize>,
    ) -> Result<(), BatchStateError> {
        if !Arc::ptr_eq(&self.identity, &chunk.identity) || self.outstanding != Some(chunk.call) {
            return Err(BatchStateError::WrongRound);
        }
        let mut seen = vec![false; self.entries.len()];
        for position in found.iter().map(|(i, _)| i).chain(pending.iter()) {
            if !chunk.positions.contains(position) || seen[*position] {
                return Err(BatchStateError::InvalidResponse);
            }
            seen[*position] = true;
        }
        for &position in &chunk.positions {
            self.entries[position].state = BatchGetState::Missing;
        }
        for position in pending {
            self.entries[position].state = BatchGetState::Pending;
        }
        for (position, record) in found {
            self.entries[position].state = BatchGetState::Found(record);
        }
        self.outstanding = None;
        Ok(())
    }

    /// Move all accumulated records and original keys without cloning them.
    /// An adapter can use this when attaching progress to a terminal failure.
    pub fn into_partial(self) -> BatchGetPartial<K, P> {
        BatchGetPartial {
            entries: self.entries,
            calls_issued: self.calls_issued,
        }
    }

    pub fn finish(self) -> Result<BatchGetComplete<P>, BatchGetIncomplete<K, P>> {
        let kind = if self.outstanding.is_some() {
            Some(BatchStateError::OutstandingRound)
        } else if self
            .entries
            .iter()
            .any(|entry| matches!(entry.state, BatchGetState::Pending))
        {
            Some(if self.calls_issued == self.budget.max_calls() {
                BatchStateError::BudgetExhausted
            } else {
                BatchStateError::PendingWork
            })
        } else {
            None
        };
        if let Some(kind) = kind {
            return Err(BatchGetIncomplete {
                kind,
                partial: self.into_partial(),
            });
        }
        Ok(BatchGetComplete {
            records: self
                .entries
                .into_iter()
                .map(|entry| match entry.state {
                    BatchGetState::Found(record) => Some(record),
                    BatchGetState::Missing => None,
                    BatchGetState::Pending => unreachable!("pending entries were checked above"),
                })
                .collect(),
        })
    }
}

fn canonical_key<R: Record>(mut key: Item) -> Result<Item, BuildError> {
    let expected = 1 + usize::from(R::SCHEMA.primary.sort.is_some());
    if key.len() != expected {
        return Err(BuildError::OperationConstraint {
            code: "invalid_batch_key_shape",
        });
    }
    for (position, max) in [
        (Some(R::SCHEMA.primary.partition), 2048),
        (R::SCHEMA.primary.sort, 1024),
    ] {
        let Some(position) = position else { continue };
        let field = R::SCHEMA
            .fields
            .get(position)
            .ok_or(BuildError::OperationConstraint {
                code: "invalid_batch_key_schema",
            })?;
        let cx = EncodeContext::new(R::SCHEMA.name).at(PathSegment::Attribute(field.name.into()));
        let value = key.get_mut(field.name).ok_or_else(|| {
            cx.error(EncodeFault::Custom {
                code: "missing_batch_key",
            })
        })?;
        crate::record::validate_key(value, max).map_err(|fault| cx.error(fault))?;
        if !matches!(
            (&*value, field.wire),
            (AttributeValue::S(_), crate::DynamoType::S)
                | (AttributeValue::N(_), crate::DynamoType::N)
                | (AttributeValue::B(_), crate::DynamoType::B)
        ) {
            return Err(cx
                .error(EncodeFault::Custom {
                    code: "batch_key_wire_type",
                })
                .into());
        }
        if let AttributeValue::N(number) = value {
            *number = number
                .parse::<Decimal>()
                .map_err(|_| cx.error(EncodeFault::NumberRange))?
                .to_string();
        }
    }
    Ok(key)
}

// Formatting these containers must not require key/record Debug implementations.
macro_rules! redacted_debug {
    ($name:ident $(<$($generic:ident),+>)?) => {
        impl$(<$($generic),+>)? fmt::Debug for $name$(<$($generic),+>)? {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(concat!(stringify!($name), "([redacted])"))
            }
        }
    };
}
redacted_debug!(BatchGetState<P>);
redacted_debug!(BatchGetEntry<K, P>);
redacted_debug!(BatchGetPartial<K, P>);
redacted_debug!(BatchGetComplete<P>);
redacted_debug!(BatchGetIncomplete<K, P>);
redacted_debug!(BatchGetChunk);
redacted_debug!(BatchGetProgress<R, K, P>);
