#[derive(tabulary::Record)]
#[tabulary(rename_all = "snake_case")]
struct Invoice {
    #[tabulary(partition_key)]
    account: String,
    total: u64,
}

#[derive(tabulary::Projection)]
#[tabulary(record = Invoice)]
struct Total {
    #[tabulary(attribute = InvoiceAttrs::TOTAL)]
    total: u64,
}

fn main() {
    let table = tabulary::Table::<Invoice>::new("invoices").unwrap();
    let key = || Invoice::primary().partition("acct-1".into());
    // A count has no item shape to project.
    let _ = table.query(key(), tabulary::ReadConsistency::Strong).count().select::<Total>();
    // A projected read cannot become a count.
    let _ = table.query(key(), tabulary::ReadConsistency::Strong).select::<Total>().count();
    let _ = table.scan(tabulary::ReadConsistency::Strong).count().select::<Total>();
    let _ = table.scan(tabulary::ReadConsistency::Strong).select::<Total>().count();
}
