#[derive(tabulary::Record)]
#[tabulary(rename_all = "snake_case", partition(name = "pk", template = "{id:dec20}"))]
struct Order { id: String }
fn main() {}
