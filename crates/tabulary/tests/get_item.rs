#![cfg(feature = "derive")]

use std::error::Error;

use aws_sdk_dynamodb::{
    Client, Config,
    error::SdkError,
    operation::get_item::GetItemOutput,
    types::{
        AttributeValue as Av, Capacity as SdkCapacity, ConsumedCapacity, ReturnConsumedCapacity,
    },
};
use tabulary::{Capacity, DecodeFault, Item, ReadConsistency, ReadError, Table};

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

#[derive(Debug, PartialEq, tabulary::Projection)]
#[tabulary(record = Order)]
struct Quantity {
    #[tabulary(attribute = OrderAttrs::QUANTITY)]
    quantity: u64,
}

fn key() -> OrderKey {
    OrderKey::new("order-lime".into(), 3).unwrap()
}
fn prepared() -> tabulary::PreparedGet<Order> {
    Table::<Order>::new("orders")
        .unwrap()
        .get(key(), ReadConsistency::Strong)
        .build()
        .unwrap()
}
fn item() -> Item {
    Order {
        id: "order-lime".into(),
        line: 3,
        quantity: 7,
        note: None,
    }
    .to_item()
    .unwrap()
}
fn capacity() -> ConsumedCapacity {
    ConsumedCapacity::builder()
        .table_name("orders")
        .capacity_units(1.5)
        .read_capacity_units(1.5)
        .table(SdkCapacity::builder().read_capacity_units(1.0).build())
        .global_secondary_indexes(
            "by_status",
            SdkCapacity::builder().capacity_units(0.5).build(),
        )
        .local_secondary_indexes(
            "by_line",
            SdkCapacity::builder().read_capacity_units(0.25).build(),
        )
        .build()
}

#[test]
fn apply_replaces_every_sdk_input_field_without_using_transport() {
    use aws_smithy_runtime_api::client::http::http_client_fn;
    let transport = http_client_fn(|_, _| panic!("GetItem adapter must not request a connector"));
    let config = Config::builder()
        .behavior_version_latest()
        .region(aws_sdk_dynamodb::config::Region::new("us-east-1"))
        .retry_config(aws_sdk_dynamodb::config::retry::RetryConfig::disabled())
        .timeout_config(aws_sdk_dynamodb::config::timeout::TimeoutConfig::disabled())
        .identity_cache(aws_sdk_dynamodb::config::IdentityCache::no_cache())
        .stalled_stream_protection(
            aws_sdk_dynamodb::config::StalledStreamProtectionConfig::disabled(),
        )
        .http_client(transport)
        .build();
    let client = Client::from_conf(config);
    let table = Table::<Order>::new("orders").unwrap();
    let full = table.get(key(), ReadConsistency::Eventual).build().unwrap();
    let selected = table
        .get(key(), ReadConsistency::Strong)
        .select::<Quantity>()
        .capacity(Capacity::Indexes)
        .build()
        .unwrap();
    let stale = client
        .get_item()
        .table_name("stale")
        .key("stale", Av::S("secret".into()))
        .attributes_to_get("stale")
        .consistent_read(true)
        .return_consumed_capacity(ReturnConsumedCapacity::Total)
        .projection_expression("#stale")
        .expression_attribute_names("#stale", "stale");
    assert_eq!(
        full.apply(stale).as_input().clone().build().unwrap(),
        *full.input()
    );
    let fresh = full.apply(client.get_item());
    assert_eq!(fresh.as_input().clone().build().unwrap(), *full.input());
    let applied = selected.apply(
        client
            .get_item()
            .attributes_to_get("stale")
            .key("stale", Av::Bool(false))
            .expression_attribute_names("#stale", "stale"),
    );
    assert_eq!(
        applied.as_input().clone().build().unwrap(),
        *selected.input()
    );
    assert_eq!(full.input().consistent_read, Some(false));
    assert_eq!(full.input().return_consumed_capacity, None);
    assert_eq!(full.input().key.as_ref().unwrap().len(), 2);
    assert_eq!(
        full.input().key.as_ref().unwrap()["line"],
        Av::N("3".into())
    );
    assert_eq!(selected.input().consistent_read, Some(true));
    assert_eq!(
        selected.input().return_consumed_capacity,
        Some(ReturnConsumedCapacity::Indexes)
    );
    assert_eq!(selected.expressions().projection(), Some("#n0"));
    assert_eq!(selected.expressions().names()["#n0"], "order.quantity");
    assert_eq!(
        selected
            .input()
            .expression_attribute_names
            .as_ref()
            .unwrap(),
        selected.expressions().names()
    );
    assert_eq!(
        selected.input().projection_expression.as_deref(),
        selected.expressions().projection()
    );
    assert!(selected.expressions().values().is_empty());
    assert!(full.expressions().names().is_empty());
    assert!(full.input().attributes_to_get.is_none());
    assert!(full.input().expression_attribute_names.is_none());
    assert!(full.input().projection_expression.is_none());
    assert!(client.config().http_client().is_some());
    for (choice, expected) in [
        (Capacity::None, None),
        (Capacity::Total, Some(ReturnConsumedCapacity::Total)),
    ] {
        let request = table
            .get(key(), ReadConsistency::Strong)
            .capacity(choice)
            .build()
            .unwrap();
        assert_eq!(request.input().return_consumed_capacity, expected);
    }
    // Neither decoding a synthetic response nor preparing/applying requests uses transport.
    assert!(
        full.decode(Ok(GetItemOutput::builder().build()))
            .unwrap()
            .item
            .is_none()
    );
}

