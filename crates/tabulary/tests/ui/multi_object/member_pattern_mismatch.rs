use tabulary::{MultiObject, Record};

#[derive(Record)]
#[tabulary(rename_all = "snake_case", partition(name = "pk", template = "CUST#{customer_ref:text}"))]
struct Customer {
    customer_ref: String,
}

#[derive(Record)]
#[tabulary(rename_all = "snake_case")]
struct Invoice {
    #[tabulary(partition_key)]
    pk: String,
}

// The member's template cannot produce the declared prefix.
#[derive(MultiObject)]
#[tabulary(partition = "pk")]
enum Mislabeled {
    #[tabulary(partition(prefix = "INV#"))]
    Customer(Customer),
}

// A plain field key promises no prefix at all.
#[derive(MultiObject)]
#[tabulary(partition = "pk")]
enum Unprefixed {
    #[tabulary(partition(prefix = "INV#"))]
    Invoice(Invoice),
}

// Members must store the shared key under the declared name.
#[derive(MultiObject)]
#[tabulary(partition = "account_key")]
enum Renamed {
    #[tabulary(partition(prefix = "CUST#"))]
    Customer(Customer),
}

fn main() {}
