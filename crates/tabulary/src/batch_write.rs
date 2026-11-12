//! Pure BatchWrite progress; SDK requests and error wrapping belong to the adapter.
use std::{error::Error, fmt, marker::PhantomData, sync::Arc};

use aws_sdk_dynamodb::types::AttributeValue;

use crate::{
    BatchBudget, BatchStateError, BilledSize, BuildError, Decimal, Document, EncodeContext,
    EncodeFault, FieldAccess, Item, PathSegment, Record, RecordKey,
};

/// A frozen unconditional put or delete. No update or write-option surface exists.
pub struct BatchAction<R> {
    key: Item,
    item: Option<Item>,
    marker: PhantomData<fn() -> R>,
}
impl<R: Record + Document> BatchAction<R> {
    pub fn put(record: &R) -> Result<Self, BuildError> {
        unversioned::<R>()?;
        // Match PutBuilder's encoding and item-size gate. Identity comes from this
        // encoded item, never a second key codec that could disagree with the put.
        let item = record.encode_map(&mut EncodeContext::new(R::NAME))?;
        BilledSize::of_item(&item)?.ensure_at_most(400 * 1024)?;
        Ok(Self {
            key: target_key::<R>(&item)?,
            item: Some(item),
            marker: PhantomData,
        })
    }

    pub fn delete<K: RecordKey<R>>(key: K) -> Result<Self, BuildError> {
        unversioned::<R>()?;
        let encoded = key.encode_key()?;
        let key = target_key::<R>(&encoded)?;
        if key.len() != encoded.len() {
            return Err(constraint("invalid_batch_key_shape"));
        }
        Ok(Self {
            key,
            item: None,
            marker: PhantomData,
        })
    }
}
impl<R> BatchAction<R> {
    /// Explicit raw physical key access, normalized for target comparison.
    pub fn key(&self) -> &Item {
        &self.key
    }

    /// Some for a put, None for a delete; the original encoded put is retained.
    pub fn item(&self) -> Option<&Item> {
        self.item.as_ref()
    }
}

fn constraint(code: &'static str) -> BuildError {
    BuildError::OperationConstraint { code }
}
fn unversioned<R: Record>() -> Result<(), BuildError> {
    if R::SCHEMA
        .fields
        .iter()
        .any(|f| f.access == FieldAccess::VersionOnly)
    {
        return Err(BuildError::VersionPolicy);
    }
    Ok(())
}
fn target_key<R: Record>(item: &Item) -> Result<Item, BuildError> {
    let mut key = Item::new();
    for (position, max) in [
        (Some(R::SCHEMA.primary.partition), 2048),
        (R::SCHEMA.primary.sort, 1024),
    ] {
        let Some(position) = position else { continue };
        let field = R::SCHEMA
            .fields
            .get(position)
            .ok_or_else(|| constraint("invalid_batch_key_schema"))?;
        let cx = EncodeContext::new(R::SCHEMA.name).at(PathSegment::Attribute(field.name.into()));
        let value = item.get(field.name).ok_or_else(|| {
            cx.error(EncodeFault::Custom {
                code: "missing_batch_key",
            })
        })?;
        crate::record::validate_key(value, max).map_err(|fault| cx.error(fault))?;
        if !matches!(
            (value, field.wire),
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
        let value = match value {
            AttributeValue::N(number) => AttributeValue::N(
                number
                    .parse::<Decimal>()
                    .map_err(|_| cx.error(EncodeFault::NumberRange))?
                    .to_string(),
            ),
            value => value.clone(),
        };
        key.insert(field.name.into(), value);
    }
    Ok(key)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BatchWriteState {
    Processed,
    Pending,
    Indeterminate,
}
pub struct BatchWriteEntry<R> {
    pub action: BatchAction<R>,
    pub state: BatchWriteState,
}
pub struct BatchWritePartial<R> {
    pub entries: Vec<BatchWriteEntry<R>>,
    pub calls_issued: u32,
}
pub struct BatchWriteComplete<R> {
    pub actions: Vec<BatchAction<R>>,
}
pub struct BatchWriteIncomplete<R> {
    pub kind: BatchStateError,
    pub partial: BatchWritePartial<R>,
}
impl<R> fmt::Display for BatchWriteIncomplete<R> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.kind.fmt(f)
    }
}
impl<R> Error for BatchWriteIncomplete<R> {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(&self.kind)
    }
}

/// Immutable issued round; action positions retain original input order.
#[derive(Clone)]
pub struct BatchWriteChunk {
    identity: Arc<()>,
    call: u32,
    positions: Vec<usize>,
}
impl BatchWriteChunk {
    pub fn positions(&self) -> &[usize] {
        &self.positions
    }
}

