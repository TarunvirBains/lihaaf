// Phase 6 (§3.2.1) discovery-corpus fixture: `type` aliases of
// `trybuild::TestCases` are NOT auto-recognized. The visitor emits a
// `discovery_unrecognized` entry naming the alias so the operator can
// either register the originating path via `--compat-trybuild-macro`
// or rewrite the call site to the canonical
// `trybuild::TestCases::new()` form. The `Foo::new(); t.compile_fail(...)`
// chain below resolves to neither a canonical TestCases call nor a
// registered alias, so the discovery produces zero fixtures.

type Foo = trybuild::TestCases;

#[test]
fn ui() {
    let t = Foo::new();
    t.compile_fail("tests/ui/foo.rs");
}
