#![cfg(feature = "derive")]

use aws_sdk_dynamodb::types::AttributeValue as Av;
use tabulary::BatchWriteState::{Indeterminate, Pending, Processed};
use tabulary::{
    BatchAction, BatchBudget, BatchStateError, BatchWriteProgress, BuildError, Item, Table,
};

// No Clone, Debug or Eq bounds on the record are needed.
#[derive(tabulary::Record)]
#[tabulary(rename_all = "snake_case")]
struct Order {
    #[tabulary(partition_key)]
    id: String,
    #[tabulary(sort_key)]
    line: tabulary::Decimal,
    quantity: u64,
}
fn order(id: &str, line: &str) -> Order {
    Order {
        id: id.into(),
        line: line.parse().unwrap(),
        quantity: 7,
    }
}
fn delete(id: &str, line: &str) -> BatchAction<Order> {
    BatchAction::delete(OrderKey::new(id.into(), line.parse().unwrap()).unwrap()).unwrap()
}
fn batch(count: usize, calls: u32) -> BatchWriteProgress<Order> {
    BatchWriteProgress::new(
        (0..count)
            .map(|i| delete("order", &i.to_string()))
            .collect(),
        BatchBudget::new(calls).unwrap(),
    )
    .unwrap()
}

#[test]
fn put_and_delete_preserve_prepared_payloads_and_full_target_identity() {
    let table = Table::<Order>::new("orders").unwrap();
    let record = order("order", "1.00");
    let put = BatchAction::put(&record).unwrap();
    assert_eq!(
        put.item(),
        table.put(&record).build().unwrap().input().item.as_ref()
    );
    let key = OrderKey::new("order".into(), "1".parse().unwrap()).unwrap();
    let prepared = table.delete(key).build().unwrap();
    let remove = delete("order", "1");
    assert_eq!(remove.key(), prepared.input().key.as_ref().unwrap());
    assert_eq!(put.key(), remove.key());
    assert!(remove.item().is_none());
    let budget = BatchBudget::new(1).unwrap();
    for actions in [
        vec![put, remove],
        vec![delete("order", "1"), delete("order", "1e0")],
        vec![
            BatchAction::put(&record).unwrap(),
            BatchAction::put(&record).unwrap(),
        ],
    ] {
        assert!(matches!(
            BatchWriteProgress::new(actions, budget),
            Err(BuildError::OperationConstraint {
                code: "duplicate_batch_target"
            })
        ));
    }
    let mut progress = BatchWriteProgress::new(
        vec![
            delete("order", "1"),
            delete("order", "2"),
            delete("other", "1"),
        ],
        budget,
    )
    .unwrap();
    assert_eq!(
        progress.next_round().unwrap().unwrap().positions(),
        &[0, 1, 2]
    );
}

#[test]
fn empty_and_over_cap_inputs_are_refused_without_splitting() {
    let budget = BatchBudget::new(1).unwrap();
    assert!(matches!(
        BatchWriteProgress::<Order>::new(vec![], budget),
        Err(BuildError::EmptyOperation)
    ));
    assert!(matches!(
        BatchWriteProgress::new(
            (0..26).map(|i| delete("order", &i.to_string())).collect(),
            budget
        ),
        Err(BuildError::RequestLimit {
            max: 25,
            actual: 26
        })
    ));
    assert_eq!(
        batch(25, 1)
            .next_round()
            .unwrap()
            .unwrap()
            .positions()
            .len(),
        25
    );
}

#[test]
fn only_unprocessed_actions_are_issued_in_original_order_with_fixed_budget() {
    let mut progress = batch(3, 2);
    let first = progress.next_round().unwrap().unwrap();
    progress.accept_round(&first, vec![2, 0]).unwrap();
    assert_eq!(progress.entries()[1].state, Processed);
    let second = progress.next_round().unwrap().unwrap();
    assert_eq!(second.positions(), &[0, 2]);
    assert_eq!(progress.calls_issued(), 2);
    progress.accept_round(&second, vec![2]).unwrap();
    assert!(progress.next_round().unwrap().is_none());
    assert!(progress.next_round().unwrap().is_none());
    let error = progress.finish().unwrap_err();
    assert_eq!(error.kind, BatchStateError::BudgetExhausted);
    assert_eq!(error.partial.calls_issued, 2);
    assert_eq!(
        error
            .partial
            .entries
            .iter()
            .map(|e| e.state)
            .collect::<Vec<_>>(),
        vec![Processed, Processed, Pending]
    );
    assert_eq!(
        error.partial.entries[2].action.key()["line"],
        Av::N("2".into())
    );
    let mut progress = batch(2, 1);
    let chunk = progress.next_round().unwrap().unwrap();
    progress.accept_round(&chunk, vec![]).unwrap();
    assert!(progress.next_round().unwrap().is_none());
    let complete = progress.finish().unwrap();
    assert_eq!(complete.actions[0].key()["line"], Av::N("0".into()));
    assert_eq!(complete.actions[1].key()["line"], Av::N("1".into()));
    assert_eq!(
        batch(1, 2).finish().unwrap_err().kind,
        BatchStateError::PendingWork
    );
}

