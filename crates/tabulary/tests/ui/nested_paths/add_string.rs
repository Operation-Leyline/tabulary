#[derive(tabulary::Document)]
#[tabulary(rename_all = "snake_case")]
struct Shipping {
    carrier: String,
    attempts: u64,
    delivered: bool,
    note: Option<String>,
    tags: tabulary::StringSet,
    labels: tabulary::List<u64>,
}
#[derive(tabulary::Document)]
#[tabulary(rename_all = "snake_case")]
struct Gift {
    message: String,
}
#[derive(tabulary::Record)]
#[tabulary(rename_all = "snake_case")]
struct Order {
    #[tabulary(partition_key)]
    order_ref: String,
    shipping: Shipping,
    gift: Option<Gift>,
}
#[derive(tabulary::Record)]
#[tabulary(rename_all = "snake_case")]
struct Invoice {
    #[tabulary(partition_key)]
    invoice_ref: String,
    shipping: Shipping,
}
fn main() {
    // ADD needs a native number or set; a nested string has no add method.
    OrderAttrs::SHIPPING.field(ShippingAttrs::CARRIER).add("express".to_owned());
}
