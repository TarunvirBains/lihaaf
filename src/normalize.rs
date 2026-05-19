//! Stderr normalization for fixture output.
//!
//! ## Why no regex
//!
//! There are no regex dependencies here by design. Fixed-string matching is
//! enough for rustc diagnostics and keeps the dependency surface small.
//!
//! ## Implementation choices
//!
//! The module keeps the replacement flow simple and explicit:
//!
//! 1. **One pass per line.** Line endings are normalized to `\n` first.
//!    Each line is then run through each rewrite category.
//! 2. **Path categories use longest-prefix-wins.** `$WORKSPACE/target/release/deps/`
//!    matches should win over `$WORKSPACE/`. Prefixes are sorted by length.
//! 3. **TypeId rewrite is a separate byte-walk.** `#` followed by
//!    digits becomes `$TYPEID`.
//! 4. **Trailing whitespace + blank-line collapse run last** so they
//!    apply after other rewrites.
//!
//! The `NormalizationContext` carries the path prefixes captured at
//! session startup. They are computed once per session and reused for
//! every fixture; only the fixture-directory prefix varies per fixture.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::util;

/// One adopter-defined `extra_substitutions` entry.
///
/// `from` is the literal-substring needle (gated by `is_path_like`
/// at config-parse time — see [`crate::config`] §3.3); `to` is the
/// replacement (subject only to the no-newline rule, per Decision #3
/// / OQ-D — adopters legitimately need `to = ""` (strip-via-
/// substitute), `to = "$RUST"`, and compound paths). The needle is
/// matched left-to-right, advancing past each match so already-
/// rewritten bytes do not re-match (same `replace_advancing` shape as
/// the built-in path substitutions).
///
/// Validation rules are in [`crate::config`]; this struct is the
/// validated typed shape passed into the normalizer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Substitution {
    /// Literal-substring needle. Allowlist-gated to be path-shaped
    /// (`is_path_like`) at config-parse time so `from` cannot rewrite
    /// arbitrary diagnostic text.
    pub from: String,
    /// Literal-substring replacement. Subject only to no-newline
    /// validation; otherwise unconstrained per Decision #3 / OQ-D.
    pub to: String,
}

/// Substring prefixes the normalizer rewrites to placeholders.
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
    /// Compat-mode flag (§3.2.2). When `true`, the normalizer emits
    /// trybuild's shorter `$CARGO/<crate>-<ver>/<rest>` form instead of
    /// the literal-prefix `$CARGO/registry/src/index.crates.io-<hash>/...`
    /// form. Default `false`; non-compat callers leave it `false` and
    /// observe byte-identical v0.1 output.
    pub compat_short_cargo: bool,
    /// Adopter-defined `extra_substitutions` (per-suite, REPLACE
    /// semantics — see `docs/spec/lihaaf-v0.1.md` §3.6). Applied
    /// left-to-right in declared order, AFTER built-in path
    /// substitutions and BEFORE TypeId collapse. Empty by default;
    /// when empty, normalizer output is byte-identical to v0.1.0-beta.9.
    pub extra_substitutions: Vec<Substitution>,
    /// Adopter-defined `strip_lines` (per-suite, REPLACE semantics).
    /// Full-line exact-match drops applied after trim-trailing-whitespace
    /// and before blank-line collapse. Empty by default. Each entry
    /// must be `is_path_like` OR `is_banner_shape` per config-parse
    /// validation — see `docs/spec/lihaaf-v0.1.md` §6.6.
    pub strip_lines: Vec<String>,
    /// Adopter-defined `strip_line_prefixes` (per-suite, REPLACE
    /// semantics). Prefix-match drops applied after trim-trailing-
    /// whitespace and before blank-line collapse. Empty by default.
    /// Each entry must be `is_path_like` OR `is_banner_shape` per
    /// config-parse validation.
    pub strip_line_prefixes: Vec<String>,
}

impl NormalizationContext {
    /// Construct a context from session-startup data. `cargo_home`
    /// defaults to `$CARGO_HOME` if set, otherwise `$HOME/.cargo`.
    ///
    /// `compat_short_cargo` is `false` by default. Compat-mode callers
    /// drive the flag via the dedicated [`Self::with_compat_short_cargo`]
    /// builder so the §3.2.2 trybuild short-form rewrite fires for the
    /// inner session; non-compat callers leave it untouched and observe
    /// byte-identical v0.1 output.
    ///
    /// The three adopter-defined override fields ([`Self::extra_substitutions`],
    /// [`Self::strip_lines`], [`Self::strip_line_prefixes`]) default to
    /// empty `Vec`s. Callers that need to wire per-suite values use the
    /// dedicated builders ([`Self::with_extra_substitutions`],
    /// [`Self::with_strip_lines`], [`Self::with_strip_line_prefixes`]).
    /// When all three are empty (the legacy path), normalizer output is
    /// byte-identical to v0.1.0-beta.9.
    pub fn new(workspace_root: PathBuf, sysroot: PathBuf) -> Self {
        let cargo_registry = std::env::var_os("CARGO_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cargo")))
            .map(|p| p.join("registry"));
        Self {
            workspace_root,
            sysroot,
            cargo_registry,
            compat_short_cargo: false,
            extra_substitutions: Vec::new(),
            strip_lines: Vec::new(),
            strip_line_prefixes: Vec::new(),
        }
    }

    /// Builder-style mutator to set [`Self::compat_short_cargo`].
    ///
    /// Returns `self` so call sites can read as
    /// `NormalizationContext::new(...).with_compat_short_cargo(true)`.
    /// Setting `false` is a no-op on a context built via `new` (the
    /// default), preserved for symmetry with future overrides.
    pub fn with_compat_short_cargo(mut self, enabled: bool) -> Self {
        self.compat_short_cargo = enabled;
        self
    }

    /// Builder-style mutator to set [`Self::extra_substitutions`].
    ///
    /// Returns `self` so call sites can chain with other builders. The
    /// caller is responsible for having run config-parse validation
    /// (`is_path_like` on each `from`, no newline in `to`) before
    /// reaching here — the normalizer does not re-validate.
    pub fn with_extra_substitutions(mut self, subs: Vec<Substitution>) -> Self {
        self.extra_substitutions = subs;
        self
    }

    /// Builder-style mutator to set [`Self::strip_lines`].
    ///
    /// Returns `self` so call sites can chain with other builders. The
    /// caller is responsible for having run config-parse validation
    /// (`is_path_like || is_banner_shape` on each entry) before
    /// reaching here.
    pub fn with_strip_lines(mut self, lines: Vec<String>) -> Self {
        self.strip_lines = lines;
        self
    }

    /// Builder-style mutator to set [`Self::strip_line_prefixes`].
    ///
    /// Returns `self` so call sites can chain with other builders. The
    /// caller is responsible for having run config-parse validation
    /// (`is_path_like || is_banner_shape` on each entry) before
    /// reaching here.
    pub fn with_strip_line_prefixes(mut self, prefixes: Vec<String>) -> Self {
        self.strip_line_prefixes = prefixes;
        self
    }
}

