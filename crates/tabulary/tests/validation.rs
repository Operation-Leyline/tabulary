#![cfg(feature = "derive")]

use aws_sdk_dynamodb::{
    error::SdkError,
    operation::{
        batch_get_item::BatchGetItemOutput, get_item::GetItemOutput, put_item::PutItemError,
        query::QueryOutput, scan::ScanOutput, transact_write_items::TransactWriteItemsError,
    },
    types::{
        AttributeValue as Av, CancellationReason,
        error::{ConditionalCheckFailedException, TransactionCanceledException},
    },
};
use tabulary::{
    BatchAction, BatchBudget, BatchGetFailure, BuildError, DecodeContext, DecodeFault, Document,
    EncodeContext, EncodeFault, Item, List, OldItem, OldItemMode, PathSegment, Projection,
    ReadConsistency::Strong, ReadError, Table, ValidationFault, ValidationPath, Value,
};

#[derive(Debug, PartialEq, tabulary::Variants)]
#[tabulary(tag = "status", rename_all = "snake_case")]
enum Status {
    Waiting,
    Sent { tracking: String },
}

#[derive(Debug, PartialEq, tabulary::Record)]
#[tabulary(rename_all = "snake_case", validate = validate_order,
    index(ident = ByQueue, name = "by_queue", kind = global, sparse,
        partition = queue, project = all))]
struct Order {
    #[tabulary(partition_key)]
    id: String,
    #[tabulary(flatten)]
    status: Status,
    #[tabulary(rename = "queue.name")]
    queue: Option<String>,
}

fn validate_order(order: &Order) -> Result<(), ValidationFault<Order>> {
    if matches!(order.status, Status::Waiting) != order.queue.is_some() {
        return Err(
            ValidationFault::new(OrderAttrs::QUEUE, "status_queue").with_path(OrderAttrs::STATUS)
        );
    }
    Ok(())
}

fn order() -> Order {
    Order {
        id: "order-secret".into(),
        status: Status::Waiting,
        queue: Some("queue-secret".into()),
    }
}

fn table() -> Table<Order> {
    Table::new("orders").unwrap()
}

fn invalid_item() -> Item {
    let mut item = order().to_item().unwrap();
    item.remove("queue.name");
    item
}

fn assert_decode(error: &tabulary::DecodeError) {
    assert_eq!((error.record, error.document), ("Order", "Order"));
    assert_eq!(
        error.kind,
        DecodeFault::Validation {
            code: "status_queue"
        }
    );
    assert_eq!(error.path.to_string(), "[\"queue.name\"]");
    assert_eq!(error.additional_paths[0].to_string(), "[\"status\"]");
    for rendered in [format!("{error:?}"), error.to_string()] {
        assert!(!rendered.contains("order-secret"));
        assert!(!rendered.contains("queue-secret"));
    }
}

#[test]
fn sparse_status_invariant_runs_on_both_codecs_and_before_key_encoding() {
    for valid in [
        order(),
        Order {
            status: Status::Sent {
                tracking: "parcel".into(),
            },
            queue: None,
            ..order()
        },
    ] {
        assert_eq!(Order::from_item(&valid.to_item().unwrap()).unwrap(), valid);
    }
    for invalid in [
        Order {
            queue: None,
            ..order()
        },
        Order {
            status: Status::Sent {
                tracking: "parcel".into(),
            },
            ..order()
        },
    ] {
        let error = invalid.to_item().unwrap_err();
        assert_eq!(
            error.kind,
            EncodeFault::Validation {
                code: "status_queue"
            }
        );
        assert_eq!((error.record, error.document), ("Order", "Order"));
        assert_eq!(error.path.to_string(), "[\"queue.name\"]");
        assert_eq!(error.additional_paths[0].to_string(), "[\"status\"]");
    }
    let invalid = Order {
        id: String::new(),
        queue: None,
        ..order()
    };
    assert_eq!(
        invalid.to_item().unwrap_err().kind,
        EncodeFault::Validation {
            code: "status_queue"
        }
    );
    let mut item = invalid_item();
    item.insert("unknown".into(), Av::Bool(true));
    assert_decode(&Order::from_item(&item).unwrap_err());
    item.remove("unknown");
    item.insert("status".into(), Av::S("sent".into()));
    item.insert("tracking".into(), Av::S("parcel".into()));
    item.insert("queue.name".into(), Av::S("queue-secret".into()));
    assert_decode(&Order::from_item(&item).unwrap_err());
}

