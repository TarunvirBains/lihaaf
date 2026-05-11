//! Stderr normalization (spec §6).
//!
//! ## Why no regex
//!
//! Spec §6.1 mandates zero regex-engine deps. Every substitution here
//! is fixed-string with a known prefix. The cost of hand-rolling is
//! bounded — each substitution maps to a fixture, and validation
//! against a real-world consumer corpus is the safety net.
//!
//! ## Implementer choice — iteration recipe
//!
//! Spec §6.4 says "the implementer chooses data structures, iteration
//! order, and matching strategy subject to these contracts." This
//! module's design:
//!
//! 1. **One pass per line.** Line endings normalized to `\n` first
//!    (spec §6.2). Each line is then fed through every category in
//!    order.
//! 2. **Path categories use longest-prefix-wins.** The categories
//!    have explicit priority — `$WORKSPACE/target/release/deps/`
//!    matches before `$WORKSPACE/`. We order them by descending prefix
//!    length and short-circuit on first match.
//! 3. **TypeId rewrite is a separate byte-walk.** `#` followed by
//!    ASCII digits → `$TYPEID`. The walk uses `str::find('#')` as the
//!    fast path; the inner loop confirms the digit run.
//! 4. **Trailing whitespace + blank-line collapse run last** so they
//!    operate on the post-substitution shape.
//!
//! The `NormalizationContext` carries the path prefixes captured at
//! session startup. They are computed once per session and reused for
//! every fixture; only the fixture-directory prefix varies per fixture.

use std::path::{Path, PathBuf};

use crate::util;

/// Substring prefixes the normalizer rewrites to placeholders. Spec
/// §6.2.
#[derive(Debug, Clone)]
pub struct NormalizationContext {
    /// Workspace root (the `package.metadata.lihaaf` host crate's
    /// parent). Path prefixes equal to this are rewritten to
    /// `$WORKSPACE`.
    pub workspace_root: PathBuf,
    /// rustc sysroot (from `rustc --print sysroot`). Rewritten to
    /// `$RUST`.
    pub sysroot: PathBuf,
    /// `<CARGO_HOME>/registry/`. Rewritten to `$CARGO/registry/`.
    pub cargo_registry: Option<PathBuf>,
}

impl NormalizationContext {
    /// Construct a context from session-startup data. `cargo_home`
    /// defaults to `$CARGO_HOME` if set, otherwise `$HOME/.cargo`.
    pub fn new(workspace_root: PathBuf, sysroot: PathBuf) -> Self {
        let cargo_registry = std::env::var_os("CARGO_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cargo")))
            .map(|p| p.join("registry"));
        Self {
            workspace_root,
            sysroot,
            cargo_registry,
        }
    }
}

