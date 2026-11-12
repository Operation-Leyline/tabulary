//! Prepare and decode exactly one Query; the caller alone sends requests.
use std::{fmt, num::NonZeroU32};

use crate::condition::{ConditionNode, Operand};
use crate::cursor::CursorBinding;
use crate::page::RawPage;
use crate::{
    BuildError, Capacity, Condition, Cursor, DecodeContext, DecodeError, Document, IndexDefinition,
    IndexKind, IndexProjection, IndexSchema, IndexTable, Item, KeyCondition, KeySchema, Page,
    PageMetadata, PreparedRequest, PrimarySchema, Projection, QueryOrder, ReadConsistency,
    ReadError, Record, RenderedExpressions, Table,
};
use aws_sdk_dynamodb::{
    error::SdkError,
    operation::query::{QueryError, QueryInput, QueryOutput, builders::QueryFluentBuilder},
};

type Selection = fn() -> Result<Vec<&'static str>, BuildError>;
pub struct QueryBuilder<R: Record, I: KeySchema<R>, S, P = R> {
    table: Table<R>,
    index: Option<IndexDefinition>,
    key: KeyCondition<R, I, S>,
    consistency: ReadConsistency,
    capacity: Capacity,
    order: QueryOrder,
    limit: Option<NonZeroU32>,
    filter: Option<Condition<R>>,
    resume: Option<Cursor<R, I>>,
    selection: Option<Selection>,
    decode_item: fn(&Item) -> Result<P, DecodeError>,
}
pub struct PreparedQuery<R, I, P = R> {
    request: PreparedRequest<QueryInput, P>,
    binding: CursorBinding<R, I>,
}
impl<R: Record + Document> Table<R> {
    pub fn query<I: PrimarySchema<R>, S>(
        &self,
        key: KeyCondition<R, I, S>,
        consistency: ReadConsistency,
    ) -> QueryBuilder<R, I, S> {
        QueryBuilder::new(self.clone(), None, key, consistency)
    }
}
impl<R: Record + Document, I: IndexSchema<R> + KeySchema<R>> IndexTable<R, I> {
    pub fn query<S>(
        &self,
        key: KeyCondition<R, I, S>,
        consistency: ReadConsistency,
    ) -> QueryBuilder<R, I, S> {
        QueryBuilder::new(self.table.clone(), Some(I::DEFINITION), key, consistency)
    }
}
impl<R: Record + Document, I: KeySchema<R>, S> QueryBuilder<R, I, S> {
    fn new(
        table: Table<R>,
        index: Option<IndexDefinition>,
        key: KeyCondition<R, I, S>,
        consistency: ReadConsistency,
    ) -> Self {
        Self {
            table,
            index,
            key,
            consistency,
            capacity: Capacity::None,
            order: QueryOrder::Ascending,
            limit: None,
            filter: None,
            resume: None,
            selection: None,
            decode_item: |item| R::decode_map(item, &mut DecodeContext::new(R::NAME)),
        }
    }
}
impl<R: Record, I: KeySchema<R>, S, P> QueryBuilder<R, I, S, P> {
    pub fn capacity(mut self, capacity: Capacity) -> Self {
        self.capacity = capacity;
        self
    }
    pub fn order(mut self, order: QueryOrder) -> Self {
        self.order = order;
        self
    }
    /// Evaluated items per service call, before filtering; never a matching-row goal.
    pub fn limit(mut self, limit: NonZeroU32) -> Self {
        self.limit = Some(limit);
        self
    }
    pub fn filter(mut self, filter: Condition<R>) -> Self {
        self.filter = Some(filter);
        self
    }
    pub fn resume(mut self, cursor: Cursor<R, I>) -> Self {
        self.resume = Some(cursor);
        self
    }
    pub fn select<V: Projection<Source = R>>(self) -> QueryBuilder<R, I, S, V> {
        QueryBuilder {
            table: self.table,
            index: self.index,
            key: self.key,
            consistency: self.consistency,
            capacity: self.capacity,
            order: self.order,
            limit: self.limit,
            filter: self.filter,
            resume: self.resume,
            selection: Some(V::attributes),
            decode_item: V::from_item,
        }
    }
    pub fn build(self) -> Result<PreparedQuery<R, I, P>, BuildError> {
        let keys = self.index.map_or(R::SCHEMA.primary, |index| index.keys);
        if keys != I::KEYS
            || self
                .index
                .is_some_and(|index| !R::SCHEMA.indexes.contains(&index))
        {
            return Err(BuildError::IncompatibleIndex);
        }
        if self.consistency == ReadConsistency::Strong
            && self
                .index
                .is_some_and(|index| index.kind == IndexKind::Global)
        {
            return Err(BuildError::UnsupportedConsistency);
        }
        let limit = self
            .limit
            .map(|limit| {
                i32::try_from(limit.get()).map_err(|_| BuildError::RequestLimit {
                    max: i32::MAX as usize,
                    actual: limit.get() as usize,
                })
            })
            .transpose()?;
        let key = self.key.build()?;
        let filter = self.filter.map(Condition::build).transpose()?;
        if let Some(filter) = &filter {
            visit(filter.root(), &mut |field| {
                if field.index == keys.partition || Some(field.index) == keys.sort {
                    Err(BuildError::OperationConstraint {
                        code: "query_key_in_filter",
                    })
                } else {
                    Ok(())
                }
            })?;
        }
        let selection = self.selection.map(|select| select()).transpose()?;
        if selection.as_ref().is_some_and(Vec::is_empty) {
            return Err(BuildError::EmptyOperation);
        }
        if let Some(index) = self
            .index
            .filter(|index| index.projection != IndexProjection::All)
        {
            let mut available = Vec::new();
            for keys in [R::SCHEMA.primary, index.keys] {
                for position in [Some(keys.partition), keys.sort].into_iter().flatten() {
                    available.push(
                        R::SCHEMA
                            .fields
                            .get(position)
                            .ok_or(BuildError::IncompatibleIndex)?
                            .name,
                    );
                }
            }
            if let IndexProjection::Include(names) = index.projection {
                available.extend_from_slice(names);
            }
            if selection
                .as_ref()
                .is_none_or(|names| names.iter().any(|name| !available.contains(name)))
            {
                return Err(BuildError::OperationConstraint {
                    code: "projection_not_available",
                });
            }
            if let Some(filter) = &filter {
                visit(filter.root(), &mut |field| {
                    if available.contains(&field.name) {
                        Ok(())
                    } else {
                        Err(BuildError::OperationConstraint {
                            code: "projection_not_available",
                        })
                    }
                })?;
            }
        }
        let binding = CursorBinding::query(
            &self.table,
            self.index,
            &key,
            filter.as_ref(),
            selection.clone(),
            self.consistency,
            self.order,
        )?;
        let start = self
            .resume
            .as_ref()
            .map(|cursor| binding.resume(cursor).cloned())
            .transpose()?;
        let mut expressions = key.render();
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
        let input = QueryInput::builder()
            .table_name(self.table.name())
            .set_index_name(self.index.map(|index| index.name.into()))
            .consistent_read(self.consistency == ReadConsistency::Strong)
            .scan_index_forward(self.order == QueryOrder::Ascending)
            .set_limit(limit)
            .set_exclusive_start_key(start)
            .set_return_consumed_capacity(self.capacity.sdk())
            .set_key_condition_expression(expressions.key_condition.clone())
            .set_filter_expression(expressions.filter.clone())
            .set_projection_expression(expressions.projection.clone())
            .set_expression_attribute_names(Some(expressions.names().clone()))
            .set_expression_attribute_values(Some(expressions.values().clone()))
            .build()
            .map_err(|_| BuildError::OperationConstraint {
                code: "invalid_query_input",
            })?;
        Ok(PreparedQuery {
            request: PreparedRequest {
                input,
                expressions,
                decode_item: self.decode_item,
            },
            binding,
        })
    }
}
fn visit(
    node: &ConditionNode,
    check: &mut impl FnMut(&crate::FieldDescriptor) -> Result<(), BuildError>,
) -> Result<(), BuildError> {
    match node {
        ConditionNode::Predicate { operands, .. } => {
            for operand in operands {
                if let Operand::Attribute(field) | Operand::Size(field) = operand {
                    check(field)?;
                }
            }
            Ok(())
        }
        ConditionNode::And(a, b) | ConditionNode::Or(a, b) => {
            visit(a, check)?;
            visit(b, check)
        }
        ConditionNode::Not(a) => visit(a, check),
    }
}
impl<R, I, P> PreparedQuery<R, I, P> {
    pub fn input(&self) -> &QueryInput {
        self.request.input()
    }
    pub fn expressions(&self) -> &RenderedExpressions {
        self.request.expressions()
    }
    /// Replace every modeled field, clearing stale legacy fields without I/O.
    pub fn apply(&self, builder: QueryFluentBuilder) -> QueryFluentBuilder {
        let input = self.input();
        builder
            .set_table_name(input.table_name.clone())
            .set_index_name(input.index_name.clone())
            .set_select(input.select.clone())
            .set_attributes_to_get(input.attributes_to_get.clone())
            .set_limit(input.limit)
            .set_consistent_read(input.consistent_read)
            .set_key_conditions(input.key_conditions.clone())
            .set_query_filter(input.query_filter.clone())
            .set_conditional_operator(input.conditional_operator.clone())
            .set_scan_index_forward(input.scan_index_forward)
            .set_exclusive_start_key(input.exclusive_start_key.clone())
            .set_return_consumed_capacity(input.return_consumed_capacity.clone())
            .set_projection_expression(input.projection_expression.clone())
            .set_filter_expression(input.filter_expression.clone())
            .set_key_condition_expression(input.key_condition_expression.clone())
            .set_expression_attribute_names(input.expression_attribute_names.clone())
            .set_expression_attribute_values(input.expression_attribute_values.clone())
    }
}
impl<R: Record, I, P> PreparedQuery<R, I, P> {
    pub fn decode(
        &self,
        result: Result<QueryOutput, SdkError<QueryError>>,
    ) -> Result<Page<P, Cursor<R, I>>, ReadError<QueryError>> {
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
impl<R, I, P> fmt::Debug for PreparedQuery<R, I, P> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.request.fmt(f)
    }
}
