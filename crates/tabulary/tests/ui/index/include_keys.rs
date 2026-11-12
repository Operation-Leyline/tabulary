#[derive(tabulary::Record)]
#[tabulary(rename_all = "snake_case", index(ident = ByValue, name = "by_value", kind = global, partition = value, project = include(id)))]
struct PrimaryKey {
    #[tabulary(partition_key)] id: String,
    value: String,
}
#[derive(tabulary::Record)]
#[tabulary(rename_all = "snake_case", index(ident = ByValue, name = "by_value", kind = global, partition = value, project = include(value)))]
struct IndexKey {
    #[tabulary(partition_key)] id: String,
    value: String,
}
fn main() {}
