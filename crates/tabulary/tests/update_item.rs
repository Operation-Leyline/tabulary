#![cfg(feature = "derive")]

use std::error::Error;

use aws_sdk_dynamodb::{
    Client, Config,
    error::SdkError,
    operation::update_item::{UpdateItemError, UpdateItemOutput},
    types::{
        AttributeAction, AttributeValue as Av, AttributeValueUpdate, Capacity as SdkCapacity,
        ConditionalOperator, ConsumedCapacity, ExpectedAttributeValue, ItemCollectionMetrics,
        ReturnConsumedCapacity, ReturnItemCollectionMetrics, ReturnValue,
        ReturnValuesOnConditionCheckFailure, error::ConditionalCheckFailedException,
    },
};
use aws_smithy_runtime_api::{client::orchestrator::HttpResponse, http::StatusCode};
use aws_smithy_types::body::SdkBody;
use tabulary::{
    BuildError, Capacity, CollectionMetrics, DecodeFault, FailureKind, Item, OldItem, OldItemMode,
    RecordKey, ReturnItem, Table, Update, WriteError,
};

#[derive(Debug, PartialEq, tabulary::Record)]
#[tabulary(rename_all = "snake_case")]
struct Order {
    #[tabulary(partition_key)]
    id: String,
    #[tabulary(sort_key)]
    line: u64,
    #[tabulary(rename = "order.quantity")]
    quantity: u64,
    note: Option<String>,
}
fn key() -> OrderKey {
    OrderKey::new("private-order".into(), 3).unwrap()
}
fn item(quantity: u64) -> Item {
    Order {
        id: "private-order".into(),
        line: 3,
        quantity,
        note: None,
    }
    .to_item()
    .unwrap()
}
fn prepared(mode: ReturnItem) -> tabulary::PreparedUpdate<Order> {
    Table::<Order>::new("orders")
        .unwrap()
        .update(key())
        .changes(Update::new().and(OrderAttrs::QUANTITY.set(7)))
        .return_item(mode)
        .build()
        .unwrap()
}
fn capacity() -> ConsumedCapacity {
    ConsumedCapacity::builder()
        .table_name("orders")
        .capacity_units(1.5)
        .write_capacity_units(1.5)
        .table(SdkCapacity::builder().write_capacity_units(1.0).build())
        .local_secondary_indexes(
            "by_line",
            SdkCapacity::builder().capacity_units(0.5).build(),
        )
        .build()
}
fn metrics() -> ItemCollectionMetrics {
    ItemCollectionMetrics::builder()
        .item_collection_key("id", Av::S("private-order".into()))
        .size_estimate_range_gb(0.01)
        .size_estimate_range_gb(0.02)
        .build()
}
fn output(raw: Option<Item>) -> UpdateItemOutput {
    UpdateItemOutput::builder()
        .set_attributes(raw)
        .consumed_capacity(capacity())
        .item_collection_metrics(metrics())
        .build()
}

