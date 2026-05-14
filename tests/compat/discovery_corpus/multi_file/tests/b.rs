// Phase 6 (§3.2.1) discovery-corpus fixture: second top-level test
// file alongside `a.rs`. See `a.rs` for the pattern description.

#[test]
fn ui_b() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/fixtures/b.rs");
}
