#![cfg(feature = "derive")]
use aws_sdk_dynamodb::types::AttributeValue as Av;
use tabulary::{IndexKind, IndexProjection, IndexSchema, Native, NoSort, Record};

#[derive(Debug, PartialEq, tabulary::Record)]
#[tabulary(rename_all = "camelCase",
    index(ident = ByCourier, name = "by_courier", kind = global, partition = courier, project = include(gift_note)),
    index(ident = ByLine, name = "by_line", kind = local, partition = order_ref, sort = line_no, project = keys_only))]
struct Order {
    #[tabulary(partition_key)]
    order_ref: String,
    #[tabulary(sort_key)]
    created: u64,
    courier: String,
    line_no: u64,
    gift_note: Option<String>,
}

#[test]
fn literal_index_updates_validate_the_target_key() {
    use tabulary::{BuildError, EncodeFault, Update};
    for action in [
        OrderAttrs::COURIER.set(String::new()),
        OrderAttrs::COURIER.set_expr(OrderAttrs::GIFT_NOTE.if_not_exists(String::new())),
    ] {
        assert!(
            matches!(Update::new().and(action).build(), Err(BuildError::Encode(e)) if e.kind == EncodeFault::EmptyKey && e.attribute() == Some("courier"))
        );
    }
    assert!(
        Update::new()
            .and(OrderAttrs::COURIER.set("courier".into()))
            .build()
            .is_ok()
    );
    assert!(
        Update::new()
            .and(OrderAttrs::GIFT_NOTE.set(String::new()))
            .build()
            .is_ok()
    );
    assert!(
        matches!(Update::new().and(OrderAttrs::COURIER.set("c".repeat(2049))).build(), Err(BuildError::Encode(e)) if matches!(e.kind, EncodeFault::KeySize { max: 2048, .. }))
    );
}

#[test]
fn hash_only_global_local_and_projected_names_share_one_schema() {
    type Text = Native<String>;
    fn global<I: IndexSchema<Order, Partition = String, PartitionCodec = Text, Sort = NoSort>>(
        _: I,
    ) {
    }
    global(OrderIndexes::BY_COURIER);
    let gsi = OrderByCourier::DEFINITION;
    assert_eq!(gsi.kind, IndexKind::Global);
    assert_eq!(gsi.keys.sort, None);
    assert_eq!(gsi.projection, IndexProjection::Include(&["giftNote"]));
    let lsi = OrderByLine::DEFINITION;
    assert_eq!(lsi.kind, IndexKind::Local);
    assert_eq!(lsi.keys.partition, Order::SCHEMA.primary.partition);
    assert_ne!(lsi.keys.sort, Order::SCHEMA.primary.sort);
    assert_eq!(Order::SCHEMA.indexes, &[gsi, lsi]);
    let record = Order {
        order_ref: "o".into(),
        created: 1,
        courier: "c".into(),
        line_no: 2,
        gift_note: None,
    };
    assert_eq!(
        Order::from_item(&record.to_item().unwrap()).unwrap(),
        record
    );
}

#[derive(tabulary::Record)]
#[tabulary(crate = "::tabulary", generated_prefix = "Stored", rename_all = "snake_case",
    index(ident = ByValue, name = "by_value", kind = global, partition = value, sort = sequence, project = all))]
struct Generic<T> {
    #[tabulary(partition_key)]
    id: String,
    value: T,
    sequence: u64,
}
#[test]
fn generic_binary_keys_validate_physical_values_without_changing_primary_keys() {
    let record = Generic {
        id: "o".into(),
        value: vec![1u8],
        sequence: 1,
    };
    let handle = StoredIndexes::<Vec<u8>>::BY_VALUE;
    let _copy = handle;
    assert_eq!(
        StoredByValue::<Vec<u8>>::DEFINITION.projection,
        IndexProjection::All
    );
    assert_eq!(
        tabulary::RecordKey::encode_key(&record.key().unwrap())
            .unwrap()
            .len(),
        1
    );
    let mut item = record.to_item().unwrap();
    item.insert("value".into(), Av::S("wrong".into()));
    let error = Generic::<Vec<u8>>::from_item(&item).err().unwrap();
    assert!(matches!(
        error.kind,
        tabulary::DecodeFault::WrongType { .. }
    ));
    assert_eq!(error.attribute(), Some("value"));
    let empty = Generic {
        value: Vec::<u8>::new(),
        ..record
    };
    assert_eq!(
        empty.to_item().unwrap_err().kind,
        tabulary::EncodeFault::EmptyKey
    );
}
