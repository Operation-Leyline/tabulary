#![cfg(feature = "derive")]
//! `Select::Count` Query and Scan pages decode counts and cursors, never items.

use aws_sdk_dynamodb::{
    operation::{query::QueryOutput, scan::ScanOutput},
    types::{AttributeValue as Av, ConsumedCapacity, Select},
};
use std::num::NonZeroU32;
use tabulary::{
    BuildError, Capacity, Cursor, DecodeFault, Item, QueryOrder, ReadConsistency::*, ReadError,
    ScanSegment, Table,
};

#[derive(Debug, PartialEq, tabulary::Record)]
#[tabulary(rename_all = "snake_case",
    index(ident = ByStatus, name = "by_status", kind = global, partition = status, sort = line, project = keys_only))]
struct Invoice {
    #[tabulary(partition_key)]
    account: String,
    #[tabulary(sort_key)]
    line: u32,
    status: String,
    total: u64,
}

#[derive(tabulary::Projection)]
#[tabulary(record = Invoice)]
struct Total {
    #[tabulary(attribute = InvoiceAttrs::TOTAL)]
    total: u64,
}

fn table() -> Table<Invoice> {
    Table::new("invoices").unwrap()
}
fn partition() -> tabulary::KeyCondition<Invoice, InvoicePrimary> {
    Invoice::primary().partition("acct-1".into())
}
fn key(line: u32) -> Item {
    [
        ("account".into(), Av::S("acct-1".into())),
        ("line".into(), Av::N(line.to_string())),
    ]
    .into()
}
fn capacity() -> ConsumedCapacity {
    ConsumedCapacity::builder()
        .table_name("invoices")
        .capacity_units(0.5)
        .build()
}
fn query_output(count: i32, scanned: i32, next: Option<Item>) -> QueryOutput {
    QueryOutput::builder()
        .count(count)
        .scanned_count(scanned)
        .set_last_evaluated_key(next)
        .consumed_capacity(capacity())
        .build()
}
fn count_query() -> tabulary::CountQueryBuilder<Invoice, InvoicePrimary, tabulary::SortComplete> {
    table()
        .query(partition().sort_ge(1), Strong)
        .count()
        .filter(InvoiceAttrs::STATUS.eq("open".into()))
}

#[test]
fn count_query_sets_select_count_without_projection() {
    let prepared = count_query()
        .limit(NonZeroU32::new(25).unwrap())
        .order(QueryOrder::Descending)
        .capacity(Capacity::Total)
        .build()
        .unwrap();
    let input = prepared.input();
    assert_eq!(input.select, Some(Select::Count));
    assert_eq!(input.projection_expression, None);
    assert_eq!(input.limit, Some(25));
    assert_eq!(input.scan_index_forward, Some(false));
    assert_eq!(prepared.expressions().filter(), Some("#n2 = :v2"));
    let items = table().query(partition(), Strong).build().unwrap();
    assert_eq!(items.input().select, None);
    let client = aws_sdk_dynamodb::Client::from_conf(
        aws_sdk_dynamodb::Config::builder()
            .behavior_version(aws_sdk_dynamodb::config::BehaviorVersion::latest())
            .region(aws_sdk_dynamodb::config::Region::new("us-east-1"))
            .build(),
    );
    let applied = prepared.apply(client.query().select(Select::AllAttributes));
    assert_eq!(applied.as_input().clone().build().unwrap(), *input);
}

#[test]
fn count_pages_decode_counts_and_paginate_with_bound_cursors() {
    let first = count_query().build().unwrap();
    let page = first.decode(Ok(query_output(3, 10, Some(key(9))))).unwrap();
    assert_eq!(page.count, 3);
    assert_eq!(page.scanned_count, 10);
    assert_eq!(page.capacity, Some(capacity()));
    let cursor = page.next_cursor.unwrap();
    assert_eq!(cursor.last_evaluated_key(), &key(9));
    assert!(!format!("{cursor:?}").contains("acct-1"));

    let second = count_query().resume(cursor.clone()).build().unwrap();
    assert_eq!(second.input().exclusive_start_key, Some(key(9)));
    let last = second.decode(Ok(query_output(0, 4, None))).unwrap();
    assert_eq!((last.count, last.scanned_count), (0, 4));
    assert!(last.next_cursor.is_none());
    let empty_map = second
        .decode(Ok(query_output(0, 0, Some(Item::new()))))
        .unwrap();
    assert!(empty_map.next_cursor.is_none());

    // Serialized count cursors resume counts and keep their binding.
    let bytes = cursor.to_bytes().unwrap();
    let parsed = Cursor::<Invoice, InvoicePrimary>::from_bytes(&bytes).unwrap();
    assert!(count_query().resume(parsed).build().is_ok());
    let parsed = Cursor::<Invoice, InvoicePrimary>::from_bytes(&bytes).unwrap();
    let error = table()
        .query(partition().sort_ge(1), Strong)
        .filter(InvoiceAttrs::STATUS.eq("open".into()))
        .resume(parsed)
        .build()
        .unwrap_err();
    assert_eq!(error, BuildError::CursorMismatch);
}

