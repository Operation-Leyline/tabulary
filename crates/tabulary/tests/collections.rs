use std::collections::{BTreeMap, HashMap};

use aws_sdk_dynamodb::{primitives::Blob, types::AttributeValue as Av};
use tabulary::{
    BinarySet, Decimal, DecodeContext, DecodeFault, DynamoType, EncodeContext, EncodeFault, List,
    Map, Nullable, NumberSet, PathSegment, SetFault, StringSet, Value,
};

fn encode<T: Value>(value: &T) -> Result<Av, tabulary::EncodeError> {
    value.encode_value(&mut EncodeContext::new("Order"))
}
fn decode<T: Value>(value: &Av) -> Result<T, tabulary::DecodeError> {
    T::decode_value(value, &mut DecodeContext::new("Order"))
}

#[test]
fn empty_lists_and_maps_survive_and_lists_preserve_order() {
    let list = List::from(Vec::<String>::new());
    assert_eq!(encode(&list).unwrap(), Av::L(vec![]));
    assert_eq!(decode::<List<String>>(&Av::L(vec![])).unwrap(), list);
    let map = Map::from(BTreeMap::<String, String>::new());
    assert_eq!(encode(&map).unwrap(), Av::M(HashMap::new()));
    assert_eq!(decode::<Map<String>>(&Av::M(HashMap::new())).unwrap(), map);
    let list = List::from(vec![3_u64, 1, 2]);
    assert_eq!(decode::<List<u64>>(&encode(&list).unwrap()).unwrap(), list);
    let nullable = List::from(vec![Nullable::Null, Nullable::Value("note".to_owned())]);
    assert_eq!(
        decode::<List<Nullable<String>>>(&encode(&nullable).unwrap()).unwrap(),
        nullable
    );
}

#[test]
fn sets_reject_empty_and_duplicates_at_construction_and_decode() {
    assert_eq!(
        StringSet::try_from(vec![]).unwrap_err().kind,
        EncodeFault::InvalidSet(SetFault::Empty)
    );
    assert_eq!(
        NumberSet::<u64>::try_from(vec![]).unwrap_err().kind,
        EncodeFault::InvalidSet(SetFault::Empty)
    );
    assert_eq!(
        BinarySet::try_from(vec![]).unwrap_err().kind,
        EncodeFault::InvalidSet(SetFault::Empty)
    );
    assert_eq!(
        StringSet::try_from(vec!["x".into(), "x".into()])
            .unwrap_err()
            .kind,
        EncodeFault::InvalidSet(SetFault::Duplicate)
    );
    assert_eq!(
        BinarySet::try_from(vec![vec![0], vec![0]])
            .unwrap_err()
            .kind,
        EncodeFault::InvalidSet(SetFault::Duplicate)
    );
    assert_eq!(
        NumberSet::try_from(vec![1_u64, 1]).unwrap_err().kind,
        EncodeFault::InvalidSet(SetFault::Duplicate)
    );
    assert_eq!(
        decode::<StringSet>(&Av::Ss(vec![])).unwrap_err().kind,
        DecodeFault::InvalidSet(SetFault::Empty)
    );
    assert_eq!(
        decode::<NumberSet<u64>>(&Av::Ns(vec![])).unwrap_err().kind,
        DecodeFault::InvalidSet(SetFault::Empty)
    );
    assert_eq!(
        decode::<BinarySet>(&Av::Bs(vec![])).unwrap_err().kind,
        DecodeFault::InvalidSet(SetFault::Empty)
    );
    assert_eq!(
        decode::<StringSet>(&Av::Ss(vec!["x".into(), "x".into()]))
            .unwrap_err()
            .kind,
        DecodeFault::InvalidSet(SetFault::Duplicate)
    );
    assert_eq!(
        decode::<BinarySet>(&Av::Bs(vec![Blob::new([0]), Blob::new([0])]))
            .unwrap_err()
            .kind,
        DecodeFault::InvalidSet(SetFault::Duplicate)
    );
}

#[test]
fn numeric_duplicates_are_mathematical_and_malformed_members_stay_typed() {
    for spellings in [["1", "1.00", "10e-1"], ["0", "-0.00", "0e99"]] {
        let value = Av::Ns(spellings.into_iter().map(str::to_owned).collect());
        let error = decode::<NumberSet<Decimal>>(&value).err().unwrap();
        assert_eq!(error.kind, DecodeFault::InvalidSet(SetFault::Duplicate));
        assert_eq!(error.path.segments(), &[PathSegment::Index(1)]);
    }
    for (text, kind) in [
        ("bad", DecodeFault::InvalidNumber),
        ("1e126", DecodeFault::NumberRange),
        ("1.5", DecodeFault::InexactNumber),
    ] {
        assert_eq!(
            decode::<NumberSet<u64>>(&Av::Ns(vec![text.into()]))
                .unwrap_err()
                .kind,
            kind
        );
    }
    assert_eq!(
        NumberSet::try_from(vec![u128::MAX]).unwrap_err().kind,
        EncodeFault::NumberRange
    );
}

