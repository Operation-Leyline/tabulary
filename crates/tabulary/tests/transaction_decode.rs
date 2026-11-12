#![cfg(feature = "derive")]

use aws_sdk_dynamodb::{
    error::SdkError,
    operation::transact_write_items::{TransactWriteItemsError, TransactWriteItemsOutput},
    types::{
        AttributeValue as Av, CancellationReason, ConsumedCapacity, ItemCollectionMetrics,
        error::{TransactionCanceledException, TransactionInProgressException},
    },
};
use aws_smithy_runtime_api::{
    client::{orchestrator::HttpResponse, result::ConnectorError},
    http::StatusCode,
};
use aws_smithy_types::{body::SdkBody, error::ErrorMetadata};
use std::error::Error;
use tabulary::{
    Cancellation, Failure, FailureKind, Item, OldItem, OldItemMode, PreparedTransaction,
    ReasonCompleteness, Table, TxAction, TxError, Update, transaction,
};

// These records intentionally do not implement Debug.
#[derive(tabulary::Record)]
#[tabulary(rename_all = "snake_case")]
struct Order {
    #[tabulary(partition_key)]
    id: String,
    quantity: u64,
}
#[derive(tabulary::Record)]
#[tabulary(rename_all = "snake_case")]
struct Invoice {
    #[tabulary(partition_key)]
    id: String,
    amount: u64,
}
#[derive(tabulary::Record)]
#[tabulary(rename_all = "snake_case")]
struct Session {
    #[tabulary(partition_key)]
    id: String,
    usable: bool,
}
type Actions = (TxAction<Order>, TxAction<Invoice>, TxAction<Session>);
type Reasons = (Failure<Order>, Failure<Invoice>, Failure<Session>);
fn prepared(mode: OldItemMode) -> PreparedTransaction<Actions> {
    transaction((
        Table::<Order>::new("orders")
            .unwrap()
            .put(&Order {
                id: "private-order".into(),
                quantity: 7,
            })
            .on_condition_failure(mode)
            .into_tx()
            .unwrap(),
        Table::<Invoice>::new("invoices")
            .unwrap()
            .update(InvoiceKey::new("invoice".into()).unwrap())
            .changes(Update::new().and(InvoiceAttrs::AMOUNT.set(9)))
            .on_condition_failure(mode)
            .into_tx()
            .unwrap(),
        Table::<Session>::new("sessions")
            .unwrap()
            .check(
                SessionKey::new("session".into()).unwrap(),
                SessionAttrs::USABLE.eq(true),
            )
            .on_condition_failure(mode)
            .into_tx()
            .unwrap(),
    ))
    .build()
    .unwrap()
}
fn response() -> HttpResponse {
    HttpResponse::new(
        StatusCode::try_from(400).unwrap(),
        SdkBody::from("private-response"),
    )
}
fn cancellation(reasons: Option<Vec<CancellationReason>>) -> SdkError<TransactWriteItemsError> {
    SdkError::service_error(
        TransactWriteItemsError::TransactionCanceledException(
            TransactionCanceledException::builder()
                .message("ConditionalCheckFailed private-sdk")
                .meta(
                    ErrorMetadata::builder()
                        .custom("request_id", "request-proof")
                        .build(),
                )
                .set_cancellation_reasons(reasons)
                .build(),
        ),
        response(),
    )
}
fn reason(code: Option<&str>, item: Option<Item>) -> CancellationReason {
    CancellationReason::builder()
        .set_code(code.map(str::to_owned))
        .message("ThrottlingError private-sdk")
        .set_item(item)
        .build()
}
fn old_order() -> Item {
    Order {
        id: "private-order".into(),
        quantity: 7,
    }
    .to_item()
    .unwrap()
}
fn cancelled(error: TxError<Reasons>) -> Cancellation<Reasons> {
    assert_redacted(&error);
    assert!(
        error
            .source()
            .unwrap()
            .is::<SdkError<TransactWriteItemsError>>()
    );
    let TxError::Cancelled(cancelled) = error else {
        panic!("expected cancellation")
    };
    cancelled
}
fn assert_redacted(value: &(impl std::fmt::Debug + std::fmt::Display)) {
    let formatted = format!("{value:?} {value}");
    for secret in ["private-order", "private-sdk", "private-response"] {
        assert!(!formatted.contains(secret));
    }
}

