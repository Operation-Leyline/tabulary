#![cfg(feature = "derive")]

use aws_sdk_dynamodb::{
    error::SdkError,
    operation::batch_get_item::BatchGetItemOutput,
    types::{AttributeValue as Av, ConsumedCapacity, KeysAndAttributes},
};
use tabulary::{
    BatchBudget, BatchGet, BatchGetFailure, BatchGetState, BatchStateError as State, Capacity,
    Item, ReadConsistency, Table,
};

#[derive(tabulary::Record)]
#[tabulary(rename_all = "snake_case")]
struct Order {
    #[tabulary(partition_key)]
    id: String,
    #[tabulary(sort_key)]
    line: u64,
    quantity: u64,
}
#[derive(Debug, PartialEq, tabulary::Projection)]
#[tabulary(record = Order)]
struct Quantity {
    #[tabulary(attribute = OrderAttrs::QUANTITY)]
    quantity: u64,
}
#[derive(tabulary::Projection)]
#[tabulary(record = Order)]
struct Identity {
    #[tabulary(attribute = OrderAttrs::ID)]
    id: String,
    #[tabulary(attribute = OrderAttrs::LINE)]
    line: u64,
}
fn batch(calls: u32) -> BatchGet<Order, OrderKey> {
    Table::<Order>::new("orders")
        .unwrap()
        .batch_get(
            (0..3)
                .map(|line| OrderKey::new("order-lime".into(), line).unwrap())
                .collect(),
            ReadConsistency::Strong,
            BatchBudget::new(calls).unwrap(),
        )
        .unwrap()
}
fn item(line: u64) -> Item {
    Order {
        id: "order-lime".into(),
        line,
        quantity: 7,
    }
    .to_item()
    .unwrap()
}
fn output(items: Vec<Item>, pending: Vec<Item>) -> BatchGetItemOutput {
    BatchGetItemOutput::builder()
        .responses("orders", items)
        .unprocessed_keys(
            "orders",
            KeysAndAttributes::builder()
                .set_keys(Some(pending))
                .build()
                .unwrap(),
        )
        .consumed_capacity(
            ConsumedCapacity::builder()
                .table_name("orders")
                .capacity_units(1.5)
                .build(),
        )
        .build()
}

#[test]
fn reordered_projection_rounds_correlate_full_keys_and_preserve_capacity() {
    let mut batch = batch(2).select::<Quantity>().unwrap();
    batch.set_capacity(Capacity::Indexes).unwrap();
    let first = batch.next_request().unwrap().unwrap();
    let table = &first.input().request_items.as_ref().unwrap()["orders"];
    assert_eq!(table.consistent_read, Some(true));
    assert_eq!(
        table.projection_expression.as_deref(),
        Some("#n0, #n1, #n2")
    );
    assert_eq!(table.expression_attribute_names.as_ref().unwrap().len(), 3);
    assert_eq!(
        table.expression_attribute_names.as_ref().unwrap()["#n0"],
        "quantity"
    );
    assert_eq!(
        table.expression_attribute_names.as_ref().unwrap()["#n1"],
        "id"
    );
    assert_eq!(
        table.expression_attribute_names.as_ref().unwrap()["#n2"],
        "line"
    );
    assert_eq!(
        batch.set_capacity(Capacity::None).unwrap_err(),
        State::AlreadyStarted
    );
    assert_eq!(batch.next_request().unwrap_err(), State::OutstandingRound);
    let mut returned = item(2);
    returned.insert("line".into(), Av::N("2.0".into()));
    let pending = first.input().request_items.as_ref().unwrap()["orders"].keys[0].clone();
    let mut raw = output(vec![returned], vec![pending.clone()]);
    let expected_capacity = raw.consumed_capacity.clone().unwrap();
    let echoed = raw
        .unprocessed_keys
        .as_mut()
        .unwrap()
        .get_mut("orders")
        .unwrap();
    echoed.consistent_read = Some(false);
    echoed.projection_expression = Some("stale".into());
    batch.accept(first.decode(Ok(raw))).unwrap();
    let second = batch.next_request().unwrap().unwrap();
    let next = &second.input().request_items.as_ref().unwrap()["orders"];
    assert_eq!(next.consistent_read, table.consistent_read);
    assert_eq!(next.projection_expression, table.projection_expression);
    assert_eq!(
        next.expression_attribute_names,
        table.expression_attribute_names
    );
    assert_eq!(
        second.input().request_items.as_ref().unwrap()["orders"].keys,
        vec![pending]
    );
    batch
        .accept(second.decode(Ok(output(vec![item(0)], vec![]))))
        .unwrap();
    assert!(batch.next_request().unwrap().is_none());
    let complete = batch.finish().unwrap();
    assert_eq!(
        complete.records,
        vec![
            Some(Quantity { quantity: 7 }),
            None,
            Some(Quantity { quantity: 7 })
        ]
    );
    assert_eq!(complete.capacity.len(), 2);
    assert_eq!(complete.capacity[0], expected_capacity);
    assert_eq!(complete.capacity[0][0].capacity_units, Some(1.5));
}

