#![cfg(feature = "derive")]

use tabulary::{
    BatchAction, BatchBudget, BilledSize, BuildError, Condition, ReadConsistency, Table, Update,
    transaction,
};

#[derive(tabulary::Record)]
#[tabulary(rename_all = "snake_case")]
struct Order {
    #[tabulary(partition_key)]
    id: String,
    payload: Vec<u8>,
    note: String,
}
fn record(id: usize, bytes: usize) -> Order {
    Order {
        id: id.to_string(),
        payload: vec![0; bytes],
        note: String::new(),
    }
}
fn table() -> Table<Order> {
    Table::new("orders").unwrap()
}
fn key(id: usize) -> OrderKey {
    OrderKey::new(id.to_string()).unwrap()
}
fn predicates(count: usize) -> Condition<Order> {
    if count == 1 {
        return OrderAttrs::NOTE.eq(String::new());
    }
    predicates(count / 2).and(predicates(count - count / 2))
}
fn operator_limit<T>(result: Result<T, BuildError>) {
    assert!(matches!(
        result,
        Err(BuildError::RequestLimit {
            max: 300,
            actual: 301
        })
    ));
}

#[test]
fn condition_operator_limit_covers_every_owning_operation() {
    let t = table();
    let row = record(0, 0);
    // 150 comparisons + 149 ANDs + NOT: equality at 300 succeeds.
    assert!(t.put(&row).condition(predicates(150).not()).build().is_ok());
    operator_limit(t.put(&row).condition(predicates(151)).build());
    operator_limit(t.delete(key(0)).condition(predicates(151)).build());
    operator_limit(
        t.update(key(0))
            .changes(Update::new().and(OrderAttrs::NOTE.set("x".into())))
            .condition(predicates(151))
            .build(),
    );
    operator_limit(t.check(key(0), predicates(151)).into_tx());
    operator_limit(
        t.query(
            Order::primary().partition("0".into()),
            ReadConsistency::Strong,
        )
        .filter(predicates(151))
        .build(),
    );
    operator_limit(
        t.scan(ReadConsistency::Strong)
            .filter(predicates(151))
            .build(),
    );
}

#[test]
fn substitution_limit_uses_shared_maps_and_allows_long_values() {
    let row = record(0, 0);
    // The 255-byte limit is on the placeholder, not its bound contents.
    assert!(
        table()
            .put(&row)
            .condition(OrderAttrs::NOTE.eq("x".repeat(256)))
            .build()
            .is_ok()
    );
    // Each fragment fits separately; their shared value map exceeds 2 MiB.
    let large = || "x".repeat(1_100_000);
    assert!(
        table()
            .delete(key(0))
            .condition(OrderAttrs::NOTE.eq(large()))
            .build()
            .is_ok()
    );
    assert!(matches!(
        table()
            .update(key(0))
            .changes(Update::new().and(OrderAttrs::NOTE.set(large())))
            .condition(OrderAttrs::NOTE.eq(large()))
            .build(),
        Err(BuildError::RequestLimit { max: 2_097_152, .. })
    ));
}

#[test]
fn binary_item_limit_uses_raw_bytes_for_put_batch_and_transaction() {
    // Physical names plus the one-byte id consume fourteen bytes.
    for extra in [0, 1] {
        let row = record(0, 400 * 1024 - 14 + extra);
        assert_eq!(
            BilledSize::of_item(&row.to_item().unwrap())
                .unwrap()
                .conservative_item_bytes(),
            409_600 + extra
        );
        let put = table().put(&row).build();
        let batch = BatchAction::put(&row);
        if extra == 0 {
            assert!(transaction(put.unwrap().into_tx().unwrap()).build().is_ok());
            assert!(
                table()
                    .batch_write(vec![batch.unwrap()], BatchBudget::new(1).unwrap())
                    .is_ok()
            );
        } else {
            assert!(matches!(
                put,
                Err(BuildError::RequestLimit {
                    max: 409_600,
                    actual: 409_601
                })
            ));
            assert!(matches!(
                batch,
                Err(BuildError::RequestLimit {
                    max: 409_600,
                    actual: 409_601
                })
            ));
        }
    }
}

#[test]
fn batch_caps_are_per_call_and_do_not_predict_read_response_size() {
    for (count, fits) in [(100, true), (101, false)] {
        let result = table().batch_get(
            (0..count).map(key).collect(),
            ReadConsistency::Strong,
            BatchBudget::new(1).unwrap(),
        );
        assert_eq!(result.is_ok(), fits);
        if !fits {
            assert!(matches!(
                result,
                Err(BuildError::RequestLimit {
                    max: 100,
                    actual: 101
                })
            ));
        }
    }
    for (count, fits) in [(25, true), (26, false)] {
        let actions = (0..count)
            .map(|i| BatchAction::<Order>::delete(key(i)).unwrap())
            .collect();
        let result = table().batch_write(actions, BatchBudget::new(1).unwrap());
        assert_eq!(result.is_ok(), fits);
        if !fits {
            assert!(matches!(
                result,
                Err(BuildError::RequestLimit {
                    max: 25,
                    actual: 26
                })
            ));
        }
    }
}

#[test]
fn transaction_reports_known_unknown_and_wire_sizes_separately() {
    let t = table();
    let row = record(0, 390 * 1024);
    let known = BilledSize::of_item(&row.to_item().unwrap())
        .unwrap()
        .conservative_item_bytes();
    let request = transaction((
        t.put(&row).into_tx().unwrap(),
        t.delete(key(1)).into_tx().unwrap(),
        t.update(key(2))
            .changes(Update::new().and(OrderAttrs::NOTE.set("x".repeat(410_000))))
            .into_tx()
            .unwrap(),
        t.check(key(3), OrderAttrs::NOTE.exists())
            .into_tx()
            .unwrap(),
    ))
    .build()
    .unwrap();
    let size = request.size();
    assert_eq!(size.known_put_item_bytes, known);
    assert_eq!(size.unknown_item_count, 3);
    assert!(size.conservative_wire_bytes > 390 * 1024 * 4 / 3 + 410_000);
    let deleted = transaction(t.delete(key(0)).into_tx().unwrap())
        .build()
        .unwrap()
        .size();
    assert_eq!(deleted.known_put_item_bytes, 0);
    assert_eq!(deleted.unknown_item_count, 1);

    // Base64 pushes JSON beyond 4 MiB while the known item aggregate still fits.
    let puts = |count| {
        (0..count)
            .map(|id| t.put(&record(id, 390 * 1024)).into_tx().unwrap())
            .collect::<Vec<_>>()
    };
    let all_puts = transaction(puts(8)).build().unwrap().size();
    assert_eq!(all_puts.unknown_item_count, 0);
    assert!(all_puts.known_put_item_bytes < 4 * 1024 * 1024);
    assert!(all_puts.conservative_wire_bytes > 4 * 1024 * 1024);
    assert!(matches!(
        transaction(puts(11)).build(),
        Err(BuildError::RequestLimit { max: 4_194_304, .. })
    ));
}

#[test]
fn long_expression_is_rejected_even_with_few_operators() {
    let leaf = || OrderAttrs::NOTE.in_values(vec![String::new(); 100]);
    let condition = leaf()
        .and(leaf())
        .and(leaf())
        .and(leaf())
        .and(leaf())
        .and(leaf())
        .and(leaf());
    assert!(matches!(
        table().delete(key(0)).condition(condition).build(),
        Err(BuildError::RequestLimit { max: 4096, .. })
    ));
}
