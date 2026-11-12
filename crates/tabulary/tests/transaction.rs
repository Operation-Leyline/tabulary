#![cfg(feature = "derive")]

use aws_sdk_dynamodb::types::{
    AttributeValue as Av, ReturnConsumedCapacity, ReturnValuesOnConditionCheckFailure,
};
use tabulary::{BuildError, Capacity, CollectionMetrics, OldItemMode, Table, Update, transaction};

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
    let request = transaction((
        put.into_tx().unwrap(),
        update.into_tx().unwrap(),
        delete.into_tx().unwrap(),
        check,
    ))
    .capacity(Capacity::Indexes)
    .client_request_token("caller-token")
    .build()
    .unwrap();
    let items = request.input().transact_items();
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
    assert_eq!(request.input().client_request_token(), Some("caller-token"));
    assert_eq!(
        request.input().return_consumed_capacity(),
        Some(&ReturnConsumedCapacity::Indexes)
    );
    assert!(!format!("{request:?}").contains("caller-token"));
}

#[test]
fn duplicate_targets_are_rejected_across_records_actions_and_name_arn_bindings() {
    let table = Table::<Order>::new("orders").unwrap();
    let record = Order {
        id: "order".into(),
        line: 1,
        quantity: 6,
    };
    assert!(matches!(
        transaction((
            table.put(&record).into_tx().unwrap(),
            table.delete(key(1)).into_tx().unwrap()
        ))
        .build(),
        Err(BuildError::OperationConstraint {
            code: "duplicate_transaction_target"
        })
    ));
    let arn = Table::<Order>::new("arn:aws:dynamodb:us-east-1:123456789012:table/orders").unwrap();
    assert!(matches!(
        transaction((
            table.update(key(1)).changes(changes()).into_tx().unwrap(),
            arn.check(key(1), OrderAttrs::QUANTITY.eq(6))
                .into_tx()
                .unwrap()
        ))
        .build(),
        Err(BuildError::OperationConstraint {
            code: "duplicate_transaction_target"
        })
    ));
    assert!(
        transaction((
            table.delete(key(1)).into_tx().unwrap(),
            table.delete(key(2)).into_tx().unwrap()
        ))
        .build()
        .is_ok()
    );
    let other = Table::<Order>::new("other_orders").unwrap();
    assert!(
        transaction((
            table.delete(key(1)).into_tx().unwrap(),
            other.delete(key(1)).into_tx().unwrap()
        ))
        .build()
        .is_ok()
    );
}

#[test]
fn token_length_and_arn_location_are_validated() {
    let table = Table::<Order>::new("orders").unwrap();
    for token in ["".to_owned(), "a".repeat(37)] {
        assert!(matches!(
            transaction((table.delete(key(1)).into_tx().unwrap(),))
                .client_request_token(token)
                .build(),
            Err(BuildError::OperationConstraint {
                code: "invalid_client_request_token"
            })
        ));
    }
    for token in ["a".to_owned(), "a".repeat(36)] {
        assert!(
            transaction((table.delete(key(1)).into_tx().unwrap(),))
                .client_request_token(token)
                .build()
                .is_ok()
        );
    }
    for suffix in ["us-west-2:123456789012", "us-east-1:999999999999"] {
        let a =
            Table::<Order>::new("arn:aws:dynamodb:us-east-1:123456789012:table/orders").unwrap();
        let b = Table::<Order>::new(format!("arn:aws:dynamodb:{suffix}:table/orders")).unwrap();
        assert!(matches!(
            transaction((
                a.delete(key(1)).into_tx().unwrap(),
                b.delete(key(2)).into_tx().unwrap()
            ))
            .build(),
            Err(BuildError::OperationConstraint {
                code: "transaction_table_location"
            })
        ));
    }
}

