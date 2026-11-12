#![cfg(feature = "derive")]

use aws_sdk_dynamodb::{
    error::SdkError,
    operation::update_item::UpdateItemError,
    types::{AttributeValue as Av, error::ConditionalCheckFailedException},
};
use aws_smithy_runtime_api::{client::orchestrator::HttpResponse, http::StatusCode};
use aws_smithy_types::body::SdkBody;
use tabulary::{
    BuildError, DecodeFault, FailureKind, OldItem, OldItemMode, Table, Update, WriteError,
};

#[allow(dead_code)]
#[path = "../../../docs/examples/03_status_record.rs"]
mod status_example;

#[derive(Clone, Debug, PartialEq, tabulary::Variants)]
#[tabulary(tag = "delivery.stage", rename_all = "snake_case")]
enum Stage {
    Waiting {
        queued: u64,
    },
    Sent {
        #[tabulary(with = tabulary::convert::u128_string)]
        tracking: u128,
        note: Option<String>,
    },
    Failed {
        failure: String,
        note: Option<String>,
    },
    Empty,
}

#[derive(Debug, PartialEq, tabulary::Record)]
#[tabulary(rename_all = "snake_case")]
struct Delivery {
    #[tabulary(partition_key)]
    id: String,
    #[tabulary(flatten)]
    stage: Stage,
    quantity: u64,
}

fn row(stage: Stage) -> Delivery {
    Delivery {
        id: "parcel".into(),
        stage,
        quantity: 1,
    }
}
fn sent() -> Stage {
    Stage::Sent {
        tracking: u128::MAX,
        note: None,
    }
}
fn changes(stage: Stage) -> Update<Delivery> {
    Update::new().transition(DeliveryAttrs::STAGE, stage, StageTags::WAITING)
}

#[test]
fn transition_and_common_changes_share_exact_update_and_guard_bindings() {
    let table = Table::<Delivery>::new("deliveries").unwrap();
    let request = table
        .update(DeliveryKey::new("parcel".into()).unwrap())
        .changes(changes(sent()).and(DeliveryAttrs::QUANTITY.set(2)))
        .condition(
            DeliveryAttrs::QUANTITY
                .eq(1)
                .or(DeliveryAttrs::QUANTITY.eq(2)),
        )
        .build()
        .unwrap();
    let expressions = request.expressions();
    assert_eq!(
        expressions.update(),
        Some("SET #n0 = :v0, #n1 = :v1, #n2 = :v2 REMOVE #n3, #n4, #n5")
    );
    assert_eq!(
        expressions.condition(),
        Some("(#n0 = :v3 AND (#n2 = :v4 OR #n2 = :v5))")
    );
    assert_eq!(
        expressions.names(),
        &[
            ("#n0".into(), "delivery.stage".into()),
            ("#n1".into(), "tracking".into()),
            ("#n2".into(), "quantity".into()),
            ("#n3".into(), "failure".into()),
            ("#n4".into(), "note".into()),
            ("#n5".into(), "queued".into()),
        ]
        .into()
    );
    assert_eq!(
        expressions.values(),
        &[
            (":v0".into(), Av::S("sent".into())),
            (":v1".into(), Av::S(u128::MAX.to_string())),
            (":v2".into(), Av::N("2".into())),
            (":v3".into(), Av::S("waiting".into())),
            (":v4".into(), Av::N("1".into())),
            (":v5".into(), Av::N("2".into())),
        ]
        .into()
    );
    assert_eq!(
        request.input().condition_expression.as_deref(),
        expressions.condition()
    );
    assert_eq!(
        request.input().expression_attribute_values.as_ref(),
        Some(expressions.values())
    );
    assert!(!format!("{request:?}").contains(&u128::MAX.to_string()));
    let built = changes(sent()).build().unwrap();
    assert!(built.condition().is_some());
    assert_eq!(built.render().condition(), Some("#n0 = :v2"));
    assert_eq!(
        built
            .render_with_condition(&DeliveryAttrs::STAGE.is(StageTags::WAITING).build().unwrap())
            .condition(),
        Some("(#n0 = :v2 AND #n0 = :v3)")
    );
}

