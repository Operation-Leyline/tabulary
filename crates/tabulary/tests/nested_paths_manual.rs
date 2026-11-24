//! Nested paths without derives: manual document metadata opts into traversal.
//! This file runs under `--no-default-features`.
use aws_sdk_dynamodb::types::AttributeValue as Av;
use tabulary::{
    __private, Attr, BuildError, DecodeContext, DecodeError, Document, EncodeContext, EncodeError,
    Item, Optional, Record, RecordSchema, Update, Value, wire,
};

struct Shipping;
const STATUS: Attr<Shipping, String> = __private::field("status", 0);
const ATTEMPTS: Attr<Shipping, u64> = __private::field("attempts", 1);
const NOTE: Attr<Shipping, String, Optional> = __private::field("note", 2);
impl Document for Shipping {
    const NAME: &'static str = "Shipping";
    const FIELDS: &'static [tabulary::FieldDescriptor] = &[
        STATUS.descriptor(),
        ATTEMPTS.descriptor(),
        NOTE.descriptor(),
    ];
    fn encode_map(&self, _: &mut EncodeContext) -> Result<Item, EncodeError> {
        unreachable!("expressions never encode the parent document")
    }
    fn decode_map(_: &Item, _: &mut DecodeContext) -> Result<Self, DecodeError> {
        unreachable!("expressions never decode")
    }
}
impl Value for Shipping {
    type Wire = wire::Map;
    fn encode_value(&self, _: &mut EncodeContext) -> Result<Av, EncodeError> {
        unreachable!("expressions never encode the parent document")
    }
    fn decode_value(_: &Av, _: &mut DecodeContext) -> Result<Self, DecodeError> {
        unreachable!("expressions never decode")
    }
}
/// A document without published metadata cannot be traversed.
struct Opaque;
impl Document for Opaque {
    const NAME: &'static str = "Opaque";
    fn encode_map(&self, _: &mut EncodeContext) -> Result<Item, EncodeError> {
        unreachable!()
    }
    fn decode_map(_: &Item, _: &mut DecodeContext) -> Result<Self, DecodeError> {
        unreachable!()
    }
}
impl Value for Opaque {
    type Wire = wire::Map;
    fn encode_value(&self, _: &mut EncodeContext) -> Result<Av, EncodeError> {
        unreachable!()
    }
    fn decode_value(_: &Av, _: &mut DecodeContext) -> Result<Self, DecodeError> {
        unreachable!()
    }
}

struct Order;
const ORDER_REF: Attr<Order, String> = __private::field("order_ref", 0);
const SHIPPING: Attr<Order, Shipping> = __private::field("shipping", 1);
const OPAQUE: Attr<Order, Opaque> = __private::field("opaque", 2);
impl Record for Order {
    const SCHEMA: RecordSchema = __private::schema(
        "Order",
        &[
            ORDER_REF.descriptor(),
            SHIPPING.descriptor(),
            OPAQUE.descriptor(),
        ],
        __private::keys(0, None),
    );
}

#[test]
fn manual_documents_render_aliased_nested_conditions_and_updates() {
    let condition = SHIPPING
        .field(STATUS)
        .eq("packed".into())
        .and(SHIPPING.field(NOTE).not_exists())
        .build()
        .unwrap();
    let update = Update::new()
        .and(SHIPPING.field(ATTEMPTS).add(1))
        .and(SHIPPING.field(STATUS).set_if_not_exists("new".into()))
        .and(SHIPPING.field(NOTE).remove())
        .build()
        .unwrap();
    let rendered = update.render_with_condition(&condition);
    assert_eq!(
        rendered.update(),
        Some("SET #n0.#n1 = if_not_exists(#n0.#n1, :v0) REMOVE #n0.#n2 ADD #n0.#n3 :v1")
    );
    assert_eq!(
        rendered.condition(),
        Some("(#n0.#n1 = :v2 AND attribute_not_exists(#n0.#n2))")
    );
    let names = rendered.names();
    assert_eq!(names["#n0"], "shipping");
    assert_eq!(names["#n1"], "status");
    assert_eq!(names["#n2"], "note");
    assert_eq!(names["#n3"], "attempts");
    assert_eq!(rendered.values()[":v1"], Av::N("1".into()));
}

#[test]
fn foreign_or_unpublished_descriptors_are_refused_at_build() {
    let forged: Attr<Shipping, String> = __private::field("status", 1);
    let refused = BuildError::OperationConstraint {
        code: "invalid_attribute_descriptor",
    };
    assert_eq!(
        SHIPPING.field(forged).eq("x".into()).build().unwrap_err(),
        refused
    );
    let hidden: Attr<Opaque, u64> = __private::field("count", 0);
    assert_eq!(OPAQUE.field(hidden).gt(1).build().unwrap_err(), refused);
    assert_eq!(
        Update::new()
            .and(OPAQUE.field(hidden).add(1))
            .build()
            .unwrap_err(),
        refused
    );
}
