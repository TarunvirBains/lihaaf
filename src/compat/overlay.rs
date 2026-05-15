//! Phase 2 of compat mode (issue #11) — sibling-manifest overlay generator.
//!
//! Reads the upstream `Cargo.toml`, canonicalizes `[lib] crate-type` so the
//! lihaaf stage-3 dylib build can succeed without mutating the upstream
//! file, and writes the result to `Cargo.lihaaf.toml` next to it.
//!
//! Per `docs/compatibility-plan.md` §3.2.3, the overlay is:
//!
//! 1. Re-serialized through the existing `toml = "1"` dependency. No
//!    second TOML crate is introduced (the v0.1 surface forbids
//!    `toml_edit` — that's a v0.2 conversation).
//! 2. Written with table keys in cargo's canonical order: `package`,
//!    `lib`, `bin`, `dependencies`, `dev-dependencies`,
//!    `build-dependencies`, `features`, `workspace`, then alphabetical
//!    for the long tail.
//! 3. Stripped of comments. The `toml` crate's `Value` data model drops
//!    comments on parse; an upstream-text scan recovers them for the
//!    §3.3 envelope's `overlay.dropped_comments` field.
//! 4. Always LF line endings, never CRLF. No trailing whitespace.
//! 5. Idempotent: a second run from the same input produces a
//!    byte-identical output, and the write is skipped (preserving mtime)
//!    when the existing sibling matches.
//!
//! The atomic write reuses [`crate::util::write_file_atomic`] so the
//! sibling is either fully written or absent — a SIGKILL mid-write
//! cannot leave a half-formed `Cargo.lihaaf.toml` for the stage-3
//! `cargo rustc` to choke on.
//!
//! ## Crate-type canonicalization
//!
//! `[lib] crate-type` is the only field the overlay modifies. The
//! semantics:
//!
//! | Input                                       | Output                                       |
//! |---------------------------------------------|----------------------------------------------|
//! | absent                                      | `["dylib", "rlib"]`                          |
//! | `["rlib"]`                                  | `["dylib", "rlib"]`                          |
//! | `["dylib"]`                                 | `["dylib", "rlib"]` (rlib appended)          |
//! | `["dylib", "rlib"]`                         | unchanged                                    |
//! | `["cdylib"]`                                | `["dylib", "rlib", "cdylib"]`                |
//! | `["rlib", "staticlib"]`                     | `["dylib", "rlib", "staticlib"]`             |
//!
//! `rlib` is retained on every output shape so the non-lihaaf
//! `cargo test` baseline (§3.4) keeps working. Other entries
//! (`cdylib`, `staticlib`, etc.) are preserved verbatim AFTER the
//! `dylib`/`rlib` pair, in their original order.
//!
//! ## What the overlay does NOT touch
//!
//! - `[patch.crates-io]` — must pass through verbatim. The spec
//!   (§3.2.3 risks section) is explicit that `[patch]` cannot add
//!   crate-type and the overlay code must not rewrite it.
//! - Every other top-level table (`dependencies`, `dev-dependencies`,
//!   `features`, `[[bin]]`, `[workspace]`, …) is preserved as parsed.

use std::path::{Path, PathBuf};

use crate::error::Error;
use crate::util;

/// One materialized overlay run. Constructed by [`materialize_overlay`]
/// after the sibling manifest is written (or skipped as idempotent).
///
/// The bundle is consumed by the §3.3 envelope writer — the
/// `overlay.generated` classification reads from this struct.
///
/// `pub` (with the parent module pinned at `pub(crate)`) so the crate
/// root can `#[doc(hidden)]` re-export this for the test crate. Not
/// part of any v0.1 stability contract.
#[derive(Debug)]
pub struct OverlayPlan {
    /// Path to the upstream `Cargo.toml` the overlay was derived from.
    pub upstream_manifest: PathBuf,
    /// Path to the generated `Cargo.lihaaf.toml`. Always the
    /// sibling of `upstream_manifest`; never inside `target/`.
    pub sibling_manifest: PathBuf,
    /// `true` when the upstream manifest already declared
    /// `[lib] crate-type = ["dylib", ...]`. The sibling is still
    /// written (idempotently) so the §3.3 envelope's
    /// `overlay.generated` classification is uniform; the flag lets the
    /// envelope record whether the dylib declaration was a real change
    /// or a redundant one.
    pub upstream_already_has_dylib: bool,
    /// Comment text dropped during canonicalization. The `toml` crate's
    /// `Value` model drops comments on parse, so the overlay code
    /// recovers them from the raw upstream bytes with a small
    /// state-machine scanner (no regex per spec §6.1) that tracks all
    /// four TOML string forms — basic, literal, multi-line basic, and
    /// multi-line literal — so a `#` inside any string is not surfaced
    /// here. Entries are stashed for the §3.3 envelope's
    /// `overlay.dropped_comments` field.
    ///
    /// Each entry is the raw comment text WITHOUT the leading `#` and
    /// WITHOUT surrounding whitespace, so the envelope can render the
    /// list directly.
    pub dropped_comments: Vec<String>,
    /// Upstream `[package].name` read out of the same Cargo.toml the
    /// overlay parsed. `Some(name)` when the upstream manifest has a
    /// non-empty string value at `package.name`; `None` for malformed
    /// manifests or workspace roots that lack the `[package]` table.
    ///
    /// Captured here so the compat driver does not have to read and
    /// parse Cargo.toml a second time to populate the §3.3 envelope's
    /// `crate_name` field — the overlay code already has the parsed
    /// `toml::Value` in hand.
    pub upstream_crate_name: Option<String>,
}

