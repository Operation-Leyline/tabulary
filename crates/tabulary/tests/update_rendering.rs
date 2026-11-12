#![cfg(feature = "derive")]
use aws_sdk_dynamodb::types::AttributeValue as Av;
use tabulary::{BuildError, List, Map, MapKey, PathSegment, StringSet, Update};

#[derive(tabulary::Document)]
#[tabulary(rename_all = "snake_case")]
struct Line<T> {
    quantity: T,
    note: Option<String>,
}
#[derive(tabulary::Record)]
#[tabulary(rename_all = "snake_case")]
struct Order {
    #[tabulary(partition_key)]
    id: String,
    #[tabulary(rename = "gift.note")]
    note: Option<String>,
    line: Line<u64>,
    lines: List<Line<u64>>,
    quantities: Map<u64>,
    count: u64,
    tags: StringSet,
}

#[test]
fn canonical_groups_preserve_declaration_order_and_share_condition_aliases() {
    let update = Update::new()
        .and(OrderAttrs::TAGS.delete_members(StringSet::try_from(vec!["gift".into()]).unwrap()))
        .and(OrderAttrs::COUNT.add(2))
        .and(OrderAttrs::NOTE.remove())
        .and(OrderAttrs::LINE.field(LineAttrs::QUANTITY).set(7))
        .and(
            OrderAttrs::LINES
                .at(2)
                .field(LineAttrs::QUANTITY)
                .increment(1),
        )
        .build()
        .unwrap();
    let condition = OrderAttrs::COUNT.eq(2).build().unwrap();
    let rendered = update.render_with_condition(&condition);
    assert_eq!(
        rendered.update(),
        Some(
            "SET #n0.#n1 = :v0, #n2[2].#n1 = #n2[2].#n1 + :v1 REMOVE #n3 ADD #n4 :v2 DELETE #n5 :v3"
        )
    );
    assert_eq!(rendered.condition(), Some("#n4 = :v4"));
    assert_eq!(
        rendered.names(),
        &[
            ("#n0", "line"),
            ("#n1", "quantity"),
            ("#n2", "lines"),
            ("#n3", "gift.note"),
            ("#n4", "count"),
            ("#n5", "tags"),
        ]
        .into_iter()
        .map(|(k, v)| (k.into(), v.into()))
        .collect()
    );
    assert_eq!(
        rendered.values(),
        &[
            (":v0", Av::N("7".into())),
            (":v1", Av::N("1".into())),
            (":v2", Av::N("2".into())),
            (":v3", Av::Ss(vec!["gift".into()])),
            (":v4", Av::N("2".into())),
        ]
        .into_iter()
        .map(|(k, v)| (k.into(), v))
        .collect()
    );
    assert_eq!(update.render().update(), rendered.update());
    assert_eq!(update.render().values().len(), 4);
}

#[test]
fn list_and_dynamic_map_segments_are_not_parsed_or_reordered() {
    let update = Update::new()
        .and(
            OrderAttrs::QUANTITIES
                .entry(MapKey::new("a.b[0] #n0").unwrap())
                .set(3),
        )
        .and(OrderAttrs::LINES.at(10).remove_element())
        .and(OrderAttrs::LINES.at(1).remove_element())
        .and(OrderAttrs::LINE.field(LineAttrs::NOTE).remove())
        .build()
        .unwrap()
        .render();
    assert_eq!(
        update.update(),
        Some("SET #n0.#n1 = :v0 REMOVE #n2[10], #n2[1], #n3.#n4")
    );
    assert_eq!(update.names()["#n1"], "a.b[0] #n0");
    assert_eq!(update.values().len(), 1);
    assert!(MapKey::new("").is_err());
}

