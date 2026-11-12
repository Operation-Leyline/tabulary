use tabulary::{ReadConsistency, Table};

#[derive(tabulary::Record)]
#[tabulary(rename_all = "snake_case")]
struct Order {
    #[tabulary(partition_key)]
    id: String,
}

fn main() {
    let table = Table::<Order>::new("orders").unwrap();
    let key = OrderKey::new("order-lime".into()).unwrap();
    let _ = table.get(key);
    let _ = ReadConsistency::default();
}
