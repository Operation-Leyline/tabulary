#![cfg(feature = "derive")]
use aws_sdk_dynamodb::types::AttributeValue as Av;
use tabulary::{
    Converter, DecodeContext, DecodeFault, Document, EncodeContext, EncodeFault, Item, Projection,
    RecordKey, Update,
};

// A logical type deliberately lacking Value: derive bounds must select Converter<T>.
#[derive(Clone, Debug, PartialEq)]
struct Code(u64);
mod padded {
    use super::*;
    pub struct Codec;
    impl Converter<Code> for Codec {
        type Wire = tabulary::wire::String;
        fn encode(value: &Code, cx: &mut EncodeContext) -> Result<Av, tabulary::EncodeError> {
            if value.0 == 13 {
                return Err(cx.error(EncodeFault::Custom {
                    code: "reserved_code",
                }));
            }
            Ok(Av::S(format!("{:020}", value.0)))
        }
        fn decode(value: &Av, cx: &mut DecodeContext) -> Result<Code, tabulary::DecodeError> {
            let text = <tabulary::Native<String> as Converter<String>>::decode(value, cx)?;
            let code = text
                .parse::<u64>()
                .ok()
                .filter(|v| *v != 13 && format!("{v:020}") == text)
                .ok_or_else(|| {
                    cx.error(DecodeFault::Custom {
                        code: "invalid_code",
                    })
                })?;
            Ok(Code(code))
        }
    }
    impl tabulary::OrderedKeyCodec<Code> for Codec {}
    impl tabulary::PrefixCodec<Code> for Codec {
        type Prefix = Code;
        fn encode_prefix(
            value: &Code,
            cx: &mut EncodeContext,
        ) -> Result<Av, tabulary::EncodeError> {
            Self::encode(value, cx)
        }
    }
}
mod text {
    pub type Codec = tabulary::Native<String>;
}
fn initial() -> u128 {
    u128::MAX
}
#[derive(Debug, PartialEq, tabulary::Document)]
#[tabulary(rename_all = "snake_case")]
struct Amount<T> {
    #[tabulary(with = padded)]
    code: T,
    #[tabulary(with = tabulary::convert::u128_string, default = initial)]
    total: u128,
    #[tabulary(with = tabulary::convert::u128_string)]
    optional: Option<u128>,
}
#[derive(tabulary::Record)]
#[tabulary(rename_all = "snake_case")]
struct Invoice {
    #[tabulary(partition_key)]
    id: String,
    #[tabulary(with = tabulary::convert::u128_string)]
    total: u128,
    native: u128,
    amount: Amount<Code>,
}
#[derive(Debug, PartialEq, tabulary::Projection)]
#[tabulary(record = Invoice)]
struct Total {
    #[tabulary(attribute = InvoiceAttrs::TOTAL)]
    value: u128,
}
#[derive(Debug, PartialEq, tabulary::Projection)]
#[tabulary(record = Amount<Code>)]
struct AmountView {
    #[tabulary(attribute = AmountAttrs::<Code>::TOTAL)]
    value: u128,
    #[tabulary(attribute = AmountAttrs::<Code>::OPTIONAL)]
    optional: Option<u128>,
}
fn invoice() -> Invoice {
    Invoice {
        id: "invoice".into(),
        total: u128::MAX,
        native: 1,
        amount: Amount {
            code: Code(7),
            total: u128::MAX,
            optional: None,
        },
    }
}
#[test]
fn strict_decode_projection_defaults_and_optional_presence_use_the_converter() {
    let record = invoice();
    let mut item = record.to_item().unwrap();
    assert_eq!(item["total"], Av::S(u128::MAX.to_string()));
    assert_eq!(Invoice::from_item(&item).unwrap().amount, record.amount);
    assert_eq!(Total::from_item(&item).unwrap().value, u128::MAX);
    assert_eq!(
        InvoiceAttrs::TOTAL.descriptor().wire,
        tabulary::DynamoType::S
    );
    assert_eq!(
        AmountView::from_item(&Item::new()).unwrap(),
        AmountView {
            value: u128::MAX,
            optional: None
        }
    );
    for (wire, fault) in [
        (
            Av::S("01".into()),
            DecodeFault::Custom {
                code: "invalid_u128_string",
            },
        ),
        (
            Av::N("1".into()),
            DecodeFault::WrongType {
                expected: tabulary::DynamoType::S,
                actual: tabulary::DynamoType::N,
            },
        ),
        (
            Av::Null(true),
            DecodeFault::WrongType {
                expected: tabulary::DynamoType::S,
                actual: tabulary::DynamoType::Null,
            },
        ),
    ] {
        item.insert("total".into(), wire.clone());
        let error = Invoice::from_item(&item).err().unwrap();
        assert_eq!(
            (error.record, error.document, error.path.to_string()),
            ("Invoice", "Invoice", "[\"total\"]".into())
        );
        assert_eq!(error.kind, fault);
        assert_eq!(Total::from_item(&item).unwrap_err().kind, fault);
        for name in ["total", "optional"] {
            let map = [(name.into(), wire.clone())].into();
            assert_eq!(AmountView::from_item(&map).unwrap_err().kind, fault);
        }
    }
    item.remove("total");
    assert_eq!(
        Invoice::from_item(&item).err().unwrap().kind,
        DecodeFault::Missing
    );
    assert_eq!(
        Total::from_item(&item).unwrap_err().kind,
        DecodeFault::Missing
    );
    let mut record = invoice();
    record.amount.optional = Some(u128::MAX);
    let item = record.to_item().unwrap();
    assert_eq!(
        Invoice::from_item(&item).unwrap().amount.optional,
        Some(u128::MAX)
    );
}
#[test]
fn converter_faults_keep_the_nested_physical_path_on_both_directions() {
    let mut record = invoice();
    record.amount.code = Code(13);
    let error = record.to_item().unwrap_err();
    assert_eq!(
        error.kind,
        EncodeFault::Custom {
            code: "reserved_code"
        }
    );
    assert_eq!(
        (error.record, error.document, error.path.to_string()),
        ("Invoice", "Amount", "[\"amount\"][\"code\"]".into())
    );
    let mut item = invoice().to_item().unwrap();
    let Av::M(amount) = item.get_mut("amount").unwrap() else {
        panic!("map")
    };
    amount.insert("code".into(), Av::S("bad".into()));
    let error = Invoice::from_item(&item).err().unwrap();
    assert_eq!(
        error.kind,
        DecodeFault::Custom {
            code: "invalid_code"
        }
    );
    assert_eq!(
        (error.record, error.document, error.path.to_string()),
        ("Invoice", "Amount", "[\"amount\"][\"code\"]".into())
    );
}
#[test]
fn conditions_updates_and_nested_handles_encode_the_same_representation_as_put() {
    let condition = InvoiceAttrs::TOTAL.eq(u128::MAX).build().unwrap().render();
    assert_eq!(condition.values()[":v0"], Av::S(u128::MAX.to_string()));
    let update = Update::new()
        .and(InvoiceAttrs::TOTAL.set(u128::MAX))
        .and(
            InvoiceAttrs::AMOUNT
                .field(AmountAttrs::<Code>::CODE)
                .set(Code(8)),
        )
        .and(
            InvoiceAttrs::AMOUNT
                .field(AmountAttrs::<Code>::OPTIONAL)
                .set(u128::MAX),
        )
        .build()
        .unwrap()
        .render();
    assert!(
        update
            .values()
            .values()
            .any(|v| v == &Av::S(u128::MAX.to_string()))
    );
    assert!(
        update
            .values()
            .values()
            .any(|v| v == &Av::S(format!("{:020}", 8)))
    );
    // The selected wire explicitly opts into ordering and prefix predicates.
    assert!(KeyedAttrs::<Code>::CODE.lt(Code(8)).build().is_ok());
    assert!(
        KeyedAttrs::<Code>::CODE
            .begins_with(Code(8))
            .build()
            .is_ok()
    );
}
#[derive(tabulary::Record)]
#[tabulary(rename_all = "snake_case", index(ident = ByCode, name = "by_code", kind = global, sparse, partition = code, sort = total, project = all))]
struct Keyed<T> {
    #[tabulary(partition_key, with = tabulary::convert::u128_string)]
    id: u128,
    #[tabulary(sort_key, with = text)]
    sort: String,
    #[tabulary(with = padded)]
    code: Option<T>,
    #[tabulary(with = tabulary::convert::u128_string)]
    total: u128,
}
#[derive(tabulary::Projection)]
#[tabulary(record = Keyed<Code>)]
struct KeyView {
    #[tabulary(attribute = KeyedAttrs::<Code>::ID)]
    id: u128,
}
#[test]
fn primary_index_and_reindex_paths_use_converters_and_validate_physical_keys() {
    let key = KeyedKey::<Code>::new(u128::MAX, "row".into()).unwrap();
    let map = key.encode_key().unwrap();
    assert_eq!(map["id"], Av::S(u128::MAX.to_string()));
    assert_eq!(KeyedKey::<Code>::decode_key(&map).unwrap().id, u128::MAX);
    assert_eq!(KeyView::from_item(&map).unwrap().id, u128::MAX);
    let record = Keyed {
        id: u128::MAX,
        sort: "row".into(),
        code: Some(Code(7)),
        total: u128::MAX,
    };
    assert_eq!(
        Keyed::<Code>::from_item(&record.to_item().unwrap())
            .unwrap()
            .code,
        Some(Code(7))
    );
    let primary = Keyed::<Code>::primary()
        .partition(u128::MAX)
        .sort_eq("row".into())
        .build()
        .unwrap()
        .render();
    assert_eq!(primary.values()[":v0"], Av::S(u128::MAX.to_string()));
    let index = KeyedIndexes::<Code>::BY_CODE
        .partition(Code(7))
        .sort_eq(u128::MAX)
        .build()
        .unwrap()
        .render();
    assert_eq!(index.values()[":v0"], Av::S(format!("{:020}", 7)));
    assert_eq!(index.values()[":v1"], Av::S(u128::MAX.to_string()));
    let changes = Update::new().reindex(
        KeyedIndexes::<Code>::BY_CODE,
        KeyedByCodeInputs::<Code>::new(Some(Code(8)), u128::MAX),
    );
    let request = tabulary::Table::<Keyed<Code>>::new("invoices")
        .unwrap()
        .update(key)
        .changes(changes)
        .build()
        .unwrap();
    assert!(
        request
            .expressions()
            .values()
            .values()
            .any(|v| v == &Av::S(format!("{:020}", 8)))
    );
    for sort in [String::new(), "x".repeat(1025)] {
        assert!(KeyedKey::<Code>::new(1, sort.clone()).is_err());
        let map = [
            ("id".into(), Av::S("1".into())),
            ("sort".into(), Av::S(sort)),
        ]
        .into();
        assert!(KeyedKey::<Code>::decode_key(&map).is_err());
    }
}
#[derive(tabulary::Record)]
#[tabulary(rename_all = "snake_case", partition(name = "pk", template = "invoice!{source:dec}"),
    index(ident = BySource, name = "by_source", kind = global, partition(name = "gpk", template = "source!{source:dec}"), sort = code, project = all))]
