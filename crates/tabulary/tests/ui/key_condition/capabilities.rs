#[derive(tabulary::Record)]
#[tabulary(rename_all = "snake_case",
    index(ident = ByHash, name = "by_hash", kind = global, partition = id, project = all))]
struct Order {
    #[tabulary(partition_key)]
    id: String,
    #[tabulary(sort_key)]
    number: u64,
}
fn equality_only<R: tabulary::Record, I: tabulary::KeySchema<R>>(
    value: tabulary::KeyCondition<R, I, tabulary::SortKey<u128, tabulary::convert::u128_string::Codec>>,
) {
    value.sort_begins_with(1);
}
fn main() {
    Order::primary().partition("o".into()).sort_begins_with(1);
    OrderIndexes::BY_HASH.partition("o".into()).sort_eq(1);
    let _: tabulary::KeyCondition<Order, OrderByHash> = Order::primary().partition("o".into());
    let _: tabulary::BuiltKeyCondition<Order, OrderByHash> = Order::primary().partition("o".into()).build().unwrap();
}
