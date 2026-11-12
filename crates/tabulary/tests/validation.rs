#![cfg(feature = "derive")]

use aws_sdk_dynamodb::{
    operation::{get_item::GetItemOutput, query::QueryOutput, scan::ScanOutput},
    types::AttributeValue as Av,
};
use tabulary::{
    BatchAction, BuildError, DecodeContext, DecodeFault, Document, EncodeFault, Item, List,
    OldItem, OldItemMode, PathSegment, Projection, ReadConsistency::Strong, ReadError, Table,
    ValidationFault, ValidationPath,
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

fn validate_order(order: &Order) -> Result<(), ValidationFault> {
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
    let table = Table::<Order>::new("orders").unwrap();
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
fn read_adapters_and_conditional_old_items_share_the_hook() {
    let table = Table::<Order>::new("orders").unwrap();
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
    let OldItem::<Order>::Invalid { error, .. } =
        OldItem::decode(OldItemMode::All, Some(invalid_item()))
    else {
        panic!("expected invalid old item")
    };
    assert_decode(&error);
}

#[derive(Debug, PartialEq, tabulary::Document)]
#[tabulary(rename_all = "camelCase", validate = validate_line)]
struct Line {
    unit_count: u64,
}
fn validate_line(line: &Line) -> Result<(), ValidationFault> {
    if line.unit_count == 0 {
        return Err(ValidationFault::new(LineAttrs::UNIT_COUNT, "positive")
            .with_path(LineAttrs::UNIT_COUNT));
    }
    Ok(())
}

#[derive(Debug, tabulary::Record)]
#[tabulary(rename_all = "snake_case")]
struct Invoice {
    #[tabulary(partition_key)]
    id: String,
    lines: List<Line>,
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
    let item = Item::from([
        ("id".into(), Av::S("invoice".into())),
        (
            "lines".into(),
            Av::L(vec![Av::M(Item::from([(
                "unitCount".into(),
                Av::N("0".into()),
            )]))]),
        ),
    ]);
    let error = Invoice::from_item(&item).unwrap_err();
    assert_eq!((error.record, error.document), ("Invoice", "Line"));
    assert_eq!(error.kind, DecodeFault::Validation { code: "positive" });
    assert_eq!(error.path.segments(), expected);
    assert_eq!(error.additional_paths[0].segments(), expected);
    assert!(
        Line { unit_count: 2 }
            .encode_map(&mut tabulary::EncodeContext::new("Line"))
            .is_ok()
    );
}

#[derive(Debug, tabulary::Document)]
#[tabulary(rename_all = "snake_case", validate = validate_parcel)]
struct Parcel {
    #[tabulary(rename = "line.detail")]
    line: Option<Line>,
}
fn validate_parcel(_: &Parcel) -> Result<(), ValidationFault> {
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
    assert_eq!(
        parcel
            .encode_map(&mut tabulary::EncodeContext::new("Parcel"))
            .unwrap_err()
            .path,
        *fault.path()
    );
    let item = Item::from([(
        "line.detail".into(),
        Av::M(Item::from([("unitCount".into(), Av::N("5".into()))])),
    )]);
    assert_eq!(
        Parcel::decode_map(&item, &mut DecodeContext::new("Parcel"))
            .unwrap_err()
            .path,
        *fault.path()
    );
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
    ) -> Result<(), tabulary::ValidationFault> {
        Err(tabulary::ValidationFault::new(
            super::QuantityAttrs::<T>::VALUE,
            "quantity",
        ))
    }
}

#[test]
fn qualified_generic_hooks_use_the_instantiated_document_type() {
    let quantity = Quantity { value: 3u64 };
    assert_eq!(
        quantity
            .encode_map(&mut tabulary::EncodeContext::new("Quantity"))
            .unwrap_err()
            .kind,
        EncodeFault::Validation { code: "quantity" }
    );
    assert_eq!(
        Quantity::<u64>::decode_map(
            &Item::from([("value".into(), Av::N("3".into()))]),
            &mut DecodeContext::new("Quantity")
        )
        .unwrap_err()
        .kind,
        DecodeFault::Validation { code: "quantity" }
    );
}