#[test]
fn duplicate_and_ancestor_targets_fail_in_both_directions() {
    for (first, second) in [
        (OrderAttrs::COUNT.set(1), OrderAttrs::COUNT.add(1)),
        (
            OrderAttrs::LINE.set(Line {
                quantity: 1,
                note: None,
            }),
            OrderAttrs::LINE.field(LineAttrs::NOTE).remove(),
        ),
        (
            OrderAttrs::LINES.at(0).field(LineAttrs::QUANTITY).set(1),
            OrderAttrs::LINES.at(0).remove_element(),
        ),
        (
            OrderAttrs::QUANTITIES
                .entry(MapKey::new("same").unwrap())
                .set(1),
            OrderAttrs::QUANTITIES
                .entry(MapKey::new("same").unwrap())
                .remove(),
        ),
    ] {
        assert!(matches!(
            Update::new().and(first).and(second).build(),
            Err(BuildError::OverlappingUpdate { .. })
        ));
    }
    let error = Update::new()
        .and(OrderAttrs::LINE.field(LineAttrs::QUANTITY).set(1))
        .and(OrderAttrs::LINE.set(Line {
            quantity: 1,
            note: None,
        }))
        .build()
        .unwrap_err();
    assert_eq!(
        error,
        BuildError::OverlappingUpdate {
            first: vec![
                PathSegment::Attribute("line".into()),
                PathSegment::Attribute("quantity".into())
            ]
            .into(),
            second: vec![PathSegment::Attribute("line".into())].into(),
        }
    );
}

#[test]
fn missing_parent_maps_are_never_initialized() {
    let update = Update::new()
        .and(
            OrderAttrs::LINE.field(LineAttrs::QUANTITY).set_expr(
                OrderAttrs::LINE
                    .field(LineAttrs::QUANTITY)
                    .if_not_exists(0)
                    .minus(1),
            ),
        )
        .build()
        .unwrap()
        .render();
    assert_eq!(
        update.update(),
        Some("SET #n0.#n1 = if_not_exists(#n0.#n1, :v0) - :v1")
    );
    assert_eq!(update.values().len(), 2);
    assert!(update.values().values().all(|v| matches!(v, Av::N(_))));
    let plain = Update::new()
        .and(OrderAttrs::LINE.field(LineAttrs::QUANTITY).increment(1))
        .build()
        .unwrap()
        .render();
    assert_eq!(plain.update(), Some("SET #n0.#n1 = #n0.#n1 + :v0"));
}

#[test]
fn malformed_nested_handles_and_indexes_are_structured_errors() {
    let forged: tabulary::Attr<Line<u64>, u64> = tabulary::__private::field("wrong", 0);
    assert_eq!(
        Update::new()
            .and(OrderAttrs::LINE.field(forged).set(1))
            .build()
            .unwrap_err(),
        BuildError::OperationConstraint {
            code: "invalid_attribute_descriptor"
        }
    );
    assert_eq!(
        Update::new()
            .and(OrderAttrs::LINES.at(-1).remove_element())
            .build()
            .unwrap_err(),
        BuildError::OperationConstraint {
            code: "invalid_list_index"
        }
    );
}

#[test]
fn top_level_defaults_append_and_secret_bindings_render_exactly() {
    let update = Update::new()
        .and(OrderAttrs::NOTE.set("private-note".into()))
        .and(OrderAttrs::COUNT.set_expr(OrderAttrs::COUNT.if_not_exists(0).plus(1)))
        .and(OrderAttrs::LINES.append_or_empty(List(vec![])))
        .build()
        .unwrap()
        .render();
    assert_eq!(
        update.update(),
        Some(
            "SET #n0 = :v0, #n1 = if_not_exists(#n1, :v1) + :v2, #n2 = list_append(if_not_exists(#n2, :v3), :v4)"
        )
    );
    assert_eq!(update.values().len(), 5);
    assert_eq!(update.values()[":v0"], Av::S("private-note".into()));
    assert_eq!(update.values()[":v3"], Av::L(vec![]));
    assert_eq!(update.values()[":v4"], Av::L(vec![]));
    assert!(!format!("{update:?}").contains("private-note"));
}

#[test]
fn nested_mutation_cannot_escape_an_immutable_root() {
    struct Envelope;
    const ROOT: tabulary::Attr<Envelope, Line<u64>> = tabulary::__private::field("root", 0);
    impl tabulary::Record for Envelope {
        const SCHEMA: tabulary::RecordSchema = tabulary::__private::schema(
            "Envelope",
            &[ROOT.descriptor()],
            tabulary::__private::keys(0, None),
        );
    }
    assert_eq!(
        Update::new()
            .and(ROOT.field(LineAttrs::QUANTITY).set(1))
            .build()
            .unwrap_err(),
        BuildError::OperationConstraint {
            code: "protected_update_path"
        }
    );
}