struct Composite {
    #[tabulary(with = tabulary::convert::u128_string)]
    source: u128,
    #[tabulary(with = padded)]
    code: Code,
}
#[test]
fn template_sources_and_custom_key_capabilities_remain_logical() {
    let record = Composite {
        source: 7,
        code: Code(8),
    };
    let mut item = record.to_item().unwrap();
    assert_eq!(item["pk"], Av::S("invoice!7".into()));
    assert_eq!(item["source"], Av::S("7".into()));
    assert_eq!(Composite::from_item(&item).unwrap().source, 7);
    item.insert("source".into(), Av::S("8".into()));
    assert_eq!(
        Composite::from_item(&item).err().unwrap().kind,
        DecodeFault::KeyMismatch
    );
    let ordered = CompositeIndexes::BY_SOURCE
        .partition(7)
        .sort_lt(Code(9))
        .build()
        .unwrap()
        .render();
    assert_eq!(ordered.values()[":v1"], Av::S(format!("{:020}", 9)));
    assert!(
        CompositeIndexes::BY_SOURCE
            .partition(7)
            .sort_begins_with(Code(9))
            .build()
            .is_ok()
    );
    let changes = Update::new().reindex(
        CompositeIndexes::BY_SOURCE,
        CompositeBySourceInputs::new(7, Code(9)),
    );
    let request = tabulary::Table::<Composite>::new("invoices")
        .unwrap()
        .update(CompositeKey::new(7).unwrap())
        .changes(changes)
        .build()
        .unwrap();
    assert!(
        request
            .expressions()
            .values()
            .values()
            .any(|v| v == &Av::S(format!("{:020}", 9)))
    );
}
#[derive(Debug, PartialEq, tabulary::Variants)]
#[tabulary(tag = "stage", rename_all = "snake_case")]
enum Stage {
    Waiting {
        #[tabulary(with = tabulary::convert::u128_string)]
        total: u128,
    },
    Sent {
        #[tabulary(with = tabulary::convert::u128_string)]
        total: u128,
    },
}
#[derive(tabulary::Document)]
#[tabulary(rename_all = "snake_case")]
struct Status {
    #[tabulary(flatten)]
    stage: Stage,
}
#[derive(tabulary::Record)]
#[tabulary(rename_all = "snake_case")]
struct Shipment {
    #[tabulary(partition_key)]
    id: String,
    #[tabulary(flatten)]
    stage: Stage,
}
#[test]
fn flattened_shared_payloads_preserve_their_converter_in_records_and_documents() {
    let status = Status {
        stage: Stage::Sent { total: u128::MAX },
    };
    let item = status
        .encode_map(&mut EncodeContext::new("Status"))
        .unwrap();
    assert_eq!(item["total"], Av::S(u128::MAX.to_string()));
    assert_eq!(
        Status::decode_map(&item, &mut DecodeContext::new("Status"))
            .unwrap()
            .stage,
        status.stage
    );
    let record = Shipment {
        id: "shipment".into(),
        stage: status.stage,
    };
    let mut item = record.to_item().unwrap();
    assert_eq!(Shipment::from_item(&item).unwrap().stage, record.stage);
    item.insert("total".into(), Av::S("01".into()));
    let error = Shipment::from_item(&item).err().unwrap();
    assert_eq!(error.path.to_string(), "[\"total\"]");
    assert_eq!(
        error.kind,
        DecodeFault::Custom {
            code: "invalid_u128_string"
        }
    );
}
#[test]
fn converter_declarations_and_capabilities_are_compile_gates() {
    trybuild::TestCases::new().compile_fail("tests/ui/converters/*.rs");
}
#[allow(dead_code)]
#[path = "../../../docs/examples/07_values_projection_scan.rs"]
mod values_projection_scan;
#[test]
fn example_07_round_trips_with_its_original_converter_attribute() {
    let item = values_projection_scan::envelope()
        .unwrap()
        .to_item()
        .unwrap();
    assert_eq!(item["full_width_number"], Av::S(u128::MAX.to_string()));
    assert!(values_projection_scan::InvoiceEnvelope::from_item(&item).is_ok());
}
