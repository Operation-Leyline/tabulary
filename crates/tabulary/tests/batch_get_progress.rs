#![cfg(feature = "derive")]

use tabulary::{BatchBudget, BatchGetProgress, BatchGetState, BatchStateError, BuildError};

// Intentionally neither Clone nor Debug: progress must move decoded records.
#[derive(tabulary::Record)]
#[tabulary(rename_all = "snake_case")]
struct Order {
    #[tabulary(partition_key)]
    id: String,
    quantity: u64,
}

fn batch(count: usize, calls: u32) -> BatchGetProgress<Order, OrderKey> {
    BatchGetProgress::new(
        (0..count)
            .map(|i| OrderKey::new(format!("order-{i}")).unwrap())
            .collect(),
        BatchBudget::new(calls).unwrap(),
    )
    .unwrap()
}

#[test]
fn nonzero_budget_counts_the_initial_call_and_never_replenishes() {
    assert!(matches!(
        BatchBudget::new(0),
        Err(BuildError::OperationConstraint {
            code: "zero_batch_budget"
        })
    ));
    let mut progress = batch(2, 1);
    assert_eq!(progress.calls_issued(), 0);
    let chunk = progress.next_round().unwrap().unwrap();
    assert_eq!(chunk.positions(), &[0, 1]);
    assert_eq!(progress.calls_issued(), 1);
    progress.accept_round(&chunk, vec![], vec![0, 1]).unwrap();
    assert!(progress.next_round().unwrap().is_none());
    assert!(progress.next_round().unwrap().is_none());
    let error = progress.finish().unwrap_err();
    assert_eq!(error.kind, BatchStateError::BudgetExhausted);
    assert_eq!(error.partial.calls_issued, 1);
    assert!(
        error
            .partial
            .entries
            .iter()
            .all(|entry| matches!(entry.state, BatchGetState::Pending))
    );
}

#[test]
fn exactly_one_round_is_outstanding_even_at_the_budget_boundary() {
    let mut progress = batch(1, 1);
    let chunk = progress.next_round().unwrap().unwrap();
    assert_eq!(
        progress.next_round().unwrap_err(),
        BatchStateError::OutstandingRound
    );
    assert_eq!(progress.calls_issued(), 1);
    assert_eq!(
        progress.finish().unwrap_err().kind,
        BatchStateError::OutstandingRound
    );
    assert_eq!(chunk.positions(), &[0]);
}

#[test]
fn foreign_and_replayed_chunks_cannot_settle_a_round() {
    let mut left = batch(1, 3);
    let mut right = batch(1, 3);
    let first = left.next_round().unwrap().unwrap();
    let foreign = right.next_round().unwrap().unwrap();
    assert_eq!(
        left.accept_round(&foreign, vec![], vec![]).unwrap_err(),
        BatchStateError::WrongRound
    );
    left.accept_round(&first, vec![], vec![0]).unwrap();
    assert_eq!(
        left.accept_round(&first, vec![], vec![]).unwrap_err(),
        BatchStateError::WrongRound
    );
    let second = left.next_round().unwrap().unwrap();
    assert_eq!(
        left.accept_round(&first, vec![], vec![]).unwrap_err(),
        BatchStateError::WrongRound
    );
    left.accept_round(&second, vec![], vec![]).unwrap();
    assert!(left.next_round().unwrap().is_none());
    assert!(left.finish().unwrap().records[0].is_none());
}

#[test]
fn records_missing_and_pending_keep_original_order_across_rounds() {
    let mut progress = batch(4, 3);
    let first = progress.next_round().unwrap().unwrap();
    assert_eq!(first.keys()[0]["id"].as_s().unwrap(), "order-0");
    progress
        .accept_round(
            &first,
            vec![
                (
                    3,
                    Order {
                        id: "order-3".into(),
                        quantity: 30,
                    },
                ),
                (
                    0,
                    Order {
                        id: "order-0".into(),
                        quantity: 10,
                    },
                ),
            ],
            vec![2],
        )
        .unwrap();
    assert!(matches!(
        progress.entries()[0].state,
        BatchGetState::Found(_)
    ));
    assert!(matches!(
        progress.entries()[1].state,
        BatchGetState::Missing
    ));
    assert!(matches!(
        progress.entries()[2].state,
        BatchGetState::Pending
    ));
    let second = progress.next_round().unwrap().unwrap();
    assert_eq!(second.positions(), &[2]);
    assert_eq!(second.keys().len(), 1);
    progress
        .accept_round(
            &second,
            vec![(
                2,
                Order {
                    id: "order-2".into(),
                    quantity: 20,
                },
            )],
            vec![],
        )
        .unwrap();
    assert!(progress.next_round().unwrap().is_none());
    let records = progress.finish().unwrap().records;
    assert_eq!(
        records
            .into_iter()
            .map(|item| item.map(|item| item.quantity))
            .collect::<Vec<_>>(),
        vec![Some(10), None, Some(20), Some(30)]
    );
}

#[test]
fn invalid_responses_are_rejected_atomically() {
    for (found, pending) in [
        (vec![0, 0], vec![]),
        (vec![0], vec![0]),
        (vec![], vec![0, 0]),
        (vec![2], vec![]),
        (vec![], vec![2]),
    ] {
        let mut progress = batch(2, 2);
        let chunk = progress.next_round().unwrap().unwrap();
        let found = found
            .into_iter()
            .map(|position| {
                (
                    position,
                    Order {
                        id: "value".into(),
                        quantity: 1,
                    },
                )
            })
            .collect();
        assert_eq!(
            progress.accept_round(&chunk, found, pending).unwrap_err(),
            BatchStateError::InvalidResponse
        );
        assert!(
            progress
                .entries()
                .iter()
                .all(|entry| matches!(entry.state, BatchGetState::Pending))
        );
        progress.accept_round(&chunk, vec![], vec![1]).unwrap();
        let next = progress.next_round().unwrap().unwrap();
        assert_eq!(
            progress.accept_round(&next, vec![], vec![0]).unwrap_err(),
            BatchStateError::InvalidResponse
        );
        progress.accept_round(&next, vec![], vec![]).unwrap();
        assert!(progress.finish().is_ok());
    }
}

