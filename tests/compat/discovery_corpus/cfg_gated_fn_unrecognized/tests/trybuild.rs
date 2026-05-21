// Discovery-corpus fixture for cfg-gated test functions:
// `#[cfg(...)]`-gated test functions must surface as
// `discovery_unrecognized` because the visitor cannot evaluate the cfg
// gate without feature resolution. The visitor must not silently
// descend into the body and treat the gated call as an active fixture
// when the feature is disabled at build time.
//
// Discovery detects ANY `#[cfg(...)]` or `#[cfg_attr(...)]` attribute on
// `ItemFn` / `ImplItemFn` and emits a single `discovery_unrecognized`
// entry naming the function. The body is NOT descended into, so calls
// inside the gated function do NOT contribute fixtures.
//
// The sibling `ui_always` function is NOT cfg-gated and must surface
// its fixture as usual — the rule is per-function.

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