#[test]
fn exhausted_budget_preserves_pending_and_processed_entries() {
    let mut batch = batch(1);
    let request = batch.next_request().unwrap().unwrap();
    let pending = request.input().request_items.as_ref().unwrap()["orders"].keys[2].clone();
    batch
        .accept(request.decode(Ok(output(vec![item(0)], vec![pending]))))
        .unwrap();
    assert!(batch.next_request().unwrap().is_none());
    let error = batch.finish().unwrap_err();
    assert!(matches!(
        error.kind,
        BatchGetFailure::State(State::BudgetExhausted)
    ));
    assert!(matches!(
        error.partial.entries[0].state,
        BatchGetState::Found(_)
    ));
    assert!(matches!(
        error.partial.entries[1].state,
        BatchGetState::Missing
    ));
    assert!(matches!(
        error.partial.entries[2].state,
        BatchGetState::Pending
    ));
    assert_eq!(error.partial.calls_issued, 1);
    assert_eq!(error.partial.capacity.len(), 1);
}

#[test]
fn protocol_faults_freeze_without_trusting_round_progress() {
    for case in 0..7 {
        let mut batch = batch(3);
        let request = batch.next_request().unwrap().unwrap();
        let key = request.input().request_items.as_ref().unwrap()["orders"].keys[0].clone();
        let mut raw = output(vec![], vec![]);
        match case {
            0 => {
                raw.responses = Some([("orders".into(), vec![item(9)])].into());
            }
            1 => {
                raw.responses = Some([("orders".into(), vec![item(0), item(0)])].into());
            }
            2 => {
                raw.responses = Some([("foreign".into(), vec![])].into());
            }
            3 => {
                raw = output(vec![item(0)], vec![key]);
            }
            4 => {
                raw = output(vec![], vec![key.clone(), key]);
            }
            5 => {
                raw = output(vec![Item::new()], vec![]);
            }
            _ => {
                raw = output(vec![], vec![item(0)]);
            }
        }
        let error = batch.accept(request.decode(Ok(raw))).unwrap_err();
        assert!(matches!(
            error.kind,
            BatchGetFailure::State(State::InvalidResponse)
        ));
        assert!(
            error
                .partial
                .entries
                .iter()
                .all(|entry| matches!(entry.state, BatchGetState::Pending))
        );
        assert_eq!(error.partial.capacity.len(), 1);
        assert_eq!(batch.next_request().unwrap_err(), State::Frozen);
    }
}

#[test]
fn decode_and_sdk_failures_keep_accumulated_progress_and_freeze() {
    for sdk in [false, true] {
        let mut batch = batch(3);
        let first = batch.next_request().unwrap().unwrap();
        let keys = first.input().request_items.as_ref().unwrap()["orders"]
            .keys
            .clone();
        batch
            .accept(first.decode(Ok(output(vec![item(0)], keys[1..].to_vec()))))
            .unwrap();
        let second = batch.next_request().unwrap().unwrap();
        let result = if sdk {
            Err(SdkError::construction_failure(std::io::Error::other(
                "private-sdk-value",
            )))
        } else {
            let mut corrupt = item(1);
            corrupt.insert("quantity".into(), Av::Bool(false));
            Ok(output(vec![corrupt, item(2)], vec![]))
        };
        let error = batch.accept(second.decode(result)).unwrap_err();
        assert!(!format!("{error:?} {error}").contains("private-sdk-value"));
        assert!(matches!(
            error.partial.entries[0].state,
            BatchGetState::Found(_)
        ));
        assert!(matches!(
            error.partial.entries[1].state,
            BatchGetState::Pending
        ));
        if sdk {
            assert!(matches!(error.kind, BatchGetFailure::Sdk(_)));
            assert!(matches!(
                error.partial.entries[2].state,
                BatchGetState::Pending
            ));
            assert!(error.partial.capacity[1].is_empty());
        } else {
            let BatchGetFailure::Decode { failures } = &error.kind else {
                panic!("decode fault")
            };
            assert_eq!(failures.len(), 1);
            let (position, source) = &failures[0];
            assert_eq!(*position, 1);
            assert_eq!(source.attribute(), Some("quantity"));
            assert_eq!(source.record, "Order");
            assert!(matches!(
                error.partial.entries[2].state,
                BatchGetState::Found(_)
            ));
        }
        assert_eq!(batch.next_request().unwrap_err(), State::Frozen);
    }
}

