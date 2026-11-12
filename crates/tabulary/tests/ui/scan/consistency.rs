use tabulary::Table;

#[derive(tabulary::Record)]
#[tabulary(rename_all = "snake_case", index(ident = ById, name = "by_id", kind = global, partition = id, project = all))]
struct Order {
    #[tabulary(partition_key)]
    id: String,
}

fn main() {
    let table = Table::<Order>::new("orders").unwrap();
    let _ = table.scan();
    let _ = table.index(OrderIndexes::BY_ID).scan();
}
