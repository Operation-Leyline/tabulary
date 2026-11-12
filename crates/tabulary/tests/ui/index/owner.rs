#[derive(tabulary::Record)]
#[tabulary(rename_all = "snake_case", index(ident = ByValue, name = "by_value", kind = global, partition = value, project = all))]
struct Order {
    #[tabulary(partition_key)] id: String,
    value: String,
}
#[derive(tabulary::Record)]
#[tabulary(rename_all = "snake_case")]
struct Invoice {
    #[tabulary(partition_key)] id: String,
}
fn invoice_index<I: tabulary::IndexSchema<Invoice>>(_: I) {}
fn main() { invoice_index(OrderIndexes::BY_VALUE); }
