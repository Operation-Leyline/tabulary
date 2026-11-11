use aws_sdk_dynamodb::Client;
use std::{error::Error, num::NonZeroU32};
use tabulary::{BatchBudget, Capacity, ReadConsistency, Record, Table};

#[derive(Record)]
#[tabulary(rename_all = "snake_case")]
pub struct Order {
    #[tabulary(partition_key)]
    order_ref: String,
    #[tabulary(sort_key)]
    line_no: u64,
    quantity: u64,
}

pub async fn read_lines(
    client: &Client,
    max_pages: NonZeroU32,
) -> Result<Vec<Order>, Box<dyn Error>> {
    let orders = Table::<Order>::new("orders")?;
    let mut lines = Vec::new();
    let mut next = None;
    for _ in 0..max_pages.get() {
        let mut query = orders
            .query(
                Order::primary()
                    .partition("ord-amber".into())
                    .sort_between(1, 9),
                ReadConsistency::Strong,
            )
            .limit(NonZeroU32::new(2).unwrap())
            .capacity(Capacity::Total);
        if let Some(cursor) = next.take() {
            query = query.resume(cursor);
        }
        let request = query.build()?;
        let raw = request.apply(client.query()).send().await;
        let page = request.decode(raw)?;
        lines.extend(page.items);
        next = page.next;
        // An empty filtered page would not be enough to stop.
        if next.is_none() {
            return Ok(lines);
        }
    }
    Err(std::io::Error::other("page budget exhausted with a continuation").into())
}

pub async fn read_selected(client: &Client) -> Result<Vec<Option<Order>>, Box<dyn Error>> {
    let orders = Table::<Order>::new("orders")?;
    let keys = vec![
        OrderKey::new("ord-amber".into(), 1)?,
        OrderKey::new("ord-amber".into(), 4)?,
        OrderKey::new("ord-amber".into(), 7)?,
    ];
    let mut batch = orders.batch_get(keys, ReadConsistency::Strong, BatchBudget::new(3)?)?;
    batch.set_capacity(Capacity::Total)?;
    while let Some(request) = batch.next_request()? {
        let raw = request.apply(client.batch_get_item()).send().await;
        // accept takes the round Result so failures include accumulated progress.
        batch.accept(request.decode(raw))?;
    }
    // Exhaustion returns successes plus Pending keys in a typed error.
    // A processed key with no returned item is None in the original input order.
    Ok(batch.finish()?.records)
}
