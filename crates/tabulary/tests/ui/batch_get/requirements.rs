use tabulary::{BatchBudget, ReadConsistency, Record, Table};

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
#[derive(tabulary::Projection)]
#[tabulary(record = Invoice)]
struct InvoiceId {
    #[tabulary(attribute = InvoiceAttrs::ID)]
    id: String,
}
fn main() {
    let table = Table::<Order>::new("orders").unwrap();
    let budget = BatchBudget::new(1).unwrap();
    let _ = table.batch_get(vec![OrderKey::new("a".into()).unwrap()], budget);
    let _ = table.batch_get(
        vec![OrderKey::new("a".into()).unwrap()],
        ReadConsistency::Strong,
    );
    let _ = table.batch_get(
        vec![InvoiceKey::new("a".into()).unwrap()],
        ReadConsistency::Strong,
        budget,
    );
    let batch = table
        .batch_get(
            vec![OrderKey::new("a".into()).unwrap()],
            ReadConsistency::Strong,
            budget,
        )
        .unwrap();
    let _ = batch.select::<InvoiceId>();
}
