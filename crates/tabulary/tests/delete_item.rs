#![cfg(feature = "derive")]

use aws_sdk_dynamodb::{
    Client, Config,
    error::SdkError,
    operation::delete_item::{DeleteItemError, DeleteItemOutput},
    types::{
        AttributeValue as Av, Capacity as SdkCapacity, ConditionalOperator, ConsumedCapacity,
        ExpectedAttributeValue, ItemCollectionMetrics, ReturnConsumedCapacity,
        ReturnItemCollectionMetrics, ReturnValue, ReturnValuesOnConditionCheckFailure,
        error::ConditionalCheckFailedException,
    },
};
use aws_smithy_runtime_api::{client::orchestrator::HttpResponse, http::StatusCode};
use aws_smithy_types::body::SdkBody;
use tabulary::{
    Capacity, CollectionMetrics, DecodeFault, FailureKind, Item, OldItem, OldItemMode, RecordKey,
    ReturnOld, Table, WriteError,
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
}

fn key() -> OrderKey {
    OrderKey::new("private-order".into(), 3).unwrap()
}
fn item() -> Item {
    Order {
        id: "private-order".into(),
        line: 3,
        quantity: 7,
    }
    .to_item()
    .unwrap()
}
fn capacity() -> ConsumedCapacity {
    ConsumedCapacity::builder()
        .table_name("orders")
        .capacity_units(1.5)
        .table(SdkCapacity::builder().write_capacity_units(1.0).build())
        .global_secondary_indexes(
            "by_status",
            SdkCapacity::builder().capacity_units(0.5).build(),
        )
        .local_secondary_indexes(
            "by_line",
            SdkCapacity::builder().capacity_units(0.25).build(),
        )
        .build()
}
fn metrics() -> ItemCollectionMetrics {
    ItemCollectionMetrics::builder()
        .item_collection_key("id", Av::S("private-order".into()))
        .size_estimate_range_gb(0.1)
        .size_estimate_range_gb(0.2)
        .build()
}
fn conditional(raw: Option<Item>) -> SdkError<DeleteItemError> {
    SdkError::service_error(
        DeleteItemError::ConditionalCheckFailedException(
            ConditionalCheckFailedException::builder()
                .message("private-sdk")
                .set_item(raw)
                .build(),
        ),
        HttpResponse::new(
            StatusCode::try_from(400).unwrap(),
            SdkBody::from("private-response"),
        ),
    )
}

#[test]
fn key_only_input_and_inline_condition_have_exact_inspectable_maps() {
    let table = Table::<Order>::new("orders").unwrap();
    let plain = table.delete(key()).build().unwrap();
    assert_eq!(plain.input().table_name(), Some("orders"));
    assert_eq!(
        plain.input().key.as_ref().unwrap(),
        &key().encode_key().unwrap()
    );
    assert_eq!(plain.input().key.as_ref().unwrap().len(), 2);
    assert!(plain.input().condition_expression.is_none());
    assert!(plain.input().expression_attribute_names.is_none());
    assert!(plain.input().expression_attribute_values.is_none());
    assert!(
        plain
            .input()
            .return_values_on_condition_check_failure
            .is_none()
    );
    assert!(plain.input().return_consumed_capacity.is_none());
    assert_eq!(plain.input().return_values, Some(ReturnValue::None));
    assert_eq!(
        plain.input().return_item_collection_metrics,
        Some(ReturnItemCollectionMetrics::None)
    );
    let request = table
        .delete(key())
        .condition(OrderAttrs::QUANTITY.eq(7).and(OrderAttrs::ID.exists()))
        .return_item(ReturnOld::AllOld)
        .on_condition_failure(OldItemMode::All)
        .capacity(Capacity::Indexes)
        .collection_metrics(CollectionMetrics::Size)
        .build()
        .unwrap();
    assert_eq!(
        request.expressions().condition(),
        Some("(#n0 = :v0 AND attribute_exists(#n1))")
    );
    assert_eq!(
        request.expressions().names(),
        &[
            ("#n0".into(), "order.quantity".into()),
            ("#n1".into(), "id".into())
        ]
        .into()
    );
    assert_eq!(
        request.expressions().values(),
        &[(":v0".into(), Av::N("7".into()))].into()
    );
    assert_eq!(
        request.input().condition_expression.as_deref(),
        request.expressions().condition()
    );
    assert_eq!(
        request.input().expression_attribute_names.as_ref(),
        Some(request.expressions().names())
    );
    assert_eq!(
        request.input().expression_attribute_values.as_ref(),
        Some(request.expressions().values())
    );
    assert_eq!(request.input().return_values, Some(ReturnValue::AllOld));
    assert_eq!(
        request.input().return_values_on_condition_check_failure,
        Some(ReturnValuesOnConditionCheckFailure::AllOld)
    );
    assert_eq!(
        request.input().return_consumed_capacity,
        Some(ReturnConsumedCapacity::Indexes)
    );
    assert_eq!(
        request.input().return_item_collection_metrics,
        Some(ReturnItemCollectionMetrics::Size)
    );
    assert!(!format!("{request:?}").contains("private-order"));
    let combined = table
        .delete(key())
        .condition(OrderAttrs::QUANTITY.eq(7))
        .condition(OrderAttrs::ID.exists())
        .build()
        .unwrap();
    assert_eq!(
        combined.expressions().condition(),
        request.expressions().condition()
    );
    assert_eq!(
        combined.expressions().names(),
        request.expressions().names()
    );
    assert_eq!(
        combined.expressions().values(),
        request.expressions().values()
    );
}

