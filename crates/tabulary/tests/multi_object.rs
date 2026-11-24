#![cfg(feature = "derive")]
//! Strict multi-object tables: dispatch, polymorphic pages, fail-closed decoding,
//! member-only operations, and interop with batches and transactions.
use aws_sdk_dynamodb::{
    operation::{query::QueryOutput, scan::ScanOutput},
    types::AttributeValue as Av,
};
use std::num::NonZeroU32;
use tabulary::{
    BatchAction, BatchBudget, BuildError, Cursor, DecodeFault, DynamoType, Item, KeyPattern,
    KeyPatterns, MultiObject, MultiTable, ReadConsistency::*, ReadError, Record, ScanSegment,
    Update, transaction,
};

// The async functions are exercised against SDK replays in `examples.rs`.
#[allow(dead_code)]
#[path = "../../../docs/examples/08_multi_object.rs"]
mod example;
use example::{
    AccountTable, Customer, CustomerKey, Invoice, OrderAudit, OrderAuditKey, OrderHeader,
    OrderLine, OrderLineAttrs, OrderLineKey, OrderTable,
};

fn orders() -> MultiTable<OrderTable> {
    MultiTable::new("orders").unwrap()
}
fn header() -> OrderHeader {
    OrderHeader {
        order_ref: "ord-teal".into(),
        customer_ref: "cust-sand".into(),
    }
}
fn line(line_no: u64) -> OrderLine {
    OrderLine {
        order_ref: "ord-teal".into(),
        line_no,
        quantity: 3,
    }
}
fn audit() -> OrderAudit {
    OrderAudit {
        order_ref: "ord-teal".into(),
        seq: 7,
        note: "placed".into(),
    }
}
fn s(text: &str) -> Av {
    Av::S(text.into())
}
fn page(items: Vec<Item>, next: Option<Item>) -> QueryOutput {
    QueryOutput::builder()
        .count(items.len() as i32)
        .scanned_count(items.len() as i32)
        .set_items(Some(items))
        .set_last_evaluated_key(next)
        .build()
}
fn decode_error(item: Item) -> tabulary::DecodeError {
    let prepared = orders()
        .query(OrderHeader::primary().partition("ord-teal".into()), Strong)
        .build()
        .unwrap();
    let error = prepared
        .decode(Ok(page(vec![header().to_item().unwrap(), item], None)))
        .unwrap_err();
    let ReadError::Decode(error) = error else {
        panic!("expected a decode error")
    };
    assert_eq!(error.position, 1, "the corrupt row keeps its page position");
    assert_eq!(error.raw_items().len(), 2);
    error.source.clone()
}

#[test]
fn derived_schema_variants_and_member_checks() {
    let schema = OrderTable::SCHEMA;
    assert_eq!(schema.name, "OrderTable");
    let names: Vec<_> = schema.fields.iter().map(|f| (f.name, f.wire)).collect();
    assert_eq!(names, [("pk", DynamoType::S), ("sk", DynamoType::S)]);
    assert_eq!(schema.primary.sort, Some(1));
    assert_eq!(
        OrderTable::VARIANTS[1].keys,
        KeyPatterns {
            partition: KeyPattern::Prefix("ORD#"),
            sort: KeyPattern::Prefix("LINE#"),
        }
    );
    assert_eq!(OrderTable::VARIANTS[2].record, "OrderAudit");
    assert_eq!(
        OrderHeader::KEY_PATTERNS,
        KeyPatterns {
            partition: KeyPattern::Prefix("ORD#"),
            sort: KeyPattern::Exact("META"),
        }
    );
    assert_eq!(Customer::KEY_PATTERNS.sort, KeyPattern::Any);
    assert_eq!(AccountTable::SCHEMA.fields.len(), 1);
    assert_eq!(tabulary::multi_object_fault::<OrderTable>(), None);
    assert_eq!(tabulary::member_fault::<OrderTable, OrderLine>(), None);
    assert_eq!(tabulary::member_fault::<AccountTable, Invoice>(), None);
    assert!(matches!(
        MultiTable::<OrderTable>::new(""),
        Err(BuildError::OperationConstraint {
            code: "empty_table_name"
        })
    ));
    assert_eq!(orders().member::<OrderLine>().name(), "orders");
}