/// Synthetic `[package.metadata.lihaaf]` table the compat driver
/// injects into the sibling overlay so the upstream pilot fork does not
/// need to hand-author a metadata block.
///
/// Constructed by the compat driver after it has resolved the crate
/// name + converted-fixtures directory; passed to
/// [`materialize_overlay_with_metadata`]. The fields map 1:1 to the
/// keys the v0.1 [`crate::config::Config`] loader expects.
#[derive(Debug, Clone)]
pub struct SyntheticMetadata {
    /// `dylib_crate` — the workspace-member crate name. The compat
    /// driver reads this from upstream `[package].name`.
    pub dylib_crate: String,
    /// `extern_crates` — list of `--extern` names handed to per-fixture
    /// rustc. The compat driver always sets this to `[dylib_crate]`;
    /// the v0.1 config loader enforces `extern_crates[0] == dylib_crate`
    /// anyway.
    pub extern_crates: Vec<String>,
    /// `fixture_dirs` — list of directories to walk for fixtures. The
    /// compat driver populates this with the converted-fixtures path
    /// under `<compat_root>/target/lihaaf-compat-converted/`. Paths
    /// are written verbatim into the TOML.
    pub fixture_dirs: Vec<String>,
}

/// Read the upstream `Cargo.toml`, materialize the sibling overlay, and
/// return the plan.
///
/// `upstream_manifest_path` must point at the upstream `Cargo.toml`
/// itself (not its parent directory). The sibling path is computed via
/// [`Path::with_file_name`] — the safest cross-platform way to swap
/// only the filename component.
///
/// **Pre-write idempotency check.** If the sibling already exists with
/// byte-identical contents, no write is performed. This preserves
/// mtime per the §3.2.3 "dirty-worktree rule" — repeated runs from
/// clean state must not churn the filesystem.
///
/// **Errors.** Returns [`Error::Io`] on read/write failure and
/// [`Error::TomlParse`] when the upstream manifest cannot be parsed as
/// TOML. The compat driver maps both into the §3.3 envelope's
/// `overlay.*` error category.
pub fn materialize_overlay(upstream_manifest_path: &Path) -> Result<OverlayPlan, Error> {
    materialize_overlay_with_metadata(upstream_manifest_path, None)
}

/// Variant of [`materialize_overlay`] that also injects a synthetic
/// `[package.metadata.lihaaf]` table into the sibling overlay.
///
/// When `synthetic_metadata` is `Some`, the table is spliced into the
/// parsed `package.metadata.lihaaf` location BEFORE the canonical
/// serializer runs, so the on-disk overlay carries the metadata block
/// the v0.1 [`crate::config::load`] entry needs to drive a compat-mode
/// inner session.
///
/// **Conflict policy.** If the upstream `Cargo.toml` already has a
/// `[package.metadata.lihaaf]` table, the synthetic metadata is
/// OVERWRITTEN with the synthetic values: compat mode owns the
/// inner-session config; an existing metadata block in a pilot fork
/// would have been written under v0.1 semantics that may not match
/// the compat-driver-synthesized `fixture_dirs` path under
/// `<compat_root>/target/lihaaf-compat-converted/`.
///
/// **Errors.** Same shape as [`materialize_overlay`].
pub fn materialize_overlay_with_metadata(
    upstream_manifest_path: &Path,
    synthetic_metadata: Option<&SyntheticMetadata>,
) -> Result<OverlayPlan, Error> {
    // Bridge to the builder-shaped entry: the builder ignores the
    // upstream name and returns the caller's pre-constructed metadata
    // (cloned because the builder owns the returned value).
    materialize_overlay_inner(upstream_manifest_path, |_name| synthetic_metadata.cloned())
}

