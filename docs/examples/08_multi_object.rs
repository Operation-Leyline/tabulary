use aws_sdk_dynamodb::Client;
use std::{error::Error, num::NonZeroU32};
use tabulary::{
    Cursor, MultiObject, MultiPartition, MultiTable, ReadConsistency, Record, transaction,
};

// Three record types share one orders table. The partition template is the
// same for all three; the sort key literal selects the member.
#[derive(Debug, Record)]
#[tabulary(
    rename_all = "snake_case",
    partition(name = "pk", template = "ORD#{order_ref:text}"),
    sort(name = "sk", template = "META")
)]
pub struct OrderHeader {
    pub order_ref: String,
    pub customer_ref: String,
}

#[derive(Debug, Record)]
#[tabulary(
    rename_all = "snake_case",
    partition(name = "pk", template = "ORD#{order_ref:text}"),
    sort(name = "sk", template = "LINE#{line_no:dec20}")
)]
pub struct OrderLine {
    pub order_ref: String,
    pub line_no: u64,
    pub quantity: u64,
}

#[derive(Debug, Record)]
#[tabulary(
    rename_all = "snake_case",
    partition(name = "pk", template = "ORD#{order_ref:text}"),
    sort(name = "sk", template = "AUDIT#{seq:dec20}")
)]
pub struct OrderAudit {
    pub order_ref: String,
    pub seq: u64,
    pub note: String,
}

/// The closed set of items the orders table may hold. Overlapping
/// discriminators, or a record whose key template cannot produce its declared
/// prefix, fail to compile.
#[derive(Debug, MultiObject)]
#[tabulary(partition = "pk", sort = "sk")]
pub enum OrderTable {
    #[tabulary(partition(prefix = "ORD#"), sort(exact = "META"))]
    Header(OrderHeader),
    #[tabulary(partition(prefix = "ORD#"), sort(prefix = "LINE#"))]
    Line(OrderLine),
    #[tabulary(partition(prefix = "ORD#"), sort(prefix = "AUDIT#"))]
    Audit(OrderAudit),
}

/// Write a header, its lines, and an audit entry in one transaction.
pub async fn place_order(client: &Client) -> Result<usize, Box<dyn Error>> {
    let orders = MultiTable::<OrderTable>::new("orders")?;
    let header = OrderHeader {
        order_ref: "ord-teal".into(),
        customer_ref: "cust-sand".into(),
    };
    let lines = [1, 2].map(|line_no| OrderLine {
        order_ref: "ord-teal".into(),
        line_no,
        quantity: line_no * 2,
    });
    let audit = OrderAudit {
        order_ref: "ord-teal".into(),
        seq: 1,
        note: "placed".into(),
    };
    let request = transaction((
        orders.put(&header).expect_new().into_tx()?,
        lines
            .iter()
            .map(|line| orders.put(line).expect_new().into_tx())
            .collect::<Result<Vec<_>, _>>()?,
        orders.put(&audit).expect_new().into_tx()?,
    ))
    .build()?;
    let actions = request.input().transact_items().len();
    request.decode(request.apply(client.transact_write_items()).send().await)?;
    Ok(actions)
}

/// Read every item of one order, in sort-key order, one bounded page at a time.
pub async fn read_order(client: &Client) -> Result<Vec<OrderTable>, Box<dyn Error>> {
    let orders = MultiTable::<OrderTable>::new("orders")?;
    let mut items = Vec::new();
    let mut next: Option<Cursor<OrderTable, MultiPartition>> = None;
    for _ in 0..4 {
        let mut query = orders
            .query(
                OrderHeader::primary().partition("ord-teal".into()),
                ReadConsistency::Strong,
            )
            .limit(NonZeroU32::new(2).unwrap());
        if let Some(cursor) = next.take() {
            query = query.resume(cursor);
        }
        let request = query.build()?;
        let page = request.decode(request.apply(client.query()).send().await)?;
        items.extend(page.items);
        match page.next {
            Some(cursor) => next = Some(cursor),
            None => return Ok(items),
        }
    }
    Err("order has more pages than this example reads".into())
}

/// Read only the lines; the sort key is narrowed to `LINE#` automatically.
pub async fn order_lines(client: &Client) -> Result<Vec<OrderLine>, Box<dyn Error>> {
    let orders = MultiTable::<OrderTable>::new("orders")?;
    let request = orders
        .query_entity(
            OrderLine::primary().partition("ord-teal".into()),
            ReadConsistency::Eventual,
        )
        .build()?;
    let page = request.decode(request.apply(client.query()).send().await)?;
    Ok(page.items)
}

// A partition-branched table: the partition prefix alone selects the member.
#[derive(Debug, Record)]
#[tabulary(
    rename_all = "snake_case",
    partition(name = "pk", template = "CUST#{customer_ref:text}")
)]
pub struct Customer {
    pub customer_ref: String,
    pub display_name: String,
}

#[derive(Debug, Record)]
#[tabulary(
    rename_all = "snake_case",
    partition(name = "pk", template = "INV#{invoice_ref:text}")
)]
pub struct Invoice {
    pub invoice_ref: String,
    pub amount_due: u64,
}

#[derive(Debug, MultiObject)]
#[tabulary(partition = "pk")]
pub enum AccountTable {
    #[tabulary(partition(prefix = "CUST#"))]
    Customer(Customer),
    #[tabulary(partition(prefix = "INV#"))]
    Invoice(Invoice),
}

/// A typed read of one member. `accounts.get::<OrderLine, _>(..)` does not compile.
pub async fn load_customer(client: &Client) -> Result<Option<Customer>, Box<dyn Error>> {
    let accounts = MultiTable::<AccountTable>::new("accounts")?;
    let request = accounts
        .get(
            CustomerKey::new("cust-sand".into())?,
            ReadConsistency::Strong,
        )
        .build()?;
    Ok(request
        .decode(request.apply(client.get_item()).send().await)?
        .item)
}
