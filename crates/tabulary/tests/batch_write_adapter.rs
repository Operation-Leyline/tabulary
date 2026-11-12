#![cfg(feature = "derive")]

use aws_sdk_dynamodb::{
    Client, Config,
    error::SdkError,
    operation::batch_write_item::BatchWriteItemOutput as Output,
    types::{AttributeValue as Av, Capacity as SdkCapacity, ConsumedCapacity, WriteRequest},
};
use tabulary::{
    BatchAction, BatchBudget, BatchStateError, BatchWriteAbort, BatchWriteBatch,
    BatchWriteErrorKind,
    BatchWriteState::{Indeterminate, Pending, Processed},
    BuildError, Capacity, Table,
};

// No Clone, Debug or Eq bounds on records.
#[derive(tabulary::Record)]
#[tabulary(rename_all = "snake_case")]
struct Order {
    #[tabulary(partition_key)]
    id: String,
    payload: String,
}
fn table() -> Table<Order> {
    Table::new("orders").unwrap()
}
fn actions(n: usize) -> Vec<BatchAction<Order>> {
    (0..n)
        .map(|i| {
            if i % 2 == 0 {
                BatchAction::put(&Order {
                    id: i.to_string(),
                    payload: "private-payload".into(),
                })
                .unwrap()
            } else {
                BatchAction::delete(OrderKey::new(i.to_string()).unwrap()).unwrap()
            }
        })
        .collect()
}
fn batch(n: usize, budget: u32) -> BatchWriteBatch<Order> {
    table()
        .batch_write(actions(n), BatchBudget::new(budget).unwrap())
        .unwrap()
}
fn output(pending: Vec<WriteRequest>, capacity: Option<Vec<ConsumedCapacity>>) -> Output {
    Output::builder()
        .unprocessed_items("orders", pending)
        .set_consumed_capacity(capacity)
        .build()
}
fn capacity() -> Vec<ConsumedCapacity> {
    vec![
        ConsumedCapacity::builder()
            .table_name("orders")
            .capacity_units(2.5)
            .table(SdkCapacity::builder().write_capacity_units(2.0).build())
            .global_secondary_indexes(
                "by_route",
                SdkCapacity::builder().capacity_units(0.5).build(),
            )
            .build(),
        ConsumedCapacity::builder().table_name("orders").build(),
    ]
}
fn states(error: &tabulary::BatchWriteError<Order>) -> Vec<tabulary::BatchWriteState> {
    error.partial.entries.iter().map(|e| e.state).collect()
}

// Manual encoding preserves valid noncanonical number spellings in the request.
struct NumericOrder(tabulary::Item);
impl tabulary::Record for NumericOrder {
    const SCHEMA: tabulary::RecordSchema = <Order as tabulary::Record>::SCHEMA;
}
impl tabulary::Document for NumericOrder {
    const NAME: &'static str = "Order";
    fn encode_map(
        &self,
        _: &mut tabulary::EncodeContext,
    ) -> Result<tabulary::Item, tabulary::EncodeError> {
        Ok(self.0.clone())
    }
    fn decode_map(
        item: &tabulary::Item,
        _: &mut tabulary::DecodeContext,
    ) -> Result<Self, tabulary::DecodeError> {
        Ok(Self(item.clone()))
    }
}

#[test]
fn echoed_string_and_binary_sets_ignore_order_but_reject_changed_members() {
    for changed in 0..3 {
        let sets = vec![
            Av::Ss(vec!["a".into(), "b".into()]),
            Av::Bs(vec![vec![1].into(), vec![2].into()]),
        ];
        let row = NumericOrder(
            [
                ("id".into(), Av::S("order".into())),
                ("sets".into(), Av::L(sets)),
            ]
            .into(),
        );
        let mut batch = Table::<NumericOrder>::new("orders")
            .unwrap()
            .batch_write(
                vec![BatchAction::put(&row).unwrap()],
                BatchBudget::new(2).unwrap(),
            )
            .unwrap();
        let request = batch.next_request().unwrap().unwrap();
        let mut write = request.input().request_items.as_ref().unwrap()["orders"][0].clone();
        write.put_request.as_mut().unwrap().item.insert(
            "sets".into(),
            Av::L(vec![
                Av::Ss(vec![
                    "b".into(),
                    if changed == 1 { "b" } else { "a" }.into(),
                ]),
                Av::Bs(vec![
                    vec![2].into(),
                    vec![if changed == 2 { 2 } else { 1 }].into(),
                ]),
            ]),
        );
        let result = batch.accept(request.decode(Ok(output(vec![write], None))));
        if changed != 0 {
            assert!(result.is_err());
        } else {
            result.unwrap();
            assert_eq!(batch.next_request().unwrap().unwrap().positions(), &[0]);
        }
    }
}

