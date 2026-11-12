#[derive(tabulary::Record)]
#[tabulary(rename_all = "snake_case", partition(name = "pk", template = "{missing:hex}"))]
struct Order { id: String }

#[derive(tabulary::Record)]
#[tabulary(rename_all = "snake_case", partition(name = "pk", template = "{id:hex}!{id:text}"))]
struct Invoice { id: String }

fn main() {}
