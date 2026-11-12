use tabulary::TtlSeconds;

#[derive(tabulary::Record)]
#[tabulary(rename_all = "snake_case")]
struct Multiple {
    #[tabulary(partition_key)]
    id: String,
    #[tabulary(ttl)]
    first: TtlSeconds,
    #[tabulary(ttl)]
    second: Option<TtlSeconds>,
}

#[derive(tabulary::Record)]
#[tabulary(rename_all = "snake_case")]
struct Duplicate {
    #[tabulary(partition_key)]
    id: String,
    #[tabulary(ttl, ttl)]
    expires: TtlSeconds,
}

#[derive(tabulary::Document)]
#[tabulary(rename_all = "snake_case")]
struct Nested {
    #[tabulary(ttl)]
    expires: TtlSeconds,
}

#[derive(tabulary::Record)]
#[tabulary(rename_all = "snake_case")]
struct Flattened {
    #[tabulary(partition_key)]
    id: String,
    #[tabulary(flatten, ttl)]
    expires: TtlSeconds,
}

fn main() {}
