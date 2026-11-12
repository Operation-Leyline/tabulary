#[derive(tabulary::Variants)]
#[tabulary(tag = "kind", rename_all = "snake_case")]
enum Tuple {
    A(String),
}

#[derive(tabulary::Variants)]
#[tabulary(tag = "kind", rename_all = "snake_case")]
enum Duplicate {
    A,
    #[tabulary(rename = "a")]
    B,
}

#[derive(tabulary::Variants)]
#[tabulary(tag = "kind", rename_all = "snake_case")]
enum Overlap {
    A { kind: String },
}

#[derive(tabulary::Variants)]
#[tabulary(tag = "kind", rename_all = "snake_case")]
enum Key {
    A {
        #[tabulary(partition_key)]
        value: String,
    },
}
fn main() {}
