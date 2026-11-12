#[derive(tabulary::Record)]
#[tabulary(rename_all = "snake_case", partition(name = "pk", template = "{shop:hex}{id:hex}"))]
struct Order { shop: String, id: String }

#[derive(tabulary::Record)]
#[tabulary(rename_all = "snake_case", partition(name = "pk", template = "{shop:hex}a{id:hex}"))]
struct Invoice { shop: String, id: String }

#[derive(tabulary::Record)]
#[tabulary(rename_all = "snake_case", sort(name = "sk", template = "{count:dec}0{id:hex}"))]
struct Parcel { count: u64, id: String }

fn main() {}
