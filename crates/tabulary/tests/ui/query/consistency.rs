use tabulary::{Document, KeyCondition, PrimarySchema, Record, Table};

fn missing<R: Record + Document, I: PrimarySchema<R>, S>(
    table: Table<R>, key: KeyCondition<R, I, S>,
) {
    table.query(key);
}
fn main() {}