#[test]
fn apply_replaces_all_fields_without_transport_and_rounds_cannot_be_replayed() {
    use aws_sdk_dynamodb::{
        Client, Config,
        config::{
            IdentityCache, Region, StalledStreamProtectionConfig, retry::RetryConfig,
            timeout::TimeoutConfig,
        },
        types::ReturnConsumedCapacity,
    };
    let client = Client::from_conf(
        Config::builder()
            .behavior_version_latest()
            .region(Region::new("us-east-1"))
            .retry_config(RetryConfig::disabled())
            .timeout_config(TimeoutConfig::disabled())
            .identity_cache(IdentityCache::no_cache())
            .stalled_stream_protection(StalledStreamProtectionConfig::disabled())
            .http_client(aws_smithy_runtime_api::client::http::http_client_fn(
                |_, _| panic!("no transport"),
            ))
            .build(),
    );
    let mut batch = batch(2);
    let request = batch.next_request().unwrap().unwrap();
    let stale = client
        .batch_get_item()
        .request_items(
            "stale",
            KeysAndAttributes::builder()
                .keys(Item::new())
                .build()
                .unwrap(),
        )
        .return_consumed_capacity(ReturnConsumedCapacity::Total);
    assert_eq!(
        request.apply(stale).as_input().clone().build().unwrap(),
        *request.input()
    );
    let keys = request.input().request_items.as_ref().unwrap()["orders"]
        .keys
        .clone();
    batch
        .accept(request.decode(Ok(output(vec![], keys))))
        .unwrap();
    let error = batch
        .accept(request.decode(Ok(output(vec![], vec![]))))
        .unwrap_err();
    assert!(matches!(
        error.kind,
        BatchGetFailure::State(State::WrongRound)
    ));
    assert_eq!(batch.next_request().unwrap_err(), State::Frozen);
    assert!(!format!("{request:?}").contains("order-lime"));
}

#[test]
fn abort_freezes_the_outstanding_round_and_finish_retains_its_cause() {
    for cause in [
        tabulary::BatchRoundAbort::Cancelled,
        tabulary::BatchRoundAbort::Deadline,
    ] {
        let mut batch = batch(2);
        assert_eq!(batch.abort_round(cause).unwrap_err(), State::WrongRound);
        let _request = batch.next_request().unwrap().unwrap();
        batch.abort_round(cause).unwrap();
        assert_eq!(batch.next_request().unwrap_err(), State::Frozen);
        let error = batch.finish().unwrap_err();
        assert!(matches!(error.kind, BatchGetFailure::Aborted(actual) if actual == cause));
        assert_eq!(error.partial.calls_issued, 1);
        assert!(
            error
                .partial
                .entries
                .iter()
                .all(|entry| matches!(entry.state, BatchGetState::Pending))
        );
    }
}

#[path = "../../../docs/examples/06_query_batch.rs"]
mod query_batch;
#[test]
fn query_batch_example_compiles() {
    let _ = query_batch::read_lines;
    let _ = query_batch::read_selected;
}

#[test]
fn selecting_primary_keys_deduplicates_transport_attributes_and_decodes_them() {
    let mut selected = batch(1).select::<Identity>().unwrap();
    let request = selected.next_request().unwrap().unwrap();
    assert_eq!(request.expressions().names().len(), 2);
    selected
        .accept(request.decode(Ok(output(vec![item(1)], vec![]))))
        .unwrap();
    let complete = selected.finish().unwrap();
    let identity = complete.records[1].as_ref().unwrap();
    assert_eq!(identity.id, "order-lime");
    assert_eq!(identity.line, 1);
    let mut started = batch(1);
    let _ = started.next_request().unwrap();
    assert!(matches!(
        started.select::<Identity>(),
        Err(tabulary::BuildError::OperationConstraint {
            code: "batch_already_started"
        })
    ));
}
#[test]
fn batch_consistency_budget_keys_and_projection_owner_are_compile_gates() {
    trybuild::TestCases::new().compile_fail("tests/ui/batch_get/*.rs");
}

