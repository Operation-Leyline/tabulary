#![cfg(feature = "derive")]
use std::collections::BTreeMap;

use aws_sdk_dynamodb::types::{BillingMode, KeyType, ProjectionType, ScalarAttributeType};
use tabulary::{Billing, BuildError, Record, TableDefinition, Throughput, TtlSeconds};

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
    #[tabulary(ttl, rename = "purgeAt")]
    purge: Option<TtlSeconds>,
}

#[derive(tabulary::Record)]
#[tabulary(rename_all = "camelCase")]
struct Invoice {
    #[tabulary(partition_key)]
    order_ref: String,
    #[tabulary(sort_key)]
    created: u64,
    purge: Option<TtlSeconds>,
}

type Expiry = TtlSeconds;

#[derive(tabulary::Record)]
#[tabulary(rename_all = "snake_case")]
struct Session {
    #[tabulary(partition_key)]
    id: String,
    #[tabulary(ttl)]
    expires: Expiry,
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

#[test]
fn merge_unions_compatible_records_and_ttl_is_a_separate_optional_request() {
    let original = Order::table_definition();
    let merged =
        TableDefinition::merge(Order::table_definition(), Invoice::table_definition()).unwrap();
    let merged = TableDefinition::merge(merged, Order::table_definition()).unwrap();
    assert_eq!(
        merged.create_input("orders", Billing::OnDemand).unwrap(),
        original.create_input("orders", Billing::OnDemand).unwrap()
    );
    assert!(
        Invoice::table_definition()
            .ttl_input("orders")
            .unwrap()
            .is_none()
    );
    let ttl = merged.ttl_input("orders").unwrap().unwrap();
    assert_eq!(ttl.table_name(), Some("orders"));
    let spec = ttl.time_to_live_specification().unwrap();
    assert!(spec.enabled());
    assert_eq!(spec.attribute_name(), "purgeAt");
    assert_eq!(
        Session::table_definition()
            .ttl_input("sessions")
            .unwrap()
            .unwrap()
            .time_to_live_specification()
            .unwrap()
            .attribute_name(),
        "expires"
    );
    assert!(TableDefinition::merge(original, Session::table_definition()).is_err());
}

#[test]
fn invalid_ttl_declarations_are_compile_errors() {
    trybuild::TestCases::new().compile_fail("tests/ui/ttl/*.rs");
}

#[derive(tabulary::Record)]
#[tabulary(rename_all = "snake_case", partition(name = "pk", template = "ORDER#{id:hex}"),
    index(ident = ByCourier, name = "by_courier", kind = global, sparse,
        partition(name = "gpk", template = "COURIER#{courier:hex}"), project = all))]
struct Sparse {
    id: String,
    courier: Option<String>,
}

#[test]
fn composite_sparse_definitions_use_synthesized_physical_keys() {
    let input = Sparse::table_definition()
        .create_input("orders", Billing::OnDemand)
        .unwrap();
    let names: Vec<_> = input
        .attribute_definitions()
        .iter()
        .map(|a| a.attribute_name())
        .collect();
    assert_eq!(names, ["gpk", "pk"]);
    assert_eq!(input.key_schema()[0].attribute_name(), "pk");
    assert_eq!(
        input.global_secondary_indexes()[0].key_schema()[0].attribute_name(),
        "gpk"
    );
}
