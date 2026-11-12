#[derive(tabulary::Record)]
#[tabulary(rename_all = "snake_case", partition(name = "pk", template = "{label:hex}"), sort(name = "sk", template = "{label:text}{number:dec20}!{token:hex}"))]
struct Order { label: String, number: u64, token: Vec<u8> }
fn main() {
    Order::primary().sort_prefix(("a".to_owned(),));
}
