use super::*;
use crate::{__private as p, KeyOnly, Native, NoSort, Required, condition::Operator};
use aws_sdk_dynamodb::primitives::Blob;

struct Row;
impl Record for Row {
    const SCHEMA: RecordSchema = p::schema(
        "Row",
        &[
            p::field::<Row, String, Required, KeyOnly, Native<String>>("pk", 0).descriptor(),
            p::field::<Row, u64, Required, KeyOnly, Native<u64>>("sk", 1).descriptor(),
            p::field::<Row, Vec<u8>, Required, KeyOnly, Native<Vec<u8>>>("gpk", 2).descriptor(),
        ],
        p::keys(0, Some(1)),
    )
    .with_indexes(&[INDEX]);
}
const INDEX: IndexDefinition = IndexDefinition {
    name: "by_group",
    kind: crate::IndexKind::Global,
    keys: p::keys(2, None),
    projection: crate::IndexProjection::All,
};
struct Primary;
impl crate::KeySchema<Row> for Primary {
    type Partition = String;
    type PartitionCodec = Native<String>;
    type Sort = NoSort;
    const KEYS: crate::KeyDefinition = Row::SCHEMA.primary;
}
fn key() -> Item {
    Item::from([
        ("pk".into(), AttributeValue::S("private-key".into())),
        ("sk".into(), AttributeValue::N("1.00e1".into())),
        ("gpk".into(), AttributeValue::B(Blob::new([1, 2]))),
    ])
}
fn query() -> CursorBinding<Row, Primary> {
    let key = p::partition::<Row, Primary, _, Native<String>>(Ok("partition".into()))
        .build()
        .unwrap();
    CursorBinding::query(
        &Table::new("orders").unwrap(),
        None,
        &key,
        None,
        None,
        ReadConsistency::Eventual,
        QueryOrder::Ascending,
    )
    .unwrap()
}
fn cursor() -> Cursor<Row, Primary> {
    query().cursor(Some(key())).unwrap().unwrap()
}

#[test]
fn retains_complete_service_key_without_record_decode_and_redacts_debug() {
    let mut binding = query();
    binding.index = Some(INDEX);
    let cursor = binding.cursor(Some(key())).unwrap().unwrap();
    assert_eq!(binding.resume(&cursor).unwrap(), &key());
    assert_eq!(cursor.last_evaluated_key(), &key());
    assert_eq!(cursor.clone().last_evaluated_key(), &key());
    assert_eq!(format!("{cursor:?}"), "Cursor([redacted])");
    assert!(!format!("{binding:?}").contains("partition"));
}

#[test]
fn absent_and_empty_keys_are_end_markers_and_invalid_keys_are_refused() {
    let binding = query();
    assert!(binding.cursor(None).unwrap().is_none());
    assert!(binding.cursor(Some(Item::new())).unwrap().is_none());
    for value in [
        AttributeValue::Bool(true),
        AttributeValue::Null(true),
        AttributeValue::L(vec![]),
        AttributeValue::M(Item::new()),
        AttributeValue::Ss(vec!["x".into()]),
        AttributeValue::S(String::new()),
        AttributeValue::B(Blob::new([])),
        AttributeValue::N("bad".into()),
        AttributeValue::N("1e126".into()),
        AttributeValue::S("x".repeat(2049)),
    ] {
        let mut raw = key();
        raw.insert("pk".into(), value);
        assert!(matches!(
            binding.cursor(Some(raw)),
            Err(BuildError::InvalidCursor)
        ));
    }
    let mut raw = key();
    raw.remove("sk");
    assert!(matches!(
        binding.cursor(Some(raw)),
        Err(BuildError::InvalidCursor)
    ));
    let mut binding = query();
    binding.index = Some(INDEX);
    let mut raw = key();
    raw.remove("gpk");
    assert!(matches!(
        binding.cursor(Some(raw)),
        Err(BuildError::InvalidCursor)
    ));
}

