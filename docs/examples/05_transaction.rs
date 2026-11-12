use aws_sdk_dynamodb::Client;
use std::error::Error;
use tabulary::{
    Capacity, Failure, FailureKind, OldItem, OldItemMode, Record, Table, TxError, Update,
    transaction,
};

#[derive(Record)]
#[tabulary(rename_all = "snake_case")]
pub struct Invoice {
    #[tabulary(partition_key)]
    invoice_ref: String,
    amount_due: u64,
    #[tabulary(version)]
    edit_no: u64,
}

#[derive(Record)]
#[tabulary(rename_all = "snake_case")]
pub struct Stock {
    #[tabulary(partition_key)]
    product_ref: String,
    on_hand: i64,
}

#[derive(Record)]
#[tabulary(rename_all = "snake_case")]
pub struct Session {
    #[tabulary(partition_key)]
    session_ref: String,
    usable: bool,
}

pub enum CheckoutAttempt {
    Committed,
    Cancelled(Box<CheckoutCancellation>),
}

pub struct CheckoutCancellation {
    pub invoice: Failure<Invoice>,
    pub stock: Failure<Stock>,
    pub session: Failure<Session>,
}

pub async fn checkout(
    client: &Client,
    idempotency_token: String,
) -> Result<CheckoutAttempt, Box<dyn Error>> {
    let invoices = Table::<Invoice>::new("invoices")?;
    let inventory = Table::<Stock>::new("inventory")?;
    let sessions = Table::<Session>::new("sessions")?;
    let invoice = Invoice {
        invoice_ref: "inv-coral".into(),
        amount_due: 12,
        edit_no: 1,
    };

    let request = transaction((
        invoices
            .put(&invoice)
            .expect_new()
            .condition(InvoiceAttrs::INVOICE_REF.not_exists())
            .on_condition_failure(OldItemMode::All)
            .into_tx()?,
        inventory
            .update(StockKey::new("product-violet".into())?)
            .changes(Update::new().and(StockAttrs::ON_HAND.increment(-1)))
            .condition(StockAttrs::ON_HAND.ge(1))
            .on_condition_failure(OldItemMode::All)
            .into_tx()?,
        sessions
            .check(
                SessionKey::new("session-celadon".into())?,
                SessionAttrs::USABLE.eq(true),
            )
            .on_condition_failure(OldItemMode::All)
            .into_tx()?,
    ))
    .client_request_token(idempotency_token)
    .capacity(Capacity::Total)
    .build()?;
    assert_eq!(request.input().transact_items().len(), 3);
    let raw = request.apply(client.transact_write_items()).send().await;

    match request.decode(raw) {
        Ok(_) => Ok(CheckoutAttempt::Committed),
        Err(TxError::Cancelled(cancelled)) => {
            // Every tuple position has its original record type.
            let (invoice, stock, session) = cancelled.reasons;
            if matches!(stock.kind, FailureKind::ConditionFailed)
                && let OldItem::Decoded(old_stock) = &stock.old
            {
                let _available = old_stock.on_hand;
            }
            // Unavailable reasons remain explicit if the endpoint omitted them.
            Ok(CheckoutAttempt::Cancelled(Box::new(CheckoutCancellation {
                invoice,
                stock,
                session,
            })))
        }
        Err(other) => Err(Box::new(other)),
    }
}
