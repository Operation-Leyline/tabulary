#![cfg(feature = "derive")]

use aws_sdk_dynamodb::{
    Client, Config,
    error::SdkError,
    operation::put_item::{PutItemError, PutItemOutput},
    types::{
        AttributeValue as Av, Capacity as SdkCapacity, ConditionalOperator, ConsumedCapacity,
        ExpectedAttributeValue, ItemCollectionMetrics, ReturnConsumedCapacity,
        ReturnItemCollectionMetrics, ReturnValue, ReturnValuesOnConditionCheckFailure,
        error::ConditionalCheckFailedException,
    },
};
use tabulary::{
    BuildError, Capacity, CollectionMetrics, DecodeFault, EncodeFault, FailureKind, Item, OldItem,
    OldItemMode, ReturnOld, Table, WriteError,
};

#[derive(Debug, PartialEq, tabulary::Record)]
#[tabulary(rename_all = "snake_case",
    index(ident = ByRoute, name = "by_route", kind = global, sparse,
        partition(name = "route_key", template = "route!{route:hex}"), project = all))]
struct Order {
    #[tabulary(partition_key)]
    id: String,
    #[tabulary(rename = "order.quantity")]
    quantity: u64,
    route: Option<String>,
    note: Option<String>,
}
fn order() -> Order {
    Order {
        id: "private-order".into(),
        quantity: 7,
        route: Some("west".into()),
        note: None,
    }
}
fn capacity() -> ConsumedCapacity {
    ConsumedCapacity::builder()
        .table_name("orders")
        .capacity_units(2.5)
        .table(SdkCapacity::builder().write_capacity_units(2.0).build())
        .global_secondary_indexes(
            "by_route",
            SdkCapacity::builder().capacity_units(0.5).build(),
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
fn request(mode: OldItemMode) -> tabulary::PreparedPut<Order> {
    Table::<Order>::new("orders")
        .unwrap()
        .put(&order())
        .on_condition_failure(mode)
        .build()
        .unwrap()
}
fn refusal(item: Option<Item>) -> SdkError<PutItemError> {
    use aws_smithy_runtime_api::{client::orchestrator::HttpResponse, http::StatusCode};
    use aws_smithy_types::body::SdkBody;
    SdkError::service_error(
        PutItemError::ConditionalCheckFailedException(
            ConditionalCheckFailedException::builder()
                .message("private-message")
                .set_item(item)
                .build(),
        ),
        HttpResponse::new(
            StatusCode::try_from(400).unwrap(),
            SdkBody::from("private-response"),
        ),
    )
}

#[test]
fn conditional_put_owns_encoded_item_and_exact_inspectable_maps() {
    let request = {
        let order = order();
        Table::<Order>::new("orders")
            .unwrap()
            .put(&order)
            .condition(OrderAttrs::ID.not_exists())
            .condition(OrderAttrs::QUANTITY.eq(7))
            .on_condition_failure(OldItemMode::All)
            .return_item(ReturnOld::AllOld)
            .capacity(Capacity::Indexes)
            .collection_metrics(CollectionMetrics::Size)
            .build()
            .unwrap()
    };
    let input = request.input();
    assert_eq!(input.table_name.as_deref(), Some("orders"));
    assert_eq!(input.item, Some(order().to_item().unwrap()));
    assert_eq!(
        input.item.as_ref().unwrap()["route_key"],
        Av::S("route!77657374".into())
    );
    assert_eq!(
        request.expressions().condition(),
        Some("(attribute_not_exists(#n0) AND #n1 = :v0)")
    );
    assert_eq!(
        request.expressions().names(),
        &[
            ("#n0".into(), "id".into()),
            ("#n1".into(), "order.quantity".into())
        ]
        .into()
    );
    assert_eq!(
        request.expressions().values(),
        &[(":v0".into(), Av::N("7".into()))].into()
    );
    assert_eq!(
        input.condition_expression.as_deref(),
        request.expressions().condition()
    );
    assert_eq!(
        input.expression_attribute_names.as_ref().unwrap(),
        request.expressions().names()
    );
    assert_eq!(
        input.expression_attribute_values.as_ref().unwrap(),
        request.expressions().values()
    );
    assert_eq!(
        input.return_values_on_condition_check_failure,
        Some(ReturnValuesOnConditionCheckFailure::AllOld)
    );
    assert_eq!(input.return_values, Some(ReturnValue::AllOld));
    assert_eq!(
        input.return_consumed_capacity,
        Some(ReturnConsumedCapacity::Indexes)
    );
    assert_eq!(
        input.return_item_collection_metrics,
        Some(ReturnItemCollectionMetrics::Size)
    );
    assert!(!format!("{request:?}").contains("private-order"));
}

#[test]
fn apply_copies_every_field_clears_legacy_state_and_never_uses_transport() {
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
            panic!("PutItem must not request a connector")
        }))
        .build();
    let client = Client::from_conf(config);
    let table = Table::<Order>::new("orders").unwrap();
    for conditional in [false, true] {
        let record = order();
        let mut builder = table.put(&record);
        // The prepared request owns the encoded item.
        if conditional {
            builder = builder
                .condition(OrderAttrs::QUANTITY.eq(7))
                .on_condition_failure(OldItemMode::All)
                .return_item(ReturnOld::AllOld)
                .capacity(Capacity::Indexes)
                .collection_metrics(CollectionMetrics::Size);
        }
        let request = builder.build().unwrap();
        let stale = client
            .put_item()
            .table_name("stale")
            .item("stale", Av::Bool(false))
            .expected(
                "stale",
                ExpectedAttributeValue::builder().exists(false).build(),
            )
            .conditional_operator(ConditionalOperator::Or)
            .condition_expression("stale")
            .expression_attribute_names("#stale", "stale")
            .expression_attribute_values(":stale", Av::Bool(false))
            .return_values(ReturnValue::AllOld)
            .return_consumed_capacity(ReturnConsumedCapacity::Total)
            .return_item_collection_metrics(ReturnItemCollectionMetrics::Size)
            .return_values_on_condition_check_failure(ReturnValuesOnConditionCheckFailure::AllOld);
        assert_eq!(
            request.apply(stale).as_input().clone().build().unwrap(),
            *request.input()
        );
        assert_eq!(
            request
                .apply(client.put_item())
                .as_input()
                .clone()
                .build()
                .unwrap(),
            *request.input()
        );
        assert!(
            request
                .decode(Ok(PutItemOutput::builder().build()))
                .unwrap()
                .item
                .is_none()
        );
        assert!(matches!(
            request.decode(Err(refusal(None))),
            Err(WriteError::Refused { .. })
        ));
        if !conditional {
            assert_eq!(request.input().return_values, Some(ReturnValue::None));
            assert_eq!(
                request.input().return_item_collection_metrics,
                Some(ReturnItemCollectionMetrics::None)
            );
            assert_eq!(request.input().return_consumed_capacity, None);
            assert_eq!(
                request.input().return_values_on_condition_check_failure,
                None
            );
        }
        assert!(request.input().expected.is_none());
        assert!(request.input().conditional_operator.is_none());
        if !conditional {
            assert!(request.input().condition_expression.is_none());
            assert!(request.input().expression_attribute_names.is_none());
            assert!(request.input().expression_attribute_values.is_none());
        }
    }
    for (choice, expected) in [
        (Capacity::None, None),
        (Capacity::Total, Some(ReturnConsumedCapacity::Total)),
        (Capacity::Indexes, Some(ReturnConsumedCapacity::Indexes)),
    ] {
        assert_eq!(
            table
                .put(&order())
                .capacity(choice)
                .build()
                .unwrap()
                .input()
                .return_consumed_capacity,
            expected
        );
    }
}

