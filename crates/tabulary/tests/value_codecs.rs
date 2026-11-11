use std::{collections::HashMap, fmt::Debug};

use aws_sdk_dynamodb::{primitives::Blob, types::AttributeValue};
use tabulary::{
    DecodeContext, DecodeFault, DynamoType, EncodeContext, PathSegment, Value, WireType, wire,
};

fn round_trip<T: Value + Debug + PartialEq>(value: T, expected: AttributeValue) {
    let encoded = value
        .encode_value(&mut EncodeContext::new("Order"))
        .unwrap();
    assert_eq!(encoded, expected);
    assert_eq!(
        T::decode_value(&expected, &mut DecodeContext::new("Order")).unwrap(),
        value
    );
}

#[test]
fn string_and_bool_use_exact_tags_without_coercion() {
    round_trip(
        "invoice 🦀\0".to_owned(),
        AttributeValue::S("invoice 🦀\0".to_owned()),
    );
    round_trip(true, AttributeValue::Bool(true));
    round_trip(false, AttributeValue::Bool(false));
}

#[test]
fn vectors_and_arrays_are_raw_binary_not_lists_or_base64() {
    let bytes = [0, 1, 127, 128, 255];
    round_trip(bytes.to_vec(), AttributeValue::B(Blob::new(bytes)));
    round_trip(bytes, AttributeValue::B(Blob::new(bytes)));
}

#[test]
fn empty_non_key_values_are_retained() {
    round_trip(String::new(), AttributeValue::S(String::new()));
    round_trip(Vec::<u8>::new(), AttributeValue::B(Blob::new([])));
    round_trip([0_u8; 0], AttributeValue::B(Blob::new([])));
}

fn rejects<T: Value + Debug>(value: &AttributeValue, kind: DecodeFault) {
    let mut cx = DecodeContext::new("Order")
        .at(PathSegment::Attribute("lines".into()))
        .at(PathSegment::Index(2))
        .in_document("Line")
        .at(PathSegment::Attribute("payload.raw".into()));
    let expected = cx.error(kind);
    let error = T::decode_value(value, &mut cx).unwrap_err();
    assert_eq!(error, expected);
    assert!(!format!("{error:?} {error}").contains("private-payload"));
}

fn rejects_wrong_tag<T: Value + Debug>(value: &AttributeValue, actual: DynamoType) {
    rejects::<T>(
        value,
        DecodeFault::WrongType {
            expected: <T::Wire as WireType>::DYNAMO_TYPE,
            actual,
        },
    );
}

#[test]
fn every_known_wrong_tag_is_a_contextual_typed_error() {
    let values = [
        (AttributeValue::S("private-payload".into()), DynamoType::S),
        (AttributeValue::N("1".into()), DynamoType::N),
        (
            AttributeValue::B(Blob::new(b"private-payload")),
            DynamoType::B,
        ),
        (AttributeValue::Bool(false), DynamoType::Bool),
        (AttributeValue::Null(true), DynamoType::Null),
        (AttributeValue::Null(false), DynamoType::Null),
        (AttributeValue::L(vec![]), DynamoType::L),
        (AttributeValue::M(HashMap::new()), DynamoType::M),
        (
            AttributeValue::Ss(vec!["private-payload".into()]),
            DynamoType::Ss,
        ),
        (AttributeValue::Ns(vec!["1".into()]), DynamoType::Ns),
        (
            AttributeValue::Bs(vec![Blob::new(b"private-payload")]),
            DynamoType::Bs,
        ),
    ];
    for (value, actual) in values {
        if actual != DynamoType::S {
            rejects_wrong_tag::<String>(&value, actual);
        }
        if actual != DynamoType::Bool {
            rejects_wrong_tag::<bool>(&value, actual);
        }
        if actual != DynamoType::B {
            rejects_wrong_tag::<Vec<u8>>(&value, actual);
            rejects_wrong_tag::<[u8; 3]>(&value, actual);
        }
    }
}

#[test]
fn fixed_arrays_reject_short_and_long_binary_by_raw_length() {
    for actual in [0, 2, 4] {
        rejects::<[u8; 3]>(
            &AttributeValue::B(Blob::new(vec![255; actual])),
            DecodeFault::InvalidLength {
                expected: 3,
                actual,
            },
        );
    }
    rejects::<[u8; 0]>(
        &AttributeValue::B(Blob::new([0])),
        DecodeFault::InvalidLength {
            expected: 0,
            actual: 1,
        },
    );
}

#[test]
fn future_sdk_types_are_unsupported_for_every_scalar_codec() {
    use aws_sdk_dynamodb::operation::get_item::{GetItem, GetItemOutput};
    use aws_smithy_runtime_api::client::{
        orchestrator::HttpResponse,
        runtime_plugin::RuntimePlugin,
        ser_de::{DeserializeResponse, SharedResponseDeserializer},
    };
    use aws_smithy_types::{body::SdkBody, config_bag::ConfigBag};

    // Unknown is non-constructible outside the SDK. Deserialize a future tag
    // through its public runtime plugin, without a client or any network I/O.
    let mut config = ConfigBag::base();
    config.push_shared_layer(GetItem::new().config().unwrap());
    let response = HttpResponse::new(
        200.try_into().unwrap(),
        SdkBody::from(r#"{"Item":{"payload":{"FUTURE":"private-payload"}}}"#),
    );
    let output = config
        .load::<SharedResponseDeserializer>()
        .unwrap()
        .deserialize_nonstreaming_with_config(&response, &config)
        .unwrap()
        .downcast::<GetItemOutput>()
        .unwrap();
    let value = &output.item().unwrap()["payload"];
    assert!(value.is_unknown());
    rejects::<String>(value, DecodeFault::UnsupportedType);
    rejects::<bool>(value, DecodeFault::UnsupportedType);
    rejects::<Vec<u8>>(value, DecodeFault::UnsupportedType);
    rejects::<[u8; 3]>(value, DecodeFault::UnsupportedType);
}

#[test]
fn wire_markers_identify_all_sdk_categories() {
    assert_eq!(wire::String::DYNAMO_TYPE, DynamoType::S);
    assert_eq!(wire::Number::DYNAMO_TYPE, DynamoType::N);
    assert_eq!(wire::Binary::DYNAMO_TYPE, DynamoType::B);
    assert_eq!(wire::Bool::DYNAMO_TYPE, DynamoType::Bool);
    assert_eq!(wire::Null::DYNAMO_TYPE, DynamoType::Null);
    assert_eq!(wire::List::DYNAMO_TYPE, DynamoType::L);
    assert_eq!(wire::Map::DYNAMO_TYPE, DynamoType::M);
    assert_eq!(wire::StringSet::DYNAMO_TYPE, DynamoType::Ss);
    assert_eq!(wire::NumberSet::DYNAMO_TYPE, DynamoType::Ns);
    assert_eq!(wire::BinarySet::DYNAMO_TYPE, DynamoType::Bs);
}
