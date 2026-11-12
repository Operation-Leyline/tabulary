use std::collections::HashMap;

use aws_sdk_dynamodb::{primitives::Blob, types::AttributeValue as Av};
use tabulary::{BilledSize, BuildError, EncodeContext, EncodeFault, Native, PathSegment, field};

fn item(name: &str, value: Av) -> HashMap<String, Av> {
    HashMap::from([(name.to_owned(), value)])
}

fn assert_size(item: &HashMap<String, Av>, estimate: usize, conservative: usize) {
    let size = BilledSize::of_item(item).unwrap();
    assert_eq!(size.estimated_item_bytes(), estimate);
    assert_eq!(size.conservative_item_bytes(), conservative);
}

#[test]
fn record_sizing_uses_its_document_codec_and_preserves_encode_faults() {
    use tabulary::{Document, Record};
    struct Invoice(u128);
    impl Record for Invoice {
        const SCHEMA: tabulary::RecordSchema = tabulary::RecordSchema {
            name: "Invoice",
            fields: &[tabulary::FieldDescriptor {
                name: "number",
                index: 0,
                presence: tabulary::Presence::Required,
                access: tabulary::FieldAccess::KeyOnly,
                wire: tabulary::DynamoType::N,
            }],
            primary: tabulary::KeyDefinition {
                partition: 0,
                sort: None,
            },
            indexes: &[],
            ttl: None,
        };
    }
    impl Document for Invoice {
        const NAME: &'static str = "Invoice";
        fn encode_map(
            &self,
            cx: &mut EncodeContext,
        ) -> Result<tabulary::Item, tabulary::EncodeError> {
            let value = <Native<u128> as tabulary::Converter<u128>>::encode(
                &self.0,
                &mut cx.at(PathSegment::Attribute("number".into())),
            )?;
            Ok(item("number", value))
        }
        fn decode_map(
            _: &tabulary::Item,
            _: &mut tabulary::DecodeContext,
        ) -> Result<Self, tabulary::DecodeError> {
            unreachable!("sizing never decodes")
        }
    }
    let size = Invoice(7).billed_size().unwrap();
    assert_eq!(size.estimated_item_bytes(), 8);
    assert_eq!(size.conservative_item_bytes(), 27);
    let error = Invoice(u128::MAX).billed_size().unwrap_err();
    assert_eq!(error.kind, EncodeFault::NumberRange);
    assert_eq!(error.record, "Invoice");
    assert_eq!(error.attribute(), Some("number"));
}

#[test]
fn scalar_bytes_are_utf8_raw_binary_and_one_byte_flags() {
    assert_size(&item("é", Av::S("配送".into())), 8, 8);
    assert_size(&item("b", Av::B(Blob::new([0, 1, 255, 2]))), 5, 5);
    assert_size(&item("b", Av::B(Blob::new([]))), 1, 1);
    assert_size(&item("s", Av::S(String::new())), 1, 1);
    for value in [Av::Bool(false), Av::Bool(true), Av::Null(true)] {
        assert_size(&item("f", value), 2, 2);
    }
}

#[test]
fn top_level_has_no_wrapper_and_omitted_attributes_cost_nothing() {
    let mut values = HashMap::new();
    assert_size(&values, 0, 0);
    field::encode_optional::<String, Native<String>>(&None, &mut EncodeContext::new("Order"))
        .unwrap()
        .into_iter()
        .for_each(|value| {
            values.insert("note".into(), value);
        });
    assert_size(&values, 0, 0);
    values.insert("orderKey".into(), Av::S("order!42".into()));
    values.insert("quantity".into(), Av::N("1".into()));
    assert_size(&values, 26, 45);
}

#[test]
fn lists_and_maps_charge_three_bytes_plus_one_per_child() {
    assert_size(&item("l", Av::L(vec![])), 4, 4);
    assert_size(&item("m", Av::M(HashMap::new())), 4, 4);
    let nested = Av::L(vec![
        Av::M(item("é", Av::S("猫".into()))),
        Av::N("-123".into()),
        Av::Null(true),
    ]);
    // name + list header + three children + map header/child/name/string + number + null.
    assert_size(&item("l", nested), 21, 38);
}

