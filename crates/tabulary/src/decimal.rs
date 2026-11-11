//! Exact normalized decimals in DynamoDB's number domain.
use crate::DecodeFault;
use std::{cmp::Ordering, fmt, str::FromStr};

/// An exact decimal; scale and input spelling are not retained.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Decimal {
    negative: bool,
    coefficient: u128,
    exponent: i16,
}

impl FromStr for Decimal {
    type Err = DecodeFault;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let negative = text.starts_with('-');
        let unsigned = text.strip_prefix(['+', '-']).unwrap_or(text);
        let (mantissa, power) = unsigned.split_once(['e', 'E']).unwrap_or((unsigned, "0"));
        let (whole, fraction) = mantissa.split_once('.').unwrap_or((mantissa, ""));
        let power_negative = power.starts_with('-');
        let power_digits = power.strip_prefix(['+', '-']).unwrap_or(power);
        if whole.len() + fraction.len() == 0
            || !whole
                .bytes()
                .chain(fraction.bytes())
                .all(|b| b.is_ascii_digit())
            || power_digits.is_empty()
            || !power_digits.bytes().all(|b| b.is_ascii_digit())
        {
            return Err(DecodeFault::InvalidNumber);
        }
        // Saturation keeps hostile exponents bounded without masking syntax errors.
        let power = power_digits.bytes().fold(0i128, |n, b| {
            n.saturating_mul(10).saturating_add(i128::from(b - b'0'))
        });
        let power = if power_negative { -power } else { power };
        let digits = whole.bytes().chain(fraction.bytes());
        let leading = digits.clone().take_while(|b| *b == b'0').count();
        let count = whole.len() + fraction.len();
        if leading == count {
            return Ok(Self {
                negative: false,
                coefficient: 0,
                exponent: 0,
            });
        }
        let trailing = digits.clone().rev().take_while(|b| *b == b'0').count();
        let significant = count - leading - trailing;
        let exponent = power
            .saturating_sub(fraction.len() as i128)
            .saturating_add(trailing as i128);
        let magnitude = exponent.saturating_add(significant as i128 - 1);
        if significant > 38 || !(-130..=125).contains(&magnitude) {
            return Err(DecodeFault::NumberRange);
        }
        let coefficient = digits
            .skip(leading)
            .take(significant)
            .fold(0, |n, b| n * 10 + u128::from(b - b'0'));
        Ok(Self {
            negative,
            coefficient,
            exponent: exponent as i16,
        })
    }
}

impl Decimal {
    pub(crate) fn integer(self) -> Result<(bool, u128), DecodeFault> {
        if self.exponent < 0 {
            return Err(DecodeFault::InexactNumber);
        }
        let factor = 10u128
            .checked_pow(self.exponent as u32)
            .ok_or(DecodeFault::NumberRange)?;
        let magnitude = self
            .coefficient
            .checked_mul(factor)
            .ok_or(DecodeFault::NumberRange)?;
        Ok((self.negative, magnitude))
    }

    fn magnitude(self) -> (i16, u128) {
        if self.coefficient == 0 {
            return (i16::MIN, 0);
        }
        let digits = self.coefficient.ilog10() + 1;
        (
            self.exponent + digits as i16,
            self.coefficient * 10u128.pow(38 - digits),
        )
    }
}

impl Ord for Decimal {
    fn cmp(&self, other: &Self) -> Ordering {
        match (self.negative, other.negative) {
            (true, false) => Ordering::Less,
            (false, true) => Ordering::Greater,
            (false, false) => self.magnitude().cmp(&other.magnitude()),
            (true, true) => other.magnitude().cmp(&self.magnitude()),
        }
    }
}

impl PartialOrd for Decimal {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl fmt::Display for Decimal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.negative {
            f.write_str("-")?;
        }
        let digits = self.coefficient.to_string();
        let point = digits.len() as i16 + self.exponent;
        if point > 38 || self.exponent < -38 {
            write!(f, "{digits}e{}", self.exponent)
        } else if self.exponent >= 0 {
            write!(f, "{digits:0<width$}", width = point as usize)
        } else if point <= 0 {
            write!(f, "0.{digits:0>width$}", width = -self.exponent as usize)
        } else {
            let (whole, fraction) = digits.split_at(point as usize);
            write!(f, "{whole}.{fraction}")
        }
    }
}
