use tabulary::{Record, Update, Variants};

#[derive(Variants)]
#[tabulary(tag = "kind", rename_all = "snake_case")]
enum Stage {
    Waiting,
    Sent { tracking: String },
}

#[derive(Variants)]
#[tabulary(tag = "kind", rename_all = "snake_case")]
enum Other {
    Waiting,
}

#[derive(Record)]
#[tabulary(rename_all = "snake_case")]
struct Delivery {
    #[tabulary(partition_key)]
    id: String,
    #[tabulary(flatten)]
    stage: Stage,
}

#[derive(Record)]
#[tabulary(rename_all = "snake_case")]
struct Nested {
    #[tabulary(partition_key)]
    id: String,
    stage: Stage,
}

fn main() {
    Update::<Nested>::new().transition(DeliveryAttrs::STAGE, Stage::Waiting, StageTags::WAITING);
    Update::<Delivery>::new().transition(DeliveryAttrs::STAGE, Other::Waiting, OtherTags::WAITING);
    Update::<Delivery>::new().transition(DeliveryAttrs::STAGE, Stage::Waiting, OtherTags::WAITING);
    Update::<Nested>::new().transition(NestedAttrs::STAGE, Stage::Waiting, StageTags::WAITING);
    DeliveryAttrs::STAGE.set(StageTags::SENT);
}
