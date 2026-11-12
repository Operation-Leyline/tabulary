//! Injective built-in composite-key codecs. They do not change a field's stored codec.
use crate::KeyFormatFault as Fault;

/// A component's canonical string representation and strict inverse.
pub trait KeyComponentCodec<T> {
    fn format(value: &T) -> Result<String, Fault>;
    fn parse(value: &str) -> Result<T, Fault>;
}

pub struct Hex;
pub struct Dec;
pub struct Dec20;
pub struct Text;

impl KeyComponentCodec<Vec<u8>> for Hex {
    fn format(value: &Vec<u8>) -> Result<String, Fault> {
        const DIGITS: &[u8] = b"0123456789abcdef";
        Ok(value
            .iter()
            .flat_map(|b| {
                [
                    DIGITS[(b >> 4) as usize] as char,
                    DIGITS[(b & 15) as usize] as char,
                ]
            })
            .collect())
    }
    fn parse(value: &str) -> Result<Vec<u8>, Fault> {
        if !value.len().is_multiple_of(2) {
            return Err(Fault::InvalidHex);
        }
        fn digit(b: u8) -> Result<u8, Fault> {
            match b {
                b'0'..=b'9' => Ok(b - b'0'),
                b'a'..=b'f' => Ok(b - b'a' + 10),
                _ => Err(Fault::InvalidHex),
            }
        }
        value
            .as_bytes()
            .chunks_exact(2)
            .map(|b| Ok(digit(b[0])? * 16 + digit(b[1])?))
            .collect()
    }
}
impl<const N: usize> KeyComponentCodec<[u8; N]> for Hex {
    fn format(value: &[u8; N]) -> Result<String, Fault> {
        <Self as KeyComponentCodec<Vec<u8>>>::format(&value.to_vec())
    }
    fn parse(value: &str) -> Result<[u8; N], Fault> {
        <Self as KeyComponentCodec<Vec<u8>>>::parse(value)?
            .try_into()
            .map_err(|_| Fault::InvalidLength)
    }
}
impl KeyComponentCodec<String> for Hex {
    fn format(value: &String) -> Result<String, Fault> {
        <Self as KeyComponentCodec<Vec<u8>>>::format(&value.as_bytes().to_vec())
    }
    fn parse(value: &str) -> Result<String, Fault> {
        String::from_utf8(<Self as KeyComponentCodec<Vec<u8>>>::parse(value)?)
            .map_err(|_| Fault::InvalidUtf8)
    }
}
macro_rules! integers {
    ($($ty:ty),*) => {$(impl KeyComponentCodec<$ty> for Dec {
        fn format(value: &$ty) -> Result<String, Fault> { Ok(value.to_string()) }
        fn parse(value: &str) -> Result<$ty, Fault> {
            let parsed: $ty = value.parse().map_err(|_| Fault::InvalidNumber)?;
            if parsed.to_string() != value { return Err(Fault::InvalidNumber); }
            Ok(parsed)
        }
    })*};
}
integers!(
    u8, u16, u32, u64, u128, usize, i8, i16, i32, i64, i128, isize
);
impl KeyComponentCodec<u64> for Dec20 {
    fn format(value: &u64) -> Result<String, Fault> {
        Ok(format!("{value:020}"))
    }
    fn parse(value: &str) -> Result<u64, Fault> {
        if value.len() != 20 {
            return Err(Fault::InvalidLength);
        }
        if !value.bytes().all(|b| b.is_ascii_digit()) {
            return Err(Fault::InvalidNumber);
        }
        value.parse().map_err(|_| Fault::InvalidNumber)
    }
}
impl KeyComponentCodec<String> for Text {
    fn format(value: &String) -> Result<String, Fault> {
        Ok(value.clone())
    }
    fn parse(value: &str) -> Result<String, Fault> {
        Ok(value.into())
    }
}

/// Extract a component in a checked run, accounting for the fixed-width tail.
#[doc(hidden)]
pub fn take<'a>(
    input: &mut &'a str,
    width: Option<usize>,
    tail: usize,
    separator: Option<char>,
) -> Result<&'a str, Fault> {
    let size = match width {
        Some(width) => width,
        None => separator
            .and_then(|c| input.find(c))
            .unwrap_or(input.len())
            .checked_sub(tail)
            .ok_or(Fault::InvalidLength)?,
    };
    let value = input.get(..size).ok_or(Fault::InvalidLength)?;
    *input = input.get(size..).ok_or(Fault::InvalidLength)?;
    Ok(value)
}
