#[derive(tabulary::Projection)]
struct Missing { value: u64 }
#[derive(tabulary::Projection)]
#[tabulary(record = String)]
struct NoHandle { value: u64 }
#[derive(tabulary::Projection)]
#[tabulary(record = String)]
struct Defaults { #[tabulary(attribute = Missing::VALUE, default = initial)] value: u64 }
#[derive(tabulary::Projection)]
#[tabulary(record = String)]
struct Tuple(u64);
#[derive(tabulary::Projection)]
#[tabulary(record = String)]
struct Borrowed<'a> { #[tabulary(attribute = Missing::VALUE)] value: &'a str }
fn main() {}
