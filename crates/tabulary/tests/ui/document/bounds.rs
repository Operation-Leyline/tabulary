use tabulary::Document;
#[derive(tabulary::Document)]
#[tabulary(rename_all = "snake_case")]
struct Envelope<T> { payload: T }
fn main() {
    let value = Envelope { payload: 1u8 };
    let _ = value.encode_map(&mut tabulary::EncodeContext::new("Envelope"));
}
