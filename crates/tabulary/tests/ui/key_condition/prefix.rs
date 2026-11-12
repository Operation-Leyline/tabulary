#[derive(tabulary::Record)]
#[tabulary(rename_all = "snake_case", sort(name = "sk", template = "{number:dec20}!{label:hex}"),
    index(ident = ByLabel, name = "by_label", kind = global, partition = label,
        sort(name = "gsk", template = "{number:dec20}!{id:hex}"), project = all))]
struct Order {
    #[tabulary(partition_key)]
    id: String,
    number: u64,
    label: String,
}
fn main() {
    Order::primary().partition("o".into()).sort_begins_with("000".to_owned());
    Order::primary().partition("o".into()).sort_begins_with((7,));
    Order::primary().sort_prefix(());
    Order::primary().sort_prefix((7, "x".to_owned()));
    let prefix = OrderIndexes::BY_LABEL.sort_prefix((7,)).unwrap();
    Order::primary().partition("o".into()).sort_begins_with(prefix);
}
