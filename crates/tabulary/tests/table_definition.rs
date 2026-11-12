#![cfg(feature = "derive")]
use std::collections::BTreeMap;

use aws_sdk_dynamodb::types::{BillingMode, KeyType, ProjectionType, ScalarAttributeType};
use tabulary::{Billing, BuildError, Record, Throughput};

#[derive(tabulary::Record)]
#[tabulary(rename_all = "camelCase",
    index(ident = ByCourier, name = "by_courier", kind = global, partition = courier, sparse, project = include(gift_note)),
    index(ident = ByLine, name = "by_line", kind = local, partition = order_ref, sort = line, project = keys_only),
    index(ident = ByToken, name = "by_token", kind = global, partition = token, sort = line, project = all))]
struct Order {
    #[tabulary(partition_key)]
    order_ref: String,
    #[tabulary(sort_key)]
    created: u64,
    courier: Option<String>,
    line: u64,
    token: Vec<u8>,
    gift_note: Option<String>,
}

#[derive(tabulary::Record)]
#[tabulary(rename_all = "snake_case")]
struct Session {
    #[tabulary(partition_key)]
    id: String,
}

#[test]
fn primary_indexes_and_projections_are_exact_inspectable_sdk_values_without_a_client() {
    let input = Order::table_definition()
        .create_input("orders_test", Billing::OnDemand)
        .unwrap();
    assert_eq!(input.table_name(), Some("orders_test"));
    assert_eq!(input.billing_mode(), Some(&BillingMode::PayPerRequest));
    assert!(input.provisioned_throughput().is_none());
    let attrs: Vec<_> = input
        .attribute_definitions()
        .iter()
        .map(|a| (a.attribute_name(), a.attribute_type().clone()))
        .collect();
    assert_eq!(
        attrs,
        vec![
            ("courier", ScalarAttributeType::S),
            ("created", ScalarAttributeType::N),
            ("line", ScalarAttributeType::N),
            ("orderRef", ScalarAttributeType::S),
            ("token", ScalarAttributeType::B)
        ]
    );
    let keys: Vec<_> = input
        .key_schema()
        .iter()
        .map(|k| (k.attribute_name(), k.key_type().clone()))
        .collect();
    assert_eq!(
        keys,
        vec![("orderRef", KeyType::Hash), ("created", KeyType::Range)]
    );
    let global = input.global_secondary_indexes();
    assert_eq!(global.len(), 2);
    assert_eq!(global[0].index_name(), "by_courier");
    assert_eq!(global[0].key_schema().len(), 1);
    assert_eq!(
        global[0].projection().unwrap().projection_type(),
        Some(&ProjectionType::Include)
    );
    assert_eq!(
        global[0].projection().unwrap().non_key_attributes(),
        &["giftNote"]
    );
    assert_eq!(
        global[1].projection().unwrap().projection_type(),
        Some(&ProjectionType::All)
    );
    assert!(global.iter().all(|i| i.provisioned_throughput().is_none()));
    let local = input.local_secondary_indexes();
    assert_eq!(local.len(), 1);
    assert_eq!(local[0].index_name(), "by_line");
    assert_eq!(local[0].key_schema()[0], input.key_schema()[0]);
    assert_eq!(local[0].key_schema()[1].attribute_name(), "line");
    assert_eq!(
        local[0].projection().unwrap().projection_type(),
        Some(&ProjectionType::KeysOnly)
    );
    let simple = Session::table_definition()
        .create_input("sessions", Billing::OnDemand)
        .unwrap();
    assert_eq!(simple.key_schema().len(), 1);
    assert!(simple.global_secondary_indexes.is_none());
    assert!(simple.local_secondary_indexes.is_none());
}

fn provisioned(indexes: &[&str], units: Throughput) -> Billing {
    Billing::Provisioned {
        table: units,
        global_indexes: indexes.iter().map(|n| ((*n).into(), units)).collect(),
    }
}

#[test]
fn throughput_is_explicit_positive_and_exactly_covers_global_indexes() {
    let units = Throughput { read: 3, write: 7 };
    for billing in [
        Billing::Provisioned {
            table: units,
            global_indexes: BTreeMap::new(),
        },
        provisioned(&["by_courier"], units),
        provisioned(&["by_courier", "by_token", "by_line"], units),
        provisioned(
            &["by_courier", "by_token"],
            Throughput { read: 0, write: 1 },
        ),
        provisioned(
            &["by_courier", "by_token"],
            Throughput { read: 1, write: -1 },
        ),
    ] {
        assert!(
            Order::table_definition()
                .create_input("orders", billing)
                .is_err()
        );
    }
    let mut billing = provisioned(&["by_courier", "by_token"], units);
    if let Billing::Provisioned { global_indexes, .. } = &mut billing {
        global_indexes.insert(
            "by_token".into(),
            Throughput {
                read: 11,
                write: 13,
            },
        );
    }
    let input = Order::table_definition()
        .create_input("orders", billing.clone())
        .unwrap();
    assert_eq!(input.billing_mode(), Some(&BillingMode::Provisioned));
    let table = input.provisioned_throughput().unwrap();
    assert_eq!(
        (table.read_capacity_units(), table.write_capacity_units()),
        (3, 7)
    );
    let gsi = input.global_secondary_indexes()[1]
        .provisioned_throughput()
        .unwrap();
    assert_eq!(
        (gsi.read_capacity_units(), gsi.write_capacity_units()),
        (11, 13)
    );
    if let Billing::Provisioned { global_indexes, .. } = &mut billing {
        global_indexes.insert("by_token".into(), Throughput { read: -1, write: 1 });
    }
    assert!(
        Order::table_definition()
            .create_input("orders", billing)
            .is_err()
    );
    assert!(matches!(
        Session::table_definition().create_input("", Billing::OnDemand),
        Err(BuildError::OperationConstraint { .. })
    ));
}
