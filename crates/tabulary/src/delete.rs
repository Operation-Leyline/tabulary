//! DeleteItem preparation and decoding; the caller alone sends the SDK request.
use aws_sdk_dynamodb::{
    error::SdkError,
    operation::delete_item::{
        DeleteItemError, DeleteItemInput, DeleteItemOutput, builders::DeleteItemFluentBuilder,
    },
    types::ReturnValuesOnConditionCheckFailure,
};

use crate::{
    BuildError, Capacity, CollectionMetrics, Condition, DecodeContext, Document, OldItemMode,
    PreparedRequest, Record, RecordKey, Redacted, RenderedExpressions, ReturnOld, Table,
    WriteDecodeError, WriteError, WriteResult,
};

pub type PreparedDelete<R> = PreparedRequest<DeleteItemInput, R>;

/// Owns its key and table binding. All fallible encoding happens at build.
pub struct DeleteBuilder<R, K> {
    table: String,
    key: K,
    condition: Option<Condition<R>>,
    return_item: ReturnOld,
    old_item_mode: OldItemMode,
    capacity: Capacity,
    collection_metrics: CollectionMetrics,
}
impl<R: Record + Document> Table<R> {
    pub fn delete<K: RecordKey<R>>(&self, key: K) -> DeleteBuilder<R, K> {
        DeleteBuilder {
            table: self.name.clone(),
            key,
            condition: None,
            return_item: ReturnOld::None,
            old_item_mode: OldItemMode::None,
            capacity: Capacity::None,
            collection_metrics: CollectionMetrics::None,
        }
    }
}
impl<R: Record + Document, K: RecordKey<R>> DeleteBuilder<R, K> {
    /// Replace the inline condition. Compose predicates explicitly with `and`/`or`.
    pub fn condition(mut self, condition: Condition<R>) -> Self {
        self.condition = Some(condition);
        self
    }
    pub fn return_item(mut self, choice: ReturnOld) -> Self {
        self.return_item = choice;
        self
    }
    pub fn on_condition_failure(mut self, mode: OldItemMode) -> Self {
        self.old_item_mode = mode;
        self
    }
    pub fn capacity(mut self, capacity: Capacity) -> Self {
        self.capacity = capacity;
        self
    }
    pub fn collection_metrics(mut self, metrics: CollectionMetrics) -> Self {
        self.collection_metrics = metrics;
        self
    }
    pub fn build(self) -> Result<PreparedDelete<R>, BuildError> {
        let key = self.key.encode_key()?;
        let expressions = match self.condition {
            Some(condition) => condition.build()?.render(),
            None => RenderedExpressions::default(),
        };
        let names = (!expressions.names().is_empty()).then(|| expressions.names().clone());
        let values = (!expressions.values().is_empty()).then(|| expressions.values().clone());
        let failure_return = match self.old_item_mode {
            OldItemMode::None => None,
            OldItemMode::All => Some(ReturnValuesOnConditionCheckFailure::AllOld),
        };
        let input = DeleteItemInput::builder()
            .table_name(self.table)
            .set_key(Some(key))
            .return_values(self.return_item.into())
            .set_return_values_on_condition_check_failure(failure_return)
            .set_return_consumed_capacity(self.capacity.sdk())
            .return_item_collection_metrics(self.collection_metrics.into())
            .set_condition_expression(expressions.condition.clone())
            .set_expression_attribute_names(names)
            .set_expression_attribute_values(values)
            .build()
            .map_err(|_| BuildError::OperationConstraint {
                code: "invalid_delete_input",
            })?;
        Ok(PreparedRequest {
            input,
            expressions,
            decode_item: |item| R::decode_map(item, &mut DecodeContext::new(R::NAME)),
        })
    }
}
impl<R: Record + Document> PreparedDelete<R> {
    /// Replace every modeled input field, including absent legacy fields.
    /// Transport configuration on the supplied fluent builder remains unchanged.
    pub fn apply(&self, builder: DeleteItemFluentBuilder) -> DeleteItemFluentBuilder {
        let input = &self.input;
        builder
            .set_table_name(input.table_name.clone())
            .set_key(input.key.clone())
            .set_expected(input.expected.clone())
            .set_conditional_operator(input.conditional_operator.clone())
            .set_return_values(input.return_values.clone())
            .set_return_consumed_capacity(input.return_consumed_capacity.clone())
            .set_return_item_collection_metrics(input.return_item_collection_metrics.clone())
            .set_condition_expression(input.condition_expression.clone())
            .set_expression_attribute_names(input.expression_attribute_names.clone())
            .set_expression_attribute_values(input.expression_attribute_values.clone())
            .set_return_values_on_condition_check_failure(
                input.return_values_on_condition_check_failure.clone(),
            )
    }
    #[expect(
        clippy::result_large_err,
        reason = "the shared WriteError contract retains the decoded old record inline"
    )]
    pub fn decode(
        &self,
        result: Result<DeleteItemOutput, SdkError<DeleteItemError>>,
    ) -> Result<WriteResult<R>, WriteError<R, DeleteItemError>> {
        let mode = match self.input.return_values_on_condition_check_failure {
            Some(ReturnValuesOnConditionCheckFailure::AllOld) => OldItemMode::All,
            _ => OldItemMode::None,
        };
        let output =
            result.map_err(|error| WriteError::<R, DeleteItemError>::from_sdk(error, mode))?;
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