/// One homogeneous, table-independent batch core. The adapter binds one table.
pub struct BatchWriteProgress<R> {
    entries: Vec<BatchWriteEntry<R>>,
    identity: Arc<()>,
    outstanding: Option<u32>,
    budget: BatchBudget,
    calls_issued: u32,
    failed: bool,
}
impl<R> BatchWriteProgress<R> {
    pub fn new(actions: Vec<BatchAction<R>>, budget: BatchBudget) -> Result<Self, BuildError> {
        if actions.is_empty() {
            return Err(BuildError::EmptyOperation);
        }
        if actions.len() > 25 {
            return Err(BuildError::RequestLimit {
                max: 25,
                actual: actions.len(),
            });
        }
        for (i, action) in actions.iter().enumerate() {
            if actions[..i]
                .iter()
                .any(|previous| previous.key == action.key)
            {
                return Err(constraint("duplicate_batch_target"));
            }
        }
        Ok(Self {
            entries: actions
                .into_iter()
                .map(|action| BatchWriteEntry {
                    action,
                    state: BatchWriteState::Pending,
                })
                .collect(),
            identity: Arc::new(()),
            outstanding: None,
            budget,
            calls_issued: 0,
            failed: false,
        })
    }
    pub fn entries(&self) -> &[BatchWriteEntry<R>] {
        &self.entries
    }
    pub fn calls_issued(&self) -> u32 {
        self.calls_issued
    }
    pub fn next_round(&mut self) -> Result<Option<BatchWriteChunk>, BatchStateError> {
        if self.failed {
            return Err(BatchStateError::Indeterminate);
        }
        if self.outstanding.is_some() {
            return Err(BatchStateError::OutstandingRound);
        }
        let positions: Vec<_> = self
            .entries
            .iter()
            .enumerate()
            .filter_map(|(i, entry)| (entry.state == BatchWriteState::Pending).then_some(i))
            .collect();
        if positions.is_empty() || self.calls_issued == self.budget.max_calls() {
            return Ok(None);
        }
        self.calls_issued += 1;
        self.outstanding = Some(self.calls_issued);
        Ok(Some(BatchWriteChunk {
            identity: Arc::clone(&self.identity),
            call: self.calls_issued,
            positions,
        }))
    }
    fn validate_round(&self, chunk: &BatchWriteChunk) -> Result<(), BatchStateError> {
        if !Arc::ptr_eq(&self.identity, &chunk.identity) || self.outstanding != Some(chunk.call) {
            return Err(BatchStateError::WrongRound);
        }
        Ok(())
    }
    /// Accept only validated unprocessed positions; all others were processed.
    /// Invalid submissions leave the entire outstanding round unchanged.
    pub fn accept_round(
        &mut self,
        chunk: &BatchWriteChunk,
        pending: Vec<usize>,
    ) -> Result<(), BatchStateError> {
        self.validate_round(chunk)?;
        let mut seen = vec![false; self.entries.len()];
        for &position in &pending {
            if !chunk.positions.contains(&position) || seen[position] {
                return Err(BatchStateError::InvalidResponse);
            }
            seen[position] = true;
        }
        for &position in &chunk.positions {
            self.entries[position].state = if seen[position] {
                BatchWriteState::Pending
            } else {
                BatchWriteState::Processed
            };
        }
        self.outstanding = None;
        Ok(())
    }
    /// Settle an ambiguous failure and freeze this batch against further calls.
    /// The SDK adapter retains the actual error or abort cause separately.
    pub fn fail_round(&mut self, chunk: &BatchWriteChunk) -> Result<(), BatchStateError> {
        self.validate_round(chunk)?;
        self.mark_indeterminate();
        self.failed = true;
        Ok(())
    }
    fn mark_indeterminate(&mut self) {
        if self.outstanding.take().is_some() {
            for entry in &mut self.entries {
                if entry.state == BatchWriteState::Pending {
                    entry.state = BatchWriteState::Indeterminate;
                }
            }
        }
    }
    /// Outstanding writes are conservatively indeterminate when the core is consumed.
    pub fn into_partial(mut self) -> BatchWritePartial<R> {
        self.mark_indeterminate();
        BatchWritePartial {
            entries: self.entries,
            calls_issued: self.calls_issued,
        }
    }
    pub fn finish(self) -> Result<BatchWriteComplete<R>, BatchWriteIncomplete<R>> {
        let kind = if self.failed {
            Some(BatchStateError::Indeterminate)
        } else if self.outstanding.is_some() {
            Some(BatchStateError::OutstandingRound)
        } else if self
            .entries
            .iter()
            .any(|entry| entry.state == BatchWriteState::Pending)
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
            return Err(BatchWriteIncomplete {
                kind,
                partial: self.into_partial(),
            });
        }
        Ok(BatchWriteComplete {
            actions: self.entries.into_iter().map(|entry| entry.action).collect(),
        })
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
redacted_debug!(BatchAction<R>);
redacted_debug!(BatchWriteEntry<R>);
redacted_debug!(BatchWritePartial<R>);
redacted_debug!(BatchWriteComplete<R>);
redacted_debug!(BatchWriteIncomplete<R>);
redacted_debug!(BatchWriteChunk);
redacted_debug!(BatchWriteProgress<R>);