#[test]
fn three_heterogeneous_reasons_decode_their_own_old_records() {
    let raw = vec![
        reason(Some("ConditionalCheckFailed"), Some(old_order())),
        reason(
            Some("TransactionConflict"),
            Some(
                Invoice {
                    id: "invoice".into(),
                    amount: 19,
                }
                .to_item()
                .unwrap(),
            ),
        ),
        reason(
            Some("None"),
            Some(
                Session {
                    id: "session".into(),
                    usable: false,
                }
                .to_item()
                .unwrap(),
            ),
        ),
    ];
    let error = prepared(OldItemMode::All)
        .decode(Err(cancellation(Some(raw.clone()))))
        .unwrap_err();
    assert!(
        error
            .sdk_error()
            .as_service_error()
            .unwrap()
            .is_transaction_canceled_exception()
    );
    let result = cancelled(error);
    assert_eq!(result.completeness, ReasonCompleteness::Complete);
    assert_eq!(result.raw_reasons(), Some(raw.as_slice()));
    let service = result.source.as_service_error().unwrap();
    assert_eq!(service.meta().extra("request_id"), Some("request-proof"));
    let (order, invoice, session): Reasons = result.reasons;
    assert_eq!(order.kind, FailureKind::ConditionFailed);
    assert_eq!(invoice.kind, FailureKind::TransactionConflict);
    assert_eq!(session.kind, FailureKind::NoFailureReported);
    let OldItem::Decoded(order) = order.old else {
        panic!("order")
    };
    let OldItem::Decoded(invoice) = invoice.old else {
        panic!("invoice")
    };
    let OldItem::Decoded(session) = session.old else {
        panic!("session")
    };
    assert_eq!(order.quantity, 7);
    assert_eq!(invoice.amount, 19);
    assert!(!session.usable);
}

#[test]
fn every_machine_code_is_classified_without_reading_messages() {
    let table = Table::<Order>::new("orders").unwrap();
    let request = transaction(
        table
            .delete(OrderKey::new("order".into()).unwrap())
            .on_condition_failure(OldItemMode::All)
            .into_tx()
            .unwrap(),
    )
    .build()
    .unwrap();
    for (code, expected) in [
        (Some("ConditionalCheckFailed"), FailureKind::ConditionFailed),
        (
            Some("TransactionConflict"),
            FailureKind::TransactionConflict,
        ),
        (
            Some("ProvisionedThroughputExceeded"),
            FailureKind::ProvisionedThroughputExceeded,
        ),
        (Some("ThrottlingError"), FailureKind::Throttled),
        (
            Some("ItemCollectionSizeLimitExceeded"),
            FailureKind::ItemCollectionSizeLimitExceeded,
        ),
        (Some("ValidationError"), FailureKind::Validation),
        (None, FailureKind::NoFailureReported),
        (Some("None"), FailureKind::NoFailureReported),
        (
            Some("FutureCode"),
            FailureKind::UnknownServiceCode("FutureCode".into()),
        ),
        (Some(""), FailureKind::UnknownServiceCode("".into())),
    ] {
        let TxError::Cancelled(result) = request
            .decode(Err(cancellation(Some(vec![reason(
                code,
                Some(old_order()),
            )]))))
            .unwrap_err()
        else {
            panic!("expected cancellation")
        };
        assert_eq!(result.reasons.kind, expected);
        assert!(matches!(
            result.reasons.old,
            OldItem::Decoded(Order { quantity: 7, .. })
        ));
    }
}

#[test]
fn missing_and_misaligned_vectors_never_guess_positions() {
    for (raw, expected) in [
        (None, ReasonCompleteness::Missing),
        (
            Some(vec![]),
            ReasonCompleteness::MalformedLength {
                expected: 3,
                actual: 0,
            },
        ),
        (
            Some(vec![reason(
                Some("ConditionalCheckFailed"),
                Some(old_order()),
            )]),
            ReasonCompleteness::MalformedLength {
                expected: 3,
                actual: 1,
            },
        ),
        (
            Some(vec![reason(Some("ValidationError"), Some(old_order())); 4]),
            ReasonCompleteness::MalformedLength {
                expected: 3,
                actual: 4,
            },
        ),
    ] {
        let result = cancelled(
            prepared(OldItemMode::All)
                .decode(Err(cancellation(raw.clone())))
                .unwrap_err(),
        );
        assert_eq!(result.completeness, expected);
        assert_eq!(result.raw_reasons(), raw.as_deref());
        let (a, b, c) = result.reasons;
        assert!(
            [a.kind, b.kind, c.kind]
                .iter()
                .all(|kind| *kind == FailureKind::Unavailable)
        );
        assert!(matches!(a.old, OldItem::Unavailable));
        assert!(matches!(b.old, OldItem::Unavailable));
        assert!(matches!(c.old, OldItem::Unavailable));
    }
}

