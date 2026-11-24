use tabulary::{MultiObject, Record};

#[derive(Record)]
#[tabulary(rename_all = "snake_case", partition(name = "pk", template = "CUST#{customer_ref:text}"))]
struct Customer {
    customer_ref: String,
}

#[derive(MultiObject)]
enum MissingPartition {
    Customer(Customer),
}

#[derive(MultiObject)]
#[tabulary(partition = "pk")]
enum SortWithoutKey {
    #[tabulary(sort(prefix = "META"))]
    Customer(Customer),
}

#[derive(MultiObject)]
#[tabulary(partition = "pk")]
enum NamedPayload {
    Customer { record: Customer },
}

#[derive(MultiObject)]
#[tabulary(partition = "pk")]
enum EmptyPrefix {
    #[tabulary(partition(prefix = ""))]
    Customer(Customer),
}

#[derive(MultiObject)]
#[tabulary(partition = "pk")]
enum Twice {
    #[tabulary(partition(prefix = "CUST#"))]
    First(Customer),
    #[tabulary(partition(prefix = "INV#"))]
    Second(Customer),
}

#[derive(MultiObject)]
#[tabulary(partition = "pk")]
enum BothForms {
    #[tabulary(partition(prefix = "CUST#", exact = "CUST#1"))]
    Customer(Customer),
}

fn main() {}
