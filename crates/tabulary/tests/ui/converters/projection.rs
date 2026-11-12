#[derive(tabulary::Document)]
#[tabulary(rename_all = "snake_case")]
struct Address { city: String }
mod custom {
    use super::Address;
    pub struct Codec;
    impl tabulary::Converter<Address> for Codec {
        type Wire = tabulary::wire::Map;
        fn encode(value: &Address, cx: &mut tabulary::EncodeContext) -> Result<tabulary::__private::AttributeValue, tabulary::EncodeError> { tabulary::Value::encode_value(value, cx) }
        fn decode(value: &tabulary::__private::AttributeValue, cx: &mut tabulary::DecodeContext) -> Result<Address, tabulary::DecodeError> { tabulary::Value::decode_value(value, cx) }
    }
}
#[derive(tabulary::Record)]
#[tabulary(rename_all = "snake_case")]
struct Order {
    #[tabulary(partition_key)] id: String,
    #[tabulary(with = custom)] address: Address,
}
#[derive(tabulary::Projection)]
#[tabulary(record = Address)]
struct City { #[tabulary(attribute = AddressAttrs::CITY)] city: String }
#[derive(tabulary::Projection)]
#[tabulary(record = Order)]
struct Bypass {
    #[tabulary(attribute = OrderAttrs::ADDRESS, nested)] address: City,
}
fn main() {}
