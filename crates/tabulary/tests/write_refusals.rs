#![cfg(feature = "derive")]

use std::error::Error;

use aws_sdk_dynamodb::{
    error::SdkError,
    operation::{
        delete_item::DeleteItemError, put_item::PutItemError, update_item::UpdateItemError,
    },
    types::{AttributeValue as Av, Capacity, ConsumedCapacity, ItemCollectionMetrics, error::*},
};
use aws_smithy_runtime_api::{
    client::{orchestrator::HttpResponse, result::ConnectorError},
    http::StatusCode,
};
use aws_smithy_types::{body::SdkBody, error::ErrorMetadata};
use tabulary::{
    DecodeFault, FailureKind, Item, OldItem, OldItemMode, Redacted, WriteDecodeError, WriteError,
};

// No Debug implementation: formatting failure wrappers must not require or reveal R.
#[derive(PartialEq, tabulary::Record)]
#[tabulary(rename_all = "snake_case")]
struct Order {
    #[tabulary(partition_key)]
    id: String,
    quantity: u64,
}

fn item() -> Item {
    Order {
        id: "private-order".into(),
        quantity: 7,
    }
    .to_item()
    .unwrap()
}
fn response() -> HttpResponse {
    HttpResponse::new(
        StatusCode::try_from(400).unwrap(),
        SdkBody::from("private-response"),
    )
}
fn sdk<E>(error: E) -> SdkError<E> {
    SdkError::service_error(error, response())
}
fn assert_redacted(value: &(impl std::fmt::Debug + std::fmt::Display)) {
    let formatted = format!("{value:?} {value}");
    for secret in ["private-order", "private-sdk", "private-response"] {
        assert!(!formatted.contains(secret));
    }
}

