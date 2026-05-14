// Phase 6 (§3.2.1) discovery-corpus fixture: a macro invocation that
// would expand to a `TestCases::new().pass(...)` chain at compile time.
// The discovery walk operates on the source AST, not on the
// post-expansion token tree, so the call is NOT recognized — the
// visitor must continue past the macro without crashing.
//
// Adopters with macro-wrapped trybuild invocations have to register
// the wrapper via `--compat-trybuild-macro`, exactly like custom
// constructors. There is no AST-time signal that distinguishes
// "macro that wraps trybuild" from "macro that does anything else".

make_tests!();
