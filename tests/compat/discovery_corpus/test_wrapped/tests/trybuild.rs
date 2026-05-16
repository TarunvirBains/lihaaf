// Phase 6 (§3.2.1) discovery-corpus fixture: pattern 3
// (`#[test]`-wrapped invocation with a local `let` binding). The
// visitor descends into the `#[test]` body, records the
// `let t = trybuild::TestCases::new();` binding, and matches the
// subsequent `t.compile_fail("...")` call against the binding.

#[test]
fn ui() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/ui/x.rs");
}
