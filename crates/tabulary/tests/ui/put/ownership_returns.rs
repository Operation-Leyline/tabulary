use tabulary::{Record, ReturnItem, Table};

#[derive(Record)]
#[tabulary(rename_all = "snake_case")]
struct Order {
    #[tabulary(partition_key)]
    id: String,
}

#[derive(Record)]
#[tabulary(rename_all = "snake_case")]
struct Invoice {
    #[tabulary(partition_key)]
    id: String,
}

fn main() {
    let table = Table::<Order>::new("orders").unwrap();
    let order = Order { id: "o".into() };
    let invoice = Invoice { id: "i".into() };
    let _ = table.put(&invoice);
    let _ = table.put(&order).condition(InvoiceAttrs::ID.not_exists());
    let _ = table.put(&order).return_item(ReturnItem::AllNew);
}
