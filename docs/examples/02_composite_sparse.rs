use aws_sdk_dynamodb::Client;
use std::error::Error;
use tabulary::{Billing, ReadConsistency, Record, Table};

#[derive(Debug, PartialEq, Record)]
#[tabulary(rename_all = "camelCase")]
#[tabulary(partition(
    name = "parcelKey",
    template = "parcel!{shop_code:hex}!{parcel_token:hex}"
))]
#[tabulary(index(
    ident = ByCourier,
    name = "by_courier",
    kind = global,
    partition = assigned_courier,
    sort = parcel_no,
    sparse,
    project = all
))]
pub struct Parcel {
    shop_code: String,
    parcel_token: [u8; 16],
    #[tabulary(sort_key)]
    parcel_no: u64,
    assigned_courier: Option<String>,
}

pub fn mapping_and_definition() -> Result<(), Box<dyn Error>> {
    let unassigned = Parcel {
        shop_code: "shop-cyan".into(),
        parcel_token: [0x5d; 16],
        parcel_no: 2,
        assigned_courier: None,
    };
    let item = unassigned.to_item()?;
    assert!(!item.contains_key(ParcelAttrs::ASSIGNED_COURIER.name()));
    assert!(item.contains_key(ParcelAttrs::PARTITION.name()));
    // Decode also parses the synthesized key and verifies both source fields.
    assert_eq!(Parcel::from_item(&item)?, unassigned);

    let definition = Parcel::table_definition();
    let create = definition.create_input("parcels_test", Billing::OnDemand)?;
    assert_eq!(create.global_secondary_indexes().len(), 1);
    Ok(())
}

pub async fn courier_page(client: &Client) -> Result<Vec<Parcel>, Box<dyn Error>> {
    let parcels = Table::<Parcel>::new("parcels")?;
    let by_courier = parcels.index(ParcelIndexes::BY_COURIER);
    let request = by_courier
        .query(
            ParcelIndexes::BY_COURIER
                .partition("courier-lime".into())
                .sort_ge(8),
            ReadConsistency::Eventual,
        )
        .build()?;
    let raw = request.apply(client.query()).send().await;
    let page = request.decode(raw)?;
    // This function intentionally returns one page; the caller can keep next.
    assert!(page.scanned_count >= page.count);
    Ok(page.items)
}
