#![cfg(feature = "derive")]

use aws_sdk_dynamodb::{
    error::SdkError,
    operation::{
        delete_item::DeleteItemError, put_item::PutItemError, update_item::UpdateItemError,
    },
    types::{AttributeValue as Av, error::ConditionalCheckFailedException},
};
use tabulary::{
    BuildError, DecodeFault, FailureKind, OldItem, OldItemMode, Table, Update, WriteError,
};

#[derive(Debug, PartialEq, tabulary::Record)]
#[tabulary(rename_all = "snake_case")]
struct Invoice {
    #[tabulary(partition_key)]
    id: String,
    balance: u64,
    #[tabulary(version, rename = "edit.no")]
    revision: u64,
}
fn invoice(revision: u64) -> Invoice {
    Invoice {
        id: "invoice-lime".into(),
        balance: 7,
        revision,
    }
}
fn table() -> Table<Invoice> {
    Table::new("invoices").unwrap()
}
fn changes() -> Update<Invoice> {
    Update::new().and(InvoiceAttrs::BALANCE.set(6))
}
fn policy<T>(result: Result<T, BuildError>) {
    assert!(matches!(result, Err(BuildError::VersionPolicy)));
}

#[test]
fn every_write_requires_one_explicit_policy() {
    let row = invoice(1);
    policy(table().put(&row).build());
    policy(table().delete(row.key().unwrap()).build());
    policy(
        table()
            .update(row.key().unwrap())
            .changes(changes())
            .build(),
    );
    policy(table().put(&row).expect_new().unchecked_version().build());
    policy(
        table()
            .delete(row.key().unwrap())
            .expect_version(1)
            .expect_version(1)
            .build(),
    );
    policy(
        table()
            .update(row.key().unwrap())
            .expect_version(1)
            .unchecked_version()
            .build(),
    );
}

#[test]
fn new_put_requires_explicit_one_and_guards_primary_absence() {
    for revision in [0, 2, u64::MAX] {
        policy(table().put(&invoice(revision)).expect_new().build());
    }
    let row = invoice(1);
    let before = row.to_item().unwrap();
    let request = table()
        .put(&row)
        .expect_new()
        .condition(InvoiceAttrs::BALANCE.eq(7))
        .build()
        .unwrap();
    assert_eq!(request.input().item.as_ref().unwrap(), &before);
    assert_eq!(
        request.expressions().condition(),
        Some("(#n0 = :v0 AND attribute_not_exists(#n1))")
    );
    assert_eq!(request.expressions().names()["#n1"], "id");
    assert_eq!(row.to_item().unwrap(), before);
}

#[test]
fn expected_put_checks_successor_without_mutating_the_record() {
    let row = invoice(5);
    let request = table().put(&row).expect_version(4).build().unwrap();
    assert_eq!(request.expressions().condition(), Some("#n0 = :v0"));
    assert_eq!(request.expressions().names()["#n0"], "edit.no");
    assert_eq!(request.expressions().values()[":v0"], Av::N("4".into()));
    assert_eq!(
        request.input().item.as_ref().unwrap()["edit.no"],
        Av::N("5".into())
    );
    assert_eq!(row.revision, 5);
    policy(table().put(&row).expect_version(5).build());
    policy(
        table()
            .put(&invoice(u64::MAX))
            .expect_version(u64::MAX)
            .build(),
    );
    assert!(
        table()
            .put(&invoice(u64::MAX))
            .expect_version(u64::MAX - 1)
            .build()
            .is_ok()
    );
}

#[test]
fn guarded_update_sets_checked_successor_and_ands_all_user_conditions() {
    let row = invoice(4);
    let request = table()
        .update(row.key().unwrap())
        .expect_version(4)
        .condition(InvoiceAttrs::BALANCE.eq(7))
        .changes(changes())
        .condition(InvoiceAttrs::ID.exists())
        .build()
        .unwrap();
    let expressions = request.expressions();
    assert_eq!(expressions.update(), Some("SET #n0 = :v0, #n1 = :v1"));
    assert_eq!(
        expressions.condition(),
        Some("((#n0 = :v2 AND attribute_exists(#n2)) AND #n1 = :v3)")
    );
    assert_eq!(expressions.names()["#n1"], "edit.no");
    assert_eq!(expressions.values()[":v1"], Av::N("5".into()));
    assert_eq!(expressions.values()[":v3"], Av::N("4".into()));
    assert_eq!(row.revision, 4);
    policy(
        table()
            .update(row.key().unwrap())
            .changes(changes())
            .expect_version(u64::MAX)
            .build(),
    );
    assert!(
        table()
            .update(row.key().unwrap())
            .expect_version(u64::MAX - 1)
            .build()
            .is_ok()
    );
}

#[test]
fn delete_does_not_increment_and_bypass_adds_no_guard_or_increment() {
    let row = invoice(0);
    let request = table()
        .delete(row.key().unwrap())
        .expect_version(u64::MAX)
        .condition(InvoiceAttrs::BALANCE.eq(7))
        .build()
        .unwrap();
    assert_eq!(
        request.expressions().condition(),
        Some("(#n0 = :v0 AND #n1 = :v1)")
    );
    assert_eq!(
        request.expressions().values()[":v1"],
        Av::N(u64::MAX.to_string())
    );
    assert!(request.expressions().update().is_none());
    let put = table().put(&row).unchecked_version().build().unwrap();
    assert!(put.expressions().condition().is_none());
    assert_eq!(
        put.input().item.as_ref().unwrap()["edit.no"],
        Av::N("0".into())
    );
    assert!(
        table()
            .delete(row.key().unwrap())
            .unchecked_version()
            .build()
            .unwrap()
            .expressions()
            .condition()
            .is_none()
    );
    let update = table()
        .update(row.key().unwrap())
        .changes(changes())
        .unchecked_version()
        .condition(InvoiceAttrs::BALANCE.eq(7))
        .build()
        .unwrap();
    assert_eq!(update.expressions().update(), Some("SET #n0 = :v0"));
    assert_eq!(update.expressions().condition(), Some("#n0 = :v1"));
    assert!(
        !update
            .expressions()
            .names()
            .values()
            .any(|name| name == "edit.no")
    );
}

