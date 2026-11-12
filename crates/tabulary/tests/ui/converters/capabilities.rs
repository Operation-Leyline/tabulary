#[derive(tabulary::Record)]
#[tabulary(rename_all = "snake_case")]
struct Invoice {
    #[tabulary(partition_key)] id: String,
    #[tabulary(sort_key, with = tabulary::convert::u128_string)] sort: u128,
    #[tabulary(with = tabulary::convert::u128_string)] total: u128,
    native: u128,
}
fn main() {
    InvoiceAttrs::TOTAL.lt(1);
    InvoiceAttrs::TOTAL.begins_with(1);
    InvoiceAttrs::TOTAL.size();
    InvoiceAttrs::TOTAL.contains(1);
    InvoiceAttrs::TOTAL.add(1);
    InvoiceAttrs::TOTAL.delete_members(1);
    InvoiceAttrs::TOTAL.increment(1);
    InvoiceAttrs::TOTAL.if_not_exists(1).plus(1);
    InvoiceAttrs::TOTAL.eq("1".to_owned());
    InvoiceAttrs::TOTAL.eq_path(InvoiceAttrs::NATIVE);
    InvoiceAttrs::TOTAL.set_expr(InvoiceAttrs::NATIVE.operand());
    Invoice::primary().partition("invoice".into()).sort_lt(1);
    Invoice::primary().partition("invoice".into()).sort_begins_with(1);
}
