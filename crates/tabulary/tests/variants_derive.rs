#![cfg(feature = "derive")]

use aws_sdk_dynamodb::types::AttributeValue as Av;
use tabulary::{DecodeContext, DecodeFault, Document, EncodeContext, Item, Value};

#[derive(Debug, PartialEq, tabulary::Variants)]
#[tabulary(tag = "delivery.stage", rename_all = "snake_case", unknown = "allow")]
enum Stage {
    Waiting,
    Sent {
        tracking_ref: String,
        note: Option<String>,
    },
    Failed {
        failure_code: String,
        note: Option<String>,
    },
    Empty {},
}

#[derive(Debug, PartialEq, tabulary::Document)]
#[tabulary(rename_all = "snake_case")]
struct Envelope {
    stage: Stage,
}

fn decode<T: Document>(item: &Item) -> Result<T, tabulary::DecodeError> {
    T::decode_map(item, &mut DecodeContext::new("Root"))
}
fn encode<T: Document>(value: &T) -> Item {
    value.encode_map(&mut EncodeContext::new("Root")).unwrap()
}

#[test]
fn every_shape_round_trips_as_maps_and_nested_values() {
    #[derive(tabulary::Variants)]
    #[tabulary(tag = "kind", rename_all = "snake_case")]
    enum UnitOnly {
        #[tabulary(rename = "ready")]
        Unit,
    }
    let mut unit = encode(&UnitOnly::Unit);
    assert_eq!(unit["kind"], Av::S("ready".into()));
    unit.insert("future".into(), Av::Bool(true));
    assert_eq!(
        decode::<UnitOnly>(&unit).err().unwrap().kind,
        DecodeFault::UnknownAttribute
    );
    let sent = Stage::Sent {
        tracking_ref: "parcel".into(),
        note: None,
    };
    let failed = Stage::Failed {
        failure_code: "late".into(),
        note: Some("retry".into()),
    };
    for stage in [Stage::Waiting, sent, failed, Stage::Empty {}] {
        let row = stage;
        let item = encode(&row);
        assert!(!item.contains_key("stage"));
        assert!(matches!(item["delivery.stage"], Av::S(_)));
        assert_eq!(decode::<Stage>(&item).unwrap(), row);
        let nested = Envelope { stage: row };
        let value = nested
            .encode_value(&mut EncodeContext::new("Root"))
            .unwrap();
        assert_eq!(
            Envelope::decode_value(&value, &mut DecodeContext::new("Root")).unwrap(),
            nested
        );
    }
}

#[test]
fn discriminator_faults_and_inactive_payload_are_distinct() {
    let mut item = encode(&Stage::Waiting);
    let wrong = DecodeFault::WrongType {
        expected: tabulary::DynamoType::S,
        actual: tabulary::DynamoType::N,
    };
    for (tag, fault) in [
        (None, DecodeFault::Missing),
        (Some(Av::N("1".into())), wrong),
        (Some(Av::S("unknown".into())), DecodeFault::UnknownVariant),
    ] {
        item.remove("delivery.stage");
        if let Some(tag) = tag {
            item.insert("delivery.stage".into(), tag);
        }
        let error = decode::<Stage>(&item).unwrap_err();
        assert_eq!((error.record, error.kind), ("Root", fault));
        assert_eq!(error.path.to_string(), "[\"delivery.stage\"]");
    }
    item.insert("delivery.stage".into(), Av::S("waiting".into()));
    item.insert("tracking_ref".into(), Av::S("stale".into()));
    assert_eq!(
        decode::<Stage>(&item).unwrap_err().kind,
        DecodeFault::InactiveVariantAttribute
    );
    item.remove("tracking_ref");
    item.insert("future".into(), Av::Bool(true));
    assert!(decode::<Stage>(&item).is_ok());
    let standalone = [
        ("delivery.stage".into(), Av::S("waiting".into())),
        ("future".into(), Av::Bool(true)),
    ]
    .into();
    assert!(decode::<Stage>(&standalone).is_ok());
}

