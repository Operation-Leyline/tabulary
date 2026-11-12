#![cfg(feature = "derive")]

use aws_sdk_dynamodb::types::{AttributeValue as Av, ReturnValuesOnConditionCheckFailure};
use tabulary::{
    BuildError, Capacity, CollectionMetrics, OldItemMode, ReturnItem, ReturnOld, Table, Update,
};

#[derive(tabulary::Record)]
#[tabulary(rename_all = "snake_case")]
struct Order {
    #[tabulary(partition_key)]
    id: String,
    #[tabulary(sort_key)]
    line: u64,
    quantity: u64,
}
#[derive(tabulary::Record)]
#[tabulary(rename_all = "snake_case")]
struct Session {
    #[tabulary(partition_key)]
    id: String,
    usable: bool,
}
fn key(line: u64) -> OrderKey {
    OrderKey::new("order".into(), line).unwrap()
}
fn changes() -> Update<Order> {
    Update::new().and(OrderAttrs::QUANTITY.set(7))
}

mod counted {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tabulary::{DecodeContext, DecodeError, EncodeContext, EncodeError, Value};
    pub static CALLS: AtomicUsize = AtomicUsize::new(0);
    pub struct Amount(pub u64);
    impl Value for Amount {
        type Wire = tabulary::wire::Number;
        fn encode_value(&self, cx: &mut EncodeContext) -> Result<super::Av, EncodeError> {
            CALLS.fetch_add(1, Ordering::SeqCst);
            self.0.encode_value(cx)
        }
        fn decode_value(value: &super::Av, cx: &mut DecodeContext) -> Result<Self, DecodeError> {
            u64::decode_value(value, cx).map(Self)
        }
    }
}

#[test]
fn conversion_never_reencodes_the_item_or_expression_constants() {
    use std::sync::atomic::Ordering;
    #[derive(tabulary::Record)]
    #[tabulary(rename_all = "snake_case")]
    struct Invoice {
        #[tabulary(partition_key)]
        id: String,
        amount: counted::Amount,
    }
    let table = Table::<Invoice>::new("invoices").unwrap();
    let record = Invoice {
        id: "invoice".into(),
        amount: counted::Amount(7),
    };
    let prepared = table
        .put(&record)
        .condition(InvoiceAttrs::AMOUNT.eq(counted::Amount(6)))
        .build()
        .unwrap();
    assert_eq!(counted::CALLS.load(Ordering::SeqCst), 2);
    let _action = prepared.into_tx().unwrap();
    assert_eq!(counted::CALLS.load(Ordering::SeqCst), 2);
}

