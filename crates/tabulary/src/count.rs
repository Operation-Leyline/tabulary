//! Typed `Select::Count` Query and Scan pages. Counts never decode items.
//!
//! A count builder is a separate type: it has no `select` method, so a
//! projection cannot be combined with a count, and its cursors bind a count
//! read that item reads cannot resume.
use std::{fmt, num::NonZeroU32};

use aws_sdk_dynamodb::{
    error::SdkError,
    operation::{
        query::{QueryError, QueryInput, QueryOutput, builders::QueryFluentBuilder},
        scan::{ScanError, ScanInput, ScanOutput, builders::ScanFluentBuilder},
    },
};

use crate::page::RawPage;
use crate::{
    BuildError, Capacity, Condition, CountPage, Cursor, KeySchema, PageMetadata, PreparedQuery,
    PreparedScan, QueryBuilder, QueryOrder, ReadError, Record, RenderedExpressions, ScanBuilder,
    ScanSegment,
};

/// A Query that returns `Select::Count` pages. Built from `QueryBuilder::count`.
pub struct CountQueryBuilder<R: Record, I: KeySchema<R>, S> {
    inner: QueryBuilder<R, I, S>,
}
/// A prepared `Select::Count` Query. Performs no I/O.
pub struct PreparedCountQuery<R, I> {
    inner: PreparedQuery<R, I, R>,
}
/// A Scan that returns `Select::Count` pages. Built from `ScanBuilder::count`.
pub struct CountScanBuilder<R: Record, I = ()> {
    inner: ScanBuilder<R, I>,
}
/// A prepared `Select::Count` Scan. Performs no I/O.
pub struct PreparedCountScan<R, I = ()> {
    inner: PreparedScan<R, I, R>,
}

impl<R: Record, I: KeySchema<R>, S> QueryBuilder<R, I, S> {
    /// Count matching items instead of returning them. Only a full-record
    /// builder can become a count, so a projection never reaches one.
    pub fn count(self) -> CountQueryBuilder<R, I, S> {
        CountQueryBuilder { inner: self }
    }
}
impl<R: Record, I: KeySchema<R>, S> CountQueryBuilder<R, I, S> {
    pub fn capacity(self, capacity: Capacity) -> Self {
        Self {
            inner: self.inner.capacity(capacity),
        }
    }
    pub fn order(self, order: QueryOrder) -> Self {
        Self {
            inner: self.inner.order(order),
        }
    }
    /// Evaluated items per service call, before filtering; never a count goal.
    pub fn limit(self, limit: NonZeroU32) -> Self {
        Self {
            inner: self.inner.limit(limit),
        }
    }
    /// AND this predicate with any existing filter.
    pub fn filter(self, filter: Condition<R>) -> Self {
        Self {
            inner: self.inner.filter(filter),
        }
    }
    /// Resume only from a cursor returned by an equivalent count Query.
    pub fn resume(self, cursor: Cursor<R, I>) -> Self {
        Self {
            inner: self.inner.resume(cursor),
        }
    }
    pub fn build(self) -> Result<PreparedCountQuery<R, I>, BuildError> {
        Ok(PreparedCountQuery {
            inner: self.inner.prepare(true)?,
        })
    }
}
impl<R: Record, I> PreparedCountQuery<R, I> {
    /// Explicit raw SDK access; formatting this input may reveal values.
    pub fn input(&self) -> &QueryInput {
        self.inner.input()
    }
    pub fn expressions(&self) -> &RenderedExpressions {
        self.inner.expressions()
    }
    /// Replace every modeled field, clearing stale legacy fields without I/O.
    pub fn apply(&self, builder: QueryFluentBuilder) -> QueryFluentBuilder {
        self.inner.apply(builder)
    }
    pub fn decode(
        &self,
        result: Result<QueryOutput, SdkError<QueryError>>,
    ) -> Result<CountPage<Cursor<R, I>>, ReadError<QueryError>> {
        let output = result.map_err(|error| ReadError::Sdk(Box::new(error)))?;
        RawPage {
            items: output.items.unwrap_or_default(),
            capacity: output.consumed_capacity,
            metadata: PageMetadata {
                count: output.count,
                scanned_count: output.scanned_count,
                last_evaluated_key: output.last_evaluated_key,
            },
        }
        .decode_count(self.inner.binding())
        .map_err(ReadError::Decode)
    }
}
impl<R, I> fmt::Debug for PreparedCountQuery<R, I> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.inner.fmt(f)
    }
}

impl<R: Record, I> ScanBuilder<R, I> {
    /// Count matching items instead of returning them. Only a full-record
    /// builder can become a count, so a projection never reaches one.
    pub fn count(self) -> CountScanBuilder<R, I> {
        CountScanBuilder { inner: self }
    }
}
impl<R: Record, I> CountScanBuilder<R, I> {
    pub fn capacity(self, capacity: Capacity) -> Self {
        Self {
            inner: self.inner.capacity(capacity),
        }
    }
    /// Evaluated items per service call, before filtering.
    pub fn limit(self, limit: NonZeroU32) -> Self {
        Self {
            inner: self.inner.limit(limit),
        }
    }
    pub fn segment(self, segment: ScanSegment) -> Self {
        Self {
            inner: self.inner.segment(segment),
        }
    }
    /// AND this predicate with existing filters. Scan filters may include keys.
    pub fn filter(self, filter: Condition<R>) -> Self {
        Self {
            inner: self.inner.filter(filter),
        }
    }
    /// Resume only from a cursor returned by an equivalent count Scan.
    pub fn resume(self, cursor: Cursor<R, I>) -> Self {
        Self {
            inner: self.inner.resume(cursor),
        }
    }
    pub fn build(self) -> Result<PreparedCountScan<R, I>, BuildError> {
        Ok(PreparedCountScan {
            inner: self.inner.prepare(true)?,
        })
    }
}
impl<R: Record, I> PreparedCountScan<R, I> {
    /// Explicit raw SDK access; formatting this input may reveal values.
    pub fn input(&self) -> &ScanInput {
        self.inner.input()
    }
    pub fn expressions(&self) -> &RenderedExpressions {
        self.inner.expressions()
    }
    /// Replace every modeled field, clearing stale legacy fields without I/O.
    pub fn apply(&self, builder: ScanFluentBuilder) -> ScanFluentBuilder {
        self.inner.apply(builder)
    }
    pub fn decode(
        &self,
        result: Result<ScanOutput, SdkError<ScanError>>,
    ) -> Result<CountPage<Cursor<R, I>>, ReadError<ScanError>> {
        let output = result.map_err(|error| ReadError::Sdk(Box::new(error)))?;
        RawPage {
            items: output.items.unwrap_or_default(),
            capacity: output.consumed_capacity,
            metadata: PageMetadata {
                count: output.count,
                scanned_count: output.scanned_count,
                last_evaluated_key: output.last_evaluated_key,
            },
        }
        .decode_count(self.inner.binding())
        .map_err(ReadError::Decode)
    }
}
impl<R, I> fmt::Debug for PreparedCountScan<R, I> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.inner.fmt(f)
    }
}
