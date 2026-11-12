use super::*;
use crate::{FieldAccess, FieldDescriptor, IndexDefinition};

struct Manual;
impl Record for Manual {
    const SCHEMA: RecordSchema = crate::__private::schema(
        "Manual",
        &[
            field("pk", 0, DynamoType::S),
            field("sk", 1, DynamoType::N),
            field("other", 2, DynamoType::B),
            field("note", 3, DynamoType::S),
        ],
        KeyDefinition {
            partition: 0,
            sort: Some(1),
        },
    );
}
const fn field(name: &'static str, index: usize, wire: DynamoType) -> FieldDescriptor {
    FieldDescriptor {
        name,
        index,
        presence: Presence::Required,
        access: FieldAccess::Mutable,
        wire,
    }
}
fn definition(schema: RecordSchema) -> TableDefinition<Manual> {
    TableDefinition::new(schema)
}
fn indexes(indexes: Vec<crate::IndexDefinition>) -> RecordSchema {
    RecordSchema {
        indexes: Box::leak(indexes.into_boxed_slice()),
        ..Manual::SCHEMA
    }
}
fn index(
    name: &'static str,
    kind: IndexKind,
    partition: usize,
    sort: Option<usize>,
    projection: IndexProjection,
) -> IndexDefinition {
    IndexDefinition {
        name,
        kind,
        keys: KeyDefinition { partition, sort },
        projection,
    }
}
fn valid_index() -> IndexDefinition {
    index("by_other", IndexKind::Global, 2, None, IndexProjection::All)
}
fn refuses(schema: RecordSchema) {
    assert_eq!(
        definition(schema).create_input("records", Billing::OnDemand),
        Err(BuildError::IncompatibleIndex)
    );
}

#[test]
fn malformed_manual_keys_indexes_and_projections_are_refused() {
    for (partition, sort) in [(99, None), (0, Some(99)), (0, Some(0))] {
        refuses(RecordSchema {
            primary: KeyDefinition { partition, sort },
            ..Manual::SCHEMA
        });
    }
    for wire in [
        DynamoType::Bool,
        DynamoType::L,
        DynamoType::M,
        DynamoType::Unknown,
    ] {
        let fields = Box::leak(vec![field("pk", 0, wire)].into_boxed_slice());
        refuses(crate::__private::schema(
            "Invalid",
            fields,
            KeyDefinition {
                partition: 0,
                sort: None,
            },
        ));
    }
    let mut optional = Manual::SCHEMA.fields.to_vec();
    optional[0].presence = Presence::Optional;
    refuses(RecordSchema {
        fields: Box::leak(optional.into_boxed_slice()),
        ..Manual::SCHEMA
    });
    for bad in [
        index(
            "by_local",
            IndexKind::Local,
            2,
            Some(1),
            IndexProjection::All,
        ),
        index("by_local", IndexKind::Local, 0, None, IndexProjection::All),
        index(
            "by_local",
            IndexKind::Local,
            0,
            Some(1),
            IndexProjection::All,
        ),
        index("bad/name", IndexKind::Global, 2, None, IndexProjection::All),
        index(
            "by_other",
            IndexKind::Global,
            99,
            None,
            IndexProjection::All,
        ),
    ] {
        refuses(indexes(vec![bad]));
    }
    let local = index(
        "by_local",
        IndexKind::Local,
        0,
        Some(2),
        IndexProjection::All,
    );
    assert!(
        definition(indexes(vec![local]))
            .create_input("records", Billing::OnDemand)
            .is_ok()
    );
    refuses(RecordSchema {
        primary: KeyDefinition {
            partition: 0,
            sort: None,
        },
        ..indexes(vec![local])
    });
    for names in [
        &[][..],
        &["pk"],
        &["other"],
        &["missing"],
        &["note", "note"],
    ] {
        let names = Box::leak(names.to_vec().into_boxed_slice());
        refuses(indexes(vec![IndexDefinition {
            projection: IndexProjection::Include(names),
            ..valid_index()
        }]));
    }
    refuses(indexes(vec![valid_index(), valid_index()]));
}

