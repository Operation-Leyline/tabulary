#![cfg(feature = "derive")]

#[path = "endpoint/support.rs"]
mod support;
#[path = "endpoint/transactions_batches.rs"]
mod transactions_batches;

use aws_sdk_dynamodb::Client;
use tabulary::{
    FailureKind, OldItem, OldItemMode, ReadConsistency, ReturnItem, ReturnOld, Table, Update,
    WriteError,
};

#[derive(Debug, PartialEq, tabulary::Record)]
#[tabulary(rename_all = "snake_case")]
struct Order {
    #[tabulary(partition_key)]
    id: String,
    #[tabulary(sort_key)]
    line: u64,
    // Reserved word and literal-dot names exercise the actual expression parser.
    #[tabulary(rename = "size")]
    quantity: u64,
    #[tabulary(rename = "gift.note")]
    note: Option<String>,
}

fn order(quantity: u64) -> Order {
    Order {
        id: "order-1".into(),
        line: 2,
        quantity,
        note: Some("gift".into()),
    }
}

fn key() -> OrderKey {
    OrderKey::new("order-1".into(), 2).unwrap()
}

async fn read(client: &Client, table: &Table<Order>) -> Option<Order> {
    let request = table.get(key(), ReadConsistency::Strong).build().unwrap();
    assert_eq!(request.input().consistent_read(), Some(true));
    request
        .decode(request.apply(client.get_item()).send().await)
        .unwrap()
        .item
}

#[tokio::test]
#[ignore = "requires explicit TABULARY_DYNAMODB_ENDPOINT, AWS region and credentials"]
async fn crud_with_strong_reads_and_full_return_values() {
    support::with_table::<Order, _, _>(|client, name| async move {
        let table = Table::<Order>::new(name).unwrap();
        assert_eq!(read(&client, &table).await, None);
        let record = order(3);
        let put = table
            .put(&record)
            .condition(OrderAttrs::ID.not_exists())
            .build()
            .unwrap();
        assert!(
            put.decode(put.apply(client.put_item()).send().await)
                .unwrap()
                .item
                .is_none()
        );
        assert_eq!(read(&client, &table).await, Some(order(3)));

        let update = table
            .update(key())
            .changes(
                Update::new()
                    .and(OrderAttrs::QUANTITY.set(7))
                    .and(OrderAttrs::NOTE.remove()),
            )
            .condition(OrderAttrs::QUANTITY.eq(3))
            .return_item(ReturnItem::AllNew)
            .build()
            .unwrap();
        let updated = Order {
            note: None,
            ..order(7)
        };
        assert_eq!(
            update
                .decode(update.apply(client.update_item()).send().await)
                .unwrap()
                .item,
            Some(updated)
        );
        assert_eq!(
            read(&client, &table).await,
            Some(Order {
                note: None,
                ..order(7)
            })
        );

        let delete = table
            .delete(key())
            .return_item(ReturnOld::AllOld)
            .build()
            .unwrap();
        assert_eq!(
            delete
                .decode(delete.apply(client.delete_item()).send().await)
                .unwrap()
                .item,
            Some(Order {
                note: None,
                ..order(7)
            })
        );
        assert_eq!(read(&client, &table).await, None);
    })
    .await;
}

fn condition_old_item<E>(error: WriteError<Order, E>, expected: &Order) {
    let WriteError::Refused { failure, source } = error else {
        panic!("expected modeled conditional refusal: {error:?}");
    };
    assert!(source.as_service_error().is_some());
    assert_eq!(failure.kind, FailureKind::ConditionFailed);
    let OldItem::Decoded(old) = failure.old else {
        panic!(
            "endpoint must supply a decodable ALL_OLD item: {:?}",
            failure.old
        );
    };
    assert_eq!(&old, expected);
}

#[tokio::test]
#[ignore = "requires explicit TABULARY_DYNAMODB_ENDPOINT, AWS region and credentials"]
async fn conditional_put_update_delete_return_typed_old_items_without_mutation() {
    support::with_table::<Order, _, _>(|client, name| async move {
        let table = Table::<Order>::new(name).unwrap();
        let original = order(3);
        let seed = table.put(&original).build().unwrap();
        seed.decode(seed.apply(client.put_item()).send().await)
            .unwrap();

        let replacement = order(99);
        let put = table
            .put(&replacement)
            .condition(OrderAttrs::ID.not_exists())
            .on_condition_failure(OldItemMode::All)
            .build()
            .unwrap();
        let raw = put.apply(client.put_item()).send().await;
        assert!(
            raw.as_ref()
                .unwrap_err()
                .as_service_error()
                .is_some_and(|e| e.is_conditional_check_failed_exception())
        );
        condition_old_item(put.decode(raw).unwrap_err(), &original);
        assert_eq!(read(&client, &table).await, Some(order(3)));

        let update = table
            .update(key())
            .changes(Update::new().and(OrderAttrs::QUANTITY.set(99)))
            .condition(OrderAttrs::QUANTITY.eq(0))
            .on_condition_failure(OldItemMode::All)
            .build()
            .unwrap();
        let raw = update.apply(client.update_item()).send().await;
        assert!(
            raw.as_ref()
                .unwrap_err()
                .as_service_error()
                .is_some_and(|e| e.is_conditional_check_failed_exception())
        );
        condition_old_item(update.decode(raw).unwrap_err(), &original);
        assert_eq!(read(&client, &table).await, Some(order(3)));

        let delete = table
            .delete(key())
            .condition(OrderAttrs::QUANTITY.eq(0))
            .on_condition_failure(OldItemMode::All)
            .build()
            .unwrap();
        let raw = delete.apply(client.delete_item()).send().await;
        assert!(
            raw.as_ref()
                .unwrap_err()
                .as_service_error()
                .is_some_and(|e| e.is_conditional_check_failed_exception())
        );
        condition_old_item(delete.decode(raw).unwrap_err(), &original);
        assert_eq!(read(&client, &table).await, Some(original));
    })
    .await;
}