#[test]
fn shared_fields_converters_and_defaults_remain_strict() {
    use tabulary as renamed;
    fn initial() -> u64 {
        4
    }
    #[derive(Debug, PartialEq, renamed::Variants)]
    #[tabulary(
        tag = "kind",
        rename_all = "camelCase",
        crate = "renamed",
        unknown = "allow"
    )]
    enum Payload<T> {
        First {
            #[tabulary(with = renamed::convert::u128_string)]
            value: u128,
            #[tabulary(default = initial)]
            count: u64,
            extra: T,
        },
        Second {
            #[tabulary(with = renamed::convert::u128_string)]
            value: u128,
            #[tabulary(default = initial)]
            count: u64,
            extra: T,
        },
    }
    let payload = Payload::First {
        value: u128::MAX,
        count: 4,
        extra: "text".to_owned(),
    };
    let mut item = encode(&payload);
    assert_eq!(item["value"], Av::S(u128::MAX.to_string()));
    item.remove("count");
    assert_eq!(decode::<Payload<String>>(&item).unwrap(), payload);
    item.insert("future".into(), Av::Bool(true));
    assert!(decode::<Payload<String>>(&item).is_ok());
    item.remove("future");
    item.insert("count".into(), Av::S("invalid".into()));
    assert!(matches!(
        decode::<Payload<String>>(&item).unwrap_err().kind,
        DecodeFault::WrongType { .. }
    ));
    assert_eq!(
        encode(&Stage::Waiting)["delivery.stage"],
        StageTags::WAITING
            .encode_value(&mut EncodeContext::new("Root"))
            .unwrap()
    );
}

#[test]
fn nested_paths_and_map_depth_match_documents() {
    let tag = [("delivery.stage".into(), Av::S("sent".into()))].into();
    let item = [("stage".into(), Av::M(tag))].into();
    let error = decode::<Envelope>(&item).unwrap_err();
    assert_eq!(
        (error.record, error.document, error.kind),
        ("Root", "Stage", DecodeFault::Missing)
    );
    assert_eq!(error.path.to_string(), "[\"stage\"][\"tracking_ref\"]");
    let mut ec = EncodeContext::new("Root");
    let mut dc = DecodeContext::new("Root");
    for _ in 0..31 {
        ec = ec.enter_container().unwrap();
        dc = dc.enter_container().unwrap();
    }
    let value = Stage::Waiting.encode_value(&mut ec).unwrap();
    assert_eq!(
        Stage::decode_value(&value, &mut dc).unwrap(),
        Stage::Waiting
    );
    ec = ec.enter_container().unwrap();
    dc = dc.enter_container().unwrap();
    assert_eq!(
        Stage::Waiting.encode_value(&mut ec).unwrap_err().kind,
        tabulary::EncodeFault::DepthLimit
    );
    assert_eq!(
        Stage::decode_value(&value, &mut dc).unwrap_err().kind,
        DecodeFault::DepthLimit
    );
    assert!(Stage::Waiting.encode_map(&mut ec).is_ok());
}

#[test]
fn variants_diagnostics() {
    trybuild::TestCases::new().compile_fail("tests/ui/variants/*.rs");
}

#[test]
fn recursive_generic_payloads_and_caller_default_names_work() {
    #[derive(Debug, PartialEq, tabulary::Variants)]
    #[tabulary(tag = "kind", rename_all = "snake_case")]
    enum Node<T> {
        Leaf { value: T },
        Branch { children: tabulary::List<Node<T>> },
    }
    let node = Node::Branch {
        children: vec![Node::Leaf { value: true }].into(),
    };
    assert_eq!(decode::<Node<bool>>(&encode(&node)).unwrap(), node);
    fn tag() -> u64 {
        1
    }
    fn item() -> u64 {
        2
    }
    fn cx() -> u64 {
        3
    }
    #[derive(Debug, PartialEq, tabulary::Variants)]
    #[tabulary(tag = "kind", rename_all = "snake_case")]
    enum Defaults {
        Named {
            #[tabulary(default = tag)]
            first: u64,
            #[tabulary(default = item)]
            second: u64,
            #[tabulary(default = cx)]
            third: u64,
        },
    }
    let item = [("kind".into(), Av::S("named".into()))].into();
    assert_eq!(
        decode::<Defaults>(&item).unwrap(),
        Defaults::Named {
            first: 1,
            second: 2,
            third: 3
        }
    );
}
