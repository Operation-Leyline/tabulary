use crate::{Decimal, DecodeFault, EncodeContext, EncodeError, EncodeFault};
use aws_sdk_dynamodb::types::AttributeValue;

fn encode_integer(
    value: impl ToString,
    cx: &mut EncodeContext,
) -> Result<AttributeValue, EncodeError> {
    let number: Decimal = value
        .to_string()
        .parse()
        .map_err(|_| cx.error(EncodeFault::NumberRange))?;
    Ok(AttributeValue::N(number.to_string()))
}

fn integer<T: TryFrom<u128> + TryFrom<i128>>(number: Decimal) -> Result<T, DecodeFault> {
    let (negative, magnitude) = number.integer()?;
    if negative {
        i128::try_from(magnitude)
            .ok()
            .and_then(|n| T::try_from(-n).ok())
    } else {
        T::try_from(magnitude).ok()
    }
    .ok_or(DecodeFault::NumberRange)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_integer_bounds_and_exponent_forms() {
        for value in [0, 1, u64::MAX] {
            assert_eq!(
                integer::<u64>(value.to_string().parse().unwrap()),
                Ok(value)
            );
        }
        for value in [i64::MIN, -1, 0, 1, i64::MAX] {
            assert_eq!(
                integer::<i64>(value.to_string().parse().unwrap()),
                Ok(value)
            );
        }
        assert_eq!(
            integer::<u64>("184467440737095516150e-1".parse().unwrap()),
            Ok(u64::MAX)
        );
        assert_eq!(
            integer::<i64>("-9.223372036854775808e18".parse().unwrap()),
            Ok(i64::MIN)
        );
        for text in ["18446744073709551616", "-1", "1e125"] {
            assert_eq!(
                integer::<u64>(text.parse().unwrap()),
                Err(DecodeFault::NumberRange)
            );
        }
        for text in ["9223372036854775808", "-9223372036854775809"] {
            assert_eq!(
                integer::<i64>(text.parse().unwrap()),
                Err(DecodeFault::NumberRange)
            );
        }
        for text in ["1.1", "-0.1", "1e-130"] {
            assert_eq!(
                integer::<u64>(text.parse().unwrap()),
                Err(DecodeFault::InexactNumber)
            );
        }
    }

    #[test]
    fn u128_precision_is_checked_without_rounding() {
        let mut cx = EncodeContext::new("Invoice");
        for value in [0, 10u128.pow(38), u128::MAX - 5] {
            let AttributeValue::N(text) = encode_integer(value, &mut cx).unwrap() else {
                panic!("expected N")
            };
            assert_eq!(integer::<u128>(text.parse().unwrap()), Ok(value));
        }
        assert_eq!(
            encode_integer(u128::MAX, &mut cx).unwrap_err().kind,
            EncodeFault::NumberRange
        );
        assert_eq!(
            u128::MAX.to_string().parse::<Decimal>().err(),
            Some(DecodeFault::NumberRange)
        );
        assert_eq!(
            integer::<u128>("340282366920938463463374607431768211460".parse().unwrap()),
            Err(DecodeFault::NumberRange)
        );
    }
}
