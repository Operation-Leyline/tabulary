#![cfg(feature = "derive")]

use aws_sdk_dynamodb::types::AttributeValue as Av;
use tabulary::{Condition, ReadConsistency::Strong, Table};

#[derive(tabulary::Variants)]
#[tabulary(tag = "delivery.stage", rename_all = "snake_case")]
enum Stage {
    Sent {
        #[tabulary(rename = "tracking.ref")]
        tracking: String,
        note: Option<String>,
        count: u64,
        #[tabulary(with = tabulary::convert::u128_string)]
        external: u128,
    },
    Failed {
        note: Option<String>,
    },
}
#[derive(tabulary::Record)]
#[tabulary(rename_all = "snake_case")]
struct Delivery {
    #[tabulary(partition_key)]
    id: String,
    #[tabulary(flatten)]
    stage: Stage,
    quantity: u64,
}
fn check(condition: Condition<Delivery>, text: &str, names: &[&str], values: Vec<Av>) {
    let rendered = condition.build().unwrap().render();
    assert_eq!(rendered.condition(), Some(text));
    assert_eq!(
        *rendered.names(),
        names
            .iter()
            .enumerate()
            .map(|(i, n)| (format!("#n{i}"), (*n).into()))
            .collect()
    );
    assert_eq!(
        *rendered.values(),
        values
            .into_iter()
            .enumerate()
            .map(|(i, v)| (format!(":v{i}"), v))
            .collect()
    );
}
fn sent() -> StageSentAttrs<Delivery> {
    DeliveryAttrs::STAGE.variant(StageTags::SENT)
}
fn s(value: &str) -> Av {
    Av::S(value.into())
}
fn n(value: &str) -> Av {
    Av::N(value.into())
}

#[test]
fn predicates_use_payload_wire_names_types_and_codecs_with_a_guard() {
    for (condition, text, value) in [
        (
            sent().tracking.eq("parcel".into()),
            "#n0 = :v0",
            s("parcel"),
        ),
        (
            sent().tracking.ne("parcel".into()),
            "#n0 <> :v0",
            s("parcel"),
        ),
        (
            sent().tracking.begins_with("par".into()),
            "begins_with(#n0, :v0)",
            s("par"),
        ),
        (
            sent().tracking.contains("ar".into()),
            "contains(#n0, :v0)",
            s("ar"),
        ),
        (sent().tracking.size().ge(3), "size(#n0) >= :v0", n("3")),
    ] {
        check(
            condition,
            &format!("({text} AND #n1 = :v1)"),
            &["tracking.ref", "delivery.stage"],
            vec![value, s("sent")],
        );
    }
    for (condition, text) in [
        (sent().note.exists(), "attribute_exists(#n0)"),
        (sent().note.not_exists(), "attribute_not_exists(#n0)"),
    ] {
        check(
            condition,
            &format!("({text} AND #n1 = :v0)"),
            &["note", "delivery.stage"],
            vec![s("sent")],
        );
    }
    check(
        sent().count.between(2, 4),
        "(#n0 BETWEEN :v0 AND :v1 AND #n1 = :v2)",
        &["count", "delivery.stage"],
        vec![n("2"), n("4"), s("sent")],
    );
    check(
        sent().external.eq(u128::MAX),
        "(#n0 = :v0 AND #n1 = :v1)",
        &["external", "delivery.stage"],
        vec![s(&u128::MAX.to_string()), s("sent")],
    );
    check(
        sent().count.in_values(vec![2, 4]),
        "(#n0 IN (:v0, :v1) AND #n1 = :v2)",
        &["count", "delivery.stage"],
        vec![n("2"), n("4"), s("sent")],
    );
    check(
        DeliveryAttrs::STAGE
            .variant(StageTags::FAILED)
            .note
            .exists(),
        "(attribute_exists(#n0) AND #n1 = :v0)",
        &["note", "delivery.stage"],
        vec![s("failed")],
    );
}