#[test]
fn only_an_absent_item_is_missing_and_capacity_is_unchanged() {
    let request = prepared();
    let output = request
        .decode(Ok(GetItemOutput::builder()
            .consumed_capacity(capacity())
            .build()))
        .unwrap();
    assert_eq!(output.item, None);
    assert_eq!(output.capacity, Some(capacity()));
    let output = request
        .decode(Ok(GetItemOutput::builder().set_item(Some(item())).build()))
        .unwrap();
    assert_eq!(output.item.unwrap().quantity, 7);
    assert_eq!(output.capacity, None);
    let raw = Item::new();
    let error = request
        .decode(Ok(GetItemOutput::builder()
            .set_item(Some(raw.clone()))
            .consumed_capacity(capacity())
            .build()))
        .unwrap_err();
    let ReadError::Decode(fault) = error else {
        panic!("expected strict decode fault")
    };
    assert_eq!(fault.position, 0);
    assert_eq!(fault.source.record, "Order");
    assert_eq!(fault.source.attribute(), Some("id"));
    assert_eq!(fault.source.kind, DecodeFault::Missing);
    assert_eq!(fault.capacity, Some(capacity()));
    assert_eq!(fault.raw_items(), &[raw]);
    assert!(fault.source().is_some());
}

#[test]
fn corrupt_present_records_preserve_diagnostics_capacity_and_raw_payload() {
    let request = prepared();
    for (name, value, expected) in [
        (
            "order.quantity",
            Av::S("private-value".into()),
            "wrong_type",
        ),
        (
            "order.quantity",
            Av::N("invalid-private-number".into()),
            "invalid_number",
        ),
        ("note", Av::Null(true), "wrong_type"),
        ("unknown", Av::S("private-value".into()), "unknown"),
    ] {
        let mut raw = item();
        raw.insert(name.into(), value);
        let error = request
            .decode(Ok(GetItemOutput::builder()
                .set_item(Some(raw.clone()))
                .consumed_capacity(capacity())
                .build()))
            .unwrap_err();
        for formatted in [format!("{error:?}"), format!("{error}")] {
            assert!(!formatted.contains("private-value"));
            assert!(!formatted.contains("invalid-private-number"));
        }
        assert!(error.source().is_some());
        let ReadError::Decode(fault) = error else {
            panic!("expected decode fault")
        };
        assert_eq!(fault.source.attribute(), Some(name));
        match expected {
            "wrong_type" => assert!(matches!(fault.source.kind, DecodeFault::WrongType { .. })),
            "invalid_number" => assert_eq!(fault.source.kind, DecodeFault::InvalidNumber),
            "unknown" => assert_eq!(fault.source.kind, DecodeFault::UnknownAttribute),
            _ => unreachable!(),
        }
        assert_eq!(fault.capacity, Some(capacity()));
        assert_eq!(fault.raw_items(), &[raw]);
    }
    assert!(!format!("{request:?}").contains("order-lime"));
}

#[test]
fn projections_decode_only_selected_fields_with_their_strict_codec() {
    let request = {
        let table = Table::<Order>::new("orders").unwrap();
        table
            .get(key(), ReadConsistency::Strong)
            .select::<Quantity>()
            .build()
            .unwrap()
    };
    let raw: Item = [("order.quantity".into(), Av::N("9".into()))].into();
    assert_eq!(
        request
            .decode(Ok(GetItemOutput::builder().set_item(Some(raw)).build()))
            .unwrap()
            .item,
        Some(Quantity { quantity: 9 })
    );
    for raw in [
        Item::new(),
        [("order.quantity".into(), Av::Bool(false))].into(),
    ] {
        let ReadError::Decode(fault) = request
            .decode(Ok(GetItemOutput::builder().set_item(Some(raw)).build()))
            .unwrap_err()
        else {
            panic!("expected selected-field fault")
        };
        assert_eq!(fault.source.record, "Order");
        assert_eq!(fault.source.attribute(), Some("order.quantity"));
    }
}

#[test]
fn sdk_errors_retain_the_exact_typed_source_and_redact_its_text() {
    let request = prepared();
    let sdk = SdkError::construction_failure(std::io::Error::other("private-sdk-value"));
    let error = request.decode(Err(sdk)).unwrap_err();
    assert!(matches!(&error, ReadError::Sdk(_)));
    assert!(matches!(
        error.sdk_error(),
        Some(SdkError::ConstructionFailure(_))
    ));
    assert!(error.source().is_some());
    assert!(!format!("{error:?} {error}").contains("private-sdk-value"));
}

#[test]
fn table_validation_and_key_encoding_fail_before_any_request() {
    assert!(Table::<Order>::new("").is_err());
    let arn = "arn:aws:dynamodb:us-east-1:123456789012:table/orders";
    let table = Table::<Order>::new(arn).unwrap().clone();
    assert_eq!(table.name(), arn);
    let invalid = OrderKey {
        id: String::new(),
        line: 1,
    };
    let error = table
        .get(invalid, ReadConsistency::Strong)
        .build()
        .unwrap_err();
    assert!(
        matches!(error, tabulary::BuildError::Encode(ref source) if source.record == "Order" && source.attribute() == Some("id") && source.kind == tabulary::EncodeFault::EmptyKey)
    );
}

#[test]
fn missing_consistency_and_foreign_keys_or_projections_are_compile_errors() {
    trybuild::TestCases::new().compile_fail("tests/ui/get/*.rs");
}
