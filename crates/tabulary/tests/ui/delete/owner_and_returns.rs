use tabulary::{ReturnItem, Table};

#[derive(tabulary::Record)]
#[tabulary(rename_all = "snake_case")]
struct Order {
    #[tabulary(partition_key)]
    id: String,
}
#[derive(tabulary::Record)]
#[tabulary(rename_all = "snake_case")]
struct Invoice {
    #[tabulary(partition_key)]
    id: String,
}

fn main() {
    let table = Table::<Order>::new("orders").unwrap();
    let _ = table.delete(InvoiceKey::new("invoice-lime".into()).unwrap());
    let _ = table
        .delete(OrderKey::new("order-lime".into()).unwrap())
        .condition(InvoiceAttrs::ID.exists());
    let _ = table
        .delete(OrderKey::new("order-lime".into()).unwrap())
        .return_item(ReturnItem::AllNew);
}