#[test]
fn echoed_numbers_match_by_decimal_value_including_nested_number_sets() {
    for echoed in ["1e0", "different", "malformed", "duplicate"] {
        let item = [
            ("id".into(), Av::S("order".into())),
            ("amount".into(), Av::N("1.00".into())),
            (
                "payload".into(),
                Av::M(
                    [(
                        "values".into(),
                        Av::L(vec![Av::Ns(vec!["1.0".into(), "2.00".into()])]),
                    )]
                    .into(),
                ),
            ),
        ]
        .into();
        let mut b = Table::<NumericOrder>::new("orders")
            .unwrap()
            .batch_write(
                vec![BatchAction::put(&NumericOrder(item)).unwrap()],
                BatchBudget::new(2).unwrap(),
            )
            .unwrap();
        let r = b.next_request().unwrap().unwrap();
        let mut write = r.input().request_items.as_ref().unwrap()["orders"][0].clone();
        let item = &mut write.put_request.as_mut().unwrap().item;
        item.insert(
            "amount".into(),
            Av::N(
                match echoed {
                    "different" => "3",
                    "malformed" => "invalid",
                    _ => "1e0",
                }
                .into(),
            ),
        );
        let numbers = if echoed == "duplicate" {
            vec!["1".into(), "1.00".into()]
        } else {
            vec!["2e0".into(), "1e0".into()]
        };
        item.insert(
            "payload".into(),
            Av::M([("values".into(), Av::L(vec![Av::Ns(numbers)]))].into()),
        );
        let result = b.accept(r.decode(Ok(output(vec![write], None))));
        if echoed == "1e0" {
            result.unwrap();
            let next = b.next_request().unwrap().unwrap();
            assert_eq!(next.positions(), &[0]);
            assert_eq!(
                next.input().request_items.as_ref().unwrap()["orders"][0]
                    .put_request
                    .as_ref()
                    .unwrap()
                    .item["amount"],
                Av::N("1.00".into())
            );
            b.accept(next.decode(Ok(output(vec![], None)))).unwrap();
            assert!(b.finish().is_ok());
        } else {
            let error = result.unwrap_err();
            assert!(matches!(
                error.kind,
                BatchWriteErrorKind::State(BatchStateError::InvalidResponse)
            ));
            assert_eq!(error.partial.entries[0].state, Indeterminate);
        }
    }
}

#[test]
fn modeled_throttling_is_indeterminate_and_retains_the_sdk_source() {
    use aws_sdk_dynamodb::{
        operation::batch_write_item::BatchWriteItemError,
        types::error::ProvisionedThroughputExceededException,
    };
    use aws_smithy_runtime_api::{client::orchestrator::HttpResponse, http::StatusCode};
    use aws_smithy_types::body::SdkBody;
    let mut b = batch(2, 2);
    let r = b.next_request().unwrap().unwrap();
    let source = SdkError::service_error(
        BatchWriteItemError::ProvisionedThroughputExceededException(
            ProvisionedThroughputExceededException::builder()
                .message("private-throttle")
                .build(),
        ),
        HttpResponse::new(
            StatusCode::try_from(400).unwrap(),
            SdkBody::from("private-response"),
        ),
    );
    let error = b.accept(r.decode(Err(source))).unwrap_err();
    assert!(matches!(
        error.sdk_error().unwrap().as_service_error(),
        Some(BatchWriteItemError::ProvisionedThroughputExceededException(
            _
        ))
    ));
    assert_eq!(states(&error), vec![Indeterminate, Indeterminate]);
    assert!(matches!(
        b.next_request(),
        Err(BatchStateError::Indeterminate)
    ));
    assert_eq!(error.capacity, vec![None]);
    assert!(!format!("{error:?} {error}").contains("private-throttle"));
}

