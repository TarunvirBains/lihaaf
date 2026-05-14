//! Phase 8 of compat mode (§3.3 of `docs/compatibility-plan.md`) —
//! deterministic JSON envelope writer.
//!
//! The §3.3 envelope is the single artifact CI consumes to make the
//! pilot-gate pass/fail decision. The byte layout must be reproducible:
//! two runs of compat mode against the same fixtures (with the only
//! variability being wall-clock duration) must produce envelopes that
//! are byte-identical modulo the recorded `dur_ms` values. That property
//! is what lets a reviewer compare envelopes across re-runs without
//! chasing spurious diffs in field ordering or sort key choice.
//!
//! ## What this module owns
//!
//! - [`CompatEnvelope`] and its nested types — the on-disk JSON schema,
//!   `schema_version: 1`.
//! - [`write_envelope`] — sort every list field, serialize through
//!   `serde_json::to_string_pretty`, append a trailing newline, and
//!   atomic-write to disk.
//! - Sort-in-the-writer-not-in-the-caller policy: every caller hands
//!   the writer a `&mut CompatEnvelope` and the writer canonicalizes
//!   the order of every `Vec` field before serialization. Callers do
//!   not need to remember the sort key for each list.
//!
//! ## What this module does NOT own
//!
//! - Construction of the envelope. Each upstream phase (overlay,
//!   baseline, discovery, normalizer, cleanup) is responsible for
//!   producing its own contribution; the driver (Phase 9+) assembles
//!   the pieces and hands the final struct to [`write_envelope`].
//! - Conversion from absolute filesystem paths to the repo-relative
//!   forward-slash form the envelope stores. Callers are responsible
//!   for that conversion at construction time (e.g. via the
//!   crate-internal `util::to_forward_slash` helper +
//!   [`std::path::Path::strip_prefix`]). The writer asserts no path
//!   conversion of its own; the input must already be in canonical
//!   form.
//!
//! ## Locked decisions
//!
//! 1. **JSON, not TOML.** §3.3 of the spec is explicit. The reasoning
//!    matches the rationale in `src/manifest.rs`: `jq`-friendly,
//!    cross-tool readable, and `serde_json` is already a hard dep.
//! 2. **`Serialize` / `Deserialize` with `preserve_order`.** The
//!    `preserve_order` feature is enabled on `serde_json` in
//!    `Cargo.toml:55`. That feature controls how a `Map<String, Value>`
//!    preserves insertion order; it does NOT directly control struct
//!    field serialization order. Struct fields serialize in declaration
//!    order regardless of `preserve_order`. The
//!    `field_declaration_order_matches_on_disk_layout` integration test
//!    (in `tests/compat/report_determinism.rs`) asserts this empirically
//!    so the test bites if the behavior ever changes.
//! 3. **`mismatch_type` / `error_type` are stringly-typed.** Lets v0.2
//!    add new types without a `schema_version` bump; v0.1 consumers
//!    parse known string values and skip unknowns. The corresponding
//!    Rust enums are intentionally NOT introduced — the spec calls
//!    out a small finite set today, but the envelope's value space is
//!    the on-disk set, not the in-process set.
//! 4. **`dur_ms` is excluded from determinism by TEST, not by
//!    serialization.** The field is always written (the value just
//!    changes per run); the determinism test strips the `dur_ms` lines
//!    before byte-equality comparison. This keeps the on-disk schema
//!    fully populated for downstream consumers that DO want timing
//!    information (e.g. a CI dashboard plot).
//! 5. **Sort happens INSIDE the writer, not in the callers.** Every
//!    `Vec` field on [`CompatEnvelope`] is sorted by [`write_envelope`]
//!    before serialization. Callers may construct the envelope in any
//!    order. The sort keys are documented per field; all are byte-order
//!    on ASCII strings (paths are forward-slash, error types are
//!    kebab-case), so sorts are locale-free.
//! 6. **Envelope-side `GeneratedPath` is distinct from
//!    [`crate::compat::cleanup::GeneratedPath`].** The cleanup-side
//!    type carries an absolute [`std::path::PathBuf`] and a non-`Serialize`
//!    classification enum; the envelope-side type carries a
//!    repo-relative forward-slash `String` path and a string-valued
//!    class for additive v0.2 evolution. [`generated_path_from_cleanup`]
//!    converts between the two — the driver calls this after Phase 5
//!    cleanup finalizes, before envelope construction.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::Error;
use crate::util;