/// Variant of [`materialize_overlay_with_metadata`] whose synthetic
/// metadata is constructed by a builder closure given the upstream
/// crate name. Lets the compat driver build the synthetic block
/// `[package.metadata.lihaaf]` using the crate name without parsing
/// `Cargo.toml` a second time — the overlay code passes the parsed
/// `[package].name` directly into the builder.
///
/// `builder` receives `Some(name)` when the upstream manifest carries a
/// non-empty `[package].name` string, and `None` for workspace roots /
/// malformed manifests (where the caller decides on a fallback). The
/// builder may return `None` to skip metadata injection entirely.
///
/// **Errors.** Same shape as [`materialize_overlay`].
pub fn materialize_overlay_with_synthetic_metadata_builder<F>(
    upstream_manifest_path: &Path,
    builder: F,
) -> Result<OverlayPlan, Error>
where
    F: FnOnce(Option<&str>) -> SyntheticMetadata,
{
    materialize_overlay_inner(upstream_manifest_path, |name| Some(builder(name)))
}

fn materialize_overlay_inner<F>(
    upstream_manifest_path: &Path,
    synthetic_metadata: F,
) -> Result<OverlayPlan, Error>
where
    F: FnOnce(Option<&str>) -> Option<SyntheticMetadata>,
{
    let raw_bytes = std::fs::read(upstream_manifest_path).map_err(|e| {
        Error::io(
            e,
            "reading upstream Cargo.toml for overlay",
            Some(upstream_manifest_path.to_path_buf()),
        )
    })?;
    let raw_text = String::from_utf8(raw_bytes).map_err(|e| {
        Error::io(
            std::io::Error::new(std::io::ErrorKind::InvalidData, e),
            "decoding upstream Cargo.toml as UTF-8",
            Some(upstream_manifest_path.to_path_buf()),
        )
    })?;

    let dropped_comments = scan_dropped_comments(&raw_text);

    let mut value: toml::Value =
        toml::from_str(&raw_text).map_err(|e: toml::de::Error| Error::TomlParse {
            path: upstream_manifest_path.to_path_buf(),
            message: e.to_string(),
        })?;

    // Spec invariant: `--compat-root` is single-crate. A workspace-root
    // manifest (`[workspace]` table without a top-level `[package]`)
    // cannot host a `[lib] crate-type` rewrite; the lihaaf stage-3
    // dylib build would have nothing to compile. `[workspace.package]`
    // is inherited-metadata for member crates and does NOT make the
    // manifest itself buildable, so it is rejected uniformly.
    // Reject with a directed diagnostic pointing the adopter at a
    // member crate's Cargo.toml. The empty / unusual case (neither
    // `[package]` nor `[workspace]`) stays tolerant — that may be a
    // test fixture or a partial manifest the operator is constructing.
    if is_workspace_root_manifest(&value) {
        return Err(Error::Cli {
            clap_exit_code: 2,
            message: format!(
                "error: `--compat-root` must point to a single-crate Cargo.toml; \
                 `{}` is a workspace root (declares `[workspace]` without `[package]`). \
                 Pass a member crate's Cargo.toml as `--compat-root` instead.",
                upstream_manifest_path.display()
            ),
        });
    }

    let upstream_already_has_dylib = inspect_existing_crate_type(&value);
    let upstream_crate_name = read_upstream_crate_name(&value);
    let synthetic = synthetic_metadata(upstream_crate_name.as_deref());

    if let toml::Value::Table(top) = &mut value {
        // Insert/extend [lib] crate-type. The canonicalization is
        // idempotent: a second run on the output is a no-op.
        let lib_table = top
            .entry("lib".to_string())
            .or_insert_with(|| toml::Value::Table(toml::map::Map::new()));
        if let toml::Value::Table(lib) = lib_table {
            canonicalize_crate_type(lib)?;
        } else {
            return Err(Error::TomlParse {
                path: upstream_manifest_path.to_path_buf(),
                message: "`[lib]` must be a table, not an inline value".to_string(),
            });
        }

        if let Some(meta) = synthetic.as_ref() {
            inject_synthetic_metadata(top, meta);
        }
    }

    let serialized = serialize_canonical(&value)?;
    let sibling_path = upstream_manifest_path.with_file_name("Cargo.lihaaf.toml");

    // Idempotent rerun guard — skip the write when bytes match. This
    // preserves mtime so a clean-state second invocation does not
    // appear as a worktree change to fork-CI greppers.
    let need_write = match std::fs::read(&sibling_path) {
        Ok(existing) => existing != serialized,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => true,
        Err(e) => {
            return Err(Error::io(
                e,
                "checking existing Cargo.lihaaf.toml for idempotent rerun",
                Some(sibling_path.clone()),
            ));
        }
    };

    if need_write {
        util::write_file_atomic(&sibling_path, &serialized)?;
    }

    Ok(OverlayPlan {
        upstream_manifest: upstream_manifest_path.to_path_buf(),
        sibling_manifest: sibling_path,
        upstream_already_has_dylib,
        dropped_comments,
        upstream_crate_name,
    })
}