#[test]
fn every_request_component_is_bound_and_page_limit_is_excluded() {
    let cursor = cursor();
    type Change = fn(&mut CursorBinding<Row, Primary>);
    let changes: Vec<Change> = vec![
        (|b| b.table = "other_orders".into()),
        (|b| b.index = Some(INDEX)),
        (|b| b.schema.name = "ChangedRow"),
        (|b| b.schema.primary = p::keys(0, None)),
        (|b| b.schema.fields = &Row::SCHEMA.fields[..2]),
        (|b| b.schema.indexes = &[]),
        (|b| b.consistency = ReadConsistency::Strong),
        (|b| b.projection = Some(vec!["pk"])),
        (|b| b.operation = Operation::Scan(None)),
        (|b| b.operation = Operation::Scan(Some((0, 2)))),
        (|b| {
            if let Operation::Query(_, order) = &mut b.operation {
                *order = QueryOrder::Descending;
            }
        }),
        (|b| b.filter = Some(CanonicalNode::Predicate(Operator::Exists, vec![]))),
        (|b| {
            b.operation = Operation::Query(
                CanonicalNode::Predicate(Operator::NotExists, vec![]),
                QueryOrder::Ascending,
            )
        }),
    ];
    for change in changes {
        let mut binding = query();
        change(&mut binding);
        assert!(matches!(
            binding.resume(&cursor),
            Err(BuildError::CursorMismatch)
        ));
    }
    // Adapters vary their evaluated-item limit independently; binding takes no limit.
    for _page_limit in [1, 50, 1000] {
        assert_eq!(query().resume(&cursor).unwrap(), &key());
    }
}

#[test]
fn scan_binds_both_segment_numbers_and_distinguishes_unsegmented() {
    let table = Table::<Row>::new("orders").unwrap();
    let scan = |segment| {
        CursorBinding::<Row, Primary>::scan(
            &table,
            None,
            None,
            None,
            ReadConsistency::Strong,
            segment,
        )
        .unwrap()
    };
    let cursor = scan(Some((1, 3))).cursor(Some(key())).unwrap().unwrap();
    assert!(scan(Some((1, 3))).resume(&cursor).is_ok());
    for segment in [None, Some((0, 3)), Some((1, 4))] {
        assert!(matches!(
            scan(segment).resume(&cursor),
            Err(BuildError::CursorMismatch)
        ));
    }
}

#[test]
fn constructors_bind_validated_conditions_index_identity_and_selected_attributes() {
    let table = Table::<Row>::new("orders").unwrap();
    let condition = |value: &str| {
        p::partition::<Row, Primary, _, Native<String>>(Ok(value.into()))
            .build()
            .unwrap()
    };
    let attr = p::field::<Row, u64, Required, KeyOnly, Native<u64>>("sk", 1);
    let filter = attr.ge(2).build().unwrap();
    let bind = |value: &str, selection| {
        CursorBinding::query(
            &table,
            Some(INDEX),
            &condition(value),
            Some(&filter),
            Some(selection),
            ReadConsistency::Eventual,
            QueryOrder::Descending,
        )
        .unwrap()
    };
    let cursor = bind("one", vec!["pk", "sk"])
        .cursor(Some(key()))
        .unwrap()
        .unwrap();
    assert!(bind("one", vec!["sk", "pk"]).resume(&cursor).is_ok());
    assert!(matches!(
        bind("two", vec!["pk", "sk"]).resume(&cursor),
        Err(BuildError::CursorMismatch)
    ));
    assert!(matches!(
        bind("one", vec!["pk"]).resume(&cursor),
        Err(BuildError::CursorMismatch)
    ));
    let mut other = bind("one", vec!["pk", "sk"]);
    other.index = Some(IndexDefinition {
        name: "other_group",
        ..INDEX
    });
    assert!(matches!(
        other.resume(&cursor),
        Err(BuildError::CursorMismatch)
    ));
    assert!(matches!(
        CursorBinding::<Row, Primary>::scan(
            &table,
            other.index,
            None,
            None,
            ReadConsistency::Eventual,
            None
        ),
        Err(BuildError::IncompatibleIndex)
    ));
}

fn canonical(value: AttributeValue) -> CanonicalValue {
    CanonicalValue::new(&value, 0).unwrap()
}
#[test]
fn canonical_values_sort_maps_and_sets_normalize_numbers_and_keep_types() {
    let a = AttributeValue::M(Item::from([
        ("z".into(), AttributeValue::Bool(false)),
        ("a".into(), AttributeValue::N("10.0".into())),
    ]));
    let b = AttributeValue::M(Item::from([
        ("a".into(), AttributeValue::N("1e1".into())),
        ("z".into(), AttributeValue::Bool(false)),
    ]));
    assert!(canonical(a) == canonical(b));
    for (a, b) in [
        (
            AttributeValue::Ss(vec!["b".into(), "a".into()]),
            AttributeValue::Ss(vec!["a".into(), "b".into()]),
        ),
        (
            AttributeValue::Ns(vec!["2.0".into(), "1".into()]),
            AttributeValue::Ns(vec!["1.0".into(), "2".into()]),
        ),
        (
            AttributeValue::Bs(vec![Blob::new([2]), Blob::new([1])]),
            AttributeValue::Bs(vec![Blob::new([1]), Blob::new([2])]),
        ),
    ] {
        assert!(canonical(a) == canonical(b));
    }
    assert!(canonical(AttributeValue::N("10".into())) != canonical(AttributeValue::S("10".into())));
    assert!(
        canonical(AttributeValue::B(Blob::new([1])))
            != canonical(AttributeValue::L(vec![AttributeValue::N("1".into())]))
    );
    assert!(
        canonical(AttributeValue::L(vec![
            AttributeValue::Bool(true),
            AttributeValue::Bool(false)
        ])) != canonical(AttributeValue::L(vec![
            AttributeValue::Bool(false),
            AttributeValue::Bool(true)
        ]))
    );
}