#[test]
fn mixed_actions_reuse_prepared_inputs_with_action_local_bindings() {
    let table = Table::<Order>::new("orders").unwrap();
    let sessions = Table::<Session>::new("sessions").unwrap();
    let record = Order {
        id: "order".into(),
        line: 1,
        quantity: 6,
    };
    let put = table
        .put(&record)
        .condition(OrderAttrs::QUANTITY.eq(6))
        .on_condition_failure(OldItemMode::All)
        .build()
        .unwrap();
    let update = table
        .update(key(2))
        .changes(changes())
        .condition(OrderAttrs::QUANTITY.eq(6))
        .on_condition_failure(OldItemMode::All)
        .build()
        .unwrap();
    let delete = table
        .delete(key(3))
        .condition(OrderAttrs::QUANTITY.eq(8))
        .on_condition_failure(OldItemMode::All)
        .build()
        .unwrap();
    let (p, u, d) = (
        put.input().clone(),
        update.input().clone(),
        delete.input().clone(),
    );
    let check = sessions
        .check(
            SessionKey::new("session".into()).unwrap(),
            SessionAttrs::USABLE.eq(true),
        )
        .on_condition_failure(OldItemMode::All)
        .into_tx()
        .unwrap();
    assert_eq!(check.expressions().condition(), Some("#n0 = :v0"));
    assert_eq!(format!("{check:?}"), "TxAction([redacted])");
    let items = [
        put.into_tx().unwrap().input().clone(),
        update.into_tx().unwrap().input().clone(),
        delete.into_tx().unwrap().input().clone(),
        check.input().clone(),
    ];
    assert_eq!(items.len(), 4);
    let put = items[0].put().unwrap();
    assert_eq!(Some(&put.item), p.item.as_ref());
    assert_eq!(put.condition_expression, p.condition_expression);
    assert_eq!(put.expression_attribute_names, p.expression_attribute_names);
    assert_eq!(
        put.expression_attribute_values,
        p.expression_attribute_values
    );
    let update = items[1].update().unwrap();
    assert_eq!(Some(&update.key), u.key.as_ref());
    assert_eq!(
        Some(&update.update_expression),
        u.update_expression.as_ref()
    );
    assert_eq!(update.condition_expression, u.condition_expression);
    assert_eq!(
        update.expression_attribute_names,
        u.expression_attribute_names
    );
    assert_eq!(
        update.expression_attribute_values,
        u.expression_attribute_values
    );
    let delete = items[2].delete().unwrap();
    assert_eq!(Some(&delete.key), d.key.as_ref());
    assert_eq!(delete.condition_expression, d.condition_expression);
    assert_eq!(
        delete.expression_attribute_names,
        d.expression_attribute_names
    );
    assert_eq!(
        delete.expression_attribute_values,
        d.expression_attribute_values
    );
    let check = items[3].condition_check().unwrap();
    assert_eq!(check.key["id"], Av::S("session".into()));
    assert_eq!(
        check.expression_attribute_names.as_ref().unwrap()["#n0"],
        "usable"
    );
    assert_eq!(
        check.expression_attribute_values.as_ref().unwrap()[":v0"],
        Av::Bool(true)
    );
    for mode in [
        &put.return_values_on_condition_check_failure,
        &update.return_values_on_condition_check_failure,
        &delete.return_values_on_condition_check_failure,
        &check.return_values_on_condition_check_failure,
    ] {
        assert_eq!(*mode, Some(ReturnValuesOnConditionCheckFailure::AllOld));
    }
}

#[test]
fn conversion_keeps_validation_and_omits_unrequested_fields() {
    let table = Table::<Order>::new("orders").unwrap();
    let action = table.delete(key(1)).into_tx().unwrap();
    let delete = action.input().delete().unwrap();
    assert!(delete.condition_expression.is_none());
    assert!(delete.expression_attribute_names.is_none());
    assert!(delete.expression_attribute_values.is_none());
    assert!(delete.return_values_on_condition_check_failure.is_none());
    assert!(matches!(
        table
            .check(key(1), OrderAttrs::QUANTITY.in_values(vec![]))
            .into_tx(),
        Err(BuildError::EmptyOperation)
    ));
    let invalid = Order {
        id: String::new(),
        line: 1,
        quantity: 6,
    };
    assert!(matches!(
        table.put(&invalid).into_tx(),
        Err(BuildError::Encode(_))
    ));
}

#[test]
fn single_write_options_are_refused_instead_of_silently_dropped() {
    let table = Table::<Order>::new("orders").unwrap();
    let record = Order {
        id: "order".into(),
        line: 1,
        quantity: 6,
    };
    for result in [
        table.put(&record).return_item(ReturnOld::AllOld).into_tx(),
        table.put(&record).capacity(Capacity::Total).into_tx(),
        table
            .delete(key(1))
            .return_item(ReturnOld::AllOld)
            .into_tx(),
        table.delete(key(1)).capacity(Capacity::Indexes).into_tx(),
        table
            .update(key(1))
            .changes(changes())
            .return_item(ReturnItem::AllNew)
            .into_tx(),
        table
            .update(key(1))
            .changes(changes())
            .return_item(ReturnItem::AllOld)
            .into_tx(),
        table
            .update(key(1))
            .changes(changes())
            .capacity(Capacity::Total)
            .into_tx(),
        table
            .put(&record)
            .collection_metrics(CollectionMetrics::Size)
            .into_tx(),
    ] {
        assert!(matches!(
            result,
            Err(BuildError::OperationConstraint {
                code: "transaction_single_write_options"
            })
        ));
    }
    assert!(matches!(
        table.update(key(1)).into_tx(),
        Err(BuildError::EmptyOperation)
    ));
}
