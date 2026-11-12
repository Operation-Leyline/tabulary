#[derive(tabulary::StringEnum)]
#[tabulary(rename_all = "snake_case")]
struct NotEnum;

#[derive(tabulary::StringEnum)]
#[tabulary(rename_all = "snake_case")]
enum Alias {
    #[tabulary(alias = "ready")]
    Sent,
}

#[derive(tabulary::StringEnum)]
#[tabulary(rename_all = "snake_case")]
enum Fallback {
    #[tabulary(other)]
    Unknown,
}

fn main() {}
