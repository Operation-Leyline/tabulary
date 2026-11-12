//! Milestone 48: endpoint evidence stays separate from deterministic SDK fixtures.
use super::support;
use aws_sdk_dynamodb::Client;
use tabulary::{
    BatchAction, BatchBudget, BatchStateError, BuildError, Document, Failure, FailureKind, OldItem,
    OldItemMode, PreparedTransaction, ReadConsistency, ReasonCompleteness, Record, RecordKey,
    Table, TxAction, TxError, Update, transaction,
};

#[path = "transactions_batches_fixtures.rs"]
mod fixtures;

#[derive(Debug, PartialEq, tabulary::Record)]
#[tabulary(rename_all = "snake_case")]
struct Order {
    #[tabulary(partition_key)]
    id: String,
    quantity: u64,
}
#[derive(Debug, PartialEq, tabulary::Record)]
#[tabulary(rename_all = "snake_case")]
struct Invoice {
    #[tabulary(partition_key)]
    id: String,
    amount: u64,
}
#[derive(Debug, PartialEq, tabulary::Record)]
#[tabulary(rename_all = "snake_case")]
struct Session {
    #[tabulary(partition_key)]
    id: String,
    usable: bool,
}

fn order(id: &str, quantity: u64) -> Order {
    Order {
        id: id.into(),
        quantity,
    }
}
fn invoice(amount: u64) -> Invoice {
    Invoice {
        id: "invoice".into(),
        amount,
    }
}
fn session(usable: bool) -> Session {
    Session {
        id: "session".into(),
        usable,
    }
}
fn key(id: &str) -> OrderKey {
    OrderKey::new(id.into()).unwrap()
}
async fn put<R: Record + Document>(client: &Client, table: &Table<R>, value: &R) {
    let request = table.put(value).build().unwrap();
    request
        .decode(request.apply(client.put_item()).send().await)
        .unwrap();
}
async fn read<R: Record + Document, K: RecordKey<R>>(
    client: &Client,
    table: &Table<R>,
    key: K,
) -> Option<R> {
    let request = table.get(key, ReadConsistency::Strong).build().unwrap();
    request
        .decode(request.apply(client.get_item()).send().await)
        .unwrap()
        .item
}
fn old<R: std::fmt::Debug + PartialEq>(failure: Failure<R>, expected: R) {
    assert_eq!(failure.kind, FailureKind::ConditionFailed);
    let OldItem::Decoded(value) = failure.old else {
        panic!("ALL_OLD must decode: {:?}", failure.old)
    };
    assert_eq!(value, expected);
}

type Actions = (TxAction<Order>, TxAction<Invoice>, TxAction<Session>);
fn writes(name: &str, fail: Option<usize>) -> PreparedTransaction<Actions> {
    transaction((
        Table::<Order>::new(name)
            .unwrap()
            .put(&order("order", 8))
            .condition(OrderAttrs::QUANTITY.eq(if fail == Some(0) { 0 } else { 7 }))
            .on_condition_failure(OldItemMode::All)
            .into_tx()
            .unwrap(),
        Table::<Invoice>::new(name)
            .unwrap()
            .update(InvoiceKey::new("invoice".into()).unwrap())
            .changes(Update::new().and(InvoiceAttrs::AMOUNT.set(12)))
            .condition(InvoiceAttrs::AMOUNT.eq(if fail == Some(1) { 0 } else { 11 }))
            .on_condition_failure(OldItemMode::All)
            .into_tx()
            .unwrap(),
        Table::<Session>::new(name)
            .unwrap()
            .delete(SessionKey::new("session".into()).unwrap())
            .condition(SessionAttrs::USABLE.eq(fail != Some(2)))
            .on_condition_failure(OldItemMode::All)
            .into_tx()
            .unwrap(),
    ))
    .build()
    .unwrap()
}

