//! Compile and exercise the SDK surface that establishes the supported API floor.
//! These fixtures need neither a client nor credentials or an async runtime.

use std::collections::HashMap;

use aws_sdk_dynamodb::{
    operation::put_item::PutItemError,
    types::{
        AttributeValue, ReturnValuesOnConditionCheckFailure, error::ConditionalCheckFailedException,
    },
};

#[test]
fn sdk_attribute_values_retain_native_wire_types() {
    let item = HashMap::from([
        (
            "key".to_owned(),
            AttributeValue::S("order-amber".to_owned()),
        ),
        ("quantity".to_owned(), AttributeValue::N("3".to_owned())),
    ]);

    assert_eq!(item["key"].as_s().unwrap(), "order-amber");
    assert_eq!(item["quantity"].as_n().unwrap(), "3");
}

#[test]
fn modeled_conditional_failure_retains_the_old_item() {
    let old_item = HashMap::from([("version".to_owned(), AttributeValue::N("7".to_owned()))]);
    let modeled = ConditionalCheckFailedException::builder()
        .set_item(Some(old_item.clone()))
        .build();
    let error = PutItemError::ConditionalCheckFailedException(modeled);

    assert!(error.is_conditional_check_failed_exception());
    let PutItemError::ConditionalCheckFailedException(modeled) = error else {
        panic!("expected the modeled condition failure");
    };
    assert_eq!(modeled.item(), Some(&old_item));
}

#[test]
fn conditional_failure_item_return_is_explicit_in_sdk_input() {
    let input = aws_sdk_dynamodb::operation::put_item::PutItemInput::builder()
        .table_name("orders")
        .item("key", AttributeValue::S("order-amber".to_owned()))
        .return_values_on_condition_check_failure(ReturnValuesOnConditionCheckFailure::AllOld)
        .build()
        .unwrap();

    assert_eq!(
        input.return_values_on_condition_check_failure(),
        Some(&ReturnValuesOnConditionCheckFailure::AllOld)
    );
}
