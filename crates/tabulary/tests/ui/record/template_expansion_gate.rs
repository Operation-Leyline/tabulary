#[derive(tabulary::Record)]
#[tabulary(rename_all = "snake_case", partition(name = "pk", template = "{{{id:hex}}}"))]
struct Order { id: String }

#[derive(tabulary::Record)]
#[tabulary(rename_all = "snake_case", sort(name = "sk", template = "{count:dec20}"))]
struct Invoice {
    #[tabulary(partition_key)]
    id: String,
    count: u64,
}

fn main() {}
