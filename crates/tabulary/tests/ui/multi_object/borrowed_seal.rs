mod accounts {
    use tabulary::{MultiObject, Record};

    #[derive(Record)]
    #[tabulary(rename_all = "snake_case", partition(name = "pk", template = "CUST#{customer_ref:text}"))]
    pub struct Customer {
        pub customer_ref: String,
    }

    #[derive(MultiObject)]
    #[tabulary(partition = "pk")]
    pub enum AccountTable {
        #[tabulary(partition(prefix = "CUST#"))]
        Customer(Customer),
    }
}

mod elsewhere {
    use super::accounts::{AccountTable, Customer};
    use tabulary::{MemberOf, MultiObject, Record};

    type Seal<R> = <AccountTable as MultiObject>::MemberSeal<R>;

    #[derive(Record)]
    #[tabulary(rename_all = "snake_case", partition(name = "pk", template = "CUST#{customer_ref:text}"))]
    pub struct Borrower {
        pub customer_ref: String,
    }

    // Another module cannot reuse a real member's seal, whose type names that member.
    impl MemberOf<AccountTable> for Borrower {
        const VARIANT: usize = 0;
        const SEAL: Seal<Self> = <Customer as MemberOf<AccountTable>>::SEAL;
        fn into_object(self) -> AccountTable {
            AccountTable::Customer(Customer {
                customer_ref: self.customer_ref,
            })
        }
    }
}

fn main() {}
