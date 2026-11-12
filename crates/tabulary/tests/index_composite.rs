#![cfg(feature = "derive")]
use aws_sdk_dynamodb::types::AttributeValue as Av;
use tabulary::{Converter, IndexSchema, Record, RecordKey};

#[derive(Debug, PartialEq, tabulary::Record)]
#[tabulary(rename_all = "snake_case", partition(name = "pk", template = "order!{order_ref:hex}"),
    index(ident = ByCourier, name = "by_courier", kind = global,
        partition(name = "gpk", template = "courier!{courier:hex}"),
        sort(name = "gsk", template = "line!{line_no:dec20}!{order_ref:hex}"), project = all),
    index(ident = ByLine, name = "by_line", kind = local,
        partition(name = "pk", template = "order!{order_ref:hex}"), sort = line_no, project = all))]
struct Parcel {
    order_ref: String,
    courier: String,
    line_no: u64,
    #[tabulary(sort_key)]
    created: u64,
}

#[test]
fn index_templates_reuse_primary_formatting_and_inverse_checks() {
    let record = Parcel {
        order_ref: "o".into(),
        courier: "é".into(),
        line_no: 7,
        created: 1,
    };
    let mut item = record.to_item().unwrap();
    assert_eq!(item["gpk"], Av::S("courier!c3a9".into()));
    assert_eq!(item["gsk"], Av::S("line!00000000000000000007!6f".into()));
    assert_eq!(Parcel::from_item(&item).unwrap(), record);
    assert_eq!(record.key().unwrap().encode_key().unwrap().len(), 2);
    let _: tabulary::Attr<Parcel, String, tabulary::Required, tabulary::IndexSourceOnly> =
        ParcelAttrs::COURIER;
    let _: tabulary::Attr<Parcel, String, tabulary::Required, tabulary::KeyOnly> =
        ParcelAttrs::ORDER_REF;
    assert_eq!(
        Parcel::SCHEMA
            .attribute_definitions()
            .unwrap()
            .iter()
            .map(|a| a.attribute_name())
            .collect::<Vec<_>>(),
        ["line_no", "created", "pk", "gpk", "gsk"]
    );
    let partition = ParcelIndexes::BY_COURIER.partition("é".into()).unwrap();
    assert_eq!(partition.as_str(), "courier!c3a9");
    assert_eq!(
        ParcelIndexes::BY_COURIER
            .sort_prefix((7,))
            .unwrap()
            .as_str(),
        "line!00000000000000000007!"
    );
    item.insert("courier".into(), Av::S("other".into()));
    assert_eq!(
        Parcel::from_item(&item).unwrap_err().kind,
        tabulary::DecodeFault::KeyMismatch
    );
    item.insert("gpk".into(), Av::S("wrong".into()));
    assert!(matches!(
        Parcel::from_item(&item).unwrap_err().kind,
        tabulary::DecodeFault::KeyFormat(_)
    ));
}
#[test]
fn logical_index_codecs_and_corruption_paths_use_the_template_contract() {
    type Codec = <ParcelByCourier as IndexSchema<Parcel>>::PartitionCodec;
    let mut encode = tabulary::EncodeContext::new("Parcel");
    let mut decode = tabulary::DecodeContext::new("Parcel");
    let wire = Codec::encode(&("courier".into(),), &mut encode).unwrap();
    assert_eq!(wire, Av::S("courier!636f7572696572".into()));
    assert_eq!(
        Codec::decode(&wire, &mut decode).unwrap(),
        ("courier".into(),)
    );
    let record = Parcel {
        order_ref: "o".into(),
        courier: "c".into(),
        line_no: 7,
        created: 1,
    };
    let item = record.to_item().unwrap();
    for attribute in ["gpk", "gsk", "courier"] {
        let mut missing = item.clone();
        missing.remove(attribute);
        let error = Parcel::from_item(&missing).unwrap_err();
        assert_eq!(error.kind, tabulary::DecodeFault::Missing);
        assert_eq!(error.attribute(), Some(attribute));
    }
    let mut corrupt = item;
    corrupt.insert("line_no".into(), Av::N("8".into()));
    let error = Parcel::from_item(&corrupt).unwrap_err();
    assert_eq!(error.kind, tabulary::DecodeFault::KeyMismatch);
    assert_eq!(error.attribute(), Some("gsk"));
    let empty = Parcel {
        courier: "c".repeat(2048),
        ..record
    };
    assert!(matches!(
        empty.to_item().unwrap_err().kind,
        tabulary::EncodeFault::KeySize { max: 2048, .. }
    ));
}
#[derive(Debug, PartialEq, tabulary::Record)]
#[tabulary(rename_all = "snake_case", generated_prefix = "Stored",    index(ident = ByValue, name = "by_value", kind = global,        partition(name = "gpk", template = "{value:hex}"),        sort(name = "gsk", template = "{sequence:dec20}!{label:text}!end"), project = all))]
struct Generic<T> {
    #[tabulary(partition_key)]
    id: String,
    value: T,
    sequence: u64,
    label: String,
}
#[test]
fn generic_fixed_width_and_text_components_preserve_types_and_boundaries() {
    let record = Generic {
        id: "o".into(),
        value: [0u8, 255],
        sequence: 3,
        label: "line".into(),
    };
    let mut item = record.to_item().unwrap();
    assert_eq!(item["gpk"], Av::S("00ff".into()));
    assert_eq!(Generic::<[u8; 2]>::from_item(&item).unwrap(), record);
    assert_eq!(
        StoredIndexes::<[u8; 2]>::BY_VALUE
            .partition([0, 255])
            .unwrap()
            .as_str(),
        "00ff"
    );
    type Codec = <StoredByValue<[u8; 2]> as IndexSchema<Generic<[u8; 2]>>>::PartitionCodec;
    assert_eq!(
        Codec::decode(&item["gpk"], &mut tabulary::DecodeContext::new("Generic")).unwrap(),
        ([0, 255],)
    );
    item.insert("gpk".into(), Av::S("00".into()));
    assert_eq!(
        Generic::<[u8; 2]>::from_item(&item).unwrap_err().kind,
        tabulary::DecodeFault::KeyFormat(tabulary::KeyFormatFault::InvalidLength)
    );
    let invalid = Generic {
        label: "line!tail".into(),
        ..record
    };
    assert_eq!(
        invalid.to_item().unwrap_err().kind,
        tabulary::EncodeFault::KeyFormat(tabulary::KeyFormatFault::InvalidSeparator)
    );
}
