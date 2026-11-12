#![cfg(feature = "derive")]

use aws_sdk_dynamodb::{
    error::SdkError,
    operation::transact_write_items::TransactWriteItemsError,
    types::{AttributeValue as Av, CancellationReason, error::TransactionCanceledException},
};
use aws_smithy_runtime_api::{client::orchestrator::HttpResponse, http::StatusCode};
use aws_smithy_types::body::SdkBody;
use tabulary::{
    BuildError, Failure, FailureKind, OldItem, OldItemMode, PreparedTransaction,
    ReasonCompleteness, Table, TxAction, TxError, Update, transaction,
};

// Composition must not add Clone or Debug bounds to records.
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

fn order(id: u64) -> Order {
    Order {
        id: id.to_string(),
        quantity: id,
    }
}
fn delete(id: u64) -> TxAction<Order> {
    Table::<Order>::new("orders")
        .unwrap()
        .delete(OrderKey::new(id.to_string()).unwrap())
        .on_condition_failure(OldItemMode::All)
        .into_tx()
        .unwrap()
}
fn actions(ids: std::ops::Range<u64>) -> Vec<TxAction<Order>> {
    ids.map(delete).collect()
}
fn reason(id: u64) -> CancellationReason {
    CancellationReason::builder()
        .code(format!("position-{id}"))
        .set_item(Some(order(id).to_item().unwrap()))
        .build()
}
fn cancellation(raw: Option<Vec<CancellationReason>>) -> SdkError<TransactWriteItemsError> {
    SdkError::service_error(
        TransactWriteItemsError::TransactionCanceledException(
            TransactionCanceledException::builder()
                .set_cancellation_reasons(raw)
                .build(),
        ),
        HttpResponse::new(StatusCode::try_from(400).unwrap(), SdkBody::empty()),
    )
}
fn assert_order(failure: Failure<Order>, id: u64) {
    assert_eq!(
        failure.kind,
        FailureKind::UnknownServiceCode(format!("position-{id}"))
    );
    let OldItem::Decoded(old) = failure.old else {
        panic!("expected old order")
    };
    assert_eq!(old.id, id.to_string());
    assert_eq!(old.quantity, id);
}

type Mixed = (
    (TxAction<Order>, (Vec<TxAction<Order>>, TxAction<Invoice>)),
    Vec<TxAction<Invoice>>,
    (TxAction<Order>,),
);
type MixedReasons = (
    (Failure<Order>, (Vec<Failure<Order>>, Failure<Invoice>)),
    Vec<Failure<Invoice>>,
    (Failure<Order>,),
);
fn mixed() -> PreparedTransaction<Mixed> {
    let orders = Table::<Order>::new("orders").unwrap();
    let invoices = Table::<Invoice>::new("invoices").unwrap();
    transaction((
        (
            orders
                .put(&order(0))
                .on_condition_failure(OldItemMode::All)
                .into_tx()
                .unwrap(),
            (
                vec![
                    orders
                        .update(OrderKey::new("1".into()).unwrap())
                        .changes(Update::new().and(OrderAttrs::QUANTITY.set(1)))
                        .on_condition_failure(OldItemMode::All)
                        .into_tx()
                        .unwrap(),
                    delete(2),
                ],
                invoices
                    .check(
                        InvoiceKey::new("3".into()).unwrap(),
                        InvoiceAttrs::AMOUNT.eq(3),
                    )
                    .on_condition_failure(OldItemMode::All)
                    .into_tx()
                    .unwrap(),
            ),
        ),
        Vec::<TxAction<Invoice>>::new(),
        (orders
            .delete(OrderKey::new("4".into()).unwrap())
            .into_tx()
            .unwrap(),),
    ))
    .build()
    .unwrap()
}

