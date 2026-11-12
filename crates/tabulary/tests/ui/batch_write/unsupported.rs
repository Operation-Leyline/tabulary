use tabulary::{BatchAction, OldItemMode, ReturnOld, Table, Update};

#[derive(tabulary::Record)]
#[tabulary(rename_all = "snake_case")]
struct Order {
    #[tabulary(partition_key)]
    id: String,
    quantity: u64,
}

fn main() {
    let record = Order {
        id: "order".into(),
        quantity: 1,
    };
    let action = BatchAction::put(&record).unwrap();
    action.condition(OrderAttrs::QUANTITY.eq(1));
    BatchAction::put(&record)
        .unwrap()
        .return_item(ReturnOld::AllOld);
    BatchAction::put(&record)
        .unwrap()
        .on_condition_failure(OldItemMode::All);
    BatchAction::put(&record).unwrap().unchecked_version();
    BatchAction::<Order>::update(Update::new().and(OrderAttrs::QUANTITY.set(2)));
    let table = Table::<Order>::new("orders").unwrap();
    BatchAction::<Order>::put(
        &table
            .put(&record)
            .condition(OrderAttrs::QUANTITY.eq(1))
            .build()
            .unwrap(),
    );
}
