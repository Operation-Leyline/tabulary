//! Frozen transaction actions reuse the single-write preparation paths.
use std::fmt;

use aws_sdk_dynamodb::types::{
    self, AttributeValue, ReturnItemCollectionMetrics, ReturnValue, TransactWriteItem,
};

use crate::{
    BuildError, Condition, Decimal, DeleteBuilder, Document, EncodeContext, EncodeFault, Item,
    OldItemMode, PathSegment, PreparedDelete, PreparedPut, PreparedRequest, PreparedUpdate,
    PutBuilder, Record, RecordKey, RenderedExpressions, Table, UpdateBuilder,
};

/// One frozen action, retaining its record decoder for typed cancellation results.
pub struct TxAction<R> {
    pub(crate) prepared: PreparedRequest<TransactWriteItem, R>,
    #[allow(
        dead_code,
        reason = "consumed by the milestone 39 request builder in the stacked follow-up"
    )]
    pub(crate) target: (String, Item),
}
impl<R> TxAction<R> {
    pub fn input(&self) -> &TransactWriteItem {
        self.prepared.input()
    }
    pub fn expressions(&self) -> &RenderedExpressions {
        self.prepared.expressions()
    }
}
impl<R> fmt::Debug for TxAction<R> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("TxAction([redacted])")
    }
}
fn constraint(code: &'static str) -> BuildError {
    BuildError::OperationConstraint { code }
}

// Extract physical keys from already encoded inputs; never run a second codec.
fn target_key<R: Record>(item: &Item) -> Result<Item, BuildError> {
    let mut key = Item::new();
    for (index, max) in [
        (Some(R::SCHEMA.primary.partition), 2048),
        (R::SCHEMA.primary.sort, 1024),
    ] {
        let Some(index) = index else { continue };
        let field = R::SCHEMA
            .fields
            .get(index)
            .ok_or_else(|| constraint("invalid_transaction_key_schema"))?;
        let cx = EncodeContext::new(R::SCHEMA.name).at(PathSegment::Attribute(field.name.into()));
        let value = item.get(field.name).ok_or_else(|| {
            cx.error(EncodeFault::Custom {
                code: "missing_transaction_key",
            })
        })?;
        crate::record::validate_key(value, max).map_err(|fault| cx.error(fault))?;
        // Numeric spelling is irrelevant to DynamoDB target identity.
        let value = match value {
            AttributeValue::N(n) => AttributeValue::N(
                n.parse::<Decimal>()
                    .map_err(|_| cx.error(EncodeFault::NumberRange))?
                    .to_string(),
            ),
            value => value.clone(),
        };
        key.insert(field.name.into(), value);
    }
    Ok(key)
}

macro_rules! conversion {
    ($prepared:ident, $sdk:ident, $kind:ident, $payload:ident, $setter:ident $(, $extra:ident = $field:ident)?) => {
        impl<R: Record + Document> $prepared<R> {
            /// Preserve the validated/rendered input; refuse standalone return/accounting options.
            pub fn into_tx(self) -> Result<TxAction<R>, BuildError> {
                let input = self.input;
                if input.return_values.as_ref().is_some_and(|v| *v != ReturnValue::None)
                    || input.return_consumed_capacity.is_some()
                    || input.return_item_collection_metrics.as_ref().is_some_and(|v| *v != ReturnItemCollectionMetrics::None) {
                    return Err(constraint("transaction_single_write_options"));
                }
                let table = input.table_name.clone().ok_or_else(|| constraint("invalid_transaction_action"))?;
                let key = target_key::<R>(input.$payload.as_ref().ok_or_else(|| constraint("invalid_transaction_action"))?)?;
                let action = types::$sdk::builder()
                    .set_table_name(input.table_name)
                    .$setter(input.$payload)
                    .set_condition_expression(input.condition_expression)
                    .set_expression_attribute_names(input.expression_attribute_names)
                    .set_expression_attribute_values(input.expression_attribute_values)
                    .set_return_values_on_condition_check_failure(input.return_values_on_condition_check_failure)
                    $(.$extra(input.$field))?
                    .build().map_err(|_| constraint("invalid_transaction_action"))?;
                Ok(TxAction {
                    prepared: PreparedRequest {
                        input: TransactWriteItem::builder().$kind(action).build(),
                        expressions: self.expressions,
                        decode_item: self.decode_item,
                    },
                    target: (table, key),
                })
            }
        }
    };
}
conversion!(PreparedPut, Put, put, item, set_item);
conversion!(PreparedDelete, Delete, delete, key, set_key);
conversion!(
    PreparedUpdate,
    Update,
    update,
    key,
    set_key,
    set_update_expression = update_expression
);

impl<R: Record + Document> PutBuilder<'_, R> {
    pub fn into_tx(self) -> Result<TxAction<R>, BuildError> {
        self.build()?.into_tx()
    }
}
impl<R: Record + Document, K: RecordKey<R>> DeleteBuilder<R, K> {
    pub fn into_tx(self) -> Result<TxAction<R>, BuildError> {
        self.build()?.into_tx()
    }
}
impl<R: Record + Document, K: RecordKey<R>> UpdateBuilder<R, K> {
    pub fn into_tx(self) -> Result<TxAction<R>, BuildError> {
        self.build()?.into_tx()
    }
}

/// A transaction-only condition, using the same key and condition preparation as delete.
pub struct CheckBuilder<R, K> {
    delete: DeleteBuilder<R, K>,
}
impl<R: Record + Document> Table<R> {
    pub fn check<K: RecordKey<R>>(&self, key: K, condition: Condition<R>) -> CheckBuilder<R, K> {
        CheckBuilder {
            delete: self.delete(key).condition(condition),
        }
    }
}
impl<R: Record + Document, K: RecordKey<R>> CheckBuilder<R, K> {
    pub fn on_condition_failure(mut self, mode: OldItemMode) -> Self {
        self.delete = self.delete.on_condition_failure(mode);
        self
    }
    pub fn into_tx(self) -> Result<TxAction<R>, BuildError> {
        let mut action = self.delete.into_tx()?;
        let delete = action
            .prepared
            .input
            .delete
            .take()
            .ok_or_else(|| constraint("invalid_transaction_action"))?;
        let check = types::ConditionCheck::builder()
            .table_name(delete.table_name)
            .set_key(Some(delete.key))
            .set_condition_expression(delete.condition_expression)
            .set_expression_attribute_names(delete.expression_attribute_names)
            .set_expression_attribute_values(delete.expression_attribute_values)
            .set_return_values_on_condition_check_failure(
                delete.return_values_on_condition_check_failure,
            )
            .build()
            .map_err(|_| constraint("invalid_transaction_action"))?;
        action.prepared.input.condition_check = Some(check);
        Ok(action)
    }
}
