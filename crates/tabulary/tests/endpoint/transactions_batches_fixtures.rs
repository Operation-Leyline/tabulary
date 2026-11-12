//! Fixture-only capabilities: partial batches, throttles and transaction conflicts.
//! Scripted HTTP responses exercise real SDK sends without an endpoint or retries.
use super::*;
use aws_sdk_dynamodb::{
    Config,
    config::{Credentials, Region, retry::RetryConfig},
};
use aws_smithy_runtime_api::{
    client::{
        http::{HttpConnector, HttpConnectorFuture, SharedHttpConnector, http_client_fn},
        orchestrator::{HttpRequest, HttpResponse},
    },
    http::StatusCode,
};
use aws_smithy_types::body::SdkBody;
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
    time::Duration,
};
use tabulary::{
    BatchGetFailure, BatchGetState, BatchWriteErrorKind,
    BatchWriteState::{Indeterminate, Pending, Processed},
    Capacity,
};

#[derive(Clone, Debug)]
struct Script {
    responses: Arc<Mutex<VecDeque<(u16, &'static str)>>>,
    calls: Arc<Mutex<Vec<HttpRequest>>>,
}
impl HttpConnector for Script {
    fn call(&self, request: HttpRequest) -> HttpConnectorFuture {
        self.calls.lock().unwrap().push(request);
        let (status, body) = self
            .responses
            .lock()
            .unwrap()
            .pop_front()
            .expect("unexpected SDK call");
        let mut response =
            HttpResponse::new(StatusCode::try_from(status).unwrap(), SdkBody::from(body));
        response
            .headers_mut()
            .insert("content-type", "application/x-amz-json-1.0");
        HttpConnectorFuture::ready(Ok(response))
    }
}
impl Script {
    fn new(responses: Vec<(u16, &'static str)>) -> (Client, Self) {
        let script = Self {
            responses: Arc::new(Mutex::new(responses.into())),
            calls: Arc::default(),
        };
        let connector = SharedHttpConnector::new(script.clone());
        let client = Client::from_conf(
            Config::builder()
                .behavior_version_latest()
                .region(Region::new("us-east-1"))
                .credentials_provider(Credentials::new("dummy", "dummy", None, None, "fixture"))
                .retry_config(RetryConfig::disabled())
                .timeout_config(
                    aws_sdk_dynamodb::config::timeout::TimeoutConfig::builder()
                        .operation_timeout(Duration::from_secs(5))
                        .build(),
                )
                .http_client(http_client_fn(move |_, _| connector.clone()))
                .build(),
        );
        (client, script)
    }
    fn count(&self, expected: usize, target: &str) {
        let calls = self.calls.lock().unwrap();
        assert_eq!(calls.len(), expected);
        assert!(
            calls
                .iter()
                .all(|call| call.headers().get("x-amz-target") == Some(target))
        );
    }
}
const GET: &str = "DynamoDB_20120810.BatchGetItem";
const WRITE: &str = "DynamoDB_20120810.BatchWriteItem";
const PARTIAL_GET: &str = r#"{"Responses":{"orders":[{"id":{"S":"a"},"quantity":{"N":"1"}},{"id":{"S":"c"},"quantity":{"N":"3"}}]},"UnprocessedKeys":{"orders":{"Keys":[{"id":{"S":"b"}}]}},"ConsumedCapacity":[{"TableName":"orders","CapacityUnits":1}]}"#;
const FINISH_GET: &str = r#"{"Responses":{"orders":[{"id":{"S":"b"},"quantity":{"N":"2"}}]},"ConsumedCapacity":[{"TableName":"orders","CapacityUnits":2}]}"#;
const PARTIAL_WRITE: &str = r#"{"UnprocessedItems":{"orders":[{"PutRequest":{"Item":{"id":{"S":"c"},"quantity":{"N":"3"}}}},{"PutRequest":{"Item":{"id":{"S":"a"},"quantity":{"N":"1"}}}}]},"ConsumedCapacity":[{"TableName":"orders","CapacityUnits":1}]}"#;
const FINISH_WRITE: &str =
    r#"{"UnprocessedItems":{},"ConsumedCapacity":[{"TableName":"orders","CapacityUnits":2}]}"#;
const THROTTLE: &str = r#"{"__type":"com.amazonaws.dynamodb.v20120810#ProvisionedThroughputExceededException","message":"opaque"}"#;

fn get_batch(budget: u32) -> tabulary::BatchGet<Order, OrderKey> {
    Table::<Order>::new("orders")
        .unwrap()
        .batch_get(
            vec![key("c"), key("missing"), key("a"), key("b")],
            ReadConsistency::Strong,
            BatchBudget::new(budget).unwrap(),
        )
        .unwrap()
}
fn write_batch(budget: u32) -> tabulary::BatchWriteBatch<Order> {
    Table::<Order>::new("orders")
        .unwrap()
        .batch_write(
            vec![
                BatchAction::put(&order("a", 1)).unwrap(),
                BatchAction::delete(key("b")).unwrap(),
                BatchAction::put(&order("c", 3)).unwrap(),
            ],
            BatchBudget::new(budget).unwrap(),
        )
        .unwrap()
}

#[tokio::test]
async fn partial_get_requires_another_explicit_send_and_preserves_order_or_exhaustion() {
    for budget in [1, 2] {
        let (client, script) = Script::new(vec![(200, PARTIAL_GET), (200, FINISH_GET)]);
        let mut batch = get_batch(budget);
        batch.set_capacity(Capacity::Total).unwrap();
        let first = batch.next_request().unwrap().unwrap();
        let applied = first.apply(client.batch_get_item());
        script.count(0, GET);
        batch.accept(first.decode(applied.send().await)).unwrap();
        script.count(1, GET);
        let second = batch.next_request().unwrap();
        script.count(1, GET);
        if budget == 1 {
            assert!(second.is_none());
            let error = batch.finish().unwrap_err();
            assert!(matches!(
                error.kind,
                BatchGetFailure::State(BatchStateError::BudgetExhausted)
            ));
            assert_eq!(error.partial.calls_issued, 1);
            assert!(
                matches!(&error.partial.entries[0].state, BatchGetState::Found(value) if *value == order("c", 3))
            );
            assert!(matches!(
                error.partial.entries[1].state,
                BatchGetState::Missing
            ));
            assert!(
                matches!(&error.partial.entries[2].state, BatchGetState::Found(value) if *value == order("a", 1))
            );
            assert!(matches!(
                error.partial.entries[3].state,
                BatchGetState::Pending
            ));
            assert_eq!(error.partial.capacity[0][0].capacity_units(), Some(1.0));
        } else {
            let second = second.unwrap();
            assert_eq!(
                second.input().request_items.as_ref().unwrap()["orders"].keys,
                vec![order("b", 2).key().unwrap().encode_key().unwrap()]
            );
            batch
                .accept(second.decode(second.apply(client.batch_get_item()).send().await))
                .unwrap();
            script.count(2, GET);
            assert!(batch.next_request().unwrap().is_none());
            let complete = batch.finish().unwrap();
            assert_eq!(
                complete.records,
                vec![
                    Some(order("c", 3)),
                    None,
                    Some(order("a", 1)),
                    Some(order("b", 2))
                ]
            );
            assert_eq!(
                complete
                    .capacity
                    .iter()
                    .map(|v| v[0].capacity_units())
                    .collect::<Vec<_>>(),
                vec![Some(1.0), Some(2.0)]
            );
        }
    }
}

#[tokio::test]
async fn partial_write_requires_another_explicit_send_and_preserves_order_or_exhaustion() {
    for budget in [1, 2] {
        let (client, script) = Script::new(vec![(200, PARTIAL_WRITE), (200, FINISH_WRITE)]);
        let mut batch = write_batch(budget);
        batch.set_capacity(Capacity::Total).unwrap();
        let first = batch.next_request().unwrap().unwrap();
        let issued = first.input().request_items.as_ref().unwrap()["orders"].clone();
        let applied = first.apply(client.batch_write_item());
        script.count(0, WRITE);
        batch.accept(first.decode(applied.send().await)).unwrap();
        script.count(1, WRITE);
        let second = batch.next_request().unwrap();
        script.count(1, WRITE);
        if budget == 1 {
            assert!(second.is_none());
            let error = batch.finish().unwrap_err();
            assert!(matches!(
                error.kind,
                BatchWriteErrorKind::State(BatchStateError::BudgetExhausted)
            ));
            assert_eq!(error.partial.calls_issued, 1);
            assert_eq!(
                error
                    .partial
                    .entries
                    .iter()
                    .map(|e| e.state)
                    .collect::<Vec<_>>(),
                vec![Pending, Processed, Pending]
            );
            assert_eq!(
                error.capacity[0].as_ref().unwrap()[0].capacity_units(),
                Some(1.0)
            );
        } else {
            let second = second.unwrap();
            assert_eq!(second.positions(), &[0, 2]);
            assert_eq!(
                second.input().request_items.as_ref().unwrap()["orders"],
                vec![issued[0].clone(), issued[2].clone()]
            );
            let raw = second.apply(client.batch_write_item()).send().await;
            batch.accept(second.decode(raw)).unwrap();
            script.count(2, WRITE);
            assert!(batch.next_request().unwrap().is_none());
            let complete = batch.finish().unwrap();
            assert_eq!(
                complete
                    .actions
                    .iter()
                    .map(|a| a.key().clone())
                    .collect::<Vec<_>>(),
                ["a", "b", "c"].map(|id| key(id).encode_key().unwrap())
            );
            assert_eq!(
                complete
                    .capacity
                    .iter()
                    .map(|v| v.as_ref().unwrap()[0].capacity_units())
                    .collect::<Vec<_>>(),
                vec![Some(1.0), Some(2.0)]
            );
        }
    }
}

#[tokio::test]
async fn throttled_get_keeps_prior_found_missing_and_pending_without_retry() {
    let (client, script) = Script::new(vec![(200, PARTIAL_GET), (400, THROTTLE)]);
    let mut batch = get_batch(3);
    let first = batch.next_request().unwrap().unwrap();
    batch
        .accept(first.decode(first.apply(client.batch_get_item()).send().await))
        .unwrap();
    let second = batch.next_request().unwrap().unwrap();
    let error = batch
        .accept(second.decode(second.apply(client.batch_get_item()).send().await))
        .unwrap_err();
    let BatchGetFailure::Sdk(source) = error.kind else {
        panic!("expected SDK error")
    };
    assert!(
        source
            .as_service_error()
            .unwrap()
            .is_provisioned_throughput_exceeded_exception()
    );
    assert_eq!(error.partial.calls_issued, 2);
    assert!(
        matches!(&error.partial.entries[0].state, BatchGetState::Found(value) if *value == order("c", 3))
    );
    assert!(matches!(
        error.partial.entries[1].state,
        BatchGetState::Missing
    ));
    assert!(
        matches!(&error.partial.entries[2].state, BatchGetState::Found(value) if *value == order("a", 1))
    );
    assert!(matches!(
        error.partial.entries[3].state,
        BatchGetState::Pending
    ));
    assert_eq!(batch.next_request().unwrap_err(), BatchStateError::Frozen);
    script.count(2, GET);
}

#[tokio::test]
async fn throttled_write_keeps_processed_and_freezes_indeterminate_actions_without_retry() {
    let (client, script) = Script::new(vec![(200, PARTIAL_WRITE), (400, THROTTLE)]);
    let mut batch = write_batch(3);
    let first = batch.next_request().unwrap().unwrap();
    let raw = first.apply(client.batch_write_item()).send().await;
    batch.accept(first.decode(raw)).unwrap();
    let second = batch.next_request().unwrap().unwrap();
    let raw = second.apply(client.batch_write_item()).send().await;
    let error = batch.accept(second.decode(raw)).unwrap_err();
    assert!(
        error
            .sdk_error()
            .unwrap()
            .as_service_error()
            .unwrap()
            .is_provisioned_throughput_exceeded_exception()
    );
    assert_eq!(error.partial.calls_issued, 2);
    assert_eq!(
        error
            .partial
            .entries
            .iter()
            .map(|e| e.state)
            .collect::<Vec<_>>(),
        vec![Indeterminate, Processed, Indeterminate]
    );
    assert!(error.capacity[1].is_none());
    assert_eq!(
        batch.next_request().unwrap_err(),
        BatchStateError::Indeterminate
    );
    script.count(2, WRITE);
}

#[tokio::test]
async fn transaction_conflict_and_throttle_reasons_remain_in_request_order_without_retry() {
    let (client, script) = Script::new(vec![(
        400,
        r#"{"__type":"com.amazonaws.dynamodb.v20120810#TransactionCanceledException","message":"opaque","CancellationReasons":[{"Code":"TransactionConflict"},{"Code":"ThrottlingError"},{"Code":"None"}]}"#,
    )]);
    let request = writes("orders", None);
    let TxError::Cancelled(cancelled) = request
        .decode(request.apply(client.transact_write_items()).send().await)
        .unwrap_err()
    else {
        panic!("expected cancellation")
    };
    assert_eq!(cancelled.completeness, ReasonCompleteness::Complete);
    assert_eq!(cancelled.reasons.0.kind, FailureKind::TransactionConflict);
    assert_eq!(cancelled.reasons.1.kind, FailureKind::Throttled);
    assert_eq!(cancelled.reasons.2.kind, FailureKind::NoFailureReported);
    assert!(matches!(cancelled.reasons.0.old, OldItem::Unavailable));
    script.count(1, "DynamoDB_20120810.TransactWriteItems");
}
