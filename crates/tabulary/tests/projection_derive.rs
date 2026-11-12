#![cfg(feature = "derive")]

use aws_sdk_dynamodb::types::AttributeValue as Av;
use tabulary::{DecodeFault, Item, Projection};

fn initial() -> u64 {
    7
}
fn initial_address() -> Address {
    Address {
        city: "Lime".into(),
        required_elsewhere: "default".into(),
    }
}

#[derive(tabulary::Document)]
#[tabulary(rename_all = "snake_case")]
struct Address {
    city: String,
    required_elsewhere: String,
}

#[derive(tabulary::Variants)]
#[tabulary(tag = "stage", rename_all = "snake_case")]
enum Stage {
    Waiting,
    Sent {
        #[tabulary(with = tabulary::convert::u128_string)]
        tracking: u128,
    },
    Failed {
        reason: String,
    },
}

#[derive(tabulary::Record)]
#[tabulary(rename_all = "snake_case")]
struct Order {
    #[tabulary(partition_key)]
    id: String,
    #[tabulary(rename = "order.quantity", default = initial)]
    quantity: u64,
    note: Option<String>,
    #[tabulary(default = initial_address)]
    address: Address,
    optional_address: Option<Address>,
    #[tabulary(flatten)]
    stage: Stage,
}

#[derive(Debug, PartialEq, tabulary::Projection)]
#[tabulary(record = Order)]
struct Quantity {
    #[tabulary(attribute = OrderAttrs::QUANTITY)]
    count: u64,
    #[tabulary(attribute = OrderAttrs::NOTE)]
    note: Option<String>,
}

#[derive(Debug, PartialEq, tabulary::Projection)]
#[tabulary(record = Address)]
struct City {
    #[tabulary(attribute = AddressAttrs::CITY)]
    city: String,
}

#[derive(Debug, PartialEq, tabulary::Projection)]
#[tabulary(record = Order)]
struct Location {
    #[tabulary(attribute = OrderAttrs::ADDRESS, nested)]
    address: City,
    #[tabulary(attribute = OrderAttrs::OPTIONAL_ADDRESS, nested)]
    optional: Option<City>,
}

#[derive(tabulary::Projection)]
#[tabulary(record = Order)]
struct Shipping {
    #[tabulary(attribute = OrderAttrs::STAGE)]
    stage: Stage,
}

#[test]
fn selected_fields_inherit_defaults_presence_and_strict_codecs() {
    let mut item = Item::new();
    assert_eq!(
        Quantity::from_item(&item).unwrap(),
        Quantity {
            count: 7,
            note: None
        }
    );
    // Transport metadata and omitted base fields do not trigger full-record validation.
    item.insert("id".into(), Av::Bool(false));
    item.insert("transport".into(), Av::Null(true));
    assert_eq!(Quantity::from_item(&item).unwrap().count, 7);
    item.insert("order.quantity".into(), Av::S("bad".into()));
    let error = Quantity::from_item(&item).unwrap_err();
    assert_eq!(error.record, "Order");
    assert_eq!(error.path.to_string(), "[\"order.quantity\"]");
    assert!(matches!(error.kind, DecodeFault::WrongType { .. }));
    item.remove("order.quantity");
    item.insert("note".into(), Av::Null(true));
    assert!(matches!(
        Quantity::from_item(&item).unwrap_err().kind,
        DecodeFault::WrongType { .. }
    ));
}

#[test]
fn nested_views_decode_only_their_selected_fields_and_keep_paths() {
    assert_eq!(
        Location::from_item(&Item::new()).unwrap().address.city,
        "Lime"
    );
    let mut item: Item = [(
        "address".into(),
        Av::M([("city".into(), Av::S("Lime".into()))].into()),
    )]
    .into();
    assert_eq!(
        Location::from_item(&item).unwrap(),
        Location {
            address: City {
                city: "Lime".into()
            },
            optional: None
        }
    );
    item.insert("optional_address".into(), item["address"].clone());
    assert_eq!(
        Location::from_item(&item).unwrap().optional.unwrap().city,
        "Lime"
    );
    item.insert("optional_address".into(), Av::Null(true));
    assert!(Location::from_item(&item).is_err());
    item.remove("optional_address");
    let mut cx = tabulary::DecodeContext::new("Order");
    for _ in 0..32 {
        cx = cx.enter_container().unwrap();
    }
    assert_eq!(
        Location::decode_map(&item, &mut cx).unwrap_err().kind,
        DecodeFault::DepthLimit
    );
    item.insert("address".into(), Av::M(Item::new()));
    let error = Location::from_item(&item).unwrap_err();
    assert_eq!(error.record, "Order");
    assert_eq!(error.document, "Address");
    assert_eq!(error.path.to_string(), "[\"address\"][\"city\"]");
    assert_eq!(error.kind, DecodeFault::Missing);
    item.insert("address".into(), Av::Bool(false));
    assert!(matches!(
        Location::from_item(&item).unwrap_err().kind,
        DecodeFault::WrongType { .. }
    ));
}