#[test]
fn atomic_changes_and_condition_share_exact_bindings_and_apply_without_io() {
    use aws_smithy_runtime_api::client::http::http_client_fn;
    let config = Config::builder()
        .behavior_version_latest()
        .region(aws_sdk_dynamodb::config::Region::new("us-east-1"))
        .retry_config(aws_sdk_dynamodb::config::retry::RetryConfig::disabled())
        .timeout_config(aws_sdk_dynamodb::config::timeout::TimeoutConfig::disabled())
        .identity_cache(aws_sdk_dynamodb::config::IdentityCache::no_cache())
        .stalled_stream_protection(
            aws_sdk_dynamodb::config::StalledStreamProtectionConfig::disabled(),
        )
        .http_client(http_client_fn(|_, _| {
            panic!("UpdateItem must not request a connector")
        }))
        .build();
    let client = Client::from_conf(config);
    let table = Table::<Order>::new("orders").unwrap();
    let full = table
        .update(key())
        .changes(
            Update::new()
                .and(OrderAttrs::NOTE.remove())
                .and(OrderAttrs::QUANTITY.set(7)),
        )
        .condition(OrderAttrs::QUANTITY.eq(6))
        .return_item(ReturnItem::AllNew)
        .on_condition_failure(OldItemMode::All)
        .capacity(Capacity::Indexes)
        .collection_metrics(CollectionMetrics::Size)
        .build()
        .unwrap();
    assert_eq!(
        full.expressions().update(),
        Some("SET #n0 = :v0 REMOVE #n1")
    );
    assert_eq!(full.expressions().condition(), Some("#n0 = :v1"));
    assert_eq!(
        full.expressions().names(),
        &[
            ("#n0".into(), "order.quantity".into()),
            ("#n1".into(), "note".into())
        ]
        .into()
    );
    assert_eq!(
        full.expressions().values(),
        &[
            (":v0".into(), Av::N("7".into())),
            (":v1".into(), Av::N("6".into()))
        ]
        .into()
    );
    assert_eq!(full.input().key, Some(key().encode_key().unwrap()));
    assert_eq!(
        full.input().update_expression.as_deref(),
        full.expressions().update()
    );
    assert_eq!(
        full.input().condition_expression.as_deref(),
        full.expressions().condition()
    );
    assert_eq!(
        full.input().expression_attribute_names.as_ref(),
        Some(full.expressions().names())
    );
    assert_eq!(
        full.input().expression_attribute_values.as_ref(),
        Some(full.expressions().values())
    );
    assert_eq!(full.input().return_values, Some(ReturnValue::AllNew));
    assert_eq!(
        full.input().return_consumed_capacity,
        Some(ReturnConsumedCapacity::Indexes)
    );
    assert_eq!(
        full.input().return_item_collection_metrics,
        Some(ReturnItemCollectionMetrics::Size)
    );
    assert_eq!(
        full.input().return_values_on_condition_check_failure,
        Some(ReturnValuesOnConditionCheckFailure::AllOld)
    );
    let minimal = table
        .update(key())
        .changes(Update::new().and(OrderAttrs::NOTE.remove()))
        .build()
        .unwrap();
    for request in [&full, &minimal] {
        let stale = client
            .update_item()
            .table_name("stale")
            .key("stale", Av::Bool(false))
            .attribute_updates(
                "stale",
                AttributeValueUpdate::builder()
                    .action(AttributeAction::Put)
                    .value(Av::Bool(false))
                    .build(),
            )
            .expected(
                "stale",
                ExpectedAttributeValue::builder().exists(true).build(),
            )
            .conditional_operator(ConditionalOperator::Or)
            .return_values(ReturnValue::UpdatedOld)
            .return_consumed_capacity(ReturnConsumedCapacity::Total)
            .return_item_collection_metrics(ReturnItemCollectionMetrics::Size)
            .update_expression("stale")
            .condition_expression("stale")
            .expression_attribute_names("#stale", "stale")
            .expression_attribute_values(":stale", Av::Bool(false))
            .return_values_on_condition_check_failure(ReturnValuesOnConditionCheckFailure::AllOld);
        assert_eq!(
            request.apply(stale).as_input().clone().build().unwrap(),
            *request.input()
        );
        assert_eq!(
            request
                .apply(client.update_item())
                .as_input()
                .clone()
                .build()
                .unwrap(),
            *request.input()
        );
    }
    assert_eq!(minimal.input().return_values, Some(ReturnValue::None));
    assert_eq!(
        minimal.input().return_item_collection_metrics,
        Some(ReturnItemCollectionMetrics::None)
    );
    assert!(
        minimal
            .input()
            .return_values_on_condition_check_failure
            .is_none()
    );
    assert!(minimal.input().return_consumed_capacity.is_none());
    assert!(minimal.input().expression_attribute_values.is_none());
    assert!(minimal.input().condition_expression.is_none());
    assert!(
        full.decode(Ok(output(Some(item(7)))))
            .unwrap()
            .item
            .is_some()
    );
    assert!(minimal.decode(Ok(output(None))).unwrap().item.is_none());
    assert!(!format!("{full:?}").contains("private-order"));
    assert!(client.config().http_client().is_some());
}