#[test]
fn invalid_old_data_keeps_the_refusal_and_raw_item() {
    for value in [
        None,
        Some(Av::Bool(true)),
        Some(Av::N("private-order".into())),
    ] {
        let mut malformed = old_order();
        malformed.remove("quantity");
        if let Some(value) = value {
            malformed.insert("quantity".into(), value);
        }
        let raw = vec![
            reason(Some("ConditionalCheckFailed"), Some(malformed.clone())),
            reason(None, None),
            reason(None, None),
        ];
        let result = cancelled(
            prepared(OldItemMode::All)
                .decode(Err(cancellation(Some(raw))))
                .unwrap_err(),
        );
        assert_eq!(result.reasons.0.kind, FailureKind::ConditionFailed);
        let OldItem::Invalid { error, raw } = result.reasons.0.old else {
            panic!("invalid old item")
        };
        assert_eq!(error.record, "Order");
        assert_eq!(error.attribute(), Some("quantity"));
        assert_eq!(raw.expose(), &malformed);
        assert!(matches!(result.reasons.1.old, OldItem::Unavailable));
    }
    let raw = vec![reason(Some("ConditionalCheckFailed"), Some(Item::new())); 3];
    let result = cancelled(
        prepared(OldItemMode::None)
            .decode(Err(cancellation(Some(raw.clone()))))
            .unwrap_err(),
    );
    assert_eq!(result.raw_reasons(), Some(raw.as_slice()));
    assert!(matches!(result.reasons.0.old, OldItem::NotRequested));
    assert!(matches!(result.reasons.1.old, OldItem::NotRequested));
    assert!(matches!(result.reasons.2.old, OldItem::NotRequested));
}

#[test]
fn success_preserves_accounting_and_absence() {
    let capacity = vec![
        ConsumedCapacity::builder()
            .table_name("orders")
            .capacity_units(2.5)
            .build(),
    ];
    let metrics = std::collections::HashMap::from([(
        "orders".into(),
        vec![
            ItemCollectionMetrics::builder()
                .item_collection_key("id", Av::S("private-order".into()))
                .build(),
        ],
    )]);
    let output = TransactWriteItemsOutput::builder()
        .set_consumed_capacity(Some(capacity.clone()))
        .set_item_collection_metrics(Some(metrics.clone()))
        .build();
    let request = prepared(OldItemMode::All);
    let result = request.decode(Ok(output)).unwrap();
    assert_eq!(result.capacity, Some(capacity));
    assert_eq!(result.item_collection_metrics, Some(metrics));
    assert!(!format!("{result:?}").contains("private-order"));
    let result = request
        .decode(Ok(TransactWriteItemsOutput::builder().build()))
        .unwrap();
    assert!(result.capacity.is_none());
    assert!(result.item_collection_metrics.is_none());
}

#[test]
fn tuple_arity_boundaries_preserve_order_and_action_local_old_item_modes() {
    let table = Table::<Order>::new("orders").unwrap();
    macro_rules! check {
        ($($id:tt),+) => {{
            let request = transaction(($(table.delete(OrderKey::new($id.to_string()).unwrap())
                .on_condition_failure(if $id == 0 { OldItemMode::None } else { OldItemMode::All })
                .into_tx().unwrap(),)+)).build().unwrap();
            let raw = vec![$(reason(Some("ConditionalCheckFailed"), Some(Order {
                id: $id.to_string(), quantity: $id,
            }.to_item().unwrap())),)+];
            let TxError::Cancelled(result) = request.decode(Err(cancellation(Some(raw)))).unwrap_err()
                else { panic!("expected cancellation") };
            assert_eq!(result.completeness, ReasonCompleteness::Complete);
            $(assert_eq!(result.reasons.$id.kind, FailureKind::ConditionFailed);
            if $id == 0 {
                assert!(matches!(result.reasons.$id.old, OldItem::NotRequested));
            } else {
                let OldItem::Decoded(old) = &result.reasons.$id.old else { panic!("old record") };
                assert_eq!(old.quantity, $id);
            })+
        }};
    }
    check!(0);
    check!(0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11);
}

#[test]
fn non_cancellation_and_transport_failures_keep_the_exact_sdk_source() {
    let request = prepared(OldItemMode::All);
    for sdk in [
        SdkError::service_error(
            TransactWriteItemsError::TransactionInProgressException(
                TransactionInProgressException::builder()
                    .message("TransactionCanceledException private-sdk")
                    .build(),
            ),
            response(),
        ),
        SdkError::construction_failure(std::io::Error::other("private-sdk")),
        SdkError::timeout_error(std::io::Error::other(
            "TransactionCanceledException private-sdk",
        )),
        SdkError::dispatch_failure(ConnectorError::io(Box::new(std::io::Error::other(
            "private-sdk",
        )))),
        SdkError::response_error(
            std::io::Error::other("TransactionCanceledException private-sdk"),
            response(),
        ),
    ] {
        let variant = std::mem::discriminant(&sdk);
        let error = request.decode(Err(sdk)).unwrap_err();
        assert!(matches!(error, TxError::Sdk(_)));
        assert_redacted(&error);
        assert_eq!(std::mem::discriminant(error.sdk_error()), variant);
        assert!(
            error
                .source()
                .unwrap()
                .is::<SdkError<TransactWriteItemsError>>()
        );
    }
}

#[allow(dead_code)]
#[path = "../../../docs/examples/05_transaction.rs"]
mod example;
#[test]
fn promoted_transaction_example_compiles() {
    let _ = example::checkout;
}
