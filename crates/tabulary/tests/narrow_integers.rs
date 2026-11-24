#![cfg(feature = "derive")]
//! Narrow native integers share the exact `N` codec of `u64` and `i64`.

use aws_sdk_dynamodb::types::AttributeValue as Av;
use tabulary::{
    DecodeContext, DecodeFault, EncodeContext, NumberSet, ReadConsistency, Table, Update, Value,
    wire,
};

#[derive(Debug, PartialEq, tabulary::Record)]
#[tabulary(rename_all = "camelCase")]
struct Shelf {
    #[tabulary(partition_key)]
    aisle: u16,
    #[tabulary(sort_key)]
    slot: i32,
    bin_count: u8,
    shelf_level: u32,
    tilt: i8,
    temperature: i16,
    reserved: Option<u8>,
    sizes: Option<NumberSet<u8>>,
}

#[derive(Debug, PartialEq, tabulary::Record)]
#[tabulary(
    rename_all = "camelCase",
    partition(name = "pk", template = "aisle!{aisle:dec}!{row:dec}"),
    sort(name = "sk", template = "bay!{bay:dec}!{offset:dec}")
)]
struct Bay {
    aisle: u16,
    row: u8,
    bay: u32,
    offset: i16,
}

fn shelf() -> Shelf {
    Shelf {
        aisle: u16::MAX,
        slot: i32::MIN,
        bin_count: u8::MAX,
        shelf_level: u32::MAX,
        tilt: i8::MIN,
        temperature: i16::MAX,
        reserved: Some(0),
        sizes: Some(NumberSet::try_from(vec![1, 2, 3]).unwrap()),
    }
}

fn decode<T: Value>(text: &str) -> Result<T, DecodeFault> {
    T::decode_value(&Av::N(text.into()), &mut DecodeContext::new("Shelf")).map_err(|e| e.kind)
}

fn bounds<T: Value + PartialEq + std::fmt::Debug + ToString + Copy>(values: [T; 3]) {
    for value in values {
        let encoded = value
            .encode_value(&mut EncodeContext::new("Shelf"))
            .unwrap();
        assert_eq!(encoded, Av::N(value.to_string()));
        assert_eq!(decode::<T>(&value.to_string()), Ok(value));
    }
}

#[test]
fn every_narrow_type_round_trips_its_bounds_as_plain_digits() {
    fn number<T: Value<Wire = wire::Number>>() {}
    number::<u8>();
    number::<u16>();
    number::<u32>();
    number::<i8>();
    number::<i16>();
    number::<i32>();
    bounds([u8::MIN, 1, u8::MAX]);
    bounds([u16::MIN, 1, u16::MAX]);
    bounds([u32::MIN, 1, u32::MAX]);
    bounds([i8::MIN, -1, i8::MAX]);
    bounds([i16::MIN, 0, i16::MAX]);
    bounds([i32::MIN, 0, i32::MAX]);
    assert_eq!(decode::<u8>("2.55e2"), Ok(255));
    assert_eq!(decode::<i16>("-3.2768e4"), Ok(i16::MIN));
    assert_eq!(decode::<u32>("-0"), Ok(0));
}

#[test]
fn overflow_negative_and_fractional_values_are_typed_faults() {
    for text in ["256", "1e3", "-1"] {
        assert_eq!(decode::<u8>(text), Err(DecodeFault::NumberRange), "{text}");
    }
    for text in ["65536", "-1"] {
        assert_eq!(decode::<u16>(text), Err(DecodeFault::NumberRange), "{text}");
    }
    for text in ["4294967296", "-1", "1e38"] {
        assert_eq!(decode::<u32>(text), Err(DecodeFault::NumberRange), "{text}");
    }
    for text in ["128", "-129"] {
        assert_eq!(decode::<i8>(text), Err(DecodeFault::NumberRange), "{text}");
    }
    for text in ["32768", "-32769"] {
        assert_eq!(decode::<i16>(text), Err(DecodeFault::NumberRange), "{text}");
    }
    for text in ["2147483648", "-2147483649"] {
        assert_eq!(decode::<i32>(text), Err(DecodeFault::NumberRange), "{text}");
    }
    for text in ["1.5", "-0.5", "1e-1"] {
        assert_eq!(decode::<u8>(text), Err(DecodeFault::InexactNumber));
        assert_eq!(decode::<u16>(text), Err(DecodeFault::InexactNumber));
        assert_eq!(decode::<u32>(text), Err(DecodeFault::InexactNumber));
        assert_eq!(decode::<i8>(text), Err(DecodeFault::InexactNumber));
        assert_eq!(decode::<i16>(text), Err(DecodeFault::InexactNumber));
        assert_eq!(decode::<i32>(text), Err(DecodeFault::InexactNumber));
    }
    assert_eq!(decode::<u8>("x"), Err(DecodeFault::InvalidNumber));
    assert!(matches!(
        u8::decode_value(&Av::S("1".into()), &mut DecodeContext::new("Shelf"))
            .unwrap_err()
            .kind,
        DecodeFault::WrongType { .. }
    ));
}

