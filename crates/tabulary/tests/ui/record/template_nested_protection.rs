#[derive(tabulary::Record)]
#[tabulary(rename_all = "snake_case", partition(name = "pk", template = "{id:hex}"))]
struct Order { id: String }
#[derive(tabulary::Record)]
#[tabulary(rename_all = "snake_case")]
struct Envelope {
    #[tabulary(partition_key)]
    id: String,
    order: Order,
}
fn main() {
    EnvelopeAttrs::ORDER.field(OrderAttrs::ID).set(String::new());
    EnvelopeAttrs::ORDER.field(OrderAttrs::PARTITION).set(String::new());
}
