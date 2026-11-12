#[derive(tabulary::Variants)]
#[tabulary(tag = "kind", rename_all = "snake_case")]
enum Stage {
    Sent {
        tracking: String,
        count: u64,
        note: Option<String>,
        items: tabulary::List<String>,
        labels: tabulary::StringSet,
        #[tabulary(with = tabulary::convert::u128_string)]
        external: u128,
    },
    Failed { failure: String },
}
#[derive(tabulary::Variants)]
#[tabulary(tag = "kind", rename_all = "snake_case")]
enum Other { Sent }
#[derive(tabulary::Record)]
#[tabulary(rename_all = "snake_case")]
struct Delivery {
    #[tabulary(partition_key)]
    id: String,
    #[tabulary(flatten)]
    stage: Stage,
}
#[derive(tabulary::Record)]
#[tabulary(rename_all = "snake_case")]
struct Nested {
    #[tabulary(partition_key)]
    id: String,
    stage: Stage,
}
#[derive(tabulary::Record)]
#[tabulary(rename_all = "snake_case")]
struct OtherRecord {
    #[tabulary(partition_key)]
    id: String,
    #[tabulary(flatten)]
    stage: Stage,
}
fn main() {
    DeliveryAttrs::ID.exists().and(OtherRecordAttrs::STAGE.variant(StageTags::SENT).note.exists());
    DeliveryAttrs::STAGE.variant(OtherTags::SENT);
    DeliveryAttrs::STAGE.variant(StageTags::SENT).failure.exists();
    DeliveryAttrs::STAGE.variant(StageTags::FAILED).tracking.exists();
    NestedAttrs::STAGE.variant(StageTags::SENT);
    DeliveryAttrs::STAGE.variant(StageTags::SENT).count.eq("wrong".to_owned());
    DeliveryAttrs::STAGE.variant(StageTags::SENT).external.lt(3);
    DeliveryAttrs::STAGE.variant(StageTags::SENT).count.begins_with(3);
    DeliveryAttrs::STAGE.variant(StageTags::SENT).items.contains(3);
}
