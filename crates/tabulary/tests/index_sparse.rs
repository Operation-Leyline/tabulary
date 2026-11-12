#![cfg(feature = "derive")]
use aws_sdk_dynamodb::types::AttributeValue as Av;
use tabulary::{DecodeFault, DynamoType, EncodeFault, KeyFormatFault, PathSegment};

#[derive(Debug, PartialEq, tabulary::Record)]
#[tabulary(rename_all = "camelCase",
    index(ident = ByCourier, name = "by_courier", kind = global, sparse,
        partition(name = "gpk", template = "courier!{courier:hex}"),
        sort(name = "gsk", template = "{sequence:dec20}!{label:text}!end"), project = all))]
struct Parcel {
    #[tabulary(partition_key)]
    id: String,
    courier: Option<String>,
    sequence: Option<u64>,
    label: String,
    common_note: String,
}
fn parcel() -> Parcel {
    Parcel {
        id: "order".into(),
        courier: Some("é".into()),
        sequence: Some(7),
        label: "line".into(),
        common_note: "required".into(),
    }
}

#[test]
fn membership_omits_both_synthetic_keys_and_preserves_ordinary_sources() {
    for courier in [None, Some("é".into())] {
        for sequence in [None, Some(7)] {
            let record = Parcel {
                courier: courier.clone(),
                sequence,
                ..parcel()
            };
            let item = record.to_item().unwrap();
            let member = courier.is_some() && sequence.is_some();
            assert_eq!(item.contains_key("gpk"), member);
            assert_eq!(item.contains_key("gsk"), member);
            assert_eq!(item.contains_key("courier"), courier.is_some());
            assert_eq!(item.contains_key("sequence"), sequence.is_some());
            assert_eq!(item["commonNote"], Av::S("required".into()));
            assert_eq!(Parcel::from_item(&item).unwrap(), record);
        }
    }
    let item = parcel().to_item().unwrap();
    assert_eq!(item["gpk"], Av::S("courier!c3a9".into()));
    assert_eq!(item["gsk"], Av::S("00000000000000000007!line!end".into()));
}

#[derive(Debug, PartialEq, tabulary::Record)]
#[tabulary(rename_all = "snake_case",
    index(ident = ByRoute, name = "by_route", kind = global, sparse,
        partition = route, sort = position, project = all),
    index(ident = ByPosition, name = "by_position", kind = local, sparse,
        partition = id, sort = position, project = keys_only))]
struct Reused {
    #[tabulary(partition_key)]
    id: String,
    #[tabulary(sort_key)]
    created: u64,
    route: Option<String>,
    position: Option<u64>,
}
#[test]
fn reused_fields_allow_partial_presence_for_global_and_local_indexes() {
    for route in [None, Some("route".into())] {
        for position in [None, Some(3)] {
            let record = Reused {
                id: "o".into(),
                created: 1,
                route: route.clone(),
                position,
            };
            let item = record.to_item().unwrap();
            assert_eq!(
                item.len(),
                2 + usize::from(route.is_some()) + usize::from(position.is_some())
            );
            assert_eq!(Reused::from_item(&item).unwrap(), record);
        }
    }
}

