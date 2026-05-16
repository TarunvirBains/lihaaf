// Fixture target for alias_use_not_recognized — the discovery walk
// must NOT surface this file because `use trybuild::TestCases as Foo;`
// aliases are unrecognized.
fn main() {}
