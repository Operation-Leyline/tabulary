use tabulary::{Decimal, DecodeFault};

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
    for text in ["1e-130", "9.9999999999999999999999999999999999999e125"] {
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