#[test]
fn pk_and_sk_branching_selects_each_member() {
    let items = [
        header().to_item().unwrap(),
        line(1).to_item().unwrap(),
        audit().to_item().unwrap(),
    ];
    assert_eq!(items[0]["sk"], s("META"));
    assert_eq!(items[1]["sk"], s("LINE#00000000000000000001"));
    assert_eq!(items[2]["sk"], s("AUDIT#00000000000000000007"));
    let decoded: Vec<_> = items
        .iter()
        .map(|item| OrderTable::from_item(item).unwrap())
        .collect();
    assert!(matches!(&decoded[0], OrderTable::Header(h) if h.customer_ref == "cust-sand"));
    assert!(matches!(&decoded[1], OrderTable::Line(l) if l.line_no == 1));
    assert!(matches!(&decoded[2], OrderTable::Audit(a) if a.note == "placed"));
    for (item, object) in items.iter().zip(&decoded) {
        assert_eq!(&object.to_item().unwrap(), item);
    }
}

#[test]
fn pk_only_branching_selects_each_member() {
    let customer = AccountTable::from(Customer {
        customer_ref: "cust-sand".into(),
        display_name: "Sand".into(),
    });
    let invoice = AccountTable::from(Invoice {
        invoice_ref: "inv-rose".into(),
        amount_due: 40,
    });
    let stored = invoice.to_item().unwrap();
    assert_eq!(stored["pk"], s("INV#inv-rose"));
    assert!(matches!(
        AccountTable::from_item(&stored).unwrap(),
        AccountTable::Invoice(i) if i.amount_due == 40
    ));
    let stored = customer.to_item().unwrap();
    assert_eq!(
        AccountTable::from_item(&stored).unwrap().variant_name(),
        "Customer"
    );
    let mut unknown = stored.clone();
    unknown.insert("pk".into(), s("SHIP#one"));
    let error = AccountTable::from_item(&unknown).unwrap_err();
    assert_eq!(
        (error.record, error.attribute()),
        ("AccountTable", Some("pk"))
    );
    assert_eq!(error.kind, DecodeFault::UnknownVariant);
}

