#![cfg(feature = "derive")]
//! Typed multi-segment paths into derived documents: conditions, updates,
//! filters, cursors, and the SDK inputs they render into.
use std::collections::HashMap;
use std::num::NonZeroU32;

use aws_sdk_dynamodb::{operation::scan::ScanOutput, types::AttributeValue as Av};
use tabulary::condition::{ConditionNode, Operand, Operator};
use tabulary::update::ActionNode;
use tabulary::{
    BuildError, Condition, Cursor, EncodeFault, Item, List, Map, MapKey, PathSegment,
    ReadConsistency::*, StringSet, Table, Update,
};

#[derive(Debug, PartialEq, tabulary::Document)]
#[tabulary(rename_all = "camelCase")]
struct Address {
    city: String,
    postal_code: String,
    #[tabulary(rename = "name")]
    recipient: String,
}

#[derive(Debug, PartialEq, tabulary::Document)]
#[tabulary(rename_all = "camelCase")]
struct Shipping {
    carrier: String,
    status: String,
    #[tabulary(rename = "size")]
    parcel_size: u64,
    attempts: u64,
    weight: f64,
    note: Option<String>,
    address: Address,
    tags: StringSet,
    labels: List<String>,
    counters: Map<u64>,
}

#[derive(Debug, PartialEq, tabulary::Document)]
#[tabulary(rename_all = "snake_case")]
struct Gift {
    message: String,
    wrap_count: Option<u64>,
}

#[derive(Debug, PartialEq, tabulary::Record)]
#[tabulary(rename_all = "snake_case",
    index(ident = ByCarrier, name = "by_carrier", kind = global, partition = region,
        project = include(shipping)))]
struct Order {
    #[tabulary(partition_key)]
    order_ref: String,
    region: String,
    shipping: Shipping,
    gift: Option<Gift>,
    #[tabulary(rename = "data")]
    payload: Option<String>,
}

fn s(value: &str) -> Av {
    Av::S(value.into())
}
fn n(value: &str) -> Av {
    Av::N(value.into())
}
fn names(pairs: &[(&str, &str)]) -> HashMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).into(), (*v).into()))
        .collect()
}
fn values(pairs: Vec<(&str, Av)>) -> HashMap<String, Av> {
    pairs.into_iter().map(|(k, v)| (k.into(), v)).collect()
}
fn check(condition: Condition<Order>, text: &str, expected: &[(&str, &str)], bound: Vec<Av>) {
    let rendered = condition.build().unwrap().render();
    assert_eq!(rendered.condition(), Some(text));
    assert_eq!(rendered.names(), &names(expected));
    let bound = bound
        .into_iter()
        .enumerate()
        .map(|(i, v)| (format!(":v{i}"), v))
        .collect::<HashMap<_, _>>();
    assert_eq!(rendered.values(), &bound);
}
fn shipping() -> Shipping {
    Shipping {
        carrier: "parcel-post".into(),
        status: "packed".into(),
        parcel_size: 3,
        attempts: 0,
        weight: 1.5,
        note: None,
        address: Address {
            city: "Portland".into(),
            postal_code: "97201".into(),
            recipient: "Front desk".into(),
        },
        tags: StringSet::try_from(vec!["fragile".into()]).unwrap(),
        labels: List(vec!["a".into()]),
        counters: Map(Default::default()),
    }
}

const CITY: [(&str, &str); 3] = [("#n0", "shipping"), ("#n1", "address"), ("#n2", "city")];