#[test]
fn unprocessed_subset_only_and_exact_capacity_vectors_by_round() {
    let mut b = batch(3, 3);
    b.set_capacity(Capacity::Indexes).unwrap();
    let first = b.next_request().unwrap().unwrap();
    assert_eq!(first.positions(), &[0, 1, 2]);
    assert!(matches!(
        b.next_request(),
        Err(BatchStateError::OutstandingRound)
    ));
    assert!(matches!(
        b.set_capacity(Capacity::Total),
        Err(BuildError::OperationConstraint {
            code: "already_started"
        })
    ));
    let writes = first.input().request_items.as_ref().unwrap()["orders"].clone();
    assert!(writes[0].put_request.is_some());
    assert!(writes[1].delete_request.is_some());
    b.accept(first.decode(Ok(output(
        vec![writes[2].clone(), writes[0].clone()],
        Some(capacity()),
    ))))
    .unwrap();
    let second = b.next_request().unwrap().unwrap();
    assert_eq!(second.positions(), &[0, 2]);
    assert_eq!(
        second.input().request_items.as_ref().unwrap()["orders"],
        vec![writes[0].clone(), writes[2].clone()]
    );
    b.accept(second.decode(Ok(output(vec![], Some(vec![])))))
        .unwrap();
    assert!(b.next_request().unwrap().is_none());
    let done = b.finish().unwrap();
    assert_eq!(done.actions.len(), 3);
    assert_eq!(done.capacity, vec![Some(capacity()), Some(vec![])]);
    assert_eq!(done.actions[1].key()["id"], Av::S("1".into()));
    let mut b = batch(1, 1);
    let r = b.next_request().unwrap().unwrap();
    b.accept(r.decode(Ok(Output::builder().build()))).unwrap();
    assert_eq!(b.finish().unwrap().capacity, vec![None]);
}

#[test]
fn exhausted_budget_contains_every_processed_and_pending_action() {
    let mut b = batch(3, 2);
    for n in [1, 2] {
        let r = b.next_request().unwrap().unwrap();
        let writes = &r.input().request_items.as_ref().unwrap()["orders"];
        let pending = if n == 1 {
            vec![writes[2].clone(), writes[0].clone()]
        } else {
            writes.clone()
        };
        b.accept(r.decode(Ok(output(pending, Some(capacity())))))
            .unwrap();
    }
    assert!(b.next_request().unwrap().is_none());
    let error = b.finish().unwrap_err();
    assert!(matches!(
        error.kind,
        BatchWriteErrorKind::State(BatchStateError::BudgetExhausted)
    ));
    assert_eq!(error.partial.calls_issued, 2);
    assert_eq!(states(&error), vec![Pending, Processed, Pending]);
    assert_eq!(
        error.partial.entries[2].action.key()["id"],
        Av::S("2".into())
    );
    assert_eq!(error.capacity, vec![Some(capacity()), Some(capacity())]);
}