#[test]
fn zero_expectation_has_only_equality_and_absent_or_malformed_versions_decode_strictly() {
    let row = invoice(1);
    let put = table().put(&row).expect_version(0).build().unwrap();
    let delete = table()
        .delete(row.key().unwrap())
        .expect_version(0)
        .build()
        .unwrap();
    let update = table()
        .update(row.key().unwrap())
        .expect_version(0)
        .build()
        .unwrap();
    for expressions in [
        put.expressions(),
        delete.expressions(),
        update.expressions(),
    ] {
        let expected = if expressions.update().is_some() {
            "#n0 = :v1"
        } else {
            "#n0 = :v0"
        };
        assert_eq!(expressions.condition(), Some(expected));
        assert!(
            expressions
                .values()
                .values()
                .any(|value| value == &Av::N("0".into()))
        );
    }
    let mut item = row.to_item().unwrap();
    item.remove("edit.no");
    let error = Invoice::from_item(&item).unwrap_err();
    assert_eq!(error.kind, DecodeFault::Missing);
    assert_eq!(error.record, "Invoice");
    assert_eq!(error.attribute(), Some("edit.no"));
    for value in [Av::S("1".into()), Av::N("bad".into()), Av::N("-1".into())] {
        item.insert("edit.no".into(), value);
        assert!(Invoice::from_item(&item).is_err());
    }
}

fn response() -> aws_smithy_runtime_api::http::Response {
    aws_smithy_runtime_api::http::Response::new(
        aws_smithy_runtime_api::http::StatusCode::try_from(400).unwrap(),
        aws_smithy_types::body::SdkBody::empty(),
    )
}

#[test]
fn stale_refusals_stay_condition_failed_with_typed_old_items() {
    let row = invoice(5);
    let old = invoice(8);
    let exception = || {
        ConditionalCheckFailedException::builder()
            .set_item(Some(old.to_item().unwrap()))
            .build()
    };
    let put = table()
        .put(&row)
        .expect_version(4)
        .on_condition_failure(OldItemMode::All)
        .build()
        .unwrap();
    let delete = table()
        .delete(row.key().unwrap())
        .expect_version(4)
        .on_condition_failure(OldItemMode::All)
        .build()
        .unwrap();
    let update = table()
        .update(row.key().unwrap())
        .expect_version(4)
        .on_condition_failure(OldItemMode::All)
        .build()
        .unwrap();
    macro_rules! refusal {
        ($result:expr) => { match $result {
            Err(WriteError::Refused { failure, .. }) => {
                assert_eq!(failure.kind, FailureKind::ConditionFailed);
                assert!(matches!(failure.old, OldItem::Decoded(record) if record == old));
            }
            _ => panic!("expected modeled conditional refusal"),
        }};
    }
    refusal!(put.decode(Err(SdkError::service_error(
        PutItemError::ConditionalCheckFailedException(exception()),
        response()
    ))));
    refusal!(delete.decode(Err(SdkError::service_error(
        DeleteItemError::ConditionalCheckFailedException(exception()),
        response()
    ))));
    refusal!(update.decode(Err(SdkError::service_error(
        UpdateItemError::ConditionalCheckFailedException(exception()),
        response()
    ))));
}

#[derive(tabulary::Record)]
#[tabulary(rename_all = "snake_case")]
struct Session {
    #[tabulary(partition_key)]
    id: String,
}
#[test]
fn unversioned_writes_need_no_policy_and_cannot_claim_a_version() {
    let row = Session {
        id: "session-lime".into(),
    };
    let table = Table::new("sessions").unwrap();
    assert!(table.put(&row).build().is_ok());
    assert!(table.delete(row.key().unwrap()).build().is_ok());
    assert!(table.put(&row).expect_new().build().is_ok());
    policy(table.put(&row).expect_version(0).build());
    policy(table.delete(row.key().unwrap()).unchecked_version().build());
}

#[test]
fn forged_mutable_version_handle_cannot_bypass_guarded_or_unchecked_updates() {
    let forged: tabulary::Attr<Invoice, u64> = tabulary::__private::field("edit.no", 2);
    for guarded in [false, true] {
        let builder = table()
            .update(invoice(1).key().unwrap())
            .changes(Update::new().and(forged.set(9)));
        let result = if guarded {
            builder.expect_version(1).build()
        } else {
            builder.unchecked_version().build()
        };
        assert!(matches!(
            result,
            Err(BuildError::OperationConstraint {
                code: "invalid_attribute_descriptor"
            })
        ));
    }
}

#[allow(dead_code)]
#[path = "../../../docs/examples/04_conditional_update.rs"]
mod conditional_update_example;

#[test]
fn invalid_version_declarations_and_mutations_fail_to_compile() {
    trybuild::TestCases::new().compile_fail("tests/ui/version/*.rs");
}
