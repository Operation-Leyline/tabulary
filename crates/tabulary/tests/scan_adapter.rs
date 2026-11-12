#![cfg(feature = "derive")]

use aws_sdk_dynamodb::{
    Client, Config,
    operation::scan::ScanOutput,
    types::{AttributeValue as Av, ConsumedCapacity},
};
use std::num::NonZeroU32;
use tabulary::{BuildError, Capacity, Item, ReadConsistency::*, ReadError, ScanSegment, Table};

#[derive(Debug, PartialEq, tabulary::Record)]
#[tabulary(rename_all = "snake_case",
    index(ident = ByLabel, name = "by_label", kind = global, partition = label, project = all),
    index(ident = ByQuantity, name = "by_quantity", kind = local, partition = id, sort = quantity, project = keys_only))]
struct Order {
    #[tabulary(partition_key)]
    id: String,
    #[tabulary(sort_key)]
    line: u64,
    label: String,
    quantity: u64,
}
#[derive(Debug, PartialEq, tabulary::Projection)]
#[tabulary(record = Order)]
struct Quantity {
    #[tabulary(attribute = OrderAttrs::QUANTITY)]
    quantity: u64,
}
fn table() -> Table<Order> {
    Table::new("orders").unwrap()
}
fn segment(n: u32, total: u32) -> ScanSegment {
    ScanSegment::new(n, total).unwrap()
}
fn key() -> Item {
    [
        ("id".into(), Av::S("private-order".into())),
        ("line".into(), Av::N("2".into())),
    ]
    .into()
}
fn output(items: Vec<Item>, key: Option<Item>) -> ScanOutput {
    ScanOutput::builder()
        .count(items.len() as i32)
        .scanned_count(5)
        .set_items(Some(items))
        .set_last_evaluated_key(key)
        .consumed_capacity(ConsumedCapacity::builder().capacity_units(2.5).build())
        .build()
}

#[test]
fn inputs_are_explicit_and_limit_counts_evaluated_items() {
    let prepared = table()
        .scan(Strong)
        .filter(OrderAttrs::ID.eq("private-order".into()))
        .select::<Quantity>()
        .limit(NonZeroU32::new(7).unwrap())
        .capacity(Capacity::Indexes)
        .build()
        .unwrap();
    let input = prepared.input();
    assert_eq!(input.segment, None);
    assert_eq!(input.total_segments, None);
    assert_eq!(input.limit, Some(7));
    assert_eq!(input.consistent_read, Some(true));
    assert_eq!(prepared.expressions().filter(), Some("#n0 = :v0"));
    assert_eq!(prepared.expressions().projection(), Some("#n1"));
    assert_eq!(prepared.expressions().names()["#n0"], "id");
    assert_eq!(
        prepared.expressions().values()[":v0"],
        Av::S("private-order".into())
    );
    let page = prepared
        .decode(Ok(output(
            vec![[("quantity".into(), Av::N("9".into()))].into()],
            None,
        )))
        .unwrap();
    assert_eq!(page.items, [Quantity { quantity: 9 }]);
    assert_eq!((page.count, page.scanned_count), (1, 5));
    assert_eq!(page.capacity.unwrap().capacity_units, Some(2.5));
    assert!(matches!(
        table()
            .scan(Eventual)
            .limit(NonZeroU32::new(u32::MAX).unwrap())
            .build(),
        Err(BuildError::RequestLimit { .. })
    ));
    for (n, total) in [(0, 1), (999_999, 1_000_000)] {
        let prepared = table()
            .scan(Eventual)
            .segment(segment(n, total))
            .build()
            .unwrap();
        assert_eq!(
            (prepared.input().segment, prepared.input().total_segments),
            (Some(n as i32), Some(total as i32))
        );
    }
}

#[test]
fn each_segment_and_total_has_its_own_continuation() {
    let make = || table().scan(Strong).segment(segment(1, 3));
    let prepared = make().build().unwrap();
    let page = prepared.decode(Ok(output(vec![], Some(key())))).unwrap();
    assert!(page.items.is_empty());
    let cursor = page.next.unwrap();
    let resumed = make()
        .resume(cursor.clone())
        .limit(NonZeroU32::new(1).unwrap())
        .capacity(Capacity::Total)
        .build()
        .unwrap();
    assert_eq!(resumed.input().exclusive_start_key, Some(key()));
    for changed in [
        table().scan(Strong),
        table().scan(Strong).segment(segment(0, 3)),
        table().scan(Strong).segment(segment(1, 4)),
        table().scan(Eventual).segment(segment(1, 3)),
    ] {
        assert!(matches!(
            changed.resume(cursor.clone()).build(),
            Err(BuildError::CursorMismatch)
        ));
    }
    let full = table().scan(Strong).build().unwrap();
    let page = full.decode(Ok(output(vec![], Some(key())))).unwrap();
    let cursor = page.next.unwrap();
    assert!(matches!(
        make().resume(cursor.clone()).build(),
        Err(BuildError::CursorMismatch)
    ));
    assert!(table().scan(Strong).resume(cursor).build().is_ok());
    for end in [None, Some(Item::new())] {
        let page = prepared.decode(Ok(output(vec![], end))).unwrap();
        assert!(page.next.is_none());
    }
    assert!(!format!("{prepared:?}").contains("private-order"));
}