/// Normalize `input` for snapshot comparison.
///
/// `fixture_dir` is the directory containing the fixture `.rs` file;
/// path prefixes equal to it are rewritten to `$DIR`. `input` is the
/// raw stderr from rustc (already UTF-8; the caller has already
/// validated this).
///
/// ## No silent drops
///
/// Spec §6.2 enumerates the rewrite categories; spec §6.3 enumerates
/// what is explicitly preserved (diagnostic text, span pointers, help
/// text, suggestions). Neither list authorizes dropping the rustc
/// summary lines `error: aborting due to N previous error[s]` or
/// `For more information about this error, try \`rustc --explain ...\``.
/// Earlier drafts of this module dropped both — that was a Cluster 10.3
/// finding from the Codex Spark xhigh review (POST_BETA, handled here).
/// The summary lines are now preserved byte-for-byte; adopters whose
/// snapshots were blessed against the prior dropping behavior will need
/// to re-bless once, but the snapshot signal is no longer fighting the
/// adopter's reading of rustc's actual output.
pub fn normalize(input: &str, ctx: &NormalizationContext, fixture_dir: &Path) -> String {
    // Pre-compute placeholder list, longest prefix first. Adopters may
    // not have one of these (e.g., no CARGO_HOME); skip empties.
    let mut substitutions: Vec<(String, &'static str)> = Vec::new();
    push_path(&mut substitutions, fixture_dir, "$DIR");
    push_path(&mut substitutions, &ctx.workspace_root, "$WORKSPACE");
    push_path(&mut substitutions, &ctx.sysroot, "$RUST");
    if let Some(reg) = &ctx.cargo_registry {
        push_path(&mut substitutions, reg, "$CARGO/registry");
    }
    // Sort by descending source-string length so the longest prefix
    // wins (spec §6.4 longest-prefix-wins rule).
    substitutions.sort_by_key(|(needle, _)| std::cmp::Reverse(needle.len()));

    // Step 1: line endings.
    let unified_le = unify_line_endings(input);

    // Step 2: per-line path substitution + TypeId + trailing space.
    // Per spec §6.2 / §6.3 (and the Cluster 10.3 fix from the Codex
    // Spark xhigh review), rustc's summary lines (`error: aborting due
    // to N previous error[s]`, `For more information about this error,
    // try \`rustc --explain ...\``) are NOT dropped — they pass through
    // alongside every other diagnostic line and are subject only to the
    // normalization categories §6.2 enumerates.
    let mut intermediate: Vec<String> = Vec::with_capacity(unified_le.lines().count() + 1);
    for line in unified_le.lines() {
        let mut s = line.to_string();
        // Backslashes inside path-shaped substrings: spec §6.2 says we
        // rewrite "backslashes in paths" — restricted to `--> ` and
        // `::: ` lines (spec §6.5 documents the limitation). For the
        // path-prefix substitution we operate on a copy with the
        // backslashes pre-converted so the prefix match works on
        // either OS.
        if has_path_marker(&s) {
            s = rewrite_path_separators_in_path_lines(&s);
        }
        for (needle, repl) in &substitutions {
            // Replace every occurrence; spec §6.4 just says rewrite
            // matches. Using `str::replace` here would scan repeatedly
            // for already-replaced content; instead we walk left-to-
            // right, advancing past each replacement so we never
            // accidentally match inside the placeholder.
            s = replace_advancing(&s, needle, repl);
        }
        s = rewrite_type_ids(&s);
        // Trailing whitespace.
        let trimmed = s.trim_end_matches([' ', '\t']);
        intermediate.push(trimmed.to_string());
    }

    // Step 3: collapse runs of blank lines to a single blank line.
    let mut out = String::with_capacity(input.len());
    let mut prev_blank = false;
    for line in intermediate {
        let is_blank = line.is_empty();
        if is_blank && prev_blank {
            continue;
        }
        out.push_str(&line);
        out.push('\n');
        prev_blank = is_blank;
    }
    // Trim trailing blank lines (more than just one newline). Snapshots
    // shouldn't carry trailing whitespace; the snapshot writer adds the
    // final newline back.
    while out.ends_with('\n') {
        out.pop();
    }
    out
}

/// Pre-format a path as a string and push it onto the substitution
/// list along with its placeholder.
fn push_path(out: &mut Vec<(String, &'static str)>, p: &Path, placeholder: &'static str) {
    let s = util::to_forward_slash(&p.to_string_lossy());
    if s.is_empty() {
        return;
    }
    out.push((s, placeholder));
}

/// Replace all occurrences of `needle` with `repl` in `s`, walking
/// left-to-right so we never re-scan inside the placeholder. Allocates
/// once when matches exist; passes through cheaply when none do.
fn replace_advancing(s: &str, needle: &str, repl: &str) -> String {
    if needle.is_empty() {
        return s.to_string();
    }
    if !s.contains(needle) {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(idx) = rest.find(needle) {
        out.push_str(&rest[..idx]);
        out.push_str(repl);
        rest = &rest[idx + needle.len()..];
    }
    out.push_str(rest);
    out
}

/// Rewrite TypeId hashes (spec §6.4 final paragraph): every occurrence
/// of `#` followed by one or more ASCII digits is replaced with
/// `$TYPEID`.
fn rewrite_type_ids(s: &str) -> String {
    if !s.contains('#') {
        return s.to_string();
    }
    let bytes = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'#' && i + 1 < bytes.len() && bytes[i + 1].is_ascii_digit() {
            // Skip past `#` and the digit run.
            let mut j = i + 1;
            while j < bytes.len() && bytes[j].is_ascii_digit() {
                j += 1;
            }
            out.push_str("$TYPEID");
            i = j;
        } else {
            // Push one char (UTF-8 boundary safe). The byte at `i`
            // starts a UTF-8 sequence; we copy until the next char
            // boundary.
            let mut j = i + 1;
            while j < bytes.len() && (bytes[j] & 0xC0) == 0x80 {
                j += 1;
            }
            out.push_str(&s[i..j]);
            i = j;
        }
    }
    out
}

/// Unify CRLF / CR / LF to LF. Spec §6.2.
fn unify_line_endings(s: &str) -> String {
    if !s.contains('\r') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'\r' {
            out.push('\n');
            // Skip a following '\n' so CRLF doesn't produce two LFs.
            if i + 1 < bytes.len() && bytes[i + 1] == b'\n' {
                i += 2;
            } else {
                i += 1;
            }
        } else {
            // Copy one char.
            let mut j = i + 1;
            while j < bytes.len() && (bytes[j] & 0xC0) == 0x80 {
                j += 1;
            }
            out.push_str(&s[i..j]);
            i = j;
        }
    }
    out
}

/// True when a line looks like it carries a path (rustc's `--> ` or
/// `::: ` marker). Used to gate the backslash-to-slash rewrite per
/// spec §6.5.
fn has_path_marker(line: &str) -> bool {
    line.contains("--> ") || line.contains("::: ")
}

/// Rewrite backslashes to forward slashes within the path portion of a
/// `--> ` / `::: ` line. We only touch the substring after the marker
/// to avoid clobbering Windows-style paths that legitimately appear
/// inside string literals quoted in the diagnostic.
fn rewrite_path_separators_in_path_lines(line: &str) -> String {
    for marker in ["--> ", "::: "] {
        if let Some(idx) = line.find(marker) {
            let head_end = idx + marker.len();
            let head = &line[..head_end];
            let tail = &line[head_end..];
            return format!("{head}{}", util::to_forward_slash(tail));
        }
    }
    line.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(workspace: &str, sysroot: &str) -> NormalizationContext {
        NormalizationContext {
            workspace_root: PathBuf::from(workspace),
            sysroot: PathBuf::from(sysroot),
            cargo_registry: Some(PathBuf::from("/home/u/.cargo/registry")),
        }
    }

    #[test]
    fn rewrites_dir_prefix_then_workspace_prefix() {
        // rustc preserves indentation in path-marker lines as part of
        // diagnostic formatting. The normalizer does NOT strip leading
        // whitespace — only trailing (spec §6.2). The test fixture
        // mirrors rustc's two-space pad so adopters reading the test
        // corpus see the byte-equivalent shape.
        let input = "  --> /p/tests/lihaaf/compile_fail/foo.rs:3:1\n";
        let c = ctx("/p", "/home/u/.rustup/x");
        let dir = PathBuf::from("/p/tests/lihaaf/compile_fail");
        let out = normalize(input, &c, &dir);
        assert_eq!(out, "  --> $DIR/foo.rs:3:1");
    }

    #[test]
    fn longest_prefix_wins() {
        // `$WORKSPACE/tests/lihaaf/compile_fail/` and `$WORKSPACE/`
        // both match the same substring; `$DIR` is longer and must
        // resolve first. The pre-sort orders descending by length.
        let input = "  --> /p/tests/lihaaf/compile_fail/foo.rs:3:1\n  ::: /p/src/lib.rs:1:1\n";
        let c = ctx("/p", "/home/u/.rustup/x");
        let dir = PathBuf::from("/p/tests/lihaaf/compile_fail");
        let out = normalize(input, &c, &dir);
        let expected = "  --> $DIR/foo.rs:3:1\n  ::: $WORKSPACE/src/lib.rs:1:1";
        assert_eq!(out, expected);
    }

    #[test]
    fn rewrites_sysroot_prefix() {
        let input = "  ::: /home/u/.rustup/x/lib/core/src/option.rs:1:1\n";
        let c = ctx("/p", "/home/u/.rustup/x");
        let dir = PathBuf::from("/p/tests/lihaaf/compile_fail");
        let out = normalize(input, &c, &dir);
        assert_eq!(out, "  ::: $RUST/lib/core/src/option.rs:1:1");
    }

    #[test]
    fn type_id_rewrite_replaces_hash_digits() {
        let input = "expected `Foo#0`, found `Bar#42`\n";
        let c = ctx("/p", "/r");
        let dir = PathBuf::from("/p/x");
        let out = normalize(input, &c, &dir);
        assert_eq!(out, "expected `Foo$TYPEID`, found `Bar$TYPEID`");
    }

    #[test]
    fn type_id_does_not_touch_hash_without_digits() {
        let input = "see issue #[123] (a TODO comment)\n";
        let c = ctx("/p", "/r");
        let dir = PathBuf::from("/p/x");
        // `#[` is not `#<digit>` so it must pass through.
        let out = normalize(input, &c, &dir);
        assert_eq!(out, "see issue #[123] (a TODO comment)");
    }

    #[test]
    fn collapses_blank_line_runs() {
        let input = "alpha\n\n\n\nomega\n";
        let c = ctx("/p", "/r");
        let dir = PathBuf::from("/p/x");
        let out = normalize(input, &c, &dir);
        assert_eq!(out, "alpha\n\nomega");
    }

    #[test]
    fn strips_trailing_whitespace() {
        let input = "alpha   \nbeta\t\t\n";
        let c = ctx("/p", "/r");
        let dir = PathBuf::from("/p/x");
        let out = normalize(input, &c, &dir);
        assert_eq!(out, "alpha\nbeta");
    }

    #[test]
    fn unifies_crlf_and_lone_cr_to_lf() {
        let input = "a\r\nb\rc\nd\n";
        let c = ctx("/p", "/r");
        let dir = PathBuf::from("/p/x");
        let out = normalize(input, &c, &dir);
        assert_eq!(out, "a\nb\nc\nd");
    }

    #[test]
    fn does_not_touch_diagnostic_text() {
        let input = "error: unknown on_delete value `bogus`; expected one of: cascade\n";
        let c = ctx("/p", "/r");
        let dir = PathBuf::from("/p/x");
        let out = normalize(input, &c, &dir);
        assert_eq!(
            out,
            "error: unknown on_delete value `bogus`; expected one of: cascade"
        );
    }

    #[test]
    fn preserves_rustc_aborting_summary() {
        // Spec §6.2 / §6.3: the summary line is not in the rewrite
        // category list and is not in the explicit-preserve list either,
        // but §6.3 makes preservation the default ("Diagnostic text …
        // preserved byte-for-byte"). Earlier drafts dropped this line;
        // Cluster 10.3 of the Codex Spark xhigh review reverted that.
        let input = "error: bad\nerror: aborting due to 1 previous error\n";
        let c = ctx("/p", "/r");
        let dir = PathBuf::from("/p/x");
        let out = normalize(input, &c, &dir);
        assert_eq!(out, "error: bad\nerror: aborting due to 1 previous error");
    }

    #[test]
    fn preserves_rustc_aborting_plural() {
        let input = "error: a\nerror: b\nerror: aborting due to 42 previous errors\n";
        let c = ctx("/p", "/r");
        let dir = PathBuf::from("/p/x");
        let out = normalize(input, &c, &dir);
        assert_eq!(
            out,
            "error: a\nerror: b\nerror: aborting due to 42 previous errors"
        );
    }

    #[test]
    fn preserves_unrelated_aborting_text() {
        let input = "error: aborting due to user request\n";
        let c = ctx("/p", "/r");
        let dir = PathBuf::from("/p/x");
        let out = normalize(input, &c, &dir);
        assert_eq!(out, "error: aborting due to user request");
    }

    #[test]
    fn preserves_rustc_explain_pointer() {
        // Spec §6.2 / §6.3: the explain pointer is preserved byte-for-
        // byte. Earlier drafts dropped it; Cluster 10.3 of the Codex
        // Spark xhigh review reverted that.
        let input =
            "error: bad\n\nFor more information about this error, try `rustc --explain E0463`.\n";
        let c = ctx("/p", "/r");
        let dir = PathBuf::from("/p/x");
        let out = normalize(input, &c, &dir);
        assert_eq!(
            out,
            "error: bad\n\nFor more information about this error, try `rustc --explain E0463`."
        );
    }

    #[test]
    fn determinism_same_inputs_produce_same_bytes() {
        let input = "  --> /p/tests/lihaaf/compile_fail/foo.rs:3:1\n";
        let c = ctx("/p", "/r");
        let dir = PathBuf::from("/p/tests/lihaaf/compile_fail");
        let a = normalize(input, &c, &dir);
        let b = normalize(input, &c, &dir);
        assert_eq!(a, b);
    }
}
