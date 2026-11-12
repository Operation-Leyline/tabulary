use aws_sdk_dynamodb::types::AttributeValue as Av;
use tabulary::condition::{ConditionNode as Node, Operand, Operator};
use tabulary::{
    __private, Attr, BinarySet, BuildError, Condition, Converter, EncodeContext, EncodeFault,
    KeyOnly, List, Mutable, Native, NumberSet, Optional, Record, RecordSchema, Required, StringSet,
};

struct Order;
const ID: Attr<Order, String, Required, KeyOnly> = __private::field("orderRef", 0);
const NOTE: Attr<Order, String, Optional> = __private::field("gift.note", 1);
const COUNT: Attr<Order, u64> = __private::field("quantity", 2);
const OTHER: Attr<Order, u64> = __private::field("other", 3);
const ITEMS: Attr<Order, List<u64>> = __private::field("items", 4);
const EXTERNAL: Attr<Order, u128, Required, Mutable, tabulary::convert::u128_string::Codec> =
    __private::field("externalRef", 5);
const BINARY: Attr<Order, Vec<u8>> = __private::field("binary", 6);
const TAGS: Attr<Order, StringSet> = __private::field("tags", 7);
const NUMBERS: Attr<Order, NumberSet<u64>> = __private::field("numbers", 8);
const BLOBS: Attr<Order, BinarySet> = __private::field("blobs", 9);
impl Record for Order {
    const SCHEMA: RecordSchema = __private::schema(
        "Order",
        &[
            ID.descriptor(),
            NOTE.descriptor(),
            COUNT.descriptor(),
            OTHER.descriptor(),
            ITEMS.descriptor(),
            EXTERNAL.descriptor(),
            BINARY.descriptor(),
            TAGS.descriptor(),
            NUMBERS.descriptor(),
            BLOBS.descriptor(),
        ],
        __private::keys(0, None),
    );
}

fn predicate(condition: Condition<Order>, operator: Operator) -> Vec<Operand> {
    match condition.build().unwrap().into_root() {
        Node::Predicate {
            operator: actual,
            operands,
        } => {
            assert_eq!(actual, operator);
            operands
        }
        _ => panic!("expected predicate"),
    }
}

macro_rules! check {
    ($condition:expr, $op:ident; $($value:expr),+ $(,)?) => {
        assert_eq!(predicate($condition, Operator::$op)[1..], [$($value),+]);
    };
}

#[test]
fn predicates_preserve_paths_and_encode_typed_constants() {
    use Operand::{Attribute, Size, Value};
    let n = |v: &str| Value(Av::N(v.into()));
    let s = |v: &str| Value(Av::S(v.into()));
    assert_eq!(
        predicate(NOTE.exists(), Operator::Exists),
        [Attribute(NOTE.descriptor())]
    );
    assert_eq!(
        predicate(NOTE.not_exists(), Operator::NotExists),
        [Attribute(NOTE.descriptor())]
    );
    for (condition, op) in [
        (COUNT.eq(3), Operator::Eq),
        (COUNT.ne(3), Operator::Ne),
        (COUNT.lt(3), Operator::Lt),
        (COUNT.le(3), Operator::Le),
        (COUNT.gt(3), Operator::Gt),
        (COUNT.ge(3), Operator::Ge),
    ] {
        assert_eq!(
            predicate(condition, op),
            [Attribute(COUNT.descriptor()), n("3")]
        );
    }
    check!(COUNT.eq_path(OTHER), Eq; Attribute(OTHER.descriptor()));
    check!(NOTE.eq_path(ID), Eq; Attribute(ID.descriptor()));
    check!(COUNT.between(1, 4), Between; n("1"), n("4"));
    check!(EXTERNAL.eq(u128::MAX), Eq; s(&u128::MAX.to_string()));
    check!(NOTE.begins_with("gift".into()), BeginsWith; s("gift"));
    check!(NOTE.contains("card".into()), Contains; s("card"));
    check!(ITEMS.contains(7), Contains; n("7"));
    let binary = || Value(Av::B(vec![1, 2].into()));
    check!(BINARY.begins_with(vec![1, 2]), BeginsWith; binary());
    check!(BINARY.lt(vec![1, 2]), Lt; binary());
    check!(TAGS.contains("card".into()), Contains; s("card"));
    check!(NUMBERS.contains(7), Contains; n("7"));
    check!(BLOBS.contains(vec![1, 2]), Contains; binary());
    assert_eq!(
        predicate(ITEMS.size().ge(2), Operator::Ge),
        [Size(ITEMS.descriptor()), n("2")]
    );
}

