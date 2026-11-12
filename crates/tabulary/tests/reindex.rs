#![cfg(feature = "derive")]
use aws_sdk_dynamodb::types::AttributeValue as Av;
use tabulary::{BuildError, EncodeFault, KeyFormatFault, Table, Update};

#[derive(tabulary::Record)]
#[tabulary(rename_all = "snake_case", partition(name = "pk", template = "order!{id:hex}"),
    index(ident = ByRoute, name = "by_route", kind = global, sparse,
        partition(name = "gpk", template = "route!{route:hex}"),
        sort(name = "gsk", template = "{position:dec20}!{id:hex}"), project = all))]
struct Parcel {
    id: String,
    route: Option<String>,
    position: u64,
    note: String,
    #[tabulary(version)]
    revision: u64,
}
fn changes(route: Option<String>, id: &str) -> Update<Parcel> {
    Update::new().reindex(
        ParcelIndexes::BY_ROUTE,
        ParcelByRouteInputs {
            route,
            position: 7,
            id: id.into(),
        },
    )
}
fn request(update: Update<Parcel>) -> Result<tabulary::PreparedUpdate<Parcel>, BuildError> {
    Table::<Parcel>::new("parcels")
        .unwrap()
        .update(ParcelKey::new("o".into()).unwrap())
        .expect_version(1)
        .changes(update)
        .build()
}
#[test]
fn full_inputs_expand_through_the_existing_update_adapter_and_version_policy() {
    let prepared =
        request(changes(Some("r".into()), "o").and(ParcelAttrs::NOTE.set("ready".into()))).unwrap();
    let e = prepared.expressions();
    assert_eq!(
        e.update(),
        Some("SET #n0 = :v0, #n1 = :v1, #n2 = :v2, #n3 = :v3, #n4 = :v4, #n5 = :v5")
    );
    assert_eq!(e.condition(), Some("#n1 = :v6"));
    assert_eq!(
        e.names(),
        &[
            ("#n0".into(), "note".into()),
            ("#n1".into(), "revision".into()),
            ("#n2".into(), "route".into()),
            ("#n3".into(), "position".into()),
            ("#n4".into(), "gpk".into()),
            ("#n5".into(), "gsk".into()),
        ]
        .into()
    );
    assert_eq!(
        e.values(),
        &[
            (":v0".into(), Av::S("ready".into())),
            (":v1".into(), Av::N("2".into())),
            (":v2".into(), Av::S("r".into())),
            (":v3".into(), Av::N("7".into())),
            (":v4".into(), Av::S("route!72".into())),
            (":v5".into(), Av::S("00000000000000000007!6f".into())),
            (":v6".into(), Av::N("1".into())),
        ]
        .into()
    );
    assert_eq!(
        prepared.input().key().unwrap()["pk"],
        Av::S("order!6f".into())
    );
    assert!(prepared.into_tx().is_ok());
    let sparse = request(changes(None, "o")).unwrap();
    assert_eq!(
        sparse.expressions().update(),
        Some("SET #n0 = :v0, #n1 = :v1 REMOVE #n2, #n3, #n4")
    );
    assert_eq!(sparse.expressions().names()["#n2"], "route");
    assert_eq!(sparse.expressions().names()["#n3"], "gpk");
    assert_eq!(sparse.expressions().names()["#n4"], "gsk");
}
#[test]
fn primary_sources_must_match_the_operation_key_and_are_never_mutated() {
    assert!(matches!(
        request(changes(None, "different")),
        Err(BuildError::OperationConstraint {
            code: "primary_source_mutation"
        })
    ));
    assert!(matches!(
        changes(None, "o").build(),
        Err(BuildError::OperationConstraint {
            code: "reindex_requires_key"
        })
    ));
    assert!(matches!(
        request(changes(None, "o").reindex(
            ParcelIndexes::BY_ROUTE,
            ParcelByRouteInputs::new(None, 8, "o".into())
        )),
        Err(BuildError::ConflictingReindex)
    ));
}

#[derive(tabulary::Record)]
#[tabulary(rename_all = "snake_case",
    index(ident = ByPosition, name = "by_position", kind = global, sparse,
        partition(name = "gpk", template = "{route:text}!end"), sort = position, project = all),
    index(ident = ByLabel, name = "by_label", kind = global, sparse,
        partition(name = "gpk", template = "{route:text}!end"), sort = label, project = all))]
