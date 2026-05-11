// Trivial compile_pass fixture. The lihaaf self-test corpus exercises
// the dispatch pipeline end-to-end against the lihaaf crate itself
// (built as a dylib via `cargo rustc --crate-type=dylib` per spec
// §4.2). This fixture's contract: rustc exits 0.

fn main() {}