#[test]
fn every_condition_on_a_nested_path_aliases_each_segment() {
    let ship = || OrderAttrs::SHIPPING;
    let carrier = [("#n0", "shipping"), ("#n1", "carrier")];
    let size = [("#n0", "shipping"), ("#n1", "size")];
    check(
        ship()
            .field(ShippingAttrs::CARRIER)
            .eq("parcel-post".into()),
        "#n0.#n1 = :v0",
        &carrier,
        vec![s("parcel-post")],
    );
    check(
        ship().field(ShippingAttrs::CARRIER).ne("courier".into()),
        "#n0.#n1 <> :v0",
        &carrier,
        vec![s("courier")],
    );
    for (condition, op) in [
        (ship().field(ShippingAttrs::PARCEL_SIZE).lt(4), "<"),
        (ship().field(ShippingAttrs::PARCEL_SIZE).le(4), "<="),
        (ship().field(ShippingAttrs::PARCEL_SIZE).gt(4), ">"),
        (ship().field(ShippingAttrs::PARCEL_SIZE).ge(4), ">="),
    ] {
        check(condition, &format!("#n0.#n1 {op} :v0"), &size, vec![n("4")]);
    }
    check(
        ship().field(ShippingAttrs::PARCEL_SIZE).between(1, 9),
        "#n0.#n1 BETWEEN :v0 AND :v1",
        &size,
        vec![n("1"), n("9")],
    );
    check(
        ship()
            .field(ShippingAttrs::PARCEL_SIZE)
            .in_values(vec![1, 2]),
        "#n0.#n1 IN (:v0, :v1)",
        &size,
        vec![n("1"), n("2")],
    );
    check(
        ship()
            .field(ShippingAttrs::CARRIER)
            .begins_with("parcel".into()),
        "begins_with(#n0.#n1, :v0)",
        &carrier,
        vec![s("parcel")],
    );
    check(
        ship().field(ShippingAttrs::TAGS).contains("fragile".into()),
        "contains(#n0.#n1, :v0)",
        &[("#n0", "shipping"), ("#n1", "tags")],
        vec![s("fragile")],
    );
    check(
        ship().field(ShippingAttrs::LABELS).contains("a".into()),
        "contains(#n0.#n1, :v0)",
        &[("#n0", "shipping"), ("#n1", "labels")],
        vec![s("a")],
    );
    check(
        ship().field(ShippingAttrs::CARRIER).size().ge(3),
        "size(#n0.#n1) >= :v0",
        &carrier,
        vec![n("3")],
    );
    check(
        ship().field(ShippingAttrs::NOTE).exists(),
        "attribute_exists(#n0.#n1)",
        &[("#n0", "shipping"), ("#n1", "note")],
        vec![],
    );
    check(
        ship().field(ShippingAttrs::NOTE).not_exists(),
        "attribute_not_exists(#n0.#n1)",
        &[("#n0", "shipping"), ("#n1", "note")],
        vec![],
    );
    check(
        ship()
            .field(ShippingAttrs::ADDRESS)
            .field(AddressAttrs::CITY)
            .eq_path(ship().field(ShippingAttrs::CARRIER)),
        "#n0.#n1.#n2 = #n0.#n3",
        &[CITY[0], CITY[1], CITY[2], ("#n3", "carrier")],
        vec![],
    );
}

#[test]
fn arbitrary_depth_renames_and_reserved_words_are_all_aliased() {
    let city = OrderAttrs::SHIPPING
        .field(ShippingAttrs::ADDRESS)
        .field(AddressAttrs::CITY);
    check(
        city.eq("Portland".into()),
        "#n0.#n1.#n2 = :v0",
        &CITY,
        vec![s("Portland")],
    );
    let condition = OrderAttrs::SHIPPING
        .field(ShippingAttrs::STATUS)
        .eq("packed".into())
        .and(
            OrderAttrs::SHIPPING
                .field(ShippingAttrs::ADDRESS)
                .field(AddressAttrs::RECIPIENT)
                .begins_with("Front".into()),
        )
        .and(
            OrderAttrs::SHIPPING
                .field(ShippingAttrs::ADDRESS)
                .field(AddressAttrs::POSTAL_CODE)
                .ne("00000".into()),
        )
        .or(OrderAttrs::PAYLOAD.exists());
    check(
        condition,
        "(((#n0.#n1 = :v0 AND begins_with(#n0.#n2.#n3, :v1)) AND #n0.#n2.#n4 <> :v2) OR attribute_exists(#n5))",
        &[
            ("#n0", "shipping"),
            ("#n1", "status"),
            ("#n2", "address"),
            ("#n3", "name"),
            ("#n4", "postalCode"),
            ("#n5", "data"),
        ],
        vec![s("packed"), s("Front"), s("00000")],
    );
}