#[test]
fn index_and_projection_limits_apply_to_the_whole_table() {
    for (kind, max) in [(IndexKind::Global, 20), (IndexKind::Local, 5)] {
        let values: Vec<_> = (0..=max)
            .map(|n| {
                let name = Box::leak(format!("index_{n}").into_boxed_str());
                index(
                    name,
                    kind,
                    if kind == IndexKind::Local { 0 } else { 2 },
                    Some(if kind == IndexKind::Local { 2 } else { 1 }),
                    IndexProjection::All,
                )
            })
            .collect();
        assert!(
            definition(indexes(values[..max].to_vec()))
                .create_input("records", Billing::OnDemand)
                .is_ok()
        );
        refuses(indexes(values));
    }
    let names: Vec<_> = (0..21)
        .map(|n| &*Box::leak(format!("note_{n}").into_boxed_str()))
        .collect();
    let mut fields = Manual::SCHEMA.fields.to_vec();
    fields.extend(
        names
            .iter()
            .enumerate()
            .map(|(i, n)| field(n, i + 4, DynamoType::S)),
    );
    let fields = Box::leak(fields.into_boxed_slice());
    let names = Box::leak(names.into_boxed_slice());
    let projected = |count, total| RecordSchema {
        fields,
        ..indexes(
            (0..count)
                .map(|i| {
                    index(
                        Box::leak(format!("index_{i}").into_boxed_str()),
                        IndexKind::Global,
                        2,
                        None,
                        IndexProjection::Include(&names[..total]),
                    )
                })
                .collect(),
        )
    };
    assert!(
        definition(projected(5, 20))
            .create_input("records", Billing::OnDemand)
            .is_ok()
    );
    refuses(projected(1, 21));
    refuses(projected(6, 20));
    let left = projected(3, 20);
    let mut right = projected(3, 20);
    right.indexes = Box::leak(
        right
            .indexes
            .iter()
            .enumerate()
            .map(|(i, index)| IndexDefinition {
                name: Box::leak(format!("right_{i}").into_boxed_str()),
                ..*index
            })
            .collect::<Vec<_>>()
            .into_boxed_slice(),
    );
    for schema in [left, right] {
        assert!(
            definition(schema)
                .create_input("records", Billing::OnDemand)
                .is_ok()
        );
    }
    assert!(definition(left).merge(definition(right)).is_err());
}

#[test]
fn merges_compare_physical_names_types_projections_and_ttl_not_field_positions() {
    let base = indexes(vec![valid_index()]);
    let union = definition(base)
        .merge(definition(indexes(vec![IndexDefinition {
            name: "another_index",
            ..valid_index()
        }])))
        .unwrap()
        .create_input("records", Billing::OnDemand)
        .unwrap();
    assert_eq!(union.global_secondary_indexes().len(), 2);
    assert_eq!(union.attribute_definitions().len(), 3);
    let mut fields = Manual::SCHEMA.fields.to_vec();
    fields.swap(0, 2);
    for (i, field) in fields.iter_mut().enumerate() {
        field.index = i;
    }
    let reordered = RecordSchema {
        fields: Box::leak(fields.into_boxed_slice()),
        primary: KeyDefinition {
            partition: 2,
            sort: Some(1),
        },
        ..indexes(vec![IndexDefinition {
            keys: KeyDefinition {
                partition: 0,
                sort: None,
            },
            ..valid_index()
        }])
    };
    assert!(definition(base).merge(definition(reordered)).is_ok());
    let local_collision = indexes(vec![index(
        "by_other",
        IndexKind::Local,
        0,
        Some(2),
        IndexProjection::All,
    )]);
    assert!(definition(base).merge(definition(local_collision)).is_err());
    for other in [
        indexes(vec![IndexDefinition {
            kind: IndexKind::Local,
            ..valid_index()
        }]),
        indexes(vec![IndexDefinition {
            projection: IndexProjection::KeysOnly,
            ..valid_index()
        }]),
        indexes(vec![IndexDefinition {
            keys: KeyDefinition {
                partition: 1,
                sort: None,
            },
            ..valid_index()
        }]),
    ] {
        assert!(definition(base).merge(definition(other)).is_err());
    }
    let mut wrong = Manual::SCHEMA.fields.to_vec();
    wrong[2].wire = DynamoType::S;
    assert!(
        definition(base)
            .merge(definition(RecordSchema {
                fields: Box::leak(wrong.into_boxed_slice()),
                ..Manual::SCHEMA
            }))
            .is_err()
    );
    let ttl = Manual::SCHEMA.with_ttl(1);
    assert!(definition(ttl).merge(definition(Manual::SCHEMA)).is_ok());
    let mut extra = Manual::SCHEMA.fields.to_vec();
    extra.push(field("expires", 4, DynamoType::N));
    let conflicting = RecordSchema {
        fields: Box::leak(extra.into_boxed_slice()),
        ..Manual::SCHEMA
    }
    .with_ttl(4);
    assert!(definition(ttl).merge(definition(conflicting)).is_err());
    assert!(
        definition(Manual::SCHEMA.with_ttl(3))
            .ttl_input("records")
            .is_err()
    );
    assert!(
        definition(Manual::SCHEMA.with_ttl(99))
            .ttl_input("records")
            .is_err()
    );
}

#[test]
fn include_order_is_physical_set_equality_and_conflicting_ttl_field_types_fail() {
    let make = |names| {
        indexes(vec![IndexDefinition {
            projection: IndexProjection::Include(names),
            ..valid_index()
        }])
    };
    let mut extra = Manual::SCHEMA.fields.to_vec();
    extra.push(field("extra", 4, DynamoType::N));
    let fields = Box::leak(extra.into_boxed_slice());
    let a = RecordSchema {
        fields,
        ..make(&["note", "extra"])
    };
    let b = RecordSchema {
        fields,
        ..make(&["extra", "note"])
    };
    assert!(definition(a).merge(definition(b)).is_ok());
    let a = a.with_ttl(4);
    let mut incompatible = fields.to_vec();
    incompatible[4].wire = DynamoType::S;
    let b = RecordSchema {
        fields: Box::leak(incompatible.into_boxed_slice()),
        ..b
    };
    assert!(definition(a).merge(definition(b)).is_err());
}