#[test]
fn flattened_variants_select_and_validate_the_whole_group() {
    assert_eq!(
        Shipping::attributes().unwrap(),
        ["stage", "reason", "tracking"]
    );
    let mut item: Item = [
        ("stage".into(), Av::S("sent".into())),
        ("tracking".into(), Av::S(u128::MAX.to_string())),
    ]
    .into();
    assert!(
        matches!(Shipping::from_item(&item).unwrap().stage, Stage::Sent { tracking } if tracking == u128::MAX)
    );
    item.remove("stage");
    assert_eq!(
        Shipping::from_item(&item).err().unwrap().kind,
        DecodeFault::Missing
    );
    item.insert("stage".into(), Av::S("waiting".into()));
    assert_eq!(
        Shipping::from_item(&item).err().unwrap().kind,
        DecodeFault::InactiveVariantAttribute
    );
}

#[test]
fn non_all_indexes_require_every_selected_attribute_and_transport_key() {
    assert!(Quantity::check_available(&["order.quantity", "note", "id"], &["id"]).is_ok());
    assert!(Quantity::check_available(&["order.quantity", "note"], &["id"]).is_err());
    assert!(Quantity::check_available(&["order.quantity", "id"], &[]).is_err());
    assert!(Shipping::check_available(&["stage", "tracking"], &[]).is_err());
    assert_eq!(
        Location::attributes().unwrap(),
        ["address", "optional_address"]
    );
}

#[test]
fn invalid_projections_are_compile_errors() {
    trybuild::TestCases::new().compile_fail("tests/ui/projection/*.rs");
}

#[test]
fn renamed_dependency_and_concrete_generic_source_work() {
    use tabulary as renamed;
    #[derive(renamed::Document)]
    #[tabulary(rename_all = "snake_case")]
    struct Cell<T> {
        value: T,
    }
    #[derive(renamed::Projection)]
    #[tabulary(record = Cell<u64>, crate = "renamed")]
    struct View {
        #[tabulary(attribute = CellAttrs::<u64>::VALUE)]
        value: u64,
    }
    assert_eq!(
        View::from_item(&[("value".into(), Av::N("9".into()))].into())
            .unwrap()
            .value,
        9
    );
}

#[test]
fn duplicate_and_forged_selections_fail_before_decode() {
    const FORGED: tabulary::Attr<Order, u64> = tabulary::__private::field("wrong", 1);
    #[derive(tabulary::Projection)]
    #[tabulary(record = Order)]
    struct Forged {
        #[tabulary(attribute = FORGED)]
        _value: u64,
    }
    #[derive(tabulary::Projection)]
    #[tabulary(record = Order)]
    struct Duplicate {
        #[tabulary(attribute = OrderAttrs::QUANTITY)]
        _a: u64,
        #[tabulary(attribute = OrderAttrs::QUANTITY)]
        _b: u64,
    }
    assert!(Forged::attributes().is_err());
    assert!(Duplicate::attributes().is_err());
    assert!(Forged::from_item(&Item::new()).is_err());
}

#[test]
fn a_source_codec_is_inherited_without_redeclaration() {
    use tabulary::{Converter, DecodeContext, DecodeError, Document, EncodeContext, EncodeError};
    type Codec = tabulary::convert::u128_string::Codec;
    struct Invoice {
        amount: u128,
    }
    const AMOUNT: tabulary::Attr<Invoice, u128, tabulary::Required, tabulary::Mutable, Codec> =
        tabulary::__private::field("amount", 0);
    impl Document for Invoice {
        const NAME: &'static str = "Invoice";
        const FIELDS: &'static [tabulary::FieldDescriptor] = &[AMOUNT.descriptor()];
        fn encode_map(&self, cx: &mut EncodeContext) -> Result<Item, EncodeError> {
            Ok([("amount".into(), Codec::encode(&self.amount, cx)?)].into())
        }
        fn decode_map(item: &Item, cx: &mut DecodeContext) -> Result<Self, DecodeError> {
            Ok(Self {
                amount: <Self as tabulary::projection::SourceField<0>>::decode(item, cx)?,
            })
        }
    }
    impl tabulary::projection::SourceField<0> for Invoice {
        type Handle = tabulary::Attr<Self, u128, tabulary::Required, tabulary::Mutable, Codec>;
        type Output = u128;
        type Presence = tabulary::Required;
        const ATTRIBUTES: &'static [&'static str] = &["amount"];
        fn decode(item: &Item, cx: &mut DecodeContext) -> Result<u128, DecodeError> {
            tabulary::field::decode_required::<u128, Codec>(
                item.get("amount"),
                &mut cx.at(tabulary::PathSegment::Attribute("amount".into())),
            )
        }
    }
    #[derive(tabulary::Projection)]
    #[tabulary(record = Invoice)]
    struct Amount {
        #[tabulary(attribute = AMOUNT)]
        value: u128,
    }
    let mut item: Item = [("amount".into(), Av::S(u128::MAX.to_string()))].into();
    assert_eq!(Amount::from_item(&item).unwrap().value, u128::MAX);
    item.insert("amount".into(), Av::N("1".into()));
    assert!(matches!(
        Amount::from_item(&item).err().unwrap().kind,
        DecodeFault::WrongType { .. }
    ));
}
