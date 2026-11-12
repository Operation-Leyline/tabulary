#[derive(tabulary::Record)]
#[tabulary(rename_all = "snake_case")]
struct Order {
    #[tabulary(partition_key)]
    id: String,
    #[tabulary(sort_key)]
    number: u64,
    label: String,
}
fn main() {
    Order::primary().build();
    Order::primary().sort_eq(1);
    Order::primary().partition("o".into()).sort_eq(1).sort_lt(2);
    Order::primary().partition("o".into()).or(OrderAttrs::LABEL.eq("x".into()));
    Order::primary().partition("o".into()).and(OrderAttrs::LABEL.eq("x".into()));
    Order::primary().partition(OrderAttrs::LABEL);
    Order::primary().partition("o".into()).sort_eq(OrderAttrs::LABEL);
}
