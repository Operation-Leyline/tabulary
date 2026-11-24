#![cfg(feature = "derive")]

use aws_sdk_dynamodb::{
    error::SdkError,
    operation::batch_get_item::BatchGetItemOutput,
    types::{AttributeValue as Av, ConsumedCapacity, KeysAndAttributes, ReturnConsumedCapacity},
};
use aws_smithy_runtime_api::client::result::ConnectorError;
use std::error::Error;
use tabulary::{
    BatchGetSlot, BatchGetSlotError, BuildError, Capacity, DecodeFault, Item, MultiBatchGet,
    MultiBatchGetError, ReadConsistency, RecordKey, Table,
};

// These records intentionally do not implement Debug.
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
struct Invoice {
    #[tabulary(partition_key)]
    id: String,
    amount: u64,
    memo: String,
}
#[derive(Debug, PartialEq, tabulary::Projection)]
#[tabulary(record = Invoice)]
struct InvoiceAmount {
    #[tabulary(attribute = InvoiceAttrs::AMOUNT)]
    amount: u64,
}
#[derive(tabulary::Record)]
#[tabulary(rename_all = "snake_case")]
struct Session {
    #[tabulary(partition_key)]
    id: String,
    usable: bool,
}

fn order_key(line: u64) -> OrderKey {
    OrderKey::new("private-order".into(), line).unwrap()
}
fn invoice_key(id: &str) -> InvoiceKey {
    InvoiceKey::new(id.into()).unwrap()
}
fn session_key(id: &str) -> SessionKey {
    SessionKey::new(id.into()).unwrap()
}
fn order(line: u64) -> Item {
    Order {
        id: "private-order".into(),
        line,
        quantity: 7 + line,
    }
    .to_item()
    .unwrap()
}
fn invoice_projection(id: &str) -> Item {
    Item::from([
        ("id".into(), Av::S(id.into())),
        ("amount".into(), Av::N("19".into())),
    ])
}
fn keys(items: Vec<Item>) -> KeysAndAttributes {
    KeysAndAttributes::builder()
        .set_keys(Some(items))
        .build()
        .unwrap()
}
fn capacity() -> ConsumedCapacity {
    ConsumedCapacity::builder()
        .table_name("orders")
        .capacity_units(2.5)
        .build()
}
struct Slots {
    orders: BatchGetSlot<OrderKey, Order>,
    invoices: BatchGetSlot<InvoiceKey, InvoiceAmount>,
    sessions: BatchGetSlot<SessionKey, Session>,
}
fn three_tables() -> (MultiBatchGet, Slots) {
    let mut batch = MultiBatchGet::new();
    let slots = Slots {
        orders: batch.get(
            &Table::new("orders").unwrap(),
            (0..3).map(order_key).collect(),
            ReadConsistency::Strong,
        ),
        invoices: batch.get_projected::<InvoiceAmount, _>(
            &Table::new("invoices").unwrap(),
            vec![invoice_key("invoice-1"), invoice_key("invoice-2")],
            ReadConsistency::Eventual,
        ),
        sessions: batch.get(
            &Table::new("sessions").unwrap(),
            vec![session_key("session-1")],
            ReadConsistency::Strong,
        ),
    };
    (batch, slots)
}