/// Read the upstream `[package].name` out of an already-parsed
/// `toml::Value`. Returns `Some(name)` when the field is a non-empty
/// string; `None` for missing tables, non-string values, or
/// workspace-root manifests that lack `[package]`. The caller falls
/// back to a basename heuristic in those cases.
fn read_upstream_crate_name(value: &toml::Value) -> Option<String> {
    value
        .get("package")
        .and_then(|p| p.get("name"))
        .and_then(|n| n.as_str())
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// Splice the synthetic `[package.metadata.lihaaf]` table into `top`.
///
/// Creates the `[package]` and `[package.metadata]` parent tables as
/// needed; replaces any pre-existing `[package.metadata.lihaaf]` entry
/// in full (the v0.1 config loader treats the table as a single typed
/// bundle, so partial merging would produce undefined behavior when the
/// adopter's pre-existing table has different `extern_crates` or
/// `fixture_dirs`).
///
/// The inserted values are typed: `dylib_crate` is a string,
/// `extern_crates` is an array of strings, `fixture_dirs` is an array of
/// strings. These match the v0.1 [`crate::config::RawMetadata`] schema.
fn inject_synthetic_metadata(
    top: &mut toml::map::Map<String, toml::Value>,
    meta: &SyntheticMetadata,
) {
    let package_entry = top
        .entry("package".to_string())
        .or_insert_with(|| toml::Value::Table(toml::map::Map::new()));
    let toml::Value::Table(package) = package_entry else {
        return;
    };
    let metadata_entry = package
        .entry("metadata".to_string())
        .or_insert_with(|| toml::Value::Table(toml::map::Map::new()));
    let toml::Value::Table(metadata) = metadata_entry else {
        return;
    };

    let mut lihaaf_table = toml::map::Map::new();
    lihaaf_table.insert(
        "dylib_crate".to_string(),
        toml::Value::String(meta.dylib_crate.clone()),
    );
    lihaaf_table.insert(
        "extern_crates".to_string(),
        toml::Value::Array(
            meta.extern_crates
                .iter()
                .cloned()
                .map(toml::Value::String)
                .collect(),
        ),
    );
    lihaaf_table.insert(
        "fixture_dirs".to_string(),
        toml::Value::Array(
            meta.fixture_dirs
                .iter()
                .cloned()
                .map(toml::Value::String)
                .collect(),
        ),
    );

    metadata.insert("lihaaf".to_string(), toml::Value::Table(lihaaf_table));
}

/// Return `true` when `value` is a workspace-root manifest: declares a
/// `[workspace]` table AND lacks a top-level `[package]`. The
/// `[workspace.package]` table is INHERITED metadata for member crates
/// (the `package.version.workspace = true` pattern) — its presence does
/// NOT make the manifest itself a buildable library; the actual crate
/// the adopter wants overlayed lives in a member directory.
fn is_workspace_root_manifest(value: &toml::Value) -> bool {
    let Some(top) = value.as_table() else {
        return false;
    };
    let has_workspace = top.get("workspace").is_some_and(|v| v.is_table());
    let has_package = top.get("package").is_some_and(|v| v.is_table());
    has_workspace && !has_package
}

/// Return `true` when the upstream `[lib] crate-type` already contains
/// `dylib`. Used only for envelope classification — the overlay rewrite
/// runs unconditionally.
fn inspect_existing_crate_type(value: &toml::Value) -> bool {
    let Some(lib) = value.get("lib") else {
        return false;
    };
    let Some(ct) = lib.get("crate-type") else {
        return false;
    };
    let Some(arr) = ct.as_array() else {
        return false;
    };
    arr.iter().filter_map(|v| v.as_str()).any(|s| s == "dylib")
}

/// Canonicalize the `[lib] crate-type` array on a `[lib]` table.
///
/// Per §3.2.3 the output array must:
/// - Start with `"dylib"`.
/// - Contain `"rlib"` (so the non-lihaaf `cargo test` baseline still
///   works).
/// - Preserve any other entries (`cdylib`, `staticlib`, …) AFTER the
///   `dylib`/`rlib` pair, in their original order.
///
/// Non-string entries in the input array trigger [`Error::TomlParse`]
/// with a directed diagnostic; downstream code can rely on the output
/// being a homogeneous string array.
pub(crate) fn canonicalize_crate_type(
    table: &mut toml::map::Map<String, toml::Value>,
) -> Result<(), Error> {
    let existing: Vec<String> = match table.get("crate-type") {
        None => Vec::new(),
        Some(toml::Value::Array(arr)) => {
            let mut out = Vec::with_capacity(arr.len());
            for (idx, v) in arr.iter().enumerate() {
                match v.as_str() {
                    Some(s) => out.push(s.to_string()),
                    None => {
                        return Err(Error::TomlParse {
                            path: PathBuf::from("<overlay>"),
                            message: format!(
                                "`[lib] crate-type` element at index {idx} is not a string; \
                                 the overlay accepts only string crate-type entries"
                            ),
                        });
                    }
                }
            }
            out
        }
        Some(other) => {
            return Err(Error::TomlParse {
                path: PathBuf::from("<overlay>"),
                message: format!(
                    "`[lib] crate-type` must be an array of strings, got `{}`",
                    type_name_of(other)
                ),
            });
        }
    };

    // Strategy: a stable interleave that always puts `dylib`/`rlib`
    // first (in that order), then everything else in input order with
    // dups removed. We do NOT alphabetize the long tail — the spec
    // says "preserved verbatim AFTER the dylib/rlib pair".
    let mut out: Vec<String> = Vec::with_capacity(existing.len() + 2);
    out.push("dylib".to_string());
    out.push("rlib".to_string());
    for entry in &existing {
        if entry == "dylib" || entry == "rlib" {
            continue;
        }
        if !out.contains(entry) {
            out.push(entry.clone());
        }
    }

    let array = out.into_iter().map(toml::Value::String).collect::<Vec<_>>();
    table.insert("crate-type".to_string(), toml::Value::Array(array));
    Ok(())
}

/// Cargo's canonical table key order. The long tail (anything not in
/// this slice) is sorted alphabetically when serialized.
///
/// This list is intentionally hardcoded; a configuration option would
/// expand the v0.1 surface for no adopter benefit.
pub(crate) fn canonical_key_order() -> &'static [&'static str] {
    &[
        "package",
        "lib",
        "bin",
        "example",
        "test",
        "bench",
        "dependencies",
        "dev-dependencies",
        "build-dependencies",
        "target",
        "features",
        "patch",
        "replace",
        "profile",
        "workspace",
    ]
}

