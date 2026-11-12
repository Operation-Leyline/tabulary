#[derive(tabulary::Record)]
#[tabulary(rename_all = "snake_case")]
struct Wrong {
    #[tabulary(partition_key)]
    id: String,
    #[tabulary(ttl)]
    expires: u64,
}

fn main() { let _ = <Wrong as tabulary::Record>::SCHEMA; }
