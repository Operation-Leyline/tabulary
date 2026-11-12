#[derive(tabulary::Record)]
#[tabulary(rename_all = "snake_case")]
struct Order {
    #[tabulary(partition_key)]
    id: String,
    quantity: u64,
}

fn main() {
    let _ = Order { id: "order-amber".into() };
    let _ = OrderKey::new();
}
