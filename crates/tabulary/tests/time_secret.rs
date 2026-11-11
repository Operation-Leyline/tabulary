use std::time::{Duration, UNIX_EPOCH};

use aws_sdk_dynamodb::types::AttributeValue as Av;
use tabulary::{
    DecodeContext, DecodeFault, DynamoType, EncodeContext, PathSegment, Secret, TimeError,
    TimestampMicros as Micros, TimestampMillis as Millis, TimestampSeconds as Seconds, TtlSeconds,
    Value, wire,
};

fn decode<T: Value>(value: Av) -> Result<T, tabulary::DecodeError> {
    T::decode_value(
        &value,
        &mut DecodeContext::new("Session").at(PathSegment::Attribute("expiresAt".into())),
    )
}

fn round_trip<T: Value<Wire = wire::Number> + PartialEq + std::fmt::Debug>(value: T, text: &str) {
    let encoded = value
        .encode_value(&mut EncodeContext::new("Session"))
        .unwrap();
    assert_eq!(encoded, Av::N(text.into()));
    assert_eq!(decode::<T>(encoded).unwrap(), value);
}

fn fails<T>(value: Result<T, TimeError>, kind: TimeError) {
    assert_eq!(value.err(), Some(kind));
}

#[test]
fn epoch_counts_use_exact_native_numbers_without_unit_guessing() {
    for n in [i64::MIN, -1, 0, 1, i64::MAX] {
        round_trip(Seconds::new(n), &n.to_string());
        round_trip(Millis::new(n), &n.to_string());
        round_trip(Micros::new(n), &n.to_string());
    }
    for n in [0, 1, u64::MAX] {
        round_trip(TtlSeconds::new(n), &n.to_string());
    }
    assert_eq!(decode::<Seconds>(Av::N("1e3".into())).unwrap().get(), 1000);
    assert_eq!(decode::<TtlSeconds>(Av::N("0".into())).unwrap().get(), 0);
}

#[test]
fn unit_conversions_are_exact_in_both_directions_and_check_boundaries() {
    for n in [-123, 0, 123] {
        let seconds = Seconds::new(n);
        let millis = seconds.to_millis().unwrap();
        let micros = seconds.to_micros().unwrap();
        assert_eq!(millis.get(), n * 1000);
        assert_eq!(micros.get(), n * 1_000_000);
        assert_eq!(millis.to_seconds().unwrap(), seconds);
        assert_eq!(micros.to_seconds().unwrap(), seconds);
        assert_eq!(millis.to_micros().unwrap(), micros);
        assert_eq!(micros.to_millis().unwrap(), millis);
    }
    for n in [i64::MIN, i64::MAX] {
        fails(Seconds::new(n).to_millis(), TimeError::Overflow);
        fails(Seconds::new(n).to_micros(), TimeError::Overflow);
        fails(Millis::new(n).to_micros(), TimeError::Overflow);
    }
    for n in [-1001, -1, 1, 1001] {
        fails(Millis::new(n).to_seconds(), TimeError::SubunitPrecision);
        fails(Micros::new(n).to_millis(), TimeError::SubunitPrecision);
        fails(Micros::new(n).to_seconds(), TimeError::SubunitPrecision);
    }
    let boundary = i64::MAX / 1000;
    assert_eq!(
        Seconds::new(boundary).to_millis().unwrap().get(),
        boundary * 1000
    );
    fails(Seconds::new(boundary + 1).to_millis(), TimeError::Overflow);
}

