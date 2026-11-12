#![cfg(feature = "derive")]
use aws_sdk_dynamodb::types::ScalarAttributeType as Scalar;
use tabulary::Record;

#[derive(Debug, PartialEq, tabulary::Record)]
#[tabulary(rename_all = "camelCase",
    index(ident = ByCourier, name = "by_courier", kind = global, partition = courier, project = include(gift_note)),
    index(ident = ByLine, name = "by_line", kind = local, partition = order_ref, sort = line_no, project = keys_only))]
struct Order {
    #[tabulary(partition_key)]
    order_ref: String,
    #[tabulary(sort_key)]
    created: u64,
    courier: String,
    line_no: u64,
    gift_note: Option<String>,
}

#[derive(tabulary::Record)]
#[tabulary(crate = "::tabulary", generated_prefix = "Stored", rename_all = "snake_case",
    index(ident = ByValue, name = "by_value", kind = global, partition = value, sort = sequence, project = all))]
struct Generic<T> {
    #[tabulary(partition_key)]
    id: String,
    value: T,
    sequence: u64,
}
#[test]
fn manual_key_name_collisions_are_rejected() {
    const FIELDS: [tabulary::FieldDescriptor; 2] = [
        Order::SCHEMA.fields[0],
        tabulary::FieldDescriptor {
            index: 1,
            ..Order::SCHEMA.fields[0]
        },
    ];
    let schema =
        tabulary::__private::schema("Manual", &FIELDS, tabulary::__private::keys(0, Some(1)));
    assert_eq!(
        schema.attribute_definitions(),
        Err(tabulary::BuildError::IncompatibleIndex)
    );
}

#[test]
fn definitions_include_only_physical_keys_in_declaration_order() {
    let definitions = Order::SCHEMA.attribute_definitions().unwrap();
    assert_eq!(
        definitions
            .iter()
            .map(|d| d.attribute_name())
            .collect::<Vec<_>>(),
        ["orderRef", "created", "courier", "lineNo"]
    );
    assert!(definitions.iter().all(|d| d.attribute_name() != "giftNote"));
    assert_eq!(
        definitions
            .iter()
            .map(|d| d.attribute_type())
            .collect::<Vec<_>>(),
        [&Scalar::S, &Scalar::N, &Scalar::S, &Scalar::N]
    );
    assert_eq!(Generic::<Vec<u8>>::SCHEMA.attribute_definitions().unwrap()[1].attribute_type(), &Scalar::B);
    let mut invalid = Generic::<Vec<u8>>::SCHEMA;
    invalid.primary.partition = 99;
    assert_eq!(invalid.attribute_definitions(), Err(tabulary::BuildError::IncompatibleIndex));
}
