#![cfg(feature = "derive")]
use aws_sdk_dynamodb::types::AttributeValue as Av;
use tabulary::{DecodeContext, DecodeFault, Document, EncodeContext, List, Value};

#[derive(Debug, PartialEq, tabulary::Document)]
#[tabulary(rename_all = "camelCase")]
struct Line<T> {
    unit_price: T,
    note: Option<String>,
}

#[derive(Debug, PartialEq, tabulary::Record)]
#[tabulary(rename_all = "snake_case")]
struct Order<T> {
    #[tabulary(partition_key)]
    id: String,
    lines: List<Line<T>>,
    bytes: Vec<u8>,
}

#[test]
fn nested_generic_records_round_trip_without_binary_list_ambiguity() {
    let order = Order {
        id: "order-lime".into(),
        lines: List(vec![Line {
            unit_price: 4u64,
            note: None,
        }]),
        bytes: vec![1, 2],
    };
    let item = order.to_item().unwrap();
    assert!(matches!(item["bytes"], Av::B(_)));
    assert!(matches!(item["lines"], Av::L(_)));
    assert_eq!(Order::from_item(&item).unwrap(), order);
    let nested = order
        .encode_value(&mut EncodeContext::new("Envelope"))
        .unwrap();
    assert_eq!(
        Order::decode_value(&nested, &mut DecodeContext::new("Envelope")).unwrap(),
        order
    );
    let Av::M(mut nested) = nested else {
        panic!("record must encode as M")
    };
    nested.insert("id".into(), Av::S(String::new()));
    let error = Order::<u64>::decode_value(&Av::M(nested), &mut DecodeContext::new("Envelope"))
        .unwrap_err();
    assert_eq!((error.record, error.document), ("Envelope", "Order"));
    assert_eq!(
        error.kind,
        DecodeFault::KeyFormat(tabulary::KeyFormatFault::InvalidLength)
    );
}

#[test]
fn nested_paths_retain_outer_record_document_and_indexes() {
    let item = tabulary::Item::from([
        ("id".into(), Av::S("order-lime".into())),
        ("bytes".into(), Av::B(vec![1].into())),
        (
            "lines".into(),
            Av::L(vec![Av::M(tabulary::Item::from([(
                "unitPrice".into(),
                Av::Bool(false),
            )]))]),
        ),
    ]);
    let error = Order::<u64>::from_item(&item).unwrap_err();
    assert_eq!((error.record, error.document), ("Order", "Line"));
    assert_eq!(
        error.path.segments(),
        &[
            tabulary::PathSegment::Attribute("lines".into()),
            tabulary::PathSegment::Index(0),
            tabulary::PathSegment::Attribute("unitPrice".into()),
        ]
    );
    assert_eq!(
        error.kind,
        DecodeFault::WrongType {
            expected: tabulary::DynamoType::N,
            actual: tabulary::DynamoType::Bool
        }
    );
    let mut missing = item;
    missing.remove("lines");
    let error = Order::<u64>::from_item(&missing).unwrap_err();
    assert_eq!(error.document, "Order");
    assert_eq!(
        error.path.segments(),
        &[tabulary::PathSegment::Attribute("lines".into())]
    );
    assert_eq!(error.kind, DecodeFault::Missing);
    let invalid = Order {
        id: "order-lime".into(),
        lines: List(vec![Line {
            unit_price: f64::NAN,
            note: None,
        }]),
        bytes: vec![],
    };
    let error = invalid.to_item().unwrap_err();
    assert_eq!((error.record, error.document), ("Order", "Line"));
    assert_eq!(error.path.segments().len(), 3);
    assert_eq!(error.kind, tabulary::EncodeFault::NonFiniteFloat);
}

#[derive(Debug, PartialEq, tabulary::Document)]
#[tabulary(rename_all = "snake_case", unknown = "allow")]
struct Invoice {
    #[tabulary(rename = "line.count", default = initial_count)]
    count: u64,
    note: Option<String>,
}
fn initial_count() -> u64 {
    7
}

