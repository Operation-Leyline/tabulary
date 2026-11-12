use std::collections::HashMap;

use aws_sdk_dynamodb::types::AttributeValue as Av;
use tabulary::{
    __private, Attr, Condition, KeyOnly, Mutable, Record, RecordSchema, Required, Secret,
};

struct Order;
const ID: Attr<Order, String, Required, KeyOnly> = __private::field("orderRef", 0);
const COUNT: Attr<Order, u64> = __private::field("Size", 1);
const NOTE: Attr<Order, String> = __private::field("gift.note", 2);
const OTHER: Attr<Order, u64> = __private::field("quantity", 3);
const SECRET: Attr<Order, Secret<String>> = __private::field("secret", 4);
const EXTERNAL: Attr<Order, u128, Required, Mutable, tabulary::convert::u128_string::Codec> =
    __private::field("externalRef", 5);
impl Record for Order {
    const SCHEMA: RecordSchema = __private::schema(
        "Order",
        &[
            ID.descriptor(),
            COUNT.descriptor(),
            NOTE.descriptor(),
            OTHER.descriptor(),
            SECRET.descriptor(),
            EXTERNAL.descriptor(),
        ],
        __private::keys(0, None),
    );
}

fn check(condition: Condition<Order>, text: &str, names: &[&str], values: &[Av]) {
    let built = condition.build().unwrap();
    let rendered = built.render();
    assert_eq!(rendered.condition(), Some(text));
    assert_eq!(rendered.update(), None);
    assert_eq!(rendered.key_condition(), None);
    assert_eq!(rendered.filter(), None);
    assert_eq!(rendered.projection(), None);
    assert_eq!(
        rendered.names(),
        &names
            .iter()
            .enumerate()
            .map(|(i, n)| (format!("#n{i}"), (*n).into()))
            .collect::<HashMap<_, _>>()
    );
    assert_eq!(
        rendered.values(),
        &values
            .iter()
            .enumerate()
            .map(|(i, v)| (format!(":v{i}"), v.clone()))
            .collect::<HashMap<_, _>>()
    );
    let again = built.render();
    assert_eq!(rendered.condition(), again.condition());
    assert_eq!(rendered.names(), again.names());
    assert_eq!(rendered.values(), again.values());
}

fn n(value: &str) -> Av {
    Av::N(value.into())
}
fn s(value: &str) -> Av {
    Av::S(value.into())
}

#[test]
fn every_predicate_has_exact_text_and_only_used_bindings() {
    for (condition, operator) in [
        (COUNT.eq(7), "="),
        (COUNT.ne(7), "<>"),
        (COUNT.lt(7), "<"),
        (COUNT.le(7), "<="),
        (COUNT.gt(7), ">"),
        (COUNT.ge(7), ">="),
    ] {
        check(
            condition,
            &format!("#n0 {operator} :v0"),
            &["Size"],
            &[n("7")],
        );
    }
    check(NOTE.exists(), "attribute_exists(#n0)", &["gift.note"], &[]);
    check(
        NOTE.not_exists(),
        "attribute_not_exists(#n0)",
        &["gift.note"],
        &[],
    );
    check(
        COUNT.eq_path(OTHER),
        "#n0 = #n1",
        &["Size", "quantity"],
        &[],
    );
    check(
        COUNT.between(1, 9),
        "#n0 BETWEEN :v0 AND :v1",
        &["Size"],
        &[n("1"), n("9")],
    );
    check(
        NOTE.begins_with("gift".into()),
        "begins_with(#n0, :v0)",
        &["gift.note"],
        &[s("gift")],
    );
    check(
        NOTE.contains("card".into()),
        "contains(#n0, :v0)",
        &["gift.note"],
        &[s("card")],
    );
    check(
        NOTE.size().ge(3),
        "size(#n0) >= :v0",
        &["gift.note"],
        &[n("3")],
    );
    check(
        EXTERNAL.eq(u128::MAX),
        "#n0 = :v0",
        &["externalRef"],
        &[s(&u128::MAX.to_string())],
    );
}

#[test]
fn grouping_preserves_traversal_dedupes_names_and_keeps_equal_values_distinct() {
    check(
        OTHER.eq(7).or(COUNT.eq(7)).and(NOTE.exists().not()),
        "((#n0 = :v0 OR #n1 = :v1) AND NOT (attribute_exists(#n2)))",
        &["quantity", "Size", "gift.note"],
        &[n("7"), n("7")],
    );
    check(
        NOTE.eq("card".into())
            .and(NOTE.eq("card".into()).or(NOTE.exists())),
        "(#n0 = :v0 AND (#n0 = :v1 OR attribute_exists(#n0)))",
        &["gift.note"],
        &[s("card"), s("card")],
    );
    check(COUNT.eq_path(COUNT), "#n0 = #n0", &["Size"], &[]);
    check(
        COUNT.eq(7).not().not(),
        "NOT (NOT (#n0 = :v0))",
        &["Size"],
        &[n("7")],
    );
}

#[test]
fn in_values_keep_order_and_allocate_through_the_service_bound() {
    check(
        COUNT.in_values(vec![9]),
        "#n0 IN (:v0)",
        &["Size"],
        &[n("9")],
    );
    check(
        COUNT.in_values(vec![9, 2, 9]),
        "#n0 IN (:v0, :v1, :v2)",
        &["Size"],
        &[n("9"), n("2"), n("9")],
    );
    let placeholders = (0..100)
        .map(|i| format!(":v{i}"))
        .collect::<Vec<_>>()
        .join(", ");
    check(
        COUNT.in_values(vec![9; 100]),
        &format!("#n0 IN ({placeholders})"),
        &["Size"],
        &vec![n("9"); 100],
    );
}

#[test]
fn debug_redacts_secret_and_ordinary_values_but_explicit_access_retains_them() {
    let built = SECRET
        .eq(Secret::new("private-token".into()))
        .and(NOTE.eq("private-note".into()))
        .build()
        .unwrap();
    let rendered = built.render();
    for debug in [format!("{rendered:?}"), format!("{rendered:#?}")] {
        assert!(!debug.contains("private-token"));
        assert!(!debug.contains("private-note"));
        assert!(debug.contains("[redacted]"));
    }
    assert_eq!(rendered.values()[":v0"], s("private-token"));
    assert_eq!(rendered.values()[":v1"], s("private-note"));
}