#[test]
fn outstanding_foreign_cloned_and_replayed_rounds_follow_batch_get_rules() {
    let mut left = batch(2, 2);
    let mut right = batch(2, 2);
    let first = left.next_round().unwrap().unwrap();
    let foreign = right.next_round().unwrap().unwrap();
    assert_eq!(
        left.next_round().unwrap_err(),
        BatchStateError::OutstandingRound
    );
    assert_eq!(
        left.accept_round(&foreign, vec![]).unwrap_err(),
        BatchStateError::WrongRound
    );
    assert_eq!(
        left.fail_round(&foreign).unwrap_err(),
        BatchStateError::WrongRound
    );
    left.accept_round(&first.clone(), vec![1]).unwrap();
    assert_eq!(
        left.accept_round(&first, vec![]).unwrap_err(),
        BatchStateError::WrongRound
    );
    let second = left.next_round().unwrap().unwrap();
    assert_eq!(
        left.accept_round(&first, vec![]).unwrap_err(),
        BatchStateError::WrongRound
    );
    assert_eq!(
        left.fail_round(&first).unwrap_err(),
        BatchStateError::WrongRound
    );
    left.accept_round(&second, vec![]).unwrap();
    assert!(left.finish().is_ok());
    assert_eq!(
        right.finish().unwrap_err().kind,
        BatchStateError::OutstandingRound
    );
}

#[test]
fn invalid_pending_subsets_do_not_mutate_or_settle_the_round() {
    for pending in [vec![0, 0], vec![2], vec![usize::MAX]] {
        let mut progress = batch(2, 2);
        let first = progress.next_round().unwrap().unwrap();
        assert_eq!(
            progress.accept_round(&first, pending).unwrap_err(),
            BatchStateError::InvalidResponse
        );
        assert!(progress.entries().iter().all(|e| e.state == Pending));
        assert_eq!(
            progress.next_round().unwrap_err(),
            BatchStateError::OutstandingRound
        );
        progress.accept_round(&first, vec![1]).unwrap();
        let second = progress.next_round().unwrap().unwrap();
        assert_eq!(
            progress.accept_round(&second, vec![0]).unwrap_err(),
            BatchStateError::InvalidResponse
        );
        assert_eq!(progress.entries()[0].state, Processed);
        progress.accept_round(&second, vec![]).unwrap();
        assert!(progress.finish().is_ok());
    }
}

#[test]
fn ambiguous_failures_freeze_the_batch_and_preserve_successes() {
    let mut progress = batch(3, 3);
    let first = progress.next_round().unwrap().unwrap();
    progress.accept_round(&first, vec![0, 2]).unwrap();
    let second = progress.next_round().unwrap().unwrap();
    progress.fail_round(&second).unwrap();
    assert_eq!(
        progress.next_round().unwrap_err(),
        BatchStateError::Indeterminate
    );
    assert_eq!(
        progress.accept_round(&second, vec![]).unwrap_err(),
        BatchStateError::WrongRound
    );
    assert_eq!(
        progress.fail_round(&second).unwrap_err(),
        BatchStateError::WrongRound
    );
    let error = progress.finish().unwrap_err();
    assert_eq!(error.kind, BatchStateError::Indeterminate);
    assert_eq!(error.partial.calls_issued, 2);
    assert_eq!(
        error
            .partial
            .entries
            .iter()
            .map(|e| e.state)
            .collect::<Vec<_>>(),
        vec![Indeterminate, Processed, Indeterminate]
    );
    let mut progress = batch(1, 1);
    let chunk = progress.next_round().unwrap().unwrap();
    assert!(!format!("{progress:?} {chunk:?}").contains("order"));
    assert_eq!(progress.into_partial().entries[0].state, Indeterminate);
    assert_eq!(batch(1, 1).into_partial().entries[0].state, Pending);
    assert!(!format!("{error:?} {error} {:?}", error.partial.entries).contains("order"));
}

#[derive(tabulary::Record)]
#[tabulary(rename_all = "snake_case")]
struct Invoice {
    #[tabulary(partition_key)]
    id: String,
    #[tabulary(version)]
    revision: u64,
}
#[test]
fn versioned_puts_and_deletes_are_refused() {
    assert!(matches!(
        BatchAction::put(&Invoice {
            id: "invoice".into(),
            revision: 1
        }),
        Err(BuildError::VersionPolicy)
    ));
    assert!(matches!(
        BatchAction::<Invoice>::delete(InvoiceKey::new("invoice".into()).unwrap()),
        Err(BuildError::VersionPolicy)
    ));
}

struct RawKey(Item);
impl tabulary::RecordKey<Order> for RawKey {
    fn encode_key(&self) -> Result<Item, tabulary::EncodeError> {
        Ok(self.0.clone())
    }
    fn decode_key(item: &Item) -> Result<Self, tabulary::DecodeError> {
        Ok(Self(item.clone()))
    }
}
#[test]
fn malformed_manual_keys_have_typed_path_errors_and_extra_fields_are_refused() {
    let valid = delete("order", "1");
    for (name, value) in [
        ("id", None),
        ("id", Some(Av::S(String::new()))),
        ("line", Some(Av::N("bad".into()))),
        ("line", Some(Av::S("1".into()))),
    ] {
        let mut raw = valid.key().clone();
        raw.remove(name);
        if let Some(value) = value {
            raw.insert(name.into(), value);
        }
        assert!(matches!(BatchAction::<Order>::delete(RawKey(raw)),
            Err(BuildError::Encode(e)) if e.record == "Order" && e.attribute() == Some(name)));
    }
    let mut raw = valid.key().clone();
    raw.insert("quantity".into(), Av::N("1".into()));
    assert!(matches!(
        BatchAction::<Order>::delete(RawKey(raw)),
        Err(BuildError::OperationConstraint {
            code: "invalid_batch_key_shape"
        })
    ));
}

#[test]
fn batch_actions_exclude_conditions_updates_and_single_write_options() {
    trybuild::TestCases::new().compile_fail("tests/ui/batch_write/*.rs");
}
