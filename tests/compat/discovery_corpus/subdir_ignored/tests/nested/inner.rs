// Phase 6 (§3.2.1) discovery-corpus fixture: this file lives under
// `tests/nested/inner.rs` (a subdirectory). The discovery walk is
// flat — `tests/*.rs` only — so this file must NOT be visited.
// Discovery from `subdir_ignored/` must produce exactly one fixture
// (from `tests/top.rs`) and zero from this file.
//
// If this file IS visited, the discovery test fails the
// "flat walk only" invariant.

#[test]
fn nested_ui() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/ui/should_not_be_seen.rs");
}