#[test]
fn every_shape_pair_replaces_the_group_including_omitted_shared_fields() {
    use tabulary::{
        condition::Operand,
        update::{ActionNode, SetNode},
    };
    let shapes = [
        Stage::Waiting { queued: 9 },
        sent(),
        Stage::Sent {
            tracking: 7,
            note: Some("old".into()),
        },
        Stage::Failed {
            failure: "late".into(),
            note: Some("retry".into()),
        },
        Stage::Empty,
    ];
    for old in &shapes {
        for new in &shapes {
            let mut item = row(old.clone()).to_item().unwrap();
            let built = changes(new.clone()).build().unwrap();
            for action in built.actions() {
                match action {
                    ActionNode::NestedSet {
                        target,
                        value: SetNode::Operand(Operand::Value(value)),
                    } => {
                        let [tabulary::PathSegment::Attribute(name)] = target.segments() else {
                            panic!("top-level target")
                        };
                        item.insert(name.clone(), value.clone());
                    }
                    ActionNode::NestedRemove { target } => {
                        let [tabulary::PathSegment::Attribute(name)] = target.segments() else {
                            panic!("top-level target")
                        };
                        item.remove(name);
                    }
                    _ => panic!("ordinary SET/REMOVE expansion"),
                }
            }
            assert_eq!(item, row(new.clone()).to_item().unwrap());
            assert_eq!(Delivery::from_item(&item).unwrap(), row(new.clone()));
        }
    }
    for tag in [
        StageTags::WAITING,
        StageTags::SENT,
        StageTags::FAILED,
        StageTags::EMPTY,
    ] {
        let expected =
            tabulary::Value::encode_value(&tag, &mut tabulary::EncodeContext::new("Delivery"))
                .unwrap();
        let built = Update::<Delivery>::new()
            .transition(DeliveryAttrs::STAGE, Stage::Empty, tag)
            .build()
            .unwrap();
        assert_eq!(built.render().values()[":v1"], expected);
    }
}

#[test]
fn transitions_and_common_actions_use_the_existing_overlap_gate() {
    for update in [
        changes(sent()).transition(DeliveryAttrs::STAGE, Stage::Empty, StageTags::SENT),
        changes(sent())
            .and(DeliveryAttrs::QUANTITY.set(2))
            .and(DeliveryAttrs::QUANTITY.increment(1)),
    ] {
        assert!(matches!(
            update.build(),
            Err(BuildError::OverlappingUpdate { .. })
        ));
    }
    let forged = tabulary::__private::field("wrong", 1);
    assert!(matches!(
        Update::<Delivery>::new()
            .transition(forged, sent(), StageTags::WAITING)
            .build(),
        Err(BuildError::OperationConstraint {
            code: "invalid_attribute_descriptor"
        })
    ));
}

