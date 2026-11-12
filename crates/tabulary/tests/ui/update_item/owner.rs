use tabulary::{Attr, Document, Record, RecordKey, Table, Update};

fn changes<R: Record + Document, S: Record, K: RecordKey<R>>(table: Table<R>, key: K) {
    table.update(key).changes(Update::<S>::new());
}
fn guard<R: Record + Document, S: Record + 'static, K: RecordKey<R>>(
    table: Table<R>,
    key: K,
    attr: Attr<S, u64>,
) {
    table.update(key).condition(attr.eq(1));
}
fn main() {}
