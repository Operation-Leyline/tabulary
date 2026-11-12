use aws_sdk_dynamodb::Client;
use std::error::Error;
use tabulary::{BilledSize, ReadConsistency, Record, Table};

#[derive(Debug, PartialEq, Record)]
#[tabulary(rename_all = "snake_case")]
pub struct Order {
    #[tabulary(partition_key)]
    order_ref: String,
    #[tabulary(sort_key)]
    line_no: u64,
    quantity: u64,
}

pub fn round_trip() -> Result<(), Box<dyn Error>> {
    let order = Order {
        order_ref: "ord-amber".into(),
        line_no: 1,
        quantity: 3,
    };
    let item = order.to_item()?;
    assert_eq!(Order::from_item(&item)?, order);
    assert!(BilledSize::of_item(&item)?.conservative_item_bytes() > 0);

    let mut incomplete = item;
    incomplete.remove(OrderAttrs::QUANTITY.name());
    let error = Order::from_item(&incomplete).unwrap_err();
    assert_eq!(error.record, "Order");
    assert!(matches!(error.kind, tabulary::DecodeFault::Missing));
    Ok(())
}

pub async fn create_and_read(client: &Client) -> Result<Option<Order>, Box<dyn Error>> {
    let orders = Table::<Order>::new("orders")?;
    let order = Order {
        order_ref: "ord-amber".into(),
        line_no: 1,
        quantity: 3,
    };
    let put = orders
        .put(&order)
        .condition(OrderAttrs::ORDER_REF.not_exists())
        .build()?;

    // The input and actual expression maps are available before the call.
    assert!(put.expressions().condition().is_some());
    assert!(!put.expressions().names().is_empty());
    let raw = put.apply(client.put_item()).send().await;
    put.decode(raw)?;

    let get = orders
        .get(
            OrderKey::new("ord-amber".into(), 1)?,
            ReadConsistency::Strong,
        )
        .build()?;
    let raw = get.apply(client.get_item()).send().await;
    Ok(get.decode(raw)?.item)
}