macro_rules! operation_suite {
    ($name:ident, $error:ident) => {
        #[test]
        fn $name() {
            for (modeled, expected) in [
                (
                    $error::ConditionalCheckFailedException(
                        ConditionalCheckFailedException::builder().build(),
                    ),
                    FailureKind::ConditionFailed,
                ),
                (
                    $error::TransactionConflictException(
                        TransactionConflictException::builder().build(),
                    ),
                    FailureKind::TransactionConflict,
                ),
                (
                    $error::ReplicatedWriteConflictException(
                        ReplicatedWriteConflictException::builder().build(),
                    ),
                    FailureKind::ReplicatedWriteConflict,
                ),
                (
                    $error::ProvisionedThroughputExceededException(
                        ProvisionedThroughputExceededException::builder().build(),
                    ),
                    FailureKind::ProvisionedThroughputExceeded,
                ),
                (
                    $error::ThrottlingException(ThrottlingException::builder().build()),
                    FailureKind::Throttled,
                ),
                (
                    $error::RequestLimitExceeded(RequestLimitExceeded::builder().build()),
                    FailureKind::RequestLimitExceeded,
                ),
            ] {
                let original = std::mem::discriminant(&modeled);
                let error = WriteError::<Order, $error>::from_sdk(sdk(modeled), OldItemMode::All);
                assert_redacted(&error);
                assert_eq!(
                    std::mem::discriminant(error.sdk_error().unwrap().as_service_error().unwrap()),
                    original
                );
                assert_eq!(
                    error
                        .sdk_error()
                        .unwrap()
                        .raw_response()
                        .unwrap()
                        .body()
                        .bytes(),
                    Some(b"private-response".as_slice())
                );
                assert!(error.source().unwrap().is::<SdkError<$error>>());
                let WriteError::Refused { failure, .. } = error else {
                    panic!("modeled refusal must remain refused");
                };
                assert_eq!(failure.kind, expected);
                assert!(matches!(failure.old, OldItem::Unavailable));
            }

            // Even a known machine code on an unmodeled error is not a modeled variant.
            for modeled in [
                $error::InternalServerError(
                    InternalServerError::builder()
                        .message("ConditionalCheckFailedException TransactionConflictException private-sdk")
                        .build(),
                ),
                $error::ResourceNotFoundException(
                    ResourceNotFoundException::builder()
                        .message("ThrottlingException RequestLimitExceeded private-sdk")
                        .build(),
                ),
                $error::ItemCollectionSizeLimitExceededException(
                    ItemCollectionSizeLimitExceededException::builder().build(),
                ),
                $error::generic(
                    ErrorMetadata::builder()
                        .code("ConditionalCheckFailedException")
                        .message("ProvisionedThroughputExceededException private-sdk")
                        .build(),
                ),
            ] {
                let original = std::mem::discriminant(&modeled);
                let error = WriteError::<Order, $error>::from_sdk(sdk(modeled), OldItemMode::All);
                assert!(matches!(error, WriteError::Sdk(_)));
                assert_eq!(
                    std::mem::discriminant(error.sdk_error().unwrap().as_service_error().unwrap()),
                    original
                );
                assert_redacted(&error);
            }

            let conditional = |raw| {
                sdk($error::ConditionalCheckFailedException(
                    ConditionalCheckFailedException::builder()
                        .message("private-sdk")
                        .meta(
                            ErrorMetadata::builder()
                                .code("ConditionalCheckFailedException")
                                .custom("request_id", "request-123")
                                .build(),
                        )
                        .set_item(raw)
                        .build(),
                ))
            };
            for raw in [None, Some(item()), Some(Item::new())] {
                let error =
                    WriteError::<Order, $error>::from_sdk(conditional(raw.clone()), OldItemMode::None);
                let service = error.sdk_error().unwrap().as_service_error().unwrap();
                let $error::ConditionalCheckFailedException(source) = service else {
                    panic!("original conditional source")
                };
                assert_eq!(source.item(), raw.as_ref());
                let WriteError::Refused { failure, .. } = error else {
                    panic!("refused")
                };
                assert!(matches!(failure.old, OldItem::NotRequested));
            }
            let error =
                WriteError::<Order, $error>::from_sdk(conditional(Some(item())), OldItemMode::All);
            assert_redacted(&error);
            assert_eq!(
                error
                    .sdk_error()
                    .unwrap()
                    .as_service_error()
                    .unwrap()
                    .meta()
                    .extra("request_id"),
                Some("request-123")
            );
            assert!(error.sdk_error().unwrap().as_service_error().unwrap()
                .is_conditional_check_failed_exception());
            let WriteError::Refused { failure, .. } = error else {
                panic!("refused")
            };
            assert!(!format!("{failure:?}").contains("private-order"));
            assert!(matches!(
                failure.old,
                OldItem::Decoded(Order { quantity: 7, .. })
            ));

            for (raw, attribute, kind) in [
                (Item::new(), "id", DecodeFault::Missing),
                (
                    {
                        let mut raw = item();
                        raw.remove("quantity");
                        raw
                    },
                    "quantity",
                    DecodeFault::Missing,
                ),
                (
                    {
                        let mut raw = item();
                        raw.insert("quantity".into(), Av::S("private-order".into()));
                        raw
                    },
                    "quantity",
                    DecodeFault::WrongType {
                        expected: tabulary::DynamoType::N,
                        actual: tabulary::DynamoType::S,
                    },
                ),
                (
                    {
                        let mut raw = item();
                        raw.insert("quantity".into(), Av::N("private-order".into()));
                        raw
                    },
                    "quantity",
                    DecodeFault::InvalidNumber,
                ),
                (
                    {
                        let mut raw = item();
                        raw.insert("extra".into(), Av::S("private-order".into()));
                        raw
                    },
                    "extra",
                    DecodeFault::UnknownAttribute,
                ),
            ] {
                let error = WriteError::<Order, $error>::from_sdk(
                    conditional(Some(raw.clone())),
                    OldItemMode::All,
                );
                assert_redacted(&error);
                let WriteError::Refused { failure, source } = error else {
                    panic!("malformed old item must not replace refusal")
                };
                assert_eq!(failure.kind, FailureKind::ConditionFailed);
                assert!(
                    source
                        .as_service_error()
                        .unwrap()
                        .is_conditional_check_failed_exception()
                );
                let OldItem::Invalid {
                    error,
                    raw: retained,
                } = failure.old
                else {
                    panic!("invalid old item")
                };
                assert_eq!(error.record, "Order");
                assert_eq!(error.attribute(), Some(attribute));
                assert_eq!(error.kind, kind);
                assert_eq!(retained.expose(), &raw);
                assert_eq!(retained.into_inner(), raw);
            }
        }
    };
}
operation_suite!(put_refusals, PutItemError);
operation_suite!(delete_refusals, DeleteItemError);
operation_suite!(update_refusals, UpdateItemError);

