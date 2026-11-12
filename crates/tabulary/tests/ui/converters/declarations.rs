#[derive(tabulary::Record)]
#[tabulary(rename_all = "snake_case")]
struct Version {
    #[tabulary(partition_key)] id: String,
    #[tabulary(version, with = tabulary::convert::u128_string)] revision: u64,
}
#[derive(tabulary::Record)]
#[tabulary(rename_all = "snake_case")]
struct Ttl {
    #[tabulary(partition_key)] id: String,
    #[tabulary(ttl, with = tabulary::convert::u128_string)] expires: tabulary::TtlSeconds,
}
#[derive(tabulary::Document)]
#[tabulary(rename_all = "snake_case")]
struct FlattenDocument {
    #[tabulary(flatten, with = tabulary::convert::u128_string)] state: bool,
}
#[derive(tabulary::Record)]
#[tabulary(rename_all = "snake_case")]
struct FlattenRecord {
    #[tabulary(partition_key)] id: String,
    #[tabulary(flatten, with = tabulary::convert::u128_string)] state: bool,
}
#[derive(tabulary::Document)]
#[tabulary(rename_all = "snake_case")]
struct Duplicate {
    #[tabulary(with = tabulary::convert::u128_string, with = tabulary::convert::u128_string)] value: u128,
}
mod boolean { pub type Codec = tabulary::Native<bool>; }
#[derive(tabulary::Record)]
#[tabulary(rename_all = "snake_case")]
struct NonScalarKey {
    #[tabulary(partition_key, with = boolean)] id: bool,
}
fn main() {}
