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
    OrderAttrs::SHIPPING.field(ShippingAttrs::DELIVERED).lt(true);
    OrderAttrs::SHIPPING.field(ShippingAttrs::ATTEMPTS).begins_with(1);
    OrderAttrs::SHIPPING.field(ShippingAttrs::ATTEMPTS).size();
    OrderAttrs::SHIPPING.field(ShippingAttrs::LABELS).contains("1".to_owned());
    OrderAttrs::SHIPPING.field(ShippingAttrs::ATTEMPTS).eq("1".to_owned());
    OrderAttrs::SHIPPING.field(ShippingAttrs::LABELS).at(0).add(1);
    OrderAttrs::SHIPPING
        .field(ShippingAttrs::TAGS)
        .delete_members(tabulary::StringSet::try_from(vec!["a".to_owned()]).unwrap());
    OrderAttrs::SHIPPING.field(ShippingAttrs::NOTE).set(None);
}
