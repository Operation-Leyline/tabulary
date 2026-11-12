#[derive(tabulary::StringEnum)]
#[tabulary(rename_all = "snake_case")]
enum Stage {
    Ready,
    #[tabulary(rename = "ready")]
    Sent,
}

#[derive(tabulary::StringEnum)]
#[tabulary(rename_all = "snake_case")]
enum Normalized {
    HTTPReady,
    HttpReady,
}

fn main() {}
