use aws_sdk_dynamodb::types::AttributeValue;
use tabulary::{
    Decimal, DecodeContext, DecodeFault, DynamoType, EncodeContext, EncodeFault, PathSegment,
    Value, wire,
};

fn encode(value: f64) -> Result<AttributeValue, EncodeFault> {
    value
        .encode_value(&mut EncodeContext::new("Invoice"))
        .map_err(|error| error.kind)
}

fn decode(text: &str) -> Result<f64, DecodeFault> {
    f64::decode_value(
        &AttributeValue::N(text.into()),
        &mut DecodeContext::new("Invoice"),
    )
    .map_err(|error| error.kind)
}

#[test]
fn shortest_decimal_encoding_round_trips_the_original_bits() {
    fn number<T: Value<Wire = wire::Number>>() {}
    number::<f64>();
    for (value, text) in [
        (0.0_f64, "0"),
        (0.1, "0.1"),
        (-12.3, "-12.3"),
        (1.0000000000000002, "1.0000000000000002"),
        (0.30000000000000004, "0.30000000000000004"),
        (9007199254740992.0, "9007199254740992"),
        (1e-130, "1e-130"),
        (9.999999999999998e125, "9999999999999998e110"),
    ] {
        assert_eq!(encode(value), Ok(AttributeValue::N(text.into())));
        assert_eq!(decode(text).unwrap().to_bits(), value.to_bits());
    }
}

#[test]
fn decimal_equivalence_ignores_spelling_without_requiring_binary_exactness() {
    for (text, expected) in [
        (".1", 0.1_f64),
        ("+000.1000e0", 0.1),
        ("1e-1", 0.1),
        ("12.30", 12.3),
        ("123e-1", 12.3),
        ("1.50e2", 150.0),
        ("0e9999999999999999999999999999999999999999", 0.0),
    ] {
        let value = decode(text).unwrap();
        assert_eq!(value.to_bits(), expected.to_bits());
        let AttributeValue::N(encoded) = encode(value).unwrap() else {
            panic!("float must encode as N");
        };
        assert!(text.parse::<Decimal>().unwrap() == encoded.parse::<Decimal>().unwrap());
    }
}

#[test]
fn inexact_reads_reject_decimal_values_that_would_change_on_write() {
    for text in [
        "9007199254740993",
        "0.10000000000000001",
        "1.0000000000000000000000000000000000001",
        "1.0000000000000000000000000000000000001e-130",
        "9.9999999999999999999999999999999999999e125",
    ] {
        for sign in ["", "-"] {
            let text = format!("{sign}{text}");
            assert!(text.parse::<Decimal>().is_ok());
            assert_eq!(decode(&text), Err(DecodeFault::InexactNumber));
        }
    }
}

#[test]
fn nonfinite_values_and_every_negative_zero_spelling_are_rejected() {
    for value in [f64::NAN, -f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert_eq!(encode(value), Err(EncodeFault::NonFiniteFloat));
    }
    assert_eq!(encode(-0.0), Err(EncodeFault::NegativeZero));
    for text in ["-0", "-0.0", "-000e20", "-.0e-9999999999999999999999"] {
        assert_eq!(decode(text), Err(DecodeFault::NumberRange));
    }
    for text in ["NaN", "-NaN", "inf", "-inf", "Infinity"] {
        assert_eq!(decode(text), Err(DecodeFault::InvalidNumber));
    }
}

#[test]
fn underflow_overflow_and_precision_are_checked_before_binary_conversion() {
    for value in [
        f64::from_bits(1),
        f64::MIN_POSITIVE,
        1e-130_f64.next_down(),
        1e126,
        f64::MAX,
    ] {
        assert_eq!(encode(value), Err(EncodeFault::NumberRange));
        assert_eq!(encode(-value), Err(EncodeFault::NumberRange));
    }
    for text in [
        "1e-131",
        "1e126",
        "1e-9999999999999999999999",
        "1e9999999999999999999999",
        "123456789012345678901234567890123456789",
    ] {
        assert_eq!(decode(text), Err(DecodeFault::NumberRange));
        assert_eq!(decode(&format!("-{text}")), Err(DecodeFault::NumberRange));
    }
}

#[test]
fn accepted_service_boundaries_round_trip_on_both_sides_of_zero() {
    for value in [1e-130_f64, 1e-130_f64.next_up(), 1e126_f64.next_down()] {
        for value in [value, -value] {
            let av = encode(value).unwrap();
            let decoded = f64::decode_value(&av, &mut DecodeContext::new("Invoice")).unwrap();
            assert_eq!(decoded.to_bits(), value.to_bits());
        }
    }
}

#[test]
fn float_failures_keep_context_and_never_reveal_values() {
    let path = PathSegment::Attribute("amount".into());
    let mut cx = DecodeContext::new("Invoice").at(path.clone());
    for (av, kind) in [
        (
            AttributeValue::S("private-amount".into()),
            DecodeFault::WrongType {
                expected: DynamoType::N,
                actual: DynamoType::S,
            },
        ),
        (
            AttributeValue::Null(true),
            DecodeFault::WrongType {
                expected: DynamoType::N,
                actual: DynamoType::Null,
            },
        ),
        (
            AttributeValue::N("private-amount".into()),
            DecodeFault::InvalidNumber,
        ),
        (
            AttributeValue::N("9007199254740993".into()),
            DecodeFault::InexactNumber,
        ),
        (AttributeValue::N("1e126".into()), DecodeFault::NumberRange),
    ] {
        let error = f64::decode_value(&av, &mut cx).unwrap_err();
        assert_eq!(error, cx.error(kind));
        assert!(!format!("{error:?} {error}").contains("private-amount"));
    }
    let mut cx = EncodeContext::new("Invoice").at(path);
    for (value, kind) in [
        (f64::INFINITY, EncodeFault::NonFiniteFloat),
        (-0.0, EncodeFault::NegativeZero),
        (1e126, EncodeFault::NumberRange),
    ] {
        assert_eq!(value.encode_value(&mut cx).unwrap_err(), cx.error(kind));
    }
}
