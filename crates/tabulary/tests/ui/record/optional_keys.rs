#[derive(tabulary::Record)]
#[tabulary(rename_all = "snake_case")]
struct Order {
    #[tabulary(partition_key)]
    id: Option<String>,
}

#[derive(tabulary::Record)]
#[tabulary(rename_all = "snake_case")]
struct Invoice {
    #[tabulary(partition_key)]
    id: String,
    #[tabulary(sort_key)]
    line: std::option::Option<u64>,
}

fn main() {}
