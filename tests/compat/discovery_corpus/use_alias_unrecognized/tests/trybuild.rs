// Discovery-corpus fixture for unregistered trybuild aliases:
// `use trybuild::TestCases as Foo;` aliases must surface as
// `discovery_unrecognized` when `Foo` is not registered via
// `--compat-trybuild-macro`. The previous visitor silently dropped the
// `Foo::new(); t.compile_fail(...)` call chain because `Foo` didn't
// match the canonical `trybuild::TestCases` path and didn't match any
// registered alias. The fix detects the `use ... as Foo;` rename and
// flags the subsequent terminal call.

use trybuild::TestCases as Foo;

#[test]
fn ui() {
    let t = Foo::new();
    t.compile_fail("tests/ui/foo.rs");
}