#[test]
fn every_malformed_position_is_identified_while_good_records_are_retained() {
    let mut batch = batch(2);
    let request = batch.next_request().unwrap().unwrap();
    let mut missing = item(0);
    missing.remove("quantity");
    let mut mistyped = item(2);
    mistyped.insert("quantity".into(), Av::Bool(false));
    let error = batch
        .accept(request.decode(Ok(output(vec![mistyped, item(1), missing], vec![]))))
        .unwrap_err();
    let BatchGetFailure::Decode { failures } = &error.kind else {
        panic!("decode failure")
    };
    assert_eq!(
        failures
            .iter()
            .map(|(position, _)| *position)
            .collect::<Vec<_>>(),
        vec![2, 0]
    );
    for (_, source) in failures {
        assert_eq!(source.record, "Order");
        assert_eq!(
            source.path.segments(),
            &[tabulary::PathSegment::Attribute("quantity".into())]
        );
    }
    assert!(matches!(
        failures[0].1.kind,
        tabulary::DecodeFault::WrongType { .. }
    ));
    assert_eq!(failures[1].1.kind, tabulary::DecodeFault::Missing);
    assert!(matches!(
        error.partial.entries[0].state,
        BatchGetState::Pending
    ));
    assert!(
        matches!(&error.partial.entries[1].state, BatchGetState::Found(record) if record.quantity == 7)
    );
    assert!(matches!(
        error.partial.entries[2].state,
        BatchGetState::Pending
    ));
    assert_eq!(error.partial.calls_issued, 1);
    assert_eq!(error.partial.capacity.len(), 1);
    assert_eq!(batch.next_request().unwrap_err(), State::Frozen);
}

#[test]
fn later_round_refuses_an_earlier_key_and_keeps_prior_progress() {
    let mut batch = batch(3);
    let first = batch.next_request().unwrap().unwrap();
    let pending = first.input().request_items.as_ref().unwrap()["orders"].keys[2].clone();
    batch
        .accept(first.decode(Ok(output(vec![item(0)], vec![pending]))))
        .unwrap();
    let second = batch.next_request().unwrap().unwrap();
    let error = batch
        .accept(second.decode(Ok(output(vec![item(0)], vec![]))))
        .unwrap_err();
    assert!(matches!(
        error.kind,
        BatchGetFailure::State(State::InvalidResponse)
    ));
    assert!(
        matches!(&error.partial.entries[0].state, BatchGetState::Found(record) if record.quantity == 7)
    );
    assert!(matches!(
        error.partial.entries[1].state,
        BatchGetState::Missing
    ));
    assert!(matches!(
        error.partial.entries[2].state,
        BatchGetState::Pending
    ));
    assert_eq!(error.partial.calls_issued, 2);
    assert_eq!(error.partial.capacity.len(), 2);
    assert_eq!(batch.next_request().unwrap_err(), State::Frozen);
}

#[test]
fn unprocessed_numeric_keys_correlate_by_canonical_value() {
    let mut batch = batch(2);
    let first = batch.next_request().unwrap().unwrap();
    let mut pending = first.input().request_items.as_ref().unwrap()["orders"].keys[2].clone();
    pending.insert("line".into(), Av::N("2.0".into()));
    batch
        .accept(first.decode(Ok(output(vec![item(0)], vec![pending]))))
        .unwrap();
    let second = batch.next_request().unwrap().unwrap();
    let keys = &second.input().request_items.as_ref().unwrap()["orders"].keys;
    assert_eq!(keys.len(), 1);
    assert_eq!(keys[0]["line"], Av::N("2".into()));
    batch
        .accept(second.decode(Ok(output(vec![item(2)], vec![]))))
        .unwrap();
    let records = batch.finish().unwrap().records;
    assert!(records[0].is_some());
    assert!(records[1].is_none());
    assert!(records[2].is_some());
}

#[test]
fn foreign_table_only_in_unprocessed_keys_is_a_protocol_fault() {
    let mut batch = batch(2);
    let request = batch.next_request().unwrap().unwrap();
    let mut raw = output(vec![], vec![]);
    let pending = request.input().request_items.as_ref().unwrap()["orders"].clone();
    raw.unprocessed_keys
        .as_mut()
        .unwrap()
        .insert("foreign".into(), pending);
    let error = batch.accept(request.decode(Ok(raw))).unwrap_err();
    assert!(matches!(
        error.kind,
        BatchGetFailure::State(State::InvalidResponse)
    ));
    assert!(
        error
            .partial
            .entries
            .iter()
            .all(|entry| matches!(entry.state, BatchGetState::Pending))
    );
    assert_eq!(batch.next_request().unwrap_err(), State::Frozen);
}

#[test]
fn eventual_consistency_is_explicit_on_each_round() {
    let mut batch = Table::<Order>::new("orders")
        .unwrap()
        .batch_get(
            vec![OrderKey::new("order-lime".into(), 0).unwrap()],
            ReadConsistency::Eventual,
            BatchBudget::new(2).unwrap(),
        )
        .unwrap();
    for pending in [true, false] {
        let request = batch.next_request().unwrap().unwrap();
        let attributes = &request.input().request_items.as_ref().unwrap()["orders"];
        assert_eq!(attributes.consistent_read, Some(false));
        let keys = if pending {
            attributes.keys.clone()
        } else {
            vec![]
        };
        batch
            .accept(request.decode(Ok(output(vec![], keys))))
            .unwrap();
    }
    assert!(batch.finish().unwrap().records[0].is_none());
}