#[test]
fn mixed_shape_flattens_all_action_kinds_and_rebuilds_reasons_depth_first() {
    let request = mixed();
    let frozen = request.actions();
    let expected = vec![
        frozen.0.0.input().clone(),
        frozen.0.1.0[0].input().clone(),
        frozen.0.1.0[1].input().clone(),
        frozen.0.1.1.input().clone(),
        frozen.2.0.input().clone(),
    ];
    assert_eq!(request.input().transact_items(), expected);
    let items = request.input().transact_items();
    let keys = [
        &items[0].put().unwrap().item,
        &items[1].update().unwrap().key,
        &items[2].delete().unwrap().key,
        &items[3].condition_check().unwrap().key,
        &items[4].delete().unwrap().key,
    ];
    for (id, key) in keys.into_iter().enumerate() {
        assert_eq!(key["id"], Av::S(id.to_string()));
    }
    let mut raw: Vec<_> = (0..5).map(reason).collect();
    raw[3].item = Some(
        Invoice {
            id: "3".into(),
            amount: 30,
        }
        .to_item()
        .unwrap(),
    );
    let TxError::Cancelled(result) = request
        .decode(Err(cancellation(Some(raw.clone()))))
        .unwrap_err()
    else {
        panic!("expected cancellation")
    };
    assert_eq!(result.completeness, ReasonCompleteness::Complete);
    assert_eq!(result.raw_reasons(), Some(raw.as_slice()));
    let ((first, (vector, invoice)), empty, (last,)): MixedReasons = result.reasons;
    assert_order(first, 0);
    assert_eq!(vector.len(), 2);
    for (id, failure) in (1..3).zip(vector) {
        assert_order(failure, id);
    }
    assert_eq!(
        invoice.kind,
        FailureKind::UnknownServiceCode("position-3".into())
    );
    let OldItem::Decoded(invoice) = invoice.old else {
        panic!("expected old invoice")
    };
    assert_eq!(invoice.id, "3");
    assert_eq!(invoice.amount, 30);
    assert!(empty.is_empty());
    assert_eq!(
        last.kind,
        FailureKind::UnknownServiceCode("position-4".into())
    );
    assert!(matches!(last.old, OldItem::NotRequested));
}

#[test]
fn missing_or_misaligned_reasons_preserve_nested_lengths_without_guessing() {
    let request = mixed();
    for (raw, completeness) in [
        (None, ReasonCompleteness::Missing),
        (
            Some(vec![]),
            ReasonCompleteness::MalformedLength {
                expected: 5,
                actual: 0,
            },
        ),
        (
            Some((0..4).map(reason).collect()),
            ReasonCompleteness::MalformedLength {
                expected: 5,
                actual: 4,
            },
        ),
        (
            Some((0..6).map(reason).collect()),
            ReasonCompleteness::MalformedLength {
                expected: 5,
                actual: 6,
            },
        ),
    ] {
        let TxError::Cancelled(result) =
            request.decode(Err(cancellation(raw.clone()))).unwrap_err()
        else {
            panic!("expected cancellation")
        };
        assert_eq!(result.completeness, completeness);
        assert_eq!(result.raw_reasons(), raw.as_deref());
        let ((first, (vector, invoice)), empty, (last,)): MixedReasons = result.reasons;
        assert_eq!(vector.len(), 2);
        assert!(empty.is_empty());
        for failure in std::iter::once(first).chain(vector) {
            assert_eq!(failure.kind, FailureKind::Unavailable);
            assert!(matches!(failure.old, OldItem::Unavailable));
        }
        assert_eq!(invoice.kind, FailureKind::Unavailable);
        assert!(matches!(invoice.old, OldItem::Unavailable));
        assert_eq!(last.kind, FailureKind::Unavailable);
        assert!(matches!(last.old, OldItem::NotRequested));
    }
}

#[test]
fn empty_vectors_contribute_nothing_but_the_final_request_must_be_nonempty() {
    assert!(matches!(
        transaction(actions(0..0)).build(),
        Err(BuildError::EmptyOperation)
    ));
    assert!(matches!(
        transaction((actions(0..0), (actions(0..0), (actions(0..0),)))).build(),
        Err(BuildError::EmptyOperation)
    ));
    let request = transaction((actions(0..0), (delete(0), actions(0..0))))
        .build()
        .unwrap();
    assert_eq!(request.input().transact_items().len(), 1);
    let TxError::Cancelled(result) = request
        .decode(Err(cancellation(Some(vec![reason(0)]))))
        .unwrap_err()
    else {
        panic!("expected cancellation")
    };
    assert_eq!(result.completeness, ReasonCompleteness::Complete);
    let (before, (only, after)) = result.reasons;
    assert!(before.is_empty());
    assert!(after.is_empty());
    assert_order(only, 0);
}

