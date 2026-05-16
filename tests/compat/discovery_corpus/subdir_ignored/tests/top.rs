// Phase 6 (§3.2.1) discovery-corpus fixture: a top-level `tests/top.rs`
// that the walk DOES visit. The sibling file under `tests/nested/inner.rs`
// must NOT be visited (the discovery walk is flat — §3.2.1 wording is
// literal).

#[test]
fn ui() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/ui/top.rs");
}
