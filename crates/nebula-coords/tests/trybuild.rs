#[test]
fn test_transition_type_safety() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/ui/transition_type_mismatch.rs");
}
