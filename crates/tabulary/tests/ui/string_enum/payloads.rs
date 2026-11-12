#[derive(tabulary::StringEnum)]
#[tabulary(rename_all = "snake_case")]
enum Tuple {
    Ready(String),
}

#[derive(tabulary::StringEnum)]
#[tabulary(rename_all = "snake_case")]
enum Named {
    Ready { count: u64 },
}

#[derive(tabulary::StringEnum)]
#[tabulary(rename_all = "snake_case")]
enum EmptyTuple {
    Ready(),
}

#[derive(tabulary::StringEnum)]
#[tabulary(rename_all = "snake_case")]
enum EmptyNamed {
    Ready {},
}

fn main() {}
