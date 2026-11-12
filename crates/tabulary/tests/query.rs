#![cfg(feature = "derive")]

use aws_sdk_dynamodb::{
    Client, Config,
    error::SdkError,
    operation::query::QueryOutput,
    types::{AttributeValue as Av, Condition, ConditionalOperator, ConsumedCapacity, Select},
};
use std::num::NonZeroU32;
use tabulary::{
    BuildError, Capacity, DecodeFault, Item, QueryOrder, ReadConsistency::*, ReadError, Table,
};

#[derive(Debug, PartialEq, tabulary::Record)]
#[tabulary(rename_all = "snake_case",
    index(ident = ByLabel, name = "by_label", kind = global, partition = label, sort = line, project = all),
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
#[derive(tabulary::Projection)]
#[tabulary(record = Order)]
struct Label {
    #[tabulary(attribute = OrderAttrs::LABEL)]
    label: String,
}
fn table() -> Table<Order> {
    Table::new("orders").unwrap()
}
fn key() -> Item {
    [
        ("id".into(), Av::S("private-order".into())),
        ("line".into(), Av::N("2".into())),
    ]
    .into()
}
fn item(n: u64) -> Item {
    Order {
        id: "private-order".into(),
        line: n,
        label: "ready".into(),
        quantity: 3,
    }
    .to_item()
    .unwrap()
}
fn capacity() -> ConsumedCapacity {
    ConsumedCapacity::builder()
        .table_name("orders")
        .capacity_units(2.5)
        .build()
}
fn partition() -> tabulary::KeyCondition<Order, OrderPrimary> {
    Order::primary().partition("private-order".into())
}
fn output(items: Vec<Item>, scanned: i32, key: Option<Item>) -> QueryOutput {
    QueryOutput::builder()
        .count(items.len() as i32)
        .set_items(Some(items))
        .scanned_count(scanned)
        .set_last_evaluated_key(key)
        .consumed_capacity(capacity())
        .build()
}
fn request() -> tabulary::PreparedQuery<Order, OrderPrimary> {
    table().query(partition(), Strong).build().unwrap()
}

#[test]
fn empty_filtered_page_retains_continuation_and_resumes_with_a_different_limit() {
    let make = || {
        table()
            .query(partition().sort_ge(2), Strong)
            .filter(OrderAttrs::QUANTITY.gt(2))
            .order(QueryOrder::Descending)
            .select::<Quantity>()
    };
    let prepared = make().limit(NonZeroU32::new(7).unwrap()).build().unwrap();
    assert_eq!(prepared.input().limit, Some(7));
    assert_eq!(prepared.input().scan_index_forward, Some(false));
    assert_eq!(
        prepared.expressions().key_condition(),
        Some("(#n0 = :v0 AND #n1 >= :v1)")
    );
    assert_eq!(prepared.expressions().filter(), Some("#n2 > :v2"));
    assert_eq!(prepared.expressions().projection(), Some("#n2"));
    let page = prepared.decode(Ok(output(vec![], 7, Some(key())))).unwrap();
    assert!(page.items.is_empty());
    assert_eq!((page.count, page.scanned_count), (0, 7));
    assert_eq!(page.capacity, Some(capacity()));
    let cursor = page.next.unwrap();
    assert_eq!(cursor.last_evaluated_key(), &key());
    let resumed = make()
        .resume(cursor.clone())
        .limit(NonZeroU32::new(1).unwrap())
        .capacity(Capacity::Indexes)
        .build()
        .unwrap();
    assert_eq!(resumed.input().exclusive_start_key, Some(key()));
    assert_eq!(resumed.input().limit, Some(1));
    assert!(matches!(
        make().order(QueryOrder::Ascending).resume(cursor).build(),
        Err(BuildError::CursorMismatch)
    ));
    for end in [None, Some(Item::new())] {
        let page = prepared.decode(Ok(output(vec![], 0, end))).unwrap();
        assert!(page.next.is_none());
    }
    let page = prepared
        .decode(Ok(output(
            vec![[("quantity".into(), Av::N("8".into()))].into()],
            1,
            None,
        )))
        .unwrap();
    assert_eq!(page.items, vec![Quantity { quantity: 8 }]);
}

#[test]
fn key_filters_and_out_of_range_limits_are_refused_before_apply() {
    for filter in [
        OrderAttrs::ID.exists(),
        OrderAttrs::ID.size().gt(0),
        OrderAttrs::LINE.eq(2),
        OrderAttrs::QUANTITY.eq_path(OrderAttrs::LINE),
        OrderAttrs::QUANTITY
            .gt(0)
            .or(OrderAttrs::ID.not_exists().not()),
    ] {
        assert!(matches!(
            table().query(partition(), Eventual).filter(filter).build(),
            Err(BuildError::OperationConstraint {
                code: "query_key_in_filter"
            })
        ));
    }
    assert!(matches!(
        table()
            .query(partition(), Eventual)
            .limit(NonZeroU32::new(u32::MAX).unwrap())
            .build(),
        Err(BuildError::RequestLimit { .. })
    ));
}