struct Shared {
    #[tabulary(partition_key)]
    id: String,
    route: String,
    position: Option<u64>,
    label: Option<String>,
}
fn shared(position: Option<u64>, label: Option<String>, route: &str) -> Update<Shared> {
    Update::new()
        .reindex(
            SharedIndexes::BY_POSITION,
            SharedByPositionInputs::new("r".into(), position),
        )
        .reindex(
            SharedIndexes::BY_LABEL,
            SharedByLabelInputs::new(route.into(), label),
        )
}
#[test]
fn shared_dependencies_require_complete_agreeing_inputs_and_union_membership() {
    assert!(matches!(
        Update::<Shared>::new()
            .reindex(
                SharedIndexes::BY_POSITION,
                SharedByPositionInputs::new("r".into(), None)
            )
            .build(),
        Err(BuildError::IncompleteReindex)
    ));
    assert!(matches!(
        shared(None, None, "other").build(),
        Err(BuildError::ConflictingReindex)
    ));
    for position in [None, Some(1)] {
        for label in [None, Some("label".into())] {
            let built = shared(position, label.clone(), "r").build().unwrap();
            let e = built.render();
            assert_eq!(
                e.values()
                    .values()
                    .filter(|v| **v == Av::S("r!end".into()))
                    .count(),
                usize::from(position.is_some() || label.is_some())
            );
            assert_eq!(built.actions().len(), 4); // each shared source/key appears once
        }
    }
    assert!(matches!(
        shared(None, None, "r")
            .and(SharedAttrs::POSITION.remove())
            .build(),
        Err(BuildError::OverlappingUpdate { .. })
    ));
}
#[test]
fn present_invalid_inputs_fail_even_when_another_input_prevents_membership() {
    let update = Update::<Shared>::new().reindex(
        SharedIndexes::BY_POSITION,
        SharedByPositionInputs::new("bad!route".into(), None),
    );
    assert!(!format!("{update:?}").contains("bad!route"));
    let Err(BuildError::Encode(error)) = update.build() else {
        panic!("invalid component")
    };
    assert_eq!(error.record, "Shared");
    assert_eq!(error.attribute(), Some("gpk"));
    assert_eq!(
        error.kind,
        EncodeFault::KeyFormat(KeyFormatFault::InvalidSeparator)
    );
}
#[test]
fn generic_renamed_inputs_and_simple_reused_keys_keep_their_types() {
    use tabulary as renamed;
    #[derive(renamed::Record)]
    #[tabulary(rename_all = "snake_case", crate = "renamed", generated_prefix = "Delivery",
        index(ident = ByValue, name = "by_value", kind = global, sparse,
            partition(name = "gpk", template = "{value:hex}"), project = all))]
    struct Generic<T> {
        #[tabulary(partition_key)]
        id: String,
        value: Option<T>,
    }
    let e = Update::<Generic<[u8; 2]>>::new()
        .reindex(
            DeliveryIndexes::<[u8; 2]>::BY_VALUE,
            DeliveryByValueInputs::new(Some([0, 255])),
        )
        .build()
        .unwrap()
        .render();
    assert_eq!(e.values()[":v1"], Av::S("00ff".into()));
    #[derive(tabulary::Record)]
    #[tabulary(rename_all = "snake_case", index(ident = ByRoute, name = "by_route", kind = global,
        sparse, partition = route, sort = id, project = all))]
    struct Simple {
        #[tabulary(partition_key)]
        id: String,
        route: Option<String>,
    }
    let table = Table::<Simple>::new("parcels").unwrap();
    let update = |route| {
        Update::new().reindex(
            SimpleIndexes::BY_ROUTE,
            SimpleByRouteInputs::new(route, "o".into()),
        )
    };
    let build = |u| {
        table
            .update(SimpleKey::new("o".into()).unwrap())
            .changes(u)
            .build()
    };
    assert_eq!(
        build(update(None)).unwrap().expressions().update(),
        Some("REMOVE #n0")
    );
    assert!(
        matches!(build(update(Some(String::new()))), Err(BuildError::Encode(e)) if e.kind == EncodeFault::EmptyKey)
    );
}
#[test]
fn complete_dependency_closure_includes_indirectly_affected_indexes() {
    #[derive(tabulary::Record)]
    #[tabulary(rename_all = "snake_case",
        index(ident = ByRoute, name = "by_route", kind = global, partition = route, project = all),
        index(ident = ByBoth, name = "by_both", kind = global, partition = route, sort = position, project = all),
        index(ident = ByPosition, name = "by_position", kind = global, partition = position, project = all))]
    struct Link {
        #[tabulary(partition_key)]
        id: String,
        route: String,
        position: u64,
    }
    let initial =
        || Update::<Link>::new().reindex(LinkIndexes::BY_ROUTE, LinkByRouteInputs::new("r".into()));
    assert!(matches!(
        initial().build(),
        Err(BuildError::IncompleteReindex)
    ));
    let two = || initial().reindex(LinkIndexes::BY_BOTH, LinkByBothInputs::new("r".into(), 1));
    assert!(matches!(two().build(), Err(BuildError::IncompleteReindex)));
    let built = two()
        .reindex(LinkIndexes::BY_POSITION, LinkByPositionInputs::new(1))
        .build()
        .unwrap();
    assert_eq!(built.actions().len(), 2);
    assert_eq!(built.render().update(), Some("SET #n0 = :v0, #n1 = :v1"));
}
#[test]
fn incomplete_and_foreign_inputs_are_compile_errors() {
    trybuild::TestCases::new().compile_fail("tests/ui/reindex/*.rs");
}