#[test]
fn canonical_set_encoding_is_independent_of_input_order_and_number_spelling() {
    let strings = StringSet::try_from(vec!["z".into(), "".into(), "a".into()]).unwrap();
    assert_eq!(
        encode(&strings).unwrap(),
        Av::Ss(vec!["".into(), "a".into(), "z".into()])
    );
    assert_eq!(
        encode(&decode::<StringSet>(&encode(&strings).unwrap()).unwrap()).unwrap(),
        encode(&strings).unwrap()
    );
    let binary = BinarySet::try_from(vec![vec![255], vec![], vec![0]]).unwrap();
    assert_eq!(
        encode(&binary).unwrap(),
        Av::Bs(vec![Blob::new([]), Blob::new([0]), Blob::new([255])])
    );
    assert_eq!(
        encode(&decode::<BinarySet>(&encode(&binary).unwrap()).unwrap()).unwrap(),
        encode(&binary).unwrap()
    );
    let numbers =
        decode::<NumberSet<Decimal>>(&Av::Ns(vec!["2.00".into(), "10e-1".into(), "-0".into()]))
            .unwrap();
    assert_eq!(
        encode(&numbers).unwrap(),
        Av::Ns(vec!["0".into(), "1".into(), "2".into()])
    );
    assert_eq!(
        encode(&NumberSet::try_from(vec![2_u64, 1, 0]).unwrap()).unwrap(),
        encode(&numbers).unwrap()
    );
}

#[test]
fn nested_list_and_literal_map_key_paths_are_preserved_on_both_directions() {
    let mut dcx = DecodeContext::new("Order")
        .at(PathSegment::Attribute("lines".into()))
        .in_document("Line");
    let raw = Av::L(vec![Av::M(HashMap::from([(
        "price.raw".into(),
        Av::S("private-payload".into()),
    )]))]);
    let error = List::<Map<u64>>::decode_value(&raw, &mut dcx).unwrap_err();
    assert_eq!(error.record, "Order");
    assert_eq!(error.document, "Line");
    assert_eq!(
        error.path.segments(),
        &[
            PathSegment::Attribute("lines".into()),
            PathSegment::Index(0),
            PathSegment::Attribute("price.raw".into())
        ]
    );
    assert!(!format!("{error:?} {error}").contains("private-payload"));
    let mut ecx = EncodeContext::new("Order")
        .at(PathSegment::Attribute("lines".into()))
        .in_document("Line");
    let data = List::from(vec![Map::from(BTreeMap::from([(
        "price.raw".into(),
        u128::MAX,
    )]))]);
    let error = data.encode_value(&mut ecx).unwrap_err();
    assert_eq!(
        error.path.segments(),
        &[
            PathSegment::Attribute("lines".into()),
            PathSegment::Index(0),
            PathSegment::Attribute("price.raw".into())
        ]
    );
    assert_eq!(error.kind, EncodeFault::NumberRange);
    assert_eq!(dcx.error(DecodeFault::Missing).path.segments().len(), 1);
    assert_eq!(ecx.error(EncodeFault::NumberRange).path.segments().len(), 1);
}

#[test]
fn wrong_tags_are_strict_and_map_failure_order_is_sorted() {
    let raw = Av::Bool(true);
    for (kind, expected) in [
        (
            decode::<List<String>>(&raw).unwrap_err().kind,
            DynamoType::L,
        ),
        (decode::<Map<String>>(&raw).unwrap_err().kind, DynamoType::M),
        (decode::<StringSet>(&raw).unwrap_err().kind, DynamoType::Ss),
        (
            decode::<NumberSet<u64>>(&raw).unwrap_err().kind,
            DynamoType::Ns,
        ),
        (decode::<BinarySet>(&raw).unwrap_err().kind, DynamoType::Bs),
    ] {
        assert_eq!(
            kind,
            DecodeFault::WrongType {
                expected,
                actual: DynamoType::Bool
            }
        );
    }
    let map = Av::M(HashMap::from([
        ("z".into(), raw.clone()),
        ("a".into(), raw),
    ]));
    assert_eq!(
        decode::<Map<String>>(&map).unwrap_err().attribute(),
        Some("a")
    );
}
