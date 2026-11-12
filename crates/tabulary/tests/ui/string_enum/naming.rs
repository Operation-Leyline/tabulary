#[derive(tabulary::StringEnum)]
enum Missing { Ready }

#[derive(tabulary::StringEnum)]
enum Empty {
    #[tabulary(rename = "")]
    Ready,
}

#[derive(tabulary::StringEnum)]
#[tabulary(rename_all = "lowercase")]
enum Unsupported { Ready }

#[derive(tabulary::StringEnum)]
enum Duplicate {
    #[tabulary(rename = "ready", rename = "sent")]
    Ready,
}

fn main() {}
