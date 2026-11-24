use tabulary::{MultiObject, MultiTable, ReadConsistency, Record};

#[derive(Record)]
#[tabulary(rename_all = "snake_case", partition(name = "pk", template = "CUST#{customer_ref:text}"))]
struct Customer {
    customer_ref: String,
}

#[derive(Record)]
#[tabulary(rename_all = "snake_case", partition(name = "pk", template = "SES#{session_ref:text}"))]
struct Session {
    session_ref: String,
}

#[derive(MultiObject)]
#[tabulary(partition = "pk")]
enum AccountTable {
    #[tabulary(partition(prefix = "CUST#"))]
    Customer(Customer),
}

fn main() {
    let accounts = MultiTable::<AccountTable>::new("accounts").unwrap();
    let session = Session {
        session_ref: "ses-plum".into(),
    };
    let _ = accounts.put(&session);
    let _ = accounts.get(SessionKey::new("ses-plum".into()).unwrap(), ReadConsistency::Strong);
    let _ = accounts.member::<Session>();
}
