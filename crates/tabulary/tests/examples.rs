#![cfg(feature = "derive")]
//! Execute the unchanged public examples with deterministic SDK responses, without I/O.
use std::{
    collections::VecDeque,
    num::NonZeroU32,
    sync::{Arc, Mutex},
};

use aws_sdk_dynamodb::{
    Client,
    config::{BehaviorVersion, Credentials, Region, retry::RetryConfig},
    types::AttributeValue as Av,
};
use aws_smithy_runtime_api::client::{
    http::{HttpConnector, HttpConnectorFuture, SharedHttpConnector, http_client_fn},
    orchestrator::{HttpRequest, HttpResponse},
};
use aws_smithy_types::body::SdkBody;
use serde_json::{Value as Json, json};
use tabulary::{FailureKind, OldItem};

#[path = "../../../docs/examples/06_query_batch.rs"]
mod batch;
#[path = "../../../docs/examples/02_composite_sparse.rs"]
mod composite;
#[path = "../../../docs/examples/04_conditional_update.rs"]
mod conditional;
#[path = "../../../docs/examples/01_simple.rs"]
mod simple;
#[path = "../../../docs/examples/03_status_record.rs"]
mod status;
#[path = "../../../docs/examples/05_transaction.rs"]
mod transaction;
mod values {
    include!("../../../docs/examples/07_values_projection_scan.rs");

    // Inspect the example's private view field without changing its public API.
    pub(super) fn quantity(view: &QuantityView) -> u64 {
        view.quantity
    }
}

#[derive(Debug)]
struct Step {
    operation: &'static str,
    request_fragments: &'static [&'static str],
    status: u16,
    response: String,
}
fn step(
    operation: &'static str,
    request_fragments: &'static [&'static str],
    response: impl Into<String>,
) -> Step {
    Step {
        operation,
        request_fragments,
        status: 200,
        response: response.into(),
    }
}
#[derive(Clone, Debug)]
struct Replay {
    steps: Arc<Mutex<VecDeque<Step>>>,
    requests: Arc<Mutex<Vec<Json>>>,
}
impl HttpConnector for Replay {
    fn call(&self, request: HttpRequest) -> HttpConnectorFuture {
        let step = self
            .steps
            .lock()
            .unwrap()
            .pop_front()
            .expect("unexpected SDK call");
        assert_eq!(
            request.headers().get("x-amz-target"),
            Some(format!("DynamoDB_20120810.{}", step.operation).as_str())
        );
        let body = std::str::from_utf8(request.body().bytes().unwrap()).unwrap();
        self.requests
            .lock()
            .unwrap()
            .push(serde_json::from_str(body).unwrap());
        for fragment in step.request_fragments {
            assert!(body.contains(fragment), "missing {fragment} in {body}");
        }
        let mut response = HttpResponse::new(
            step.status.try_into().unwrap(),
            SdkBody::from(step.response),
        );
        response
            .headers_mut()
            .insert("content-type", "application/x-amz-json-1.0");
        HttpConnectorFuture::new(async move { Ok(response) })
    }
}
impl Replay {
    fn client(steps: Vec<Step>) -> (Client, Self) {
        let replay = Self {
            steps: Arc::new(Mutex::new(steps.into())),
            requests: Arc::new(Mutex::new(Vec::new())),
        };
        let connector = SharedHttpConnector::new(replay.clone());
        let config = aws_sdk_dynamodb::Config::builder()
            .behavior_version(BehaviorVersion::latest())
            .region(Region::new("us-east-1"))
            .credentials_provider(Credentials::new("dummy", "dummy", None, None, "fixture"))
            .retry_config(RetryConfig::disabled())
            .http_client(http_client_fn(move |_, _| connector.clone()))
            .build();
        (Client::from_conf(config), replay)
    }
    fn done(&self) {
        assert!(
            self.steps.lock().unwrap().is_empty(),
            "example omitted an expected call"
        );
    }
}
fn line(number: u64) -> String {
    format!(
        r#"{{"order_ref":{{"S":"ord-amber"}},"line_no":{{"N":"{number}"}},"quantity":{{"N":"3"}}}}"#
    )
}