#[test]
fn composition_never_negates_or_bypasses_the_scope() {
    check(
        sent().note.eq_path(sent().tracking).not(),
        "((NOT (#n0 = #n1) AND #n2 = :v0) AND #n2 = :v1)",
        &["note", "tracking.ref", "delivery.stage"],
        vec![s("sent"), s("sent")],
    );
    check(
        sent().note.exists().or(DeliveryAttrs::STAGE
            .variant(StageTags::FAILED)
            .note
            .exists()),
        "(((attribute_exists(#n0) OR attribute_exists(#n0)) AND #n1 = :v0) AND #n1 = :v1)",
        &["note", "delivery.stage"],
        vec![s("sent"), s("failed")],
    );
    check(
        sent().tracking.eq("parcel".into()).not(),
        "(NOT (#n0 = :v0) AND #n1 = :v1)",
        &["tracking.ref", "delivery.stage"],
        vec![s("parcel"), s("sent")],
    );
    check(
        sent().note.exists().or(DeliveryAttrs::QUANTITY.eq(1)).not(),
        "(NOT ((attribute_exists(#n0) OR #n1 = :v0)) AND #n2 = :v1)",
        &["note", "quantity", "delivery.stage"],
        vec![n("1"), s("sent")],
    );
    check(
        DeliveryAttrs::QUANTITY
            .eq(1)
            .and(sent().note.exists())
            .not()
            .not(),
        "(NOT (NOT ((#n0 = :v0 AND attribute_exists(#n1)))) AND #n2 = :v1)",
        &["quantity", "note", "delivery.stage"],
        vec![n("1"), s("sent")],
    );
    check(
        sent().note.exists().and(sent().count.gt(2)),
        "(((attribute_exists(#n0) AND #n1 > :v0) AND #n2 = :v1) AND #n2 = :v2)",
        &["note", "count", "delivery.stage"],
        vec![n("2"), s("sent"), s("sent")],
    );
}

#[test]
fn scoped_conditions_feed_write_query_and_scan_builders() {
    let table = Table::<Delivery>::new("deliveries").unwrap();
    let predicate = || sent().note.exists().not();
    let expected = predicate().build().unwrap().render();
    let write = table
        .delete(DeliveryKey::new("parcel".into()).unwrap())
        .condition(predicate())
        .build()
        .unwrap();
    assert_eq!(write.expressions().condition(), expected.condition());
    assert_eq!(write.expressions().names(), expected.names());
    assert_eq!(write.expressions().values(), expected.values());
    let scan = table.scan(Strong).filter(predicate()).build().unwrap();
    assert_eq!(scan.expressions().filter(), expected.condition());
    assert_eq!(scan.expressions().names(), expected.names());
    assert_eq!(scan.expressions().values(), expected.values());
    let query = table
        .query(Delivery::primary().partition("parcel".into()), Strong)
        .filter(predicate())
        .build()
        .unwrap();
    assert_eq!(
        query.expressions().filter(),
        Some("(NOT (attribute_exists(#n1)) AND #n2 = :v1)")
    );
    assert_eq!(
        *query.expressions().names(),
        [
            ("#n0".into(), "id".into()),
            ("#n1".into(), "note".into()),
            ("#n2".into(), "delivery.stage".into())
        ]
        .into()
    );
    assert_eq!(
        *query.expressions().values(),
        [(":v0".into(), s("parcel")), (":v1".into(), s("sent"))].into()
    );
}

#[test]
fn scoped_handles_reject_invalid_types_fields_and_mutations() {
    trybuild::TestCases::new().compile_fail("tests/ui/scoped_variants/*.rs");
}

#[test]
fn generics_renamed_runtime_recursive_self_and_marker_names_work() {
    use tabulary as renamed;
    #[derive(renamed::Variants)]
    #[tabulary(
        tag = "kind",
        rename_all = "snake_case",
        generated_prefix = "Payload",
        crate = "renamed"
    )]
    enum State<T> {
        Active {
            value: T,
            __tabulary_marker: u64,
            children: renamed::List<Self>,
        },
        Empty,
    }
    #[derive(renamed::Record)]
    #[tabulary(rename_all = "snake_case", crate = "renamed")]
    struct Parcel<T> {
        #[tabulary(partition_key)]
        id: String,
        #[tabulary(flatten)]
        state: State<T>,
    }
    let _: PayloadEmptyAttrs<Parcel<String>, String> =
        ParcelAttrs::<String>::STATE.variant(PayloadTags::EMPTY);
    let active = || ParcelAttrs::<String>::STATE.variant(PayloadTags::ACTIVE);
    assert_eq!(
        active()
            .value
            .eq("ready".into())
            .build()
            .unwrap()
            .render()
            .values()[":v0"],
        s("ready")
    );
    assert!(active().__tabulary_marker.eq(1).build().is_ok());
    assert!(active().children.contains(State::Empty).build().is_ok());
    let error = ParcelAttrs::<f64>::STATE
        .variant(PayloadTags::ACTIVE)
        .value
        .eq(f64::NAN)
        .build()
        .unwrap_err();
    let tabulary::BuildError::Encode(error) = error else {
        panic!("typed encode fault")
    };
    assert_eq!(error.kind, tabulary::EncodeFault::NonFiniteFloat);
    assert_eq!((error.record, error.attribute()), ("Parcel", Some("value")));
    assert!(
        ParcelAttrs::<String>::STATE
            .is(PayloadTag::Active)
            .build()
            .is_ok()
    );
}
