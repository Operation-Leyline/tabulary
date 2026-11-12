#![cfg(feature = "derive")]

use aws_sdk_dynamodb::types::AttributeValue as Av;
use tabulary::{DecodeContext, DecodeFault, DynamoType, EncodeContext, Value, WireType};

#[derive(Debug, PartialEq, tabulary::StringEnum)]
#[tabulary(rename_all = "snake_case")]
#[allow(non_camel_case_types)]
enum Stage {
    HTTPReady,
    #[tabulary(rename = "sent.v2")]
    Sent,
    r#type,
}

#[derive(Debug, PartialEq, tabulary::Record)]
#[tabulary(rename_all = "snake_case")]
struct Order {
    #[tabulary(partition_key)]
    order_ref: String,
    stage: Stage,
}

#[test]
fn variants_round_trip_as_strings_with_explicit_overrides_and_raw_names() {
    assert_eq!(<Stage as Value>::Wire::DYNAMO_TYPE, DynamoType::S);
    for (value, tag) in [
        (Stage::HTTPReady, "http_ready"),
        (Stage::Sent, "sent.v2"),
        (Stage::r#type, "type"),
    ] {
        let encoded = value
            .encode_value(&mut EncodeContext::new("Stage"))
            .unwrap();
        assert_eq!(encoded, Av::S(tag.into()));
        assert_eq!(
            Stage::decode_value(&encoded, &mut DecodeContext::new("Stage")).unwrap(),
            value
        );
        let order = Order {
            order_ref: "order-lime".into(),
            stage: value,
        };
        let item = order.to_item().unwrap();
        assert_eq!(item["stage"], encoded);
        assert_eq!(Order::from_item(&item).unwrap(), order);
    }
}

#[test]
fn all_rename_rules_apply_to_variant_names() {
    macro_rules! check {
        ($rule:literal, $expected:literal) => {{
            #[derive(tabulary::StringEnum)]
            #[tabulary(rename_all = $rule)]
            enum Unit {
                HTTPReady,
            }
            assert_eq!(
                Unit::HTTPReady
                    .encode_value(&mut EncodeContext::new("Unit"))
                    .unwrap(),
                Av::S($expected.into())
            );
        }};
    }
    check!("snake_case", "http_ready");
    check!("camelCase", "httpReady");
    check!("PascalCase", "HttpReady");
    check!("SCREAMING_SNAKE_CASE", "HTTP_READY");
}

#[test]
fn explicit_names_need_no_container_rule_and_support_configured_crate_path() {
    use tabulary as renamed;
    #[derive(Debug, PartialEq, tabulary::StringEnum)]
    #[tabulary(crate = "renamed")]
    enum Unit {
        #[tabulary(rename = "retail-unit")]
        Retail = 7,
    }
    let value = Unit::Retail
        .encode_value(&mut EncodeContext::new("Unit"))
        .unwrap();
    assert_eq!(value, Av::S("retail-unit".into()));
    assert_eq!(
        Unit::decode_value(&value, &mut DecodeContext::new("Unit")).unwrap(),
        Unit::Retail
    );
}

#[test]
fn unknown_wrong_and_missing_values_have_structured_field_errors() {
    let mut item = Order {
        order_ref: "order-lime".into(),
        stage: Stage::Sent,
    }
    .to_item()
    .unwrap();
    for (value, fault) in [
        (
            Av::S("future-private-tag".into()),
            DecodeFault::UnknownVariant,
        ),
        (Av::S("Sent".into()), DecodeFault::UnknownVariant),
        (Av::S("sent".into()), DecodeFault::UnknownVariant),
        (Av::S(String::new()), DecodeFault::UnknownVariant),
        (
            Av::N("1".into()),
            DecodeFault::WrongType {
                expected: DynamoType::S,
                actual: DynamoType::N,
            },
        ),
        (
            Av::Null(true),
            DecodeFault::WrongType {
                expected: DynamoType::S,
                actual: DynamoType::Null,
            },
        ),
    ] {
        item.insert("stage".into(), value);
        let error = Order::from_item(&item).unwrap_err();
        assert_eq!(error.record, "Order");
        assert_eq!(error.document, "Order");
        assert_eq!(error.attribute(), Some("stage"));
        assert_eq!(error.kind, fault);
        assert!(!format!("{error:?} {error}").contains("future-private-tag"));
    }
    item.remove("stage");
    assert_eq!(
        Order::from_item(&item).unwrap_err().kind,
        DecodeFault::Missing
    );
}

#[test]
fn invalid_declarations_fail_to_compile_at_the_offending_input() {
    trybuild::TestCases::new().compile_fail("tests/ui/string_enum/*.rs");
}