#[test]
fn write_adapters_share_the_hook() {
    let invalid = Order {
        queue: None,
        ..order()
    };
    let table = table();
    for error in [
        table.put(&invalid).build().unwrap_err(),
        table.put(&invalid).into_tx().unwrap_err(),
        BatchAction::put(&invalid).unwrap_err(),
    ] {
        let BuildError::Encode(error) = error else {
            panic!("expected encode fault")
        };
        assert_eq!(
            error.kind,
            EncodeFault::Validation {
                code: "status_queue"
            }
        );
    }
}

#[test]
fn read_adapters_share_the_hook() {
    let table = table();
    let get = table
        .get(OrderKey::new("order-secret".into()).unwrap(), Strong)
        .build()
        .unwrap();
    let query = table
        .query(Order::primary().partition("order-secret".into()), Strong)
        .build()
        .unwrap();
    let scan = table.scan(Strong).build().unwrap();
    fn check<E>(error: ReadError<E>) {
        let ReadError::Decode(error) = error else {
            panic!("expected decode fault")
        };
        assert_decode(&error.source);
    }
    check(
        get.decode(Ok(GetItemOutput::builder()
            .set_item(Some(invalid_item()))
            .build()))
            .unwrap_err(),
    );
    check(
        query
            .decode(Ok(QueryOutput::builder()
                .items(invalid_item())
                .count(1)
                .scanned_count(1)
                .build()))
            .unwrap_err(),
    );
    check(
        scan.decode(Ok(ScanOutput::builder()
            .items(invalid_item())
            .count(1)
            .scanned_count(1)
            .build()))
            .unwrap_err(),
    );
}

fn sdk_error<E>(error: E) -> SdkError<E> {
    use aws_smithy_runtime_api::{client::orchestrator::HttpResponse, http::StatusCode};
    SdkError::service_error(
        error,
        HttpResponse::new(
            StatusCode::try_from(400).unwrap(),
            aws_smithy_types::body::SdkBody::empty(),
        ),
    )
}
fn assert_old(old: OldItem<Order>) {
    let OldItem::Invalid { error, .. } = old else {
        panic!("expected invalid old item")
    };
    assert_decode(&error);
}

#[test]
fn conditional_put_decodes_validation_in_all_old_refusal() {
    let request = table()
        .put(&order())
        .condition(OrderAttrs::ID.exists())
        .on_condition_failure(OldItemMode::All)
        .build()
        .unwrap();
    let error = PutItemError::ConditionalCheckFailedException(
        ConditionalCheckFailedException::builder()
            .set_item(Some(invalid_item()))
            .build(),
    );
    let tabulary::WriteError::Refused { failure, .. } =
        request.decode(Err(sdk_error(error))).unwrap_err()
    else {
        panic!("expected refusal")
    };
    assert_old(failure.old);
}

#[test]
fn transaction_cancellation_decodes_validation_in_old_item() {
    let action = table()
        .put(&order())
        .on_condition_failure(OldItemMode::All)
        .into_tx()
        .unwrap();
    let request = tabulary::transaction(action).build().unwrap();
    let reason = CancellationReason::builder()
        .code("ConditionalCheckFailed")
        .set_item(Some(invalid_item()))
        .build();
    let error = TransactWriteItemsError::TransactionCanceledException(
        TransactionCanceledException::builder()
            .cancellation_reasons(reason)
            .build(),
    );
    let tabulary::TxError::Cancelled(error) = request.decode(Err(sdk_error(error))).unwrap_err()
    else {
        panic!("expected cancellation")
    };
    assert_old(error.reasons.old);
}

