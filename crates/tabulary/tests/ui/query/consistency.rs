use tabulary::{Document, KeyCondition, PrimarySchema, Record, Table};

fn missing<R: Record + Document, I: PrimarySchema<R>, S>(
    table: Table<R>, key: KeyCondition<R, I, S>,
) {
    table.query(key);
}
fn missing_index<R: Record + Document, I: tabulary::IndexSchema<R> + tabulary::KeySchema<R>, S>(
    table: tabulary::IndexTable<R, I>, key: KeyCondition<R, I, S>,
) {
    table.query(key);
}
fn main() {}