#[test]
fn polymorphic_query_preserves_order_and_paginates_with_bound_cursors() {
    let build = |order| {
        orders()
            .query(OrderHeader::primary().partition("ord-teal".into()), Strong)
            .order(order)
            .limit(NonZeroU32::new(2).unwrap())
    };
    let prepared = build(tabulary::QueryOrder::Ascending).build().unwrap();
    let input = prepared.input();
    assert_eq!(input.table_name(), Some("orders"));
    assert_eq!(input.key_condition_expression(), Some("#n0 = :v0"));
    assert_eq!(input.limit(), Some(2));
    assert_eq!(prepared.expressions().names()["#n0"], "pk");
    assert_eq!(prepared.expressions().values()[":v0"], s("ORD#ord-teal"));
    assert_eq!(input.filter_expression(), None);

    let next: Item = [
        ("pk".into(), s("ORD#ord-teal")),
        ("sk".into(), s("LINE#00000000000000000001")),
    ]
    .into();
    let first = prepared
        .decode(Ok(page(
            vec![audit().to_item().unwrap(), line(1).to_item().unwrap()],
            Some(next.clone()),
        )))
        .unwrap();
    let kinds: Vec<_> = first.items.iter().map(OrderTable::variant_name).collect();
    assert_eq!(kinds, ["Audit", "Line"]);
    let cursor: Cursor<OrderTable, tabulary::MultiPartition> = first.next.unwrap();
    assert_eq!(cursor.last_evaluated_key(), &next);

    // The cursor survives bytes and resumes a query anchored on another member,
    // because every member shares the physical keys and partition value.
    let restored = Cursor::from_bytes(&cursor.to_bytes().unwrap()).unwrap();
    let resumed = orders()
        .query(OrderLine::primary().partition("ord-teal".into()), Strong)
        .limit(NonZeroU32::new(2).unwrap())
        .resume(restored)
        .build()
        .unwrap();
    assert_eq!(resumed.input().exclusive_start_key(), Some(&next));
    let last = resumed
        .decode(Ok(page(
            vec![line(2).to_item().unwrap(), header().to_item().unwrap()],
            None,
        )))
        .unwrap();
    let kinds: Vec<_> = last.items.iter().map(OrderTable::variant_name).collect();
    assert_eq!(kinds, ["Line", "Header"]);
    assert!(last.next.is_none());

    assert!(matches!(
        build(tabulary::QueryOrder::Descending)
            .resume(cursor.clone())
            .build(),
        Err(BuildError::CursorMismatch)
    ));
    let other_partition = orders()
        .query(OrderHeader::primary().partition("ord-sage".into()), Strong)
        .resume(cursor)
        .build();
    assert!(matches!(other_partition, Err(BuildError::CursorMismatch)));

    let foreign: Item = [("pk".into(), s("ORD#ord-teal")), ("note".into(), s("x"))].into();
    assert!(matches!(
        prepared.decode(Ok(page(vec![], Some(foreign)))),
        Err(ReadError::Decode(error)) if error.source.kind == DecodeFault::Custom { code: "invalid_page_cursor" }
    ));
}

#[test]
fn unknown_discriminators_fail_closed_with_the_enum_and_key() {
    let mut item = line(1).to_item().unwrap();
    item.insert("sk".into(), s("SHIP#00000000000000000001"));
    let error = decode_error(item);
    assert_eq!(
        (error.record, error.attribute()),
        ("OrderTable", Some("sk"))
    );
    assert_eq!(error.kind, DecodeFault::UnknownVariant);

    let mut item = line(1).to_item().unwrap();
    item.insert("pk".into(), s("CUST#ord-teal"));
    let error = decode_error(item);
    assert_eq!(error.attribute(), Some("pk"));
    assert_eq!(error.kind, DecodeFault::UnknownVariant);

    let mut item = line(1).to_item().unwrap();
    item.remove("sk");
    let error = decode_error(item);
    assert_eq!(
        (error.attribute(), error.kind),
        (Some("sk"), DecodeFault::Missing)
    );

    let mut item = line(1).to_item().unwrap();
    item.insert("pk".into(), Av::N("1".into()));
    let error = decode_error(item);
    assert_eq!(error.attribute(), Some("pk"));
    assert_eq!(
        error.kind,
        DecodeFault::WrongType {
            expected: DynamoType::S,
            actual: DynamoType::N
        }
    );
}

#[test]
fn corrupt_member_attributes_fail_closed_with_the_member_document() {
    let mut item = line(1).to_item().unwrap();
    item.insert("quantity".into(), s("three"));
    let error = decode_error(item);
    assert_eq!(
        (error.record, error.document, error.attribute()),
        ("OrderTable", "OrderLine", Some("quantity"))
    );
    assert!(matches!(error.kind, DecodeFault::WrongType { .. }));

    // The discriminator matches, but the member's own key template refuses it.
    let mut item = line(1).to_item().unwrap();
    item.insert("sk".into(), s("LINE#one"));
    let error = decode_error(item);
    assert_eq!(
        (error.document, error.attribute()),
        ("OrderLine", Some("sk"))
    );
    assert!(matches!(error.kind, DecodeFault::KeyFormat(_)));

    let mut item = audit().to_item().unwrap();
    item.remove("note");
    let error = decode_error(item);
    assert_eq!(
        (error.document, error.attribute()),
        ("OrderAudit", Some("note"))
    );
    assert_eq!(error.kind, DecodeFault::Missing);

    let mut item = header().to_item().unwrap();
    item.insert("surplus".into(), s("x"));
    let error = decode_error(item);
    assert_eq!(
        (error.document, error.attribute()),
        ("OrderHeader", Some("surplus"))
    );
    assert_eq!(error.kind, DecodeFault::UnknownAttribute);
}

