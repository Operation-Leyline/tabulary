use aws_sdk_dynamodb::types::AttributeValue as Av;
use tabulary::condition::Operand;
use tabulary::update::{Action, ActionNode, SetNode};
use tabulary::{
    __private, Attr, BinarySet, BuildError, DerivedOnly, EncodeFault, IndexSourceOnly, KeyOnly,
    List, Mutable, NumberSet, Optional, Record, RecordSchema, Required, StringSet, Update,
    VersionOnly,
};

struct Order;
const ID: Attr<Order, String, Required, KeyOnly> = __private::field("orderRef", 0);
const NOTE: Attr<Order, String, Optional> = __private::field("gift.note", 1);
const COUNT: Attr<Order, u64, Optional> = __private::field("quantity", 2);
const ITEMS: Attr<Order, List<u64>> = __private::field("items", 3);
const TAGS: Attr<Order, StringSet> = __private::field("tags", 4);
const NUMBERS: Attr<Order, NumberSet<u64>> = __private::field("numbers", 5);
const BLOBS: Attr<Order, BinarySet> = __private::field("blobs", 6);
const EXTERNAL: Attr<Order, u128, Required, Mutable, tabulary::convert::u128_string::Codec> =
    __private::field("externalRef", 7);
const VERSION: Attr<Order, u64, Required, VersionOnly> = __private::field("version", 8);
const SOURCE: Attr<Order, u64, Optional, IndexSourceOnly> = __private::field("indexSource", 9);
const DERIVED: Attr<Order, u64, Optional, DerivedOnly> = __private::field("indexKey", 10);
const TOTAL: Attr<Order, u64> = __private::field("total", 11);
impl Record for Order {
    const SCHEMA: RecordSchema = __private::schema(
        "Order",
        &[
            ID.descriptor(),
            NOTE.descriptor(),
            COUNT.descriptor(),
            ITEMS.descriptor(),
            TAGS.descriptor(),
            NUMBERS.descriptor(),
            BLOBS.descriptor(),
            EXTERNAL.descriptor(),
            VERSION.descriptor(),
            SOURCE.descriptor(),
            DERIVED.descriptor(),
            TOTAL.descriptor(),
        ],
        __private::keys(0, None),
    );
}
fn node(action: Action<Order>) -> ActionNode {
    Update::new()
        .and(action)
        .build()
        .unwrap()
        .into_actions()
        .remove(0)
}
fn set(action: Action<Order>, field: tabulary::FieldDescriptor, value: SetNode) {
    assert_eq!(
        node(action),
        ActionNode::Set {
            target: field,
            value
        }
    );
}
fn n(value: &str) -> Operand {
    Operand::Value(Av::N(value.into()))
}
fn path() -> SetNode {
    SetNode::Operand(Operand::Attribute(COUNT.descriptor()))
}
fn initial() -> SetNode {
    SetNode::IfNotExists {
        path: COUNT.descriptor(),
        default: n("0"),
    }
}

#[test]
fn actions_preserve_targets_constants_and_composition_order() {
    let actions = Update::new()
        .and(COUNT.set(7))
        .and(NOTE.remove())
        .and(TOTAL.add(3))
        .and(TAGS.delete_members(StringSet::try_from(vec!["gift".into()]).unwrap()))
        .build()
        .unwrap();
    assert_eq!(
        actions.actions(),
        [
            ActionNode::Set {
                target: COUNT.descriptor(),
                value: SetNode::Operand(n("7"))
            },
            ActionNode::Remove {
                target: NOTE.descriptor()
            },
            ActionNode::Add {
                target: TOTAL.descriptor(),
                value: n("3")
            },
            ActionNode::Delete {
                target: TAGS.descriptor(),
                value: Operand::Value(Av::Ss(vec!["gift".into()]))
            },
        ]
    );
    for (add, delete, target, value) in [
        (
            TAGS.add(StringSet::try_from(vec!["gift".into()]).unwrap()),
            TAGS.delete_members(StringSet::try_from(vec!["gift".into()]).unwrap()),
            TAGS.descriptor(),
            Av::Ss(vec!["gift".into()]),
        ),
        (
            NUMBERS.add(NumberSet::try_from(vec![3]).unwrap()),
            NUMBERS.delete_members(NumberSet::try_from(vec![3]).unwrap()),
            NUMBERS.descriptor(),
            Av::Ns(vec!["3".into()]),
        ),
        (
            BLOBS.add(BinarySet::try_from(vec![vec![1]]).unwrap()),
            BLOBS.delete_members(BinarySet::try_from(vec![vec![1]]).unwrap()),
            BLOBS.descriptor(),
            Av::Bs(vec![vec![1].into()]),
        ),
    ] {
        assert_eq!(
            node(add),
            ActionNode::Add {
                target,
                value: Operand::Value(value.clone())
            }
        );
        assert_eq!(
            node(delete),
            ActionNode::Delete {
                target,
                value: Operand::Value(value)
            }
        );
    }
    set(
        EXTERNAL.set(u128::MAX),
        EXTERNAL.descriptor(),
        SetNode::Operand(Operand::Value(Av::S(u128::MAX.to_string()))),
    );
    assert_eq!(
        Update::<Order>::new().build().unwrap_err(),
        BuildError::EmptyOperation
    );
}