#[test]
fn homogeneous_vectors_accept_one_and_one_hundred_actions_and_refuse_over_cap() {
    for count in [1, 100, 101] {
        let result = transaction(actions(0..count)).build();
        if count > 100 {
            assert!(matches!(
                result,
                Err(BuildError::RequestLimit {
                    max: 100,
                    actual: 101
                })
            ));
            continue;
        }
        let request = result.unwrap();
        assert_eq!(request.input().transact_items().len(), count as usize);
        for (id, item) in request.input().transact_items().iter().enumerate() {
            assert_eq!(item.delete().unwrap().key["id"], Av::S(id.to_string()));
        }
        let TxError::Cancelled(result) = request
            .decode(Err(cancellation(Some((0..count).map(reason).collect()))))
            .unwrap_err()
        else {
            panic!("expected cancellation")
        };
        assert_eq!(result.completeness, ReasonCompleteness::Complete);
        let reasons: Vec<Failure<Order>> = result.reasons;
        assert_eq!(reasons.len(), count as usize);
        for (id, failure) in (0..count).zip(reasons) {
            assert_order(failure, id);
        }
    }
}

#[test]
fn cap_counts_all_leaves_across_tuple_and_vector_boundaries() {
    for end in [100, 101] {
        let result = transaction((actions(0..49), (delete(49), actions(50..end)))).build();
        if end == 101 {
            assert!(matches!(
                result,
                Err(BuildError::RequestLimit {
                    max: 100,
                    actual: 101
                })
            ));
            continue;
        }
        let request = result.unwrap();
        assert_eq!(request.input().transact_items().len(), 100);
        let TxError::Cancelled(result) = request
            .decode(Err(cancellation(Some((0..100).map(reason).collect()))))
            .unwrap_err()
        else {
            panic!("expected cancellation")
        };
        assert_eq!(result.completeness, ReasonCompleteness::Complete);
        let (before, (middle, after)) = result.reasons;
        assert_eq!(before.len(), 49);
        assert_eq!(after.len(), 50);
        for (id, failure) in (0..100).zip(before.into_iter().chain([middle]).chain(after)) {
            assert_order(failure, id);
        }
    }
}

#[test]
fn nested_tuples_extend_beyond_twelve_actions_and_retain_order() {
    let request = transaction((
        (
            delete(0),
            delete(1),
            delete(2),
            delete(3),
            delete(4),
            delete(5),
            delete(6),
            delete(7),
            delete(8),
            delete(9),
            delete(10),
            delete(11),
        ),
        (delete(12),),
    ))
    .build()
    .unwrap();
    assert_eq!(request.input().transact_items().len(), 13);
    for (id, item) in request.input().transact_items().iter().enumerate() {
        assert_eq!(item.delete().unwrap().key["id"], Av::S(id.to_string()));
    }
    let TxError::Cancelled(result) = request
        .decode(Err(cancellation(Some((0..13).map(reason).collect()))))
        .unwrap_err()
    else {
        panic!("expected cancellation")
    };
    assert_eq!(result.completeness, ReasonCompleteness::Complete);
    let ((a, b, c, d, e, f, g, h, i, j, k, l), (m,)) = result.reasons;
    for (id, failure) in (0..13).zip([a, b, c, d, e, f, g, h, i, j, k, l, m]) {
        assert_order(failure, id);
    }
}

#[test]
fn duplicate_targets_across_composition_boundaries_are_rejected() {
    assert!(matches!(
        transaction((actions(0..2), (delete(2), (delete(1),)))).build(),
        Err(BuildError::OperationConstraint {
            code: "duplicate_transaction_target"
        })
    ));
}

#[test]
fn composition_record_types_are_checked_at_compile_time() {
    trybuild::TestCases::new().compile_fail("tests/ui/transaction_composition/*.rs");
}
