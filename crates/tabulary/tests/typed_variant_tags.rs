#![cfg(feature = "derive")]
use aws_sdk_dynamodb::types::AttributeValue as Av;
use tabulary::{EncodeContext, Value};

#[derive(tabulary::Variants)]
#[tabulary(tag = "stage", rename_all = "snake_case")]
enum Stage {
    Sent,
    #[tabulary(rename = "deliveryFailed")]
    Failed,
}
#[test]
fn singleton_tags_convert_to_a_shared_enum_for_collections_and_matches() {
    let _variants = [Stage::Sent, Stage::Failed];
    let mixed: Vec<StageTag> = vec![StageTag::from(StageTags::SENT), StageTags::FAILED.into()];
    let wire_tags: Vec<_> = mixed
        .into_iter()
        .map(|tag| {
            let expected = match tag {
                StageTag::Sent => "sent",
                StageTag::Failed => "deliveryFailed",
            };
            assert_eq!(
                tag.encode_value(&mut EncodeContext::new("Delivery"))
                    .unwrap(),
                Av::S(expected.into())
            );
            expected
        })
        .collect();
    assert_eq!(wire_tags, ["sent", "deliveryFailed"]);
    assert_eq!(
        StageTags::SENT
            .encode_value(&mut EncodeContext::new("Delivery"))
            .unwrap(),
        Av::S("sent".into())
    );
}