#[test]
fn numeric_defaults_and_arithmetic_are_explicit_not_evaluated_locally() {
    set(
        COUNT.increment(3),
        COUNT.descriptor(),
        SetNode::Plus(Box::new(path()), n("3")),
    );
    set(
        COUNT.decrement(3),
        COUNT.descriptor(),
        SetNode::Minus(Box::new(path()), n("3")),
    );
    set(
        COUNT.set_expr(COUNT.if_not_exists(0)),
        COUNT.descriptor(),
        initial(),
    );
    set(
        COUNT.set_expr(COUNT.if_not_exists(0).plus(3)),
        COUNT.descriptor(),
        SetNode::Plus(Box::new(initial()), n("3")),
    );
    set(
        COUNT.set_expr(COUNT.if_not_exists(0).minus(3)),
        COUNT.descriptor(),
        SetNode::Minus(Box::new(initial()), n("3")),
    );
    set(
        NOTE.set_expr(ID.operand()),
        NOTE.descriptor(),
        SetNode::Operand(Operand::Attribute(ID.descriptor())),
    );
}

#[test]
fn list_defaults_are_only_present_in_append_or_empty() {
    let value = Operand::Value(Av::L(vec![Av::N("3".into())]));
    let plain = SetNode::Operand(Operand::Attribute(ITEMS.descriptor()));
    let initialized = SetNode::IfNotExists {
        path: ITEMS.descriptor(),
        default: Operand::Value(Av::L(vec![])),
    };
    set(
        ITEMS.list_append(List(vec![3])),
        ITEMS.descriptor(),
        SetNode::ListAppend(Box::new(plain), value.clone()),
    );
    set(
        ITEMS.append_or_empty(List(vec![3])),
        ITEMS.descriptor(),
        SetNode::ListAppend(Box::new(initialized), value),
    );
}

#[test]
fn protected_and_forged_targets_and_sources_fail_before_encoding() {
    struct Invoice;
    const PARTITION: Attr<Invoice, u64> = __private::field("invoiceRef", 0);
    const SORT: Attr<Invoice, u64, Optional> = __private::field("lineNo", 1);
    impl Record for Invoice {
        const SCHEMA: RecordSchema = __private::schema(
            "Invoice",
            &[PARTITION.descriptor(), SORT.descriptor()],
            __private::keys(0, Some(1)),
        );
    }
    for action in [
        PARTITION.set(1),
        PARTITION.add(1),
        PARTITION.increment(1),
        SORT.remove(),
    ] {
        assert_eq!(
            Update::new().and(action).build().unwrap_err(),
            BuildError::OperationConstraint {
                code: "immutable_key"
            }
        );
    }
    let forged: Attr<Order, u64, Optional> = __private::field("wrong", 2);
    let version: Attr<Order, u64> = __private::field("version", 8);
    let source: Attr<Order, u64, Optional> = __private::field("indexSource", 9);
    let derived: Attr<Order, u64, Optional> = __private::field("indexKey", 10);
    for action in [
        forged.set(1),
        forged.remove(),
        forged.add(1),
        forged.increment(1),
        COUNT.set_expr(forged.if_not_exists(0)),
        version.set(1),
        source.remove(),
        derived.add(1),
    ] {
        assert_eq!(
            Update::new().and(action).build().unwrap_err(),
            BuildError::OperationConstraint {
                code: "invalid_attribute_descriptor"
            }
        );
    }
    let float: Attr<Order, f64, Optional> = __private::field("quantity", 2);
    for action in [
        float.set(f64::NAN),
        float.increment(f64::NAN),
        float.set_expr(float.if_not_exists(f64::NAN)),
    ] {
        match Update::new().and(action).build().unwrap_err() {
            BuildError::Encode(error) => {
                assert_eq!(error.kind, EncodeFault::NonFiniteFloat);
                assert_eq!(error.record, "Order");
                assert_eq!(error.attribute(), Some("quantity"));
            }
            _ => panic!("expected structured encode fault"),
        }
    }
}

#[test]
fn conversion_is_deferred_and_debug_never_displays_constants() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static CALLS: AtomicUsize = AtomicUsize::new(0);
    struct Uppercase;
    impl tabulary::Converter<String> for Uppercase {
        type Wire = tabulary::wire::String;
        fn encode(
            value: &String,
            _: &mut tabulary::EncodeContext,
        ) -> Result<Av, tabulary::EncodeError> {
            CALLS.fetch_add(1, Ordering::Relaxed);
            Ok(Av::S(value.to_uppercase()))
        }
        fn decode(
            _: &Av,
            _: &mut tabulary::DecodeContext,
        ) -> Result<String, tabulary::DecodeError> {
            unreachable!()
        }
    }
    let field: Attr<Order, String, Optional, Mutable, Uppercase> = __private::field("gift.note", 1);
    let forged: Attr<Order, String, Optional, Mutable, Uppercase> = __private::field("wrong", 1);
    assert!(
        Update::new()
            .and(forged.set("secret".into()))
            .build()
            .is_err()
    );
    assert!(
        Update::new()
            .and(field.set_expr(forged.if_not_exists("secret".into())))
            .build()
            .is_err()
    );
    assert_eq!(CALLS.load(Ordering::Relaxed), 0);
    let action = field.set_expr(field.if_not_exists("private-value".into()));
    assert!(!format!("{action:?}").contains("private-value"));
    let update = Update::new().and(action);
    assert_eq!(CALLS.load(Ordering::Relaxed), 0);
    assert!(!format!("{update:?}").contains("private-value"));
    let built = update.build().unwrap();
    assert_eq!(CALLS.load(Ordering::Relaxed), 1);
    assert!(!format!("{built:?}").contains("PRIVATE-VALUE"));
    assert_eq!(
        built.actions(),
        [ActionNode::Set {
            target: field.descriptor(),
            value: SetNode::IfNotExists {
                path: field.descriptor(),
                default: Operand::Value(Av::S("PRIVATE-VALUE".into()))
            }
        }]
    );
}
