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

    // Same record name, keys, and prefix as the real member.
    #[derive(Record)]
    #[tabulary(rename_all = "snake_case", partition(name = "pk", template = "CUST#{customer_ref:text}"))]
    pub struct Impostor {
        pub customer_ref: String,
    }

    type Seal<R> = <AccountTable as MultiObject>::MemberSeal<R>;

    // Another module cannot construct the enum's seal for its own type.
    impl MemberOf<AccountTable> for Impostor {
        const VARIANT: usize = 0;
        const SEAL: Seal<Self> = Seal { 0: std::marker::PhantomData };
        fn into_object(self) -> AccountTable {
            AccountTable::Customer(Customer {
                customer_ref: self.customer_ref,
            })
        }
    }
}

fn main() {}
