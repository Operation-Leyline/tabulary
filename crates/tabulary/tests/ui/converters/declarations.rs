#[derive(tabulary::Record)]
#[tabulary(rename_all = "snake_case")]
struct Version {
    #[tabulary(partition_key)] id: String,
    #[tabulary(version, with = tabulary::convert::u128_string)] revision: u64,
}
#[derive(tabulary::Record)]
#[tabulary(rename_all = "snake_case")]
struct Ttl {
    #[tabulary(partition_key)] id: String,
    #[tabulary(ttl, with = tabulary::convert::u128_string)] expires: tabulary::TtlSeconds,
}
#[derive(tabulary::Document)]
#[tabulary(rename_all = "snake_case")]
struct FlattenDocument {
    #[tabulary(flatten, with = tabulary::convert::u128_string)] state: bool,
}
#[derive(tabulary::Record)]
#[tabulary(rename_all = "snake_case")]
struct FlattenRecord {
    #[tabulary(partition_key)] id: String,
    #[tabulary(flatten, with = tabulary::convert::u128_string)] state: bool,
}
#[derive(tabulary::Document)]
#[tabulary(rename_all = "snake_case")]
struct Duplicate {
    #[tabulary(with = tabulary::convert::u128_string, with = tabulary::convert::u128_string)] value: u128,
}
mod boolean {
    pub struct Codec;
    impl tabulary::Converter<String> for Codec {
        type Wire = tabulary::wire::Bool;
        fn encode(_: &String, _: &mut tabulary::EncodeContext) -> Result<tabulary::__private::AttributeValue, tabulary::EncodeError> { Ok(tabulary::__private::AttributeValue::Bool(true)) }
        fn decode(_: &tabulary::__private::AttributeValue, cx: &mut tabulary::DecodeContext) -> Result<String, tabulary::DecodeError> { Err(cx.error(tabulary::DecodeFault::Custom { code: "unsupported" })) }
    }
}
#[derive(tabulary::Record)]
#[tabulary(rename_all = "snake_case")]
struct NonScalarKey {
    #[tabulary(partition_key, with = boolean)] id: String,
}
fn main() {}