#[test]
fn sets_sum_scalar_members_without_container_or_child_overhead() {
    assert_size(&item("s", Av::Ss(vec!["é".into(), "猫".into()])), 6, 6);
    assert_size(
        &item("b", Av::Bs(vec![Blob::new([]), Blob::new([1, 2, 3])])),
        4,
        4,
    );
    assert_size(&item("n", Av::Ns(vec!["0".into(), "-123".into()])), 7, 43);
}

#[test]
fn numbers_normalize_digits_exponents_and_negative_zero() {
    for (text, bytes) in [
        ("0", 2),
        ("-0e9999", 2),
        ("+000.000", 2),
        ("123", 3),
        ("-123", 4),
        ("1234", 3),
        ("00120.000e+10", 2),
        ("-0.001200", 3),
        ("1e-130", 2),
        ("-1e125", 3),
        ("99999999999999999999999999999999999999", 20),
        ("-99999999999999999999999999999999999999", 21),
    ] {
        assert_size(&item("n", Av::N(text.into())), 1 + bytes, 22);
    }
}

#[test]
fn maximum_check_is_inclusive_and_uses_the_conservative_model() {
    let size = BilledSize::of_item(&item("n", Av::N("0".into()))).unwrap();
    assert_eq!(size.ensure_at_most(22), Ok(()));
    assert_eq!(size.ensure_at_most(usize::MAX), Ok(()));
    assert_eq!(
        size.ensure_at_most(3),
        Err(BuildError::RequestLimit { max: 3, actual: 22 })
    );
    let cap = 400 * 1024;
    let exact = BilledSize::of_item(&item("b", Av::B(Blob::new(vec![0; cap - 1])))).unwrap();
    assert_eq!(exact.ensure_at_most(cap), Ok(()));
    assert_eq!(
        exact.ensure_at_most(cap - 1),
        Err(BuildError::RequestLimit {
            max: cap - 1,
            actual: cap
        })
    );
}

#[test]
fn malformed_numbers_preserve_nested_paths_without_payloads() {
    for text in [
        "secret-value",
        "1e126",
        "1e-131",
        "123456789012345678901234567890123456789",
    ] {
        let error =
            BilledSize::of_item(&item("lines", Av::L(vec![Av::N(text.into())]))).unwrap_err();
        assert_eq!(error.record, "Item");
        assert_eq!(
            error.path.segments(),
            &[
                PathSegment::Attribute("lines".into()),
                PathSegment::Index(0)
            ]
        );
        assert_eq!(error.kind, EncodeFault::NumberRange);
        assert!(!format!("{error:?} {error}").contains(text));
    }
}

#[test]
fn document_nesting_is_bounded_at_32_containers() {
    let mut nested = Av::S(String::new());
    for _ in 0..32 {
        nested = Av::L(vec![nested]);
    }
    assert_size(&item("l", nested.clone()), 129, 129);
    let error = BilledSize::of_item(&item("l", Av::L(vec![nested]))).unwrap_err();
    assert_eq!(error.kind, EncodeFault::DepthLimit);
}

#[test]
fn invalid_null_and_future_sdk_types_never_get_a_size_default() {
    use aws_sdk_dynamodb::operation::get_item::{GetItem, GetItemOutput};
    use aws_smithy_runtime_api::client::{
        orchestrator::HttpResponse,
        runtime_plugin::RuntimePlugin,
        ser_de::{DeserializeResponse, SharedResponseDeserializer},
    };
    use aws_smithy_types::{body::SdkBody, config_bag::ConfigBag};

    let mut config = ConfigBag::base();
    config.push_shared_layer(GetItem::new().config().unwrap());
    let response = HttpResponse::new(
        200.try_into().unwrap(),
        SdkBody::from(r#"{"Item":{"payload":{"FUTURE":"private-payload"}}}"#),
    );
    let output = config
        .load::<SharedResponseDeserializer>()
        .unwrap()
        .deserialize_nonstreaming_with_config(&response, &config)
        .unwrap()
        .downcast::<GetItemOutput>()
        .unwrap();
    for (values, code) in [
        (output.item().unwrap(), "unsupported_type"),
        (&item("payload", Av::Null(false)), "invalid_null"),
    ] {
        let error = BilledSize::of_item(values).unwrap_err();
        assert_eq!(error.attribute(), Some("payload"));
        assert_eq!(error.kind, EncodeFault::Custom { code });
        assert!(!format!("{error:?}").contains("private-payload"));
    }
}