#[test]
fn entity_queries_narrow_the_sort_key_to_one_member() {
    let lines = orders()
        .query_entity(OrderLine::primary().partition("ord-teal".into()), Eventual)
        .build()
        .unwrap();
    let expressions = lines.expressions();
    assert_eq!(
        expressions.key_condition(),
        Some("(#n0 = :v0 AND begins_with(#n1, :v1))")
    );
    assert_eq!(expressions.names()["#n1"], "sk");
    assert_eq!(expressions.values()[":v1"], s("LINE#"));
    let decoded = lines
        .decode(Ok(page(vec![line(4).to_item().unwrap()], None)))
        .unwrap();
    assert_eq!(decoded.items[0].line_no, 4);
    // Strict member decoding still refuses any other member.
    let Err(ReadError::Decode(error)) =
        lines.decode(Ok(page(vec![header().to_item().unwrap()], None)))
    else {
        panic!("a header must not decode as a line")
    };
    assert_eq!(
        (
            error.position,
            error.source.document,
            error.source.attribute()
        ),
        (0, "OrderLine", Some("sk"))
    );
    assert!(matches!(error.source.kind, DecodeFault::KeyFormat(_)));

    let header = orders()
        .query_entity(OrderHeader::primary().partition("ord-teal".into()), Strong)
        .build()
        .unwrap();
    assert_eq!(
        header.expressions().key_condition(),
        Some("(#n0 = :v0 AND #n1 = :v1)")
    );
    assert_eq!(header.expressions().values()[":v1"], s("META"));

    let accounts = MultiTable::<AccountTable>::new("accounts").unwrap();
    let customers = accounts
        .query_entity(Customer::primary().partition("cust-sand".into()), Strong)
        .build()
        .unwrap();
    assert_eq!(customers.expressions().key_condition(), Some("#n0 = :v0"));
    assert_eq!(customers.expressions().values()[":v0"], s("CUST#cust-sand"));
}

#[test]
fn polymorphic_scan_decodes_members_and_binds_its_cursor() {
    let prepared = orders()
        .scan(Eventual)
        .segment(ScanSegment::new(1, 4).unwrap())
        .limit(NonZeroU32::new(5).unwrap())
        .build()
        .unwrap();
    assert_eq!(prepared.input().segment(), Some(1));
    assert_eq!(prepared.input().total_segments(), Some(4));
    assert_eq!(prepared.input().filter_expression(), None);
    let next: Item = [("pk".into(), s("ORD#ord-teal")), ("sk".into(), s("META"))].into();
    let items = vec![header().to_item().unwrap(), audit().to_item().unwrap()];
    let output = ScanOutput::builder()
        .count(2)
        .scanned_count(2)
        .set_items(Some(items))
        .set_last_evaluated_key(Some(next.clone()))
        .build();
    let page = prepared.decode(Ok(output)).unwrap();
    let kinds: Vec<_> = page.items.iter().map(OrderTable::variant_name).collect();
    assert_eq!(kinds, ["Header", "Audit"]);
    let cursor: Cursor<OrderTable, ()> = page.next.unwrap();
    let resumed = orders()
        .scan(Eventual)
        .segment(ScanSegment::new(1, 4).unwrap())
        .resume(cursor.clone())
        .build()
        .unwrap();
    assert_eq!(resumed.input().exclusive_start_key(), Some(&next));
    assert!(matches!(
        orders().scan(Eventual).resume(cursor).build(),
        Err(BuildError::CursorMismatch)
    ));
}

