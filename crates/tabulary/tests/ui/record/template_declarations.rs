#[derive(tabulary::Record)]
#[tabulary(rename_all = "snake_case", partition(template = "{id:hex}"))]
struct Order { id: String }

#[derive(tabulary::Record)]
#[tabulary(rename_all = "snake_case", partition(name = "pk", name = "other", template = "{id:hex}"))]
struct Invoice { id: String }

#[derive(tabulary::Record)]
#[tabulary(rename_all = "camelCase", partition(name = "orderId", template = "{order_id:hex}"))]
struct Purchase { order_id: String }

#[derive(tabulary::Record)]
#[tabulary(rename_all = "snake_case", partition(name = "pk", template = "{id:hex}"))]
struct Parcel {
    #[tabulary(partition_key)]
    id: String,
}

fn main() {}