/// Re-serialize a parsed [`toml::Value`] into bytes with the canonical
/// table order, no comments, no trailing whitespace, LF line endings.
///
/// **Why a custom shim:** `toml = "1"`'s default serializer emits
/// table keys in `BTreeMap` (alphabetical) order, which is NOT the
/// cargo-canonical order the spec mandates. We work around this by
/// serializing each top-level key as its own single-key wrapper
/// (preserving the crate's stable inline-key ordering for inner
/// tables) and concatenating the segments with the canonical order
/// applied.
///
/// **Why the segments are concatenated with `\n` separators:** each
/// `toml::ser::to_string` call ends with `\n`, so prepending another
/// `\n` produces exactly one blank line between sections. Post-
/// processing collapses any accidental triple-newlines back to a
/// single blank line for the byte-determinism guarantee.
pub(crate) fn serialize_canonical(value: &toml::Value) -> Result<Vec<u8>, Error> {
    let top = match value {
        toml::Value::Table(t) => t,
        other => {
            return Err(Error::TomlParse {
                path: PathBuf::from("<overlay>"),
                message: format!(
                    "overlay serializer expected a TOML document (table) at the top level, got `{}`",
                    type_name_of(other)
                ),
            });
        }
    };

    // Build the canonical key sequence: every canonical key that is
    // present in the input, in canonical order, followed by every
    // other key in alphabetical order.
    let mut emitted: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
    let mut order: Vec<String> = Vec::with_capacity(top.len());
    for canonical in canonical_key_order() {
        if top.contains_key(*canonical) {
            order.push((*canonical).to_string());
            emitted.insert(*canonical);
        }
    }
    let mut leftovers: Vec<&String> = top
        .keys()
        .filter(|k| !emitted.contains(k.as_str()))
        .collect();
    leftovers.sort();
    for k in leftovers {
        order.push(k.clone());
    }

    let mut segments: Vec<String> = Vec::with_capacity(order.len());
    for key in &order {
        let v = top.get(key).expect("key came from `top`'s own iteration");
        let mut wrapper = toml::map::Map::new();
        wrapper.insert(key.clone(), v.clone());
        let segment =
            toml::ser::to_string(&toml::Value::Table(wrapper)).map_err(|e: toml::ser::Error| {
                Error::TomlParse {
                    path: PathBuf::from("<overlay>"),
                    message: format!("overlay serializer failed for `{key}`: {e}"),
                }
            })?;
        segments.push(segment);
    }

    let joined = segments.join("\n");
    let normalized = post_process_output(&joined);
    Ok(normalized.into_bytes())
}

