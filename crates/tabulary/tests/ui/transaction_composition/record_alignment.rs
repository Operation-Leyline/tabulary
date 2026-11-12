use aws_sdk_dynamodb::operation::transact_write_items::TransactWriteItemsOutput;
use tabulary::{Failure, PreparedTransaction, TxAction, TxError, transaction};

struct Order;
struct Invoice;

fn heterogeneous_vector(order: TxAction<Order>, invoice: TxAction<Invoice>) {
    let _ = transaction((vec![order, invoice],));
}

fn misaligned_reasons(request: PreparedTransaction<(Vec<TxAction<Order>>, (TxAction<Invoice>,))>) {
    if let Err(TxError::Cancelled(cancelled)) =
        request.decode(Ok(TransactWriteItemsOutput::builder().build()))
    {
        let _: (Vec<Failure<Invoice>>, (Failure<Order>,)) = cancelled.reasons;
    }
}

fn main() {}
