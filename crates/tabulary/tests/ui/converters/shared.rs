use tabulary::convert::u128_string as alias;
#[derive(tabulary::Variants)]
#[tabulary(tag = "stage", rename_all = "snake_case")]
enum Stage {
    Waiting { #[tabulary(with = tabulary::convert::u128_string)] total: u128 },
    Sent { #[tabulary(with = alias)] total: u128 },
}
fn main() {}
