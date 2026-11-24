//! TransactGet and multi-table BatchGet with manual codecs, so these run without derives.
use aws_sdk_dynamodb::{
    operation::{batch_get_item::BatchGetItemOutput, transact_get_items::TransactGetItemsOutput},
    types::{AttributeValue as Av, ItemResponse, KeysAndAttributes},
};
use tabulary::{
    __private, Attr, BatchGetSlotError, BuildError, DecodeContext, DecodeError, DecodeFault,
    Document, EncodeContext, EncodeError, EncodeFault, GetSlotError, Item, KeyOnly, MultiBatchGet,
    PathSegment, ReadConsistency, Record, RecordKey, RecordSchema, Required, Table, TransactGet,
};

struct Order {
    id: String,
    quantity: u64,
}
const ID: Attr<Order, String, Required, KeyOnly> = __private::field("id", 0);
const QUANTITY: Attr<Order, u64> = __private::field("quantity", 1);
impl Record for Order {
    const SCHEMA: RecordSchema = __private::schema(
        "Order",
        &[ID.descriptor(), QUANTITY.descriptor()],
        __private::keys(0, None),
    );
}
fn decode_fault(cx: &DecodeContext, name: &str, kind: DecodeFault) -> DecodeError {
    cx.at(PathSegment::Attribute(name.into())).error(kind)
}
impl Document for Order {
    const NAME: &'static str = "Order";
    fn encode_map(&self, cx: &mut EncodeContext) -> Result<Item, EncodeError> {
        if self.id.is_empty() {
            return Err(cx.error(EncodeFault::EmptyKey));
        }
        Ok(Item::from([
            ("id".into(), Av::S(self.id.clone())),
            ("quantity".into(), Av::N(self.quantity.to_string())),
        ]))
    }
    fn decode_map(item: &Item, cx: &mut DecodeContext) -> Result<Self, DecodeError> {
        let Some(Av::S(id)) = item.get("id") else {
            return Err(decode_fault(cx, "id", DecodeFault::Missing));
        };
        let quantity = match item.get("quantity") {
            None => return Err(decode_fault(cx, "quantity", DecodeFault::Missing)),
            Some(Av::N(n)) => n
                .parse()
                .map_err(|_| decode_fault(cx, "quantity", DecodeFault::InvalidNumber))?,
            Some(_) => {
                return Err(decode_fault(
                    cx,
                    "quantity",
                    DecodeFault::WrongType {
                        expected: tabulary::DynamoType::N,
                        actual: tabulary::DynamoType::S,
                    },
                ));
            }
        };
        Ok(Self {
            id: id.clone(),
            quantity,
        })
    }
}
struct OrderKey(String);
impl RecordKey<Order> for OrderKey {
    // Deliberately unvalidated: request builders own the physical key checks.
    fn encode_key(&self) -> Result<Item, EncodeError> {
        Ok(Item::from([("id".into(), Av::S(self.0.clone()))]))
    }
    fn decode_key(item: &Item) -> Result<Self, DecodeError> {
        match item.get("id") {
            Some(Av::S(id)) => Ok(Self(id.clone())),
            _ => Err(decode_fault(
                &DecodeContext::new("Order"),
                "id",
                DecodeFault::Missing,
            )),
        }
    }
}
fn order(id: &str, quantity: u64) -> Item {
    Order {
        id: id.into(),
        quantity,
    }
    .encode_map(&mut EncodeContext::new("Order"))
    .unwrap()
}

#[test]
fn transact_get_reads_two_tables_with_manual_codecs() {
    let mut tx = TransactGet::new();
    let current = tx.get(
        &Table::<Order>::new("orders").unwrap(),
        OrderKey("order-1".into()),
    );
    let archived = tx.get(
        &Table::<Order>::new("orders-archive").unwrap(),
        OrderKey("order-1".into()),
    );
    let corrupt = tx.get(
        &Table::<Order>::new("orders").unwrap(),
        OrderKey("order-2".into()),
    );
    let request = tx.build().unwrap();
    let mut wrong = order("order-2", 1);
    wrong.insert("quantity".into(), Av::S("one".into()));
    let output = TransactGetItemsOutput::builder()
        .set_responses(Some(vec![
            ItemResponse::builder()
                .item("id", Av::S("order-1".into()))
                .item("quantity", Av::N("3".into()))
                .build(),
            ItemResponse::builder().build(),
            ItemResponse::builder().set_item(Some(wrong)).build(),
        ]))
        .build();
    let response = request.decode(Ok(output)).unwrap();
    assert_eq!(response.get(&current).unwrap().unwrap().quantity, 3);
    assert!(response.get(&archived).unwrap().is_none());
    let Err(GetSlotError::Decode(error)) = response.get(&corrupt) else {
        panic!("expected decode")
    };
    assert_eq!(error.attribute(), Some("quantity"));
}

#[test]
fn key_faults_surface_at_build_for_both_requests() {
    let orders = Table::<Order>::new("orders").unwrap();
    let mut tx = TransactGet::new();
    tx.get(&orders, OrderKey("order-1".into()));
    tx.get(&orders, OrderKey(String::new()));
    let Err(BuildError::Encode(error)) = tx.build() else {
        panic!("expected an encode fault")
    };
    assert_eq!(error.kind, EncodeFault::EmptyKey);
    assert_eq!(error.attribute(), Some("id"));

    let mut tx = TransactGet::new();
    tx.get(&orders, OrderKey("x".repeat(2049)));
    assert!(matches!(
        tx.build(),
        Err(BuildError::Encode(error)) if matches!(error.kind, EncodeFault::KeySize { max: 2048, actual: 2049 })
    ));

    let mut batch = MultiBatchGet::new();
    batch.get(
        &orders,
        vec![OrderKey("order-1".into()), OrderKey(String::new())],
        ReadConsistency::Eventual,
    );
    assert!(matches!(
        batch.build(),
        Err(BuildError::Encode(error)) if error.kind == EncodeFault::EmptyKey
    ));
}