#[test]
fn invalid_unprocessed_payloads_freeze_the_round_and_keep_accounting() {
    for case in 0..7 {
        let mut b = batch(2, 3);
        let r = b.next_request().unwrap().unwrap();
        let writes = &r.input().request_items.as_ref().unwrap()["orders"];
        let mut changed = writes[0].clone();
        changed
            .put_request
            .as_mut()
            .unwrap()
            .item
            .insert("payload".into(), Av::S("changed".into()));
        let raw = match case {
            0 => output(vec![writes[0].clone(), writes[0].clone()], Some(capacity())),
            1 => output(vec![WriteRequest::builder().build()], Some(capacity())),
            2 => output(vec![changed], Some(capacity())),
            3 => output(
                vec![
                    WriteRequest::builder()
                        .set_put_request(writes[0].put_request.clone())
                        .set_delete_request(writes[1].delete_request.clone())
                        .build(),
                ],
                Some(capacity()),
            ),
            4 => Output::builder()
                .unprocessed_items("other", vec![writes[0].clone()])
                .set_consumed_capacity(Some(capacity()))
                .build(),
            5 => output(
                vec![
                    WriteRequest::builder()
                        .delete_request(
                            aws_sdk_dynamodb::types::DeleteRequest::builder()
                                .set_key(Some(actions(1)[0].key().clone()))
                                .build()
                                .unwrap(),
                        )
                        .build(),
                ],
                Some(capacity()),
            ),
            _ => output(
                vec![
                    WriteRequest::builder()
                        .delete_request(
                            aws_sdk_dynamodb::types::DeleteRequest::builder()
                                .key("id", Av::S("unknown".into()))
                                .build()
                                .unwrap(),
                        )
                        .build(),
                ],
                Some(capacity()),
            ),
        };
        let decoded = r.decode(Ok(raw));
        assert!(decoded.is_err());
        let error = b.accept(decoded).unwrap_err();
        assert!(matches!(
            error.kind,
            BatchWriteErrorKind::State(BatchStateError::InvalidResponse)
        ));
        assert_eq!(states(&error), vec![Indeterminate, Indeterminate]);
        assert_eq!(error.capacity, vec![Some(capacity())]);
        assert!(matches!(
            b.next_request(),
            Err(BatchStateError::Indeterminate)
        ));
        assert!(std::sync::Arc::ptr_eq(&error, &b.finish().unwrap_err()));
        assert!(!format!("{error:?} {error}").contains("private-payload"));
    }
}

#[test]
fn sdk_failure_and_abort_never_reissue_indeterminate_writes() {
    for abort in [
        None,
        Some(BatchWriteAbort::Cancelled),
        Some(BatchWriteAbort::Deadline),
    ] {
        let mut b = batch(2, 3);
        let first = b.next_request().unwrap().unwrap();
        let pending = first.input().request_items.as_ref().unwrap()["orders"][1].clone();
        b.accept(first.decode(Ok(output(vec![pending], Some(capacity())))))
            .unwrap();
        let r = b.next_request().unwrap().unwrap();
        if let Some(cause) = abort {
            b.abort_round(cause).unwrap();
        } else {
            let error = b
                .accept(
                    r.decode(Err(SdkError::construction_failure(std::io::Error::other(
                        "private-sdk",
                    )))),
                )
                .unwrap_err();
            assert!(error.sdk_error().is_some());
            assert!(std::error::Error::source(error.as_ref()).is_some());
            assert!(!format!("{error:?} {error}").contains("private-sdk"));
        }
        assert!(matches!(
            b.next_request(),
            Err(BatchStateError::Indeterminate)
        ));
        let error = b.finish().unwrap_err();
        if let Some(cause) = abort {
            assert!(matches!(error.kind, BatchWriteErrorKind::Aborted(c) if c == cause));
        }
        assert_eq!(states(&error), vec![Processed, Indeterminate]);
        assert_eq!(error.partial.calls_issued, 2);
        assert_eq!(error.capacity, vec![Some(capacity()), None]);
    }
    assert!(matches!(
        batch(1, 2).abort_round(BatchWriteAbort::Cancelled),
        Err(BatchStateError::WrongRound)
    ));
}

#[test]
fn request_and_action_caps_are_checked_before_issuing_a_round() {
    assert!(matches!(
        table().batch_write(vec![], BatchBudget::new(1).unwrap()),
        Err(BuildError::EmptyOperation)
    ));
    assert!(matches!(
        table().batch_write(actions(26), BatchBudget::new(1).unwrap()),
        Err(BuildError::RequestLimit {
            max: 25,
            actual: 26
        })
    ));
    assert_eq!(
        batch(25, 1)
            .next_request()
            .unwrap()
            .unwrap()
            .positions()
            .len(),
        25
    );
}

#[test]
fn wire_cap_counts_escaped_payloads_independently_of_item_size() {
    for text in ["a", "\0", "\n", "\r", "\t", "\u{8}", "\u{c}"] {
        let payload = text.repeat(300_000);
        let actions = (0..25)
            .map(|i| {
                BatchAction::put(&Order {
                    id: i.to_string(),
                    payload: payload.clone(),
                })
                .unwrap()
            })
            .collect();
        let result = table().batch_write(actions, BatchBudget::new(1).unwrap());
        if payload.starts_with('\0') {
            assert!(
                matches!(result, Err(BuildError::RequestLimit { max, actual })
                if max == 16 * 1024 * 1024 && actual > max)
            );
        } else {
            assert!(result.is_ok());
        }
    }
}

