#[derive(tabulary::Variants)]
#[tabulary(tag = "kind", rename_all = "snake_case")]
enum Stage {
    Waiting,
    Sent { tracking: String },
}

#[derive(tabulary::Record)]
#[tabulary(rename_all = "snake_case")]
struct Multiple {
    #[tabulary(partition_key)]
    id: String,
    #[tabulary(flatten)]
    first: Stage,
    #[tabulary(flatten)]
    second: Stage,
}

#[derive(tabulary::Document)]
#[tabulary(rename_all = "snake_case")]
struct Collision {
    kind: String,
    #[tabulary(flatten)]
    stage: Stage,
}

#[derive(tabulary::Record)]
#[tabulary(rename_all = "snake_case")]
struct PayloadCollision {
    #[tabulary(partition_key)]
    tracking: String,
    #[tabulary(flatten)]
    stage: Stage,
}

#[derive(tabulary::Document)]
struct Optional {
    #[tabulary(flatten)]
    stage: Option<Stage>,
}
fn main() {}
