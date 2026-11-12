//! PutItem preparation and decoding; the caller alone sends the SDK request.
use aws_sdk_dynamodb::{
    error::SdkError,
    operation::put_item::{
        PutItemError, PutItemInput, PutItemOutput, builders::PutItemFluentBuilder,
    },
    types::ReturnValuesOnConditionCheckFailure,
};

use crate::{
    BilledSize, BuildError, Capacity, CollectionMetrics, Condition, DecodeContext, Document,
    EncodeContext, OldItemMode, PreparedRequest, Record, Redacted, RenderedExpressions, ReturnOld,
    Table, WriteDecodeError, WriteError, WriteResult,
};

pub type PreparedPut<R> = PreparedRequest<PutItemInput, R>;

/// Borrows the proposed record until build; the prepared request owns its encoded item.
pub struct PutBuilder<'a, R> {
    table: String,
    record: &'a R,
    condition: Option<Condition<R>>,
    old_item: OldItemMode,
    return_item: ReturnOld,
    capacity: Capacity,
    metrics: CollectionMetrics,
}
impl<R: Record + Document> Table<R> {
    pub fn put<'a>(&self, record: &'a R) -> PutBuilder<'a, R> {
        PutBuilder {
            table: self.name.clone(),
            record,
            condition: None,
            old_item: OldItemMode::None,
            return_item: ReturnOld::None,
            capacity: Capacity::None,
            metrics: CollectionMetrics::None,
        }
    }
}
impl<R: Record + Document> PutBuilder<'_, R> {
    /// AND this predicate with any existing inline condition.
    pub fn condition(mut self, condition: Condition<R>) -> Self {
        self.condition = Some(match self.condition {
            Some(existing) => existing.and(condition),
            None => condition,
        });
        self
    }
    pub fn on_condition_failure(mut self, mode: OldItemMode) -> Self {
        self.old_item = mode;
        self
    }
    pub fn return_item(mut self, choice: ReturnOld) -> Self {
        self.return_item = choice;
        self
    }
    pub fn capacity(mut self, capacity: Capacity) -> Self {
        self.capacity = capacity;
        self
    }
    pub fn collection_metrics(mut self, metrics: CollectionMetrics) -> Self {
        self.metrics = metrics;
        self
    }
    pub fn build(self) -> Result<PreparedPut<R>, BuildError> {
        let item = self.record.encode_map(&mut EncodeContext::new(R::NAME))?;
        BilledSize::of_item(&item)?.ensure_at_most(400 * 1024)?;
        let expressions = match self.condition {
            Some(condition) => condition.build()?.render(),
            None => RenderedExpressions::default(),
        };
        let names = (!expressions.names().is_empty()).then(|| expressions.names().clone());
        let values = (!expressions.values().is_empty()).then(|| expressions.values().clone());
        let input = PutItemInput::builder()
            .table_name(self.table)
            .set_item(Some(item))
            .set_condition_expression(expressions.condition.clone())
            .set_expression_attribute_names(names)
            .set_expression_attribute_values(values)
            .return_values(self.return_item.into())
            .set_return_consumed_capacity(self.capacity.sdk())
            .return_item_collection_metrics(self.metrics.into())
            .set_return_values_on_condition_check_failure(match self.old_item {
                OldItemMode::None => None,
                OldItemMode::All => Some(ReturnValuesOnConditionCheckFailure::AllOld),
            })
            .build()
            .map_err(|_| BuildError::OperationConstraint {
                code: "invalid_put_input",
            })?;
        Ok(PreparedRequest {
            input,
            expressions,
            decode_item: |item| R::decode_map(item, &mut DecodeContext::new(R::NAME)),
        })
    }
}
impl<R: Record + Document> PreparedPut<R> {
    /// Replace all modeled fields, including absent legacy fields, without I/O.
    /// Transport configuration on the caller's fluent builder remains unchanged.
    pub fn apply(&self, builder: PutItemFluentBuilder) -> PutItemFluentBuilder {
        let input = &self.input;
        builder
            .set_table_name(input.table_name.clone())
            .set_item(input.item.clone())
            .set_expected(input.expected.clone())
            .set_return_values(input.return_values.clone())
            .set_return_consumed_capacity(input.return_consumed_capacity.clone())
            .set_return_item_collection_metrics(input.return_item_collection_metrics.clone())
            .set_conditional_operator(input.conditional_operator.clone())
            .set_condition_expression(input.condition_expression.clone())
            .set_expression_attribute_names(input.expression_attribute_names.clone())
            .set_expression_attribute_values(input.expression_attribute_values.clone())
            .set_return_values_on_condition_check_failure(
                input.return_values_on_condition_check_failure.clone(),
            )
    }
    #[allow(
        clippy::result_large_err,
        reason = "the shared WriteError contract retains typed old records inline"
    )]
    pub fn decode(
        &self,
        result: Result<PutItemOutput, SdkError<PutItemError>>,
    ) -> Result<WriteResult<R>, WriteError<R, PutItemError>> {
        let mode = match self.input.return_values_on_condition_check_failure {
            Some(ReturnValuesOnConditionCheckFailure::AllOld) => OldItemMode::All,
            _ => OldItemMode::None,
        };
        let output =
            result.map_err(|error| WriteError::<R, PutItemError>::from_sdk(error, mode))?;
        let capacity = output.consumed_capacity;
        let item_collection_metrics = output.item_collection_metrics;
        let item = match output.attributes {
            None => None,
            Some(raw) => match (self.decode_item)(&raw) {
                Ok(record) => Some(record),
                Err(source) => {
                    return Err(WriteError::Decode(Box::new(WriteDecodeError {
                        source,
                        raw: Redacted::new(raw),
                        capacity,
                        item_collection_metrics,
                    })));
                }
            },
        };
        Ok(WriteResult {
            item,
            capacity,
            item_collection_metrics,
        })
    }
}
