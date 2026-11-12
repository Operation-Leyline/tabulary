#![cfg(feature = "derive")]

use std::sync::atomic::{AtomicUsize, Ordering};

use aws_sdk_dynamodb::types::AttributeValue as Av;
use tabulary::{DecodeFault, DynamoType, Item};

static CALLS: AtomicUsize = AtomicUsize::new(0);
fn item() -> u64 {
    CALLS.fetch_add(1, Ordering::SeqCst);
    7
}
fn cx() -> Option<String> {
    CALLS.fetch_add(1, Ordering::SeqCst);
    Some("pending".into())
}

#[derive(Debug, tabulary::Record)]
#[tabulary(rename_all = "camelCase", unknown = "allow")]
struct Order {
    #[tabulary(partition_key)]
    order_ref: String,
    #[tabulary(rename = "item.count", default = item)]
    quantity: u64,
    #[tabulary(default = cx)]
    gift_note: Option<String>,
}

#[test]
fn defaults_run_only_on_absence_and_never_on_encode_or_corruption() {
    let mut item = Item::from([("orderRef".into(), Av::S("order-lime".into()))]);
    CALLS.store(0, Ordering::SeqCst);
    let decoded = Order::from_item(&item).unwrap();
    assert_eq!(decoded.quantity, 7);
    assert_eq!(decoded.gift_note.as_deref(), Some("pending"));
    assert_eq!(CALLS.load(Ordering::SeqCst), 2);
    let encoded = decoded.to_item().unwrap();
    assert_eq!(encoded["item.count"], Av::N("7".into()));
    assert_eq!(CALLS.load(Ordering::SeqCst), 2);
    item.insert("item.count".into(), Av::N("0".into()));
    item.insert("giftNote".into(), Av::S(String::new()));
    assert_eq!(Order::from_item(&item).unwrap().quantity, 0);
    assert_eq!(
        Order::from_item(&item).unwrap().gift_note.as_deref(),
        Some("")
    );
    for (value, fault) in [
        (
            Av::Bool(false),
            DecodeFault::WrongType {
                expected: DynamoType::N,
                actual: DynamoType::Bool,
            },
        ),
        (
            Av::Null(true),
            DecodeFault::WrongType {
                expected: DynamoType::N,
                actual: DynamoType::Null,
            },
        ),
        (
            Av::Null(false),
            DecodeFault::WrongType {
                expected: DynamoType::N,
                actual: DynamoType::Null,
            },
        ),
        (Av::N("bad".into()), DecodeFault::InvalidNumber),
        (Av::N("1.5".into()), DecodeFault::InexactNumber),
        (
            Av::N("18446744073709551616".into()),
            DecodeFault::NumberRange,
        ),
    ] {
        item.insert("item.count".into(), value);
        let error = Order::from_item(&item).unwrap_err();
        assert_eq!(error.record, "Order");
        assert_eq!(error.document, "Order");
        assert_eq!(error.attribute(), Some("item.count"));
        assert_eq!(error.kind, fault);
    }
    item.insert("item.count".into(), Av::N("3".into()));
    item.insert("giftNote".into(), Av::Null(true));
    let error = Order::from_item(&item).unwrap_err();
    assert_eq!(error.attribute(), Some("giftNote"));
    assert_eq!(
        error.kind,
        DecodeFault::WrongType {
            expected: DynamoType::S,
            actual: DynamoType::Null
        }
    );
    assert_eq!(CALLS.load(Ordering::SeqCst), 2);
}

#[derive(tabulary::Record)]
#[tabulary(rename_all = "snake_case")]
struct Session {
    #[tabulary(partition_key)]
    id: String,
    #[tabulary(default = String::new)]
    note: String,
}

#[test]
fn qualified_default_paths_produce_the_declared_field_type() {
    let item = Item::from([("id".into(), Av::S("session-lime".into()))]);
    let decoded = Session::from_item(&item).unwrap();
    assert_eq!(decoded.to_item().unwrap()["note"], Av::S(String::new()));
}

#[derive(Debug, tabulary::Record)]
#[tabulary(rename_all = "camelCase", unknown = "deny")]
struct Invoice {
    #[tabulary(partition_key)]
    order_ref: String,
    #[tabulary(rename = "item.count")]
    quantity: u64,
    gift_note: Option<String>,
}

#[test]
fn unknown_policy_preserves_known_field_validation_and_drops_allowed_extras() {
    let mut item = Item::from([
        ("orderRef".into(), Av::S("order-lime".into())),
        ("item.count".into(), Av::N("3".into())),
        ("giftNote".into(), Av::S("packed".into())),
        ("z.extra".into(), Av::Bool(true)),
        ("a.extra".into(), Av::Null(false)),
    ]);
    let decoded = Order::from_item(&item).unwrap();
    let encoded = decoded.to_item().unwrap();
    assert_eq!(encoded.len(), 3);
    assert!(!encoded.contains_key("a.extra"));
    assert!(!encoded.contains_key("z.extra"));
    assert_eq!(
        Invoice::from_item(&encoded).unwrap().to_item().unwrap(),
        encoded
    );
    let error = Invoice::from_item(&item).unwrap_err();
    assert_eq!(error.kind, DecodeFault::UnknownAttribute);
    assert_eq!(error.record, "Invoice");
    assert_eq!(error.attribute(), Some("a.extra"));
    for attribute in ["orderRef", "item.count", "giftNote"] {
        let value = item.insert(attribute.into(), Av::Bool(false)).unwrap();
        for error in [
            Order::from_item(&item).unwrap_err(),
            Invoice::from_item(&item).unwrap_err(),
        ] {
            assert_eq!(error.attribute(), Some(attribute));
            assert!(matches!(error.kind, DecodeFault::WrongType { .. }));
        }
        item.insert(attribute.into(), value);
    }
    item.remove("item.count");
    let error = Invoice::from_item(&item).unwrap_err();
    assert_eq!(error.attribute(), Some("item.count"));
    assert_eq!(error.kind, DecodeFault::Missing);
}
