use tabulary::{
    __private, Attr, BuildError, Converter, DerivedOnly, DynamoType, FieldAccess, IndexSourceOnly,
    KeyOnly, Mutable, Native, Optional, Presence, PresenceMarker, Record, RecordSchema, Required,
    VersionOnly,
};

struct Order;
const KEY: Attr<Order, String, Required, KeyOnly> = __private::field("orderRef", 0);
const NOTE: Attr<Order, String, Optional> = __private::field("gift.note", 1);
const QUANTITY: Attr<Order, u64> = __private::field("quantity", 2);
const VERSION: Attr<Order, u64, Required, VersionOnly> = __private::field("version", 3);
const SOURCE: Attr<Order, String, Optional, IndexSourceOnly> = __private::field("courier", 4);
const DERIVED: Attr<Order, String, Optional, DerivedOnly> = __private::field("courierKey", 5);
const EXTERNAL: Attr<Order, u128, Required, Mutable, tabulary::convert::u128_string::Codec> =
    __private::field("externalRef", 6);
impl Record for Order {
    const SCHEMA: RecordSchema = __private::schema(
        "Order",
        &[
            KEY.descriptor(),
            NOTE.descriptor(),
            QUANTITY.descriptor(),
            VERSION.descriptor(),
            SOURCE.descriptor(),
            DERIVED.descriptor(),
            EXTERNAL.descriptor(),
        ],
        __private::keys(0, None),
    );
}

#[test]
fn manual_neutral_descriptors_are_static_and_typed() {
    assert_eq!(Order::SCHEMA.name, "Order");
    assert_eq!(Order::SCHEMA.primary.partition, 0);
    assert_eq!(Order::SCHEMA.primary.sort, None);
    assert_eq!(KEY.name(), "orderRef");
    assert_eq!(KEY.descriptor().access, FieldAccess::KeyOnly);
    assert_eq!(NOTE.descriptor().presence, Presence::Optional);
    assert_eq!(QUANTITY.descriptor().wire, DynamoType::N);
    assert_eq!(EXTERNAL.descriptor().wire, DynamoType::S);
    assert!(KEY.validate().is_ok());
    assert!(VERSION.validate().is_ok());
    assert!(SOURCE.validate().is_ok());
    assert!(DERIVED.validate().is_ok());
    assert!(NOTE.validate_mutation().is_ok());
    assert!(QUANTITY.validate_mutation().is_ok());
    assert!(EXTERNAL.validate_mutation().is_ok());
    fn copy<T: Copy>(value: T) -> (T, T) {
        (value, value)
    }
    let (first, second) = copy(KEY);
    assert_eq!(first.descriptor(), second.descriptor());
}

#[test]
fn forged_descriptors_are_rejected_before_build() {
    fn rejected<T, P, C>(name: &'static str, index: usize)
    where
        P: PresenceMarker,
        C: Converter<T>,
    {
        let attr: Attr<Order, T, P, Mutable, C> = __private::field(name, index);
        assert!(matches!(
            attr.validate_mutation(),
            Err(BuildError::OperationConstraint {
                code: "invalid_attribute_descriptor"
            })
        ));
    }
    rejected::<String, Required, Native<String>>("orderRef", 0);
    rejected::<String, Required, Native<String>>("gift.note", 1);
    rejected::<String, Optional, Native<String>>("other", 1);
    rejected::<bool, Required, Native<bool>>("quantity", 2);
    rejected::<u64, Required, Native<u64>>("version", 3);
    rejected::<String, Optional, Native<String>>("courier", 4);
    rejected::<String, Optional, Native<String>>("courierKey", 5);
    rejected::<u128, Required, Native<u128>>("externalRef", 6);
    rejected::<u64, Required, Native<u64>>("quantity", 7);
}

#[test]
fn physical_keys_remain_immutable_even_if_manual_metadata_marks_them_mutable() {
    struct Invoice;
    const PARTITION: Attr<Invoice, String> = __private::field("invoiceRef", 0);
    const SORT: Attr<Invoice, u64> = __private::field("lineNo", 1);
    impl Record for Invoice {
        const SCHEMA: RecordSchema = __private::schema(
            "Invoice",
            &[PARTITION.descriptor(), SORT.descriptor()],
            __private::keys(0, Some(1)),
        );
    }
    assert_eq!(
        PARTITION.validate_mutation(),
        Err(BuildError::OperationConstraint {
            code: "immutable_key"
        })
    );
    assert_eq!(
        SORT.validate_mutation(),
        Err(BuildError::OperationConstraint {
            code: "immutable_key"
        })
    );
}