#[test]
fn batch_get_decodes_validation_in_returned_item() {
    let mut batch = table()
        .batch_get(
            vec![OrderKey::new("order-secret".into()).unwrap()],
            Strong,
            BatchBudget::new(1).unwrap(),
        )
        .unwrap();
    let request = batch.next_request().unwrap().unwrap();
    let output = BatchGetItemOutput::builder()
        .responses("orders", vec![invalid_item()])
        .build();
    let error = batch.accept(request.decode(Ok(output))).unwrap_err();
    let BatchGetFailure::Decode { failures } = error.kind else {
        panic!("expected decode fault")
    };
    assert_eq!(failures.len(), 1);
    assert_decode(&failures[0].1);
}

#[derive(Debug, PartialEq, tabulary::Document)]
#[tabulary(rename_all = "camelCase", validate = validate_line)]
struct Line {
    unit_count: u64,
}
fn validate_line(line: &Line) -> Result<(), ValidationFault<Line>> {
    if line.unit_count == 0 {
        return Err(ValidationFault::new(LineAttrs::UNIT_COUNT, "positive")
            .with_path(LineAttrs::UNIT_COUNT));
    }
    Ok(())
}

#[derive(Debug, tabulary::Record)]
#[tabulary(rename_all = "snake_case", validate = validate_invoice)]
struct Invoice {
    #[tabulary(partition_key)]
    id: String,
    lines: List<Line>,
}
fn validate_invoice(invoice: &Invoice) -> Result<(), ValidationFault<Invoice>> {
    if invoice.id == "invalid" {
        return Err(ValidationFault::new(InvoiceAttrs::ID, "invoice"));
    }
    Ok(())
}

fn invoice_item(id: &str) -> Item {
    Item::from([
        ("id".into(), Av::S(id.into())),
        (
            "lines".into(),
            Av::L(vec![Av::M(Item::from([(
                "unitCount".into(),
                Av::N("0".into()),
            )]))]),
        ),
    ])
}

#[test]
fn nested_document_hook_keeps_root_document_and_list_path() {
    let invoice = Invoice {
        id: "invoice".into(),
        lines: List(vec![Line { unit_count: 0 }]),
    };
    let expected = [
        PathSegment::Attribute("lines".into()),
        PathSegment::Index(0),
        PathSegment::Attribute("unitCount".into()),
    ];
    let error = invoice.to_item().unwrap_err();
    assert_eq!((error.record, error.document), ("Invoice", "Line"));
    assert_eq!(error.path.segments(), expected);
    assert_eq!(error.additional_paths[0].segments(), expected);
    let item = invoice_item("invoice");
    let error = Invoice::from_item(&item).unwrap_err();
    assert_eq!((error.record, error.document), ("Invoice", "Line"));
    assert_eq!(error.kind, DecodeFault::Validation { code: "positive" });
    assert_eq!(error.path.segments(), expected);
    assert_eq!(error.additional_paths[0].segments(), expected);
    assert!(
        Line { unit_count: 2 }
            .encode_map(&mut EncodeContext::new("Line"))
            .is_ok()
    );
}

#[test]
fn outer_hook_precedes_inner_on_encode_and_follows_it_on_decode() {
    let invoice = Invoice {
        id: "invalid".into(),
        lines: List(vec![Line { unit_count: 0 }]),
    };
    let error = invoice.to_item().unwrap_err();
    assert_eq!(error.kind, EncodeFault::Validation { code: "invoice" });
    assert_eq!(error.path.to_string(), "[\"id\"]");
    let item = invoice_item("invalid");
    let error = Invoice::from_item(&item).unwrap_err();
    assert_eq!(error.kind, DecodeFault::Validation { code: "positive" });
    assert_eq!(error.path.to_string(), "[\"lines\"][0][\"unitCount\"]");
}

