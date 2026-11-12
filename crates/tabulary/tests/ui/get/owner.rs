use tabulary::{ReadConsistency, Table};

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
#[derive(tabulary::Projection)]
#[tabulary(record = Invoice)]
struct InvoiceId {
    #[tabulary(attribute = InvoiceAttrs::ID)]
    id: String,
}

fn main() {
    let table = Table::<Order>::new("orders").unwrap();
    let _ = table.get(
        InvoiceKey::new("invoice-lime".into()).unwrap(),
        ReadConsistency::Strong,
    );
    let _ = table
        .get(
            OrderKey::new("order-lime".into()).unwrap(),
            ReadConsistency::Strong,
        )
        .select::<InvoiceId>();
}
