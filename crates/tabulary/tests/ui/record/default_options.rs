#[derive(tabulary::Record)]
#[tabulary(rename_all = "snake_case")]
struct Order {
    #[tabulary(partition_key)]
    id: String,
    #[tabulary(default)]
    quantity: u64,
}

#[derive(tabulary::Record)]
#[tabulary(rename_all = "snake_case")]
struct Invoice {
    #[tabulary(partition_key)]
    id: String,
    #[tabulary(default = initial_quantity, default = initial_quantity)]
    quantity: u64,
}

fn initial_quantity() -> u64 {
    1
}
fn main() {}
