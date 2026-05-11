// Compile_pass fixture that links against the lihaaf dylib via
// `--extern lihaaf=<path>`. Verifies the per-fixture rustc dispatch
// resolves the `--extern` flag end-to-end (spec §4.1 stage 7).

fn main() {
    // VERSION is a const, not a function, so the link is satisfied at
    // compile time without the dylib needing dynamic-symbol exports
    // beyond what `-C prefer-dynamic` already provides.
    let _ = lihaaf::VERSION;
}