#[tokio::test]
async fn simple_round_trip_and_conditional_create_then_strong_read() {
    simple::round_trip().unwrap();
    let (client, replay) = Replay::client(vec![
        step(
            "PutItem",
            &["attribute_not_exists", "\"TableName\":\"orders\""],
            "{}",
        ),
        step(
            "GetItem",
            &["\"ConsistentRead\":true"],
            format!(r#"{{"Item":{}}}"#, line(1)),
        ),
    ]);
    let order = simple::create_and_read(&client)
        .await
        .unwrap()
        .unwrap()
        .to_item()
        .unwrap();
    assert_eq!(order["quantity"], Av::N("3".into()));
    assert_eq!(order["order_ref"], Av::S("ord-amber".into()));
    replay.done();
}

#[tokio::test]
async fn composite_mapping_definition_and_one_courier_page() {
    composite::mapping_and_definition().unwrap();
    let parcel = r#"{"parcelKey":{"S":"parcel!73686f702d6379616e!5d5d5d5d5d5d5d5d5d5d5d5d5d5d5d5d"},"shopCode":{"S":"shop-cyan"},"parcelToken":{"B":"XV1dXV1dXV1dXV1dXV1dXQ=="},"parcelNo":{"N":"8"},"assignedCourier":{"S":"courier-lime"}}"#;
    let (client, replay) = Replay::client(vec![step(
        "Query",
        &[
            "\"IndexName\":\"by_courier\"",
            "\"ConsistentRead\":false",
            "courier-lime",
        ],
        format!(r#"{{"Items":[{parcel}],"Count":1,"ScannedCount":1}}"#),
    )]);
    let page = composite::courier_page(&client).await.unwrap();
    assert_eq!(page.len(), 1);
    assert_eq!(page[0].to_item().unwrap()["parcelNo"], Av::N("8".into()));
    replay.done();
}

#[tokio::test]
async fn status_waiting_shape_and_guarded_transition_cleanup() {
    let item = status::waiting_item().unwrap();
    assert_eq!(item["deliveryStage"], Av::S("waiting".into()));
    assert_eq!(item["queued_on"], Av::N("9000000".into()));
    let (client, replay) = Replay::client(vec![step("UpdateItem", &[], "{}")]);
    assert!(status::mark_sent(&client).await.is_ok());
    assert_eq!(
        replay.requests.lock().unwrap()[0],
        json!({
            "TableName": "deliveries",
            "Key": {"delivery_ref": {"S": "del-ochre"}},
            "UpdateExpression": "SET #n0 = :v0, #n1 = :v1 REMOVE #n2, #n3",
            "ConditionExpression": "(#n0 = :v2 AND #n0 = :v3)",
            "ExpressionAttributeNames": {
                "#n0": "deliveryStage", "#n1": "tracking_ref",
                "#n2": "failure_code", "#n3": "queued_on"
            },
            "ExpressionAttributeValues": {
                ":v0": {"S": "sent"}, ":v1": {"S": "track-jade"},
                ":v2": {"S": "waiting"}, ":v3": {"S": "waiting"}
            },
            "ReturnValues": "NONE", "ReturnItemCollectionMetrics": "NONE",
            "ReturnValuesOnConditionCheckFailure": "ALL_OLD"
        })
    );
    replay.done();
}

#[tokio::test]
async fn conditional_update_returns_new_record_or_typed_old_record() {
    let invoice = r#"{"invoice_ref":{"S":"inv-coral"},"balance_due":{"N":"7"},"paid":{"BOOL":false},"edit_no":{"N":"2"}}"#;
    let mut refused = step(
        "UpdateItem",
        &["ALL_NEW", "ALL_OLD", "edit_no", "balance_due"],
        format!(r#"{{"__type":"ConditionalCheckFailedException","Item":{invoice}}}"#),
    );
    refused.status = 400;
    let (client, replay) = Replay::client(vec![
        step(
            "UpdateItem",
            &["ALL_NEW", "TOTAL"],
            format!(r#"{{"Attributes":{invoice}}}"#),
        ),
        refused,
    ]);
    let conditional::PaymentAttempt::Applied(new) =
        conditional::apply_payment(&client, 1).await.unwrap()
    else {
        panic!("expected applied payment")
    };
    assert_eq!(new.to_item().unwrap()["edit_no"], Av::N("2".into()));
    let conditional::PaymentAttempt::Refused(failure) =
        conditional::apply_payment(&client, 1).await.unwrap()
    else {
        panic!("expected refusal")
    };
    assert_eq!(failure.kind, FailureKind::ConditionFailed);
    let OldItem::Decoded(old) = failure.old else {
        panic!("expected old invoice")
    };
    assert_eq!(old.to_item().unwrap()["balance_due"], Av::N("7".into()));
    replay.done();
}

#[tokio::test]
async fn transaction_example_preserves_commit_and_typed_cancellation_results() {
    let mut refused = step(
        "TransactWriteItems",
        &[
            "\"ClientRequestToken\":\"checkout-two\"",
            "inventory",
            "sessions",
        ],
        r#"{"__type":"TransactionCanceledException","CancellationReasons":[{"Code":"None"},{"Code":"ConditionalCheckFailed","Item":{"product_ref":{"S":"product-violet"},"on_hand":{"N":"0"}}},{"Code":"None"}]}"#,
    );
    refused.status = 400;
    let (client, replay) = Replay::client(vec![
        step(
            "TransactWriteItems",
            &["\"ClientRequestToken\":\"checkout-one\"", "invoices"],
            "{}",
        ),
        refused,
    ]);
    assert!(matches!(
        transaction::checkout(&client, "checkout-one".into())
            .await
            .unwrap(),
        transaction::CheckoutAttempt::Committed
    ));
    let transaction::CheckoutAttempt::Cancelled(cancelled) =
        transaction::checkout(&client, "checkout-two".into())
            .await
            .unwrap()
    else {
        panic!("expected cancelled checkout")
    };
    assert_eq!(cancelled.invoice.kind, FailureKind::NoFailureReported);
    assert_eq!(cancelled.stock.kind, FailureKind::ConditionFailed);
    assert_eq!(cancelled.session.kind, FailureKind::NoFailureReported);
    let OldItem::Decoded(stock) = cancelled.stock.old else {
        panic!("expected old stock")
    };
    assert_eq!(stock.to_item().unwrap()["on_hand"], Av::N("0".into()));
    replay.done();
}

#[tokio::test]
async fn query_example_resumes_empty_pages_and_enforces_its_page_budget() {
    let key = r#"{"order_ref":{"S":"ord-amber"},"line_no":{"N":"1"}}"#;
    let continuing =
        format!(r#"{{"Items":[],"Count":0,"ScannedCount":1,"LastEvaluatedKey":{key}}}"#);
    let (client, replay) = Replay::client(vec![
        step(
            "Query",
            &["\"ConsistentRead\":true", "\"Limit\":2"],
            continuing.clone(),
        ),
        step(
            "Query",
            &["ExclusiveStartKey"],
            format!(r#"{{"Items":[{}],"Count":1,"ScannedCount":1}}"#, line(4)),
        ),
        step("Query", &[], continuing),
    ]);
    let lines = batch::read_lines(&client, NonZeroU32::new(2).unwrap())
        .await
        .unwrap();
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0].to_item().unwrap()["line_no"], Av::N("4".into()));
    let exhausted = batch::read_lines(&client, NonZeroU32::new(1).unwrap())
        .await
        .err()
        .unwrap();
    assert_eq!(
        exhausted.downcast::<std::io::Error>().unwrap().kind(),
        std::io::ErrorKind::Other
    );
    let expected = json!({
        "TableName": "orders", "ConsistentRead": true, "ScanIndexForward": true,
        "Limit": 2, "ReturnConsumedCapacity": "TOTAL",
        "KeyConditionExpression": "(#n0 = :v0 AND #n1 BETWEEN :v1 AND :v2)",
        "ExpressionAttributeNames": {"#n0": "order_ref", "#n1": "line_no"},
        "ExpressionAttributeValues": {
            ":v0": {"S": "ord-amber"}, ":v1": {"N": "1"}, ":v2": {"N": "9"}
        }
    });
    let requests = replay.requests.lock().unwrap();
    assert_eq!(requests[0], expected);
    let mut resumed = expected.clone();
    resumed["ExclusiveStartKey"] = json!({"order_ref": {"S": "ord-amber"}, "line_no": {"N": "1"}});
    assert_eq!(requests[1], resumed);
    assert_eq!(requests[2], expected);
    replay.done();
}

#[tokio::test]
async fn batch_example_explicitly_submits_pending_keys_and_retains_input_order() {
    let (client, replay) = Replay::client(vec![
        step(
            "BatchGetItem",
            &["\"ConsistentRead\":true"],
            format!(
                r#"{{"Responses":{{"orders":[{}]}},"UnprocessedKeys":{{"orders":{{"Keys":[{{"order_ref":{{"S":"ord-amber"}},"line_no":{{"N":"1"}}}}]}}}}}}"#,
                line(7)
            ),
        ),
        step(
            "BatchGetItem",
            &["\"N\":\"1\""],
            format!(r#"{{"Responses":{{"orders":[{}]}}}}"#, line(1)),
        ),
    ]);
    let rows = batch::read_selected(&client).await.unwrap();
    assert_eq!(rows.len(), 3);
    assert_eq!(
        rows[0].as_ref().unwrap().to_item().unwrap()["line_no"],
        Av::N("1".into())
    );
    assert!(rows[1].is_none());
    assert_eq!(
        rows[2].as_ref().unwrap().to_item().unwrap()["line_no"],
        Av::N("7".into())
    );
    let key = |number: &str| json!({"order_ref": {"S": "ord-amber"}, "line_no": {"N": number}});
    assert_eq!(
        *replay.requests.lock().unwrap(),
        vec![
            json!({"RequestItems": {"orders": {"ConsistentRead": true, "Keys": [key("1"), key("4"), key("7")]}}, "ReturnConsumedCapacity": "TOTAL"}),
            json!({"RequestItems": {"orders": {"ConsistentRead": true, "Keys": [key("1")]}}, "ReturnConsumedCapacity": "TOTAL"})
        ]
    );
    replay.done();
}

#[tokio::test]
async fn values_example_round_trips_and_decodes_a_projected_scan_segment() {
    let item = values::envelope().unwrap().to_item().unwrap();
    assert_eq!(
        values::InvoiceEnvelope::from_item(&item)
            .unwrap()
            .to_item()
            .unwrap(),
        item
    );
    let (client, replay) = Replay::client(vec![step(
        "Scan",
        &[
            "\"Segment\":1",
            "\"TotalSegments\":3",
            "\"Limit\":5",
            "quantity",
            "\"ConsistentRead\":true",
        ],
        r#"{"Items":[{"quantity":{"N":"2"}}],"Count":1,"ScannedCount":1}"#,
    )]);
    let views = values::projected_segment(&client).await.unwrap();
    assert_eq!(views.len(), 1);
    assert_eq!(values::quantity(&views[0]), 2);
    replay.done();
}

#[test]
fn nested_changes_render_exactly_with_physical_names_and_values() {
    let rendered = values::nested_changes().build().unwrap().render();
    assert_eq!(
        rendered.update(),
        Some("SET #n0.#n1 = :v0, #n2 = list_append(#n2, :v1) REMOVE #n3")
    );
    assert_eq!(
        rendered.names(),
        &[
            ("#n0".into(), "postal".into()),
            ("#n1".into(), "destination_city".into()),
            ("#n2".into(), "notes".into()),
            ("#n3".into(), "optional_note".into())
        ]
        .into()
    );
    assert_eq!(
        rendered.values(),
        &[
            (":v0".into(), Av::S("Otherburg".into())),
            (":v1".into(), Av::L(vec![Av::S("include a card".into())]))
        ]
        .into()
    );
}