#[test]
fn wire_request_boundary_pins_root_action_and_value_allowances() {
    // For these 25 two-string items, the documented allowances contribute
    // 1,951 bytes plus 6 per NUL and 1 per trailing ASCII character.
    // 25 * 111,835 NULs plus two more leave three bytes below 16 MiB.
    for (tail, expected) in [(2, 16_777_215), (3, 16_777_216), (4, 16_777_217)] {
        let actions = (0..25)
            .map(|i| {
                let mut payload = "\0".repeat(111_835);
                if i == 24 {
                    payload.push_str("\0\0");
                    payload.push_str(&"a".repeat(tail));
                }
                BatchAction::put(&Order {
                    id: i.to_string(),
                    payload,
                })
                .unwrap()
            })
            .collect();
        let result = table().batch_write(actions, BatchBudget::new(1).unwrap());
        if expected <= 16_777_216 {
            assert!(result.is_ok(), "bound {expected} must fit");
        } else {
            assert!(
                matches!(result, Err(BuildError::RequestLimit { max: 16_777_216, actual }) if actual == expected)
            );
        }
    }
}

#[test]
fn foreign_results_freeze_only_the_recipient_and_outstanding_finish_is_indeterminate() {
    let mut left = batch(1, 1);
    let mut right = batch(1, 1);
    let l = left.next_request().unwrap().unwrap();
    let r = right.next_request().unwrap().unwrap();
    let error = left
        .accept(r.decode(Ok(Output::builder().build())))
        .unwrap_err();
    assert!(matches!(
        error.kind,
        BatchWriteErrorKind::State(BatchStateError::WrongRound)
    ));
    assert_eq!(states(&error), vec![Indeterminate]);
    assert_eq!(error.capacity, vec![None]);
    let error = right.finish().unwrap_err();
    assert!(matches!(
        error.kind,
        BatchWriteErrorKind::State(BatchStateError::OutstandingRound)
    ));
    assert_eq!(states(&error), vec![Indeterminate]);
    assert_eq!(error.capacity, vec![None]);
    drop(l);
}

#[test]
fn apply_copies_every_field_clears_absent_fields_and_does_no_io() {
    use aws_smithy_runtime_api::client::http::http_client_fn;
    let client = Client::from_conf(
        Config::builder()
            .behavior_version_latest()
            .region(aws_sdk_dynamodb::config::Region::new("us-east-1"))
            .retry_config(aws_sdk_dynamodb::config::retry::RetryConfig::disabled())
            .timeout_config(aws_sdk_dynamodb::config::timeout::TimeoutConfig::disabled())
            .identity_cache(aws_sdk_dynamodb::config::IdentityCache::no_cache())
            .stalled_stream_protection(
                aws_sdk_dynamodb::config::StalledStreamProtectionConfig::disabled(),
            )
            .http_client(http_client_fn(|_, _| {
                panic!("must not request a connector")
            }))
            .build(),
    );
    for capacity in [Capacity::None, Capacity::Total, Capacity::Indexes] {
        let mut b = batch(2, 1);
        b.set_capacity(capacity).unwrap();
        let r = b.next_request().unwrap().unwrap();
        let stale = client
            .batch_write_item()
            .request_items("stale", vec![WriteRequest::builder().build()])
            .return_consumed_capacity(aws_sdk_dynamodb::types::ReturnConsumedCapacity::Total)
            .return_item_collection_metrics(
                aws_sdk_dynamodb::types::ReturnItemCollectionMetrics::Size,
            );
        assert_eq!(
            r.apply(stale).as_input().clone().build().unwrap(),
            *r.input()
        );
        assert_eq!(
            r.input().return_consumed_capacity.is_some(),
            capacity != Capacity::None
        );
        assert!(r.expressions().names().is_empty());
        assert!(!format!("{r:?} {b:?}").contains("private-payload"));
        b.accept(r.decode(Ok(Output::builder().build()))).unwrap();
        assert!(b.finish().is_ok());
    }
}
