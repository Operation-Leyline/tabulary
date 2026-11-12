#[derive(tabulary::Record)]
#[tabulary(rename_all = "snake_case",
    index(ident = First, name = "by_value", kind = global, partition = value, project = all),
    index(ident = Second, name = "by_value", kind = global, partition = value, project = all))]
struct DuplicateName {
    #[tabulary(partition_key)] id: String,
    value: String,
}
#[derive(tabulary::Record)]
#[tabulary(rename_all = "snake_case",
    index(ident = ByValue, name = "by_value", kind = global, partition = value, project = all),
    index(ident = ByVALUE, name = "by_other", kind = global, partition = value, project = all))]
struct DuplicateHandle {
    #[tabulary(partition_key)] id: String,
    value: String,
}
#[derive(tabulary::Record)]
#[tabulary(rename_all = "snake_case", index(ident = ByValue, name = "by_value", kind = global, partition = value, project = include(absent)))]
struct UnknownInclude {
    #[tabulary(partition_key)] id: String,
    value: String,
}
fn main() {}