#[test]
fn apply_copies_every_field_and_clears_stale_fields_without_io() {
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
            panic!("delete must not use transport")
        }))
        .build();
    let client = Client::from_conf(config);
    let table = Table::<Order>::new("orders").unwrap();
    for configured in [false, true] {
        let mut builder = table.delete(key());
        if configured {
            builder = builder
                .condition(OrderAttrs::QUANTITY.eq(7))
                .return_item(ReturnOld::AllOld)
                .on_condition_failure(OldItemMode::All)
                .capacity(Capacity::Total)
                .collection_metrics(CollectionMetrics::Size);
        }
        let request = builder.build().unwrap();
        let stale = client
            .delete_item()
            .table_name("stale")
            .key("stale", Av::Bool(false))
            .expected(
                "stale",
                ExpectedAttributeValue::builder().exists(true).build(),
            )
            .conditional_operator(ConditionalOperator::Or)
            .return_values(ReturnValue::AllOld)
            .return_consumed_capacity(ReturnConsumedCapacity::Indexes)
            .return_item_collection_metrics(ReturnItemCollectionMetrics::Size)
            .condition_expression("#stale = :stale")
            .expression_attribute_names("#stale", "stale")
            .expression_attribute_values(":stale", Av::S("secret".into()))
            .return_values_on_condition_check_failure(ReturnValuesOnConditionCheckFailure::AllOld);
        assert_eq!(
            request.apply(stale).as_input().clone().build().unwrap(),
            *request.input()
        );
        assert_eq!(
            request
                .apply(client.delete_item())
                .as_input()
                .clone()
                .build()
                .unwrap(),
            *request.input()
        );
        assert!(request.input().expected.is_none());
        assert!(request.input().conditional_operator.is_none());
        assert!(
            request
                .decode(Ok(DeleteItemOutput::builder().build()))
                .unwrap()
                .item
                .is_none()
        );
    }
    assert!(client.config().http_client().is_some());
}

