// Phase 6 (§3.2.1) discovery-corpus fixture: a Rust file that does NOT
// parse. The discovery walk must produce a single
// `discovery_unrecognized` entry of `detail = "parse_failed: ..."` and
// continue with the rest of the directory.

fn ui( {
    // Unclosed paren, intentional syntax error.
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/ui/foo.rs");
}