#[test]
fn built_trees_carry_the_complete_physical_path() {
    let built = OrderAttrs::SHIPPING
        .field(ShippingAttrs::ADDRESS)
        .field(AddressAttrs::CITY)
        .size()
        .gt(0)
        .build()
        .unwrap();
    let path: tabulary::AttributePath = ["shipping", "address", "city"]
        .map(|name| PathSegment::Attribute(name.into()))
        .to_vec()
        .into();
    assert_eq!(
        built.root(),
        &ConditionNode::Predicate {
            operator: Operator::Gt,
            operands: vec![Operand::PathSize(path.clone()), Operand::Value(n("0"))],
        }
    );
    assert_eq!(
        format!("{:?}", Operand::Path(path)),
        r#"Path(AttributePath([Attribute("shipping"), Attribute("address"), Attribute("city")]))"#
    );
}

#[test]
fn constant_encoding_errors_name_the_nested_document_and_path() {
    let error = OrderAttrs::SHIPPING
        .field(ShippingAttrs::WEIGHT)
        .gt(f64::NAN)
        .build()
        .unwrap_err();
    let BuildError::Encode(error) = error else {
        panic!("expected an encode error, got {error:?}");
    };
    assert_eq!(error.kind, EncodeFault::NonFiniteFloat);
    assert_eq!(error.record, "Order");
    assert_eq!(error.document, "Shipping");
    assert_eq!(
        error.path.segments(),
        &[
            PathSegment::Attribute("shipping".into()),
            PathSegment::Attribute("weight".into())
        ]
    );
}

#[test]
fn nested_updates_render_set_if_not_exists_add_and_remove() {
    let update = Update::new()
        .and(
            OrderAttrs::SHIPPING
                .field(ShippingAttrs::CARRIER)
                .set("courier".into()),
        )
        .and(
            OrderAttrs::SHIPPING
                .field(ShippingAttrs::ADDRESS)
                .field(AddressAttrs::CITY)
                .set_if_not_exists("Portland".into()),
        )
        .and(OrderAttrs::SHIPPING.field(ShippingAttrs::ATTEMPTS).add(1))
        .and(
            OrderAttrs::SHIPPING
                .field(ShippingAttrs::TAGS)
                .add(StringSet::try_from(vec!["late".into()]).unwrap()),
        )
        .and(OrderAttrs::SHIPPING.field(ShippingAttrs::NOTE).remove())
        .and(
            OrderAttrs::SHIPPING
                .field(ShippingAttrs::COUNTERS)
                .entry(MapKey::new("scan.count").unwrap())
                .add(2),
        )
        .and(OrderAttrs::GIFT.field(GiftAttrs::WRAP_COUNT).remove())
        .and(OrderAttrs::REGION.set_if_not_exists("west".into()))
        .build()
        .unwrap();
    let rendered = update.render();
    assert_eq!(
        rendered.update(),
        Some(
            "SET #n0.#n1 = :v0, #n0.#n2.#n3 = if_not_exists(#n0.#n2.#n3, :v1), \
             #n4 = if_not_exists(#n4, :v2) \
             REMOVE #n0.#n5, #n6.#n7 \
             ADD #n0.#n8 :v3, #n0.#n9 :v4, #n0.#n10.#n11 :v5"
        )
    );
    assert_eq!(
        rendered.names(),
        &names(&[
            ("#n0", "shipping"),
            ("#n1", "carrier"),
            ("#n2", "address"),
            ("#n3", "city"),
            ("#n4", "region"),
            ("#n5", "note"),
            ("#n6", "gift"),
            ("#n7", "wrap_count"),
            ("#n8", "attempts"),
            ("#n9", "tags"),
            ("#n10", "counters"),
            ("#n11", "scan.count"),
        ])
    );
    assert_eq!(
        rendered.values(),
        &values(vec![
            (":v0", s("courier")),
            (":v1", s("Portland")),
            (":v2", s("west")),
            (":v3", n("1")),
            (":v4", Av::Ss(vec!["late".into()])),
            (":v5", n("2")),
        ])
    );
    let path = |names: &[&str]| -> tabulary::AttributePath {
        names
            .iter()
            .map(|name| PathSegment::Attribute((*name).into()))
            .collect::<Vec<_>>()
            .into()
    };
    assert_eq!(
        update.actions()[2],
        ActionNode::NestedAdd {
            target: path(&["shipping", "attempts"]),
            value: Operand::Value(n("1")),
        }
    );
}

