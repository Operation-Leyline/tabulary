#[derive(tabulary::Record)]
#[tabulary(rename_all = "snake_case")]
struct Order {
    #[tabulary(partition_key, default = String::new)]
    id: String,
}

#[derive(tabulary::Record)]
#[tabulary(rename_all = "snake_case")]
struct Invoice {
    #[tabulary(partition_key)]
    id: String,
    #[tabulary(sort_key, default = initial_line)]
    line: u64,
}

fn initial_line() -> u64 {
    1
}
fn main() {}
