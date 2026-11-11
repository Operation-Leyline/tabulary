use aws_sdk_dynamodb::types::AttributeValue as Av;
use tabulary::field::{decode_optional, decode_required, encode_optional};
use tabulary::{
    Converter, DecodeContext, DecodeFault, DynamoType, EncodeContext, Native, Nullable,
    PathSegment, Value, convert::u128_string::Codec,
};

type TextCodec = Native<String>;
type NullableTextCodec = Native<Nullable<String>>;

fn decode_cx() -> DecodeContext {
    DecodeContext::new("Order")
        .at(PathSegment::Attribute("lines".into()))
        .at(PathSegment::Index(2))
        .in_document("Line")
        .at(PathSegment::Attribute("externalNumber".into()))
}

fn encode_cx() -> EncodeContext {
    EncodeContext::new("Order").at(PathSegment::Attribute("externalNumber".into()))
}

#[test]
fn optional_absence_is_distinct_from_present_null_wrong_type_and_empty() {
    let cx = &mut decode_cx();
    assert!(
        decode_optional::<String, TextCodec>(None, cx)
            .unwrap()
            .is_none()
    );
    assert_eq!(
        decode_optional::<String, TextCodec>(Some(&Av::S(String::new())), cx).unwrap(),
        Some(String::new())
    );
    for (value, actual) in [
        (Av::Null(true), DynamoType::Null),
        (Av::Bool(false), DynamoType::Bool),
    ] {
        let error = decode_optional::<String, TextCodec>(Some(&value), cx).unwrap_err();
        assert_eq!(
            error.kind,
            DecodeFault::WrongType {
                expected: DynamoType::S,
                actual
            }
        );
        assert_eq!(error.record, "Order");
        assert_eq!(error.document, "Line");
        assert_eq!(error.path.to_string(), r#"["lines"][2]["externalNumber"]"#);
    }
    let cx = &mut encode_cx();
    assert!(
        encode_optional::<String, TextCodec>(&None, cx)
            .unwrap()
            .is_none()
    );
    assert_eq!(
        encode_optional::<String, TextCodec>(&Some(String::new()), cx).unwrap(),
        Some(Av::S(String::new()))
    );
}

#[test]
fn required_values_never_default_and_nullable_still_requires_presence() {
    fn missing<T, C: Converter<T>>() {
        assert!(matches!(decode_required::<T, C>(None, &mut decode_cx()),
            Err(error) if error.kind == DecodeFault::Missing));
    }
    missing::<String, TextCodec>();
    missing::<bool, Native<bool>>();
    missing::<Nullable<String>, NullableTextCodec>();
    missing::<u128, Codec>();
}

#[test]
fn nullable_round_trips_explicit_null_and_value_but_refuses_false_null() {
    for value in [Nullable::Null, Nullable::Value("invoice".to_owned())] {
        let wire = value.encode_value(&mut encode_cx()).unwrap();
        assert_eq!(
            wire,
            match &value {
                Nullable::Null => Av::Null(true),
                Nullable::Value(s) => Av::S(s.clone()),
            }
        );
        assert_eq!(
            Nullable::<String>::decode_value(&wire, &mut decode_cx()).unwrap(),
            value
        );
    }
    let error = Nullable::<String>::decode_value(&Av::Null(false), &mut decode_cx()).unwrap_err();
    assert_eq!(
        error.kind,
        DecodeFault::Custom {
            code: "invalid_null"
        }
    );
    assert_eq!(error.document, "Line");
    assert_eq!(error.path.to_string(), r#"["lines"][2]["externalNumber"]"#);
    assert!(matches!(
        Nullable::<String>::decode_value(&Av::Bool(true), &mut decode_cx())
            .unwrap_err()
            .kind,
        DecodeFault::WrongType {
            expected: DynamoType::S,
            actual: DynamoType::Bool
        }
    ));
}

#[test]
fn optional_nullable_has_three_distinct_states() {
    for value in [
        None,
        Some(Nullable::Null),
        Some(Nullable::Value("paid".to_owned())),
    ] {
        let wire = encode_optional::<_, NullableTextCodec>(&value, &mut encode_cx()).unwrap();
        assert_eq!(
            decode_optional::<_, NullableTextCodec>(wire.as_ref(), &mut decode_cx()).unwrap(),
            value
        );
    }
    assert!(
        decode_optional::<_, NullableTextCodec>(Some(&Av::Null(false)), &mut decode_cx()).is_err()
    );
}

#[test]
fn nested_nullable_refuses_lossy_inner_null_but_preserves_other_states() {
    type Nested = Nullable<Nullable<String>>;
    let inner_null = Nested::Value(Nullable::Null);
    let error = inner_null.encode_value(&mut encode_cx()).unwrap_err();
    assert_eq!(
        error.kind,
        tabulary::EncodeFault::Custom {
            code: "null_inner_value"
        }
    );
    for value in [Nested::Null, Nested::Value(Nullable::Value("paid".into()))] {
        let wire = value.encode_value(&mut encode_cx()).unwrap();
        assert_eq!(
            Nested::decode_value(&wire, &mut decode_cx()).unwrap(),
            value
        );
    }
}

#[test]
fn full_unsigned_range_round_trips_as_strings_and_constants_use_the_same_codec() {
    fn constant<C: Converter<u128>>(value: &u128) -> Av {
        C::encode(value, &mut encode_cx()).unwrap()
    }
    for value in [
        0,
        1,
        u64::MAX as u128,
        10_u128.pow(38),
        u128::MAX - 1,
        u128::MAX,
    ] {
        let encoded = Codec::encode(&value, &mut encode_cx()).unwrap();
        assert_eq!(encoded, Av::S(value.to_string()));
        assert_eq!(Codec::decode(&encoded, &mut decode_cx()).unwrap(), value);
        let field = encode_optional::<_, Codec>(&Some(value), &mut encode_cx())
            .unwrap()
            .unwrap();
        assert_eq!(field, constant::<Codec>(&value));
    }
    fn string_wire<C: Converter<u128, Wire = tabulary::convert::u128_string::Wire>>() {}
    string_wire::<Codec>();
}

#[test]
fn string_converter_rejects_malformed_overflow_and_other_tags_without_values_in_errors() {
    for text in [
        "",
        "+1",
        "-1",
        "01",
        "00",
        " 1",
        "1 ",
        "1.0",
        "1e2",
        "\u{ff11}",
        "private-input",
        "340282366920938463463374607431768211456",
    ] {
        let error = Codec::decode(&Av::S(text.into()), &mut decode_cx()).unwrap_err();
        assert_eq!(
            error.kind,
            DecodeFault::Custom {
                code: "invalid_u128_string"
            }
        );
        assert_eq!((error.record, error.document), ("Order", "Line"));
        assert_eq!(error.path.to_string(), r#"["lines"][2]["externalNumber"]"#);
        if text.len() > 10 {
            assert!(!format!("{error} {error:?}").contains(text));
        }
    }
    for (wire, actual) in [
        (Av::N("1".into()), DynamoType::N),
        (Av::Null(true), DynamoType::Null),
    ] {
        assert_eq!(
            Codec::decode(&wire, &mut decode_cx()).unwrap_err().kind,
            DecodeFault::WrongType {
                expected: DynamoType::S,
                actual
            }
        );
    }
}

#[test]
fn custom_failures_propagate_and_absence_never_invokes_the_converter() {
    struct Reject;
    impl Converter<u128> for Reject {
        type Wire = <Codec as Converter<u128>>::Wire;
        fn encode(_: &u128, cx: &mut EncodeContext) -> Result<Av, tabulary::EncodeError> {
            Err(cx.error(tabulary::EncodeFault::Custom { code: "rejected" }))
        }
        fn decode(_: &Av, cx: &mut DecodeContext) -> Result<u128, tabulary::DecodeError> {
            Err(cx.error(DecodeFault::Custom { code: "rejected" }))
        }
    }
    assert!(
        encode_optional::<_, Reject>(&None, &mut encode_cx())
            .unwrap()
            .is_none()
    );
    assert!(
        decode_optional::<_, Reject>(None, &mut decode_cx())
            .unwrap()
            .is_none()
    );
    let encode = encode_optional::<_, Reject>(&Some(7), &mut encode_cx()).unwrap_err();
    assert_eq!(
        encode.kind,
        tabulary::EncodeFault::Custom { code: "rejected" }
    );
    assert_eq!(encode.attribute(), Some("externalNumber"));
    let decode =
        decode_optional::<_, Reject>(Some(&Av::S("7".into())), &mut decode_cx()).unwrap_err();
    assert_eq!(decode.kind, DecodeFault::Custom { code: "rejected" });
    assert_eq!(decode.path.to_string(), r#"["lines"][2]["externalNumber"]"#);
}
