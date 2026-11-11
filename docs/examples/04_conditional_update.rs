use aws_sdk_dynamodb::Client;
use std::error::Error;
use tabulary::{
    Capacity, Failure, FailureKind, OldItem, OldItemMode, Record, ReturnItem, Table, Update,
    WriteError,
};

#[derive(Debug, Record)]
#[tabulary(rename_all = "snake_case")]
pub struct Invoice {
    #[tabulary(partition_key)]
    invoice_ref: String,
    balance_due: i64,
    paid: bool,
    #[tabulary(version)]
    edit_no: u64,
}

pub enum PaymentAttempt {
    Applied(Invoice),
    Refused(Failure<Invoice>),
}

pub async fn apply_payment(
    client: &Client,
    observed_version: u64,
) -> Result<PaymentAttempt, Box<dyn Error>> {
    let invoices = Table::<Invoice>::new("invoices")?;
    let request = invoices
        .update(InvoiceKey::new("inv-coral".into())?)
        .changes(Update::new().and(InvoiceAttrs::BALANCE_DUE.increment(-5)))
        .condition(
            InvoiceAttrs::BALANCE_DUE
                .ge(5)
                .and(InvoiceAttrs::PAID.eq(false)),
        )
        .expect_version(observed_version)
        .on_condition_failure(OldItemMode::All)
        .return_item(ReturnItem::AllNew)
        .capacity(Capacity::Total)
        .build()?;
    let raw = request.apply(client.update_item()).send().await;

    match request.decode(raw) {
        Ok(written) => {
            let invoice = written
                .item
                .ok_or_else(|| std::io::Error::other("expected returned invoice"))?;
            Ok(PaymentAttempt::Applied(invoice))
        }
        Err(WriteError::Refused { failure, .. }) => {
            if matches!(failure.kind, FailureKind::ConditionFailed) {
                match &failure.old {
                    OldItem::Decoded(old) => {
                        // The guard and version may both matter; no error text
                        // determines which business decision the caller makes.
                        let _observed_balance = old.balance_due;
                    }
                    OldItem::Invalid { error, .. } => {
                        assert_eq!(error.record, "Invoice");
                    }
                    OldItem::Unavailable | OldItem::NotRequested => {}
                }
            }
            Ok(PaymentAttempt::Refused(failure))
        }
        Err(other) => Err(Box::new(other)),
    }
}
