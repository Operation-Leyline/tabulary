#[derive(tabulary::Record)]
#[tabulary(rename_all = "snake_case", index(ident = ByValue, name = "by_value", kind = global, partition = value, sparse, project = all))]
struct Sparse {
    #[tabulary(partition_key)] id: String,
    value: Option<String>,
}
#[derive(tabulary::Record)]
#[tabulary(rename_all = "snake_case", index(ident = ByValue, name = "by_value", kind = global, partition = value, project = all))]
struct OptionalInput {
    #[tabulary(partition_key)] id: String,
    value: Option<String>,
}
#[derive(tabulary::Record)]
#[tabulary(rename_all = "snake_case", index(ident = ByValue, name = "by_value", kind = global, partition(name = "gpk", template = "value!{value:hex}"), project = all))]
struct Composite {
    #[tabulary(partition_key)] id: String,
    value: String,
}
fn main() {}