#[test]
fn count_and_item_cursors_never_interchange() {
    let count_cursor = count_query()
        .build()
        .unwrap()
        .decode(Ok(query_output(1, 1, Some(key(2)))))
        .unwrap()
        .next_cursor
        .unwrap();
    let items = || {
        table()
            .query(partition().sort_ge(1), Strong)
            .filter(InvoiceAttrs::STATUS.eq("open".into()))
    };
    assert_eq!(
        items().resume(count_cursor).build().unwrap_err(),
        BuildError::CursorMismatch
    );
    let item = Invoice {
        account: "acct-1".into(),
        line: 2,
        status: "open".into(),
        total: 5,
    }
    .to_item()
    .unwrap();
    let item_cursor = items()
        .build()
        .unwrap()
        .decode(Ok(QueryOutput::builder()
            .count(1)
            .scanned_count(1)
            .items(item)
            .last_evaluated_key("account", Av::S("acct-1".into()))
            .last_evaluated_key("line", Av::N("2".into()))
            .build()))
        .unwrap()
        .next
        .unwrap();
    assert_eq!(
        count_query().resume(item_cursor).build().unwrap_err(),
        BuildError::CursorMismatch
    );
}

#[test]
fn malformed_count_pages_are_typed_faults_with_raw_metadata() {
    let prepared = count_query().build().unwrap();
    let fault = |output: QueryOutput| {
        let ReadError::Decode(error) = prepared.decode(Ok(output)).unwrap_err() else {
            panic!("decode fault expected");
        };
        error
    };
    for (output, attribute, code) in [
        (query_output(-1, 0, None), "Count", "invalid_page_count"),
        (
            query_output(2, 1, None),
            "ScannedCount",
            "invalid_scanned_count",
        ),
        (
            query_output(0, -1, None),
            "ScannedCount",
            "invalid_scanned_count",
        ),
        (
            QueryOutput::builder()
                .count(1)
                .scanned_count(1)
                .items(key(1))
                .build(),
            "Items",
            "unexpected_count_items",
        ),
        (
            query_output(1, 1, Some([("other".into(), Av::S("x".into()))].into())),
            "LastEvaluatedKey",
            "invalid_page_cursor",
        ),
    ] {
        let error = fault(output);
        assert_eq!(error.position, 0);
        assert_eq!(error.source.record, "Invoice");
        assert_eq!(error.source.kind, DecodeFault::Custom { code });
        assert_eq!(error.source.attribute(), Some(attribute));
        assert!(error.page.is_some());
    }
}

#[test]
fn count_scans_bind_segments_and_decode_counts() {
    let segment = ScanSegment::new(1, 4).unwrap();
    let build = || {
        table()
            .scan(Eventual)
            .count()
            .segment(segment)
            .filter(InvoiceAttrs::TOTAL.gt(100))
            .limit(NonZeroU32::new(50).unwrap())
    };
    let prepared = build().build().unwrap();
    let input = prepared.input();
    assert_eq!(input.select, Some(Select::Count));
    assert_eq!((input.segment, input.total_segments), (Some(1), Some(4)));
    assert_eq!(input.projection_expression, None);
    let page = prepared
        .decode(Ok(ScanOutput::builder()
            .count(7)
            .scanned_count(50)
            .set_last_evaluated_key(Some(key(4)))
            .build()))
        .unwrap();
    assert_eq!((page.count, page.scanned_count), (7, 50));
    let cursor = page.next_cursor.unwrap();
    assert!(build().resume(cursor.clone()).build().is_ok());
    let other_segment = table()
        .scan(Eventual)
        .count()
        .segment(ScanSegment::new(2, 4).unwrap())
        .filter(InvoiceAttrs::TOTAL.gt(100))
        .resume(cursor.clone())
        .build();
    assert_eq!(other_segment.unwrap_err(), BuildError::CursorMismatch);
    let items = table()
        .scan(Eventual)
        .segment(segment)
        .filter(InvoiceAttrs::TOTAL.gt(100))
        .resume(cursor)
        .build();
    assert_eq!(items.unwrap_err(), BuildError::CursorMismatch);
    let ReadError::Decode(error) = prepared
        .decode(Ok(ScanOutput::builder()
            .count(1)
            .scanned_count(1)
            .items(key(1))
            .build()))
        .unwrap_err()
    else {
        panic!("decode fault expected");
    };
    assert_eq!(
        error.source.kind,
        DecodeFault::Custom {
            code: "unexpected_count_items"
        }
    );
}

#[test]
fn non_all_index_counts_need_no_selection_but_filters_stay_in_the_index() {
    let index = table().index(InvoiceIndexes::BY_STATUS);
    let key = InvoiceIndexes::BY_STATUS.partition("open".into());
    assert!(index.query(key, Eventual).build().is_err());
    let key = InvoiceIndexes::BY_STATUS.partition("open".into());
    let prepared = index.query(key, Eventual).count().build().unwrap();
    assert_eq!(prepared.input().select, Some(Select::Count));
    assert_eq!(prepared.input().index_name.as_deref(), Some("by_status"));
    let key = InvoiceIndexes::BY_STATUS.partition("open".into());
    let outside = index
        .query(key, Eventual)
        .count()
        .filter(InvoiceAttrs::TOTAL.gt(1))
        .build()
        .unwrap_err();
    assert_eq!(
        outside,
        BuildError::OperationConstraint {
            code: "projection_not_available"
        }
    );
    let scan = index
        .scan(Eventual)
        .count()
        .filter(InvoiceAttrs::ACCOUNT.eq("acct-1".into()))
        .build()
        .unwrap();
    assert_eq!(scan.input().select, Some(Select::Count));
    assert!(index.scan(Strong).count().build().is_err());
}

#[test]
fn projections_and_counts_cannot_combine() {
    let page = table()
        .query(partition(), Strong)
        .select::<Total>()
        .build()
        .unwrap()
        .decode(Ok(QueryOutput::builder()
            .count(1)
            .scanned_count(1)
            .items([("total".into(), Av::N("5".into()))].into())
            .build()))
        .unwrap();
    assert_eq!(page.items[0].total, 5);
    trybuild::TestCases::new().compile_fail("tests/ui/count/*.rs");
}
