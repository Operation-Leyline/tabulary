#[derive(tabulary::Record)]
#[tabulary(rename_all = "snake_case", unknown = "ignore")]
struct Order {
    #[tabulary(partition_key)]
    id: String,
}

#[derive(tabulary::Record)]
#[tabulary(rename_all = "snake_case", unknown = "allow", unknown = "deny")]
struct Invoice {
    #[tabulary(partition_key)]
    id: String,
}

fn main() {}
