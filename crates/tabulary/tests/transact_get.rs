#![cfg(feature = "derive")]

use aws_sdk_dynamodb::{
    error::SdkError,
    operation::transact_get_items::{TransactGetItemsError, TransactGetItemsOutput},
    types::{
        AttributeValue as Av, CancellationReason, ConsumedCapacity, ItemResponse,
        ReturnConsumedCapacity, error::TransactionCanceledException,
    },
};
use aws_smithy_runtime_api::{
    client::{orchestrator::HttpResponse, result::ConnectorError},
    http::StatusCode,
};
use aws_smithy_types::body::SdkBody;
use std::error::Error;
use tabulary::{
    BuildError, Capacity, DecodeFault, DynamoType, FailureKind, GetSlot, GetSlotError, Item,
    ReasonCompleteness, RecordKey, Table, TransactGet, TransactGetError,
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

fn orders() -> Table<Order> {
    Table::new("orders").unwrap()
}
fn invoices() -> Table<Invoice> {
    Table::new("invoices").unwrap()
}
fn sessions() -> Table<Session> {
    Table::new("sessions").unwrap()
}
fn order_key(line: u64) -> OrderKey {
    OrderKey::new("private-order".into(), line).unwrap()
}
fn order(line: u64) -> Item {
    Order {
        id: "private-order".into(),
        line,
        quantity: 7,
    }
    .to_item()
    .unwrap()
}
fn invoice_projection() -> Item {
    Item::from([
        ("id".into(), Av::S("invoice-1".into())),
        ("amount".into(), Av::N("19".into())),
    ])
}
fn session(id: &str) -> Item {
    Session {
        id: id.into(),
        usable: true,
    }
    .to_item()
    .unwrap()
}
fn output(items: Vec<Option<Item>>) -> TransactGetItemsOutput {
    TransactGetItemsOutput::builder()
        .set_responses(Some(
            items
                .into_iter()
                .map(|item| ItemResponse::builder().set_item(item).build())
                .collect(),
        ))
        .consumed_capacity(
            ConsumedCapacity::builder()
                .table_name("orders")
                .capacity_units(6.0)
                .build(),
        )
        .build()
}
struct Slots {
    order: GetSlot<Order>,
    invoice: GetSlot<InvoiceAmount>,
    session: GetSlot<Session>,
}
fn three_tables() -> (TransactGet, Slots) {
    let mut tx = TransactGet::new();
    let slots = Slots {
        order: tx.get(&orders(), order_key(1)),
        invoice: tx.get_projected::<InvoiceAmount, _>(
            &invoices(),
            InvoiceKey::new("invoice-1".into()).unwrap(),
        ),
        session: tx.get(&sessions(), SessionKey::new("session-1".into()).unwrap()),
    };
    (tx, slots)
}
fn response() -> HttpResponse {
    HttpResponse::new(
        StatusCode::try_from(400).unwrap(),
        SdkBody::from("private-response"),
    )
}
fn cancellation(reasons: Option<Vec<CancellationReason>>) -> SdkError<TransactGetItemsError> {
    SdkError::service_error(
        TransactGetItemsError::TransactionCanceledException(
            TransactionCanceledException::builder()
                .message("TransactionConflict private-sdk")
                .set_cancellation_reasons(reasons)
                .build(),
        ),
        response(),
    )
}
fn reason(code: Option<&str>) -> CancellationReason {
    CancellationReason::builder()
        .set_code(code.map(str::to_owned))
        .message("ThrottlingError private-sdk")
        .build()
}
fn assert_redacted(value: &(impl std::fmt::Debug + std::fmt::Display)) {
    let formatted = format!("{value:?} {value}");
    for secret in ["private-order", "private-sdk", "private-response"] {
        assert!(!formatted.contains(secret), "{formatted}");
    }
}

#[test]
fn three_tables_decode_present_missing_and_projected_slots_with_capacity() {
    let (mut tx, slots) = three_tables();
    assert_eq!(
        (
            slots.order.position(),
            slots.invoice.position(),
            slots.session.position()
        ),
        (0, 1, 2)
    );
    tx.set_capacity(Capacity::Total);
    let request = tx.build().unwrap();
    let input = request.input();
    assert_eq!(
        input.return_consumed_capacity,
        Some(ReturnConsumedCapacity::Total)
    );
    let items = input.transact_items.as_ref().unwrap();
    let gets: Vec<_> = items
        .iter()
        .map(|item| item.get.as_ref().unwrap())
        .collect();
    assert_eq!(
        gets.iter()
            .map(|get| get.table_name.as_str())
            .collect::<Vec<_>>(),
        ["orders", "invoices", "sessions"]
    );
    assert_eq!(gets[0].key, OrderKey::encode_key(&order_key(1)).unwrap());
    assert_eq!(gets[0].projection_expression, None);
    assert_eq!(gets[0].expression_attribute_names, None);
    // The projection selects its attribute, then the omitted physical key.
    assert_eq!(gets[1].projection_expression.as_deref(), Some("#n0, #n1"));
    let names = gets[1].expression_attribute_names.as_ref().unwrap();
    assert_eq!(
        (names["#n0"].as_str(), names["#n1"].as_str()),
        ("amount", "id")
    );
    assert_eq!(request.expressions()[1].projection(), Some("#n0, #n1"));
    assert_eq!(request.expressions()[0].projection(), None);
    assert!(!format!("{request:?}").contains("private-order"));

    let raw = output(vec![Some(order(1)), Some(invoice_projection()), None]);
    let capacity = raw.consumed_capacity.clone();
    let response = request.decode(Ok(raw)).unwrap();
    assert_eq!(response.capacity, capacity);
    let order = response.get(&slots.order).unwrap().unwrap();
    assert_eq!((order.line, order.quantity), (1, 7));
    assert_eq!(
        response.get(&slots.invoice).unwrap(),
        Some(InvoiceAmount { amount: 19 })
    );
    assert!(response.get(&slots.session).unwrap().is_none());
    // Slot access is repeatable and never consumes the response.
    assert!(response.get(&slots.order.clone()).unwrap().is_some());
    assert_eq!(response.raw_items().len(), 3);
    assert!(!format!("{response:?}").contains("private-order"));
}

#[test]
fn apply_replaces_every_request_field_without_transport() {
    use aws_sdk_dynamodb::{
        Client, Config,
        config::{IdentityCache, Region, retry::RetryConfig, timeout::TimeoutConfig},
        types::{Get, TransactGetItem},
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
    let (tx, _) = three_tables();
    let request = tx.build().unwrap();
    let stale = client
        .transact_get_items()
        .transact_items(
            TransactGetItem::builder()
                .get(
                    Get::builder()
                        .table_name("stale")
                        .key("id", Av::S("stale".into()))
                        .build()
                        .unwrap(),
                )
                .build(),
        )
        .return_consumed_capacity(ReturnConsumedCapacity::Indexes);
    assert_eq!(
        request.apply(stale).as_input().clone().build().unwrap(),
        *request.input()
    );
    assert_eq!(request.input().return_consumed_capacity, None);
}

#[test]
fn build_refuses_empty_and_more_than_one_hundred_reads() {
    assert_eq!(
        TransactGet::new().build().unwrap_err(),
        BuildError::EmptyOperation
    );
    assert_eq!(
        TransactGet::default().build().unwrap_err(),
        BuildError::EmptyOperation
    );
    for (count, expected) in [
        (1, Ok(())),
        (100, Ok(())),
        (
            101,
            Err(BuildError::RequestLimit {
                max: 100,
                actual: 101,
            }),
        ),
    ] {
        let mut tx = TransactGet::new();
        for line in 0..count {
            tx.get(&orders(), order_key(line));
        }
        assert_eq!(tx.build().map(|_| ()), expected);
    }
}

#[test]
fn duplicate_targets_aliases_and_locations_are_refused() {
    let duplicate = |second: &str| {
        let mut tx = TransactGet::new();
        tx.get(&orders(), order_key(1));
        tx.get(&Table::<Order>::new(second).unwrap(), order_key(1));
        tx.build().map(|_| ())
    };
    let code = |code| Err(BuildError::OperationConstraint { code });
    assert_eq!(duplicate("orders"), code("duplicate_transaction_target"));
    assert_eq!(
        duplicate("arn:aws:dynamodb:us-east-1:111122223333:table/orders"),
        code("duplicate_transaction_target")
    );
    // The same key in another table is a different target.
    assert_eq!(duplicate("orders-archive"), Ok(()));
    // A projected and a full read of one item are still one target.
    let mut tx = TransactGet::new();
    tx.get(&invoices(), InvoiceKey::new("invoice-1".into()).unwrap());
    tx.get_projected::<InvoiceAmount, _>(&invoices(), InvoiceKey::new("invoice-1".into()).unwrap());
    assert_eq!(tx.build().map(|_| ()), code("duplicate_transaction_target"));

    let mut tx = TransactGet::new();
    tx.get(
        &Table::<Order>::new("arn:aws:dynamodb:us-east-1:111122223333:table/orders").unwrap(),
        order_key(1),
    );
    tx.get(
        &Table::<Session>::new("arn:aws:dynamodb:eu-west-1:111122223333:table/sessions").unwrap(),
        SessionKey::new("session-1".into()).unwrap(),
    );
    assert_eq!(tx.build().map(|_| ()), code("transaction_table_location"));
}

#[test]
fn cancellation_reasons_map_to_their_own_typed_slots() {
    let (tx, slots) = three_tables();
    let request = tx.build().unwrap();
    let raw = vec![
        reason(Some("None")),
        reason(Some("TransactionConflict")),
        reason(Some("ThrottlingError")),
    ];
    let error = request
        .decode(Err(cancellation(Some(raw.clone()))))
        .unwrap_err();
    assert_redacted(&error);
    assert!(error.sdk_error().is_some());
    assert!(
        error
            .source()
            .unwrap()
            .source()
            .unwrap()
            .is::<SdkError<TransactGetItemsError>>()
    );
    let TransactGetError::Cancelled(cancelled) = error else {
        panic!("expected cancellation")
    };
    assert_eq!(cancelled.completeness, ReasonCompleteness::Complete);
    assert_eq!(cancelled.raw_reasons(), Some(raw.as_slice()));
    assert_eq!(
        cancelled.reason(&slots.order),
        Ok(&FailureKind::NoFailureReported)
    );
    assert_eq!(
        cancelled.reason(&slots.invoice),
        Ok(&FailureKind::TransactionConflict)
    );
    assert_eq!(
        cancelled.reason(&slots.session),
        Ok(&FailureKind::Throttled)
    );
    assert_eq!(cancelled.reasons().len(), 3);

    // Unknown codes stay distinct; a null code is not a success.
    let error = request
        .decode(Err(cancellation(Some(vec![
            reason(None),
            reason(Some("ValidationError")),
            reason(Some("FutureCode")),
        ]))))
        .unwrap_err();
    let TransactGetError::Cancelled(cancelled) = error else {
        panic!("expected cancellation")
    };
    assert_eq!(
        cancelled.reasons(),
        [
            FailureKind::NoFailureReported,
            FailureKind::Validation,
            FailureKind::UnknownServiceCode("FutureCode".into())
        ]
    );
}

#[test]
fn absent_or_misaligned_reason_vectors_are_never_guessed() {
    let (tx, slots) = three_tables();
    let request = tx.build().unwrap();
    for (reasons, completeness) in [
        (None, ReasonCompleteness::Missing),
        (
            Some(vec![]),
            ReasonCompleteness::MalformedLength {
                expected: 3,
                actual: 0,
            },
        ),
        (
            Some(vec![reason(Some("TransactionConflict"))]),
            ReasonCompleteness::MalformedLength {
                expected: 3,
                actual: 1,
            },
        ),
    ] {
        let present = reasons.is_some();
        let TransactGetError::Cancelled(cancelled) =
            request.decode(Err(cancellation(reasons))).unwrap_err()
        else {
            panic!("expected cancellation")
        };
        assert_eq!(cancelled.completeness, completeness);
        assert_eq!(cancelled.raw_reasons().is_some(), present);
        assert_eq!(
            cancelled.reason(&slots.invoice),
            Ok(&FailureKind::Unavailable)
        );
        assert!(
            cancelled
                .reasons()
                .iter()
                .all(|kind| *kind == FailureKind::Unavailable)
        );
    }
}

#[test]
fn other_sdk_failures_remain_typed_sdk_errors() {
    let (tx, _) = three_tables();
    let request = tx.build().unwrap();
    let error = request
        .decode(Err(SdkError::timeout_error(ConnectorError::timeout(
            "private-sdk".into(),
        ))))
        .unwrap_err();
    assert_redacted(&error);
    assert!(matches!(error, TransactGetError::Sdk(_)));
    assert!(error.sdk_error().is_some());
}

#[test]
fn wrong_type_and_missing_attributes_fail_closed_on_their_own_slot() {
    let (tx, slots) = three_tables();
    let request = tx.build().unwrap();
    let mut corrupt = order(1);
    corrupt.insert("quantity".into(), Av::S("seven".into()));
    let mut projection = invoice_projection();
    projection.remove("amount");
    let response = request
        .decode(Ok(output(vec![
            Some(corrupt),
            Some(projection),
            Some(session("session-1")),
        ])))
        .unwrap();
    let Err(GetSlotError::Decode(error)) = response.get(&slots.order) else {
        panic!("expected decode")
    };
    assert_eq!(error.record, "Order");
    assert_eq!(error.attribute(), Some("quantity"));
    assert_eq!(
        error.kind,
        DecodeFault::WrongType {
            expected: DynamoType::N,
            actual: DynamoType::S
        }
    );
    let error = response.get(&slots.invoice).unwrap_err();
    assert!(error.source().is_some());
    let GetSlotError::Decode(error) = error else {
        panic!("expected decode")
    };
    assert_eq!(error.attribute(), Some("amount"));
    assert_eq!(error.kind, DecodeFault::Missing);
    // A malformed neighbour never hides a valid slot.
    assert!(response.get(&slots.session).unwrap().unwrap().usable);
}

#[test]
fn misaligned_or_foreign_key_responses_reject_the_whole_response() {
    let (tx, _) = three_tables();
    let request = tx.build().unwrap();
    let capacity = output(vec![]).consumed_capacity;
    let error = request
        .decode(Ok(output(vec![Some(order(1)), None])))
        .unwrap_err();
    assert!(matches!(
        &error,
        TransactGetError::ResponseCount { expected: 3, actual: Some(2), capacity: c } if *c == capacity
    ));
    assert!(error.sdk_error().is_none());
    let error = request
        .decode(Ok(TransactGetItemsOutput::builder().build()))
        .unwrap_err();
    assert!(matches!(
        error,
        TransactGetError::ResponseCount {
            expected: 3,
            actual: None,
            ..
        }
    ));
    // Swapped positions, another key, a projected item without its key, and
    // a present empty map are all protocol faults rather than decode results.
    let mut keyless = invoice_projection();
    keyless.remove("id");
    for (items, position) in [
        (vec![Some(session("session-1")), None, Some(order(1))], 0),
        (vec![Some(order(2)), None, None], 0),
        (vec![None, Some(keyless), None], 1),
        (vec![None, None, Some(session("session-2"))], 2),
        (vec![None, None, Some(Item::new())], 2),
    ] {
        let error = request.decode(Ok(output(items))).unwrap_err();
        assert_redacted(&error);
        assert!(matches!(
            &error,
            TransactGetError::KeyMismatch { position: p, capacity: c } if *p == position && *c == capacity
        ));
    }
}

#[test]
fn slots_from_another_request_are_refused() {
    let (first, first_slots) = three_tables();
    let (second, _) = three_tables();
    let first = first.build().unwrap();
    let second = second.build().unwrap();
    let response = second
        .decode(Ok(output(vec![Some(order(1)), None, None])))
        .unwrap();
    assert!(matches!(
        response.get(&first_slots.order),
        Err(GetSlotError::Foreign)
    ));
    assert!(GetSlotError::Foreign.source().is_none());
    let TransactGetError::Cancelled(cancelled) = second
        .decode(Err(cancellation(Some(vec![reason(None); 3]))))
        .unwrap_err()
    else {
        panic!("expected cancellation")
    };
    assert_eq!(
        cancelled.reason(&first_slots.session),
        Err(GetSlotError::Foreign)
    );
    // The originating request accepts its slots in every decoded response.
    let response = first
        .decode(Ok(output(vec![Some(order(1)), None, None])))
        .unwrap();
    assert!(response.get(&first_slots.order).unwrap().is_some());
}
