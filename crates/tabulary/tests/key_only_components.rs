#![cfg(feature = "derive")]
//! Key-only template components live solely inside their synthesized keys.

use aws_sdk_dynamodb::{operation::query::QueryOutput, types::AttributeValue as Av};
use tabulary::{
    Cursor, DecodeFault, Document, FieldAccess, Item, KeyFormatFault as K, PathSegment,
    ReadConsistency, Record, RecordKey, Table,
};

#[derive(Debug, PartialEq, tabulary::Record)]
#[tabulary(
    rename_all = "camelCase",
    partition(name = "pk", template = "session!{tenant:text}!{session_id:hex}"),
    sort(name = "sk", template = "seq!{sequence:dec20}")
)]
struct Session {
    #[tabulary(key_only)]
    tenant: String,
    #[tabulary(key_only)]
    session_id: [u8; 4],
    sequence: u64,
    device: Option<String>,
}

/// One component repeated in both primary keys must agree on decode.
#[derive(Debug, PartialEq, tabulary::Record)]
#[tabulary(
    rename_all = "snake_case",
    partition(name = "pk", template = "shop!{shop:text}"),
    sort(name = "sk", template = "{shop:text}#{line:dec20}")
)]
struct OrderLine {
    #[tabulary(key_only)]
    shop: String,
    #[tabulary(key_only)]
    line: u64,
    quantity: u32,
}

/// Generic records need no `Value` codec for a key-only component type.
#[derive(Debug, PartialEq, tabulary::Record)]
#[tabulary(
    rename_all = "snake_case",
    partition(name = "pk", template = "bin!{slot:dec}")
)]
struct Bin<T> {
    #[tabulary(key_only)]
    slot: usize,
    payload: T,
}

#[derive(Debug, PartialEq, tabulary::Projection)]
#[tabulary(record = Session)]
struct Device {
    #[tabulary(attribute = SessionAttrs::DEVICE)]
    device: Option<String>,
}

fn session() -> Session {
    Session {
        tenant: "north".into(),
        session_id: [0, 1, 254, 255],
        sequence: 42,
        device: Some("kiosk".into()),
    }
}

fn path(segments: &[PathSegment], attribute: &str, component: &'static str) {
    assert_eq!(
        segments,
        [
            PathSegment::Attribute(attribute.into()),
            PathSegment::Component(component),
        ]
    );
}

#[test]
fn components_are_stored_only_inside_their_keys_and_round_trip() {
    let record = session();
    let item = record.to_item().unwrap();
    assert_eq!(item["pk"], Av::S("session!north!0001feff".into()));
    assert_eq!(item["sk"], Av::S("seq!00000000000000000042".into()));
    assert_eq!(item["sequence"], Av::N("42".into()));
    assert_eq!(item["device"], Av::S("kiosk".into()));
    assert_eq!(item.len(), 4, "no tenant or sessionId attribute");
    assert_eq!(Session::from_item(&item).unwrap(), record);
    assert!(record.billed_size().is_ok());

    let key = record.key().unwrap();
    let encoded = key.encode_key().unwrap();
    assert_eq!(encoded.len(), 2);
    let decoded = SessionKey::decode_key(&encoded).unwrap();
    assert_eq!(decoded.tenant, "north");
    assert_eq!(decoded.session_id, [0, 1, 254, 255]);
    assert_eq!(decoded.sequence, 42);
    let built = SessionKey::new("north".into(), [0, 1, 254, 255], 42).unwrap();
    assert_eq!(built.encode_key().unwrap(), encoded);

    let shared = OrderLine {
        shop: "east".into(),
        line: 7,
        quantity: 3,
    };
    let item = shared.to_item().unwrap();
    assert_eq!(item.len(), 3);
    assert_eq!(item["sk"], Av::S("east#00000000000000000007".into()));
    assert_eq!(OrderLine::from_item(&item).unwrap(), shared);
}

#[test]
fn generic_records_accept_key_only_components_without_value_codecs() {
    let bin = Bin {
        slot: usize::MAX,
        payload: 7u8,
    };
    let item = bin.to_item().unwrap();
    assert_eq!(item["pk"], Av::S(format!("bin!{}", usize::MAX)));
    assert_eq!(item.len(), 2);
    assert_eq!(Bin::<u8>::from_item(&item).unwrap(), bin);
}

