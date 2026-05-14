//! Hand-rolled proc-macros for lihaaf's integration corpus.
//!
//! Stdlib-only — no `syn`, no `quote`, no `proc-macro2`. The macros are
//! deliberately minimal: each one exercises one verdict-class invariant
//! lihaaf's CI relies on (`OK`, `SNAPSHOT_DIFF`, `SNAPSHOT_MISSING`,
//! `LARGE_SNAPSHOT`, `TIMEOUT`, `MEMORY_EXHAUSTED`).
//!
//! Re-exported through the parent `integration_corpus` crate; fixtures
//! never name this crate directly. See `../../src/lib.rs`.

extern crate proc_macro;

use proc_macro::{Delimiter, Group, Ident, Literal, Punct, Spacing, Span, TokenStream, TokenTree};

/// No-op macro for `compile_pass/uses_corpus_noop.rs`. Validates that
/// the macro crate links + the dylib boundary is intact end-to-end.
#[proc_macro]
pub fn corpus_noop(_input: TokenStream) -> TokenStream {
    TokenStream::new()
}

/// Wrap the input in `compile_error!(<input>);`. The input is expected
/// to be a single string literal; we don't re-parse it, we pass it
/// through into the macro group verbatim. Used by
/// `compile_fail/corpus_error_basic.rs` and
/// `compile_fail/missing_snapshot.rs` to drive the SNAPSHOT_DIFF and
/// SNAPSHOT_MISSING verdict paths.
#[proc_macro]
pub fn corpus_error(input: TokenStream) -> TokenStream {
    let mut out: Vec<TokenTree> = Vec::new();
    out.push(TokenTree::Ident(Ident::new(
        "compile_error",
        Span::call_site(),
    )));
    out.push(TokenTree::Punct(Punct::new('!', Spacing::Alone)));
    out.push(TokenTree::Group(Group::new(Delimiter::Parenthesis, input)));
    out.push(TokenTree::Punct(Punct::new(';', Spacing::Alone)));
    TokenStream::from_iter(out)
}

/// Emit `compile_error!("line 0\nline 1\n...line N-1");` where N is a
/// positive integer literal parsed from input. Used by
/// `compile_fail/large_snapshot.rs` to drive a >10000-line diagnostic
/// (exceeds `crate::diff::SOFT_LINE_CEILING`) without embedding a
/// half-megabyte string literal in the fixture source.
#[proc_macro]
pub fn corpus_error_with_n_lines(input: TokenStream) -> TokenStream {
    let n_str = input.to_string();
    let n: usize = n_str.trim().parse().expect("integer literal");
    // Real newline characters — `Literal::string` escapes them as
    // `\n` in the emitted source representation, but rustc-the-parser
    // reads the resulting string literal back with actual newline
    // bytes. The `compile_error!` body therefore renders as N
    // separate lines in the diagnostic, which is what
    // `LARGE_SNAPSHOT` needs to fire (the diff is line-granular).
    let body: String = (0..n)
        .map(|i| format!("line {i}"))
        .collect::<Vec<_>>()
        .join("\n");
    let lit = Literal::string(&body);
    let mut out: Vec<TokenTree> = Vec::new();
    out.push(TokenTree::Ident(Ident::new(
        "compile_error",
        Span::call_site(),
    )));
    out.push(TokenTree::Punct(Punct::new('!', Spacing::Alone)));
    let mut grp_inner = TokenStream::new();
    grp_inner.extend(std::iter::once(TokenTree::Literal(lit)));
    out.push(TokenTree::Group(Group::new(Delimiter::Parenthesis, grp_inner)));
    out.push(TokenTree::Punct(Punct::new(';', Spacing::Alone)));
    TokenStream::from_iter(out)
}

/// Sleep the macro-expansion thread forever. lihaaf's per-fixture
/// timeout watchdog kills the worker after `fixture_timeout_secs` and
/// the verdict becomes `TIMEOUT`. Used by
/// `compile_pass/intentional_timeout.rs`.
#[proc_macro]
pub fn corpus_sleep_forever(_input: TokenStream) -> TokenStream {
    std::thread::sleep(std::time::Duration::MAX);
    // Unreachable — the worker is killed before this returns. Present
    // so the function signature compiles.
    TokenStream::new()
}

/// Allocate memory in a paced loop until lihaaf's RSS sampler trips
/// the `per_fixture_memory_mb` ceiling and kills the worker with
/// `MEMORY_EXHAUSTED`. Pacing matters: lihaaf samples RSS every
/// 100ms (`worker::spawn_and_monitor` poll cadence) and a tight loop
/// without sleep races the OS OOM killer, which would surface as
/// `WORKER_CRASHED` instead. The 50ms sleep gives the sampler at
/// least one tick per allocation.
///
/// Allocation rate: 64 MiB / 50 ms ≈ 1280 MiB/s. At
/// `per_fixture_memory_mb = 1024`, MEMORY_EXHAUSTED fires at ~0.8 s —
/// well under the 3 s TIMEOUT cap, so the two verdicts remain
/// deterministically distinct.
#[proc_macro]
pub fn corpus_oom_allocate(_input: TokenStream) -> TokenStream {
    // 64 MiB per iteration. At per_fixture_memory_mb=1024, the harness
    // should trip after ~16 iterations (~0.8 s at 50 ms/iter).
    //
    // Two implementation details matter for RSS attribution:
    //
    // 1. `vec![0u8; N]` takes the `alloc_zeroed` fast path: the kernel
    //    hands back an anonymous mapping that reads as zero but has no
    //    committed pages until something writes to it. RSS does NOT
    //    grow off the back of `vec![0u8; N]` alone. We have to actually
    //    write to every page, so we touch one byte per 4 KiB page in
    //    the iteration's freshly-allocated buffer. (Linux page size on
    //    every supported platform is <= 4 KiB except 16/64 KiB on some
    //    aarch64 servers; touching every 4096th byte over-samples on
    //    those, which is fine — it still forces commits.)
    //
    // 2. `keep_alive` retains every allocation across iterations so
    //    the allocator cannot recycle pages mid-loop. Without this
    //    the kernel may reclaim early iterations' pages before RSS
    //    crosses the ceiling.
    let mut keep_alive: Vec<Vec<u8>> = Vec::new();
    loop {
        let mut buf = vec![0u8; 64 * 1024 * 1024];
        let mut i = 0;
        while i < buf.len() {
            // Write a non-zero byte to force the kernel to commit a
            // real page rather than returning the shared zero page.
            buf[i] = 1;
            i += 4096;
        }
        keep_alive.push(buf);
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}
