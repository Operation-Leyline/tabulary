#[derive(tabulary::Variants)]
#[tabulary(tag = "kind", rename_all = "snake_case")]
enum Types {
    A { value: String },
    B { value: u64 },
}

#[derive(tabulary::Variants)]
#[tabulary(tag = "kind", rename_all = "snake_case")]
enum Presence {
    A { value: String },
    B { value: Option<String> },
}

#[derive(tabulary::Variants)]
#[tabulary(tag = "kind", rename_all = "snake_case")]
enum Codecs {
    A {
        value: u128,
    },
    B {
        #[tabulary(with = tabulary::convert::u128_string)]
        value: u128,
    },
}
fn main() {}
