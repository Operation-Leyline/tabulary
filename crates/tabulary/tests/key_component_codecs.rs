use tabulary::{KeyComponentCodec as Codec, KeyFormatFault as K, key_component::*};

fn round_trip<T: std::fmt::Debug + PartialEq, C: Codec<T>>(values: &[T]) {
    for value in values {
        assert_eq!(&C::parse(&C::format(value).unwrap()).unwrap(), value);
    }
}

#[test]
fn built_in_domains_have_strict_canonical_inverses() {
    round_trip::<_, Hex>(&[String::new(), "é!包装".into()]);
    round_trip::<_, Hex>(&[vec![], vec![0, 255, 128]]);
    round_trip::<_, Hex>(&[[0u8; 0]]);
    round_trip::<_, Hex>(&[[0, 255]]);
    round_trip::<_, Text>(&[String::new(), "é!包装".into()]);
    round_trip::<_, Dec>(&[0u64, u64::MAX]);
    round_trip::<_, Dec>(&[i64::MIN, -1, 0, i64::MAX]);
    round_trip::<_, Dec>(&[0u128, u128::MAX]);
    round_trip::<_, Dec20>(&[0u64, 7, u64::MAX]);
    for text in ["01", "+1", "-0", " 1", "1e0", "1.0", "18446744073709551616"] {
        assert_eq!(<Dec as Codec<u64>>::parse(text), Err(K::InvalidNumber));
    }
    assert_eq!(<Hex as Codec<String>>::parse("ff"), Err(K::InvalidUtf8));
    assert_eq!(<Hex as Codec<Vec<u8>>>::parse("FF"), Err(K::InvalidHex));
    assert_eq!(<Hex as Codec<[u8; 2]>>::parse("00"), Err(K::InvalidLength));
    assert_eq!(<Dec20 as Codec<u64>>::parse("1"), Err(K::InvalidLength));
}

#[test]
fn component_extraction_checks_utf8_boundaries_and_fixed_tails() {
    let mut rest = "éff界tail";
    assert_eq!(take(&mut rest, None, 2, Some('界')), Ok("é"));
    assert_eq!(take(&mut rest, Some(2), 0, None), Ok("ff"));
    assert_eq!(rest, "界tail");
    for (width, tail) in [(Some(1), 0), (Some(100), 0), (None, 100)] {
        let mut rest = "é";
        assert_eq!(take(&mut rest, width, tail, None), Err(K::InvalidLength));
        assert_eq!(rest, "é");
    }
}
