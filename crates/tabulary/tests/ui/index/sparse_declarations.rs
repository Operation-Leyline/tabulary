#[derive(tabulary::Record)]
#[tabulary(rename_all = "snake_case", index(ident = ByValue, name = "by_value", kind = global, partition = value, sparse, project = all))]
struct Sparse {
    #[tabulary(partition_key)] id: String,
    value: String,
}
#[derive(tabulary::Record)]
#[tabulary(rename_all = "snake_case", index(ident = ByValue, name = "by_value", kind = global, partition = value, project = all))]
struct OptionalInput {
    #[tabulary(partition_key)] id: String,
    value: Option<String>,
}
fn main() {}

#[derive(tabulary::Record)]
#[tabulary(rename_all = "snake_case", index(ident = ByValue, name = "by_value", kind = global, partition = value, sparse, sparse, project = all))]
struct DuplicateSparse {
    #[tabulary(partition_key)] id: String,
    value: Option<String>,
}

#[derive(tabulary::Record)]
#[tabulary(rename_all = "snake_case", index(ident = ByValue, name = "by_value", kind = global, partition(name = "gpk", template = "{value:hex}"), sparse, project = all))]
struct DefaultedInput {
    #[tabulary(partition_key)] id: String,
    #[tabulary(default = fallback)] value: Option<String>,
}
fn fallback() -> Option<String> { None }
