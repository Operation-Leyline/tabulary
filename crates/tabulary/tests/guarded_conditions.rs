#![cfg(feature = "derive")]
use aws_sdk_dynamodb::types::AttributeValue as Av;
use tabulary::{Condition, ScopedAttr};

#[derive(tabulary::Variants)]
#[tabulary(tag = "stage", rename_all = "snake_case")]
enum Stage {
    Sent { note: String },
    Failed { note: String },
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
// Manual bindings exercise the runtime before the dependent derive integration.
fn note(tag: &'static str) -> ScopedAttr<Delivery, String> {
    tabulary::__private::scoped::<
        Delivery,
        StageTag,
        String,
        tabulary::Required,
        tabulary::Native<String>,
    >(DeliveryAttrs::STAGE, tag, "note")
}
fn s(value: &str) -> Av {
    Av::S(value.into())
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
#[test]
fn guarded_leaves_preserve_boolean_branches_and_negation() {
    check(
        note("sent").exists().or(note("failed").exists()),
        "((attribute_exists(#n0) AND #n1 = :v0) OR (attribute_exists(#n0) AND #n1 = :v1))",
        &["note", "stage"],
        vec![s("sent"), s("failed")],
    );
    check(
        note("sent")
            .exists()
            .or(DeliveryAttrs::QUANTITY.eq(1))
            .not(),
        "((NOT (attribute_exists(#n0)) AND #n1 = :v0) AND NOT (#n2 = :v1))",
        &["note", "stage", "quantity"],
        vec![s("sent"), Av::N("1".into())],
    );
    check(
        note("sent").size().ge(3).not(),
        "(NOT (size(#n0) >= :v0) AND #n1 = :v1)",
        &["note", "stage"],
        vec![Av::N("3".into()), s("sent")],
    );
    check(
        note("sent").exists().not().not(),
        "(attribute_exists(#n0) AND #n1 = :v0)",
        &["note", "stage"],
        vec![s("sent")],
    );
}