#[test]
fn canonical_ast_preserves_operators_grouping_constants_and_projection_sets() {
    let attr = p::field::<Row, String, Required, KeyOnly, Native<String>>("pk", 0);
    let a = attr
        .eq("one".into())
        .and(attr.ne("two".into()))
        .build()
        .unwrap();
    let b = attr
        .eq("one".into())
        .or(attr.ne("two".into()))
        .build()
        .unwrap();
    assert!(CanonicalNode::new(a.root()).unwrap() != CanonicalNode::new(b.root()).unwrap());
    let mut binding = query();
    binding.filter = Some(CanonicalNode::new(a.root()).unwrap());
    let cursor = binding.cursor(Some(key())).unwrap().unwrap();
    binding.filter =
        Some(CanonicalNode::new(attr.eq("other".into()).build().unwrap().root()).unwrap());
    assert!(matches!(
        binding.resume(&cursor),
        Err(BuildError::CursorMismatch)
    ));
    assert_eq!(
        projection(Some(vec!["sk", "pk", "pk"])),
        projection(Some(vec!["pk", "sk"]))
    );
    assert_ne!(projection(None), projection(Some(vec!["pk", "sk"])));
}

#[test]
fn canonical_values_refuse_malformed_sets_nulls_and_excess_depth() {
    for value in [
        AttributeValue::Null(false),
        AttributeValue::Ss(vec![]),
        AttributeValue::Ns(vec!["1".into(), "1.0".into()]),
        AttributeValue::Ss(vec!["a".into(), "a".into()]),
        AttributeValue::Bs(vec![Blob::new([1]), Blob::new([1])]),
    ] {
        assert!(matches!(
            CanonicalValue::new(&value, 0),
            Err(BuildError::InvalidCursor)
        ));
    }
    let mut value = AttributeValue::Null(true);
    for _ in 0..32 {
        value = AttributeValue::L(vec![value]);
    }
    assert!(CanonicalValue::new(&value, 0).is_ok());
    value = AttributeValue::L(vec![value]);
    assert!(matches!(
        CanonicalValue::new(&value, 0),
        Err(BuildError::InvalidCursor)
    ));
}

#[test]
fn v1_encoding_sorts_service_keys_and_retains_scalar_spellings() {
    let a = cursor();
    let mut b = a.clone();
    let mut entries: Vec<_> = key().into_iter().collect();
    entries.sort_unstable_by_key(|(name, _)| name.clone());
    b.key = entries.into_iter().rev().collect();
    let bytes = a.to_bytes().unwrap();
    assert_eq!(bytes[0], 1);
    assert_eq!(bytes, b.to_bytes().unwrap());
    for private in [b"private-key".as_slice(), b"1.00e1", &[1, 2]] {
        assert!(bytes.windows(private.len()).any(|v| v == private));
    }
    assert!(!format!("{a:?}").contains("private-key"));
}

fn with_constant(value: AttributeValue) -> Cursor<Row, Primary> {
    let mut binding = query();
    binding.filter = Some(CanonicalNode::Predicate(
        Operator::Eq,
        vec![CanonicalOperand::Value(canonical(value))],
    ));
    binding.cursor(Some(key())).unwrap().unwrap()
}