/// §3.3 envelope. `Serialize`/`Deserialize` round-trips through
/// `serde_json::to_string_pretty` with `preserve_order` so the
/// on-disk byte layout matches this struct's field declaration order.
///
/// Field order is part of the schema contract: a downstream
/// human reader (and CI grep patterns) anchor on the first few lines
/// containing `"schema_version": 1, "mode": "compat", ...`. The
/// `field_declaration_order_matches_on_disk_layout` integration test
/// asserts this empirically.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CompatEnvelope {
    /// Integer envelope schema version. Currently `1`. Breaking changes
    /// increment; additive fields do not.
    pub schema_version: u32,
    /// Always `"compat"` for v0.1. Reserved for future mode-style
    /// variants (e.g. a `"strict"` mode that tightens conservatism).
    pub mode: String,
    /// Crate name from the upstream `[package].name` (read out of the
    /// upstream `Cargo.toml`).
    pub crate_name: String,
    /// Commit SHA from `--compat-commit`, or empty string when omitted.
    pub commit: String,
    /// Commands actually executed. Always populated.
    pub commands: Commands,
    /// Pass/fail counts plus exit codes and `dur_ms`.
    pub results: Results,
    /// Per-fixture mismatches, sorted by `fixture` (repo-relative
    /// forward-slash ASCII byte order).
    pub mismatch_examples: Vec<MismatchExample>,
    /// Infrastructure / discovery / overlay errors, sorted by
    /// `(file, line, error_type)`. `error_type` is the third sort key
    /// so two errors at the same source location stay
    /// deterministically ordered.
    pub errors: Vec<EnvelopeError>,
    /// Fixtures intentionally skipped, sorted by `fixture`.
    pub excluded_fixtures: Vec<ExcludedFixture>,
    /// Generated paths, sorted by `path`. Per Phase 5 / issue #10.
    pub generated_paths: Vec<GeneratedPath>,
    /// Overlay metadata (`dropped_comments`, etc.).
    pub overlay: OverlayMetadata,
    /// Toolchain info from §3.4 (recorded but not gated). Typically the
    /// `rustc --version --verbose` first-line value captured at startup.
    pub toolchain: String,
}

/// Shell-renderable command strings for human display.
///
/// The strings are formatted for one-line copy/paste into a terminal,
/// NOT for re-execution by a shell (§3.1 forbids shell command lines
/// throughout compat mode). The §3.3 envelope records them for the
/// operator to inspect, not for tooling to re-run.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Commands {
    /// The baseline `cargo test` invocation rendered as a shell-style
    /// string.
    pub baseline: String,
    /// The lihaaf compat-mode invocation rendered as a shell-style
    /// string.
    pub lihaaf: String,
}

/// Pass/fail counts plus the mismatch tally.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Results {
    /// Baseline (`cargo test`) per-fixture counts.
    pub baseline: BaselineCounts,
    /// Lihaaf per-fixture counts.
    pub lihaaf: LihaafCounts,
    /// Number of `mismatch_examples` entries. Maintained alongside the
    /// list so the §5 gate can read a scalar without parsing the array.
    pub mismatch_count: u32,
}

/// Baseline (`cargo test`) per-fixture counts.
///
/// `dur_ms` is intentionally NOT excluded from serialization — see
/// locked decision §4 in the module header for the rationale. The
/// determinism test strips `dur_ms` lines before byte comparison.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BaselineCounts {
    /// Number of fixtures the baseline reported `pass` for.
    pub pass: u32,
    /// Number of fixtures the baseline reported `fail` for.
    pub fail: u32,
    /// Number of libtest output lines the conservative parser
    /// (Phase 4 / issue #9) could not correlate to a recognized
    /// fixture. Always present (`0` when every line correlated).
    pub unknown_count: u32,
    /// `cargo test`'s process exit code.
    pub exit_code: i32,
    /// Wall-clock duration in milliseconds. EXCLUDED from byte-equality
    /// determinism checks by the test, not by serialization.
    pub dur_ms: u64,
}

