#[derive(tabulary::Record)]
#[tabulary(rename_all = "snake_case", partition(name = "pk", template = "{id:hex}"), sort(name = "sk", template = "{count:dec20}!{label:text}"))]
struct Order { id: String, count: u64, label: String }

fn main() {
    OrderAttrs::ID.set(String::new());
    OrderAttrs::COUNT.increment(1);
    OrderAttrs::PARTITION.set(String::new());
    Order::primary().sort_prefix(("partial".to_owned(),));
    Order::primary().sort_prefix((1u64, "whole".to_owned()));
}