#[test]
fn full_old_and_new_returns_keep_records_and_complete_accounting() {
    for (choice, sdk, quantity) in [
        (ReturnItem::AllOld, ReturnValue::AllOld, 6),
        (ReturnItem::AllNew, ReturnValue::AllNew, 7),
    ] {
        let request = prepared(choice);
        assert_eq!(request.input().return_values, Some(sdk));
        let decoded = request.decode(Ok(output(Some(item(quantity))))).unwrap();
        assert_eq!(
            decoded.item,
            Some(Order {
                id: "private-order".into(),
                line: 3,
                quantity,
                note: None
            })
        );
        assert_eq!(decoded.capacity, Some(capacity()));
        assert_eq!(decoded.item_collection_metrics, Some(metrics()));
    }
    // An update may create an item, so there need not be an ALL_OLD image.
    for choice in [ReturnItem::None, ReturnItem::AllOld] {
        let decoded = prepared(choice).decode(Ok(output(None))).unwrap();
        assert_eq!(decoded.item, None);
        assert_eq!(decoded.capacity, Some(capacity()));
        assert_eq!(decoded.item_collection_metrics, Some(metrics()));
    }
    let decoded = prepared(ReturnItem::None)
        .decode(Ok(UpdateItemOutput::builder().build()))
        .unwrap();
    assert!(decoded.capacity.is_none());
    assert!(decoded.item_collection_metrics.is_none());
}

#[test]
fn malformed_full_returns_are_committed_write_errors_with_unchanged_metadata() {
    let mut wrong = item(7);
    wrong.insert("order.quantity".into(), Av::S("private-value".into()));
    for (raw, attribute, kind) in [
        (Item::new(), "id", DecodeFault::Missing),
        (
            [("order.quantity".into(), Av::N("7".into()))].into(),
            "id",
            DecodeFault::Missing,
        ),
        (
            wrong,
            "order.quantity",
            DecodeFault::WrongType {
                expected: tabulary::DynamoType::N,
                actual: tabulary::DynamoType::S,
            },
        ),
    ] {
        for choice in [ReturnItem::AllOld, ReturnItem::AllNew] {
            let error = prepared(choice)
                .decode(Ok(output(Some(raw.clone()))))
                .unwrap_err();
            assert!(!format!("{error:?} {error}").contains("private-value"));
            assert!(!format!("{error:?} {error}").contains("private-order"));
            assert!(error.source().is_some());
            assert!(error.sdk_error().is_none());
            let WriteError::Decode(fault) = error else {
                panic!("expected committed write decode error")
            };
            assert_eq!(fault.source.record, "Order");
            assert_eq!(fault.source.attribute(), Some(attribute));
            assert_eq!(fault.source.kind, kind);
            assert_eq!(fault.raw.expose(), &raw);
            assert_eq!(fault.capacity, Some(capacity()));
            assert_eq!(fault.item_collection_metrics, Some(metrics()));
        }
    }
}

#[test]
fn response_presence_must_agree_with_the_requested_return_mode() {
    for (choice, raw, code) in [
        (
            ReturnItem::None,
            Some(item(7)),
            "unexpected_returned_attributes",
        ),
        (
            ReturnItem::None,
            Some(Item::new()),
            "unexpected_returned_attributes",
        ),
        (ReturnItem::AllNew, None, "missing_returned_attributes"),
    ] {
        let error = prepared(choice)
            .decode(Ok(output(raw.clone())))
            .unwrap_err();
        let WriteError::Decode(fault) = error else {
            panic!("expected response shape fault")
        };
        assert_eq!(fault.source.kind, DecodeFault::Validation { code });
        assert_eq!(fault.raw.expose(), &raw.unwrap_or_default());
        assert_eq!(fault.capacity, Some(capacity()));
        assert_eq!(fault.item_collection_metrics, Some(metrics()));
    }
}