#[test]
fn nested_add_and_whole_map_writes_overlap() {
    let error = Update::new()
        .and(OrderAttrs::SHIPPING.field(ShippingAttrs::ATTEMPTS).add(1))
        .and(OrderAttrs::SHIPPING.set(shipping()))
        .build()
        .unwrap_err();
    assert!(matches!(error, BuildError::OverlappingUpdate { .. }));
}

#[test]
fn optional_parent_maps_are_guarded_explicitly() {
    // A child predicate on an absent map is false (and not_exists true); the
    // caller states the parent requirement instead of relying on inference.
    let guard = OrderAttrs::GIFT
        .exists()
        .and(OrderAttrs::GIFT.field(GiftAttrs::MESSAGE).ne(String::new()));
    check(
        guard,
        "(attribute_exists(#n0) AND #n0.#n1 <> :v0)",
        &[("#n0", "gift"), ("#n1", "message")],
        vec![s("")],
    );
    let table = Table::<Order>::new("orders").unwrap();
    let plan = table
        .update(OrderKey::new("order-1".into()).unwrap())
        .changes(
            Update::new().and(
                OrderAttrs::GIFT
                    .field(GiftAttrs::MESSAGE)
                    .set("Happy birthday".into()),
            ),
        )
        .condition(OrderAttrs::GIFT.exists())
        .build()
        .unwrap();
    let input = plan.input();
    assert_eq!(input.update_expression(), Some("SET #n0.#n1 = :v0"));
    assert_eq!(input.condition_expression(), Some("attribute_exists(#n0)"));
    assert_eq!(
        input.expression_attribute_names(),
        Some(&names(&[("#n0", "gift"), ("#n1", "message")]))
    );
    assert_eq!(
        input.expression_attribute_values(),
        Some(&values(vec![(":v0", s("Happy birthday"))]))
    );
    // The parent map may also be written whole, which needs no guard.
    let whole = table
        .update(OrderKey::new("order-1".into()).unwrap())
        .changes(Update::new().and(OrderAttrs::GIFT.set(Gift {
            message: "Hi".into(),
            wrap_count: None,
        })))
        .build()
        .unwrap();
    assert_eq!(whole.input().update_expression(), Some("SET #n0 = :v0"));
}

#[test]
fn update_item_inputs_share_one_alias_space_for_nested_guards() {
    let plan = Table::<Order>::new("orders")
        .unwrap()
        .update(OrderKey::new("order-1".into()).unwrap())
        .changes(
            Update::new()
                .and(
                    OrderAttrs::SHIPPING
                        .field(ShippingAttrs::STATUS)
                        .set("shipped".into()),
                )
                .and(OrderAttrs::SHIPPING.field(ShippingAttrs::ATTEMPTS).add(1)),
        )
        .condition(
            OrderAttrs::SHIPPING
                .field(ShippingAttrs::STATUS)
                .eq("packed".into()),
        )
        .build()
        .unwrap();
    let input = plan.input();
    assert_eq!(input.table_name(), Some("orders"));
    assert_eq!(
        input.update_expression(),
        Some("SET #n0.#n1 = :v0 ADD #n0.#n2 :v1")
    );
    assert_eq!(input.condition_expression(), Some("#n0.#n1 = :v2"));
    assert_eq!(
        input.expression_attribute_names(),
        Some(&names(&[
            ("#n0", "shipping"),
            ("#n1", "status"),
            ("#n2", "attempts")
        ]))
    );
    assert_eq!(
        input.expression_attribute_values(),
        Some(&values(vec![
            (":v0", s("shipped")),
            (":v1", n("1")),
            (":v2", s("packed")),
        ]))
    );
    assert_eq!(
        Some(plan.expressions().names()),
        input.expression_attribute_names()
    );
}