#[test]
fn documents_inside_maps_validate_before_encoding_their_fields() {
    let map = tabulary::Map(std::collections::BTreeMap::from([(
        "entry".into(),
        Line { unit_count: 0 },
    )]));
    let error = map
        .encode_value(&mut EncodeContext::new("Invoice"))
        .unwrap_err();
    assert_eq!(error.kind, EncodeFault::Validation { code: "positive" });
    assert_eq!(error.path.to_string(), "[\"entry\"][\"unitCount\"]");
}

#[derive(Debug, tabulary::Document)]
#[tabulary(rename_all = "snake_case", validate = validate_parcel)]
struct Parcel {
    #[tabulary(rename = "line.detail")]
    line: Option<Line>,
}
fn validate_parcel(_: &Parcel) -> Result<(), ValidationFault<Parcel>> {
    Err(ValidationFault::new(
        ValidationPath::from(ParcelAttrs::LINE).field(LineAttrs::UNIT_COUNT),
        "parcel_count",
    ))
}

#[test]
fn fault_handles_descend_through_physical_document_paths_without_values() {
    let parcel = Parcel {
        line: Some(Line { unit_count: 5 }),
    };
    let fault = validate_parcel(&parcel).unwrap_err();
    assert_eq!(fault.path().to_string(), "[\"line.detail\"][\"unitCount\"]");
    assert_eq!(fault.code(), "parcel_count");
    assert!(fault.additional_paths().is_empty());
    assert!(!format!("{fault:?} {fault}").contains('5'));
    let encode = parcel
        .encode_map(&mut EncodeContext::new("Parcel"))
        .unwrap_err();
    assert_eq!(encode.path, *fault.path());
    let item = Item::from([(
        "line.detail".into(),
        Av::M(Item::from([("unitCount".into(), Av::N("5".into()))])),
    )]);
    let decode = Parcel::decode_map(&item, &mut DecodeContext::new("Parcel")).unwrap_err();
    assert_eq!(decode.path, *fault.path());
}

#[derive(Debug, PartialEq, tabulary::Projection)]
#[tabulary(record = Order)]
struct Queue {
    #[tabulary(attribute = OrderAttrs::QUEUE)]
    queue: Option<String>,
}

#[test]
fn projections_do_not_validate_the_incomplete_source() {
    assert_eq!(
        Queue::from_item(&invalid_item()).unwrap(),
        Queue { queue: None }
    );
    assert_eq!(
        Queue::project_source(
            Order {
                queue: None,
                ..order()
            },
            &mut DecodeContext::new("Order")
        )
        .unwrap(),
        Queue { queue: None }
    );
}

#[test]
fn declarations_fail_at_the_attribute() {
    trybuild::TestCases::new().compile_fail("tests/ui/validation/*.rs");
}

#[derive(Debug, tabulary::Document)]
#[tabulary(rename_all = "snake_case", validate = checks::quantity)]
struct Quantity<T> {
    value: T,
}
mod checks {
    pub fn quantity<T: tabulary::Value>(
        _: &super::Quantity<T>,
    ) -> Result<(), tabulary::ValidationFault<super::Quantity<T>>> {
        Err(tabulary::ValidationFault::new(
            super::QuantityAttrs::<T>::VALUE,
            "quantity",
        ))
    }
}

#[test]
fn qualified_generic_hooks_use_the_instantiated_document_type() {
    let quantity = Quantity { value: 3u64 };
    let encode = quantity
        .encode_map(&mut EncodeContext::new("Quantity"))
        .unwrap_err();
    assert_eq!(encode.kind, EncodeFault::Validation { code: "quantity" });
    let item = Item::from([("value".into(), Av::N("3".into()))]);
    let decode =
        Quantity::<u64>::decode_map(&item, &mut DecodeContext::new("Quantity")).unwrap_err();
    assert_eq!(decode.kind, DecodeFault::Validation { code: "quantity" });
}
