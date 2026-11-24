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
    // A child handle must belong to the parent's document type.
    OrderAttrs::SHIPPING.field(GiftAttrs::MESSAGE);
    // Paths keep their root record through conditions, updates, and comparisons.
    let _: tabulary::Condition<Order> =
        InvoiceAttrs::SHIPPING.field(ShippingAttrs::CARRIER).exists();
    tabulary::Update::<Order>::new()
        .and(InvoiceAttrs::SHIPPING.field(ShippingAttrs::ATTEMPTS).add(1));
    OrderAttrs::SHIPPING
        .field(ShippingAttrs::CARRIER)
        .eq_path(InvoiceAttrs::SHIPPING.field(ShippingAttrs::CARRIER));
    OrderAttrs::SHIPPING
        .field(ShippingAttrs::CARRIER)
        .eq_path(OrderAttrs::SHIPPING.field(ShippingAttrs::ATTEMPTS));
}