/// Lihaaf per-fixture counts.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LihaafCounts {
    /// Number of fixtures lihaaf reported `pass` for.
    pub pass: u32,
    /// Number of fixtures lihaaf reported `fail` for.
    pub fail: u32,
    /// Lihaaf's process exit code.
    pub exit_code: i32,
    /// Wall-clock duration in milliseconds. EXCLUDED from byte-equality
    /// determinism checks by the test, not by serialization.
    pub dur_ms: u64,
    /// Rustc identity at the dispatch time, captured per §3.4. The same
    /// value lands in [`CompatEnvelope::toolchain`]; duplicated here so
    /// future v0.2 work that records mid-session toolchain mutation has
    /// a per-side field to populate without changing the schema.
    pub toolchain: String,
}

/// One mismatch entry surfaced for §5 gate evaluation.
///
/// Stored sorted by `fixture` (repo-relative forward-slash ASCII byte
/// order) in [`CompatEnvelope::mismatch_examples`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MismatchExample {
    /// Repo-relative forward-slash path to the fixture. Sort key.
    pub fixture: String,
    /// Stringly-typed mismatch kind. Known values:
    /// `"baseline_only_fail"`, `"lihaaf_only_fail"`, `"verdict_mismatch"`,
    /// `"snapshot_mismatch"` (subtype `"non_span_path_rewrite"` is
    /// possible), `"infra_error"`. v0.2 may add new values; v0.1
    /// consumers skip unknowns.
    pub mismatch_type: String,
    /// Human-readable detail surfaced to the operator.
    pub notes: String,
}

/// One envelope-level error entry. Sorted by `(file, line, error_type)`
/// in [`CompatEnvelope::errors`].
///
/// `fixture` is optional because infrastructure errors (e.g.
/// `manifest_overlay_failed`, `toolchain_drift`) may not have a
/// per-fixture site, and serializing `null` for those would clutter the
/// JSON. `skip_serializing_if = "Option::is_none"` keeps the absent
/// case absent on disk.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EnvelopeError {
    /// Stringly-typed error kind. Known values:
    /// `"discovery_unrecognized"`, `"toolchain_drift"`,
    /// `"manifest_overlay_failed"`, `"overlay_serializer_drift"`,
    /// `"baseline_unknown"`, `"snapshot_mismatch"`. v0.2 may add new
    /// values; v0.1 consumers skip unknowns. Tiebreak sort key after
    /// `(file, line)`.
    pub error_type: String,
    /// Repo-relative forward-slash fixture path, when the error has
    /// fixture-level granularity. Absent on disk when `None`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fixture: Option<String>,
    /// Source file the error references (repo-relative forward-slash).
    /// Primary sort key.
    pub file: String,
    /// Line number in `file`. Secondary sort key.
    pub line: u32,
    /// Human-readable detail.
    pub detail: String,
}

/// One excluded fixture entry. Sorted by `fixture` in
/// [`CompatEnvelope::excluded_fixtures`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExcludedFixture {
    /// Repo-relative forward-slash fixture path.
    pub fixture: String,
    /// Human-readable reason the fixture was excluded.
    pub reason: String,
}

/// One generated-path entry. Distinct from
/// [`crate::compat::cleanup::GeneratedPath`] — the cleanup-side type
/// carries absolute paths and a non-`Serialize` enum; this one carries
/// repo-relative forward-slash strings for envelope consumption.
///
/// Sorted by `path` (ASCII byte order) in
/// [`CompatEnvelope::generated_paths`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GeneratedPath {
    /// Repo-relative forward-slash path.
    pub path: String,
    /// Stringly-typed classification. Known values: `"committed"`,
    /// `"ignored"`, `"cleaned"`, `"kept"`. Matches the lower-cased
    /// names of [`crate::compat::cleanup::GeneratedPathClass`] for
    /// human readability.
    pub class: String,
}

