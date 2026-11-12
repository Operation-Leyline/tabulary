#[derive(tabulary::Record)]
#[tabulary(rename_all = "snake_case")]
struct Invoice {
    #[tabulary(partition_key)]
    id: String,
    #[tabulary(version)]
    revision: u64,
}
fn main() {
    InvoiceAttrs::REVISION.set(2);
    InvoiceAttrs::REVISION.increment(1);
    let table = tabulary::Table::<Invoice>::new("invoices").unwrap();
    table
        .delete(InvoiceKey::new("i".into()).unwrap())
        .expect_new();
    table
        .update(InvoiceKey::new("i".into()).unwrap())
        .expect_new();
}