#[test]
fn three_tables_decode_found_missing_and_typed_unprocessed_keys() {
    let (mut batch, slots) = three_tables();
    batch.set_capacity(Capacity::Indexes);
    let request = batch.build().unwrap();
    let input = request.input();
    assert_eq!(
        input.return_consumed_capacity,
        Some(ReturnConsumedCapacity::Indexes)
    );
    let items = input.request_items.as_ref().unwrap();
    assert_eq!(items.len(), 3);
    assert_eq!(items["orders"].keys.len(), 3);
    assert_eq!(items["orders"].consistent_read, Some(true));
    assert_eq!(items["orders"].projection_expression, None);
    assert_eq!(items["invoices"].consistent_read, Some(false));
    assert_eq!(
        items["invoices"].projection_expression.as_deref(),
        Some("#n0, #n1")
    );
    let names = items["invoices"]
        .expression_attribute_names
        .as_ref()
        .unwrap();
    assert_eq!(
        (names["#n0"].as_str(), names["#n1"].as_str()),
        ("amount", "id")
    );
    assert_eq!(
        items["sessions"].keys,
        vec![session_key("session-1").encode_key().unwrap()]
    );
    assert_eq!(request.expressions().len(), 3);
    assert_eq!(request.expressions()[1].projection(), Some("#n0, #n1"));
    assert!(!format!("{request:?}").contains("private-order"));

    let pending = items["orders"].keys[1].clone();
    let raw = BatchGetItemOutput::builder()
        // Responses arrive in any order and are correlated by full key.
        .responses("orders", vec![order(2), order(0)])
        .responses("invoices", vec![invoice_projection("invoice-1")])
        .unprocessed_keys("orders", keys(vec![pending]))
        .unprocessed_keys(
            "sessions",
            keys(vec![session_key("session-1").encode_key().unwrap()]),
        )
        .consumed_capacity(capacity())
        .build();
    let response = request.decode(Ok(raw)).unwrap();
    assert_eq!(response.capacity, Some(vec![capacity()]));
    assert!(!format!("{response:?}").contains("private-order"));

    let orders = response.get(&slots.orders).unwrap();
    assert_eq!(
        orders
            .found
            .iter()
            .map(|order| (order.line, order.quantity))
            .collect::<Vec<_>>(),
        [(0, 7), (2, 9)]
    );
    assert!(orders.missing.is_empty());
    assert_eq!(orders.unprocessed.len(), 1);
    assert_eq!(
        orders.unprocessed[0].encode_key().unwrap(),
        order_key(1).encode_key().unwrap()
    );
    assert_eq!(
        format!("{orders:?}"),
        "BatchGetTableResult { found: 2, missing: 0, unprocessed: 1 }"
    );

    let invoices = response.get(&slots.invoices).unwrap();
    assert_eq!(invoices.found, [InvoiceAmount { amount: 19 }]);
    assert_eq!(invoices.missing.len(), 1);
    assert_eq!(
        invoices.missing[0].encode_key().unwrap(),
        invoice_key("invoice-2").encode_key().unwrap()
    );
    assert!(invoices.unprocessed.is_empty());

    let sessions = response.get(&slots.sessions.clone()).unwrap();
    assert!(sessions.found.is_empty() && sessions.missing.is_empty());

    // The caller owns retry: unprocessed keys feed a new request directly.
    let mut retry = MultiBatchGet::new();
    retry.get(
        &Table::<Order>::new("orders").unwrap(),
        orders.unprocessed,
        ReadConsistency::Strong,
    );
    retry.get(
        &Table::<Session>::new("sessions").unwrap(),
        sessions.unprocessed,
        ReadConsistency::Strong,
    );
    let retry = retry.build().unwrap();
    let retried = retry.input().request_items.as_ref().unwrap();
    assert_eq!(
        retried["orders"].keys,
        vec![order_key(1).encode_key().unwrap()]
    );
    assert_eq!(retried["sessions"].keys, items["sessions"].keys);
}

#[test]
fn apply_replaces_every_request_field_without_transport() {
    use aws_sdk_dynamodb::{
        Client, Config,
        config::{IdentityCache, Region, retry::RetryConfig, timeout::TimeoutConfig},
    };
    let client = Client::from_conf(
        Config::builder()
            .behavior_version_latest()
            .region(Region::new("us-east-1"))
            .retry_config(RetryConfig::disabled())
            .timeout_config(TimeoutConfig::disabled())
            .identity_cache(IdentityCache::no_cache())
            .http_client(aws_smithy_runtime_api::client::http::http_client_fn(
                |_, _| panic!("no transport"),
            ))
            .build(),
    );
    let (batch, _) = three_tables();
    let request = batch.build().unwrap();
    let stale = client
        .batch_get_item()
        .request_items("stale", keys(vec![Item::new()]))
        .return_consumed_capacity(ReturnConsumedCapacity::Total);
    assert_eq!(
        request.apply(stale).as_input().clone().build().unwrap(),
        *request.input()
    );
    assert_eq!(request.input().return_consumed_capacity, None);
}

