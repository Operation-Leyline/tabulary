#![cfg(feature = "derive")]

#[test]
fn record_diagnostics_point_to_invalid_declarations() {
    trybuild::TestCases::new().compile_fail("tests/ui/record/*.rs");
}
