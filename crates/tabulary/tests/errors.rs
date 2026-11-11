use std::error::Error;
use tabulary::{
    AttributePath, BuildError, DecodeContext, DecodeError, DecodeFault, DynamoType, EncodeContext,
    EncodeError, EncodeFault, KeyFormatFault, PathSegment, SetFault,
};

fn attribute(name: &str) -> PathSegment {
    PathSegment::Attribute(name.into())
}

#[test]
fn literal_names_are_escaped_and_distinct_from_nesting() {
    let literal = AttributePath::from(vec![attribute("a.b\"\\\n")]);
    assert_eq!(literal.to_string(), r#"["a.b\"\\\n"]"#);
    let nested = AttributePath::from(vec![attribute("a"), attribute("b")]);
    assert_eq!(nested.to_string(), r#"["a"]["b"]"#);
    assert_ne!(AttributePath::from(vec![attribute("a.b")]), nested);
    assert_eq!(literal.segments(), &[attribute("a.b\"\\\n")]);
}

#[test]
fn nested_list_errors_keep_root_document_and_physical_path() {
    let root = DecodeContext::new("Order");
    let lines = root.at(attribute("lines"));
    let price = lines
        .at(PathSegment::Index(2))
        .in_document("Line")
        .at(attribute("price"));
    let error = price.error(DecodeFault::WrongType {
        expected: DynamoType::N,
        actual: DynamoType::S,
    });
    assert_eq!(
        (error.record, error.document, error.attribute()),
        ("Order", "Line", Some("lines"))
    );
    assert_eq!(error.path.to_string(), r#"["lines"][2]["price"]"#);
    assert_eq!(
        error.path.segments(),
        &[
            attribute("lines"),
            PathSegment::Index(2),
            attribute("price")
        ]
    );
    assert_eq!(
        lines.error(DecodeFault::Missing).path.to_string(),
        r#"["lines"]"#
    );
    let sibling = root.at(attribute("quantity")).error(DecodeFault::Missing);
    assert_eq!(sibling.document, "Order");
    assert_eq!(sibling.attribute(), Some("quantity"));
    assert!(root.error(DecodeFault::Missing).path.segments().is_empty());
    assert_eq!(root.error(DecodeFault::Missing).attribute(), None);
}

#[test]
fn components_and_custom_codes_remain_structured_on_both_paths() {
    let segment = PathSegment::Component("shop_code");
    let decode = DecodeContext::new("Parcel")
        .at(attribute("parcelKey"))
        .at(segment.clone())
        .error(DecodeFault::Custom {
            code: "invalid_shop",
        });
    assert_eq!(
        decode.kind,
        DecodeFault::Custom {
            code: "invalid_shop"
        }
    );
    assert_eq!(
        decode.path.segments(),
        &[attribute("parcelKey"), segment.clone()]
    );
    assert_eq!(
        decode.path.to_string(),
        r#"["parcelKey"][component:"shop_code"]"#
    );
    let root = EncodeContext::new("Parcel");
    let encode = root
        .at(attribute("parcelKey"))
        .in_document("Shop")
        .at(segment)
        .error(EncodeFault::Custom {
            code: "invalid_shop",
        });
    assert_eq!(
        (encode.record, encode.document, encode.attribute()),
        ("Parcel", "Shop", Some("parcelKey"))
    );
    assert_eq!(
        encode.kind,
        EncodeFault::Custom {
            code: "invalid_shop"
        }
    );
    assert_eq!(encode.path, decode.path);
    assert_eq!(root.error(EncodeFault::DepthLimit).attribute(), None);
}

#[test]
fn formatting_contains_only_metadata_and_never_converter_input_or_text() {
    let rejected = "private customer value";
    let converter_text = format!("could not parse {rejected}");
    let decode = DecodeContext::new("Order")
        .at(attribute("amount"))
        .error(DecodeFault::Custom { code: "currency" });
    let encode = EncodeContext::new("Order")
        .at(attribute("amount"))
        .error(EncodeFault::Validation { code: "positive" });
    let build = BuildError::from(encode.clone());
    for output in [
        format!("{decode}"),
        format!("{decode:?}"),
        format!("{encode}"),
        format!("{encode:?}"),
        format!("{build}"),
        format!("{build:?}"),
    ] {
        assert!(output.contains("Order") && output.contains("amount"));
        assert!(!output.contains(rejected) && !output.contains(&converter_text));
    }
    assert!(decode.to_string().contains("currency"));
    assert!(encode.to_string().contains("positive"));
}

#[test]
fn standard_errors_preserve_encode_source_and_leaf_errors_have_none() {
    fn standard<T: Error + Send + Sync + 'static>(_: &T) {}
    let decode = DecodeContext::new("Order").error(DecodeFault::InvalidSet(SetFault::Duplicate));
    let encode =
        EncodeContext::new("Order").error(EncodeFault::KeyFormat(KeyFormatFault::InvalidHex));
    standard::<DecodeError>(&decode);
    standard::<EncodeError>(&encode);
    assert!(decode.source().is_none() && encode.source().is_none());
    let build: BuildError = encode.clone().into();
    standard(&build);
    assert!(matches!(&build, BuildError::Encode(inner) if inner == &encode));
    assert_eq!(
        build.source().unwrap().downcast_ref::<EncodeError>(),
        Some(&encode)
    );
    let constraint = BuildError::OverlappingUpdate {
        first: AttributePath::from(vec![attribute("lines")]),
        second: AttributePath::from(vec![attribute("lines"), PathSegment::Index(0)]),
    };
    assert!(constraint.source().is_none());
    assert!(constraint.to_string().contains("OverlappingUpdate"));
}