/// Normalize `input` for snapshot comparison.
///
/// `fixture_dir` is the directory containing the fixture `.rs` file.
/// Prefixes there are rewritten to `$DIR`. `input` is the raw rustc
/// stderr (already UTF-8 by this stage).
///
/// ## No silent drops
///
/// the policy enumerates the rewrite categories; the policy enumerates
/// what is explicitly preserved (diagnostic text, span pointers, help
/// text, suggestions). Neither list authorizes dropping the rustc
/// summary lines `error: aborting due to N previous error[s]` or
/// `For more information about this error, try \`rustc --explain ...\``.
/// Earlier drafts dropped both lines; they are now preserved byte-for-byte.
/// Adopters with previously blessed snapshots may need one re-bless, but the
/// output now mirrors rustc’s real messages more closely.
pub fn normalize(input: &str, ctx: &NormalizationContext, fixture_dir: &Path) -> String {
    // Pre-compute placeholder list, longest prefix first. Adopters may
    // not have one of these (e.g., no CARGO_HOME); skip empties.
    let mut substitutions: Vec<(String, &'static str)> = Vec::new();
    push_path(&mut substitutions, fixture_dir, "$DIR");
    push_path(&mut substitutions, &ctx.workspace_root, "$WORKSPACE");
    push_path(&mut substitutions, &ctx.sysroot, "$RUST");
    // The `$CARGO/registry` literal substitution is only pushed in v0.1
    // stable mode. In compat mode (§3.2.2) the registry rewrite runs as
    // a structural post-pass — see `rewrite_cargo_short` below — because
    // the trybuild short form `$CARGO/<crate>-<ver>/<rest>` requires
    // hash-shape recognition that does not fit the literal-prefix loop.
    if let Some(reg) = &ctx.cargo_registry
        && !ctx.compat_short_cargo
    {
        push_path(&mut substitutions, reg, "$CARGO/registry");
    }
    // Sort by descending source-string length so the longest prefix
    // wins (the policy longest-prefix-wins rule).
    substitutions.sort_by_key(|(needle, _)| std::cmp::Reverse(needle.len()));

    // Step 1: line endings.
    let unified_le = unify_line_endings(input);

    // Step 2: per-line path substitution + TypeId + trailing space.
    // Per the policy (and the Cluster 10.3 fix from the Codex
    // Spark xhigh review), rustc's summary lines (`error: aborting due
    // to N previous error[s]`, `For more information about this error,
    // try \`rustc --explain ...\``) are NOT dropped — they pass through
    // alongside every other diagnostic line and are subject only to the
    // normalization categories the policy enumerates.
    let mut intermediate: Vec<String> = Vec::with_capacity(unified_le.lines().count() + 1);
    for line in unified_le.lines() {
        let mut s = line.to_string();
        // Backslashes inside path-shaped substrings: the policy says to
        // rewrite "backslashes in paths" — restricted to `--> ` and
        // `::: ` lines (the policy documents the limitation). For the
        // path-prefix substitution, a copy with backslashes pre-converted
        // is used so the prefix match works on either OS.
        if has_path_marker(&s) {
            s = rewrite_path_separators_in_path_lines(&s);
        }
        // rustc's "long-type written to" note carries a target-dir +
        // session-dir absolute path with a per-type hash in the
        // filename. Collapse the whole quoted path to a single
        // placeholder BEFORE path-prefix substitution so the inner
        // path is replaced atomically and partial `$WORKSPACE` /
        // `$DIR` matches don't leak through.
        s = rewrite_long_type_note_path(&s);
        for (needle, repl) in &substitutions {
            // Replace every occurrence; the policy just says rewrite
            // matches. Using `str::replace` here would scan repeatedly
            // for already-replaced content; instead the walk goes left-to-
            // right, advancing past each replacement so no accidental
            // match occurs inside the placeholder.
            s = replace_advancing(&s, needle, repl);
        }
        // Compat-mode post-pass (§3.2.2): rewrite
        // `<cargo_registry>/src/<host>-<16hex>/` to `$CARGO/` so the
        // output matches trybuild's short form
        // `$CARGO/<crate>-<ver>/<rest>`. Runs AFTER the literal $DIR /
        // $WORKSPACE / $RUST substitutions so the longest-prefix-wins
        // invariant on those three placeholders is intact, and only
        // when the compat flag is set with a known registry root —
        // non-compat callers see byte-identical v0.1 output.
        if ctx.compat_short_cargo
            && let Some(reg) = &ctx.cargo_registry
        {
            s = rewrite_cargo_short(&s, reg);
        }
        // Step 7: adopter-defined `extra_substitutions` (issue #45 /
        // beta.10). Applied AFTER built-in path substitutions and
        // (when compat is on) the short-CARGO post-pass — so adopter
        // rules can refer to the post-built-in placeholders (e.g.,
        // `{ from = "$RUST/lib/rust-1.95.0", to = "$RUST" }`) — and
        // BEFORE TypeId collapse so any introduced `#<digits>`
        // collapses on the same line (§4). Entries are validated
        // upstream (`is_path_like` on `from`, no newline in `to`) —
        // no per-line revalidation. `replace_advancing` matches the
        // built-in-substitution shape so already-rewritten bytes
        // don't re-match.
        for sub in &ctx.extra_substitutions {
            s = replace_advancing(&s, &sub.from, &sub.to);
        }
        s = rewrite_type_ids(&s);
        // Trailing whitespace.
        let trimmed = s.trim_end_matches([' ', '\t']);
        intermediate.push(trimmed.to_string());
    }

    // Step 10: adopter-defined line-drop (`strip_lines` /
    // `strip_line_prefixes`, issue #45 / beta.10). Runs AFTER per-
    // line trim-trailing-whitespace (so strip patterns match logical
    // content) and BEFORE blank-line collapse (so a stripped line
    // participates in the collapse — adjacent blank lines stay
    // collapsed even when a non-blank line between them gets
    // dropped). Empty allow-vectors short-circuit each line to a
    // simple push so the absent-config path remains byte-identical
    // to beta.9.
    let mut after_strip: Vec<String> = Vec::with_capacity(intermediate.len());
    for line in intermediate {
        if should_strip_line(&line, &ctx.strip_lines, &ctx.strip_line_prefixes) {
            continue;
        }
        after_strip.push(line);
    }

    // Step 3: collapse runs of blank lines to a single blank line.
    let mut out = String::with_capacity(input.len());
    let mut prev_blank = false;
    for line in after_strip {
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

/// Test whether `line` matches any adopter-defined drop rule from
/// `strip_lines` (full-line exact equality) or `strip_line_prefixes`
/// (prefix match).
///
/// Both rule sets are validated upstream at config-parse time
/// (`is_path_like || is_banner_shape`) — see `crate::config` §3.3.
/// On the absent-config (empty-vector) path the two `iter().any`
/// calls collapse to constant `false` so the legacy normalizer
/// output is byte-identical to v0.1.0-beta.9.
fn should_strip_line(line: &str, exact: &[String], prefixes: &[String]) -> bool {
    if exact.iter().any(|e| e == line) {
        return true;
    }
    if prefixes.iter().any(|p| line.starts_with(p.as_str())) {
        return true;
    }
    false
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
/// left-to-right, never re-scanning inside the placeholder. Allocates
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

/// Compat-mode rewrite (§3.2.2): replace
/// `<cargo_registry>/src/<host>-<16 lowercase hex>/` with `$CARGO/`,
/// emitting trybuild's short form `$CARGO/<crate>-<ver>/<rest>`.
///
/// Recognized hosts (mirrors `trybuild/src/normalize.rs:305-306`):
///   - `github.com-<16 lowercase hex>` (cargo ≤ 1.69, `[source]` overrides)
///   - `index.crates.io-<16 lowercase hex>` (cargo ≥ 1.70 sparse registry)
///
/// All three structural conditions must hold for a match:
///   1. The byte sequence `<cargo_registry>/src/<host>-` appears.
///   2. The 16 bytes after the dash are ASCII lowercase hex (matches
///      `b'0'..=b'9' | b'a'..=b'f'`).
///   3. The byte after the 16-hex sequence is `/`.
///
/// On match, `<cargo_registry>/src/<host>-<hash>` is replaced with
/// `$CARGO`, leaving `/<crate>-<ver>/<rest>` intact. The line is
/// scanned left to right, advancing past each replacement so
/// already-rewritten bytes don't re-match. Non-matching content
/// passes through unchanged.
///
/// No regex (§6.1). Pure byte-walk.
fn rewrite_cargo_short(s: &str, cargo_registry: &Path) -> String {
    // Build the two full prefix forms once per call:
    //   `<cargo_registry>/src/github.com-`
    //   `<cargo_registry>/src/index.crates.io-`
    // Slash-normalize so the match works on either OS (the registry
    // path may have been captured as a native Windows path; rustc's
    // emitted diagnostics typically use forward slashes, and the
    // surrounding pipeline already normalizes `--> ` / `::: ` lines —
    // applying the same normalization here keeps the prefix shape
    // aligned with the line's shape).
    let registry = util::to_forward_slash(&cargo_registry.to_string_lossy());
    if registry.is_empty() {
        return s.to_string();
    }
    let middles: [String; 2] = [
        format!("{registry}/src/github.com-"),
        format!("{registry}/src/index.crates.io-"),
    ];
    const HEX_LEN: usize = 16;

    // Fast path — no candidate prefix anywhere in the line. Avoids
    // an allocation on the common case (most diagnostic lines never
    // mention a registry path at all).
    if !middles.iter().any(|m| s.contains(m)) {
        return s.to_string();
    }

    let bytes = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < bytes.len() {
        // Try every recognized middle at this position. Match shape:
        //   `<middle><16 lowercase hex>/`
        let mut consumed = 0usize;
        for middle in &middles {
            let m = middle.as_bytes();
            if i + m.len() + HEX_LEN + 1 > bytes.len() {
                continue;
            }
            if &bytes[i..i + m.len()] != m {
                continue;
            }
            let hex_start = i + m.len();
            let hex_end = hex_start + HEX_LEN;
            let hex = &bytes[hex_start..hex_end];
            if !hex.iter().all(|&b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
                continue;
            }
            if bytes[hex_end] != b'/' {
                continue;
            }
            // Full structural match: `<registry>/src/<host>-<16hex>`
            // is replaced with `$CARGO`. The trailing `/` is left for
            // the copy loop so the output reads `$CARGO/<crate>-<ver>/...`.
            out.push_str("$CARGO");
            consumed = m.len() + HEX_LEN;
            break;
        }
        if consumed > 0 {
            i += consumed;
            continue;
        }
        // No match at this offset — copy one char (UTF-8 boundary safe;
        // the byte at `i` starts a UTF-8 sequence so the continuation
        // bytes `(b & 0xC0) == 0x80` follow).
        let mut j = i + 1;
        while j < bytes.len() && (bytes[j] & 0xC0) == 0x80 {
            j += 1;
        }
        out.push_str(&s[i..j]);
        i = j;
    }
    out
}

/// Rewrite TypeId hashes (the policy final paragraph): every occurrence
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
            // starts a UTF-8 sequence; copy until the next char
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

/// Unify CRLF / CR / LF to LF. the policy.
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
/// the policy.
fn has_path_marker(line: &str) -> bool {
    line.contains("--> ") || line.contains("::: ")
}

/// Rewrite the volatile path inside rustc's "long-type written to"
/// note to a stable `$LONGTYPE_FILE` placeholder.
///
/// rustc emits this note when a type is too large to render inline:
///
/// ```text
///     = note: the full name for the type has been written to '<path>'
/// ```
///
/// (newer rustc versions phrase the same note as `the full type name
/// has been written to '<path>'`). The path points at a spillover file
/// inside lihaaf's per-session target sub-tree
/// (`<target>/lihaaf-session-<rand>/<fixture-name>/<fixture>.long-type-<u64-hash>.txt`).
/// Every component after the target root — the session-dir random
/// suffix and the per-type hash in the filename — changes every run,
/// so the raw note diff-fails against any blessed snapshot.
///
/// This rewrite collapses the entire quoted path to `$LONGTYPE_FILE`,
/// preserving the surrounding note text (so adopters still see what
/// rustc reported). Both note phrasings are accepted so a rustc
/// release that swaps the wording doesn't force a re-bless.
///
/// The path is unreachable from the snapshot anyway (it lives only in
/// the originating session's tempdir, often already cleaned up by the
/// time anyone reads the snapshot), so collapsing it loses no
/// actionable information.
fn rewrite_long_type_note_path(line: &str) -> String {
    // Two phrasings rustc has emitted historically. Match order is
    // longest-first so a future variant that extends the prefix won't
    // accidentally shadow a shorter match.
    const MARKERS: &[&str] = &[
        "the full name for the type has been written to '",
        "the full type name has been written to '",
    ];
    for marker in MARKERS {
        let Some(prefix_idx) = line.find(marker) else {
            continue;
        };
        let after_quote = prefix_idx + marker.len();
        // Find the closing single quote that terminates the path.
        // If rustc ever emits an unterminated quote, pass the line
        // through unchanged rather than guess where the path ends.
        let Some(close_rel) = line[after_quote..].find('\'') else {
            return line.to_string();
        };
        let close_abs = after_quote + close_rel;
        let mut out = String::with_capacity(line.len());
        out.push_str(&line[..after_quote]);
        out.push_str("$LONGTYPE_FILE");
        out.push_str(&line[close_abs..]);
        return out;
    }
    line.to_string()
}

/// Rewrite backslashes to forward slashes within the path portion of a
/// `--> ` / `::: ` line. Only the substring after the marker is touched,
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
            compat_short_cargo: false,
            extra_substitutions: Vec::new(),
            strip_lines: Vec::new(),
            strip_line_prefixes: Vec::new(),
        }
    }

    /// Run `normalize` against `input` with the standard "/p" workspace,
    /// "/r" sysroot, and "/p/x" fixture-directory triplet, and assert the
    /// output is byte-equal to `expected`. Used by the cluster of
    /// text-handling tests below whose only varying inputs are `input` and
    /// `expected` — the path-rewriting tests that need a non-default
    /// workspace/sysroot/dir keep their own setup.
    fn assert_normalizes(input: &str, expected: &str) {
        let c = ctx("/p", "/r");
        let dir = PathBuf::from("/p/x");
        let out = normalize(input, &c, &dir);
        assert_eq!(out, expected);
    }

    #[test]
    fn rewrites_dir_prefix_then_workspace_prefix() {
        // rustc preserves indentation in path-marker lines as part of
        // diagnostic formatting. The normalizer does NOT strip leading
        // whitespace — only trailing (the policy). The test fixture
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
        assert_normalizes(
            "expected `Foo#0`, found `Bar#42`\n",
            "expected `Foo$TYPEID`, found `Bar$TYPEID`",
        );
    }

    #[test]
    fn type_id_does_not_touch_hash_without_digits() {
        // `#[` is not `#<digit>` so it must pass through.
        assert_normalizes(
            "see issue #[123] (a TODO comment)\n",
            "see issue #[123] (a TODO comment)",
        );
    }

    #[test]
    fn collapses_blank_line_runs() {
        assert_normalizes("alpha\n\n\n\nomega\n", "alpha\n\nomega");
    }

    #[test]
    fn strips_trailing_whitespace() {
        assert_normalizes("alpha   \nbeta\t\t\n", "alpha\nbeta");
    }

    #[test]
    fn unifies_crlf_and_lone_cr_to_lf() {
        assert_normalizes("a\r\nb\rc\nd\n", "a\nb\nc\nd");
    }

    #[test]
    fn does_not_touch_diagnostic_text() {
        assert_normalizes(
            "error: unknown on_delete value `bogus`; expected one of: cascade\n",
            "error: unknown on_delete value `bogus`; expected one of: cascade",
        );
    }

    #[test]
    fn preserves_rustc_aborting_summary() {
        // This summary line is not in the rewrite category list and is not
        // in the explicit-preserve list either, but preservation stays the
        // default ("Diagnostic text …").
        // preserved byte-for-byte"). Earlier drafts dropped this line;
        // Cluster 10.3 of the Codex Spark xhigh review reverted that.
        assert_normalizes(
            "error: bad\nerror: aborting due to 1 previous error\n",
            "error: bad\nerror: aborting due to 1 previous error",
        );
    }

    #[test]
    fn preserves_rustc_aborting_plural() {
        assert_normalizes(
            "error: a\nerror: b\nerror: aborting due to 42 previous errors\n",
            "error: a\nerror: b\nerror: aborting due to 42 previous errors",
        );
    }

    #[test]
    fn preserves_unrelated_aborting_text() {
        assert_normalizes(
            "error: aborting due to user request\n",
            "error: aborting due to user request",
        );
    }

    #[test]
    fn preserves_rustc_explain_pointer() {
        // The explain pointer is preserved byte-for-byte. Earlier drafts
        // dropped it; Codex Spark review reverted that.
        assert_normalizes(
            "error: bad\n\nFor more information about this error, try `rustc --explain E0463`.\n",
            "error: bad\n\nFor more information about this error, try `rustc --explain E0463`.",
        );
    }

    #[test]
    fn determinism_same_inputs_produce_same_bytes() {
        // Per plan §7.4: this test was extended in #45 / beta.10 with a
        // non-empty `extra_substitutions` + `strip_lines` +
        // `strip_line_prefixes` triple that exercises BOTH allowlist
        // predicates (one path-shaped + one banner-shaped entry in each
        // strip key). Determinism guarantee covers the full normalizer
        // surface including adopter-defined override paths, not just the
        // built-in path-substitution loop.
        let input = "\
  --> /p/tests/lihaaf/compile_fail/foo.rs:3:1
/nix/store/abc123-rust-1.95.0/lib/rustlib/x.rs:1:1
/build/sandbox/internal/wrappers/cc-wrapper-1.0
error: aborting due to 1 previous error

For more information about this error, try `rustc --explain E0277`.
$WORKSPACE/.cargo-cache/dropped
";
        let mut c = ctx("/p", "/r");
        c.extra_substitutions = vec![Substitution {
            from: "/nix/store/abc123-rust-1.95.0/lib/rustlib".into(),
            to: "$RUST/lib/rustlib".into(),
        }];
        c.strip_lines = vec![
            "/build/sandbox/internal/wrappers/cc-wrapper-1.0".into(),
            "error: aborting due to 1 previous error".into(),
        ];
        c.strip_line_prefixes = vec![
            "$WORKSPACE/.cargo-cache/".into(),
            "For more information about this error".into(),
        ];
        let dir = PathBuf::from("/p/tests/lihaaf/compile_fail");
        let a = normalize(input, &c, &dir);
        let b = normalize(input, &c, &dir);
        assert_eq!(a, b);
    }

    // ---- rustc "long-type written to" note normalization ----
    //
    // When a Rust type is too large to render inline, rustc spills the
    // full type name into a sibling text file in its build target dir
    // and emits a note pointing at it:
    //
    //   = note: the full name for the type has been written to
    //     '<target>/.../<fixture>.long-type-<hash>.txt'
    //
    // Both the path prefix (target dir + lihaaf session-scoped sub-dir
    // with a random suffix) and the trailing `<hash>` are session-local
    // and change every run, so the raw note byte-diffs against any
    // blessed snapshot. The normalizer collapses the whole quoted path
    // to a single stable placeholder so adopters can bless once and
    // re-run the suite anywhere — different target dir, different
    // session, different rustc build of the type table — without
    // re-blessing.

    #[test]
    fn long_type_note_two_sessions_normalize_to_same_bytes() {
        // Reproduces the exact failure from the djogi validation rerun:
        // two real lihaaf sessions produced two different paths for the
        // same fixture's long-type spillover note (different target
        // root, different session dir suffix, different type hash).
        // After normalization both inputs must be byte-identical.
        let session_a = "     = note: the full name for the type has been written to '/tmp/phase85-orchestration/lihaaf-djogi-validation/target/lihaaf-session-NqO1Du/tests_lihaaf_compile_fail_sealed_into_distinct_columns.rs/sealed_into_distinct_columns.long-type-13784649802967031202.txt'\n";
        let session_b = "     = note: the full name for the type has been written to '/tmp/phase85-targets/djogi-lihaaf/lihaaf-session-b8ldWS/tests_lihaaf_compile_fail_sealed_into_distinct_columns.rs/sealed_into_distinct_columns.long-type-3815226114102655174.txt'\n";
        let c = ctx("/p", "/r");
        let dir = PathBuf::from("/p/tests/lihaaf/compile_fail");
        let out_a = normalize(session_a, &c, &dir);
        let out_b = normalize(session_b, &c, &dir);
        assert_eq!(
            out_a, out_b,
            "two sessions' long-type notes must normalize identically:\n  a = {out_a:?}\n  b = {out_b:?}",
        );
        // Placeholder is embedded; raw volatile substrings are gone.
        assert!(
            out_a.contains("$LONGTYPE_FILE"),
            "expected $LONGTYPE_FILE placeholder, got: {out_a:?}",
        );
        assert!(
            !out_a.contains("lihaaf-session-"),
            "session-dir suffix must be normalized away, got: {out_a:?}",
        );
        assert!(
            !out_a.contains("13784649802967031202"),
            "type-hash digits from session a must be normalized away: {out_a:?}",
        );
        assert!(
            !out_b.contains("3815226114102655174"),
            "type-hash digits from session b must be normalized away: {out_b:?}",
        );
        // The note prefix stays so the snapshot remains legible.
        assert!(
            out_a.contains("the full name for the type has been written to"),
            "primary note text must be preserved, got: {out_a:?}",
        );
    }

    #[test]
    fn long_type_note_normalizes_alternative_phrasing() {
        // Some rustc versions phrase the note as "the full type name has
        // been written to ..." instead. Both forms must collapse to the
        // same placeholder so adopters don't re-bless on a rustc upgrade
        // that only swaps the wording.
        let line = "     = note: the full type name has been written to '/var/folders/abc/T/lihaaf-session-xyz/foo.long-type-9999.txt'\n";
        let c = ctx("/p", "/r");
        let dir = PathBuf::from("/p/x");
        let out = normalize(line, &c, &dir);
        assert!(
            out.contains("$LONGTYPE_FILE"),
            "expected $LONGTYPE_FILE placeholder, got: {out:?}",
        );
        assert!(
            !out.contains("lihaaf-session-xyz"),
            "session-dir suffix must be normalized away: {out:?}",
        );
        assert!(
            !out.contains("9999"),
            "type-hash digits must be normalized away: {out:?}",
        );
        assert!(
            out.contains("the full type name has been written to"),
            "alt-phrasing note text must be preserved: {out:?}",
        );
    }

    #[test]
    fn long_type_note_preserves_surrounding_diagnostic() {
        // Primary diagnostic and the secondary `--verbose` hint must
        // survive byte-for-byte (spec §6.3 — preserve diagnostic text).
        // Only the quoted path is rewritten.
        let input = "\
error[E0277]: the trait bound is not satisfied
  --> /p/tests/foo.rs:1:1
   |
1  | bad code here
   | ^^^
   = note: the full name for the type has been written to '/tmp/x/lihaaf-session-AbCdEf/foo.long-type-12345.txt'
   = note: consider using `--verbose` to print the full type name to the console

error: aborting due to 1 previous error
";
        let c = ctx("/p", "/r");
        let dir = PathBuf::from("/p/tests");
        let out = normalize(input, &c, &dir);
        assert!(
            out.contains("error[E0277]: the trait bound is not satisfied"),
            "primary error code+message must be preserved: {out:?}",
        );
        assert!(
            out.contains("consider using `--verbose`"),
            "secondary `--verbose` hint must be preserved: {out:?}",
        );
        assert!(
            out.contains("error: aborting due to 1 previous error"),
            "rustc summary line must be preserved: {out:?}",
        );
        assert!(
            out.contains("$LONGTYPE_FILE"),
            "long-type path must be normalized to placeholder: {out:?}",
        );
        assert!(
            !out.contains("lihaaf-session-AbCdEf"),
            "volatile session dir must be normalized away: {out:?}",
        );
        assert!(
            !out.contains("long-type-12345"),
            "type-hash digits must be normalized away: {out:?}",
        );
    }

    #[test]
    fn long_type_note_left_intact_when_no_match() {
        // Note lines that *don't* carry the long-type marker pass
        // through untouched. Specifically, the `--verbose` hint that
        // rustc emits on the same diagnostic must not be confused with
        // a long-type note even though it shares the "note:" prefix.
        assert_normalizes(
            "   = note: consider using `--verbose` to print the full type name to the console\n",
            "   = note: consider using `--verbose` to print the full type name to the console",
        );
    }

    // ---- §3.2.2 compat-mode short-$CARGO rewrite (cases a-d) ----
    //
    // The cases below mirror the spec's named test list at the bottom
    // of §3.2.2. They use the public `NormalizationContext` directly so
    // the assertions sit against the byte-for-byte normalizer output.
    // The cross-module integration variants live in
    // `tests/compat/normalizer_compat_cargo.rs`.

    /// Build a compat-mode context (flag on). The sysroot is set to a
    /// path that cannot substring-collide with the input lines (the
    /// shared `ctx()` helper uses `/r` which happens to live inside the
    /// substring `/registry`; that's harmless for the existing tests
    /// but would corrupt our $CARGO assertions here).
    fn ctx_compat(workspace: &str, sysroot: &str) -> NormalizationContext {
        NormalizationContext {
            workspace_root: PathBuf::from(workspace),
            sysroot: PathBuf::from(sysroot),
            cargo_registry: Some(PathBuf::from("/home/u/.cargo/registry")),
            compat_short_cargo: true,
            extra_substitutions: Vec::new(),
            strip_lines: Vec::new(),
            strip_line_prefixes: Vec::new(),
        }
    }

    fn ctx_non_compat_no_collision(workspace: &str, sysroot: &str) -> NormalizationContext {
        NormalizationContext {
            workspace_root: PathBuf::from(workspace),
            sysroot: PathBuf::from(sysroot),
            cargo_registry: Some(PathBuf::from("/home/u/.cargo/registry")),
            compat_short_cargo: false,
            extra_substitutions: Vec::new(),
            strip_lines: Vec::new(),
            strip_line_prefixes: Vec::new(),
        }
    }

    #[test]
    fn compat_a_index_crates_io_rewrites_to_short_form() {
        let input = "  --> /home/u/.cargo/registry/src/index.crates.io-1234567890abcdef/foo-1.0.0/src/lib.rs:3:1\n";
        let c = ctx_compat("/p", "/sysroot");
        let dir = PathBuf::from("/p/x");
        let out = normalize(input, &c, &dir);
        assert_eq!(out, "  --> $CARGO/foo-1.0.0/src/lib.rs:3:1");
    }

    #[test]
    fn compat_b_github_com_handled_identically() {
        let input = "  --> /home/u/.cargo/registry/src/github.com-1234567890abcdef/foo-1.0.0/src/lib.rs:3:1\n";
        let c = ctx_compat("/p", "/sysroot");
        let dir = PathBuf::from("/p/x");
        let out = normalize(input, &c, &dir);
        assert_eq!(out, "  --> $CARGO/foo-1.0.0/src/lib.rs:3:1");
    }

    #[test]
    fn compat_c_line_without_registry_segment_unchanged() {
        // No `/registry/src/...` anywhere — line passes through the
        // post-pass untouched. The other three placeholders ($DIR /
        // $WORKSPACE / $RUST) still apply.
        let input = "  --> /p/tests/foo.rs:3:1\n";
        let c = ctx_compat("/p", "/sysroot");
        let dir = PathBuf::from("/p/tests");
        let out = normalize(input, &c, &dir);
        assert_eq!(out, "  --> $DIR/foo.rs:3:1");
    }

    #[test]
    fn compat_d_flag_off_byte_identical_to_v0_1() {
        // With `compat_short_cargo = false`, the literal-prefix path
        // (`$CARGO/registry`) substitution fires exactly as in v0.1.
        // Regression bite for the v0.1 stable contract: identical
        // input must produce byte-identical output regardless of the
        // compat-short-CARGO opt-in (compat mode flips the flag on;
        // every other run leaves it off and must match v0.1 byte
        // for byte).
        let input = "  --> /home/u/.cargo/registry/src/index.crates.io-1234567890abcdef/foo-1.0.0/src/lib.rs:3:1\n";
        let c = ctx_non_compat_no_collision("/p", "/sysroot");
        let dir = PathBuf::from("/p/x");
        let out = normalize(input, &c, &dir);
        assert_eq!(
            out,
            "  --> $CARGO/registry/src/index.crates.io-1234567890abcdef/foo-1.0.0/src/lib.rs:3:1"
        );
    }

    // ====================================================================
    // Issue #45 / v0.1.0-beta.10 — `extra_substitutions` framework
    //
    // Per plan §7.1 (`docs/spec/extra-substitutions-plan-2026-05-19.md`).
    // Tests cover normalizer composition: extras apply AFTER built-ins,
    // extras apply in declared order, strip drops with both exact and
    // prefix matchers, strip runs after trim-trailing-whitespace and
    // before blank-line collapse, no interference with diagnostic text.
    //
    // Predicate-level + field-level allowlist tests for the validators
    // live in `src/config.rs` tests module.
    // ====================================================================

    /// Build a context with the three #45 override fields populated.
    /// Uses long, non-colliding workspace/sysroot paths so the
    /// built-in path-substitution loop doesn't accidentally swallow
    /// substrings inside test-input literals (`/r` would match
    /// `/rust-...`, `/p` would match `/path/...`).
    fn ctx_with_extras(
        extras: Vec<Substitution>,
        strip_lines: Vec<String>,
        strip_line_prefixes: Vec<String>,
    ) -> NormalizationContext {
        NormalizationContext {
            workspace_root: PathBuf::from("/lihaaf_test_ws_root"),
            sysroot: PathBuf::from("/lihaaf_test_sysroot"),
            cargo_registry: Some(PathBuf::from("/lihaaf_test_cargo/registry")),
            compat_short_cargo: false,
            extra_substitutions: extras,
            strip_lines,
            strip_line_prefixes,
        }
    }

    /// Test-only constant for tests that use the long-form
    /// non-colliding fixture dir. The dir is chosen so it doesn't
    /// substring-overlap with the input lines used in these tests.
    fn test_fixture_dir() -> PathBuf {
        PathBuf::from("/lihaaf_test_fixture_dir")
    }

    #[test]
    fn extra_substitutions_apply_after_builtins() {
        // Adopter rule rewrites a NixOS sysroot literal to `$RUST` after
        // built-in path substitutions have already resolved `/p` →
        // `$WORKSPACE` etc. The built-in `$RUST` substitution does NOT
        // fire on the Nix path; the adopter rule has to be what
        // produces the placeholder.
        let input = "  ::: /nix/store/abc123-rust-1.95.0/lib/rustlib/std/src/option.rs:1:1\n";
        let extras = vec![Substitution {
            from: "/nix/store/abc123-rust-1.95.0/lib/rustlib".into(),
            to: "$RUST/lib/rustlib".into(),
        }];
        let c = ctx_with_extras(extras, vec![], vec![]);
        let out = normalize(input, &c, &test_fixture_dir());
        assert_eq!(out, "  ::: $RUST/lib/rustlib/std/src/option.rs:1:1");
    }

    #[test]
    fn extra_substitutions_apply_in_declared_order() {
        // Two rules; the first rewrites a literal path to `$RUST/lib/...`,
        // the second collapses the inner version-stamped sub-tree to
        // bare `$RUST`. Order matters: applying them out of order would
        // either fail to match (if rule 2 runs first, the input has no
        // `$RUST/...` substring to collapse) or land on the wrong target.
        let input = "  ::: /opt/vendored/rust-1.95.0/lib/rust-1.95.0/std/src/option.rs:1:1\n";
        let extras = vec![
            Substitution {
                from: "/opt/vendored/rust-1.95.0/lib".into(),
                to: "$RUST/lib".into(),
            },
            Substitution {
                from: "$RUST/lib/rust-1.95.0".into(),
                to: "$RUST".into(),
            },
        ];
        let c = ctx_with_extras(extras, vec![], vec![]);
        let out = normalize(input, &c, &test_fixture_dir());
        assert_eq!(out, "  ::: $RUST/std/src/option.rs:1:1");
    }

    #[test]
    fn extra_substitutions_empty_default_byte_identical() {
        // Regression guard for §1.3 default invariant: when the three new
        // vectors are empty, normalizer output must be byte-identical to
        // the legacy beta.9 path. A simple round-trip with a path-marker
        // line covers the most common shape.
        let input = "  --> /p/tests/lihaaf/compile_fail/foo.rs:3:1\n";
        let c = ctx("/p", "/r");
        let dir = PathBuf::from("/p/tests/lihaaf/compile_fail");
        let out = normalize(input, &c, &dir);
        assert_eq!(out, "  --> $DIR/foo.rs:3:1");
    }

    #[test]
    fn strip_lines_drops_full_line_match() {
        let input = "alpha\n/build/sandbox/internal/wrappers/cc-wrapper-1.0\nomega\n";
        let c = ctx_with_extras(
            vec![],
            vec!["/build/sandbox/internal/wrappers/cc-wrapper-1.0".into()],
            vec![],
        );
        let out = normalize(input, &c, &test_fixture_dir());
        assert_eq!(out, "alpha\nomega");
    }

    #[test]
    fn strip_lines_drops_banner_line_match() {
        // Exact-match strip of the rustc `error: aborting due to ...`
        // summary banner. Adopter opt-in: built-in normalization
        // preserves this line; strip removes it.
        let input = "error: bad\nerror: aborting due to 1 previous error\n";
        let c = ctx_with_extras(
            vec![],
            vec!["error: aborting due to 1 previous error".into()],
            vec![],
        );
        let out = normalize(input, &c, &test_fixture_dir());
        assert_eq!(out, "error: bad");
    }

    #[test]
    fn strip_lines_no_partial_match() {
        // `strip_lines` is full-line exact-match only. A line that
        // contains the strip pattern as a substring must NOT be dropped;
        // adopters who want partial matches use prefix or substitution.
        let input = "/build/sandbox/internal/wrappers/cc-wrapper-1.0 plus more\nbeta\n";
        let c = ctx_with_extras(
            vec![],
            vec!["/build/sandbox/internal/wrappers/cc-wrapper-1.0".into()],
            vec![],
        );
        let out = normalize(input, &c, &test_fixture_dir());
        assert_eq!(
            out,
            "/build/sandbox/internal/wrappers/cc-wrapper-1.0 plus more\nbeta",
        );
    }

    #[test]
    fn strip_line_prefixes_matches_prefix_only() {
        // Prefix match drops every line starting with the pattern; lines
        // that don't start with it survive.
        let input =
            "$WORKSPACE/.cargo-cache/aaa-001\n$WORKSPACE/.cargo-cache/bbb-002\nother line\n";
        let c = ctx_with_extras(vec![], vec![], vec!["$WORKSPACE/.cargo-cache/".into()]);
        let out = normalize(input, &c, &test_fixture_dir());
        assert_eq!(out, "other line");
    }

    #[test]
    fn strip_line_prefixes_drops_explain_footer_family() {
        // Prefix strip of the rustc explain-footer family across multiple
        // error codes. A single adopter prefix collapses every variant.
        let input = "error: bad\n\nFor more information about this error, try `rustc --explain E0277`.\nFor more information about this error, try `rustc --explain E0463`.\n";
        let c = ctx_with_extras(
            vec![],
            vec![],
            vec!["For more information about this error".into()],
        );
        let out = normalize(input, &c, &test_fixture_dir());
        // Both explain-footer lines drop; blank-line collapse runs after
        // the drop so the trailing blank between `error: bad` and the
        // first dropped line collapses with the absence of follow-on
        // content. Result: just `error: bad`.
        assert_eq!(out, "error: bad");
    }

    #[test]
    fn strip_line_prefixes_drops_macro_origin_trailer_family() {
        let input = "error: bad\nnote: this error originates from the macro `m` in the crate `c`\nnote: this error originates from the attribute macro `derive_more::Display`\nfinal\n";
        let c = ctx_with_extras(
            vec![],
            vec![],
            vec!["note: this error originates from ".into()],
        );
        let out = normalize(input, &c, &test_fixture_dir());
        assert_eq!(out, "error: bad\nfinal");
    }

    #[test]
    fn strip_patterns_apply_after_trim_trailing_whitespace() {
        // Per plan §4: line-drop runs AFTER per-line trim-trailing-
        // whitespace. The input line has trailing whitespace; after the
        // trim, the trimmed body matches the strip rule, so the line
        // drops. If the trim hadn't run first, the pattern wouldn't match.
        let input = "alpha\n/build/sandbox/internal/wrappers/cc-wrapper-1.0   \t\nomega\n";
        let c = ctx_with_extras(
            vec![],
            vec!["/build/sandbox/internal/wrappers/cc-wrapper-1.0".into()],
            vec![],
        );
        let out = normalize(input, &c, &test_fixture_dir());
        assert_eq!(out, "alpha\nomega");
    }

    #[test]
    fn strip_patterns_do_not_affect_diagnostic_text() {
        // NOTE (NIT-2): normalizer composition test, not user-facing
        // compat support. The strip vectors here are path-shaped (would
        // pass `is_path_like` at config-parse time) but happen not to
        // match any line in the input — so diagnostic text passes
        // through unchanged. Adopters cannot write diagnostic-text strip
        // patterns at config-parse time; this test pins composition
        // behavior at the normalizer layer.
        let input = "error: unknown on_delete value `bogus`; expected one of: cascade\n";
        let c = ctx_with_extras(vec![], vec![], vec!["$WORKSPACE/.cargo-cache/".into()]);
        let out = normalize(input, &c, &test_fixture_dir());
        assert_eq!(
            out,
            "error: unknown on_delete value `bogus`; expected one of: cascade",
        );
    }

    #[test]
    fn extra_substitutions_run_before_type_id_collapse() {
        // Per plan §4: extras run BEFORE TypeId. An adopter rule that
        // introduces a `#<digits>` sequence must collapse to `$TYPEID`
        // on the same line. Without the ordering guarantee, the
        // introduced sequence would slip past TypeId normalization.
        //
        // The rule's `from` is path-shaped to pass the upstream
        // allowlist; the `to` introduces a path-prefixed `#0` shape.
        let input = "  ::: /vendored/cache/path:1:1\n";
        let extras = vec![Substitution {
            from: "/vendored/cache/path".into(),
            to: "/x/#0/y".into(),
        }];
        let c = ctx_with_extras(extras, vec![], vec![]);
        let out = normalize(input, &c, &test_fixture_dir());
        // `#0` collapses to `$TYPEID` because TypeId runs after extras.
        assert_eq!(out, "  ::: /x/$TYPEID/y:1:1");
    }

    #[test]
    fn compose_with_compat_short_cargo() {
        // NOTE (NIT-2): exercises normalizer-internal composition.
        // Adopter extras remain unsupported in compat mode per §5 /
        // §6.6 of the v0.1 spec — but the composition order is pinned
        // here so if compat-mode adopter extras land in beta.11+, the
        // ordering is already correct: short-CARGO post-pass runs
        // first, then adopter extras, then TypeId.
        let input = "  --> /home/u/.cargo/registry/src/index.crates.io-1234567890abcdef/foo-1.0.0/src/lib.rs:3:1\n";
        let extras = vec![Substitution {
            from: "$CARGO/foo-1.0.0".into(),
            to: "$CARGO/foo".into(),
        }];
        let c = NormalizationContext {
            workspace_root: PathBuf::from("/lihaaf_test_ws_root"),
            sysroot: PathBuf::from("/lihaaf_test_sysroot"),
            cargo_registry: Some(PathBuf::from("/home/u/.cargo/registry")),
            compat_short_cargo: true,
            extra_substitutions: extras,
            strip_lines: vec![],
            strip_line_prefixes: vec![],
        };
        let out = normalize(input, &c, &test_fixture_dir());
        // Short-CARGO post-pass rewrites the registry path to
        // `$CARGO/foo-1.0.0/...` first; the adopter rule then collapses
        // the version-stamped sub-tree to bare `$CARGO/foo/...`.
        assert_eq!(out, "  --> $CARGO/foo/src/lib.rs:3:1");
    }

    #[test]
    fn extra_substitutions_no_newline_in_to_debug_assertion() {
        // Debug-assertion mirror of the config-parse validation rule.
        // The `to` field must NOT contain a newline. We can't trip the
        // validator here (the normalizer doesn't validate), but we CAN
        // verify the normalizer doesn't blow up when fed a multi-line
        // `to`: it would produce a malformed snapshot, which is exactly
        // why the upstream validator rejects it.
        //
        // This test simply documents the contract by demonstrating the
        // outcome of bypassing it: synthesized newline lands in stderr
        // and disrupts the line model. Adopters cannot reach this
        // shape via TOML — the validator catches it first.
        let input = "  ::: /a/b/c:1:1\n";
        let extras = vec![Substitution {
            from: "/a/b/c".into(),
            to: "$WORKSPACE/inserted\nbad".into(),
        }];
        let c = ctx_with_extras(extras, vec![], vec![]);
        let out = normalize(input, &c, &test_fixture_dir());
        // Normalizer doesn't re-validate; the multi-line `to` lands in
        // the substituted output. The contract is that callers prevent
        // this via the config-parse validator; the test pins the
        // expected (admittedly malformed) outcome so a future tightening
        // of `replace_advancing` doesn't silently change behavior.
        assert!(out.contains("$WORKSPACE/inserted"));
        assert!(out.contains("bad:1:1"));
    }
}
