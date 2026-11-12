#[derive(tabulary::Record)]
#[tabulary(rename_all = "snake_case", sort(name = "sk", template = "{count:dec}"))]
struct Decimal {
    #[tabulary(partition_key)]
    id: String,
    count: u64,
}
#[derive(tabulary::Record)]
#[tabulary(rename_all = "snake_case", sort(name = "sk", template = "{signed:dec}"))]
struct Signed {
    #[tabulary(partition_key)]
    id: String,
    signed: i64,
}
#[derive(tabulary::Record)]
#[tabulary(rename_all = "snake_case", sort(name = "sk", template = "{label:text}!{count:dec20}"))]
struct Variable {
    #[tabulary(partition_key)]
    id: String,
    label: String,
    count: u64,
}
#[derive(tabulary::Record)]
#[tabulary(rename_all = "snake_case",
    index(ident = ByLabel, name = "by_label", kind = global, partition = id,
        sort(name = "gsk", template = "{label:text}!end"), project = all))]
struct Suffix {
    #[tabulary(partition_key)]
    id: String,
    label: String,
}
#[derive(tabulary::Record)]
#[tabulary(rename_all = "snake_case", sort(name = "sk", template = "{bytes:hex}!{count:dec20}"),
    index(ident = ByBytes, name = "by_bytes", kind = global, partition = id,
        sort(name = "gsk", template = "{bytes:hex}"), project = all))]
struct VariableHex {
    #[tabulary(partition_key)]
    id: String,
    bytes: Vec<u8>,
    count: u64,
}
fn main() {
    Decimal::primary().partition("o".into()).sort_ge((9,));
    Signed::primary().partition("o".into()).sort_gt((-1,));
    Variable::primary().partition("o".into()).sort_lt(("a".into(), 9));
    Variable::primary().partition("o".into()).sort_le(("a".into(), 9));
    SuffixIndexes::BY_LABEL.partition("o".into()).sort_between(("a".into(),), ("z".into(),));
    VariableHex::primary().partition("o".into()).sort_ge((vec![1], 9));
    VariableHexIndexes::BY_BYTES.partition("o".into()).sort_ge((vec![1],));
}