#[tokio::test]
#[ignore = "requires explicit TABULARY_DYNAMODB_ENDPOINT, AWS region and credentials"]
async fn three_item_transaction_commits_or_rolls_back_every_action_with_typed_old_items() {
    support::with_table::<Order, _, _>(|client, name| async move {
        let orders = Table::<Order>::new(&name).unwrap();
        let invoices = Table::<Invoice>::new(&name).unwrap();
        let sessions = Table::<Session>::new(&name).unwrap();
        let original_invoice = invoice(11);
        let original_session = session(true);
        put(&client, &orders, &order("order", 7)).await;
        put(&client, &invoices, &original_invoice).await;
        put(&client, &sessions, &original_session).await;
        // Exactly one condition fails per submission; no condition-priority assumption.
        for failed in 0..3 {
            let request = writes(&name, Some(failed));
            assert_eq!(request.input().transact_items().len(), 3);
            let TxError::Cancelled(cancelled) = request
                .decode(request.apply(client.transact_write_items()).send().await)
                .unwrap_err()
            else {
                panic!("expected modeled cancellation")
            };
            assert_eq!(cancelled.completeness, ReasonCompleteness::Complete);
            let (a, b, c) = cancelled.reasons;
            for (position, kind) in [&a.kind, &b.kind, &c.kind].into_iter().enumerate() {
                assert_eq!(
                    *kind,
                    if position == failed {
                        FailureKind::ConditionFailed
                    } else {
                        FailureKind::NoFailureReported
                    }
                );
            }
            match failed {
                0 => old(a, order("order", 7)),
                1 => old(b, invoice(11)),
                2 => old(c, session(true)),
                _ => unreachable!(),
            }
            assert_eq!(
                read(&client, &orders, key("order")).await,
                Some(order("order", 7))
            );
            assert_eq!(
                read(&client, &invoices, original_invoice.key().unwrap()).await,
                Some(invoice(11))
            );
            assert_eq!(
                read(&client, &sessions, original_session.key().unwrap()).await,
                Some(session(true))
            );
        }
        let request = writes(&name, None);
        request
            .decode(request.apply(client.transact_write_items()).send().await)
            .unwrap();
        assert_eq!(
            read(&client, &orders, key("order")).await,
            Some(order("order", 8))
        );
        assert_eq!(
            read(&client, &invoices, original_invoice.key().unwrap()).await,
            Some(invoice(12))
        );
        assert_eq!(
            read(&client, &sessions, original_session.key().unwrap()).await,
            None
        );
    })
    .await;
}

#[tokio::test]
#[ignore = "requires explicit TABULARY_DYNAMODB_ENDPOINT, AWS region and credentials"]
async fn nested_transaction_condition_check_guards_both_writes() {
    support::with_table::<Order, _, _>(|client, name| async move {
        let orders = Table::<Order>::new(&name).unwrap();
        let invoices = Table::<Invoice>::new(&name).unwrap();
        let sessions = Table::<Session>::new(&name).unwrap();
        let session_key = SessionKey::new("session".into()).unwrap();
        put(&client, &orders, &order("order", 7)).await;
        let original_invoice = invoice(11);
        put(&client, &invoices, &original_invoice).await;
        put(&client, &sessions, &session(false)).await;
        let request = transaction((
            (
                orders.put(&order("order", 8)).into_tx().unwrap(),
                vec![
                    invoices
                        .update(original_invoice.key().unwrap())
                        .changes(Update::new().and(InvoiceAttrs::AMOUNT.set(12)))
                        .into_tx()
                        .unwrap(),
                ],
            ),
            sessions
                .check(session_key.clone(), SessionAttrs::USABLE.eq(true))
                .on_condition_failure(OldItemMode::All)
                .into_tx()
                .unwrap(),
        ))
        .build()
        .unwrap();
        let TxError::Cancelled(cancelled) = request
            .decode(request.apply(client.transact_write_items()).send().await)
            .unwrap_err()
        else {
            panic!("expected check cancellation")
        };
        assert_eq!(cancelled.completeness, ReasonCompleteness::Complete);
        assert_eq!(cancelled.reasons.0.0.kind, FailureKind::NoFailureReported);
        assert_eq!(cancelled.reasons.0.1.len(), 1);
        assert_eq!(
            cancelled.reasons.0.1[0].kind,
            FailureKind::NoFailureReported
        );
        old(cancelled.reasons.1, session(false));
        assert_eq!(
            read(&client, &orders, key("order")).await,
            Some(order("order", 7))
        );
        assert_eq!(
            read(&client, &invoices, original_invoice.key().unwrap()).await,
            Some(invoice(11))
        );
        assert_eq!(
            read(&client, &sessions, session_key.clone()).await,
            Some(session(false))
        );
        put(&client, &sessions, &session(true)).await;
        request
            .decode(request.apply(client.transact_write_items()).send().await)
            .unwrap();
        assert_eq!(
            read(&client, &orders, key("order")).await,
            Some(order("order", 8))
        );
        assert_eq!(
            read(&client, &invoices, original_invoice.key().unwrap()).await,
            Some(invoice(12))
        );
        assert_eq!(
            read(&client, &sessions, session_key).await,
            Some(session(true))
        );
    })
    .await;
}

