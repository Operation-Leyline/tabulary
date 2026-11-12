#![cfg(feature = "derive")]
#[test]
fn invalid_indexes_fail_at_their_declarations() {
    trybuild::TestCases::new().compile_fail("tests/ui/index/*.rs");
}
