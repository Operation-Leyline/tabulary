//! Local conservative JSON wire bound, independent of billed item bytes.
use aws_sdk_dynamodb::types::AttributeValue as Av;

use crate::{BatchAction, BuildError, Item};

pub(crate) fn check<'a, R: 'a>(
    table: &str,
    actions: impl Iterator<Item = &'a BatchAction<R>>,
) -> Result<(), BuildError> {
    // Reserve root/map/list syntax and both optional accounting fields, even
    // though capacity can only be selected after creating the batch.
    let mut size = add(128, string(table)?)?;
    for action in actions {
        size = add(size, add(32, map(action.item().unwrap_or(action.key()))?)?)?;
    }
    let max = 16 * 1024 * 1024;
    if size > max {
        return Err(BuildError::RequestLimit { max, actual: size });
    }
    Ok(())
}
fn add(a: usize, b: usize) -> Result<usize, BuildError> {
    a.checked_add(b).ok_or(BuildError::SizeOverflow)
}
fn string(s: &str) -> Result<usize, BuildError> {
    s.bytes().try_fold(2, |n, b| {
        add(
            n,
            match b {
                b'\n' | b'\r' | b'\t' | 8 | 12 => 2,
                0..=31 => 6,
                b'"' | b'\\' => 2,
                _ => 1,
            },
        )
    })
}
fn binary(n: usize) -> Result<usize, BuildError> {
    add(
        2,
        add(n, 2)?
            .checked_div(3)
            .and_then(|n| n.checked_mul(4))
            .ok_or(BuildError::SizeOverflow)?,
    )
}
fn list<T>(
    values: &[T],
    size: impl Fn(&T) -> Result<usize, BuildError>,
) -> Result<usize, BuildError> {
    values.iter().try_fold(2, |n, v| add(n, add(1, size(v)?)?))
}
fn map(item: &Item) -> Result<usize, BuildError> {
    item.iter()
        .try_fold(2, |n, (k, v)| add(n, add(2, add(string(k)?, value(v)?)?)?))
}
fn value(v: &Av) -> Result<usize, BuildError> {
    // Eight bytes bound tag/object punctuation for scalar/container tags.
    // BOOL and NULL need one more byte for their four-character tags.
    add(
        8,
        match v {
            Av::S(s) | Av::N(s) => string(s)?,
            Av::B(b) => binary(b.as_ref().len())?,
            Av::Bool(_) | Av::Null(_) => 6,
            Av::Ss(v) | Av::Ns(v) => list(v, |s| string(s))?,
            Av::Bs(v) => list(v, |b| binary(b.as_ref().len()))?,
            Av::L(v) => list(v, value)?,
            Av::M(v) => map(v)?,
            _ => {
                return Err(BuildError::OperationConstraint {
                    code: "unsupported_batch_wire_type",
                });
            }
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn wire_expansion_and_arithmetic_are_separate_from_item_bytes() {
        assert_eq!(string("é\0\"\\").unwrap(), 14);
        assert_eq!(string("\n\r\t\u{8}\u{c}").unwrap(), 12);
        assert_eq!(binary(3).unwrap(), 6);
        assert_eq!(binary(4).unwrap(), 10);
        assert!(value(&Av::Bool(false)).unwrap() >= r#"{"BOOL":false}"#.len());
        assert!(value(&Av::Null(true)).unwrap() >= r#"{"NULL":true}"#.len());
        assert!(matches!(binary(usize::MAX), Err(BuildError::SizeOverflow)));
        assert!(matches!(add(usize::MAX, 1), Err(BuildError::SizeOverflow)));
        let nested = Av::M(
            [(
                "x".into(),
                Av::L(vec![Av::Bs(vec![vec![0; 4].into()]), Av::Bool(true)]),
            )]
            .into(),
        );
        assert!(value(&nested).unwrap() > 30);
    }
}
