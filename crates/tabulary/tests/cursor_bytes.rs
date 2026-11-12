#![cfg(feature = "derive")]
use tabulary::{BuildError, ReadConsistency::Eventual, Table};

#[derive(tabulary::Record)]
#[tabulary(rename_all = "snake_case")]
struct Order {
    #[tabulary(partition_key)]
    id: String,
    note: String,
}

#[test]
fn query_and_scan_refuse_oversized_bindings_at_request_build() {
    let table = Table::<Order>::new("orders").unwrap();
    let large = || OrderAttrs::NOTE.eq("x".repeat(4 * 1024 * 1024));
    assert!(matches!(
        table
            .query(Order::primary().partition("order".into()), Eventual)
            .filter(large())
            .build(),
        Err(BuildError::CursorTooLarge)
    ));
    assert!(matches!(
        table.scan(Eventual).filter(large()).build(),
        Err(BuildError::CursorTooLarge)
    ));
}