/// Apply the §3.2.3 byte-shape invariants:
///
/// - LF line endings only (strip any `\r`).
/// - No trailing whitespace on any line.
/// - Collapse two-or-more consecutive blank lines down to one blank
///   line, so segment concatenation can't produce churning whitespace
///   when one segment ends with an internal blank line.
/// - Exactly one trailing `\n`.
fn post_process_output(input: &str) -> String {
    // First pass: normalize line endings and strip trailing whitespace.
    let mut lines: Vec<&str> = Vec::with_capacity(input.lines().count());
    for line in input.lines() {
        // `lines()` already strips both `\n` and `\r\n`, so the only
        // `\r` we can see is one embedded mid-line (extremely rare in
        // hand-edited TOML). We re-trim per-line to belt-and-suspenders
        // against a Windows-checkout `\r\n` upstream.
        let trimmed = line.trim_end_matches([' ', '\t', '\r']);
        lines.push(trimmed);
    }

    // Second pass: collapse runs of blank lines down to one blank.
    let mut out = String::with_capacity(input.len());
    let mut prev_blank = false;
    for line in &lines {
        let is_blank = line.is_empty();
        if is_blank && prev_blank {
            continue;
        }
        out.push_str(line);
        out.push('\n');
        prev_blank = is_blank;
    }

    // Strip a trailing blank line (we always end with a single `\n`,
    // not a `\n\n`), then make sure exactly one trailing `\n` is
    // present.
    while out.ends_with("\n\n") {
        out.pop();
    }
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out
}

/// Walk the raw TOML bytes once and pull every `#`-prefixed comment
/// out, both line-leading (`# foo`) and trailing (`name = "x" # foo`).
///
/// **Why this is fixed-string, not regex:** spec §6.1 forbids a regex
/// engine in this crate. The scanner is a single byte-stream walk that
/// tracks four kinds of string state — basic (`"..."`), literal
/// (`'...'`), multi-line basic (`"""..."""`), and multi-line literal
/// (`'''...'''`) — so a `#` inside any TOML string form is never
/// recorded as a comment. The multi-line forms are line-spanning, so
/// the walker explicitly cannot be a line-by-line split: a `#` inside
/// `"""..."""` on a continuation line must still be treated as content.
fn scan_dropped_comments(text: &str) -> Vec<String> {
    let bytes = text.as_bytes();
    let mut out: Vec<String> = Vec::new();
    let mut i = 0usize;
    // Mutually exclusive: at most one of these is `true` at any time.
    let mut in_basic = false;
    let mut in_literal = false;
    let mut in_multi_basic = false;
    let mut in_multi_literal = false;

    while i < bytes.len() {
        let b = bytes[i];

        // Inside a multi-line basic string (`"""..."""`). Honors `\`
        // escapes per TOML basic-string rules; the close is the first
        // un-escaped `"""`.
        if in_multi_basic {
            if b == b'\\' && i + 1 < bytes.len() {
                i += 2;
                continue;
            }
            if b == b'"' && i + 2 < bytes.len() && bytes[i + 1] == b'"' && bytes[i + 2] == b'"' {
                in_multi_basic = false;
                i += 3;
                continue;
            }
            i += 1;
            continue;
        }

        // Inside a multi-line literal string (`'''...'''`). No escapes;
        // the close is the first `'''`.
        if in_multi_literal {
            if b == b'\'' && i + 2 < bytes.len() && bytes[i + 1] == b'\'' && bytes[i + 2] == b'\'' {
                in_multi_literal = false;
                i += 3;
                continue;
            }
            i += 1;
            continue;
        }

        // Inside a single-line basic string. Honors `\` escapes; newline
        // closes the scope defensively (TOML forbids unescaped newlines
        // in basic strings, but a malformed input must not strand the
        // scanner in the wrong mode).
        if in_basic {
            if b == b'\\' && i + 1 < bytes.len() {
                i += 2;
                continue;
            }
            if b == b'"' {
                in_basic = false;
                i += 1;
                continue;
            }
            if b == b'\n' {
                in_basic = false;
                i += 1;
                continue;
            }
            i += 1;
            continue;
        }

        // Inside a single-line literal string. No escapes; newline
        // closes defensively (same reasoning as basic strings).
        if in_literal {
            if b == b'\'' {
                in_literal = false;
                i += 1;
                continue;
            }
            if b == b'\n' {
                in_literal = false;
                i += 1;
                continue;
            }
            i += 1;
            continue;
        }

        // Out of any string. Three openers and one comment marker to
        // recognize, plus newline as the boundary that lets `extract`
        // capture per-line comment bodies.
        if b == b'#' {
            let start = i + 1;
            let mut end = start;
            while end < bytes.len() && bytes[end] != b'\n' {
                end += 1;
            }
            // `start..end` is ASCII-safe inside the slice because we
            // only consumed `#` (a single ASCII byte) and stopped at
            // either end-of-input or `\n` (another single ASCII byte);
            // we never split a multibyte UTF-8 codepoint. `text` is
            // valid UTF-8, so the slice is too.
            let body = &text[start..end];
            out.push(body.trim().to_string());
            i = end;
            continue;
        }

        if b == b'"' {
            if i + 2 < bytes.len() && bytes[i + 1] == b'"' && bytes[i + 2] == b'"' {
                in_multi_basic = true;
                i += 3;
                continue;
            }
            in_basic = true;
            i += 1;
            continue;
        }

        if b == b'\'' {
            if i + 2 < bytes.len() && bytes[i + 1] == b'\'' && bytes[i + 2] == b'\'' {
                in_multi_literal = true;
                i += 3;
                continue;
            }
            in_literal = true;
            i += 1;
            continue;
        }

        i += 1;
    }

    out
}

