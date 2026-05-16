// Trybuild compile_fail fixture (corpus-only — not actually compiled
// by the discovery test, just required to exist so the glob/literal
// resolver finds the path on disk).
fn main() {
    let _ = "intentional fixture";
}