#[test]
fn malformed_present_inputs_are_errors_even_for_nonmembers() {
    let record = Parcel {
        sequence: None,
        label: "bad!label".into(),
        ..parcel()
    };
    let error = record.to_item().unwrap_err();
    assert_eq!(error.record, "Parcel");
    assert_eq!(
        error.kind,
        EncodeFault::KeyFormat(KeyFormatFault::InvalidSeparator)
    );
    assert_eq!(error.attribute(), Some("gsk"));
    let mut item = Parcel {
        sequence: None,
        ..parcel()
    }
    .to_item()
    .unwrap();
    item.insert("label".into(), Av::S("bad!label".into()));
    let error = Parcel::from_item(&item).unwrap_err();
    assert_eq!(
        error.kind,
        DecodeFault::KeyFormat(KeyFormatFault::InvalidSeparator)
    );
    assert_eq!(
        error.path.segments(),
        &[
            PathSegment::Attribute("gsk".into()),
            PathSegment::Component("label")
        ]
    );
    for value in [Av::Null(true), Av::Null(false), Av::N("1".into())] {
        item.insert("courier".into(), value);
        let error = Parcel::from_item(&item).unwrap_err();
        assert!(matches!(
            error.kind,
            DecodeFault::WrongType {
                expected: DynamoType::S,
                ..
            }
        ));
        assert_eq!(error.attribute(), Some("courier"));
    }
    let record = Reused {
        id: "o".into(),
        created: 1,
        route: Some(String::new()),
        position: None,
    };
    assert_eq!(record.to_item().unwrap_err().kind, EncodeFault::EmptyKey);
    let mut item = Reused {
        route: None,
        ..record
    }
    .to_item()
    .unwrap();
    for value in [Av::S(String::new()), Av::S("x".repeat(2049))] {
        item.insert("route".into(), value);
        let error = Reused::from_item(&item).unwrap_err();
        assert_eq!(
            error.kind,
            DecodeFault::KeyFormat(KeyFormatFault::InvalidLength)
        );
        assert_eq!(error.attribute(), Some("route"));
    }
    item.remove("route");
    item.insert("position".into(), Av::N("not-a-number".into()));
    assert_eq!(
        Reused::from_item(&item).unwrap_err().kind,
        DecodeFault::InvalidNumber
    );
}

#[test]
fn partial_unexpected_malformed_and_mismatched_synthetic_keys_are_corruption() {
    let item = parcel().to_item().unwrap();
    for name in ["gpk", "gsk"] {
        let mut partial = item.clone();
        partial.remove(name);
        let error = Parcel::from_item(&partial).unwrap_err();
        assert_eq!(error.kind, DecodeFault::Missing);
        assert_eq!(error.attribute(), Some(name));
    }
    for missing in ["courier", "sequence"] {
        let mut stale = item.clone();
        stale.remove(missing);
        assert_eq!(
            Parcel::from_item(&stale).unwrap_err().kind,
            DecodeFault::IndexMismatch
        );
    }
    let mut mismatch = item.clone();
    mismatch.insert("courier".into(), Av::S("different".into()));
    let error = Parcel::from_item(&mismatch).unwrap_err();
    assert_eq!(error.kind, DecodeFault::KeyMismatch);
    assert_eq!(error.attribute(), Some("gpk"));
    mismatch.insert("gpk".into(), Av::S("wrong".into()));
    assert_eq!(
        Parcel::from_item(&mismatch).unwrap_err().kind,
        DecodeFault::KeyFormat(KeyFormatFault::InvalidPrefix)
    );
    let mut missing_common = item;
    missing_common.remove("commonNote");
    assert_eq!(
        Parcel::from_item(&missing_common).unwrap_err().kind,
        DecodeFault::Missing
    );
}

#[derive(Debug, PartialEq, tabulary::Record)]
#[tabulary(rename_all = "snake_case",
    index(ident = ByRoute, name = "by_route", kind = global, sparse,
        partition(name = "gpk", template = "{route:hex}"), sort = position, project = all),
    index(ident = ByLabel, name = "by_label", kind = global, sparse,
        partition(name = "gpk", template = "{route:hex}"), sort = label, project = all))]
struct Shared {
    #[tabulary(partition_key)]
    id: String,
    route: String,
    position: Option<u64>,
    label: Option<String>,
}
#[test]
fn shared_templates_are_present_when_any_consuming_index_is_a_member() {
    for position in [None, Some(1)] {
        for label in [None, Some("label".into())] {
            let record = Shared {
                id: "o".into(),
                route: "r".into(),
                position,
                label: label.clone(),
            };
            let item = record.to_item().unwrap();
            assert_eq!(
                item.contains_key("gpk"),
                position.is_some() || label.is_some()
            );
            assert_eq!(Shared::from_item(&item).unwrap(), record);
        }
    }
}

#[derive(Debug, PartialEq, tabulary::Record)]
#[tabulary(rename_all = "snake_case", index(ident = ByValue, name = "by_value", kind = global,
    sparse, partition(name = "gpk", template = "{value:hex}"), project = all))]
