#![cfg(feature = "derive")]

use aws_sdk_dynamodb::types::AttributeValue as Av;
use tabulary::{BuildError, EncodeFault};

#[derive(tabulary::Record)]
#[tabulary(rename_all = "snake_case",
    index(ident = ByLabel, name = "by_label", kind = global, partition = label, sort = bytes, project = all),
    index(ident = ByCount, name = "by_count", kind = local, partition = id, sort = sequence, project = all),
    index(ident = ByShared, name = "by_shared", kind = global, partition = bytes, sort = label, project = all),
    index(ident = ByHash, name = "by_hash", kind = global, partition = label, project = all))]
struct Order {
    #[tabulary(partition_key, rename = "order.id")]
    id: String,
    #[tabulary(sort_key, rename = "size")]
    count: u64,
    label: String,
    bytes: Vec<u8>,
    sequence: u64,
}

#[test]
fn partition_equality_and_every_sort_operator_render_exact_bindings() {
    let partition = Order::primary()
        .partition("private-order".into())
        .build()
        .unwrap();
    let text = partition.render();
    assert_eq!(text.key_condition(), Some("#n0 = :v0"));
    assert_eq!(text.condition(), None);
    assert_eq!(text.names()["#n0"], "order.id");
    assert_eq!(text.values()[":v0"], Av::S("private-order".into()));
    assert!(!format!("{partition:?} {text:?}").contains("private-order"));
    for (condition, expected) in [
        (
            Order::primary().partition("o".into()).sort_eq(7),
            "#n1 = :v1",
        ),
        (
            Order::primary().partition("o".into()).sort_lt(7),
            "#n1 < :v1",
        ),
        (
            Order::primary().partition("o".into()).sort_le(7),
            "#n1 <= :v1",
        ),
        (
            Order::primary().partition("o".into()).sort_gt(7),
            "#n1 > :v1",
        ),
        (
            Order::primary().partition("o".into()).sort_ge(7),
            "#n1 >= :v1",
        ),
        (
            Order::primary().partition("o".into()).sort_between(7, 9),
            "#n1 BETWEEN :v1 AND :v2",
        ),
    ] {
        let built = condition.build().unwrap();
        let text = built.render();
        assert_eq!(
            text.key_condition(),
            Some(format!("(#n0 = :v0 AND {expected})").as_str())
        );
        assert_eq!(text.names().len(), 2);
        assert_eq!(text.names()["#n1"], "size");
        assert_eq!(text.values()[":v1"], Av::N("7".into()));
        assert_eq!(
            text.values().len(),
            if expected.contains(":v2") { 3 } else { 2 }
        );
        if expected.contains(":v2") {
            assert_eq!(text.values()[":v2"], Av::N("9".into()));
        }
        assert_eq!(text.values(), built.render().values());
    }
}

#[test]
fn index_identity_and_binary_prefixes_use_their_own_key_descriptors() {
    let built = OrderIndexes::BY_LABEL
        .partition("c".into())
        .sort_begins_with(vec![0, 255])
        .build()
        .unwrap();
    let _: &tabulary::BuiltKeyCondition<Order, OrderByLabel> = &built;
    let text = built.render();
    assert_eq!(
        text.key_condition(),
        Some("(#n0 = :v0 AND begins_with(#n1, :v1))")
    );
    assert_eq!(text.names()["#n0"], "label");
    assert_eq!(text.names()["#n1"], "bytes");
    assert_eq!(text.values()[":v1"], Av::B(vec![0, 255].into()));
    assert!(
        OrderIndexes::BY_COUNT
            .partition("o".into())
            .sort_eq(1)
            .build()
            .is_ok()
    );
    assert!(OrderIndexes::BY_HASH.partition("c".into()).build().is_ok());
}

#[test]
fn invalid_key_values_fail_with_record_and_physical_attribute_context() {
    assert!(
        OrderIndexes::BY_HASH
            .partition("x".repeat(1024))
            .build()
            .is_ok()
    );
    let BuildError::Encode(error) = OrderIndexes::BY_HASH
        .partition("x".repeat(1025))
        .build()
        .unwrap_err()
    else {
        panic!("expected encode error")
    };
    assert_eq!(error.record, "Order");
    assert_eq!(error.attribute(), Some("label"));
    assert_eq!(
        error.kind,
        EncodeFault::KeySize {
            max: 1024,
            actual: 1025
        }
    );
    assert!(
        matches!(OrderIndexes::BY_SHARED.partition(vec![1; 1025]).build(), Err(BuildError::Encode(e)) if e.kind == EncodeFault::KeySize { max: 1024, actual: 1025 })
    );
    for (condition, attribute, fault) in [
        (
            Order::primary().partition(String::new()).sort_eq(1),
            "order.id",
            EncodeFault::EmptyKey,
        ),
        (
            Order::primary().partition("x".repeat(2049)).sort_eq(1),
            "order.id",
            EncodeFault::KeySize {
                max: 2048,
                actual: 2049,
            },
        ),
    ] {
        let BuildError::Encode(error) = condition.build().unwrap_err() else {
            panic!("expected encode error")
        };
        assert_eq!(error.record, "Order");
        assert_eq!(error.attribute(), Some(attribute));
        assert_eq!(error.kind, fault);
    }
    assert!(
        matches!(OrderIndexes::BY_LABEL.partition("c".into()).sort_eq(vec![]).build(), Err(BuildError::Encode(e)) if e.kind == EncodeFault::EmptyKey)
    );
}

