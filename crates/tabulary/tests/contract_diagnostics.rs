#![cfg(feature = "derive")]

#[test]
fn public_contract_invalid_expressions_and_missing_fields_have_source_snapshots() {
    trybuild::TestCases::new().compile_fail("tests/ui/contract/*.rs");
}