#[test]
fn success_decodes_full_old_record_and_preserves_all_accounting() {
    let request = Table::<Order>::new("orders")
        .unwrap()
        .put(&order())
        .return_item(ReturnOld::AllOld)
        .build()
        .unwrap();
    for raw in [None, Some(order().to_item().unwrap())] {
        let result = request
            .decode(Ok(PutItemOutput::builder()
                .set_attributes(raw.clone())
                .consumed_capacity(capacity())
                .item_collection_metrics(metrics())
                .build()))
            .unwrap();
        assert_eq!(result.item, raw.map(|_| order()));
        assert_eq!(result.capacity, Some(capacity()));
        assert_eq!(result.item_collection_metrics, Some(metrics()));
    }
    let result = request
        .decode(Ok(PutItemOutput::builder().build()))
        .unwrap();
    assert!(result.capacity.is_none());
    assert!(result.item_collection_metrics.is_none());
}

#[test]
fn committed_corrupt_returns_are_decode_errors_with_raw_data_and_accounting() {
    let request = request(OldItemMode::None);
    let mut wrong = order().to_item().unwrap();
    wrong.insert("order.quantity".into(), Av::S("private-number".into()));
    let mut missing = order().to_item().unwrap();
    missing.remove("order.quantity");
    for (raw, attribute, kind) in [
        (Item::new(), "id", DecodeFault::Missing),
        (missing, "order.quantity", DecodeFault::Missing),
        (
            wrong,
            "order.quantity",
            DecodeFault::WrongType {
                expected: tabulary::DynamoType::N,
                actual: tabulary::DynamoType::S,
            },
        ),
    ] {
        let error = request
            .decode(Ok(PutItemOutput::builder()
                .set_attributes(Some(raw.clone()))
                .consumed_capacity(capacity())
                .item_collection_metrics(metrics())
                .build()))
            .unwrap_err();
        assert!(!format!("{error:?} {error}").contains("private-"));
        let WriteError::Decode(fault) = error else {
            panic!("committed decode failure required")
        };
        assert_eq!(fault.source.record, "Order");
        assert_eq!(fault.source.attribute(), Some(attribute));
        assert_eq!(fault.source.kind, kind);
        assert_eq!(fault.raw.expose(), &raw);
        assert_eq!(fault.capacity, Some(capacity()));
        assert_eq!(fault.item_collection_metrics, Some(metrics()));
    }
}

