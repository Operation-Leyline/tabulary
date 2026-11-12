use tabulary::{ValidationFault, ValidationPath};

#[derive(tabulary::Document)]
#[tabulary(rename_all = "snake_case")]
struct Line { count: u64 }
#[derive(tabulary::Document)]
#[tabulary(rename_all = "snake_case")]
struct Parcel { line: Line }

#[derive(tabulary::Record)]
#[tabulary(rename_all = "snake_case")]
struct Invoice { #[tabulary(partition_key)] id: String }
#[derive(tabulary::Record)]
#[tabulary(rename_all = "snake_case")]
struct Order { #[tabulary(partition_key)] id: String }
fn wrong_owner() -> ValidationFault<Order> { ValidationFault::<Order>::new(InvoiceAttrs::ID, "invalid") }
fn wrong_additional_owner() -> ValidationFault<Order> { ValidationFault::new(OrderAttrs::ID, "invalid").with_path(InvoiceAttrs::ID) }

fn main() {
    let _ = ValidationFault::new("count", "invalid");
    let _ = ValidationPath::from(ParcelAttrs::LINE).field(ParcelAttrs::LINE);
}