fn scan_output(key: Option<Item>) -> ScanOutput {
    ScanOutput::builder()
        .count(0)
        .scanned_count(0)
        .set_items(Some(vec![]))
        .set_last_evaluated_key(key)
        .build()
}

#[test]
fn nested_filters_reach_scan_inputs_and_bind_cursors() {
    let table = Table::<Order>::new("orders").unwrap();
    let filter = || {
        OrderAttrs::SHIPPING
            .field(ShippingAttrs::ADDRESS)
            .field(AddressAttrs::CITY)
            .eq("Portland".into())
            .and(OrderAttrs::SHIPPING.field(ShippingAttrs::TAGS).size().gt(0))
    };
    let plan = table.scan(Strong).filter(filter()).build().unwrap();
    let input = plan.input();
    assert_eq!(
        input.filter_expression(),
        Some("(#n0.#n1.#n2 = :v0 AND size(#n0.#n3) > :v1)")
    );
    assert_eq!(
        input.expression_attribute_names(),
        Some(&names(&[CITY[0], CITY[1], CITY[2], ("#n3", "tags")]))
    );
    let key: Item = [("order_ref".into(), s("order-1"))].into();
    let cursor = plan
        .decode(Ok(scan_output(Some(key.clone()))))
        .unwrap()
        .next;
    let bytes = cursor.unwrap().to_bytes().unwrap();
    let cursor = Cursor::<Order, ()>::from_bytes(&bytes).unwrap();
    let resumed = table
        .scan(Strong)
        .filter(filter())
        .resume(cursor.clone())
        .limit(NonZeroU32::new(5).unwrap())
        .build()
        .unwrap();
    assert_eq!(resumed.input().exclusive_start_key, Some(key));
    // A different nested path is a different filter.
    let other = OrderAttrs::SHIPPING
        .field(ShippingAttrs::ADDRESS)
        .field(AddressAttrs::POSTAL_CODE)
        .eq("Portland".into())
        .and(OrderAttrs::SHIPPING.field(ShippingAttrs::TAGS).size().gt(0));
    assert!(matches!(
        table.scan(Strong).filter(other).resume(cursor).build(),
        Err(BuildError::CursorMismatch)
    ));
}

#[test]
fn index_filters_check_the_nested_root_against_the_projection() {
    #[derive(tabulary::Projection)]
    #[tabulary(record = Order)]
    struct Region {
        #[tabulary(attribute = OrderAttrs::REGION)]
        region: String,
    }
    let index = || {
        Table::<Order>::new("orders")
            .unwrap()
            .index(OrderIndexes::BY_CARRIER)
            .scan(Eventual)
            .select::<Region>()
    };
    let included = index()
        .filter(OrderAttrs::SHIPPING.field(ShippingAttrs::CARRIER).exists())
        .build();
    assert!(included.is_ok());
    let item: Item = [("region".into(), s("west"))].into();
    let view = <Region as tabulary::Projection>::from_item(&item).unwrap();
    assert_eq!(view.region, "west");
    let missing = index()
        .filter(OrderAttrs::GIFT.field(GiftAttrs::MESSAGE).exists())
        .build();
    assert!(matches!(
        missing,
        Err(BuildError::OperationConstraint {
            code: "projection_not_available"
        })
    ));
}

#[test]
fn nested_paths_with_wrong_types_or_capabilities_do_not_compile() {
    trybuild::TestCases::new().compile_fail("tests/ui/nested_paths/*.rs");
}