#[test]
fn build_refuses_empty_duplicate_and_more_than_one_hundred_keys() {
    let orders = Table::<Order>::new("orders").unwrap();
    let sessions = Table::<Session>::new("sessions").unwrap();
    let code = |code| Err(BuildError::OperationConstraint { code });
    assert_eq!(
        MultiBatchGet::new().build().unwrap_err(),
        BuildError::EmptyOperation
    );
    let mut batch = MultiBatchGet::default();
    batch.get(&orders, Vec::<OrderKey>::new(), ReadConsistency::Strong);
    assert_eq!(batch.build().unwrap_err(), BuildError::EmptyOperation);

    let mut batch = MultiBatchGet::new();
    batch.get(
        &orders,
        vec![order_key(1), order_key(1)],
        ReadConsistency::Strong,
    );
    assert_eq!(batch.build().map(|_| ()), code("duplicate_batch_key"));

    // One key in two different tables is not a duplicate.
    let mut batch = MultiBatchGet::new();
    batch.get(&orders, vec![order_key(1)], ReadConsistency::Strong);
    batch.get(
        &Table::<Order>::new("orders-archive").unwrap(),
        vec![order_key(1)],
        ReadConsistency::Strong,
    );
    assert!(batch.build().is_ok());

    // Same-named tables in different accounts or regions are distinct tables.
    for (first, second, expected) in [
        (
            "arn:aws:dynamodb:us-east-1:111122223333:table/orders",
            "arn:aws:dynamodb:us-east-1:444455556666:table/orders",
            Ok(()),
        ),
        (
            "arn:aws:dynamodb:us-east-1:111122223333:table/orders",
            "arn:aws:dynamodb:eu-west-1:111122223333:table/orders",
            Ok(()),
        ),
        (
            "arn:aws:dynamodb:us-east-1:111122223333:table/orders",
            "arn:aws:dynamodb:us-east-1:111122223333:table/orders",
            code("duplicate_batch_table"),
        ),
        (
            "orders",
            "arn:aws:dynamodb:us-east-1:444455556666:table/orders",
            code("duplicate_batch_table"),
        ),
    ] {
        let mut batch = MultiBatchGet::new();
        batch.get(
            &Table::<Order>::new(first).unwrap(),
            vec![order_key(1)],
            ReadConsistency::Strong,
        );
        batch.get(
            &Table::<Order>::new(second).unwrap(),
            vec![order_key(2)],
            ReadConsistency::Strong,
        );
        assert_eq!(batch.build().map(|_| ()), expected, "{first} {second}");
    }

    for alias in [
        "orders",
        "arn:aws:dynamodb:us-east-1:111122223333:table/orders",
    ] {
        let mut batch = MultiBatchGet::new();
        batch.get(&orders, vec![order_key(1)], ReadConsistency::Strong);
        batch.get(
            &Table::<Order>::new(alias).unwrap(),
            vec![order_key(2)],
            ReadConsistency::Eventual,
        );
        assert_eq!(batch.build().map(|_| ()), code("duplicate_batch_table"));
    }

    for (order_count, expected) in [
        (99, Ok(())),
        (
            100,
            Err(BuildError::RequestLimit {
                max: 100,
                actual: 101,
            }),
        ),
    ] {
        let mut batch = MultiBatchGet::new();
        batch.get(
            &orders,
            (0..order_count).map(order_key).collect(),
            ReadConsistency::Strong,
        );
        batch.get(
            &sessions,
            vec![session_key("session-1")],
            ReadConsistency::Strong,
        );
        assert_eq!(batch.build().map(|_| ()), expected);
    }
}

#[test]
fn malformed_items_fail_their_own_table_listing_every_position() {
    let (batch, slots) = three_tables();
    let request = batch.build().unwrap();
    let mut wrong = order(0);
    wrong.insert("quantity".into(), Av::S("seven".into()));
    let mut missing = order(2);
    missing.remove("quantity");
    let raw = BatchGetItemOutput::builder()
        .responses("orders", vec![missing, order(1), wrong])
        .responses("invoices", vec![invoice_projection("invoice-2")])
        .build();
    let response = request.decode(Ok(raw)).unwrap();
    let error = response.get(&slots.orders).unwrap_err();
    assert!(error.source().is_some());
    assert!(!format!("{error:?} {error}").contains("seven"));
    let BatchGetSlotError::Decode { failures } = error else {
        panic!("expected decode")
    };
    assert_eq!(
        failures
            .iter()
            .map(|(position, error)| (*position, error.attribute().unwrap(), error.kind))
            .collect::<Vec<_>>(),
        [
            (
                0,
                "quantity",
                DecodeFault::WrongType {
                    expected: tabulary::DynamoType::N,
                    actual: tabulary::DynamoType::S
                }
            ),
            (2, "quantity", DecodeFault::Missing),
        ]
    );
    // Other tables of the same response still decode strictly.
    let invoices = response.get(&slots.invoices).unwrap();
    assert_eq!(invoices.found, [InvoiceAmount { amount: 19 }]);
    assert_eq!(invoices.missing.len(), 1);
    assert_eq!(response.get(&slots.sessions).unwrap().missing.len(), 1);
}

