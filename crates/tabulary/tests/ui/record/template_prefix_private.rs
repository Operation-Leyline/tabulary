mod model {
    #[derive(tabulary::Record)]
    #[tabulary(rename_all = "snake_case", partition(name = "pk", template = "{label:hex}"), sort(name = "sk", template = "{number:dec20}!{label:text}"))]
    pub struct Order { pub label: String, pub number: u64 }
}
fn main() {
    let _ = model::OrderSortPrefix { value: "123".into(), marker: std::marker::PhantomData };
    let _ = model::OrderPartition { value: "123".into(), marker: std::marker::PhantomData };
}
