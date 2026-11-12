#[derive(tabulary::Record)]
#[tabulary(rename_all = "snake_case", index(ident = ByValue, name = "by_value", kind = global, partition(name = "gpk", template = "{value:hex}"), project = all))]
struct Order {
    #[tabulary(partition_key)] id: String,
    value: String,
}
fn main() {
    let _ = OrderAttrs::VALUE.set("other".into());
    let _ = OrderAttrs::BY_VALUE_PARTITION.set("raw".into());
}
