//! UpdateItem preparation and decoding; the caller alone sends the SDK request.
use aws_sdk_dynamodb::{
    error::SdkError,
    operation::update_item::{
        UpdateItemError, UpdateItemInput, UpdateItemOutput, builders::UpdateItemFluentBuilder,
    },
    types::{ReturnValue, ReturnValuesOnConditionCheckFailure},
};

use crate::{
    BuildError, Capacity, CollectionMetrics, Condition, DecodeContext, DecodeFault, Document,
    OldItemMode, PreparedRequest, Record, RecordKey, Redacted, ReturnItem, Table, Update,
    WriteDecodeError, WriteError, WriteResult,
};

pub type PreparedUpdate<R> = PreparedRequest<UpdateItemInput, R>;

/// Owns its key, changes, and guards; all fallible encoding happens at build.
pub struct UpdateBuilder<R, K> {
    table: String,
    key: K,
    changes: Update<R>,
    condition: Option<Condition<R>>,
    version: crate::version::Policy,
    return_item: ReturnItem,
    old_item: OldItemMode,
    capacity: Capacity,
    collection_metrics: CollectionMetrics,
}
impl<R: Record + Document> Table<R> {
    pub fn update<K: RecordKey<R>>(&self, key: K) -> UpdateBuilder<R, K> {
        UpdateBuilder {
            table: self.name.clone(),
            key,
            changes: Update::new(),
            condition: None,
            version: crate::version::Policy::Missing,
            return_item: ReturnItem::None,
            old_item: OldItemMode::None,
            capacity: Capacity::None,
            collection_metrics: CollectionMetrics::None,
        }
    }
}
impl<R: Record + Document, K: RecordKey<R>> UpdateBuilder<R, K> {
    /// Guard the stored version and SET its checked successor atomically.
    pub fn expect_version(mut self, version: u64) -> Self {
        self.version
            .select(crate::version::Policy::Expected(version));
        self
    }
    /// Explicit maintenance bypass; no version guard or increment is added.
    pub fn unchecked_version(mut self) -> Self {
        self.version.select(crate::version::Policy::Unchecked);
        self
    }
    /// Replace the mutation set; a version guard may supply its only SET.
    pub fn changes(mut self, changes: Update<R>) -> Self {
        self.changes = changes;
        self
    }
    /// Repeated calls AND their predicates; a guard is never silently replaced.
    pub fn condition(mut self, condition: Condition<R>) -> Self {
        self.condition = Some(match self.condition {
            Some(previous) => previous.and(condition),
            None => condition,
        });
        self
    }
    pub fn return_item(mut self, choice: ReturnItem) -> Self {
        self.return_item = choice;
        self
    }
    pub fn on_condition_failure(mut self, choice: OldItemMode) -> Self {
        self.old_item = choice;
        self
    }
    pub fn capacity(mut self, choice: Capacity) -> Self {
        self.capacity = choice;
        self
    }
    pub fn collection_metrics(mut self, choice: CollectionMetrics) -> Self {
        self.collection_metrics = choice;
        self
    }
    /// Validate the expressions and key. The resulting stored item size remains
    /// unknown locally; the service checks its 400 KiB cap against the actual row.
    pub fn build(self) -> Result<PreparedUpdate<R>, BuildError> {
        let key = self.key.encode_key()?;
        let mut changes = self.changes;
        let condition = self
            .version
            .expand(crate::version::Write::Update(&mut changes), self.condition)?;
        let changes = changes.build_for_key(Some(&key))?;
        let expressions = match condition {
            Some(condition) => changes.render_with_condition(&condition.build()?),
            None => changes.render(),
        };
        expressions.validate()?;
        let input = UpdateItemInput::builder()
            .table_name(self.table)
            .set_key(Some(key))
            .set_update_expression(expressions.update.clone())
            .set_condition_expression(expressions.condition.clone())
            .set_expression_attribute_names(
                (!expressions.names().is_empty()).then(|| expressions.names().clone()),
            )
            .set_expression_attribute_values(
                (!expressions.values().is_empty()).then(|| expressions.values().clone()),
            )
            .return_values(self.return_item.into())
            .set_return_consumed_capacity(self.capacity.sdk())
            .return_item_collection_metrics(self.collection_metrics.into())
            .set_return_values_on_condition_check_failure(
                (self.old_item == OldItemMode::All)
                    .then_some(ReturnValuesOnConditionCheckFailure::AllOld),
            )
            .build()
            .map_err(|_| BuildError::OperationConstraint {
                code: "invalid_update_input",
            })?;
        Ok(PreparedRequest {
            input,
            expressions,
            decode_item: |item| R::decode_map(item, &mut DecodeContext::new(R::NAME)),
        })
    }
}
impl<R: Record + Document> PreparedUpdate<R> {
    /// Replace every modeled field, including absent legacy fields, without I/O.
    pub fn apply(&self, builder: UpdateItemFluentBuilder) -> UpdateItemFluentBuilder {
        let input = &self.input;
        builder
            .set_table_name(input.table_name.clone())
            .set_key(input.key.clone())
            .set_attribute_updates(input.attribute_updates.clone())
            .set_expected(input.expected.clone())
            .set_conditional_operator(input.conditional_operator.clone())
            .set_return_values(input.return_values.clone())
            .set_return_consumed_capacity(input.return_consumed_capacity.clone())
            .set_return_item_collection_metrics(input.return_item_collection_metrics.clone())
            .set_update_expression(input.update_expression.clone())
            .set_condition_expression(input.condition_expression.clone())
            .set_expression_attribute_names(input.expression_attribute_names.clone())
            .set_expression_attribute_values(input.expression_attribute_values.clone())
            .set_return_values_on_condition_check_failure(
                input.return_values_on_condition_check_failure.clone(),
            )
    }
    // The shared refusal owns R inline; retain the specified WriteError API.
    #[allow(clippy::result_large_err)]
    pub fn decode(
        &self,
        result: Result<UpdateItemOutput, SdkError<UpdateItemError>>,
    ) -> Result<WriteResult<R>, WriteError<R, UpdateItemError>> {
        let old_mode = match self.input.return_values_on_condition_check_failure {
            Some(ReturnValuesOnConditionCheckFailure::AllOld) => OldItemMode::All,
            _ => OldItemMode::None,
        };
        let output =
            result.map_err(|error| WriteError::<R, UpdateItemError>::from_sdk(error, old_mode))?;
        let capacity = output.consumed_capacity;
        let item_collection_metrics = output.item_collection_metrics;
        let shape_fault = match (&self.input.return_values, &output.attributes) {
            (Some(ReturnValue::None), Some(_)) => Some("unexpected_returned_attributes"),
            (Some(ReturnValue::AllNew), None) => Some("missing_returned_attributes"),
            _ => None,
        };
        let decoded = match (shape_fault, output.attributes.as_ref()) {
            (Some(code), _) => {
                Err(DecodeContext::new(R::NAME).error(DecodeFault::Validation { code }))
            }
            (None, Some(raw)) => (self.decode_item)(raw).map(Some),
            (None, None) => Ok(None),
        };
        let item = decoded.map_err(|source| {
            WriteError::Decode(Box::new(WriteDecodeError {
                source,
                raw: Redacted::new(output.attributes.unwrap_or_default()),
                capacity: capacity.clone(),
                item_collection_metrics: item_collection_metrics.clone(),
            }))
        })?;
        Ok(WriteResult {
            item,
            capacity,
            item_collection_metrics,
        })
    }
}
