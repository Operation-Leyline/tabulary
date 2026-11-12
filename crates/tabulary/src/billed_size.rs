//! Local item-size models; remote billing equivalence is unverified.
use std::collections::{BTreeMap, HashMap};

use aws_sdk_dynamodb::types::AttributeValue;

use crate::{BuildError, Decimal, EncodeContext, EncodeError, EncodeFault, PathSegment};

/// Estimated and conservative item bytes, excluding JSON/base64 and storage overhead.
///
/// Numbers use an approximate significant-digit estimate and a conservative
/// 21-byte reservation. Neither count is a service-certified billing result.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BilledSize {
    estimated: usize,
    conservative: usize,
}

impl BilledSize {
    /// Size an encoded item by borrowing its attributes, without cloning it.
    ///
    /// The top-level item has no map wrapper or per-attribute overhead. Only
    /// present attributes count, including any physical/synthetic key names.
    /// Errors use `Item` as their record/document name and retain physical paths.
    /// This is a size model, not general schema or set-uniqueness validation.
    pub fn of_item(item: &HashMap<String, AttributeValue>) -> Result<Self, EncodeError> {
        map_size(item, &EncodeContext::new("Item"), 0, false)
    }

    pub fn estimated_item_bytes(&self) -> usize {
        self.estimated
    }

    pub fn conservative_item_bytes(&self) -> usize {
        self.conservative
    }

    /// Accept equality; reject using the conservative count, even if the estimate fits.
    /// The caller supplies the bound (for example, `400 * 1024` for an item).
    pub fn ensure_at_most(&self, max_bytes: usize) -> Result<(), BuildError> {
        if self.conservative > max_bytes {
            Err(BuildError::RequestLimit {
                max: max_bytes,
                actual: self.conservative,
            })
        } else {
            Ok(())
        }
    }

    fn exact(bytes: usize) -> Self {
        Self {
            estimated: bytes,
            conservative: bytes,
        }
    }

    fn add(self, other: Self, cx: &EncodeContext) -> Result<Self, EncodeError> {
        Ok(Self {
            estimated: self
                .estimated
                .checked_add(other.estimated)
                .ok_or_else(|| cx.error(EncodeFault::SizeOverflow))?,
            conservative: self
                .conservative
                .checked_add(other.conservative)
                .ok_or_else(|| cx.error(EncodeFault::SizeOverflow))?,
        })
    }
}

fn map_size(
    item: &HashMap<String, AttributeValue>,
    cx: &EncodeContext,
    depth: usize,
    nested: bool,
) -> Result<BilledSize, EncodeError> {
    let mut total = BilledSize::exact(if nested { 3 } else { 0 });
    // Sort borrowed names so diagnostics do not depend on HashMap iteration order.
    for (name, value) in item.iter().collect::<BTreeMap<_, _>>() {
        let child = cx.at(PathSegment::Attribute(name.clone()));
        total = total.add(BilledSize::exact(name.len()), &child)?;
        if nested {
            total = total.add(BilledSize::exact(1), &child)?;
        }
        total = total.add(value_size(value, &child, depth)?, &child)?;
    }
    Ok(total)
}

fn number_size(text: &str, cx: &EncodeContext) -> Result<BilledSize, EncodeError> {
    // Reuse the native codec's exact domain validation, never f64 or rounded digits.
    text.parse::<Decimal>()
        .map_err(|_| cx.error(EncodeFault::NumberRange))?;
    let unsigned = text.strip_prefix(['+', '-']).unwrap_or(text);
    let mantissa = unsigned.split(['e', 'E']).next().unwrap_or(unsigned);
    let digits = mantissa.bytes().filter(u8::is_ascii_digit);
    let count = digits.clone().count();
    let leading = digits.clone().take_while(|digit| *digit == b'0').count();
    let significant = if leading == count {
        0
    } else {
        count - leading - digits.rev().take_while(|digit| *digit == b'0').count()
    };
    let negative = usize::from(significant != 0 && text.starts_with('-'));
    Ok(BilledSize {
        estimated: significant.max(1).div_ceil(2) + 1 + negative,
        conservative: 21,
    })
}

fn value_size(
    value: &AttributeValue,
    cx: &EncodeContext,
    depth: usize,
) -> Result<BilledSize, EncodeError> {
    if matches!(value, AttributeValue::L(_) | AttributeValue::M(_)) && depth >= 32 {
        return Err(cx.error(EncodeFault::DepthLimit));
    }
    match value {
        AttributeValue::S(text) => Ok(BilledSize::exact(text.len())),
        AttributeValue::B(blob) => Ok(BilledSize::exact(blob.as_ref().len())),
        AttributeValue::Bool(_) | AttributeValue::Null(true) => Ok(BilledSize::exact(1)),
        AttributeValue::Null(false) => Err(cx.error(EncodeFault::Custom {
            code: "invalid_null",
        })),
        AttributeValue::N(text) => number_size(text, cx),
        AttributeValue::M(item) => map_size(item, cx, depth + 1, true),
        AttributeValue::L(values) => {
            let mut total = BilledSize::exact(3);
            for (index, value) in values.iter().enumerate() {
                let child = cx.at(PathSegment::Index(index));
                total = total.add(BilledSize::exact(1), &child)?;
                total = total.add(value_size(value, &child, depth + 1)?, &child)?;
            }
            Ok(total)
        }
        AttributeValue::Ss(values) => values
            .iter()
            .try_fold(BilledSize::exact(0), |total, value| {
                total.add(BilledSize::exact(value.len()), cx)
            }),
        AttributeValue::Bs(values) => values
            .iter()
            .try_fold(BilledSize::exact(0), |total, value| {
                total.add(BilledSize::exact(value.as_ref().len()), cx)
            }),
        AttributeValue::Ns(values) => {
            values
                .iter()
                .enumerate()
                .try_fold(BilledSize::exact(0), |total, (index, value)| {
                    let child = cx.at(PathSegment::Index(index));
                    total.add(number_size(value, &child)?, &child)
                })
        }
        _ => Err(cx.error(EncodeFault::Custom {
            code: "unsupported_type",
        })),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checked_addition_refuses_overflow_in_either_count() {
        let cx = EncodeContext::new("Item");
        for size in [
            BilledSize {
                estimated: usize::MAX,
                conservative: 0,
            },
            BilledSize {
                estimated: 0,
                conservative: usize::MAX,
            },
        ] {
            assert_eq!(
                size.add(BilledSize::exact(1), &cx).unwrap_err().kind,
                EncodeFault::SizeOverflow
            );
        }
        let maximum = BilledSize::exact(usize::MAX);
        assert_eq!(maximum.add(BilledSize::exact(0), &cx), Ok(maximum));
    }
}
