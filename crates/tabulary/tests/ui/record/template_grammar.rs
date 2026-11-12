#[derive(tabulary::Record)]
#[tabulary(rename_all = "snake_case", partition(name = "pk", template = "{id}"))]
struct Order { id: String }

#[derive(tabulary::Record)]
#[tabulary(rename_all = "snake_case", partition(name = "pk", template = "{id:base64}"))]
struct Invoice { id: String }

#[derive(tabulary::Record)]
#[tabulary(rename_all = "snake_case", sort(name = "sk", template = "bad}"))]
struct Parcel { id: String }

fn main() {}
