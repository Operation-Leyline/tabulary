#[derive(tabulary::Record)]
#[tabulary(rename_all = "snake_case", partition(name = "pk", template = "{id:hex}"))]
struct Order { id: Option<String> }

#[derive(tabulary::Record)]
#[tabulary(rename_all = "snake_case", sort(name = "sk", template = "{id:hex}"))]
struct Invoice {
    #[tabulary(default = initial_id)]
    id: String,
}

fn initial_id() -> String { String::new() }
fn main() {}