#[test]
fn canonical_encoding_covers_every_value_tag_and_sorts_maps_and_sets() {
    let values = |reversed| {
        let mut entries = vec![
            ("s".into(), AttributeValue::S("private-filter".into())),
            (
                "n".into(),
                AttributeValue::N(if reversed { "10.0" } else { "1e1" }.into()),
            ),
            ("b".into(), AttributeValue::B(Blob::new([0, 255]))),
            ("bool".into(), AttributeValue::Bool(true)),
            ("null".into(), AttributeValue::Null(true)),
            (
                "list".into(),
                AttributeValue::L(vec![AttributeValue::Bool(false)]),
            ),
            (
                "ss".into(),
                AttributeValue::Ss(vec!["z".into(), "a".into()]),
            ),
            (
                "ns".into(),
                AttributeValue::Ns(vec!["2.0".into(), "-1".into()]),
            ),
            (
                "bs".into(),
                AttributeValue::Bs(vec![Blob::new([2]), Blob::new([1])]),
            ),
        ];
        if reversed {
            entries.reverse();
        }
        AttributeValue::M(entries.into_iter().collect())
    };
    let a = with_constant(values(false));
    let b = with_constant(values(true));
    assert_eq!(a.to_bytes().unwrap(), b.to_bytes().unwrap());
    assert!(
        a.to_bytes()
            .unwrap()
            .windows(14)
            .any(|b| b == b"private-filter")
    );
    assert!(!format!("{a:?}").contains("private-filter"));
    assert_ne!(
        with_constant(AttributeValue::S("10".into()))
            .to_bytes()
            .unwrap(),
        with_constant(AttributeValue::N("10".into()))
            .to_bytes()
            .unwrap()
    );
}

#[test]
fn encoded_identity_keeps_schema_metadata_and_ast_structure() {
    let base = cursor().to_bytes().unwrap();
    let changes: Vec<fn(&mut CursorBinding<Row, Primary>)> = vec![
        |b| b.table = "other_orders".into(),
        |b| b.schema.ttl = Some(1),
        |b| b.schema.name = "OtherRow",
        |b| b.schema.indexes = &[],
        |b| b.index = Some(INDEX),
        |b| b.projection = Some(vec!["pk"]),
        |b| b.consistency = ReadConsistency::Strong,
        |b| b.operation = Operation::Scan(None),
        |b| b.operation = Operation::Scan(Some((1, 2))),
        |b| {
            if let Operation::Query(_, order) = &mut b.operation {
                *order = QueryOrder::Descending
            }
        },
    ];
    for change in changes {
        let mut b = query();
        change(&mut b);
        assert_ne!(
            base,
            b.cursor(Some(key())).unwrap().unwrap().to_bytes().unwrap()
        );
    }
    let attr = p::field::<Row, String, Required, KeyOnly, Native<String>>("pk", 0);
    let bytes = |condition: crate::Condition<Row>| {
        let mut b = query();
        b.filter = Some(CanonicalNode::new(condition.build().unwrap().root()).unwrap());
        b.cursor(Some(key())).unwrap().unwrap().to_bytes().unwrap()
    };
    let and = bytes(attr.eq("one".into()).and(attr.ne("two".into())));
    assert_ne!(and, bytes(attr.eq("one".into()).or(attr.ne("two".into()))));
    assert_ne!(and, bytes(attr.ne("two".into()).and(attr.eq("one".into()))));
    assert_ne!(
        and,
        bytes(attr.eq("one".into()).and(attr.ne("two".into())).not())
    );
}

#[test]
fn total_encoded_size_accepts_exact_cap_and_refuses_one_more_byte() {
    let mut cursor = cursor();
    let base = cursor.to_bytes().unwrap().len();
    let name = "x".repeat(4 * 1024 * 1024 - base - 10);
    cursor
        .key
        .insert(name.clone(), AttributeValue::S("a".into()));
    assert_eq!(cursor.to_bytes().unwrap().len(), 4 * 1024 * 1024);
    cursor.key.remove(&name);
    cursor
        .key
        .insert(format!("{name}x"), AttributeValue::S("a".into()));
    assert!(matches!(cursor.to_bytes(), Err(BuildError::CursorTooLarge)));
}

#[test]
fn request_build_bounds_expression_depth() {
    for depth in [32, 33] {
        let attr = p::field::<Row, String, Required, KeyOnly, Native<String>>("pk", 0);
        let mut filter = attr.eq("private-filter".into());
        for _ in 0..depth {
            filter = filter.not();
        }
        let filter = filter.build().unwrap();
        let key = p::partition::<Row, Primary, _, Native<String>>(Ok("partition".into()))
            .build()
            .unwrap();
        let result = CursorBinding::query(
            &Table::new("orders").unwrap(),
            None,
            &key,
            Some(&filter),
            None,
            ReadConsistency::Eventual,
            QueryOrder::Ascending,
        );
        if depth == 32 {
            assert!(result.is_ok());
        } else {
            assert!(matches!(result, Err(BuildError::InvalidCursor)));
        }
    }
}