#[test]
fn distinct_members_commit_in_one_transaction_on_the_same_table() {
    let orders = orders();
    let prepared = transaction((
        orders.put(&header()).expect_new().into_tx().unwrap(),
        orders
            .update(OrderLineKey::new("ord-teal".into(), 1).unwrap())
            .changes(Update::new().and(OrderLineAttrs::QUANTITY.increment(1)))
            .into_tx()
            .unwrap(),
        orders
            .delete(OrderAuditKey::new("ord-teal".into(), 7).unwrap())
            .into_tx()
            .unwrap(),
        orders
            .check(
                OrderLineKey::new("ord-teal".into(), 2).unwrap(),
                OrderLineAttrs::QUANTITY.ge(1),
            )
            .into_tx()
            .unwrap(),
    ))
    .build()
    .unwrap();
    let items = prepared.input().transact_items();
    assert_eq!(items.len(), 4);
    assert_eq!(items[0].put().unwrap().table_name(), "orders");
    assert_eq!(items[0].put().unwrap().item()["sk"], s("META"));
    let update = items[1].update().unwrap();
    assert_eq!(update.table_name(), "orders");
    assert_eq!(update.key()["sk"], s("LINE#00000000000000000001"));
    // The existence guard survives transaction conversion.
    assert_eq!(update.condition_expression(), Some("attribute_exists(#n1)"));
    assert_eq!(update.expression_attribute_names().unwrap()["#n1"], "pk");
    assert_eq!(
        items[2].delete().unwrap().key()["sk"],
        s("AUDIT#00000000000000000007")
    );
    assert_eq!(
        items[3].condition_check().unwrap().key()["sk"],
        s("LINE#00000000000000000002")
    );

    // Same physical target twice is still refused across members.
    let twice = transaction((
        orders.put(&line(1)).into_tx().unwrap(),
        orders
            .delete(OrderLineKey::new("ord-teal".into(), 1).unwrap())
            .into_tx()
            .unwrap(),
    ))
    .build();
    assert!(twice.is_err());
}

#[test]
fn member_updates_never_create_partial_items() {
    let key = || OrderLineKey::new("ord-teal".into(), 1).unwrap();
    let guarded = orders()
        .update(key())
        .changes(Update::new().and(OrderLineAttrs::QUANTITY.increment(1)))
        .build()
        .unwrap();
    let input = guarded.input();
    assert_eq!(input.condition_expression(), Some("attribute_exists(#n1)"));
    assert_eq!(input.expression_attribute_names().unwrap()["#n1"], "pk");
    let both = orders()
        .update(key())
        .changes(Update::new().and(OrderLineAttrs::QUANTITY.increment(1)))
        .condition(OrderLineAttrs::QUANTITY.lt(9))
        .build()
        .unwrap();
    assert_eq!(
        both.input().condition_expression(),
        Some("(attribute_exists(#n1) AND #n0 < :v1)")
    );
}

#[test]
fn member_batches_target_the_shared_table() {
    let orders = orders();
    let mut writes = orders
        .batch_write(
            vec![
                BatchAction::put(&line(1)).unwrap(),
                BatchAction::put(&line(2)).unwrap(),
            ],
            BatchBudget::new(1).unwrap(),
        )
        .unwrap();
    let request = writes.next_request().unwrap().unwrap();
    let items = &request.input().request_items().unwrap()["orders"];
    assert_eq!(items.len(), 2);
    let accounts = MultiTable::<AccountTable>::new("accounts").unwrap();
    let mut reads = accounts
        .batch_get(
            vec![CustomerKey::new("cust-sand".into()).unwrap()],
            Strong,
            BatchBudget::new(1).unwrap(),
        )
        .unwrap();
    let request = reads.next_request().unwrap().unwrap();
    let keys = request.input().request_items().unwrap()["accounts"].keys();
    assert_eq!(keys[0]["pk"], s("CUST#cust-sand"));
}

#[test]
fn undeclared_types_and_overlapping_prefixes_fail_to_compile() {
    trybuild::TestCases::new().compile_fail("tests/ui/multi_object/*.rs");
}