#[test]
fn numeric_identity_ignores_spelling_and_record_type() {
    struct Alternate(OrderKey);
    impl tabulary::RecordKey<Order> for Alternate {
        fn encode_key(&self) -> Result<tabulary::Item, tabulary::EncodeError> {
            let mut item = self.0.encode_key()?;
            if let Some(Av::N(n)) = item.get_mut("line") {
                n.push_str(".0");
            }
            Ok(item)
        }
        fn decode_key(item: &tabulary::Item) -> Result<Self, tabulary::DecodeError> {
            OrderKey::decode_key(item).map(Self)
        }
    }
    let table = Table::<Order>::new("orders").unwrap();
    assert!(matches!(
        transaction((
            table.delete(key(1)).into_tx().unwrap(),
            table.delete(Alternate(key(1))).into_tx().unwrap()
        ))
        .build(),
        Err(BuildError::OperationConstraint {
            code: "duplicate_transaction_target"
        })
    ));
    #[derive(tabulary::Record)]
    #[tabulary(rename_all = "snake_case")]
    struct Invoice {
        #[tabulary(partition_key)]
        id: String,
        #[tabulary(sort_key)]
        line: u64,
        amount: u64,
    }
    let invoices = Table::<Invoice>::new("orders").unwrap();
    assert!(matches!(
        transaction((
            table.delete(key(1)).into_tx().unwrap(),
            invoices
                .delete(InvoiceKey::new("order".into(), 1).unwrap())
                .into_tx()
                .unwrap()
        ))
        .build(),
        Err(BuildError::OperationConstraint {
            code: "duplicate_transaction_target"
        })
    ));
}

#[test]
fn synthesized_partition_and_binary_sort_keys_define_the_target() {
    #[derive(tabulary::Record)]
    #[tabulary(
        rename_all = "snake_case",
        partition(name = "pk", template = "parcel!{region:hex}")
    )]
    struct Parcel {
        region: String,
        #[tabulary(sort_key)]
        token: Vec<u8>,
        delivered: bool,
    }
    let parcels = Table::<Parcel>::new("parcels").unwrap();
    let parcel = Parcel {
        region: "coast".into(),
        token: vec![1, 2],
        delivered: false,
    };
    assert!(matches!(
        transaction((
            parcels.put(&parcel).into_tx().unwrap(),
            parcels
                .check(
                    ParcelKey::new("coast".into(), vec![1, 2]).unwrap(),
                    ParcelAttrs::DELIVERED.eq(false)
                )
                .into_tx()
                .unwrap()
        ))
        .build(),
        Err(BuildError::OperationConstraint {
            code: "duplicate_transaction_target"
        })
    ));
    assert!(
        transaction((
            parcels.put(&parcel).into_tx().unwrap(),
            parcels
                .delete(ParcelKey::new("coast".into(), vec![1, 3]).unwrap())
                .into_tx()
                .unwrap()
        ))
        .build()
        .is_ok()
    );
}

#[test]
fn apply_copies_every_field_and_clears_stale_options_without_io() {
    use aws_sdk_dynamodb::{
        Client, Config,
        types::{ReturnItemCollectionMetrics, TransactWriteItem},
    };
    use aws_smithy_runtime_api::client::http::http_client_fn;
    let client = Client::from_conf(
        Config::builder()
            .behavior_version_latest()
            .region(aws_sdk_dynamodb::config::Region::new("us-east-1"))
            .retry_config(aws_sdk_dynamodb::config::retry::RetryConfig::disabled())
            .timeout_config(aws_sdk_dynamodb::config::timeout::TimeoutConfig::disabled())
            .identity_cache(aws_sdk_dynamodb::config::IdentityCache::no_cache())
            .stalled_stream_protection(
                aws_sdk_dynamodb::config::StalledStreamProtectionConfig::disabled(),
            )
            .http_client(http_client_fn(|_, _| {
                panic!("transaction must not use transport")
            }))
            .build(),
    );
    let table = Table::<Order>::new("orders").unwrap();
    for configured in [false, true] {
        let mut builder = transaction((table.delete(key(1)).into_tx().unwrap(),));
        if configured {
            builder = builder
                .capacity(Capacity::Total)
                .client_request_token("caller-token")
                .collection_metrics(CollectionMetrics::Size);
        }
        let request = builder.build().unwrap();
        let stale = client
            .transact_write_items()
            .transact_items(TransactWriteItem::builder().build())
            .client_request_token("stale")
            .return_consumed_capacity(ReturnConsumedCapacity::Indexes)
            .return_item_collection_metrics(ReturnItemCollectionMetrics::Size);
        assert_eq!(
            request.apply(stale).as_input().clone().build().unwrap(),
            *request.input()
        );
        assert_eq!(
            request
                .apply(client.transact_write_items())
                .as_input()
                .clone()
                .build()
                .unwrap(),
            *request.input()
        );
        assert_eq!(
            request.actions().0.input(),
            &request.input().transact_items()[0]
        );
        if !configured {
            assert!(request.input().client_request_token.is_none());
            assert!(request.input().return_consumed_capacity.is_none());
        }
    }
}