#[test]
fn all_old_success_decodes_full_record_and_retains_accounting() {
    let request = Table::<Order>::new("orders")
        .unwrap()
        .delete(key())
        .return_item(ReturnOld::AllOld)
        .build()
        .unwrap();
    for raw in [None, Some(item())] {
        let result = request
            .decode(Ok(DeleteItemOutput::builder()
                .set_attributes(raw.clone())
                .consumed_capacity(capacity())
                .item_collection_metrics(metrics())
                .build()))
            .unwrap();
        assert_eq!(result.item.map(|record| record.to_item().unwrap()), raw);
        assert_eq!(result.capacity, Some(capacity()));
        assert_eq!(result.item_collection_metrics, Some(metrics()));
    }
    for (raw, attribute, fault) in [
        (Item::new(), "id", DecodeFault::Missing),
        (
            [
                ("id".into(), Av::S("private-order".into())),
                ("line".into(), Av::N("3".into())),
                ("order.quantity".into(), Av::N("private-number".into())),
            ]
            .into(),
            "order.quantity",
            DecodeFault::InvalidNumber,
        ),
    ] {
        let error = request
            .decode(Ok(DeleteItemOutput::builder()
                .set_attributes(Some(raw.clone()))
                .consumed_capacity(capacity())
                .item_collection_metrics(metrics())
                .build()))
            .unwrap_err();
        assert!(!format!("{error:?} {error}").contains("private-"));
        let WriteError::Decode(error) = error else {
            panic!("committed decode error required")
        };
        assert_eq!(error.source.record, "Order");
        assert_eq!(error.source.attribute(), Some(attribute));
        assert_eq!(error.source.kind, fault);
        assert_eq!(error.raw.expose(), &raw);
        assert_eq!(error.capacity, Some(capacity()));
        assert_eq!(error.item_collection_metrics, Some(metrics()));
    }
}

#[test]
fn all_old_failures_preserve_refusal_and_distinguish_missing_invalid_and_unrequested() {
    for mode in [OldItemMode::None, OldItemMode::All] {
        let request = Table::<Order>::new("orders")
            .unwrap()
            .delete(key())
            .on_condition_failure(mode)
            .build()
            .unwrap();
        for raw in [None, Some(item()), Some(Item::new())] {
            let error = request.decode(Err(conditional(raw.clone()))).unwrap_err();
            assert!(!format!("{error:?} {error}").contains("private-"));
            assert!(
                error
                    .sdk_error()
                    .unwrap()
                    .as_service_error()
                    .unwrap()
                    .is_conditional_check_failed_exception()
            );
            let WriteError::Refused { failure, source } = error else {
                panic!("refusal required")
            };
            assert_eq!(
                source.raw_response().unwrap().body().bytes(),
                Some(b"private-response".as_slice())
            );
            assert_eq!(failure.kind, FailureKind::ConditionFailed);
            match (mode, raw, failure.old) {
                (OldItemMode::None, _, OldItem::NotRequested) => {}
                (OldItemMode::All, None, OldItem::Unavailable) => {}
                (OldItemMode::All, Some(raw), OldItem::Decoded(record)) => {
                    assert_eq!(record.to_item().unwrap(), raw)
                }
                (
                    OldItemMode::All,
                    Some(raw),
                    OldItem::Invalid {
                        error,
                        raw: retained,
                    },
                ) => {
                    assert_eq!(error.attribute(), Some("id"));
                    assert_eq!(error.kind, DecodeFault::Missing);
                    assert_eq!(retained.expose(), &raw);
                }
                _ => panic!("old item state must match request and payload"),
            }
        }
        let error = request
            .decode(Err(SdkError::construction_failure(std::io::Error::other(
                "private-sdk",
            ))))
            .unwrap_err();
        assert!(matches!(&error, WriteError::Sdk(_)));
        assert!(matches!(
            error.sdk_error(),
            Some(SdkError::ConstructionFailure(_))
        ));
        assert!(!format!("{error:?} {error}").contains("private-sdk"));
    }
}

#[test]
fn invalid_keys_and_conditions_fail_at_build() {
    let table = Table::<Order>::new("orders").unwrap();
    let invalid = OrderKey {
        id: String::new(),
        line: 3,
    };
    assert!(
        matches!(table.delete(invalid).build(), Err(tabulary::BuildError::Encode(error)) if error.kind == tabulary::EncodeFault::EmptyKey && error.attribute() == Some("id"))
    );
    assert!(matches!(
        table
            .delete(key())
            .condition(OrderAttrs::QUANTITY.in_values(Vec::<u64>::new()))
            .build(),
        Err(tabulary::BuildError::EmptyOperation)
    ));
}

#[test]
fn foreign_keys_conditions_and_unsupported_returns_are_compile_errors() {
    trybuild::TestCases::new().compile_fail("tests/ui/delete/*.rs");
}