/// Single-line variant kept for the unit tests that exercise the
/// per-line classification logic. Real scanning goes through
/// [`scan_dropped_comments`] which handles multi-line strings.
#[cfg(test)]
fn extract_unquoted_comment(line: &str) -> Option<String> {
    let comments = scan_dropped_comments(line);
    comments.into_iter().next()
}

/// Human-readable name for a [`toml::Value`] variant. Used in error
/// messages so the diagnostic names the actual shape encountered rather
/// than echoing a generic "wrong type".
fn type_name_of(v: &toml::Value) -> &'static str {
    match v {
        toml::Value::String(_) => "string",
        toml::Value::Integer(_) => "integer",
        toml::Value::Float(_) => "float",
        toml::Value::Boolean(_) => "boolean",
        toml::Value::Datetime(_) => "datetime",
        toml::Value::Array(_) => "array",
        toml::Value::Table(_) => "table",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonicalize_inserts_dylib_rlib_when_absent() {
        let mut t = toml::map::Map::new();
        canonicalize_crate_type(&mut t).unwrap();
        let ct = t.get("crate-type").unwrap().as_array().unwrap();
        let strs: Vec<&str> = ct.iter().filter_map(|v| v.as_str()).collect();
        assert_eq!(strs, vec!["dylib", "rlib"]);
    }

    #[test]
    fn canonicalize_prepends_dylib_to_rlib_only() {
        let mut t = toml::map::Map::new();
        t.insert(
            "crate-type".into(),
            toml::Value::Array(vec![toml::Value::String("rlib".into())]),
        );
        canonicalize_crate_type(&mut t).unwrap();
        let ct = t.get("crate-type").unwrap().as_array().unwrap();
        let strs: Vec<&str> = ct.iter().filter_map(|v| v.as_str()).collect();
        assert_eq!(strs, vec!["dylib", "rlib"]);
    }

    #[test]
    fn canonicalize_appends_rlib_when_only_dylib() {
        let mut t = toml::map::Map::new();
        t.insert(
            "crate-type".into(),
            toml::Value::Array(vec![toml::Value::String("dylib".into())]),
        );
        canonicalize_crate_type(&mut t).unwrap();
        let ct = t.get("crate-type").unwrap().as_array().unwrap();
        let strs: Vec<&str> = ct.iter().filter_map(|v| v.as_str()).collect();
        assert_eq!(strs, vec!["dylib", "rlib"]);
    }

    #[test]
    fn canonicalize_preserves_cdylib_after_pair() {
        let mut t = toml::map::Map::new();
        t.insert(
            "crate-type".into(),
            toml::Value::Array(vec![toml::Value::String("cdylib".into())]),
        );
        canonicalize_crate_type(&mut t).unwrap();
        let ct = t.get("crate-type").unwrap().as_array().unwrap();
        let strs: Vec<&str> = ct.iter().filter_map(|v| v.as_str()).collect();
        assert_eq!(strs, vec!["dylib", "rlib", "cdylib"]);
    }

    #[test]
    fn canonicalize_dedups_duplicates() {
        let mut t = toml::map::Map::new();
        t.insert(
            "crate-type".into(),
            toml::Value::Array(vec![
                toml::Value::String("rlib".into()),
                toml::Value::String("dylib".into()),
                toml::Value::String("rlib".into()),
                toml::Value::String("cdylib".into()),
            ]),
        );
        canonicalize_crate_type(&mut t).unwrap();
        let ct = t.get("crate-type").unwrap().as_array().unwrap();
        let strs: Vec<&str> = ct.iter().filter_map(|v| v.as_str()).collect();
        assert_eq!(strs, vec!["dylib", "rlib", "cdylib"]);
    }

    #[test]
    fn canonicalize_rejects_non_string_element() {
        let mut t = toml::map::Map::new();
        t.insert(
            "crate-type".into(),
            toml::Value::Array(vec![toml::Value::Integer(1)]),
        );
        let err = canonicalize_crate_type(&mut t).unwrap_err();
        let s = format!("{err:?}");
        assert!(
            s.contains("not a string"),
            "diagnostic must name the failure: {s}"
        );
    }

    #[test]
    fn canonical_key_order_starts_with_package() {
        assert_eq!(canonical_key_order()[0], "package");
    }

    #[test]
    fn extract_unquoted_comment_strips_leading_hash() {
        assert_eq!(
            extract_unquoted_comment("# a leading comment"),
            Some("a leading comment".into())
        );
    }

    #[test]
    fn extract_unquoted_comment_handles_trailing() {
        assert_eq!(
            extract_unquoted_comment(r#"name = "demo" # trailing"#),
            Some("trailing".into())
        );
    }

    #[test]
    fn extract_unquoted_comment_ignores_hash_inside_string() {
        assert_eq!(
            extract_unquoted_comment(r#"url = "http://example.com/#anchor""#),
            None
        );
    }

    #[test]
    fn extract_unquoted_comment_ignores_hash_inside_single_quote() {
        assert_eq!(extract_unquoted_comment(r#"name = 'foo#bar'"#), None);
    }

    #[test]
    fn scan_ignores_hash_inside_multiline_basic_string() {
        let text = "description = \"\"\"\nline with #notacomment\n\"\"\"\n";
        let comments = scan_dropped_comments(text);
        assert!(
            comments.iter().all(|c| !c.contains("notacomment")),
            "multi-line basic string body must not be classified as a comment; got {comments:?}",
        );
    }

    #[test]
    fn scan_ignores_hash_inside_multiline_literal_string() {
        let text = "description = '''\nline with #stillnotacomment\n'''\n";
        let comments = scan_dropped_comments(text);
        assert!(
            comments.iter().all(|c| !c.contains("stillnotacomment")),
            "multi-line literal string body must not be classified as a comment; got {comments:?}",
        );
    }

    #[test]
    fn scan_recognizes_comment_after_multiline_string_closes() {
        // A trailing `#real comment` after the closing `"""` must still
        // surface — the close pop is load-bearing.
        let text = "description = \"\"\"\nblock\n\"\"\" # real comment\n";
        let comments = scan_dropped_comments(text);
        assert!(
            comments.iter().any(|c| c == "real comment"),
            "comment AFTER the multi-line string close must be captured; got {comments:?}",
        );
        assert!(
            comments.iter().all(|c| !c.contains("block")),
            "multi-line body must never appear as a comment; got {comments:?}",
        );
    }

    #[test]
    fn scan_basic_string_escape_does_not_strand_state() {
        // `"foo \" bar"` is a single basic string; the escaped `"` must
        // not flip the in-string flag off and let a subsequent `#` leak
        // as a comment.
        let text = "name = \"foo \\\" #notacomment\"\n# real\n";
        let comments = scan_dropped_comments(text);
        assert!(
            !comments.iter().any(|c| c.contains("notacomment")),
            "escaped quote inside basic string must keep scanner in-string; got {comments:?}",
        );
        assert!(
            comments.iter().any(|c| c == "real"),
            "comment on the following line must still be captured; got {comments:?}",
        );
    }

    #[test]
    fn post_process_strips_trailing_whitespace() {
        let raw = "foo = 1  \nbar = 2\t\n";
        let out = post_process_output(raw);
        assert!(out.lines().all(|l| !l.ends_with(' ') && !l.ends_with('\t')));
    }

    #[test]
    fn post_process_strips_cr() {
        let raw = "foo = 1\r\nbar = 2\r\n";
        let out = post_process_output(raw);
        assert!(!out.contains('\r'));
    }

    #[test]
    fn post_process_collapses_blank_runs() {
        let raw = "foo = 1\n\n\n\nbar = 2\n";
        let out = post_process_output(raw);
        assert_eq!(out, "foo = 1\n\nbar = 2\n");
    }

    #[test]
    fn serialize_canonical_emits_package_first() {
        let input = r#"
[features]
default = []

[dependencies]
serde = "1"

[package]
name = "demo"
version = "0.1.0"
"#;
        let val: toml::Value = toml::from_str(input).unwrap();
        let bytes = serialize_canonical(&val).unwrap();
        let out = String::from_utf8(bytes).unwrap();
        let first_header = out.lines().find(|l| l.starts_with('[')).unwrap();
        assert_eq!(first_header, "[package]", "got:\n{out}");
    }
}