#[test]
fn protocol_faults_reject_the_whole_response_and_keep_capacity() {
    let (batch, _) = three_tables();
    let request = batch.build().unwrap();
    let order_keys = request.input().request_items.as_ref().unwrap()["orders"]
        .keys
        .clone();
    let mut with_extra = order_keys[0].clone();
    with_extra.insert("quantity".into(), Av::N("1".into()));
    let mut keyless = invoice_projection("invoice-1");
    keyless.remove("id");
    let cases: Vec<BatchGetItemOutput> = vec![
        // A table that was never requested, as items or as unprocessed keys.
        BatchGetItemOutput::builder()
            .responses("customers", vec![])
            .build(),
        BatchGetItemOutput::builder()
            .unprocessed_keys("customers", keys(vec![order_keys[0].clone()]))
            .build(),
        // A key that was never requested.
        BatchGetItemOutput::builder()
            .responses("orders", vec![order(9)])
            .build(),
        // A requested key in the wrong table.
        BatchGetItemOutput::builder()
            .responses("sessions", vec![order(0)])
            .build(),
        // The same item twice, or both returned and unprocessed.
        BatchGetItemOutput::builder()
            .responses("orders", vec![order(0), order(0)])
            .build(),
        BatchGetItemOutput::builder()
            .responses("orders", vec![order(0)])
            .unprocessed_keys("orders", keys(vec![order_keys[0].clone()]))
            .build(),
        // Unprocessed entries must be exact keys; items need their full key.
        BatchGetItemOutput::builder()
            .unprocessed_keys("orders", keys(vec![with_extra]))
            .build(),
        BatchGetItemOutput::builder()
            .responses("invoices", vec![keyless])
            .build(),
    ];
    for mut raw in cases {
        raw.consumed_capacity = Some(vec![capacity()]);
        let error = request.decode(Ok(raw)).unwrap_err();
        assert!(error.sdk_error().is_none());
        assert!(!format!("{error:?} {error}").contains("private-order"));
        let MultiBatchGetError::InvalidResponse { capacity: received } = error else {
            panic!("expected protocol fault")
        };
        assert_eq!(received, Some(vec![capacity()]));
    }
}

#[test]
fn unprocessed_numeric_keys_correlate_by_canonical_value() {
    let (batch, slots) = three_tables();
    let request = batch.build().unwrap();
    let mut spelled = order_key(1).encode_key().unwrap();
    spelled.insert("line".into(), Av::N("1.0".into()));
    let raw = BatchGetItemOutput::builder()
        .unprocessed_keys("orders", keys(vec![spelled]))
        .build();
    let response = request.decode(Ok(raw)).unwrap();
    let orders = response.get(&slots.orders).unwrap();
    assert_eq!(orders.missing.len(), 2);
    assert_eq!(
        orders.unprocessed[0].encode_key().unwrap(),
        order_key(1).encode_key().unwrap()
    );
}

#[test]
fn sdk_failures_and_foreign_slots_are_typed() {
    let (first, first_slots) = three_tables();
    let (second, _) = three_tables();
    let first = first.build().unwrap();
    let second = second.build().unwrap();
    let error = second
        .decode(Err(SdkError::timeout_error(ConnectorError::timeout(
            "private-sdk".into(),
        ))))
        .unwrap_err();
    assert!(matches!(error, MultiBatchGetError::Sdk(_)));
    assert!(error.sdk_error().is_some());
    assert!(error.source().is_some());
    assert!(!format!("{error:?} {error}").contains("private-sdk"));

    let empty = || BatchGetItemOutput::builder().build();
    let response = second.decode(Ok(empty())).unwrap();
    assert_eq!(response.capacity, None);
    assert_eq!(
        response.get(&first_slots.orders).unwrap_err(),
        BatchGetSlotError::Foreign
    );
    let response = first.decode(Ok(empty())).unwrap();
    assert_eq!(response.get(&first_slots.orders).unwrap().missing.len(), 3);
}
