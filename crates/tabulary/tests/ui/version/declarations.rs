#[derive(tabulary::Record)]
#[tabulary(rename_all = "snake_case")]
struct Optional {
    #[tabulary(partition_key)]
    id: String,
    #[tabulary(version)]
    revision: Option<u64>,
}
#[derive(tabulary::Record)]
#[tabulary(rename_all = "snake_case")]
struct WrongType {
    #[tabulary(partition_key)]
    id: String,
    #[tabulary(version)]
    revision: i64,
}
#[derive(tabulary::Record)]
#[tabulary(rename_all = "snake_case")]
struct Defaulted {
    #[tabulary(partition_key)]
    id: String,
    #[tabulary(version, default = one)]
    revision: u64,
}
fn one() -> u64 {
    1
}
#[derive(tabulary::Record)]
#[tabulary(rename_all = "snake_case")]
struct Duplicate {
    #[tabulary(partition_key)]
    id: String,
    #[tabulary(version)]
    revision: u64,
    #[tabulary(version)]
    other: u64,
}
#[derive(tabulary::Record)]
#[tabulary(rename_all = "snake_case")]
struct Key {
    #[tabulary(partition_key, version)]
    id: u64,
}
#[derive(tabulary::Record)]
#[tabulary(
    rename_all = "snake_case",
    partition(name = "pk", template = "inv!{revision:dec}")
)]
struct Source {
    #[tabulary(version)]
    revision: u64,
}
#[derive(tabulary::Document)]
#[tabulary(rename_all = "snake_case")]
struct Nested {
    #[tabulary(version)]
    revision: u64,
}
#[derive(tabulary::Record)]
#[tabulary(rename_all = "snake_case")]
struct Converter {
    #[tabulary(partition_key)]
    id: String,
    #[tabulary(version, with = "tabulary::Native<u64>")]
    revision: u64,
}
fn main() {}
