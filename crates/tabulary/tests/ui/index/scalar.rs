#[derive(tabulary::Record)]
#[tabulary(rename_all = "snake_case", index(ident = ByFlag, name = "by_flag", kind = global, partition = flag, project = all))]
struct NonScalar {
    #[tabulary(partition_key)] id: String,
    flag: bool,
}
#[derive(tabulary::Record)]
#[tabulary(rename_all = "snake_case", index(ident = ByItem, name = "by_item", kind = global, partition = id, sort = items, project = all))]
struct NonScalarSort {
    #[tabulary(partition_key)] id: String,
    items: tabulary::List<String>,
}
fn main() {}
