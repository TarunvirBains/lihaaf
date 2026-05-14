// Phase 6 (§3.2.1) discovery-corpus fixture: multiple top-level
// `tests/*.rs` files, each carrying one TestCases invocation. The
// discovery walk must visit both files in deterministic ASCII order.

#[test]
fn ui_a() {
    let t = trybuild::TestCases::new();
    t.pass("tests/fixtures/a.rs");
}
