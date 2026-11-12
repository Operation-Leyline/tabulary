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

#[test]
fn query_and_segmented_scan_resume_from_bytes_with_changed_page_limits() {
    use aws_sdk_dynamodb::{
        operation::{query::QueryOutput, scan::ScanOutput},
        types::AttributeValue,
    };
    use std::num::NonZeroU32;
    use tabulary::{Cursor, Item, ScanSegment};
    let table = Table::<Order>::new("orders").unwrap();
    let key = Item::from([("id".into(), AttributeValue::S("order".into()))]);
    let query = || {
        table
            .query(Order::primary().partition("order".into()), Eventual)
            .filter(OrderAttrs::NOTE.eq("private-filter".into()))
    };
    let page = query()
        .build()
        .unwrap()
        .decode(Ok(QueryOutput::builder()
            .count(0)
            .scanned_count(1)
            .set_last_evaluated_key(Some(key.clone()))
            .build()))
        .unwrap();
    let bytes = page.next.unwrap().to_bytes().unwrap();
    let decoded = Cursor::from_bytes(&bytes).unwrap();
    let resumed = query()
        .limit(NonZeroU32::new(7).unwrap())
        .resume(decoded)
        .build()
        .unwrap();
    assert_eq!(resumed.input().exclusive_start_key.as_ref(), Some(&key));
    assert_eq!(resumed.input().limit, Some(7));
    assert!(matches!(
        table
            .query(Order::primary().partition("order".into()), Eventual)
            .resume(Cursor::from_bytes(&bytes).unwrap())
            .build(),
        Err(BuildError::CursorMismatch)
    ));

    let segment = ScanSegment::new(1, 3).unwrap();
    let page = table
        .scan(Eventual)
        .segment(segment)
        .build()
        .unwrap()
        .decode(Ok(ScanOutput::builder()
            .count(0)
            .scanned_count(1)
            .set_last_evaluated_key(Some(key.clone()))
            .build()))
        .unwrap();
    let bytes = page.next.unwrap().to_bytes().unwrap();
    let resumed = table
        .scan(Eventual)
        .segment(segment)
        .limit(NonZeroU32::new(9).unwrap())
        .resume(Cursor::from_bytes(&bytes).unwrap())
        .build()
        .unwrap();
    assert_eq!(resumed.input().exclusive_start_key.as_ref(), Some(&key));
    assert_eq!(resumed.input().limit, Some(9));
    assert!(matches!(
        table
            .scan(Eventual)
            .segment(ScanSegment::new(0, 3).unwrap())
            .resume(Cursor::from_bytes(&bytes).unwrap())
            .build(),
        Err(BuildError::CursorMismatch)
    ));
    assert!(matches!(
        table
            .scan(Eventual)
            .resume(Cursor::from_bytes(&bytes).unwrap())
            .build(),
        Err(BuildError::CursorMismatch)
    ));
}
