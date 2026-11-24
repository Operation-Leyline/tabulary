use tabulary::Document;
#[derive(tabulary::Document)]
#[tabulary(rename_all = "snake_case")]
struct Envelope<T> { payload: T }
fn main() {
    let value = Envelope { payload: 'x' };
    let _ = value.encode_map(&mut tabulary::EncodeContext::new("Envelope"));
}
