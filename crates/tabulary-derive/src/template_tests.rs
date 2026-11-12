use super::*;
use syn::parse_quote;

fn fields() -> syn::FieldsNamed {
    parse_quote!({
        shop: String,
        token: Vec<u8>,
        fixed: [u8; 16],
        count: u64,
        signed: i64,
        other: String,
        r#type: String,
    })
}

fn checked(value: &str) -> syn::Result<Template> {
    parse(
        &syn::LitStr::new(value, proc_macro2::Span::call_site()),
        &fields(),
    )
}

#[test]
fn literals_braces_unicode_and_rust_names_are_preserved() {
    let template = checked("包裹{{{shop:hex}}}!{type:text}").unwrap();
    assert_eq!(template.parts[0], Part::Literal("包裹{".into()));
    assert_eq!(
        template.parts[1],
        Part::Component {
            field: 0,
            codec: Codec::Hex,
            width: None
        }
    );
    assert_eq!(template.parts[2], Part::Literal("}!".into()));
    assert_eq!(
        template.parts[3],
        Part::Component {
            field: 6,
            codec: Codec::Text,
            width: None
        }
    );
}

#[test]
fn fixed_widths_and_single_components_need_no_separator() {
    for value in [
        "{shop:hex}",
        "{shop:text}",
        "{signed:dec}",
        "{count:dec20}",
        "{fixed:hex}{count:dec20}",
        "{shop:hex}{fixed:hex}",
        "{fixed:hex}{shop:hex}",
        "{{}}",
        "constant",
    ] {
        assert!(checked(value).is_ok(), "{value}");
    }
    let template = checked("{fixed:hex}{count:dec20}").unwrap();
    assert_eq!(
        template.parts[0],
        Part::Component {
            field: 2,
            codec: Codec::Hex,
            width: Some(32)
        }
    );
    assert_eq!(
        template.parts[1],
        Part::Component {
            field: 3,
            codec: Codec::Dec20,
            width: Some(20)
        }
    );
}

#[test]
fn malformed_grammar_and_unknown_or_duplicate_references_are_rejected() {
    for value in [
        "",
        "{",
        "}",
        "{{}",
        "{shop}",
        "{:hex}",
        "{shop:}",
        "{shop:hex:hex}",
        "{shop.hex:hex}",
        "{shop :hex}",
        "{shop:{hex}}",
        "{missing:hex}",
        "{shop:base64}",
        "{shop:hex}!{shop:text}",
        "{type:text}!{r#type:text}",
    ] {
        assert!(checked(value).is_err(), "{value}");
    }
}

#[test]
fn ambiguous_runs_and_codec_separators_are_rejected() {
    for value in [
        "{shop:hex}{token:hex}",
        "{shop:hex}{fixed:hex}{token:hex}",
        "{shop:text}{other:text}",
        "{shop:hex}a{token:hex}",
        "{count:dec}0{signed:dec}",
        "{signed:dec}-{count:dec20}",
        "{shop:hex}a!{token:hex}",
        "{shop:text}{fixed:hex}a{count:dec}",
    ] {
        assert!(checked(value).is_err(), "{value}");
    }
    for value in [
        "{shop:hex}!{token:hex}",
        "{shop:hex}界{token:hex}",
        "{shop:text}!{other:text}",
        "{shop:text}a{token:hex}",
        "{count:dec}!{signed:dec}",
        "{fixed:hex}a{token:hex}",
        "{shop:hex}{count:dec20}!{token:hex}",
        "{{{shop:hex}}}",
    ] {
        assert!(checked(value).is_ok(), "{value}");
    }
}

#[test]
fn sources_cannot_be_optional_or_defaulted() {
    for fields in [
        parse_quote!({ shop: Option<String> }),
        parse_quote!({ #[tabulary(default = initial_shop)] shop: String }),
    ] {
        assert!(parse(&parse_quote!("{shop:hex}"), &fields).is_err());
    }
}

#[test]
fn unknown_array_lengths_are_conservatively_variable() {
    let fields = parse_quote!({ fixed: [u8; LENGTH], shop: String });
    assert!(parse(&parse_quote!("{fixed:hex}!{shop:hex}"), &fields).is_ok());
    assert!(parse(&parse_quote!("{fixed:hex}{shop:hex}"), &fields).is_err());
}

#[test]
fn references_use_rust_names_even_when_the_wire_name_differs() {
    let fields = parse_quote!({ #[tabulary(rename = "shopCode")] shop: String });
    assert!(parse(&parse_quote!("{shop:hex}"), &fields).is_ok());
    assert!(parse(&parse_quote!("{shopCode:hex}"), &fields).is_err());
}

#[test]
fn declaration_options_are_strict() {
    for input in [
        "partition(name = \"pk\")",
        "partition(name = \"pk\", template = 4)",
        "partition(name = \"pk\", template = \"x\", extra)",
        "partition(name = \"pk\", template = \"x\"), partition(name = \"pk\", template = \"y\")",
    ] {
        let input: syn::DeriveInput = syn::parse_str(&format!(
            "#[tabulary({input})] struct Order {{ id: String }}"
        ))
        .unwrap();
        assert!(crate::parse::options(&input.attrs, true).is_err());
    }
}