#[test]
fn conditional_refusals_use_requested_old_item_mode_and_retain_sdk_source() {
    for mode in [OldItemMode::None, OldItemMode::All] {
        for raw in [None, Some(Item::new()), Some(order().to_item().unwrap())] {
            let error = request(mode).decode(Err(refusal(raw.clone()))).unwrap_err();
            assert!(!format!("{error:?} {error}").contains("private-"));
            let WriteError::Refused { failure, source } = error else {
                panic!("refusal required")
            };
            assert_eq!(failure.kind, FailureKind::ConditionFailed);
            assert!(
                source
                    .as_service_error()
                    .unwrap()
                    .is_conditional_check_failed_exception()
            );
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
                    assert_eq!(record, order())
                }
                _ => panic!("wrong old-item outcome"),
            }
        }
    }
    let error = request(OldItemMode::All)
        .decode(Err(SdkError::construction_failure(std::io::Error::other(
            "private-sdk",
        ))))
        .unwrap_err();
    assert!(matches!(
        error.sdk_error(),
        Some(SdkError::ConstructionFailure(_))
    ));
    assert!(matches!(error, WriteError::Sdk(_)));
}

#[test]
fn put_reuses_strict_record_encoding_and_conservative_item_limit() {
    let table = Table::<Order>::new("orders").unwrap();
    let mut record = order();
    record.id.clear();
    assert!(
        matches!(table.put(&record).build(), Err(BuildError::Encode(e)) if e.attribute() == Some("id") && e.kind == EncodeFault::EmptyKey)
    );
    record = order();
    record.route = Some(String::new());
    // The template prefix still makes this a valid physical index key.
    assert!(table.put(&record).build().is_ok());
    record.route = Some("x".repeat(1025));
    assert!(
        matches!(table.put(&record).build(), Err(BuildError::Encode(e))
        if e.record == "Order" && e.attribute() == Some("route_key")
        && matches!(e.kind, EncodeFault::KeySize { max: 2048, .. }))
    );
    record.route = None;
    assert!(
        !table
            .put(&record)
            .build()
            .unwrap()
            .input()
            .item
            .as_ref()
            .unwrap()
            .contains_key("route_key")
    );
    let base = tabulary::BilledSize::of_item(&record.to_item().unwrap())
        .unwrap()
        .conservative_item_bytes();
    record.note = Some("x".repeat(400 * 1024 - base - "note".len()));
    assert!(table.put(&record).build().is_ok());
    record.note.as_mut().unwrap().push('x');
    assert!(matches!(
        table.put(&record).build(),
        Err(BuildError::RequestLimit {
            max: 409600,
            actual: 409601
        })
    ));
    assert!(matches!(
        table
            .put(&order())
            .condition(OrderAttrs::QUANTITY.in_values(vec![]))
            .condition(OrderAttrs::ID.not_exists())
            .build(),
        Err(BuildError::EmptyOperation)
    ));
}

#[path = "../../../docs/examples/01_simple.rs"]
mod simple;
#[test]
fn promoted_simple_example_compiles_and_round_trips() {
    simple::round_trip().unwrap();
    let _ = simple::create_and_read;
}
#[test]
fn put_rejects_foreign_records_conditions_and_unsupported_returns() {
    trybuild::TestCases::new().compile_fail("tests/ui/put/*.rs");
}
