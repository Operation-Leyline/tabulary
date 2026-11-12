#[derive(tabulary::Record)]
#[tabulary(rename_all = "snake_case")]
struct Order {
    #[tabulary(partition_key, rename = "orderRef")]
    id: String,
    #[tabulary(rename = "orderRef")]
    reference: String,
}

#[derive(tabulary::Record)]
#[tabulary(rename_all = "camelCase")]
struct Invoice {
    #[tabulary(partition_key)]
    order_ref: String,
    #[tabulary(rename = "orderRef")]
    reference: String,
}

fn main() {}
