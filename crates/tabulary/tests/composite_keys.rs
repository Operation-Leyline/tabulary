#![cfg(feature = "derive")]

use aws_sdk_dynamodb::types::AttributeValue as Av;
use tabulary::{DecodeFault, EncodeFault, KeyFormatFault as K, PathSegment, Record, RecordKey};

#[derive(Debug, PartialEq, tabulary::Record)]
#[tabulary(
    rename_all = "camelCase",
    partition(name = "pk", template = "parcel!{shop:hex}!{token:hex}"),
    sort(
        name = "sk",
        template = "line!{number:dec20}!{delta:dec}!{label:text}界end"
    )
)]
struct Parcel {
    shop: String,
    token: [u8; 2],
    number: u64,
    delta: i64,
    label: String,
    note: Option<String>,
}

fn parcel() -> Parcel {
    Parcel {
        shop: "é!".into(),
        token: [0, 255],
        number: u64::MAX,
        delta: i64::MIN,
        label: "包装".into(),
        note: None,
    }
}

fn path(segments: &[PathSegment], attribute: &str, component: &'static str) {
    let expected = [
        PathSegment::Attribute(attribute.into()),
        PathSegment::Component(component),
    ];
    assert_eq!(segments, expected);
}

fn key_round_trip<R: Record, K: RecordKey<R>>(key: K) -> tabulary::Item {
    let item = key.encode_key().unwrap();
    assert_eq!(K::decode_key(&item).unwrap().encode_key().unwrap(), item);
    item
}

fn fault(item: &tabulary::Item) -> DecodeFault {
    Parcel::from_item(item).unwrap_err().kind
}

#[test]
fn exact_format_and_inverse_preserve_utf8_fixed_bytes_and_integer_bounds() {
    let record = parcel();
    let item = record.to_item().unwrap();
    assert_eq!(item["pk"], Av::S("parcel!c3a921!00ff".into()));
    assert_eq!(
        item["sk"],
        Av::S("line!18446744073709551615!-9223372036854775808!包装界end".into())
    );
    assert_eq!(item.len(), 7);
    assert_eq!(Parcel::from_item(&item).unwrap(), record);
    let key = key_round_trip(record.key().unwrap());
    assert_eq!(key.len(), 2);
    let p = record;
    let built = ParcelKey::new(p.shop, p.token, p.number, p.delta, p.label).unwrap();
    assert_eq!(built.encode_key().unwrap(), key);
    assert_eq!(ParcelAttrs::PARTITION.name(), "pk");
    assert_eq!(
        Parcel::SCHEMA.fields[Parcel::SCHEMA.primary.partition].name,
        "pk"
    );
}

#[test]
fn corrupt_keys_are_typed_and_located_at_physical_key_and_component() {
    for (wire, kind, component) in [
        ("wrong!c3a921!00ff", K::InvalidPrefix, "shop"),
        ("parcel!C3a921!00ff", K::InvalidHex, "shop"),
        ("parcel!ff!00ff", K::InvalidUtf8, "shop"),
        ("parcel!abc!00ff", K::InvalidHex, "shop"),
        ("parcel!c3a921!00", K::InvalidLength, "token"),
        ("parcel!c3a921!00ffextra", K::InvalidLength, "token"),
    ] {
        let mut item = parcel().to_item().unwrap();
        item.insert("pk".into(), Av::S(wire.into()));
        let error = Parcel::from_item(&item).unwrap_err();
        assert_eq!(error.record, "Parcel");
        assert_eq!(error.kind, DecodeFault::KeyFormat(kind), "{wire}");
        path(error.path.segments(), "pk", component);
        assert!(!format!("{error:?}").contains(wire));
    }
    for wire in [
        "line!00000000000000000001!01!x界end",
        "line!18446744073709551616!0!x界end",
        "line!0000000000000000001!0!x界end",
        "line!00000000000000000001!-0!x界end",
        "line!00000000000000000001!+1!x界end",
    ] {
        let mut item = parcel().to_item().unwrap();
        item.insert("sk".into(), Av::S(wire.into()));
        assert!(matches!(fault(&item), DecodeFault::KeyFormat(_)));
    }
}

#[test]
fn sources_are_required_and_must_agree_with_every_parsed_component() {
    let mut item = parcel().to_item().unwrap();
    item.insert("shop".into(), Av::S("other".into()));
    let error = Parcel::from_item(&item).unwrap_err();
    assert_eq!(error.kind, DecodeFault::KeyMismatch);
    path(error.path.segments(), "pk", "shop");
    item.remove("shop");
    let error = Parcel::from_item(&item).unwrap_err();
    assert_eq!(error.kind, DecodeFault::Missing);
    assert_eq!(error.attribute(), Some("shop"));
    let mut item = parcel().to_item().unwrap();
    item.remove("pk");
    assert_eq!(fault(&item), DecodeFault::Missing);
    item.insert("pk".into(), Av::Bool(false));
    assert!(matches!(fault(&item), DecodeFault::WrongType { .. }));
}

#[test]
fn text_reserves_the_first_separator_character_and_key_size_is_checked() {
    let mut record = parcel();
    record.label = "x界end".into();
    let error = record.to_item().unwrap_err();
    assert_eq!(error.kind, EncodeFault::KeyFormat(K::InvalidSeparator));
    path(error.path.segments(), "sk", "label");
    assert!(record.key().is_err());
    record.label = "x".repeat(1024);
    assert!(matches!(
        record.to_item().unwrap_err().kind,
        EncodeFault::KeySize { max: 1024, .. }
    ));
}

