// Phase 6 (§3.2.1) discovery-corpus fixture for round-5 review fix:
// `#[cfg(...)]`-gated test functions must surface as
// `discovery_unrecognized` because the visitor cannot evaluate the cfg
// gate without feature resolution. Without the round-5 fix the visitor
// silently descends into the body and treats the gated call as an
// active fixture even when the feature is disabled at build time —
// adopters then see a false-positive fixture in the discovery output.
//
// The fix detects ANY `#[cfg(...)]` or `#[cfg_attr(...)]` attribute on
// `ItemFn` / `ImplItemFn` and emits a single `discovery_unrecognized`
// entry naming the function. The body is NOT descended into, so calls
// inside the gated function do NOT contribute fixtures.
//
// The sibling `ui_always` function is NOT cfg-gated and must surface
// its fixture as usual — the fix is per-function.

#[cfg(feature = "foo")]
#[test]
fn ui() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/ui/foo.rs");
}

#[test]
fn ui_always() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/ui/bar.rs");
}
