#[derive(tabulary::Record)]
#[tabulary(rename_all = "snake_case", validate = wrong_input)]
struct WrongInput { #[tabulary(partition_key)] id: String }
fn wrong_input(_: &str) -> Result<(), tabulary::ValidationFault> { Ok(()) }

#[derive(tabulary::Document)]
#[tabulary(rename_all = "snake_case", validate = wrong_output)]
struct WrongOutput { id: String }
fn wrong_output(_: &WrongOutput) -> Result<(), &'static str> { Ok(()) }

#[derive(tabulary::Document)]
#[tabulary(rename_all = "snake_case", validate = mutable)]
struct Mutable { id: String }
fn mutable(_: &mut Mutable) -> Result<(), tabulary::ValidationFault> { Ok(()) }

#[derive(tabulary::Record)]
#[tabulary(rename_all = "snake_case", validate = asynchronous)]
struct Async { #[tabulary(partition_key)] id: String }
async fn asynchronous(_: &Async) -> Result<(), tabulary::ValidationFault> { Ok(()) }

fn main() {}