#[test]
fn query_prefixes_only_accept_complete_leading_components() {
    let partition = Parcel::primary()
        .partition("\u{e9}!".into(), [0, 255])
        .unwrap();
    let first = Parcel::primary().sort_prefix((7,)).unwrap();
    let second = Parcel::primary().sort_prefix((7, -2)).unwrap();
    assert_eq!(partition.as_str(), "parcel!c3a921!00ff");
    assert_eq!(first.as_str(), "line!00000000000000000007!");
    assert_eq!(second.as_str(), "line!00000000000000000007!-2!");
    assert_eq!(format!("{first:?}"), "ParcelSortPrefix");
}

#[derive(Debug, PartialEq, tabulary::Record)]
#[tabulary(
    rename_all = "snake_case",
    partition(name = "pk", template = "{{{output:text}{component:hex}}}"),
    sort(name = "sk", template = "{component:hex}{cx:dec20}")
)]
struct Runs {
    output: String,
    component: [u8; 1],
    cx: u64,
}

#[test]
fn adjacent_fixed_and_variable_runs_and_shared_sources_round_trip() {
    for text in ["", "é", "a0", "包装"] {
        let record = Runs {
            output: text.into(),
            component: [255],
            cx: 7,
        };
        let item = record.to_item().unwrap();
        let prefix = Runs::primary().partition(text.into(), [255]).unwrap();
        assert_eq!(prefix.as_str(), item["pk"].as_s().unwrap());
        assert_eq!(Runs::from_item(&item).unwrap(), record);
        key_round_trip(record.key().unwrap());
    }
}

#[derive(Debug, PartialEq, tabulary::Record)]
#[tabulary(partition(name = "pk", template = "constant"))]
struct Constant {}

#[test]
fn literal_only_templates_are_valid_and_checked() {
    let record = Constant {};
    let mut item = record.to_item().unwrap();
    assert_eq!(Constant::from_item(&item).unwrap(), record);
    assert_eq!(
        key_round_trip(ConstantKey::new().unwrap())["pk"],
        Av::S("constant".into())
    );
    item.insert("pk".into(), Av::S("other".into()));
    assert!(matches!(
        Constant::from_item(&item).unwrap_err().kind,
        DecodeFault::KeyFormat(K::InvalidPrefix)
    ));
}

#[derive(Debug, PartialEq, tabulary::Record)]
#[tabulary(
    rename_all = "snake_case",
    sort(name = "sk", template = "{id:hex}!{count:dec}")
)]
struct Invoice {
    #[tabulary(partition_key)]
    id: String,
    count: u64,
}

#[derive(Debug, PartialEq, tabulary::Record)]
#[tabulary(
    rename_all = "snake_case",
    partition(name = "pk", template = "{id:hex}")
)]
struct Line {
    id: String,
    #[tabulary(sort_key)]
    number: u64,
}

#[test]
fn mixed_field_and_template_keys_deduplicate_inputs_in_logical_order() {
    let record = Invoice {
        id: "a".into(),
        count: 0,
    };
    let mut item = record.to_item().unwrap();
    assert_eq!(Invoice::from_item(&item).unwrap(), record);
    let built = InvoiceKey::new("a".into(), 0).unwrap();
    assert_eq!(
        built.encode_key().unwrap(),
        record.key().unwrap().encode_key().unwrap()
    );
    item.insert("id".into(), Av::S("b".into()));
    assert_eq!(
        Invoice::from_item(&item).unwrap_err().kind,
        DecodeFault::KeyMismatch
    );
    assert_eq!(
        InvoiceKey::decode_key(&item).err().unwrap().kind,
        DecodeFault::KeyMismatch
    );
    let record = Line {
        id: "a".into(),
        number: 7,
    };
    assert_eq!(
        LineKey::new("a".into(), 7).unwrap().encode_key().unwrap(),
        record.key().unwrap().encode_key().unwrap()
    );
    assert_eq!(Line::from_item(&record.to_item().unwrap()).unwrap(), record);
}

extern crate tabulary as storage;
#[derive(storage::Record)]
#[tabulary(
    crate = "storage",
    rename_all = "snake_case",
    partition(name = "pk", template = "{id:hex}"),
    sort(name = "sk", template = "{id:text}{number:dec20}!{token:hex}")
)]
struct Generic<T> {
    id: T,
    number: u64,
    token: Vec<u8>,
}

#[test]
fn renamed_generic_records_keep_key_sources_and_nested_validation() {
    use tabulary::{DecodeContext, EncodeContext, Value};
    let record = Generic {
        id: "é".to_owned(),
        number: 7,
        token: vec![255],
    };
    let value = record
        .encode_value(&mut EncodeContext::new("Envelope"))
        .unwrap();
    let decoded =
        Generic::<String>::decode_value(&value, &mut DecodeContext::new("Envelope")).unwrap();
    assert_eq!(decoded.to_item().unwrap(), record.to_item().unwrap());
    key_round_trip(record.key().unwrap().clone());
    let prefix = Generic::<String>::primary()
        .sort_prefix(("é".to_owned(), 7))
        .unwrap();
    assert_eq!(prefix.as_str(), "é00000000000000000007!");
}
