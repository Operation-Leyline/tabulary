// Shadow the default crate name, so any leaked ::tabulary reference fails.
extern crate tabulary as renamed;
mod tabulary {}
use renamed::{Document, Value, RecordKey};
#[derive(renamed::Document)]
#[tabulary(crate = "::renamed", rename_all = "snake_case")]
struct Detail<T> { value: T }
#[derive(renamed::Record)]
#[tabulary(crate = "::renamed", rename_all = "snake_case")]
struct Order<T> { #[tabulary(partition_key)] id: String, detail: Detail<T> }
fn main() {
    let order = Order { id: "order-lime".into(), detail: Detail { value: 1u64 } };
    let item = order.to_item().unwrap();
    assert_eq!(Order::<u64>::from_item(&item).unwrap().detail.value, 1);
    assert_eq!(order.key().unwrap().encode_key().unwrap().len(), 1);
    assert_eq!(OrderAttrs::<u64>::DETAIL.name(), "detail");
    let value = order.encode_value(&mut renamed::EncodeContext::new("Envelope")).unwrap();
    Order::<u64>::decode_value(&value, &mut renamed::DecodeContext::new("Envelope")).unwrap();
    let _ = order.detail.encode_map(&mut renamed::EncodeContext::new("Detail"));
}