#[test]
fn documents_keep_explicit_defaults_and_unknown_policy() {
    let mut item = tabulary::Item::from([("future".into(), Av::Bool(true))]);
    let value = Invoice::decode_map(&item, &mut DecodeContext::new("Envelope")).unwrap();
    assert_eq!(
        value,
        Invoice {
            count: 7,
            note: None
        }
    );
    assert_eq!(
        value
            .encode_map(&mut EncodeContext::new("Envelope"))
            .unwrap()
            .len(),
        1
    );
    item.insert("line.count".into(), Av::Null(true));
    let error = Invoice::decode_map(&item, &mut DecodeContext::new("Envelope")).unwrap_err();
    assert_eq!((error.record, error.document), ("Envelope", "Invoice"));
    assert_eq!(error.attribute(), Some("line.count"));
    assert!(matches!(error.kind, DecodeFault::WrongType { .. }));
    let mut item = tabulary::Item::from([
        ("unitPrice".into(), Av::N("1".into())),
        ("z".into(), Av::Bool(true)),
        ("a".into(), Av::Bool(true)),
    ]);
    let error = Line::<u64>::decode_map(&item, &mut DecodeContext::new("Order")).unwrap_err();
    assert_eq!(error.kind, DecodeFault::UnknownAttribute);
    assert_eq!(error.attribute(), Some("a"));
    item.insert("note".into(), Av::Null(true));
    assert!(matches!(
        Line::<u64>::decode_map(&item, &mut DecodeContext::new("Order"))
            .unwrap_err()
            .kind,
        DecodeFault::WrongType { .. }
    ));
}

#[derive(Debug, PartialEq, tabulary::Document)]
#[tabulary(rename_all = "snake_case")]
struct Node {
    children: List<Node>,
}
fn chain(depth: usize) -> Node {
    Node {
        children: List(if depth == 0 {
            vec![]
        } else {
            vec![chain(depth - 1)]
        }),
    }
}
#[test]
fn maximum_nesting_counts_maps_and_lists_but_not_the_top_level_item() {
    // Each edge is a list plus a nested document; the terminal empty list counts too.
    let node = chain(15);
    let item = node.encode_map(&mut EncodeContext::new("Node")).unwrap(); // 31
    assert_eq!(
        Node::decode_map(&item, &mut DecodeContext::new("Node")).unwrap(),
        node
    );
    let value = node
        .encode_value(&mut EncodeContext::new("Envelope"))
        .unwrap(); // 32
    assert_eq!(
        Node::decode_value(&value, &mut DecodeContext::new("Envelope")).unwrap(),
        node
    );
    let value = Av::L(vec![value]); // 33
    let error =
        List::<Node>::decode_value(&value, &mut DecodeContext::new("Envelope")).unwrap_err();
    assert_eq!(error.kind, DecodeFault::DepthLimit);
    assert_eq!(error.document, "Node");
    let error = List(vec![node])
        .encode_value(&mut EncodeContext::new("Envelope"))
        .unwrap_err();
    assert_eq!(error.kind, tabulary::EncodeFault::DepthLimit);
    assert_eq!(error.path.segments().len(), 32);
    assert_eq!(
        chain(16)
            .encode_map(&mut EncodeContext::new("Node"))
            .unwrap_err()
            .kind,
        tabulary::EncodeFault::DepthLimit
    );
}

#[test]
fn generic_keys_handles_const_parameters_and_nonclone_payloads_work() {
    use tabulary::RecordKey;
    #[derive(tabulary::StringEnum)]
    enum Payload {
        #[tabulary(rename = "sent")]
        Sent,
    }
    #[derive(tabulary::Record)]
    #[tabulary(rename_all = "snake_case")]
    struct Session<K, T, const N: usize>
    where
        K: PartialEq,
    {
        #[tabulary(partition_key)]
        id: K,
        payload: T,
        bytes: [u8; N],
    }
    let record = Session {
        id: 9u64,
        payload: Payload::Sent,
        bytes: [1u8; 3],
    };
    let item = record.to_item().unwrap();
    let key = record.key().unwrap().clone();
    fn copy<T: Copy>(value: T) -> T {
        value
    }
    let _ = copy(Session::<u64, Payload, 3>::primary());
    assert_eq!(key.encode_key().unwrap()["id"], Av::N("9".into()));
    assert_eq!(
        Session::<u64, Payload, 3>::from_item(&item).unwrap().bytes,
        [1, 1, 1]
    );
    assert_eq!(
        SessionAttrs::<u64, Payload, 3>::PAYLOAD
            .validate_mutation()
            .unwrap()
            .index,
        1
    );
    assert_eq!(
        <Session<u64, Payload, 3> as tabulary::Record>::SCHEMA
            .fields
            .len(),
        3
    );
    assert_eq!(LineAttrs::<u64>::UNIT_PRICE.name(), "unitPrice");
    assert_eq!(LineAttrs::<u64>::NOTE.name(), "note");
}

#[test]
fn invalid_declarations_are_rejected() {
    let tests = trybuild::TestCases::new();
    tests.compile_fail("tests/ui/document/*.rs");
    tests.pass("tests/ui/document_renamed.rs");
}
