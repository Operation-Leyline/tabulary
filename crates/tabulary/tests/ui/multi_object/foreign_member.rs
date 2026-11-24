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

    type Seal = <AccountTable as MultiObject>::MemberSeal;

    // Another module cannot construct the enum's seal, so it cannot add a member.
    impl MemberOf<AccountTable> for Impostor {
        const VARIANT: usize = 0;
        const SEAL: Seal = Seal { 0: () };
        fn into_object(self) -> AccountTable {
            AccountTable::Customer(Customer {
                customer_ref: self.customer_ref,
            })
        }
    }
}

fn main() {}
