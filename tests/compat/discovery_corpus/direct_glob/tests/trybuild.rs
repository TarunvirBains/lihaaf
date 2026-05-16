// Phase 6 (§3.2.1) discovery-corpus fixture: pattern 2 (glob argument).
// `tests/ui/*.rs` expands deterministically to every `.rs` file in
// `tests/ui/` — the corpus provides `bar.rs` and `foo.rs`, so the
// discovery walk must surface both in ASCII byte order.

#[test]
fn ui() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/ui/*.rs");
}