#[test]
fn malformed_or_disagreeing_keys_fail_closed_with_typed_paths() {
    let item = session().to_item().unwrap();
    for (attribute, wire, kind, component) in [
        ("pk", "visit!north!0001feff", K::InvalidPrefix, "tenant"),
        ("pk", "session!north!0001FEFF", K::InvalidHex, "session_id"),
        ("pk", "session!north!0001fe", K::InvalidLength, "session_id"),
        (
            "pk",
            "session!north!0001feff00",
            K::InvalidLength,
            "session_id",
        ),
        ("sk", "seq!42", K::InvalidLength, "sequence"),
    ] {
        let mut item = item.clone();
        item.insert(attribute.into(), Av::S(wire.into()));
        let error = Session::from_item(&item).unwrap_err();
        assert_eq!(error.record, "Session");
        assert_eq!(error.kind, DecodeFault::KeyFormat(kind), "{wire}");
        path(error.path.segments(), attribute, component);
        assert!(!format!("{error:?}").contains(wire));
        assert_eq!(
            SessionKey::decode_key(&item).err().unwrap().kind,
            DecodeFault::KeyFormat(kind)
        );
    }
    let mut missing = item.clone();
    missing.remove("pk");
    let error = Session::from_item(&missing).unwrap_err();
    assert_eq!(error.kind, DecodeFault::Missing);
    assert_eq!(error.attribute(), Some("pk"));
    let mut wrong = item.clone();
    wrong.insert("pk".into(), Av::N("1".into()));
    assert!(matches!(
        Session::from_item(&wrong).unwrap_err().kind,
        DecodeFault::WrongType { .. }
    ));

    let mut item = OrderLine {
        shop: "east".into(),
        line: 7,
        quantity: 3,
    }
    .to_item()
    .unwrap();
    item.insert("sk".into(), Av::S("west#00000000000000000007".into()));
    let error = OrderLine::from_item(&item).unwrap_err();
    assert_eq!(error.kind, DecodeFault::KeyMismatch);
    path(error.path.segments(), "sk", "shop");
    assert_eq!(
        OrderLineKey::decode_key(&item).err().unwrap().kind,
        DecodeFault::KeyMismatch
    );
}

#[test]
fn a_stored_copy_of_a_key_only_component_is_an_unknown_attribute() {
    for name in ["tenant", "sessionId"] {
        let mut item = session().to_item().unwrap();
        item.insert(name.into(), Av::S("north".into()));
        let error = Session::from_item(&item).unwrap_err();
        assert_eq!(error.kind, DecodeFault::UnknownAttribute);
        assert_eq!(error.attribute(), Some(name));
    }
}

#[test]
fn schema_marks_key_components_without_a_stored_attribute() {
    let fields = Session::SCHEMA.fields;
    for (position, name) in [(0, "tenant"), (1, "sessionId")] {
        assert_eq!(fields[position].name, name);
        assert_eq!(fields[position].index, position);
        assert_eq!(fields[position].access, FieldAccess::KeyComponent);
        assert_eq!(fields[position].wire, tabulary::DynamoType::S);
    }
    assert_eq!(fields[2].access, FieldAccess::KeyOnly);
    assert_eq!(fields[3].access, FieldAccess::Mutable);
    assert_eq!(<Session as Document>::FIELDS, fields);
    assert_eq!(SessionAttrs::PARTITION.name(), "pk");
    assert_eq!(SessionAttrs::DEVICE.descriptor(), fields[3]);
    let input = Session::table_definition()
        .create_input("sessions", tabulary::Billing::OnDemand)
        .unwrap();
    let mut defined: Vec<_> = input
        .attribute_definitions()
        .iter()
        .map(|a| a.attribute_name())
        .collect();
    defined.sort_unstable();
    assert_eq!(defined, ["pk", "sk"]);
}

#[test]
fn queries_projections_and_cursor_bytes_accept_key_component_schemas() {
    let table = Table::<Session>::new("sessions").unwrap();
    let build = || {
        table
            .query(
                Session::primary().partition("north".into(), [0, 1, 254, 255]),
                ReadConsistency::Strong,
            )
            .select::<Device>()
    };
    let prepared = build().build().unwrap();
    assert_eq!(
        prepared.expressions().values()[":v0"],
        Av::S("session!north!0001feff".into())
    );
    let next: Item = [
        ("pk".into(), Av::S("session!north!0001feff".into())),
        ("sk".into(), Av::S("seq!00000000000000000042".into())),
    ]
    .into();
    let page = prepared
        .decode(Ok(QueryOutput::builder()
            .count(1)
            .scanned_count(1)
            .items([("device".into(), Av::S("kiosk".into()))].into())
            .set_last_evaluated_key(Some(next.clone()))
            .build()))
        .unwrap();
    assert_eq!(
        page.items,
        [Device {
            device: Some("kiosk".into())
        }]
    );
    let bytes = page.next.unwrap().to_bytes().unwrap();
    let cursor = Cursor::<Session, SessionPrimary>::from_bytes(&bytes).unwrap();
    assert_eq!(cursor.last_evaluated_key(), &next);
    assert!(build().resume(cursor).build().is_ok());
}

#[test]
fn invalid_key_only_declarations_are_compile_errors() {
    trybuild::TestCases::new().compile_fail("tests/ui/key_only/*.rs");
}
