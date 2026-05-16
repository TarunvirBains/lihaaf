// Phase 6 (§3.2.1) discovery-corpus fixture: a `macro_rules!`
// definition is NOT a macro invocation — it is a local helper the
// adopter has declared for use within the same file or crate. The
// visitor must skip the definition without surfacing it as
// `discovery_unrecognized`. The expression-position invocation
// (`helper_macro!()`) lives inside a `#[test]` body and is therefore
// not at item-position, so the item-macro hook never sees it either.

macro_rules! helper_macro {
    () => {
        println!("hello");
    };
}

#[test]
fn use_it() {
    helper_macro!();
}