#[derive(tabulary::Record)]
#[tabulary(rename_all = "snake_case", partition(name = "pk", template = "order!{id:hex}"),
    sort(name = "sk", template = "line!{count:dec20}!{label:text}"),
    index(ident = ByLine, name = "by_line", kind = global,
        partition(name = "gpk", template = "label!{label:hex}"),
        sort(name = "gsk", template = "line!{count:dec20}!{id:hex}"), project = all),
    index(ident = ByDecimal, name = "by_decimal", kind = global, partition = id,
        sort(name = "decimal", template = "{count:dec}"), project = all))]
struct Parcel {
    id: String,
    count: u64,
    label: String,
}

#[test]
fn composite_conditions_reuse_complete_partition_and_prefix_carriers() {
    let text = ParcelIndexes::BY_DECIMAL
        .partition("o".into())
        .sort_ge((9,))
        .build()
        .unwrap()
        .render();
    assert_eq!(text.key_condition(), Some("(#n0 = :v0 AND #n1 >= :v1)"));
    assert_eq!(text.values()[":v1"], Av::S("9".into()));
    let item = Parcel {
        id: "o".into(),
        count: 10,
        label: "line".into(),
    }
    .to_item()
    .unwrap();
    assert_eq!(item["decimal"], Av::S("10".into()));
    assert!(item["decimal"].as_s().unwrap() < text.values()[":v1"].as_s().unwrap());
    let prefix = Parcel::primary().sort_prefix((7,)).unwrap();
    let text = Parcel::primary()
        .partition("o".into())
        .sort_begins_with(prefix)
        .build()
        .unwrap()
        .render();
    assert_eq!(text.names()["#n0"], "pk");
    assert_eq!(text.values()[":v0"], Av::S("order!6f".into()));
    assert_eq!(
        text.values()[":v1"],
        Av::S("line!00000000000000000007!".into())
    );
    let text = ParcelIndexes::BY_LINE
        .partition("c".into())
        .sort_eq((8, "o".into()))
        .build()
        .unwrap()
        .render();
    assert_eq!(text.values()[":v0"], Av::S("label!63".into()));
    assert_eq!(
        text.values()[":v1"],
        Av::S("line!00000000000000000008!6f".into())
    );
    let prefix = ParcelIndexes::BY_LINE.sort_prefix((8,)).unwrap();
    assert!(
        ParcelIndexes::BY_LINE
            .partition("c".into())
            .sort_begins_with(prefix)
            .build()
            .is_ok()
    );
    assert!(
        matches!(Parcel::primary().partition("x".repeat(2048)).build(), Err(BuildError::Encode(e)) if matches!(e.kind, EncodeFault::KeySize { .. }))
    );
}

#[test]
fn invalid_key_condition_programs_fail_to_compile() {
    trybuild::TestCases::new().compile_fail("tests/ui/key_condition/*.rs");
}

#[derive(tabulary::Record)]
#[tabulary(rename_all = "snake_case", generated_prefix = "Stored")]
struct Generic<T> {
    #[tabulary(partition_key)]
    id: T,
    #[tabulary(sort_key)]
    label: String,
}

#[test]
fn generic_binary_partitions_and_native_string_prefixes_preserve_types() {
    let text = StoredPrimary::<Vec<u8>>::partition(&Generic::<Vec<u8>>::primary(), vec![1])
        .sort_begins_with("line!".into())
        .build()
        .unwrap()
        .render();
    assert_eq!(text.values()[":v0"], Av::B(vec![1].into()));
    assert_eq!(text.values()[":v1"], Av::S("line!".into()));
    assert!(
        matches!(Generic::<String>::primary().partition("o".into()).sort_eq("x".repeat(1025)).build(),
        Err(BuildError::Encode(e)) if e.kind == EncodeFault::KeySize { max: 1024, actual: 1025 })
    );
}
