use aws_sdk_dynamodb::types::AttributeValue;
use tabulary::{
    Decimal, DecodeContext, DecodeFault, DynamoType, EncodeContext, EncodeFault, PathSegment,
    Value, wire,
};

fn decimal(text: &str) -> Decimal {
    text.parse().unwrap()
}

#[test]
fn decimal_normalizes_spelling_and_zero_without_rounding() {
    for text in ["12.30", "+001.2300e1", "123e-1"] {
        assert!(decimal(text) == decimal("12.3"));
        assert_eq!(decimal(text).to_string(), "123e-1");
    }
    for text in ["0", "-0.000", "+0e9999999999999999999999999999999999999999"] {
        assert!(decimal(text) == decimal("0"));
        assert_eq!(decimal(text).to_string(), "0");
    }
    assert_eq!(decimal(".5").to_string(), "5e-1");
    assert_eq!(decimal("1.").to_string(), "1");
}

#[test]
fn decimal_enforces_service_precision_and_signed_magnitude_boundaries() {
    for text in [
        "1e-130",
        "1.0000000000000000000000000000000000001e-130",
        "9.9999999999999999999999999999999999999e125",
    ] {
        for sign in ["", "-"] {
            let value = decimal(&format!("{sign}{text}"));
            assert!(decimal(&value.to_string()) == value);
        }
    }
    for text in [
        "1e-131",
        "1e126",
        "-1e-131",
        "-1e126",
        "123456789012345678901234567890123456789",
        "1e9999999999999999999999999999999999999999",
        "1e-9999999999999999999999999999999999999999",
    ] {
        assert_eq!(
            text.parse::<Decimal>().err(),
            Some(DecodeFault::NumberRange)
        );
    }
    assert!(decimal(&format!("{}e-1000", "1".to_owned() + &"0".repeat(1000))) == decimal("1"));
    assert!(decimal(&format!("0.{}1e1001", "0".repeat(1000))) == decimal("1"));
}

#[test]
fn malformed_decimal_is_distinct_from_range_failure() {
    for text in [
        "",
        "+",
        "-",
        ".",
        "1e",
        "1e+",
        "1e-",
        "1e1e1",
        "1.2.3",
        "NaN",
        "inf",
        " 1",
        "1\n",
        "1_0",
        "0x10",
        "١",
        "1e999999999999999999999x",
    ] {
        assert_eq!(
            text.parse::<Decimal>().err(),
            Some(DecodeFault::InvalidNumber),
            "{text:?}"
        );
    }
}

#[test]
fn decimal_order_is_mathematical_across_signs_scales_and_zero() {
    let values = [
        "-1e125", "-1000", "-1.2", "-1", "-1e-130", "0", "1e-130", ".01", ".1", "1", "1.01", "10",
        "1e125",
    ];
    for pair in values.windows(2) {
        assert!(decimal(pair[0]) < decimal(pair[1]));
    }
}

fn decode<T: Value>(text: &str) -> Result<T, DecodeFault> {
    T::decode_value(
        &AttributeValue::N(text.into()),
        &mut DecodeContext::new("Invoice"),
    )
    .map_err(|e| e.kind)
}

fn round_trip<T: Value + PartialEq>(value: T) {
    let av = value
        .encode_value(&mut EncodeContext::new("Invoice"))
        .unwrap();
    assert!(matches!(&av, AttributeValue::N(_)));
    assert!(T::decode_value(&av, &mut DecodeContext::new("Invoice")).unwrap() == value);
}

#[test]
fn native_integer_bounds_and_integral_exponent_forms() {
    for value in [0, 1, u64::MAX] {
        round_trip(value);
    }
    for value in [i64::MIN, -1, 0, 1, i64::MAX] {
        round_trip(value);
    }
    assert_eq!(decode::<u64>("184467440737095516150e-1"), Ok(u64::MAX));
    assert_eq!(decode::<i64>("-9.223372036854775808e18"), Ok(i64::MIN));
    for text in ["1.00", "10e-1", "100e-2", "+001e0"] {
        assert_eq!(decode::<u64>(text), Ok(1));
        assert_eq!(decode::<i64>(text), Ok(1));
        assert_eq!(decode::<u128>(text), Ok(1));
    }
    for text in ["18446744073709551616", "-1", "1e125"] {
        assert_eq!(decode::<u64>(text), Err(DecodeFault::NumberRange));
    }
    for text in ["9223372036854775808", "-9223372036854775809"] {
        assert_eq!(decode::<i64>(text), Err(DecodeFault::NumberRange));
    }
    for text in ["1.1", "-0.1", "1e-130"] {
        assert_eq!(decode::<u64>(text), Err(DecodeFault::InexactNumber));
        assert_eq!(decode::<i64>(text), Err(DecodeFault::InexactNumber));
        assert_eq!(decode::<u128>(text), Err(DecodeFault::InexactNumber));
    }
    for text in ["-0", "0.0e125"] {
        assert_eq!(decode::<u64>(text), Ok(0));
        assert_eq!(decode::<i64>(text), Ok(0));
        assert_eq!(decode::<u128>(text), Ok(0));
    }
}

#[test]
fn u128_precision_is_checked_without_rounding() {
    for value in [0u128, 10u128.pow(38) - 1, 10u128.pow(38), u128::MAX - 5] {
        round_trip(value);
    }
    let mut cx = EncodeContext::new("Invoice").at(PathSegment::Attribute("quantity".into()));
    assert_eq!(
        u128::MAX.encode_value(&mut cx).unwrap_err(),
        cx.error(EncodeFault::NumberRange)
    );
    assert_eq!(
        decode::<u128>(&u128::MAX.to_string()),
        Err(DecodeFault::NumberRange)
    );
    for text in ["340282366920938463463374607431768211460", "-1", "1e39"] {
        assert_eq!(decode::<u128>(text), Err(DecodeFault::NumberRange));
    }
}

#[test]
fn numeric_wire_tags_and_fault_context_are_strict() {
    fn number<T: Value<Wire = wire::Number>>() {}
    number::<Decimal>();
    number::<u64>();
    number::<i64>();
    number::<u128>();
    round_trip(decimal("1e-130"));
    assert_eq!(
        decimal("12.30")
            .encode_value(&mut EncodeContext::new("Invoice"))
            .unwrap(),
        AttributeValue::N("123e-1".into())
    );
    let mut cx = DecodeContext::new("Invoice")
        .at(PathSegment::Attribute("lines".into()))
        .at(PathSegment::Index(2))
        .in_document("Line")
        .at(PathSegment::Attribute("quantity".into()));
    for (av, kind) in [
        (
            AttributeValue::S("42".into()),
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
        (AttributeValue::N("1.5".into()), DecodeFault::InexactNumber),
        (AttributeValue::N("1e126".into()), DecodeFault::NumberRange),
        (
            AttributeValue::N("secret-input".into()),
            DecodeFault::InvalidNumber,
        ),
    ] {
        let error = u64::decode_value(&av, &mut cx).unwrap_err();
        assert!(!format!("{error:?} {error}").contains("secret-input"));
        assert_eq!(u64::decode_value(&av, &mut cx).unwrap_err(), cx.error(kind));
        assert_eq!(i64::decode_value(&av, &mut cx).unwrap_err(), cx.error(kind));
        assert_eq!(
            u128::decode_value(&av, &mut cx).unwrap_err(),
            cx.error(kind)
        );
        if kind != DecodeFault::InexactNumber {
            assert_eq!(
                Decimal::decode_value(&av, &mut cx).err(),
                Some(cx.error(kind))
            );
        }
    }
}
