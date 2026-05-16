// Phase 6 (§3.2.1) discovery-corpus fixture: `use ... as ...` aliases
// are NOT syntactically recognized (Q6 locked decision). The visitor
// must surface a `discovery_unrecognized` entry rather than treating
// `Foo` as `trybuild::TestCases`. Adopters with `use` aliases must
// register them via `--compat-trybuild-macro`.

use trybuild::TestCases as Foo;

#[test]
fn ui() {
    let t = Foo::new();
    t.compile_fail("tests/ui/foo.rs");
}
