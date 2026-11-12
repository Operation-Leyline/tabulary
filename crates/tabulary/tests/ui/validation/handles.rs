use tabulary::{ValidationFault, ValidationPath};

#[derive(tabulary::Document)]
#[tabulary(rename_all = "snake_case")]
struct Line { count: u64 }
#[derive(tabulary::Document)]
#[tabulary(rename_all = "snake_case")]
struct Parcel { line: Line }

fn main() {
    let _ = ValidationFault::new("count", "invalid");
    let _ = ValidationPath::from(ParcelAttrs::LINE).field(ParcelAttrs::LINE);
}