/// Overlay metadata recorded by Phase 2.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OverlayMetadata {
    /// `true` when the overlay was materialized this run. Always
    /// `true` in v0.1 because the driver always generates a sibling
    /// overlay; reserved for a future `--no-overlay` debug flag.
    pub generated: bool,
    /// Comments scraped from the upstream `Cargo.toml`'s source text.
    /// Sorted lexicographically (ASCII byte order).
    pub dropped_comments: Vec<String>,
    /// `true` when the upstream `[lib] crate-type` already contained
    /// `"dylib"` so the overlay's `crate-type` canonicalization was a
    /// no-op for that dimension.
    pub upstream_already_has_dylib: bool,
}

/// Convert a [`crate::compat::cleanup::GeneratedPath`] (cleanup-side,
/// absolute path) into the envelope-side
/// [`GeneratedPath`] (repo-relative forward-slash string).
///
/// `compat_root` is the adopter's `--compat-root` directory. The path
/// is stripped of the prefix and rendered forward-slash via the
/// crate-internal `util::to_forward_slash` helper. If the path is not
/// under `compat_root` (which would indicate a driver bug — every
/// tracked path is supposed to live under the adopter's checkout),
/// the result preserves the full path verbatim, again in forward-
/// slash form, so the envelope is still readable.
///
/// The cleanup-side classification enum is stringified to its
/// lower-case discriminant name for v0.2-additive evolution. See the
/// `class` field documentation on [`GeneratedPath`] for the known
/// value set.
#[allow(dead_code)] // Phase 9 wires this in `compat::run`; tests exercise it via the re-export.
pub fn generated_path_from_cleanup(
    cleanup_entry: &crate::compat::cleanup::GeneratedPath,
    compat_root: &Path,
) -> GeneratedPath {
    let rel = util::relative_to(&cleanup_entry.path, compat_root);
    let class = match cleanup_entry.class {
        crate::compat::cleanup::GeneratedPathClass::Committed => "committed",
        crate::compat::cleanup::GeneratedPathClass::Ignored => "ignored",
        crate::compat::cleanup::GeneratedPathClass::Cleaned => "cleaned",
        crate::compat::cleanup::GeneratedPathClass::Kept => "kept",
    };
    GeneratedPath {
        path: rel,
        class: class.to_string(),
    }
}

/// Canonicalize every list field on `envelope` to its determinism-
/// preserving sort order.
///
/// Mutates in place. Idempotent — calling twice produces the same
/// post-state. Exposed as a separate step (rather than inlined into
/// [`write_envelope`]) so the determinism test can call it without
/// serializing.
pub fn canonicalize(envelope: &mut CompatEnvelope) {
    envelope
        .mismatch_examples
        .sort_by(|a, b| a.fixture.cmp(&b.fixture));
    envelope.errors.sort_by(|a, b| {
        a.file
            .cmp(&b.file)
            .then_with(|| a.line.cmp(&b.line))
            .then_with(|| a.error_type.cmp(&b.error_type))
    });
    envelope
        .excluded_fixtures
        .sort_by(|a, b| a.fixture.cmp(&b.fixture));
    envelope.generated_paths.sort_by(|a, b| a.path.cmp(&b.path));
    envelope.overlay.dropped_comments.sort();
}

