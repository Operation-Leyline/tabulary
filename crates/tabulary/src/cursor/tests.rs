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
    ])
}
fn indexed_key() -> Item {
    let mut key = key();
    key.insert("gpk".into(), AttributeValue::B(Blob::new([1, 2])));
    key
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
    let cursor = binding.cursor(Some(indexed_key())).unwrap().unwrap();
    assert_eq!(binding.resume(&cursor).unwrap(), &indexed_key());
    assert_eq!(cursor.last_evaluated_key(), &indexed_key());
    assert_eq!(cursor.clone().last_evaluated_key(), &indexed_key());
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
    let decoded = Cursor::<Row, Primary>::from_bytes(&cursor.to_bytes().unwrap()).unwrap();
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
        assert!(matches!(
            binding.resume(&decoded),
            Err(BuildError::CursorMismatch)
        ));
    }
    // Adapters vary their evaluated-item limit independently; binding takes no limit.
    for _page_limit in [1, 50, 1000] {
        assert_eq!(query().resume(&cursor).unwrap(), &key());
        assert_eq!(query().resume(&decoded).unwrap(), &key());
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
        .cursor(Some(indexed_key()))
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
    let mut binding = query();
    binding.index = Some(INDEX);
    let a = binding.cursor(Some(indexed_key())).unwrap().unwrap();
    let mut b = a.clone();
    let mut entries: Vec<_> = indexed_key().into_iter().collect();
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
    let decoded = Cursor::<Row, Primary>::from_bytes(&a.to_bytes().unwrap()).unwrap();
    assert_eq!(decoded.to_bytes().unwrap(), a.to_bytes().unwrap());
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
            b.cursor(Some(if b.index.is_some() {
                indexed_key()
            } else {
                key()
            }))
            .unwrap()
            .unwrap()
            .to_bytes()
            .unwrap()
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
    let base = with_constant(AttributeValue::S(String::new()))
        .to_bytes()
        .unwrap()
        .len();
    let padding = 4 * 1024 * 1024 - base;
    let cursor = with_constant(AttributeValue::S("x".repeat(padding)));
    assert_eq!(cursor.to_bytes().unwrap().len(), 4 * 1024 * 1024);
    let decoded = Cursor::<Row, Primary>::from_bytes(&cursor.to_bytes().unwrap()).unwrap();
    assert_eq!(decoded.last_evaluated_key(), cursor.last_evaluated_key());
    let cursor = with_constant(AttributeValue::S("x".repeat(padding + 1)));
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

fn parse(bytes: &[u8]) -> Result<Cursor<Row, Primary>, CursorError> {
    Cursor::from_bytes(bytes)
}

#[test]
fn bytes_round_trip_full_key_and_private_filter_and_keep_resume_binding() {
    let mut b = query();
    b.index = Some(INDEX);
    b.filter = Some(CanonicalNode::Predicate(
        Operator::Eq,
        vec![CanonicalOperand::Value(canonical(AttributeValue::M(
            Item::from([
                (
                    "list".into(),
                    AttributeValue::L(vec![AttributeValue::Null(true)]),
                ),
                (
                    "strings".into(),
                    AttributeValue::Ss(vec!["z".into(), "a".into()]),
                ),
                (
                    "numbers".into(),
                    AttributeValue::Ns(vec!["-2".into(), "1.0".into()]),
                ),
                (
                    "binaries".into(),
                    AttributeValue::Bs(vec![Blob::new([2]), Blob::new([1])]),
                ),
                ("secret".into(), AttributeValue::S("private-filter".into())),
            ]),
        )))],
    ));
    b.projection = Some(vec!["pk", "sk"]);
    let bytes = b
        .cursor(Some(indexed_key()))
        .unwrap()
        .unwrap()
        .to_bytes()
        .unwrap();
    let decoded = parse(&bytes).unwrap();
    assert_eq!(decoded.last_evaluated_key(), &indexed_key());
    assert_eq!(decoded.clone().to_bytes().unwrap(), bytes);
    assert_eq!(b.resume(&decoded).unwrap(), &indexed_key());
    assert_eq!(format!("{decoded:?}"), "Cursor([redacted])");
    assert!(matches!(
        query().resume(&decoded),
        Err(BuildError::CursorMismatch)
    ));
    b.filter = None;
    assert!(matches!(
        b.resume(&decoded),
        Err(BuildError::CursorMismatch)
    ));
}

#[test]
fn parsed_schema_mismatch_is_reported_at_resume_not_lost_during_parse() {
    let mut b = query();
    b.schema.name = "OtherRow";
    let bytes = b.cursor(Some(key())).unwrap().unwrap().to_bytes().unwrap();
    let cursor = parse(&bytes).unwrap();
    assert!(matches!(
        query().resume(&cursor),
        Err(BuildError::CursorMismatch)
    ));
}

#[test]
fn parser_refuses_every_truncation_trailing_bytes_and_unknown_versions() {
    let bytes = cursor().to_bytes().unwrap();
    let mut rich = query();
    rich.index = Some(INDEX);
    rich.operation = Operation::Scan(Some((1, 3)));
    rich.filter = Some(CanonicalNode::Predicate(
        Operator::Eq,
        vec![CanonicalOperand::Value(CanonicalValue::String(
            "private-filter".into(),
        ))],
    ));
    let rich = rich
        .cursor(Some(indexed_key()))
        .unwrap()
        .unwrap()
        .to_bytes()
        .unwrap();
    for token in [&bytes, &rich] {
        assert!(parse(token).is_ok());
        for end in 0..token.len() {
            assert!(
                matches!(parse(&token[..end]), Err(CursorError::Truncated)),
                "prefix {end}"
            );
        }
    }
    let mut trailing = bytes.clone();
    trailing.push(0);
    assert!(matches!(parse(&trailing), Err(CursorError::TrailingBytes)));
    for version in [0, 2, 255] {
        let mut bytes = bytes.clone();
        bytes[0] = version;
        assert!(
            matches!(parse(&bytes), Err(CursorError::UnknownVersion { version: v }) if v == version)
        );
    }
}

fn blob(out: &mut Vec<u8>, bytes: &[u8]) {
    out.extend((bytes.len() as u32).to_be_bytes());
    out.extend(bytes);
}
fn filter_value(value: &[u8]) -> Vec<u8> {
    let mut filter = vec![1, 0, 2, 0, 0, 0, 1, 2];
    filter.extend(value);
    scan_with_filter(&filter)
}
fn raw_keys(entries: &[(&str, u8, &[u8])]) -> Vec<u8> {
    let mut bytes = cursor().to_bytes().unwrap();
    bytes.truncate(1 + super::bytes::binding_bytes(&query()).unwrap().len());
    bytes.extend((entries.len() as u32).to_be_bytes());
    for (name, tag, value) in entries {
        blob(&mut bytes, name.as_bytes());
        bytes.push(*tag);
        blob(&mut bytes, value);
    }
    bytes
}

#[test]
fn parser_checks_canonical_numbers_and_all_set_and_map_gates() {
    for number in ["1.0", "01", "1e0", "-0"] {
        let mut value = vec![1];
        blob(&mut value, number.as_bytes());
        assert!(matches!(
            parse(&filter_value(&value)),
            Err(CursorError::InvalidFormat)
        ));
    }
    for tag in [6, 7, 8, 9] {
        for members in [["1", "1"], ["2", "1"]] {
            let mut value = vec![tag, 0, 0, 0, 2];
            for member in members {
                blob(&mut value, member.as_bytes());
                if tag == 6 {
                    value.push(4);
                }
            }
            assert!(
                matches!(
                    parse(&filter_value(&value)),
                    Err(CursorError::InvalidFormat)
                ),
                "tag {tag}"
            );
        }
        if tag != 6 {
            assert!(matches!(
                parse(&filter_value(&[tag, 0, 0, 0, 0])),
                Err(CursorError::InvalidFormat)
            ));
        }
    }
    let mut numbers = vec![8, 0, 0, 0, 2];
    for number in ["-2", "1"] {
        blob(&mut numbers, number.as_bytes());
    }
    assert!(parse(&filter_value(&numbers)).is_ok());
}

#[test]
fn parser_refuses_unknown_value_operand_node_operator_field_and_index_tags() {
    let original = cursor().to_bytes().unwrap();
    let first_field = 1 + 4 + "orders".len() + 4 + "Row".len() + 4;
    let mut operand = vec![1, 0, 2, 0, 0, 0, 1, 255];
    operand.extend(&original[first_field..first_field + 4 + "pk".len() + 8 + 3]);
    // Valid payloads after unknown tags ensure permissive fallbacks would succeed.
    for filter in [
        vec![1, 255, 0, 0, 0, 0, 0, 0],
        vec![1, 0, 255, 0, 0, 0, 0],
        operand,
    ] {
        assert!(matches!(
            parse(&scan_with_filter(&filter)),
            Err(CursorError::InvalidFormat)
        ));
    }
    assert!(matches!(
        parse(&filter_value(&[255, 0, 0, 0, 1, 0, 0, 0, 1, b'a'])),
        Err(CursorError::InvalidFormat)
    ));
    assert!(matches!(
        parse(&filter_value(&[3, 2])),
        Err(CursorError::InvalidFormat)
    ));
    // First field's presence/access/wire tags follow its name and position.
    for tag in 0..3 {
        let mut bytes = original.clone();
        bytes[first_field + 4 + "pk".len() + 8 + tag] = 255;
        assert!(matches!(parse(&bytes), Err(CursorError::InvalidFormat)));
    }
    let mut index = vec![];
    blob(&mut index, b"by_group");
    index.push(0); // Global index.
    index.extend(2_u64.to_be_bytes());
    index.extend([0, 0]); // No sort key, ALL projection.
    let start = original
        .windows(index.len())
        .position(|v| v == index)
        .unwrap();
    for at in [start + 4 + "by_group".len(), start + index.len() - 1] {
        let mut bytes = original.clone();
        bytes[at] = 255;
        assert!(matches!(parse(&bytes), Err(CursorError::InvalidFormat)));
    }
}

#[test]
fn parser_bounds_map_depth_and_scan_segment() {
    let mut binding = query();
    binding.operation = Operation::Scan(None);
    let mut bytes = binding
        .cursor(Some(key()))
        .unwrap()
        .unwrap()
        .to_bytes()
        .unwrap();
    let operation = 1 + super::bytes::binding_bytes(&binding).unwrap().len() - 5;
    bytes[operation] = 255;
    assert!(matches!(parse(&bytes), Err(CursorError::InvalidFormat)));
    for depth in [32, 33] {
        let mut value = vec![];
        for _ in 0..depth {
            value.extend([6, 0, 0, 0, 1]);
            blob(&mut value, b"a");
        }
        value.push(4);
        let result = parse(&filter_value(&value));
        if depth == 32 {
            assert!(result.is_ok());
        } else {
            assert!(matches!(result, Err(CursorError::DepthLimit)));
        }
    }
    for segment in [(3, 3), (0, 0), (0, 1_000_001)] {
        let mut b = query();
        b.operation = Operation::Scan(Some(segment));
        assert!(matches!(
            parse(&b.cursor(Some(key())).unwrap().unwrap().to_bytes().unwrap()),
            Err(CursorError::InvalidFormat)
        ));
    }
}

#[test]
fn parser_requires_exact_sorted_unique_keys_and_utf8_string_values() {
    for keys in [
        vec![("sk", 1, b"1".as_slice()), ("pk", 0, b"a".as_slice())],
        vec![
            ("pk", 0, b"a".as_slice()),
            ("pk", 0, b"a".as_slice()),
            ("sk", 1, b"1".as_slice()),
        ],
        vec![("pk", 0, [255].as_slice()), ("sk", 1, b"1".as_slice())],
        vec![
            ("pk", 0, b"a".as_slice()),
            ("sk", 1, b"1".as_slice()),
            ("zz", 0, b"x".as_slice()),
        ],
    ] {
        assert!(matches!(
            parse(&raw_keys(&keys)),
            Err(CursorError::InvalidKey)
        ));
    }
    let mut extra = key();
    extra.insert("zz".into(), AttributeValue::S("x".into()));
    assert!(matches!(
        query().cursor(Some(extra)),
        Err(BuildError::InvalidCursor)
    ));
}

#[test]
fn numeric_key_text_is_bounded_before_decimal_parsing() {
    let maximum = format!("-9.{}e-130", "9".repeat(37));
    assert_eq!(maximum.len(), 45);
    assert!(parse(&raw_keys(&[("pk", 0, b"a"), ("sk", 1, maximum.as_bytes())])).is_ok());
    // Decimal accepts these redundant zeros; the cursor must reject the spelling.
    for length in [46, 2049] {
        let number = "0".repeat(length);
        assert!(number.parse::<Decimal>().is_ok());
        assert!(matches!(
            parse(&raw_keys(&[("pk", 0, b"a"), ("sk", 1, number.as_bytes())])),
            Err(CursorError::InvalidKey)
        ));
        let mut key = key();
        key.insert("sk".into(), AttributeValue::N(number));
        assert!(matches!(
            query().cursor(Some(key)),
            Err(BuildError::InvalidCursor)
        ));
    }
}

#[test]
fn parser_deduplicates_shared_base_and_index_key_requirements() {
    const SHARED: IndexDefinition = IndexDefinition {
        keys: p::keys(0, Some(1)),
        ..INDEX
    };
    let mut binding = query();
    binding.schema.indexes = &[SHARED];
    binding.index = Some(SHARED);
    let cursor = binding.cursor(Some(key())).unwrap().unwrap();
    let decoded = parse(&cursor.to_bytes().unwrap()).unwrap();
    assert_eq!(binding.resume(&decoded).unwrap(), &key());
    // A field belonging to a different index is still extraneous on a base read.
    assert!(matches!(
        parse(&raw_keys(&[
            ("gpk", 2, &[1]),
            ("pk", 0, b"a"),
            ("sk", 1, b"1")
        ])),
        Err(CursorError::InvalidKey)
    ));
    assert!(matches!(
        query().cursor(Some(indexed_key())),
        Err(BuildError::InvalidCursor)
    ));
}

#[test]
fn parser_checks_outer_size_blob_lengths_and_counts_before_copying() {
    assert!(matches!(
        parse(&vec![0; 4 * 1024 * 1024 + 1]),
        Err(CursorError::TooLarge)
    ));
    let mut bytes = cursor().to_bytes().unwrap();
    // Table-name byte length is the first length in V1.
    bytes[1..5].copy_from_slice(&u32::MAX.to_be_bytes());
    assert!(matches!(parse(&bytes), Err(CursorError::TooLarge)));
    bytes[1..5].copy_from_slice(&1_000_000_u32.to_be_bytes());
    assert!(matches!(parse(&bytes), Err(CursorError::Truncated)));
    let mut bytes = cursor().to_bytes().unwrap();
    let count = 1 + 4 + "orders".len() + 4 + "Row".len();
    bytes[count..count + 4].copy_from_slice(&u32::MAX.to_be_bytes());
    assert!(matches!(parse(&bytes), Err(CursorError::TooLarge)));
    let mut bytes = cursor().to_bytes().unwrap();
    bytes[5] = 255;
    assert!(matches!(parse(&bytes), Err(CursorError::InvalidFormat)));
}

#[test]
fn parser_refuses_invalid_empty_and_missing_keys_and_sort_key_overflow() {
    for value in [
        AttributeValue::S(String::new()),
        AttributeValue::N("bad".into()),
        AttributeValue::B(Blob::new([])),
        AttributeValue::S("x".repeat(2049)),
    ] {
        let mut c = cursor();
        c.key.insert("pk".into(), value);
        assert!(matches!(
            parse(&c.to_bytes().unwrap()),
            Err(CursorError::InvalidKey)
        ));
    }
    let mut c = cursor();
    c.key.clear();
    assert!(matches!(
        parse(&c.to_bytes().unwrap()),
        Err(CursorError::InvalidKey)
    ));
    let mut c = cursor();
    c.key.remove("sk");
    assert!(matches!(
        parse(&c.to_bytes().unwrap()),
        Err(CursorError::InvalidKey)
    ));
    let mut b = query();
    const FIELDS: &[FieldDescriptor] = &[
        p::field::<Row, String, Required, KeyOnly, Native<String>>("pk", 0).descriptor(),
        p::field::<Row, String, Required, KeyOnly, Native<String>>("sk", 1).descriptor(),
    ];
    b.schema.fields = FIELDS;
    let mut c = Cursor {
        key: key(),
        binding: Identity::Memory(Box::new(b)),
    };
    c.key
        .insert("sk".into(), AttributeValue::S("x".repeat(1025)));
    assert!(matches!(
        parse(&c.to_bytes().unwrap()),
        Err(CursorError::InvalidKey)
    ));
}

fn scan_with_filter(filter: &[u8]) -> Vec<u8> {
    let mut b = query();
    b.operation = Operation::Scan(None);
    let binding_len = super::bytes::binding_bytes(&b).unwrap().len();
    let mut bytes = b.cursor(Some(key())).unwrap().unwrap().to_bytes().unwrap();
    // Replace absent filter, immediately before projection and consistency.
    bytes.splice(
        1 + binding_len - 3..1 + binding_len - 2,
        filter.iter().copied(),
    );
    bytes
}

#[test]
fn parser_bounds_ast_and_document_depth_and_rejects_untrusted_inner_counts() {
    for depth in [32, 33] {
        // Present filter, predicate Eq, one value operand, nested lists, null.
        let mut filter = vec![1, 0, 2, 0, 0, 0, 1, 2];
        for _ in 0..depth {
            filter.extend([5, 0, 0, 0, 1]);
        }
        filter.push(4);
        let result = parse(&scan_with_filter(&filter));
        if depth == 32 {
            assert!(result.is_ok());
        } else {
            assert!(matches!(result, Err(CursorError::DepthLimit)));
        }
        let mut filter = vec![1];
        filter.extend(std::iter::repeat_n(3, depth));
        filter.extend([0, 0, 0, 0, 0, 0]); // Exists with zero operands.
        let result = parse(&scan_with_filter(&filter));
        if depth == 32 {
            assert!(result.is_ok());
        } else {
            assert!(matches!(result, Err(CursorError::DepthLimit)));
        }
    }
    for tag in [5, 6, 7, 8, 9] {
        let filter = [1, 0, 2, 0, 0, 0, 1, 2, tag, 255, 255, 255, 255];
        assert!(matches!(
            parse(&scan_with_filter(&filter)),
            Err(CursorError::TooLarge)
        ));
    }
    // Unsorted/duplicate map names and set members are not canonical V1 data.
    for tag in [6, 7, 9] {
        let mut filter = vec![1, 0, 2, 0, 0, 0, 1, 2, tag, 0, 0, 0, 2];
        for name in *b"za" {
            filter.extend([0, 0, 0, 1, name]);
            if tag == 6 {
                filter.push(4);
            }
        }
        assert!(matches!(
            parse(&scan_with_filter(&filter)),
            Err(CursorError::InvalidFormat)
        ));
    }
}

#[test]
fn canonicalization_checks_depth_before_recursing_or_cloning() {
    let leaf = || ConditionNode::Predicate {
        operator: Operator::Exists,
        operands: vec![],
    };
    for depth in [32, 33] {
        for branch in 0..3 {
            let mut node = leaf();
            for _ in 0..depth {
                node = match branch {
                    0 => ConditionNode::Not(Box::new(node)),
                    1 => ConditionNode::And(Box::new(node), Box::new(leaf())),
                    _ => ConditionNode::Or(Box::new(node), Box::new(leaf())),
                };
            }
            let result = CanonicalNode::new(&node);
            if depth == 32 {
                assert!(result.is_ok());
            } else {
                assert!(matches!(result, Err(BuildError::InvalidCursor)));
            }
        }
    }
}