#[test]
fn failed_conditions_delegate_old_images_to_the_existing_classifier() {
    for mode in [OldItemMode::None, OldItemMode::All] {
        for raw in [None, Some(item(6)), Some(Item::new())] {
            let request = Table::<Order>::new("orders")
                .unwrap()
                .update(key())
                .changes(Update::new().and(OrderAttrs::QUANTITY.set(7)))
                .condition(OrderAttrs::QUANTITY.eq(6))
                .on_condition_failure(mode)
                .build()
                .unwrap();
            let sdk = SdkError::service_error(
                UpdateItemError::ConditionalCheckFailedException(
                    ConditionalCheckFailedException::builder()
                        .message("private-sdk")
                        .set_item(raw.clone())
                        .build(),
                ),
                HttpResponse::new(
                    StatusCode::try_from(400).unwrap(),
                    SdkBody::from("private-response"),
                ),
            );
            let error = request.decode(Err(sdk)).unwrap_err();
            assert!(
                error
                    .sdk_error()
                    .unwrap()
                    .as_service_error()
                    .unwrap()
                    .is_conditional_check_failed_exception()
            );
            assert!(!format!("{error:?} {error}").contains("private-"));
            let WriteError::Refused { failure, .. } = error else {
                panic!("expected refusal")
            };
            assert_eq!(failure.kind, FailureKind::ConditionFailed);
            match (mode, raw, failure.old) {
                (OldItemMode::None, _, OldItem::NotRequested)
                | (OldItemMode::All, None, OldItem::Unavailable) => {}
                (
                    OldItemMode::All,
                    Some(raw),
                    OldItem::Invalid {
                        error,
                        raw: retained,
                    },
                ) => {
                    assert!(raw.is_empty());
                    assert_eq!(error.kind, DecodeFault::Missing);
                    assert_eq!(retained.expose(), &raw);
                }
                (OldItemMode::All, Some(_), OldItem::Decoded(record)) => {
                    assert_eq!(record.quantity, 6)
                }
                _ => panic!("unexpected old item state"),
            }
        }
    }
    let error = prepared(ReturnItem::None)
        .decode(Err(SdkError::construction_failure(std::io::Error::other(
            "private-sdk",
        ))))
        .unwrap_err();
    assert!(matches!(
        error.sdk_error(),
        Some(SdkError::ConstructionFailure(_))
    ));
    assert!(matches!(&error, WriteError::Sdk(_)));
    assert!(!format!("{error:?} {error}").contains("private-sdk"));
}

#[test]
fn build_refuses_missing_overlapping_or_invalid_changes_and_keys() {
    let table = Table::<Order>::new("orders").unwrap();
    assert!(matches!(
        table.update(key()).build(),
        Err(BuildError::EmptyOperation)
    ));
    assert!(matches!(
        table
            .update(key())
            .changes(
                Update::new()
                    .and(OrderAttrs::QUANTITY.set(1))
                    .and(OrderAttrs::QUANTITY.set(2))
            )
            .build(),
        Err(BuildError::OverlappingUpdate { .. })
    ));
    let invalid_key = OrderKey {
        id: String::new(),
        line: 3,
    };
    assert!(
        matches!(table.update(invalid_key).changes(Update::new().and(OrderAttrs::QUANTITY.set(7))).build(), Err(BuildError::Encode(source)) if source.kind == tabulary::EncodeFault::EmptyKey && source.attribute() == Some("id"))
    );
    let forged: tabulary::Attr<Order, u64> = tabulary::__private::field("wrong", 2);
    assert!(
        table
            .update(key())
            .changes(Update::new().and(forged.set(7)))
            .build()
            .is_err()
    );
    assert!(
        table
            .update(key())
            .changes(Update::new().and(OrderAttrs::QUANTITY.set(7)))
            .condition(forged.eq(6))
            .build()
            .is_err()
    );
    let guarded = table
        .update(key())
        .changes(Update::new().and(OrderAttrs::QUANTITY.set(7)))
        .condition(OrderAttrs::QUANTITY.eq(6))
        .condition(OrderAttrs::ID.exists())
        .build()
        .unwrap();
    assert_eq!(
        guarded.expressions().condition(),
        Some("(#n0 = :v1 AND attribute_exists(#n1))")
    );
}

#[test]
fn partial_return_modes_and_foreign_expression_owners_are_compile_errors() {
    trybuild::TestCases::new().compile_fail("tests/ui/update_item/*.rs");
}