#[test]
fn system_time_round_trips_signed_pre_epoch_and_fractional_units() {
    for time in [
        UNIX_EPOCH,
        UNIX_EPOCH - Duration::from_secs(2),
        UNIX_EPOCH + Duration::from_secs(2),
    ] {
        assert_eq!(
            Seconds::from_system_time(time)
                .unwrap()
                .to_system_time()
                .unwrap(),
            time
        );
    }
    for time in [
        UNIX_EPOCH - Duration::from_millis(1500),
        UNIX_EPOCH + Duration::from_millis(1500),
    ] {
        let millis = Millis::from_system_time(time).unwrap();
        assert_eq!(millis.get(), if time < UNIX_EPOCH { -1500 } else { 1500 });
        assert_eq!(millis.to_system_time().unwrap(), time);
        fails(Seconds::from_system_time(time), TimeError::SubunitPrecision);
    }
    for time in [
        UNIX_EPOCH - Duration::from_micros(1),
        UNIX_EPOCH + Duration::from_micros(1),
    ] {
        assert_eq!(
            Micros::from_system_time(time)
                .unwrap()
                .to_system_time()
                .unwrap(),
            time
        );
        fails(Millis::from_system_time(time), TimeError::SubunitPrecision);
    }
    for time in [
        UNIX_EPOCH - Duration::from_nanos(100),
        UNIX_EPOCH + Duration::from_nanos(100),
    ] {
        assert_ne!(time, UNIX_EPOCH);
        fails(Micros::from_system_time(time), TimeError::SubunitPrecision);
    }
    assert_eq!(
        Seconds::new(i64::MIN).to_system_time(),
        UNIX_EPOCH
            .checked_sub(Duration::from_secs(i64::MIN.unsigned_abs()))
            .ok_or(TimeError::Overflow)
    );
}

#[test]
fn ttl_is_unsigned_whole_seconds_without_expiry_filtering() {
    assert_eq!(TtlSeconds::from_system_time(UNIX_EPOCH).unwrap().get(), 0);
    assert_eq!(
        TtlSeconds::new(2).to_system_time().unwrap(),
        UNIX_EPOCH + Duration::from_secs(2)
    );
    fails(
        TtlSeconds::from_system_time(UNIX_EPOCH - Duration::from_nanos(100)),
        TimeError::BeforeEpoch,
    );
    fails(
        TtlSeconds::from_system_time(UNIX_EPOCH + Duration::from_nanos(100)),
        TimeError::SubunitPrecision,
    );
    assert_eq!(
        TtlSeconds::new(u64::MAX).to_system_time(),
        UNIX_EPOCH
            .checked_add(Duration::from_secs(u64::MAX))
            .ok_or(TimeError::Overflow)
    );
}

#[test]
fn wrappers_preserve_strict_numeric_faults_and_physical_context() {
    fn faults<T: Value>() {
        for (value, kind) in [
            (Av::N("private-input".into()), DecodeFault::InvalidNumber),
            (Av::N("1.5".into()), DecodeFault::InexactNumber),
            (Av::N("1e126".into()), DecodeFault::NumberRange),
            (
                Av::S("1".into()),
                DecodeFault::WrongType {
                    expected: DynamoType::N,
                    actual: DynamoType::S,
                },
            ),
        ] {
            let error = decode::<T>(value).err().unwrap();
            assert_eq!(error.kind, kind);
            assert_eq!(
                (error.record, error.attribute()),
                ("Session", Some("expiresAt"))
            );
            assert!(!format!("{error:?} {error}").contains("private-input"));
        }
    }
    faults::<Seconds>();
    faults::<Millis>();
    faults::<Micros>();
    faults::<TtlSeconds>();
    faults::<Secret<TtlSeconds>>();
    assert!(
        matches!(decode::<TtlSeconds>(Av::N("-1".into())), Err(e) if e.kind == DecodeFault::NumberRange)
    );
}

#[test]
fn secret_debug_is_empty_even_when_inner_debug_is_unavailable() {
    struct Opaque;
    let opaque = Secret::new(Opaque);
    assert_eq!(format!("{opaque:?}"), "");
    assert_eq!(format!("{opaque:#?}"), "");
    assert_eq!(format!("{opaque:>20?}"), "");
    let secret = Secret::new("private-note".to_owned());
    assert_eq!(secret.expose_secret(), "private-note");
    let encoded = secret
        .encode_value(&mut EncodeContext::new("Session"))
        .unwrap();
    assert_eq!(encoded, Av::S("private-note".into()));
    assert_eq!(
        decode::<Secret<String>>(encoded).unwrap().into_inner(),
        "private-note"
    );
    assert_eq!(secret.into_inner(), "private-note");
}
