#[derive(tabulary::Document)]
#[tabulary(rename_all = "snake_case")]
struct Key { #[tabulary(partition_key)] id: String }
#[derive(tabulary::Document)]
#[tabulary(rename_all = "snake_case")]
struct Tuple(String);
#[derive(tabulary::Document)]
#[tabulary(rename_all = "snake_case")]
enum Shape { Unit }
#[derive(tabulary::Document)]
#[tabulary(rename_all = "snake_case")]
struct Duplicate { id: String, #[tabulary(rename = "id")] other: String }
#[derive(tabulary::Document)]
#[tabulary(rename_all = "snake_case", partition(name = "pk", template = "{id:hex}"))]
struct Composite { id: String }
fn main() {}