/// Write `envelope` to `path` in canonical, deterministic form.
///
/// 1. Sort every list field in place (see [`canonicalize`]).
/// 2. Serialize through `serde_json::to_string_pretty`. Struct fields
///    serialize in declaration order; the `preserve_order` feature on
///    `serde_json` keeps `Map<String, Value>` insertion order stable
///    (no maps appear in this schema today, but the feature is enabled
///    for forward-compatibility with additive v0.2 fields).
/// 3. Append a trailing `\n` so `cat` output reads cleanly and
///    line-oriented diff tools do not flag a missing final newline.
/// 4. Atomic write through the crate-internal `util::write_file_atomic`
///    helper so a SIGKILL mid-write cannot leave a half-formed envelope
///    for the operator to chase.
///
/// Takes `&mut CompatEnvelope` because step 1 mutates the field order.
/// Callers that need to re-use the envelope after write observe the
/// sorted state — which is a benefit, not a hazard: a second
/// [`write_envelope`] call produces byte-identical output, matching
/// the idempotency guarantee.
pub fn write_envelope(envelope: &mut CompatEnvelope, path: &Path) -> Result<(), Error> {
    canonicalize(envelope);

    let mut text = serde_json::to_string_pretty(envelope).map_err(|e| Error::JsonParse {
        context: "serializing compat envelope".into(),
        message: e.to_string(),
    })?;
    text.push('\n');

    util::write_file_atomic(path, text.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Minimal envelope for unit testing the sorter / writer without
    /// pulling in the full integration corpus.
    fn sample_envelope() -> CompatEnvelope {
        CompatEnvelope {
            schema_version: 1,
            mode: "compat".into(),
            crate_name: "demo".into(),
            commit: String::new(),
            commands: Commands {
                baseline: "cargo test".into(),
                lihaaf: "cargo lihaaf --compat --compat-root .".into(),
            },
            results: Results {
                baseline: BaselineCounts {
                    pass: 0,
                    fail: 0,
                    unknown_count: 0,
                    exit_code: 0,
                    dur_ms: 0,
                },
                lihaaf: LihaafCounts {
                    pass: 0,
                    fail: 0,
                    exit_code: 0,
                    dur_ms: 0,
                    toolchain: "rustc 1.95.0".into(),
                },
                mismatch_count: 0,
            },
            mismatch_examples: Vec::new(),
            errors: Vec::new(),
            excluded_fixtures: Vec::new(),
            generated_paths: Vec::new(),
            overlay: OverlayMetadata {
                generated: true,
                dropped_comments: Vec::new(),
                upstream_already_has_dylib: false,
            },
            toolchain: "rustc 1.95.0".into(),
        }
    }

    #[test]
    fn canonicalize_is_idempotent() {
        let mut env = sample_envelope();
        env.mismatch_examples = vec![
            MismatchExample {
                fixture: "tests/z.rs".into(),
                mismatch_type: "verdict_mismatch".into(),
                notes: String::new(),
            },
            MismatchExample {
                fixture: "tests/a.rs".into(),
                mismatch_type: "verdict_mismatch".into(),
                notes: String::new(),
            },
        ];
        canonicalize(&mut env);
        let after_first = env.clone();
        canonicalize(&mut env);
        assert_eq!(
            after_first, env,
            "canonicalize must be idempotent across repeated calls"
        );
        assert_eq!(env.mismatch_examples[0].fixture, "tests/a.rs");
        assert_eq!(env.mismatch_examples[1].fixture, "tests/z.rs");
    }

    #[test]
    fn errors_sort_by_file_then_line_then_type() {
        let mut env = sample_envelope();
        env.errors = vec![
            EnvelopeError {
                error_type: "z_type".into(),
                fixture: None,
                file: "tests/foo.rs".into(),
                line: 10,
                detail: String::new(),
            },
            EnvelopeError {
                error_type: "a_type".into(),
                fixture: None,
                file: "tests/foo.rs".into(),
                line: 10,
                detail: String::new(),
            },
            EnvelopeError {
                error_type: "m_type".into(),
                fixture: None,
                file: "tests/bar.rs".into(),
                line: 100,
                detail: String::new(),
            },
        ];
        canonicalize(&mut env);
        assert_eq!(env.errors[0].file, "tests/bar.rs");
        assert_eq!(env.errors[1].file, "tests/foo.rs");
        assert_eq!(
            env.errors[1].error_type, "a_type",
            "errors at the same (file, line) must tiebreak by error_type"
        );
        assert_eq!(env.errors[2].error_type, "z_type");
    }
}