#[test]
fn index_scans_check_consistency_projection_and_complete_service_keys() {
    let gsi = || table().index(OrderIndexes::BY_LABEL);
    assert!(matches!(
        gsi().scan(Strong).build(),
        Err(BuildError::UnsupportedConsistency)
    ));
    let prepared = gsi().scan(Eventual).build().unwrap();
    assert_eq!(prepared.input().index_name.as_deref(), Some("by_label"));
    let mut complete = key();
    complete.insert("label".into(), Av::S("ready".into()));
    let cursor = prepared
        .decode(Ok(output(vec![], Some(complete.clone()))))
        .unwrap()
        .next
        .unwrap();
    let resumed = gsi().scan(Eventual).resume(cursor).build().unwrap();
    assert_eq!(resumed.input().exclusive_start_key, Some(complete));
    assert!(matches!(
        prepared.decode(Ok(output(vec![], Some(key())))),
        Err(ReadError::Decode(_))
    ));
    let lsi = || table().index(OrderIndexes::BY_QUANTITY).scan(Strong);
    assert!(matches!(
        lsi().build(),
        Err(BuildError::OperationConstraint {
            code: "projection_not_available"
        })
    ));
    assert!(
        lsi()
            .select::<Quantity>()
            .filter(OrderAttrs::ID.exists())
            .build()
            .is_ok()
    );
    assert!(
        lsi()
            .select::<Quantity>()
            .filter(OrderAttrs::LABEL.size().gt(0).not())
            .build()
            .is_err()
    );
}

#[test]
fn corrupt_pages_preserve_raw_items_capacity_and_continuation() {
    let prepared = table().scan(Strong).build().unwrap();
    let ReadError::Decode(fault) = prepared
        .decode(Ok(output(vec![Item::new()], Some(key()))))
        .unwrap_err()
    else {
        panic!("strict fault")
    };
    assert_eq!(fault.source.attribute(), Some("id"));
    assert_eq!(fault.raw_items(), &[Item::new()]);
    assert_eq!(fault.capacity.unwrap().capacity_units, Some(2.5));
    assert_eq!(fault.page.unwrap().last_evaluated_key(), Some(&key()));
}

#[test]
fn apply_clears_every_stale_sdk_field_and_never_calls_transport_or_fans_out() {
    use aws_sdk_dynamodb::types::{
        ComparisonOperator, Condition, ConditionalOperator, ReturnConsumedCapacity, Select,
    };
    use aws_smithy_runtime_api::client::http::http_client_fn;
    let config = Config::builder()
        .behavior_version_latest()
        .region(aws_sdk_dynamodb::config::Region::new("us-east-1"))
        .retry_config(aws_sdk_dynamodb::config::retry::RetryConfig::disabled())
        .timeout_config(aws_sdk_dynamodb::config::timeout::TimeoutConfig::disabled())
        .identity_cache(aws_sdk_dynamodb::config::IdentityCache::no_cache())
        .stalled_stream_protection(
            aws_sdk_dynamodb::config::StalledStreamProtectionConfig::disabled(),
        )
        .http_client(http_client_fn(|_, _| {
            panic!("Scan must never request a connector")
        }))
        .build();
    let client = Client::from_conf(config);
    let full = table().scan(Eventual).build().unwrap();
    let legacy = Condition::builder()
        .comparison_operator(ComparisonOperator::Eq)
        .build()
        .unwrap();
    let stale = client
        .scan()
        .table_name("stale")
        .index_name("stale")
        .select(Select::Count)
        .attributes_to_get("stale")
        .limit(1)
        .consistent_read(true)
        .scan_filter("stale", legacy)
        .conditional_operator(ConditionalOperator::Or)
        .exclusive_start_key("stale", Av::Bool(true))
        .return_consumed_capacity(ReturnConsumedCapacity::Total)
        .total_segments(10)
        .segment(4)
        .projection_expression("stale")
        .filter_expression("stale")
        .expression_attribute_names("#stale", "stale")
        .expression_attribute_values(":stale", Av::Bool(true));
    assert_eq!(
        full.apply(stale).as_input().clone().build().unwrap(),
        *full.input()
    );
    assert!(full.input().expression_attribute_names.is_none());
    assert!(full.input().expression_attribute_values.is_none());
    let prepared = table()
        .scan(Strong)
        .segment(segment(1, 3))
        .select::<Quantity>()
        .filter(OrderAttrs::QUANTITY.gt(2))
        .limit(NonZeroU32::new(7).unwrap())
        .capacity(Capacity::Indexes)
        .build()
        .unwrap();
    let fluent = prepared.apply(client.scan());
    assert_eq!(
        fluent.as_input().clone().build().unwrap(),
        *prepared.input()
    );
    let page = prepared.decode(Ok(output(vec![], Some(key())))).unwrap();
    assert!(page.next.is_some());
}

#[test]
fn consistency_is_required_on_table_and_index_scans() {
    trybuild::TestCases::new().compile_fail("tests/ui/scan/*.rs");
}