#[test]
fn grouping_is_explicit_and_debug_omits_values() {
    let condition = COUNT
        .eq(1)
        .or(COUNT.eq(2))
        .and(NOTE.eq("private-value".into()).not());
    assert!(!format!("{condition:?}").contains("private-value"));
    let built = condition.build().unwrap();
    assert!(!format!("{built:?}").contains("private-value"));
    match built.into_root() {
        Node::And(left, right) => {
            assert!(matches!(*left, Node::Or(_, _)));
            assert!(matches!(*right, Node::Not(_)));
        }
        _ => panic!("grouping lost"),
    }
}

#[test]
fn in_is_nonempty_and_bounded_before_encoding() {
    for length in [1, 100] {
        assert_eq!(
            predicate(COUNT.in_values(vec![1; length]), Operator::In).len(),
            length + 1
        );
    }
    assert_eq!(
        COUNT.in_values(Vec::new()).build().unwrap_err(),
        BuildError::EmptyOperation
    );
    assert_eq!(
        COUNT.in_values(vec![1; 101]).build().unwrap_err(),
        BuildError::RequestLimit {
            max: 100,
            actual: 101
        }
    );
}

#[test]
fn custom_codec_is_deferred_and_used_for_contains_elements() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static CALLS: AtomicUsize = AtomicUsize::new(0);
    struct Uppercase;
    impl Converter<String> for Uppercase {
        type Wire = tabulary::wire::String;
        fn encode(value: &String, cx: &mut EncodeContext) -> Result<Av, tabulary::EncodeError> {
            CALLS.fetch_add(1, Ordering::Relaxed);
            Native::<String>::encode(&value.to_uppercase(), cx)
        }
        fn decode(
            value: &Av,
            cx: &mut tabulary::DecodeContext,
        ) -> Result<String, tabulary::DecodeError> {
            Native::<String>::decode(value, cx)
        }
    }
    impl tabulary::condition::Contains<String> for Uppercase {
        type Element = String;
        type ElementCodec = Uppercase;
    }
    let field: Attr<Order, String, Optional, Mutable, Uppercase> = __private::field("gift.note", 1);
    let condition = field.eq("gift".into()).and(field.contains("card".into()));
    assert_eq!(CALLS.load(Ordering::Relaxed), 0);
    let root = condition.build().unwrap().into_root();
    assert_eq!(CALLS.load(Ordering::Relaxed), 2);
    if let Node::And(left, right) = root {
        for (node, text) in [(left, "GIFT"), (right, "CARD")] {
            assert!(
                matches!(*node, Node::Predicate { operands, .. } if operands[1] == Operand::Value(Av::S(text.into())))
            );
        }
    } else {
        panic!("expected conjunction");
    }
    assert_eq!(
        field
            .in_values(vec!["gift".into(); 101])
            .build()
            .unwrap_err(),
        BuildError::RequestLimit {
            max: 100,
            actual: 101
        }
    );
    assert_eq!(CALLS.load(Ordering::Relaxed), 2);
}

#[test]
fn invalid_handles_and_values_fail_at_build_with_context() {
    let forged: Attr<Order, u64> = __private::field("wrong", 2);
    for condition in [forged.exists(), forged.eq(1), COUNT.eq_path(forged)] {
        assert_eq!(
            condition.build().unwrap_err(),
            BuildError::OperationConstraint {
                code: "invalid_attribute_descriptor"
            }
        );
    }
    let native: Attr<Order, u128> = __private::field("externalRef", 5);
    assert!(native.eq(1).build().is_err());
    const FLOAT: Attr<Order, f64> = __private::field("quantity", 2);
    let condition = FLOAT.eq(f64::NAN);
    let error = condition.build().unwrap_err();
    match error {
        BuildError::Encode(error) => {
            assert_eq!(error.kind, EncodeFault::NonFiniteFloat);
            assert_eq!(error.record, "Order");
            assert_eq!(error.attribute(), Some("quantity"));
        }
        _ => panic!("expected structured encoding error"),
    }
}
