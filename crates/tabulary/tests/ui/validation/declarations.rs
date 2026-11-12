#[derive(tabulary::Record)]
#[tabulary(rename_all = "snake_case", validate = check, validate = check)]
struct DuplicateRecord { #[tabulary(partition_key)] id: String }

#[derive(tabulary::Document)]
#[tabulary(rename_all = "snake_case", validate = check)]
#[tabulary(validate = check)]
struct DuplicateDocument { id: String }

#[derive(tabulary::Record)]
#[tabulary(rename_all = "snake_case", validate = "check")]
struct LiteralRecord { #[tabulary(partition_key)] id: String }

#[derive(tabulary::Document)]
#[tabulary(rename_all = "snake_case", validate = || Ok(()))]
struct ClosureDocument { id: String }

fn main() {}
