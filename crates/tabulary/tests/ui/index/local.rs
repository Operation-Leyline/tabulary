#[derive(tabulary::Record)]
#[tabulary(rename_all = "snake_case", index(ident = ByLine, name = "by_line", kind = local, partition = other, sort = line, project = all))]
struct WrongPartition {
    #[tabulary(partition_key)] id: String,
    #[tabulary(sort_key)] time: u64,
    other: String,
    line: u64,
}
#[derive(tabulary::Record)]
#[tabulary(rename_all = "snake_case", index(ident = ByLine, name = "by_line", kind = local, partition = id, sort = time, project = all))]
struct SameSort {
    #[tabulary(partition_key)] id: String,
    #[tabulary(sort_key)] time: u64,
}
#[derive(tabulary::Record)]
#[tabulary(rename_all = "snake_case", index(ident = ByLine, name = "by_line", kind = local, partition = id, sort = line, project = all))]
struct HashTable {
    #[tabulary(partition_key)] id: String,
    line: u64,
}
#[derive(tabulary::Record)]
#[tabulary(rename_all = "snake_case", index(ident = ByLine, name = "by_line", kind = local, partition = id, project = all))]
struct MissingSort {
    #[tabulary(partition_key)] id: String,
    #[tabulary(sort_key)] time: u64,
}
fn main() {}