#[test]
fn old_items_use_the_complete_record_codec_including_sparse_index_invariants() {
    #[derive(tabulary::Record)]
    #[tabulary(rename_all = "snake_case",
        index(ident = ByStatus, name = "by_status", kind = global, sparse,
            partition(name = "gpk", template = "status!{status:hex}"), project = all))]
    struct Shipment {
        #[tabulary(partition_key)]
        id: String,
        status: Option<String>,
    }
    let raw = Shipment {
        id: "private-order".into(),
        status: Some("pending".into()),
    }
    .to_item()
    .unwrap();
    assert!(
        matches!(OldItem::<Shipment>::decode(OldItemMode::All, Some(raw.clone())),
        OldItem::Decoded(Shipment { status: Some(status), .. }) if status == "pending")
    );
    let mut malformed = raw;
    malformed.insert("gpk".into(), Av::S("status!73686970706564".into()));
    let error = WriteError::<Shipment, PutItemError>::from_sdk(
        sdk(PutItemError::ConditionalCheckFailedException(
            ConditionalCheckFailedException::builder()
                .set_item(Some(malformed.clone()))
                .build(),
        )),
        OldItemMode::All,
    );
    assert_redacted(&error);
    let WriteError::Refused { failure, .. } = error else {
        panic!("refused")
    };
    assert_eq!(failure.kind, FailureKind::ConditionFailed);
    let OldItem::Invalid { error, raw } = failure.old else {
        panic!("invalid full record")
    };
    assert_eq!(error.record, "Shipment");
    assert_eq!(error.attribute(), Some("gpk"));
    assert_eq!(error.kind, DecodeFault::KeyMismatch);
    assert_eq!(raw.expose(), &malformed);
}

#[test]
fn transport_and_construction_failures_are_never_refusals() {
    for (sdk, expected) in [
        (
            SdkError::construction_failure(std::io::Error::other("private-sdk")),
            0,
        ),
        (
            SdkError::timeout_error(std::io::Error::other(
                "ConditionalCheckFailedException private-sdk",
            )),
            1,
        ),
        (
            SdkError::dispatch_failure(ConnectorError::io(Box::new(std::io::Error::other(
                "ThrottlingException private-sdk",
            )))),
            2,
        ),
        (
            SdkError::response_error(
                std::io::Error::other("TransactionConflictException private-sdk"),
                response(),
            ),
            3,
        ),
    ] {
        let error = WriteError::<Order, PutItemError>::from_sdk(sdk, OldItemMode::All);
        assert!(matches!(error, WriteError::Sdk(_)));
        assert_redacted(&error);
        assert!(matches!(
            (expected, error.sdk_error().unwrap()),
            (0, SdkError::ConstructionFailure(_))
                | (1, SdkError::TimeoutError(_))
                | (2, SdkError::DispatchFailure(_))
                | (3, SdkError::ResponseError(_))
        ));
        assert!(error.source().unwrap().is::<SdkError<PutItemError>>());
    }
}

#[test]
fn committed_write_decode_faults_preserve_accounting_and_redact_raw_items() {
    let mut raw = item();
    raw.insert("quantity".into(), Av::N("private-order".into()));
    let OldItem::<Order>::Invalid { error: source, .. } =
        OldItem::decode(OldItemMode::All, Some(raw.clone()))
    else {
        panic!("strict decode")
    };
    let capacity = ConsumedCapacity::builder()
        .capacity_units(2.5)
        .write_capacity_units(2.5)
        .table(Capacity::builder().write_capacity_units(1.0).build())
        .global_secondary_indexes("by_status", Capacity::builder().capacity_units(1.5).build())
        .build();
    let metrics = ItemCollectionMetrics::builder()
        .item_collection_key("id", Av::S("private-order".into()))
        .size_estimate_range_gb(0.1)
        .size_estimate_range_gb(0.2)
        .build();
    let error = WriteError::<Order, PutItemError>::Decode(Box::new(WriteDecodeError {
        source: source.clone(),
        raw: Redacted::new(raw.clone()),
        capacity: Some(capacity.clone()),
        item_collection_metrics: Some(metrics.clone()),
    }));
    assert_redacted(&error);
    assert!(error.sdk_error().is_none());
    assert!(error.source().unwrap().is::<WriteDecodeError>());
    let WriteError::Decode(fault) = error else {
        panic!("decode fault")
    };
    assert_eq!(fault.source, source);
    assert_eq!(fault.capacity, Some(capacity));
    assert_eq!(fault.item_collection_metrics, Some(metrics));
    assert_eq!(fault.raw.expose(), &raw);
    assert!(fault.source().unwrap().is::<tabulary::DecodeError>());
    assert!(matches!(
        OldItem::<Order>::decode(OldItemMode::All, None),
        OldItem::Unavailable
    ));
    assert_eq!(OldItemMode::default(), OldItemMode::None);
}
