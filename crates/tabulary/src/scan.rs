//! Prepare and decode one Scan page; callers schedule every segment and request.
use std::{fmt, num::NonZeroU32};

use aws_sdk_dynamodb::{
    error::SdkError,
    operation::scan::{ScanError, ScanInput, ScanOutput, builders::ScanFluentBuilder},
};

use crate::cursor::CursorBinding;
use crate::page::RawPage;
use crate::{
    BuildError, Capacity, Condition, Cursor, DecodeContext, DecodeError, Document, IndexDefinition,
    IndexSchema, IndexTable, Item, Page, PageMetadata, PreparedRequest, Projection,
    ReadConsistency, ReadError, Record, RenderedExpressions, Table,
};

/// A validated zero-based segment and total. Construction never schedules work.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScanSegment {
    segment: u32,
    total: u32,
}
impl ScanSegment {
    pub fn new(segment: u32, total: u32) -> Result<Self, BuildError> {
        if total == 0 || total > 1_000_000 || segment >= total {
            return Err(BuildError::OperationConstraint {
                code: "invalid_scan_segment",
            });
        }
        Ok(Self { segment, total })
    }
}

type Selection = fn() -> Result<Vec<&'static str>, BuildError>;
/// Table scans use `()` as the cursor marker; index scans use their index peer.
pub struct ScanBuilder<R: Record, I = (), P = R> {
    table: Table<R>,
    index: Option<IndexDefinition>,
    consistency: ReadConsistency,
    capacity: Capacity,
    limit: Option<NonZeroU32>,
    segment: Option<ScanSegment>,
    filter: Option<Condition<R>>,
    resume: Option<Cursor<R, I>>,
    selection: Option<Selection>,
    decode_item: fn(&Item) -> Result<P, DecodeError>,
}
pub struct PreparedScan<R, I = (), P = R> {
    request: PreparedRequest<ScanInput, P>,
    binding: CursorBinding<R, I>,
}
impl<R: Record + Document> Table<R> {
    pub fn scan(&self, consistency: ReadConsistency) -> ScanBuilder<R> {
        ScanBuilder::new(self.clone(), None, consistency)
    }
}
impl<R: Record + Document, I: IndexSchema<R>> IndexTable<R, I> {
    pub fn scan(&self, consistency: ReadConsistency) -> ScanBuilder<R, I> {
        ScanBuilder::new(self.table.clone(), Some(I::DEFINITION), consistency)
    }
}
impl<R: Record + Document, I> ScanBuilder<R, I> {
    fn new(table: Table<R>, index: Option<IndexDefinition>, consistency: ReadConsistency) -> Self {
        Self {
            table,
            index,
            consistency,
            capacity: Capacity::None,
            limit: None,
            segment: None,
            filter: None,
            resume: None,
            selection: None,
            decode_item: |item| R::decode_map(item, &mut DecodeContext::new(R::NAME)),
        }
    }
}
impl<R: Record, I, P> ScanBuilder<R, I, P> {
    pub fn capacity(mut self, capacity: Capacity) -> Self {
        self.capacity = capacity;
        self
    }
    /// Evaluated items per service call, before filtering.
    pub fn limit(mut self, limit: NonZeroU32) -> Self {
        self.limit = Some(limit);
        self
    }
    pub fn segment(mut self, segment: ScanSegment) -> Self {
        self.segment = Some(segment);
        self
    }
    /// AND this predicate with existing filters. Scan filters may include keys.
    pub fn filter(mut self, filter: Condition<R>) -> Self {
        self.filter = Some(match self.filter {
            Some(existing) => existing.and(filter),
            None => filter,
        });
        self
    }
    pub fn resume(mut self, cursor: Cursor<R, I>) -> Self {
        self.resume = Some(cursor);
        self
    }
    pub fn select<V: Projection<Source = R>>(self) -> ScanBuilder<R, I, V> {
        ScanBuilder {
            table: self.table,
            index: self.index,
            consistency: self.consistency,
            capacity: self.capacity,
            limit: self.limit,
            segment: self.segment,
            filter: self.filter,
            resume: self.resume,
            selection: Some(V::attributes),
            decode_item: V::from_item,
        }
    }
    pub fn build(self) -> Result<PreparedScan<R, I, P>, BuildError> {
        crate::read_selection::validate_consistency(self.index, self.consistency)?;
        let limit = self
            .limit
            .map(|limit| {
                i32::try_from(limit.get()).map_err(|_| BuildError::RequestLimit {
                    max: i32::MAX as usize,
                    actual: limit.get() as usize,
                })
            })
            .transpose()?;
        let filter = self.filter.map(Condition::build).transpose()?;
        let selection = self.selection.map(|select| select()).transpose()?;
        crate::read_selection::validate_selection::<R>(
            self.index,
            selection.as_deref(),
            filter.as_ref(),
        )?;
        let binding = CursorBinding::scan(
            &self.table,
            self.index,
            filter.as_ref(),
            selection.clone(),
            self.consistency,
            self.segment.map(|segment| (segment.segment, segment.total)),
        )?;
        let start = self
            .resume
            .as_ref()
            .map(|cursor| binding.resume(cursor).cloned())
            .transpose()?;
        let mut expressions = RenderedExpressions::default();
        if let Some(filter) = filter {
            expressions.filter = Some(crate::condition_renderer::render_condition(
                filter.root(),
                &mut expressions.aliases,
            ));
        }
        if let Some(names) = selection {
            expressions.projection = Some(
                names
                    .iter()
                    .map(|name| expressions.aliases.name(name))
                    .collect::<Vec<_>>()
                    .join(", "),
            );
        }
        expressions.validate()?;
        let input = ScanInput::builder()
            .table_name(self.table.name())
            .set_index_name(self.index.map(|index| index.name.into()))
            .consistent_read(self.consistency == ReadConsistency::Strong)
            .set_limit(limit)
            .set_segment(self.segment.map(|segment| segment.segment as i32))
            .set_total_segments(self.segment.map(|segment| segment.total as i32))
            .set_exclusive_start_key(start)
            .set_return_consumed_capacity(self.capacity.sdk())
            .set_filter_expression(expressions.filter.clone())
            .set_projection_expression(expressions.projection.clone())
            .set_expression_attribute_names(
                (!expressions.names().is_empty()).then(|| expressions.names().clone()),
            )
            .set_expression_attribute_values(
                (!expressions.values().is_empty()).then(|| expressions.values().clone()),
            )
            .build()
            .map_err(|_| BuildError::OperationConstraint {
                code: "invalid_scan_input",
            })?;
        Ok(PreparedScan {
            request: PreparedRequest {
                input,
                expressions,
                decode_item: self.decode_item,
            },
            binding,
        })
    }
}
impl<R, I, P> PreparedScan<R, I, P> {
    pub fn input(&self) -> &ScanInput {
        self.request.input()
    }
    pub fn expressions(&self) -> &RenderedExpressions {
        self.request.expressions()
    }
    /// Replace every modeled field, clearing stale legacy fields without I/O.
    pub fn apply(&self, builder: ScanFluentBuilder) -> ScanFluentBuilder {
        let input = self.input();
        builder
            .set_table_name(input.table_name.clone())
            .set_index_name(input.index_name.clone())
            .set_select(input.select.clone())
            .set_attributes_to_get(input.attributes_to_get.clone())
            .set_limit(input.limit)
            .set_consistent_read(input.consistent_read)
            .set_scan_filter(input.scan_filter.clone())
            .set_conditional_operator(input.conditional_operator.clone())
            .set_exclusive_start_key(input.exclusive_start_key.clone())
            .set_return_consumed_capacity(input.return_consumed_capacity.clone())
            .set_segment(input.segment)
            .set_total_segments(input.total_segments)
            .set_projection_expression(input.projection_expression.clone())
            .set_filter_expression(input.filter_expression.clone())
            .set_expression_attribute_names(input.expression_attribute_names.clone())
            .set_expression_attribute_values(input.expression_attribute_values.clone())
    }
}
impl<R: Record, I, P> PreparedScan<R, I, P> {
    pub fn decode(
        &self,
        result: Result<ScanOutput, SdkError<ScanError>>,
    ) -> Result<Page<P, Cursor<R, I>>, ReadError<ScanError>> {
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
        .decode(&self.binding, self.request.decode_item)
        .map_err(ReadError::Decode)
    }
}
impl<R, I, P> fmt::Debug for PreparedScan<R, I, P> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.request.fmt(f)
    }
}
