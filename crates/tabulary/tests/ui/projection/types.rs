use tabulary::{Attr, Projection};
#[derive(tabulary::Record)]
#[tabulary(rename_all = "snake_case")]
struct Order { #[tabulary(partition_key)] id: String, count: u64, note: Option<String> }
#[derive(tabulary::Record)]
#[tabulary(rename_all = "snake_case")]
struct Invoice { #[tabulary(partition_key)] id: String }
#[derive(Projection)]
#[tabulary(record = Order)]
struct Foreign { #[tabulary(attribute = InvoiceAttrs::ID)] id: String }
#[derive(Projection)]
#[tabulary(record = Order)]
struct WrongType { #[tabulary(attribute = OrderAttrs::COUNT)] count: String }
#[derive(Projection)]
#[tabulary(record = Order)]
struct OptionalRequired { #[tabulary(attribute = OrderAttrs::COUNT)] count: Option<u64> }
#[derive(Projection)]
#[tabulary(record = Order)]
struct RequiredOptional { #[tabulary(attribute = OrderAttrs::NOTE)] note: String }
struct OtherCodec;
impl tabulary::Converter<u64> for OtherCodec {
    type Wire = tabulary::wire::Number;
    fn encode(value: &u64, cx: &mut tabulary::EncodeContext) -> Result<tabulary::__private::AttributeValue, tabulary::EncodeError> { tabulary::Value::encode_value(value, cx) }
    fn decode(value: &tabulary::__private::AttributeValue, cx: &mut tabulary::DecodeContext) -> Result<u64, tabulary::DecodeError> { tabulary::Value::decode_value(value, cx) }
}
const BAD_CODEC: Attr<Order, u64, tabulary::Required, tabulary::Mutable, OtherCodec> = tabulary::__private::field("count", 1);
#[derive(Projection)]
#[tabulary(record = Order)]
struct Codec { #[tabulary(attribute = BAD_CODEC)] count: u64 }
fn main() {}