#[test]
fn derived_records_name_the_record_and_attribute_of_a_narrow_fault() {
    let record = shelf();
    let item = record.to_item().unwrap();
    assert_eq!(item["aisle"], Av::N("65535".into()));
    assert_eq!(item["slot"], Av::N("-2147483648".into()));
    assert_eq!(item["binCount"], Av::N("255".into()));
    assert_eq!(item["tilt"], Av::N("-128".into()));
    assert_eq!(
        item["sizes"],
        Av::Ns(vec!["1".into(), "2".into(), "3".into()])
    );
    assert_eq!(Shelf::from_item(&item).unwrap(), record);
    for (attribute, text, kind) in [
        ("binCount", "256", DecodeFault::NumberRange),
        ("shelfLevel", "-1", DecodeFault::NumberRange),
        ("tilt", "128", DecodeFault::NumberRange),
        ("temperature", "1.25", DecodeFault::InexactNumber),
        ("reserved", "-1", DecodeFault::NumberRange),
        ("aisle", "65536", DecodeFault::NumberRange),
        ("slot", "2147483648", DecodeFault::NumberRange),
    ] {
        let mut item = item.clone();
        item.insert(attribute.into(), Av::N(text.into()));
        let error = Shelf::from_item(&item).unwrap_err();
        assert_eq!(error.record, "Shelf");
        assert_eq!(error.kind, kind, "{attribute}");
        assert_eq!(error.attribute(), Some(attribute));
    }
    let mut item = item;
    item.insert("sizes".into(), Av::Ns(vec!["1".into(), "256".into()]));
    let error = Shelf::from_item(&item).unwrap_err();
    assert_eq!(error.kind, DecodeFault::NumberRange);
    assert_eq!(error.attribute(), Some("sizes"));
}

#[test]
fn narrow_keys_conditions_and_arithmetic_use_the_same_codec() {
    let key = ShelfKey::new(7, -3).unwrap();
    let table = Table::<Shelf>::new("inventory").unwrap();
    let query = table
        .query(
            Shelf::primary().partition(7).sort_between(-3, 9),
            ReadConsistency::Strong,
        )
        .filter(ShelfAttrs::BIN_COUNT.ge(200))
        .filter(ShelfAttrs::TILT.lt(-1))
        .build()
        .unwrap();
    let values = query.expressions().values();
    for value in ["7", "-3", "9", "200", "-1"] {
        assert!(
            values.values().any(|v| *v == Av::N(value.into())),
            "{value}"
        );
    }
    let update = Update::<Shelf>::new()
        .and(ShelfAttrs::BIN_COUNT.add(5))
        .and(ShelfAttrs::SHELF_LEVEL.set_expr(ShelfAttrs::SHELF_LEVEL.operand().plus(2)))
        .and(ShelfAttrs::TEMPERATURE.set(-40))
        .build()
        .unwrap();
    let condition = ShelfAttrs::RESERVED.le(10).build().unwrap();
    let rendered = update.render_with_condition(&condition);
    assert_eq!(
        rendered.update(),
        Some("SET #n0 = #n0 + :v0, #n1 = :v1 ADD #n2 :v2")
    );
    let numbers: Vec<_> = rendered.values().values().cloned().collect();
    for value in ["5", "2", "-40", "10"] {
        assert!(numbers.contains(&Av::N(value.into())), "{value}");
    }
    let prepared = table.get(key, ReadConsistency::Strong).build().unwrap();
    assert_eq!(
        prepared.input().key.as_ref().unwrap()["aisle"],
        Av::N("7".into())
    );
    assert_eq!(
        prepared.input().key.as_ref().unwrap()["slot"],
        Av::N("-3".into())
    );
}

#[test]
fn narrow_integers_are_strict_dec_template_components() {
    let bay = Bay {
        aisle: 12,
        row: 255,
        bay: 0,
        offset: -7,
    };
    let item = bay.to_item().unwrap();
    assert_eq!(item["pk"], Av::S("aisle!12!255".into()));
    assert_eq!(item["sk"], Av::S("bay!0!-7".into()));
    assert_eq!(Bay::from_item(&item).unwrap(), bay);
    for (key, wire) in [
        ("pk", "aisle!12!256"),
        ("pk", "aisle!65536!1"),
        ("pk", "aisle!12!-1"),
        ("sk", "bay!4294967296!0"),
        ("sk", "bay!1!32768"),
    ] {
        let mut item = item.clone();
        item.insert(key.into(), Av::S(wire.into()));
        let error = Bay::from_item(&item).unwrap_err();
        assert!(
            matches!(error.kind, DecodeFault::KeyFormat(_)),
            "{wire}: {:?}",
            error.kind
        );
        assert_eq!(error.attribute(), Some(key));
    }
    let condition = Bay::primary().partition(12, 255).sort_eq((0, -7));
    assert!(condition.build().is_ok());
}
