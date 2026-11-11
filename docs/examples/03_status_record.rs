use aws_sdk_dynamodb::Client;
use std::error::Error;
use tabulary::{OldItemMode, Record, Table, TimestampMicros, Update, Variants};

#[derive(Debug, PartialEq, Variants)]
#[tabulary(tag = "deliveryStage", rename_all = "snake_case")]
pub enum DeliveryStage {
    Waiting { queued_on: TimestampMicros },
    Sent { tracking_ref: String },
    Failed { failure_code: String },
}

#[derive(Debug, PartialEq, Record)]
#[tabulary(rename_all = "snake_case")]
pub struct Delivery {
    #[tabulary(partition_key)]
    delivery_ref: String,
    #[tabulary(flatten)]
    stage: DeliveryStage,
}

pub fn waiting_item() -> Result<tabulary::Item, tabulary::EncodeError> {
    Delivery {
        delivery_ref: "del-ochre".into(),
        stage: DeliveryStage::Waiting {
            queued_on: TimestampMicros::new(9_000_000),
        },
    }
    .to_item()
}

pub async fn mark_sent(client: &Client) -> Result<(), Box<dyn Error>> {
    let deliveries = Table::<Delivery>::new("deliveries")?;
    let changes = Update::new().transition(
        DeliveryAttrs::STAGE,
        DeliveryStage::Sent {
            tracking_ref: "track-jade".into(),
        },
        DeliveryStageTags::WAITING,
    );
    let request = deliveries
        .update(DeliveryKey::new("del-ochre".into())?)
        .changes(changes)
        .condition(DeliveryAttrs::STAGE.is(DeliveryStageTags::WAITING))
        .on_condition_failure(OldItemMode::All)
        .build()?;

    // The generated update removes queued_on and failure_code while writing
    // the discriminator and tracking_ref. The transition already guards the tag;
    // the explicit condition above is legal and visible, though redundant.
    let raw = request.apply(client.update_item()).send().await;
    request.decode(raw)?;
    Ok(())
}