#[test]
fn generic_renamed_records_defer_codec_faults_to_build_with_flat_paths() {
    use tabulary as renamed;
    #[derive(renamed::Variants)]
    #[tabulary(tag = "kind", rename_all = "snake_case", crate = "renamed")]
    enum State<T> {
        Active { value: T },
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
    let update = Update::<Parcel<f64>>::new().transition(
        ParcelAttrs::<f64>::STATE,
        State::Active { value: f64::NAN },
        StateTags::EMPTY,
    );
    assert!(!format!("{update:?}").contains("NaN"));
    let Err(BuildError::Encode(error)) = update.build() else {
        panic!("codec fault at build")
    };
    assert_eq!(
        (error.record, error.document, error.attribute()),
        ("Parcel", "State", Some("value"))
    );
    assert_eq!(error.kind, renamed::EncodeFault::NonFiniteFloat);
    assert!(
        Update::<Parcel<String>>::new()
            .transition(
                ParcelAttrs::<String>::STATE,
                State::Active {
                    value: "ready".into()
                },
                StateTags::EMPTY
            )
            .build()
            .is_ok()
    );
    assert!(
        Update::<Parcel<String>>::new()
            .transition(
                ParcelAttrs::<String>::STATE,
                State::Empty,
                StateTags::ACTIVE
            )
            .build()
            .is_ok()
    );
}

#[test]
fn manual_bindings_refuse_key_targets_common_collisions_and_wrong_positions() {
    use tabulary::{__private, Attr, DerivedOnly, FlattenedVariants, Native, Record, Required};
    struct Manual<const MODE: u8>;
    impl<const MODE: u8> Manual<MODE> {
        const FIELDS: &'static [tabulary::FieldDescriptor] = &[
            __private::field::<Self, String, Required, tabulary::KeyOnly, Native<String>>("id", 0)
                .descriptor(),
            __private::field::<Self, StageTag, Required, DerivedOnly, Native<StageTag>>(
                "delivery.stage",
                1,
            )
            .descriptor(),
            __private::field::<Self, String, Required, tabulary::Mutable, Native<String>>(
                if MODE == 1 { "tracking" } else { "common" },
                2,
            )
            .descriptor(),
        ];
    }
    impl<const MODE: u8> Record for Manual<MODE> {
        const SCHEMA: tabulary::RecordSchema = __private::schema(
            "Manual",
            Self::FIELDS,
            __private::keys(if MODE == 2 { 1 } else { 0 }, None),
        );
    }
    impl<const MODE: u8> FlattenedVariants<Stage> for Manual<MODE> {
        const FIELD: usize = if MODE == 3 { 0 } else { 1 };
    }
    fn build<const MODE: u8>() -> Result<tabulary::BuiltUpdate<Manual<MODE>>, BuildError> {
        let field: Attr<Manual<MODE>, StageTag, Required, DerivedOnly> =
            __private::field("delivery.stage", 1);
        Update::new()
            .transition(field, sent(), StageTags::WAITING)
            .build()
    }
    assert!(build::<0>().is_ok());
    for result in [
        build::<1>().map(|_| ()),
        build::<2>().map(|_| ()),
        build::<3>().map(|_| ()),
    ] {
        assert_eq!(
            result,
            Err(BuildError::OperationConstraint {
                code: "invalid_flattened_variant"
            })
        );
    }
}

#[test]
fn failed_transition_decodes_the_old_discriminator_without_assuming_new_shape() {
    let request = Table::<Delivery>::new("deliveries")
        .unwrap()
        .update(DeliveryKey::new("parcel".into()).unwrap())
        .changes(changes(sent()))
        .on_condition_failure(OldItemMode::All)
        .build()
        .unwrap();
    let old = row(Stage::Waiting { queued: 9 });
    let mut malformed = old.to_item().unwrap();
    malformed.insert("tracking".into(), Av::S("stale".into()));
    for raw in [Some(old.to_item().unwrap()), Some(malformed.clone()), None] {
        let sdk = SdkError::service_error(
            UpdateItemError::ConditionalCheckFailedException(
                ConditionalCheckFailedException::builder()
                    .set_item(raw.clone())
                    .build(),
            ),
            HttpResponse::new(StatusCode::try_from(400).unwrap(), SdkBody::empty()),
        );
        let error = request.decode(Err(sdk)).unwrap_err();
        assert!(
            error
                .sdk_error()
                .unwrap()
                .as_service_error()
                .unwrap()
                .is_conditional_check_failed_exception()
        );
        let WriteError::Refused { failure, .. } = error else {
            panic!("condition refusal")
        };
        assert_eq!(failure.kind, FailureKind::ConditionFailed);
        match (raw, failure.old) {
            (Some(raw), OldItem::Decoded(value)) => {
                assert_eq!(raw, old.to_item().unwrap());
                assert_eq!(value, old);
            }
            (
                Some(raw),
                OldItem::Invalid {
                    error,
                    raw: retained,
                },
            ) => {
                assert_eq!(raw, malformed);
                assert_eq!(retained.expose(), &raw);
                assert_eq!(error.kind, DecodeFault::InactiveVariantAttribute);
                assert_eq!(
                    (error.record, error.attribute()),
                    ("Delivery", Some("tracking"))
                );
            }
            (None, OldItem::Unavailable) => {}
            _ => panic!("old image state"),
        }
    }
}

#[test]
fn status_example_is_registered_and_invalid_transition_types_are_rejected() {
    assert!(
        status_example::waiting_item()
            .unwrap()
            .contains_key("deliveryStage")
    );
    trybuild::TestCases::new().compile_fail("tests/ui/transitions/*.rs");
}
