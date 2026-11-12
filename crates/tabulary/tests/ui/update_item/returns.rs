use aws_sdk_dynamodb::types::ReturnValue;
use tabulary::{Document, Record, RecordKey, ReturnItem, Table};

fn partial<R: Record + Document, K: RecordKey<R>>(table: Table<R>, key: K) {
    table.update(key).return_item(ReturnValue::UpdatedNew);
}
fn partial_old<R: Record + Document, K: RecordKey<R>>(table: Table<R>, key: K) {
    table.update(key).return_item(ReturnValue::UpdatedOld);
}
fn absent_choices() {
    let _ = ReturnItem::UpdatedNew;
    let _ = ReturnItem::UpdatedOld;
}
fn main() {}