#[test]
fn same_target_write_and_check_are_refused_before_any_send() {
    let table = Table::<Order>::new("orders").unwrap();
    let request = transaction((
        vec![table.put(&order("order", 7)).into_tx().unwrap()],
        (table
            .check(key("order"), OrderAttrs::QUANTITY.eq(7))
            .into_tx()
            .unwrap(),),
    ))
    .build();
    assert!(matches!(
        request,
        Err(BuildError::OperationConstraint {
            code: "duplicate_transaction_target"
        })
    ));
}

#[tokio::test]
#[ignore = "requires explicit TABULARY_DYNAMODB_ENDPOINT, AWS region and credentials"]
async fn batch_put_delete_and_get_keep_original_input_order() {
    support::with_table::<Order, _, _>(|client, name| async move {
        let table = Table::<Order>::new(&name).unwrap();
        put(&client, &table, &order("b", 2)).await;
        let actions = vec![
            BatchAction::put(&order("c", 3)).unwrap(),
            BatchAction::delete(key("b")).unwrap(),
            BatchAction::put(&order("a", 1)).unwrap(),
        ];
        let expected: Vec<_> = actions
            .iter()
            .map(|a| (a.key().clone(), a.item().cloned()))
            .collect();
        let mut batch = table
            .batch_write(actions, BatchBudget::new(1).unwrap())
            .unwrap();
        let request = batch.next_request().unwrap().unwrap();
        assert_eq!(request.positions(), &[0, 1, 2]);
        assert_eq!(
            batch.next_request().unwrap_err(),
            BatchStateError::OutstandingRound
        );
        let raw = request.apply(client.batch_write_item()).send().await;
        batch.accept(request.decode(raw)).unwrap();
        assert!(batch.next_request().unwrap().is_none());
        let result = batch.finish().unwrap();
        assert_eq!(
            result
                .actions
                .iter()
                .map(|a| (a.key().clone(), a.item().cloned()))
                .collect::<Vec<_>>(),
            expected
        );
        let mut batch = table
            .batch_get(
                vec![key("c"), key("b"), key("a"), key("missing")],
                ReadConsistency::Strong,
                BatchBudget::new(1).unwrap(),
            )
            .unwrap();
        let request = batch.next_request().unwrap().unwrap();
        assert_eq!(
            batch.next_request().unwrap_err(),
            BatchStateError::OutstandingRound
        );
        batch
            .accept(request.decode(request.apply(client.batch_get_item()).send().await))
            .unwrap();
        assert!(batch.next_request().unwrap().is_none());
        assert_eq!(
            batch.finish().unwrap().records,
            vec![Some(order("c", 3)), None, Some(order("a", 1)), None]
        );
    })
    .await;
}