#[test]
fn index_consistency_projection_and_complete_cursor_keys_are_checked() {
    let make = |consistency| {
        table().index(OrderIndexes::BY_LABEL).query(
            OrderIndexes::BY_LABEL.partition("ready".into()),
            consistency,
        )
    };
    assert!(matches!(
        make(Strong).build(),
        Err(BuildError::UnsupportedConsistency)
    ));
    let prepared = make(Eventual).build().unwrap();
    assert_eq!(prepared.input().index_name.as_deref(), Some("by_label"));
    let page = make(Eventual)
        .select::<Label>()
        .build()
        .unwrap()
        .decode(Ok(QueryOutput::builder()
            .items([("label".into(), Av::S("ready".into()))].into())
            .count(1)
            .scanned_count(1)
            .build()))
        .unwrap();
    assert_eq!(page.items[0].label, "ready");
    let mut complete = key();
    complete.insert("label".into(), Av::S("ready".into()));
    let page = prepared
        .decode(Ok(QueryOutput::builder()
            .set_last_evaluated_key(Some(complete.clone()))
            .build()))
        .unwrap();
    assert_eq!(
        make(Eventual)
            .resume(page.next.unwrap())
            .build()
            .unwrap()
            .input()
            .exclusive_start_key,
        Some(complete)
    );
    let lsi = || {
        table().index(OrderIndexes::BY_QUANTITY).query(
            OrderIndexes::BY_QUANTITY.partition("private-order".into()),
            Strong,
        )
    };
    assert!(matches!(
        lsi().build(),
        Err(BuildError::OperationConstraint {
            code: "projection_not_available"
        })
    ));
    assert!(lsi().select::<Quantity>().build().is_ok());
    assert!(lsi().select::<Label>().build().is_err());
    assert!(
        lsi()
            .select::<Quantity>()
            .filter(OrderAttrs::LABEL.exists())
            .build()
            .is_err()
    );
    assert!(
        make(Eventual)
            .filter(OrderAttrs::ID.exists())
            .build()
            .is_ok()
    );
    assert!(
        make(Eventual)
            .filter(OrderAttrs::LABEL.exists())
            .build()
            .is_err()
    );
}

#[test]
fn corrupt_pages_keep_all_rows_position_accounting_and_unmodified_continuation() {
    let prepared = request();
    let rows = vec![item(9), Item::new(), item(1)];
    let error = prepared
        .decode(Ok(output(rows.clone(), 5, Some(key()))))
        .unwrap_err();
    assert!(!format!("{error:?} {error}").contains("private-order"));
    let ReadError::Decode(fault) = error else {
        panic!("strict page decode")
    };
    assert_eq!(fault.position, 1);
    assert_eq!(fault.source.kind, DecodeFault::Missing);
    assert_eq!(fault.source.attribute(), Some("id"));
    assert_eq!(fault.raw_items(), rows);
    assert_eq!(fault.capacity, Some(capacity()));
    let metadata = fault.page.unwrap();
    assert_eq!((metadata.count, metadata.scanned_count), (3, 5));
    assert_eq!(metadata.last_evaluated_key(), Some(&key()));
    let page = prepared
        .decode(Ok(output(vec![item(9), item(1)], 2, None)))
        .unwrap();
    assert_eq!(
        page.items.iter().map(|item| item.line).collect::<Vec<_>>(),
        [9, 1]
    );
    let error = prepared
        .decode(Err(SdkError::construction_failure(std::io::Error::other(
            "private-sdk",
        ))))
        .unwrap_err();
    assert!(matches!(
        error.sdk_error(),
        Some(SdkError::ConstructionFailure(_))
    ));
    assert!(!format!("{error:?} {error}").contains("private-sdk"));
}

#[test]
fn apply_copies_all_modeled_fields_and_never_uses_transport_or_follows_a_page() {
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
            panic!("Query must not request a connector")
        }))
        .build();
    let client = Client::from_conf(config);
    let prepared = request();
    let legacy = Condition::builder()
        .comparison_operator(aws_sdk_dynamodb::types::ComparisonOperator::Eq)
        .build()
        .unwrap();
    let stale = client
        .query()
        .table_name("stale")
        .index_name("stale")
        .select(Select::Count)
        .attributes_to_get("stale")
        .limit(1)
        .consistent_read(false)
        .scan_index_forward(false)
        .key_conditions("stale", legacy.clone())
        .query_filter("stale", legacy)
        .conditional_operator(ConditionalOperator::Or)
        .exclusive_start_key("stale", Av::S("secret".into()))
        .return_consumed_capacity(aws_sdk_dynamodb::types::ReturnConsumedCapacity::Total)
        .projection_expression("stale")
        .filter_expression("stale")
        .key_condition_expression("stale")
        .expression_attribute_names("#stale", "stale")
        .expression_attribute_values(":stale", Av::Bool(true));
    assert_eq!(
        prepared.apply(stale).as_input().clone().build().unwrap(),
        *prepared.input()
    );
    assert_eq!(prepared.input().scan_index_forward, Some(true));
    assert_eq!(prepared.input().limit, None);
    assert!(!format!("{prepared:?}").contains("private-order"));
    assert!(
        prepared
            .decode(Ok(QueryOutput::builder()
                .set_last_evaluated_key(Some(key()))
                .build()))
            .unwrap()
            .next
            .is_some()
    );
    let selected = table()
        .query(partition(), Eventual)
        .filter(OrderAttrs::LABEL.eq("ready".into()))
        .select::<Quantity>()
        .limit(NonZeroU32::new(i32::MAX as u32).unwrap())
        .order(QueryOrder::Descending)
        .capacity(Capacity::Total)
        .build()
        .unwrap();
    let applied = selected.apply(client.query());
    assert_eq!(
        applied.as_input().clone().build().unwrap(),
        *selected.input()
    );
}

#[test]
fn consistency_record_and_index_are_compile_time_requirements() {
    trybuild::TestCases::new().compile_fail("tests/ui/query/*.rs");
}
