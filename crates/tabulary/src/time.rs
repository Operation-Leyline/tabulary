//! Explicit epoch units with exact, checked conversions and no clock access.
use std::{
    fmt,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use crate::{DecodeContext, DecodeError, EncodeContext, EncodeError, Value, wire};
use aws_sdk_dynamodb::types::AttributeValue;

/// A time conversion failed without rounding or saturating its input.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum TimeError {
    Overflow,
    SubunitPrecision,
    BeforeEpoch,
}

impl fmt::Display for TimeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Overflow => "time conversion overflow",
            Self::SubunitPrecision => "time conversion would lose precision",
            Self::BeforeEpoch => "TTL time precedes the Unix epoch",
        })
    }
}

impl std::error::Error for TimeError {}

macro_rules! epoch_value {
    ($name:ident, $count:ty, $doc:literal) => {
        #[doc = $doc]
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name($count);

        impl $name {
            pub const fn new(count: $count) -> Self {
                Self(count)
            }
            /// The explicit epoch count in this wrapper's unit.
            pub const fn get(self) -> $count {
                self.0
            }
        }

        impl Value for $name {
            type Wire = wire::Number;
            fn encode_value(&self, cx: &mut EncodeContext) -> Result<AttributeValue, EncodeError> {
                self.0.encode_value(cx)
            }
            fn decode_value(
                value: &AttributeValue,
                cx: &mut DecodeContext,
            ) -> Result<Self, DecodeError> {
                <$count>::decode_value(value, cx).map(Self)
            }
        }
    };
}

fn exact_count(nanos: i128, unit: i128) -> Result<i64, TimeError> {
    if nanos % unit != 0 {
        return Err(TimeError::SubunitPrecision);
    }
    i64::try_from(nanos / unit).map_err(|_| TimeError::Overflow)
}

macro_rules! timestamp {
    ($name:ident, $unit:expr, $doc:literal) => {
        epoch_value!($name, i64, $doc);
        impl $name {
            /// Convert exactly; reject subunit precision and count overflow.
            pub fn from_system_time(time: SystemTime) -> Result<Self, TimeError> {
                let nanos = match time.duration_since(UNIX_EPOCH) {
                    Ok(duration) => duration.as_nanos() as i128,
                    Err(error) => -(error.duration().as_nanos() as i128),
                };
                exact_count(nanos, $unit).map(Self)
            }
            /// Reject times outside the platform's SystemTime range.
            pub fn to_system_time(self) -> Result<SystemTime, TimeError> {
                let nanos = u128::from(self.0.unsigned_abs()) * $unit as u128;
                let duration = Duration::new(
                    (nanos / 1_000_000_000) as u64,
                    (nanos % 1_000_000_000) as u32,
                );
                if self.0 < 0 {
                    UNIX_EPOCH.checked_sub(duration)
                } else {
                    UNIX_EPOCH.checked_add(duration)
                }
                .ok_or(TimeError::Overflow)
            }
            pub fn to_seconds(self) -> Result<TimestampSeconds, TimeError> {
                exact_count(i128::from(self.0) * $unit, 1_000_000_000).map(TimestampSeconds)
            }
            pub fn to_millis(self) -> Result<TimestampMillis, TimeError> {
                exact_count(i128::from(self.0) * $unit, 1_000_000).map(TimestampMillis)
            }
            pub fn to_micros(self) -> Result<TimestampMicros, TimeError> {
                exact_count(i128::from(self.0) * $unit, 1000).map(TimestampMicros)
            }
        }
    };
}

timestamp!(
    TimestampSeconds,
    1_000_000_000,
    "Signed whole seconds since the Unix epoch."
);
timestamp!(
    TimestampMillis,
    1_000_000,
    "Signed whole milliseconds since the Unix epoch."
);
timestamp!(
    TimestampMicros,
    1000,
    "Signed whole microseconds since the Unix epoch."
);
epoch_value!(
    TtlSeconds,
    u64,
    "Unsigned whole epoch seconds; decoding does not filter expired values."
);

impl TtlSeconds {
    /// Reject pre-epoch times before checking whole-second precision.
    pub fn from_system_time(time: SystemTime) -> Result<Self, TimeError> {
        let duration = time
            .duration_since(UNIX_EPOCH)
            .map_err(|_| TimeError::BeforeEpoch)?;
        if duration.subsec_nanos() != 0 {
            return Err(TimeError::SubunitPrecision);
        }
        Ok(Self(duration.as_secs()))
    }
    pub fn to_system_time(self) -> Result<SystemTime, TimeError> {
        UNIX_EPOCH
            .checked_add(Duration::from_secs(self.0))
            .ok_or(TimeError::Overflow)
    }
}