#[test]
fn multi_batch_get_returns_typed_unprocessed_keys_with_manual_codecs() {
    let mut batch = MultiBatchGet::new();
    let current = batch.get(
        &Table::<Order>::new("orders").unwrap(),
        vec![OrderKey("order-1".into()), OrderKey("order-2".into())],
        ReadConsistency::Strong,
    );
    let archived = batch.get(
        &Table::<Order>::new("orders-archive").unwrap(),
        vec![OrderKey("order-3".into())],
        ReadConsistency::Eventual,
    );
    let request = batch.build().unwrap();
    let output = BatchGetItemOutput::builder()
        .responses("orders", vec![order("order-2", 5)])
        .unprocessed_keys(
            "orders-archive",
            KeysAndAttributes::builder()
                .keys(Item::from([("id".into(), Av::S("order-3".into()))]))
                .build()
                .unwrap(),
        )
        .build();
    let response = request.decode(Ok(output)).unwrap();
    let current = response.get(&current).unwrap();
    assert_eq!(current.found.len(), 1);
    assert_eq!(current.found[0].quantity, 5);
    assert_eq!(
        current
            .missing
            .iter()
            .map(|k| k.0.as_str())
            .collect::<Vec<_>>(),
        ["order-1"]
    );
    let archived = response.get(&archived).unwrap();
    assert!(archived.found.is_empty());
    assert_eq!(archived.unprocessed[0].0, "order-3");

    // A malformed item fails only its own table.
    let mut batch = MultiBatchGet::new();
    let slot = batch.get(
        &Table::<Order>::new("orders").unwrap(),
        vec![OrderKey("order-1".into())],
        ReadConsistency::Strong,
    );
    let request = batch.build().unwrap();
    let output = BatchGetItemOutput::builder()
        .responses(
            "orders",
            vec![Item::from([("id".into(), Av::S("order-1".into()))])],
        )
        .build();
    let response = request.decode(Ok(output)).unwrap();
    let Err(BatchGetSlotError::Decode { failures }) = response.get(&slot) else {
        panic!("expected decode")
    };
    assert_eq!(failures.len(), 1);
    assert_eq!(failures[0].0, 0);
    assert_eq!(failures[0].1.kind, DecodeFault::Missing);
}

/// Counts encoder calls; used only by the over-limit test below.
static ENCODED: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
struct CountedKey(String);
impl RecordKey<Order> for CountedKey {
    fn encode_key(&self) -> Result<Item, EncodeError> {
        ENCODED.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        OrderKey(self.0.clone()).encode_key()
    }
    fn decode_key(item: &Item) -> Result<Self, DecodeError> {
        OrderKey::decode_key(item).map(|key| Self(key.0))
    }
}

#[test]
fn over_limit_or_faulty_requests_stop_encoding_further_keys() {
    use std::sync::atomic::Ordering::SeqCst;
    let orders = Table::<Order>::new("orders").unwrap();
    let counted = |i: usize| CountedKey(format!("order-{i}"));

    let mut tx = TransactGet::new();
    for i in 0..150 {
        tx.get(&orders, counted(i));
    }
    assert_eq!(ENCODED.swap(0, SeqCst), 100);
    assert_eq!(
        tx.build().unwrap_err(),
        BuildError::RequestLimit {
            max: 100,
            actual: 150
        }
    );

    // After the first fault no later read is prepared; the fault is reported.
    let mut tx = TransactGet::new();
    tx.get(&orders, CountedKey(String::new()));
    for i in 0..10 {
        tx.get(&orders, counted(i));
    }
    assert_eq!(ENCODED.swap(0, SeqCst), 1);
    assert!(matches!(tx.build(), Err(BuildError::Encode(_))));

    let mut batch = MultiBatchGet::new();
    batch.get(
        &orders,
        (0..60).map(counted).collect(),
        ReadConsistency::Strong,
    );
    batch.get(
        &Table::<Order>::new("orders-archive").unwrap(),
        (0..10_000).map(counted).collect(),
        ReadConsistency::Strong,
    );
    batch.get(
        &Table::<Order>::new("orders-history").unwrap(),
        (0..10).map(counted).collect(),
        ReadConsistency::Strong,
    );
    assert_eq!(ENCODED.swap(0, SeqCst), 60);
    assert_eq!(
        batch.build().unwrap_err(),
        BuildError::RequestLimit {
            max: 100,
            actual: 10_070
        }
    );

    // Many empty tables fault once and retain nothing.
    let mut batch = MultiBatchGet::new();
    for i in 0..1_000 {
        batch.get(
            &Table::<Order>::new(format!("orders-{i}")).unwrap(),
            Vec::<CountedKey>::new(),
            ReadConsistency::Strong,
        );
    }
    batch.get(&orders, vec![counted(1)], ReadConsistency::Strong);
    assert_eq!(ENCODED.swap(0, SeqCst), 0);
    assert_eq!(batch.build().unwrap_err(), BuildError::EmptyOperation);
}
