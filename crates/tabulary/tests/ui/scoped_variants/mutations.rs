#[derive(tabulary::Variants)]
#[tabulary(tag = "kind", rename_all = "snake_case")]
enum Stage {
    Sent {
        count: u64,
        note: Option<String>,
        items: tabulary::List<String>,
        labels: tabulary::StringSet,
    },
}
#[derive(tabulary::Record)]
#[tabulary(rename_all = "snake_case")]
struct Delivery {
    #[tabulary(partition_key)]
    id: String,
    #[tabulary(flatten)]
    stage: Stage,
}
fn main() {
    DeliveryAttrs::STAGE.variant(StageTags::SENT).count.set(1);
    DeliveryAttrs::STAGE.variant(StageTags::SENT).note.remove();
    DeliveryAttrs::STAGE.variant(StageTags::SENT).count.add(1);
    DeliveryAttrs::STAGE.variant(StageTags::SENT).count.increment(1);
    DeliveryAttrs::STAGE.variant(StageTags::SENT).count.decrement(1);
    DeliveryAttrs::STAGE.variant(StageTags::SENT).count.set_expr(1);
    DeliveryAttrs::STAGE.variant(StageTags::SENT).items.list_append(tabulary::List(vec![]));
    DeliveryAttrs::STAGE.variant(StageTags::SENT).items.append_or_empty(tabulary::List(vec![]));
    DeliveryAttrs::STAGE.variant(StageTags::SENT).labels.delete_members(tabulary::StringSet::try_from(vec!["label".into()]).unwrap());
    DeliveryAttrs::STAGE.variant(StageTags::SENT).count.operand();
    DeliveryAttrs::STAGE.variant(StageTags::SENT).count.if_not_exists(1);
}
