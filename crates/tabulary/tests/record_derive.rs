#![cfg(feature = "derive")]

use aws_sdk_dynamodb::types::AttributeValue as Av;
use tabulary::{DecodeFault, DynamoType, EncodeFault, Record, RecordKey};

#[derive(Debug, PartialEq, tabulary::Record)]
#[tabulary(rename_all = "camelCase")]
struct Order {
    quantity: u64,
    #[tabulary(partition_key)]
    order_ref: String,
    #[tabulary(sort_key)]
    line_no: u64,
    #[tabulary(rename = "gift.note")]
    gift_note: Option<String>,
}

fn order() -> Order {
    Order {
        quantity: 3,
        order_ref: "order-lime".into(),
        line_no: 8,
        gift_note: None,
    }
}

#[test]
fn record_and_key_round_trip_with_exact_names_tags_and_descriptors() {
    let record = order();
    let item = record.to_item().unwrap();
    assert_eq!(item.len(), 3);
    assert_eq!(item["orderRef"], Av::S("order-lime".into()));
    assert_eq!(item["lineNo"], Av::N("8".into()));
    assert_eq!(Order::from_item(&item).unwrap(), record);
    let encode = |key: OrderKey| key.encode_key().unwrap();
    let encoded = encode(record.key().unwrap());
    assert_eq!(encoded.len(), 2);
    assert_eq!(encode(OrderKey::decode_key(&item).unwrap()), encoded);
    assert_eq!(
        encode(OrderKey::new("order-lime".into(), 8).unwrap()),
        encoded
    );
    assert_eq!(Order::SCHEMA.primary.partition, 1);
    assert_eq!(Order::SCHEMA.primary.sort, Some(2));
    assert_eq!(OrderAttrs::GIFT_NOTE.name(), "gift.note");
    assert!(OrderAttrs::QUANTITY.validate_mutation().is_ok());
    let mut present = item;
    present.insert("gift.note".into(), Av::S(String::new()));
    assert_eq!(
        Order::from_item(&present).unwrap().gift_note.as_deref(),
        Some("")
    );
}

fn assert_decode(item: &tabulary::Item, attribute: &str, kind: DecodeFault) {
    let error = Order::from_item(item).unwrap_err();
    assert_eq!(error.record, "Order");
    assert_eq!(error.document, "Order");
    assert_eq!(error.attribute(), Some(attribute));
    assert_eq!(error.kind, kind);
}

#[test]
fn missing_and_wrong_types_name_the_record_and_physical_attribute() {
    let mut item = order().to_item().unwrap();
    item.remove("quantity");
    item.remove("orderRef");
    assert_decode(&item, "orderRef", DecodeFault::Missing);
    item.insert("orderRef".into(), Av::S("order-lime".into()));
    assert_decode(&item, "quantity", DecodeFault::Missing);
    item.insert("quantity".into(), Av::Bool(false));
    assert_decode(
        &item,
        "quantity",
        DecodeFault::WrongType {
            expected: DynamoType::N,
            actual: DynamoType::Bool,
        },
    );
    item.insert("quantity".into(), Av::N("3".into()));
    item.insert("gift.note".into(), Av::Null(true));
    assert_decode(
        &item,
        "gift.note",
        DecodeFault::WrongType {
            expected: DynamoType::S,
            actual: DynamoType::Null,
        },
    );
}

#[test]
fn key_validation_runs_at_construction_encoding_and_decoding() {
    assert!(matches!(OrderKey::new(String::new(), 0), Err(e) if e.kind == EncodeFault::EmptyKey));
    assert!(OrderKey::new("a".repeat(2048), 0).is_ok());
    assert!(
        matches!(OrderKey::new("é".repeat(1025), 0), Err(e) if e.kind == EncodeFault::KeySize { max: 2048, actual: 2050 })
    );
    let mut record = order();
    record.order_ref.clear();
    assert_eq!(record.to_item().unwrap_err().kind, EncodeFault::EmptyKey);
    let key = OrderKey {
        order_ref: String::new(),
        line_no: 0,
    };
    assert_eq!(key.encode_key().unwrap_err().kind, EncodeFault::EmptyKey);
    let mut item = order().to_item().unwrap();
    item.insert("orderRef".into(), Av::S(String::new()));
    assert!(matches!(
        Order::from_item(&item).unwrap_err().kind,
        DecodeFault::KeyFormat(_)
    ));
}

#[derive(tabulary::Record)]
#[tabulary(generated_prefix = "Invoice")]
struct BinaryInvoice {
    #[tabulary(rename = "pk", partition_key)]
    r#type: Vec<u8>,
    #[tabulary(rename = "sk", sort_key)]
    line: String,
}

#[test]
fn binary_partition_and_string_sort_obey_independent_byte_limits() {
    let record = BinaryInvoice {
        r#type: vec![0; 2048],
        line: "a".repeat(1024),
    };
    let item = record.to_item().unwrap();
    assert!(matches!(&item["pk"], Av::B(value) if value.as_ref().len() == 2048));
    let decoded = BinaryInvoice::from_item(&item).unwrap();
    assert_eq!(decoded.to_item().unwrap(), item);
    assert_eq!(InvoiceAttrs::TYPE.name(), "pk");
    assert!(InvoiceKey::new(vec![1; 2049], "a".into()).is_err());
    assert!(InvoiceKey::new(vec![1], "a".repeat(1025)).is_err());
    assert!(InvoiceKey::new(Vec::new(), "a".into()).is_err());
    assert!(InvoiceKey::new(vec![1], String::new()).is_err());
}

#[derive(tabulary::Record)]
#[tabulary(rename_all = "PascalCase")]
struct Session {
    #[tabulary(partition_key)]
    r#type: u64,
    item: String,
    cx: Option<String>,
}

#[test]
fn hash_only_keys_and_raw_identifiers_work_and_unknown_fields_are_denied() {
    assert_eq!(SessionAttrs::TYPE.name(), "Type");
    assert_eq!(Session::SCHEMA.primary.sort, None);
    let session = Session {
        r#type: 7,
        item: String::new(),
        cx: None,
    };
    let mut item = session.to_item().unwrap();
    assert_eq!(session.key().unwrap().encode_key().unwrap().len(), 1);
    assert_eq!(Session::from_item(&item).unwrap().r#type, 7);
    item.insert("z".into(), Av::Bool(false));
    item.insert("a".into(), Av::Bool(false));
    let error = Session::from_item(&item).err().unwrap();
    assert_eq!(error.kind, DecodeFault::UnknownAttribute);
    assert_eq!(error.attribute(), Some("a"));
}

mod expansion_fixture {
    use tabulary::RecordKey;
    struct Order {
        order_ref: String,
        line_no: u64,
        quantity: u64,
    }
    include!("../../tabulary-derive/tests/fixtures/record.expanded.tokens");

    #[test]
    fn checked_expansion_compiles_and_round_trips() {
        let _ = (Order::primary(), OrderIndexes);
        assert!(
            Order::primary()
                .partition("o".into())
                .sort_eq(8)
                .build()
                .is_ok()
        );
        let record = Order {
            order_ref: "order-lime".into(),
            line_no: 8,
            quantity: 3,
        };
        let item = record.to_item().unwrap();
        assert_eq!(Order::from_item(&item).unwrap().to_item().unwrap(), item);
        assert_eq!(record.key().unwrap().encode_key().unwrap().len(), 2);
    }
}
