//! Nested document paths against the actual expression parser: aliased
//! segments, nested ADD on numbers, sets and map entries, and parent maps.
use aws_sdk_dynamodb::error::ProvideErrorMetadata;
use tabulary::{
    FailureKind, Map, MapKey, ReadConsistency::Strong, ReturnItem, StringSet, Table, Update,
    WriteError,
};

#[derive(Debug, PartialEq, tabulary::Document)]
#[tabulary(rename_all = "camelCase")]
struct Address {
    city: String,
    #[tabulary(rename = "name")]
    recipient: String,
}

#[derive(Debug, PartialEq, tabulary::Document)]
#[tabulary(rename_all = "camelCase")]
struct Shipping {
    // Reserved words at every nested level exercise per-segment aliasing.
    status: String,
    #[tabulary(rename = "size")]
    parcel_size: u64,
    attempts: u64,
    note: Option<String>,
    address: Address,
    tags: StringSet,
    counters: Map<u64>,
}

#[derive(Debug, PartialEq, tabulary::Document)]
#[tabulary(rename_all = "snake_case")]
struct Gift {
    message: String,
}

#[derive(Debug, PartialEq, tabulary::Record)]
#[tabulary(rename_all = "snake_case")]
struct Shipment {
    #[tabulary(partition_key)]
    id: String,
    #[tabulary(rename = "data")]
    shipping: Shipping,
    gift: Option<Gift>,
}

fn tags(values: &[&str]) -> StringSet {
    StringSet::try_from(values.iter().map(|v| (*v).to_owned()).collect::<Vec<_>>()).unwrap()
}

fn shipment() -> Shipment {
    Shipment {
        id: "shipment-1".into(),
        shipping: Shipping {
            status: "packed".into(),
            parcel_size: 3,
            attempts: 1,
            note: Some("leave at desk".into()),
            address: Address {
                city: "Portland".into(),
                recipient: "Front desk".into(),
            },
            tags: tags(&["fragile"]),
            counters: Map([("scan.count".to_owned(), 1)].into()),
        },
        gift: None,
    }
}

fn key() -> ShipmentKey {
    ShipmentKey::new("shipment-1".into()).unwrap()
}

#[tokio::test]
#[ignore = "requires explicit TABULARY_DYNAMODB_ENDPOINT, AWS region and credentials"]
async fn nested_conditions_and_updates_reach_the_expression_parser() {
    crate::support::with_table::<Shipment, _, _>(|client, name| async move {
        let table = Table::<Shipment>::new(name).unwrap();
        let seed = table.put(&shipment()).build().unwrap();
        seed.decode(seed.apply(client.put_item()).send().await)
            .unwrap();

        let ship = || ShipmentAttrs::SHIPPING;
        let update = table
            .update(key())
            .changes(
                Update::new()
                    .and(ship().field(ShippingAttrs::STATUS).set("shipped".into()))
                    .and(
                        ship()
                            .field(ShippingAttrs::ADDRESS)
                            .field(AddressAttrs::CITY)
                            .set_if_not_exists("Salem".into()),
                    )
                    .and(ship().field(ShippingAttrs::ATTEMPTS).add(2))
                    .and(ship().field(ShippingAttrs::TAGS).add(tags(&["late"])))
                    .and(
                        ship()
                            .field(ShippingAttrs::COUNTERS)
                            .entry(MapKey::new("scan.count").unwrap())
                            .add(4),
                    )
                    .and(
                        ship()
                            .field(ShippingAttrs::COUNTERS)
                            .entry(MapKey::new("new").unwrap())
                            .add(5),
                    )
                    .and(ship().field(ShippingAttrs::NOTE).remove()),
            )
            .condition(
                ship()
                    .field(ShippingAttrs::ADDRESS)
                    .field(AddressAttrs::CITY)
                    .eq("Portland".into())
                    .and(ship().field(ShippingAttrs::PARCEL_SIZE).between(1, 5))
                    .and(ship().field(ShippingAttrs::TAGS).contains("fragile".into()))
                    .and(ship().field(ShippingAttrs::STATUS).size().eq(6))
                    .and(
                        ship()
                            .field(ShippingAttrs::ADDRESS)
                            .field(AddressAttrs::RECIPIENT)
                            .begins_with("Front".into()),
                    )
                    .and(ship().field(ShippingAttrs::NOTE).exists())
                    .and(ShipmentAttrs::GIFT.field(GiftAttrs::MESSAGE).not_exists()),
            )
            .return_item(ReturnItem::AllNew)
            .build()
            .unwrap();
        let updated = update
            .decode(update.apply(client.update_item()).send().await)
            .unwrap()
            .item
            .unwrap();
        let mut expected = shipment();
        expected.shipping.status = "shipped".into();
        expected.shipping.attempts = 3;
        expected.shipping.note = None;
        expected.shipping.tags = tags(&["fragile", "late"]);
        expected.shipping.counters =
            Map([("scan.count".to_owned(), 5), ("new".to_owned(), 5)].into());
        assert_eq!(updated, expected);

        // A false nested predicate is a modeled conditional refusal.
        let refused = table
            .update(key())
            .changes(Update::new().and(ship().field(ShippingAttrs::ATTEMPTS).add(1)))
            .condition(ship().field(ShippingAttrs::STATUS).eq("packed".into()))
            .build()
            .unwrap();
        let error = refused
            .decode(refused.apply(client.update_item()).send().await)
            .unwrap_err();
        let WriteError::Refused { failure, .. } = error else {
            panic!("expected a conditional refusal: {error:?}");
        };
        assert_eq!(failure.kind, FailureKind::ConditionFailed);

        // An absent optional parent map: a guarded write is refused by the
        // condition; an unguarded write is rejected by the service.
        let message = || ShipmentAttrs::GIFT.field(GiftAttrs::MESSAGE);
        let guarded = table
            .update(key())
            .changes(Update::new().and(message().set("hello".into())))
            .condition(ShipmentAttrs::GIFT.exists())
            .build()
            .unwrap();
        let error = guarded
            .decode(guarded.apply(client.update_item()).send().await)
            .unwrap_err();
        assert!(matches!(error, WriteError::Refused { .. }), "{error:?}");
        let unguarded = table
            .update(key())
            .changes(Update::new().and(message().set("hello".into())))
            .build()
            .unwrap();
        let error = unguarded
            .decode(unguarded.apply(client.update_item()).send().await)
            .unwrap_err();
        let WriteError::Sdk(source) = error else {
            panic!("expected a service validation error: {error:?}");
        };
        assert_eq!(
            source.as_service_error().and_then(|e| e.code()),
            Some("ValidationException")
        );

        let scan = table
            .scan(Strong)
            .filter(
                ship()
                    .field(ShippingAttrs::COUNTERS)
                    .entry(MapKey::new("scan.count").unwrap())
                    .ge(5),
            )
            .build()
            .unwrap();
        let page = scan.decode(scan.apply(client.scan()).send().await).unwrap();
        assert_eq!(page.items, vec![expected]);
    })
    .await;
}
