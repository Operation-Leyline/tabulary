use tabulary::{Document, IndexSchema, KeyCondition, KeySchema, PrimarySchema, ReadConsistency, Record, Table};

fn foreign<R: Record + Document, T: Record, I: PrimarySchema<T>, S>(
    table: Table<R>, key: KeyCondition<T, I, S>,
) {
    table.query(key, ReadConsistency::Strong);
}
fn index<R: Record + Document, I: IndexSchema<R> + KeySchema<R>, J: KeySchema<R>, S>(
    table: Table<R>, index: I, key: KeyCondition<R, J, S>,
) {
    table.index(index).query(key, ReadConsistency::Eventual);
}
fn main() {}
