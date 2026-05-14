// Phase 6 (§3.2.1) discovery-corpus fixture: pattern 1 with a literal
// (non-glob) string argument. The discovery walk must produce exactly
// one DiscoveredFixture pointing at `tests/ui/foo.rs`.
//
// This file is NOT compiled by cargo — `Cargo.toml` excludes the
// `tests/compat/discovery_corpus/` tree, and cargo's `tests/`
// auto-discovery only scans the top-level `tests/`, not subdirectories.
// The file's only consumer is `syn::parse_file` inside the
// `discovery_syn` integration test.

#[test]
fn ui() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/ui/foo.rs");
}