#[test]
fn empty_duplicate_and_over_cap_inputs_are_not_chunked() {
    assert!(matches!(
        BatchGetProgress::<Order, OrderKey>::new(vec![], BatchBudget::new(1).unwrap()),
        Err(BuildError::EmptyOperation)
    ));
    let keys = vec![
        OrderKey::new("same".into()).unwrap(),
        OrderKey::new("same".into()).unwrap(),
    ];
    assert!(matches!(
        BatchGetProgress::<Order, _>::new(keys, BatchBudget::new(1).unwrap()),
        Err(BuildError::OperationConstraint {
            code: "duplicate_batch_key"
        })
    ));
    let keys = (0..101)
        .map(|i| OrderKey::new(format!("order-{i}")).unwrap())
        .collect();
    assert!(matches!(
        BatchGetProgress::<Order, _>::new(keys, BatchBudget::new(1).unwrap()),
        Err(BuildError::RequestLimit {
            max: 100,
            actual: 101
        })
    ));
    let mut progress = batch(100, 1);
    assert_eq!(
        progress.next_round().unwrap().unwrap().positions().len(),
        100
    );
}

#[derive(tabulary::Record)]
#[tabulary(rename_all = "snake_case")]
struct Invoice {
    #[tabulary(partition_key)]
    number: tabulary::Decimal,
    #[tabulary(sort_key)]
    suffix: Vec<u8>,
}
// Manual keys need neither Clone, Debug nor Eq, just as in GetItem.
struct RawKey(tabulary::Item);
impl tabulary::RecordKey<Invoice> for RawKey {
    fn encode_key(&self) -> Result<tabulary::Item, tabulary::EncodeError> {
        Ok(self.0.clone())
    }
    fn decode_key(item: &tabulary::Item) -> Result<Self, tabulary::DecodeError> {
        Ok(Self(item.clone()))
    }
}
fn raw(number: &str, suffix: &[u8]) -> RawKey {
    use aws_sdk_dynamodb::types::AttributeValue as Av;
    RawKey(
        [
            ("number".into(), Av::N(number.into())),
            ("suffix".into(), Av::B(suffix.to_vec().into())),
        ]
        .into(),
    )
}

#[test]
fn identity_uses_both_physical_keys_and_normalizes_numeric_spellings() {
    let budget = BatchBudget::new(u32::MAX).unwrap();
    assert!(matches!(
        BatchGetProgress::<Invoice, _>::new(vec![raw("1", b"a"), raw("1.00e0", b"a")], budget),
        Err(BuildError::OperationConstraint {
            code: "duplicate_batch_key"
        })
    ));
    let mut progress = BatchGetProgress::<Invoice, _>::new(
        vec![raw("1.00", b"a"), raw("1", b"b"), raw("2", b"a")],
        budget,
    )
    .unwrap();
    let chunk = progress.next_round().unwrap().unwrap();
    assert_eq!(chunk.keys()[0]["number"].as_n().unwrap(), "1");
    assert_eq!(chunk.positions(), &[0, 1, 2]);
    progress
        .accept_round(&chunk.clone(), vec![], vec![])
        .unwrap();
    assert_eq!(
        progress.accept_round(&chunk, vec![], vec![]).unwrap_err(),
        BatchStateError::WrongRound
    );
    for key in [raw("malformed", b"a"), raw("1", b"")] {
        assert!(matches!(
            BatchGetProgress::<Invoice, _>::new(vec![key], budget),
            Err(BuildError::Encode(_))
        ));
    }
    assert!(
        matches!(BatchGetProgress::<Order, _>::new(vec![OrderKey { id: String::new() }], budget), Err(BuildError::Encode(source)) if source.record == "Order" && source.attribute() == Some("id"))
    );
}

#[test]
fn incomplete_finish_preserves_ordered_successes_keys_and_pending_without_debug_leaks() {
    assert_eq!(
        batch(1, 2).finish().unwrap_err().kind,
        BatchStateError::PendingWork
    );
    let mut progress = batch(3, 1);
    let chunk = progress.next_round().unwrap().unwrap();
    progress
        .accept_round(
            &chunk,
            vec![(
                0,
                Order {
                    id: "private-record".into(),
                    quantity: 1,
                },
            )],
            vec![2],
        )
        .unwrap();
    assert!(!format!("{progress:?} {chunk:?}").contains("order-0"));
    let error = progress.finish().unwrap_err();
    assert_eq!(error.kind, BatchStateError::BudgetExhausted);
    assert!(!format!("{error:?} {error} {:?}", error.partial.entries).contains("private-record"));
    let entries = error.partial.entries;
    assert_eq!(
        entries
            .iter()
            .map(|entry| entry.key.id.as_str())
            .collect::<Vec<_>>(),
        vec!["order-0", "order-1", "order-2"]
    );
    assert!(matches!(entries[0].state, BatchGetState::Found(_)));
    assert!(matches!(entries[1].state, BatchGetState::Missing));
    assert!(matches!(entries[2].state, BatchGetState::Pending));
}