struct Generic<T> {
    #[tabulary(partition_key)]
    id: String,
    value: Option<T>,
}
#[test]
fn optional_generic_components_keep_inner_codec_types() {
    let record = Generic {
        id: "o".into(),
        value: Some([0u8, 255]),
    };
    let item = record.to_item().unwrap();
    assert_eq!(item["gpk"], Av::S("00ff".into()));
    assert_eq!(Generic::<[u8; 2]>::from_item(&item).unwrap(), record);
    assert_eq!(
        GenericIndexes::<[u8; 2]>::BY_VALUE
            .partition([0, 255])
            .unwrap()
            .as_str(),
        "00ff"
    );
}

#[derive(Debug, PartialEq, tabulary::Projection)]
#[tabulary(record = Parcel)]
struct IndexKeys {
    #[tabulary(attribute = ParcelAttrs::BY_COURIER_PARTITION)]
    partition: Option<String>,
    #[tabulary(attribute = ParcelAttrs::BY_COURIER_SORT)]
    sort: Option<String>,
}
#[test]
fn conditional_key_descriptors_and_projection_hooks_preserve_presence() {
    use tabulary::{Projection, Record};
    let _: tabulary::Attr<Parcel, String, tabulary::Optional, tabulary::DerivedOnly> =
        ParcelAttrs::BY_COURIER_PARTITION;
    let _: tabulary::Attr<Parcel, String, tabulary::Optional, tabulary::IndexSourceOnly> =
        ParcelAttrs::COURIER;
    assert_eq!(
        Parcel::SCHEMA.fields.last().unwrap().presence,
        tabulary::Presence::Optional
    );
    for sequence in [None, Some(7)] {
        let record = Parcel {
            sequence,
            ..parcel()
        };
        let view = IndexKeys::from_item(&record.to_item().unwrap()).unwrap();
        assert_eq!(view.partition.is_some(), sequence.is_some());
        assert_eq!(view.sort.is_some(), sequence.is_some());
        assert_eq!(
            IndexKeys::project_source(record, &mut tabulary::DecodeContext::new("Parcel")).unwrap(),
            view
        );
    }
    let mut item = parcel().to_item().unwrap();
    item.insert("gpk".into(), Av::N("1".into()));
    assert!(matches!(
        Parcel::from_item(&item).unwrap_err().kind,
        DecodeFault::WrongType {
            expected: DynamoType::S,
            ..
        }
    ));
    let mut absent = Parcel {
        sequence: None,
        ..parcel()
    }
    .to_item()
    .unwrap();
    absent.insert("gsk".into(), Av::S("00000000000000000007!line!end".into()));
    let error = Parcel::from_item(&absent).unwrap_err();
    assert_eq!(error.kind, DecodeFault::IndexMismatch);
    assert_eq!(error.attribute(), Some("gsk"));
}

#[derive(Debug, PartialEq, tabulary::Record)]
#[tabulary(rename_all = "snake_case", partition(name = "pk", template = "{id:hex}"),
    index(ident = ByRoute, name = "by_route", kind = global, sparse,
        partition(name = "pk", template = "{id:hex}"), sort = route, project = all))]
struct PrimaryShared {
    id: String,
    route: Option<Vec<u8>>,
}
#[test]
fn primary_templates_are_retained_and_optional_binary_keys_validate() {
    for route in [None, Some(vec![1, 2])] {
        let record = PrimaryShared {
            id: "o".into(),
            route,
        };
        let item = record.to_item().unwrap();
        assert_eq!(item["pk"], Av::S("6f".into()));
        assert_eq!(PrimaryShared::from_item(&item).unwrap(), record);
    }
    let record = PrimaryShared {
        id: "o".into(),
        route: Some(vec![]),
    };
    assert_eq!(record.to_item().unwrap_err().kind, EncodeFault::EmptyKey);
    let mut item = PrimaryShared {
        route: None,
        ..record
    }
    .to_item()
    .unwrap();
    item.insert("route".into(), Av::B(vec![1; 1025].into()));
    let error = PrimaryShared::from_item(&item).unwrap_err();
    assert_eq!(
        error.kind,
        DecodeFault::KeyFormat(KeyFormatFault::InvalidLength)
    );
    assert_eq!(error.attribute(), Some("route"));
}
