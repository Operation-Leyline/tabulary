#[derive(tabulary::Record)]
#[tabulary(rename_all = "snake_case")]
struct Order<'a> { #[tabulary(partition_key)] id: String, note: &'a str }
#[derive(tabulary::Document)]
#[tabulary(rename_all = "snake_case")]
struct Invoice { note: Option<&'static str> }
fn main() {}
