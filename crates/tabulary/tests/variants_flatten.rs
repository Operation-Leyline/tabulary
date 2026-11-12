#![cfg(feature = "derive")]

use aws_sdk_dynamodb::types::AttributeValue as Av;
use tabulary::{DecodeContext, DecodeFault, Document, EncodeContext, Item};

#[derive(Debug, PartialEq, tabulary::Variants)]
#[tabulary(tag = "delivery.stage", rename_all = "snake_case")]
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

#[derive(Debug, PartialEq, tabulary::Record)]
#[tabulary(rename_all = "snake_case", unknown = "allow")]
struct Delivery {
    #[tabulary(partition_key)]
    id: String,
    #[tabulary(flatten)]
    stage: Stage,
    quantity: u64,
}

fn decode<T: Document>(item: &Item) -> Result<T, tabulary::DecodeError> {
    T::decode_map(item, &mut DecodeContext::new("Root"))
}
fn encode<T: Document>(value: &T) -> Item {
    value.encode_map(&mut EncodeContext::new("Root")).unwrap()
}
fn delivery(stage: Stage) -> Delivery {
    Delivery {
        id: "order".into(),
        stage,
        quantity: 1,
    }
}

#[test]
fn every_shape_round_trips_flattened() {
    #[derive(tabulary::Variants)]
    #[tabulary(tag = "kind", rename_all = "snake_case")]
    enum UnitOnly {
        Unit,
    }
    assert_eq!(encode(&UnitOnly::Unit)["kind"], Av::S("unit".into()));
    let sent = Stage::Sent {
        tracking_ref: "parcel".into(),
        note: None,
    };
    let failed = Stage::Failed {
        failure_code: "late".into(),
        note: Some("retry".into()),
    };
    for stage in [Stage::Waiting, sent, failed, Stage::Empty {}] {
        let row = delivery(stage);
        let item = row.to_item().unwrap();
        assert!(!item.contains_key("stage"));
        assert!(matches!(item["delivery.stage"], Av::S(_)));
        assert_eq!(Delivery::from_item(&item).unwrap(), row);
    }
}

#[test]
fn discriminator_faults_and_inactive_payload_are_distinct() {
    let mut item = delivery(Stage::Waiting).to_item().unwrap();
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
        let error = Delivery::from_item(&item).unwrap_err();
        assert_eq!((error.record, error.kind), ("Delivery", fault));
        assert_eq!(error.path.to_string(), "[\"delivery.stage\"]");
    }
    item.insert("delivery.stage".into(), Av::S("waiting".into()));
    item.insert("tracking_ref".into(), Av::S("stale".into()));
    assert_eq!(
        Delivery::from_item(&item).unwrap_err().kind,
        DecodeFault::InactiveVariantAttribute
    );
    item.remove("tracking_ref");
    item.insert("future".into(), Av::Bool(true));
    assert!(Delivery::from_item(&item).is_ok());
    let standalone = [
        ("delivery.stage".into(), Av::S("waiting".into())),
        ("future".into(), Av::Bool(true)),
    ]
    .into();
    assert_eq!(
        decode::<Stage>(&standalone).unwrap_err().kind,
        DecodeFault::UnknownAttribute
    );
}

#[test]
fn shared_fields_converters_defaults_and_decode_order_remain_strict() {
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
    #[derive(Debug, PartialEq, tabulary::Document)]
    #[tabulary(rename_all = "snake_case")]
    struct Flat<T> {
        #[tabulary(flatten)]
        payload: Payload<T>,
        common: u64,
    }
    let payload = Payload::First {
        value: u128::MAX,
        count: 4,
        extra: "text".to_owned(),
    };
    let row = Flat { payload, common: 1 };
    let mut item = encode(&row);
    assert_eq!(item["value"], Av::S(u128::MAX.to_string()));
    item.remove("count");
    assert_eq!(decode::<Flat<String>>(&item).unwrap(), row);
    item.insert("future".into(), Av::Bool(true));
    assert_eq!(
        decode::<Flat<String>>(&item).unwrap_err().kind,
        DecodeFault::UnknownAttribute
    );
    assert!(decode::<Payload<String>>(&item).is_ok());
    item.remove("future");
    item.insert("count".into(), Av::S("invalid".into()));
    assert!(matches!(
        decode::<Flat<String>>(&item).unwrap_err().kind,
        DecodeFault::WrongType { .. }
    ));
    item.remove("kind");
    item.remove("common");
    assert_eq!(
        decode::<Flat<String>>(&item).unwrap_err().attribute(),
        Some("common")
    );
    DeliveryAttrs::STAGE.is(StageTags::WAITING).build().unwrap();
    assert_eq!(DeliveryAttrs::STAGE.name(), "delivery.stage");
}
