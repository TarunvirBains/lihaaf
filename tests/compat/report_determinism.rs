//! Phase 8 of compat mode (§3.3 of `docs/compatibility-plan.md`) —
//! deterministic JSON envelope writer integration tests.
//!
//! Every test in this file reaches the envelope writer through the
//! `#[doc(hidden)]` re-exports declared in `src/lib.rs`
//! (`lihaaf::CompatEnvelope`, `lihaaf::compat_write_envelope`, etc.).
//! The re-exports exist exclusively for this test crate (and for the
//! `cargo-lihaaf` binary, once Phase 9 wires the writer into
//! `compat::run`). The v0.1 supported entry to compat mode is `cargo
//! lihaaf --compat`, not the Rust API.
//!
//! ## The contract under test
//!
//! `docs/compatibility-plan.md` §3.3 — the envelope is the single
//! artifact CI consumes for the pilot gate. Determinism guarantees:
//!
//! 1. Every list field is sorted before serialization.
//! 2. Every path is repo-relative + forward-slash.
//! 3. Struct field declaration order matches on-disk JSON layout.
//! 4. `dur_ms` is written but excluded from byte-equality checks.
//! 5. `serde_json::to_string_pretty` with `preserve_order` produces the
//!    declared field order; final byte is `\n`.
//!
//! ## Why every test is hermetic
//!
//! Each test owns a `tempfile::TempDir` (or no filesystem at all) and
//! synthesizes envelopes via the public constructor types. The tests
//! that exercise byte equality / round-trip write into the tempdir and
//! read back; everything else operates purely in memory. No test
//! touches the lihaaf source tree.
//!
//! ## What every test bites
//!
//! - `envelope_round_trips_through_disk`: bites if the writer fails to
//!   produce a valid `Serialize`/`Deserialize` round-trip on the v1
//!   schema, or if the trailing-newline rule regresses.
//! - `two_runs_byte_equal_modulo_dur_ms`: bites if any list field is
//!   sorted differently between two synthesized envelopes that should
//!   produce byte-identical output, or if `dur_ms` accidentally leaks
//!   into a non-`dur_ms` line.
//! - `mismatch_examples_sorted_by_fixture` / `errors_sorted_by_file_then_line`
//!   / `excluded_fixtures_sorted_by_fixture` / `generated_paths_sorted_by_path`:
//!   bite if a caller passes an unsorted list and the writer fails to
//!   canonicalize before serialization.
//! - `paths_are_repo_relative_forward_slash`: bites if a path field is
//!   serialized with backslashes (Windows native) or an absolute prefix
//!   (`/abs/...` or `C:/...`).
//! - `schema_version_is_one` / `mode_is_compat`: bite if a refactor
//!   accidentally bumps the schema version without a documented
//!   migration, or if the mode literal is renamed.
//! - `additive_field_evolution_compatibility`: bites if a `#[serde(...)]`
//!   attribute is added that flips `deny_unknown_fields` on, which
//!   would break v0.2 forward-compatibility.
//! - `gate_fields_present`: bites if a refactor renames or removes a
//!   field the §5 CI gate reads, silently breaking the pilot gate.
//! - `field_declaration_order_matches_on_disk_layout`: bites if
//!   `preserve_order` semantics change in a future `serde_json` release,
//!   or if a refactor reorders struct fields in a way the §3.3 schema
//!   does not authorize.

use std::path::PathBuf;

use lihaaf::{
    CompatBaselineCounts as BaselineCounts, CompatCommands as Commands, CompatEnvelope,
    CompatEnvelopeError as EnvelopeError, CompatEnvelopeGeneratedPath as GeneratedPath,
    CompatExcludedFixture as ExcludedFixture, CompatGeneratedPath as CleanupGeneratedPath,
    CompatGeneratedPathClass as CleanupGeneratedPathClass, CompatLihaafCounts as LihaafCounts,
    CompatMismatchExample as MismatchExample, CompatOverlayMetadata as OverlayMetadata,
    CompatResults as Results, compat_envelope_generated_path_from_cleanup as from_cleanup,
    compat_normalize_error_detail_paths as normalize_error_detail_paths,
    compat_write_envelope as write_envelope,
};

/// Build a minimal CompatEnvelope with no list entries. Tests that
/// exercise a specific list field overwrite that field after calling
/// this; tests that only inspect scalar fields use the empty default.
fn empty_envelope() -> CompatEnvelope {
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
                toolchain: "rustc 1.95.0 (abc 2026-01-01)".into(),
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
        toolchain: "rustc 1.95.0 (abc 2026-01-01)".into(),
    }
}

/// Helper: write an envelope to `<tmp>/compat-envelope.json` and
/// return both the path and the file's bytes. Reused by every test
/// that needs an on-disk write.
fn write_to_tmp(envelope: &mut CompatEnvelope) -> (tempfile::TempDir, PathBuf, Vec<u8>) {
    let tmp = tempfile::tempdir().expect("tempdir for envelope write");
    let path = tmp.path().join("compat-envelope.json");
    write_envelope(envelope, &path).expect("envelope write must succeed");
    let bytes = std::fs::read(&path).expect("envelope must be readable after write");
    (tmp, path, bytes)
}

/// Helper: drop the two `dur_ms` lines from a serialized envelope so
/// two runs that differ only in timing produce byte-identical output.
/// The §3.3 contract is that `dur_ms` is always written but excluded
/// from determinism comparisons — implemented in tests, not by the
/// serializer. The strip is line-based and looks for the literal
/// substring `"dur_ms":` after stripping leading whitespace, so it is
/// robust to pretty-printer indent changes.
fn strip_dur_ms_lines(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for line in text.lines() {
        if !line.trim_start().starts_with("\"dur_ms\":") {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

/// Test 1 — write the envelope, read it back via `serde_json::from_str`,
/// compare struct equality. Pins the round-trip invariant for the v1
/// schema.
#[test]
fn envelope_round_trips_through_disk() {
    let mut original = empty_envelope();
    original.mismatch_examples = vec![MismatchExample {
        fixture: "tests/foo.rs".into(),
        mismatch_type: "verdict_mismatch".into(),
        notes: "lihaaf=pass, baseline=fail".into(),
    }];
    original.errors = vec![EnvelopeError {
        error_type: "discovery_unrecognized".into(),
        fixture: Some("tests/bar.rs".into()),
        file: "tests/bar.rs".into(),
        line: 12,
        detail: "unknown trybuild invocation pattern".into(),
    }];
    original.excluded_fixtures = vec![ExcludedFixture {
        fixture: "tests/baz.rs".into(),
        reason: "manual --compat-exclude".into(),
    }];
    original.generated_paths = vec![GeneratedPath {
        path: "target/lihaaf-overlay/Cargo.toml".into(),
        class: "cleaned".into(),
    }];

    let (_tmp, _path, bytes) = write_to_tmp(&mut original);
    let text = std::str::from_utf8(&bytes).expect("envelope must be valid UTF-8");
    let parsed: CompatEnvelope =
        serde_json::from_str(text).expect("envelope must round-trip via serde_json");
    assert_eq!(
        parsed, original,
        "round-trip through disk must preserve every field exactly"
    );
    assert!(
        bytes.ends_with(b"\n"),
        "envelope must end with a trailing newline"
    );
}

/// Test 2 — two envelopes that differ only in `dur_ms` produce
/// byte-identical output after `dur_ms` lines are stripped. This is
/// the load-bearing determinism property the §5 gate relies on: a
/// reviewer comparing two re-runs of compat mode must see zero diff
/// outside the timing fields.
///
/// Synthesizes both envelopes from the same fixture rather than
/// re-running compat mode end-to-end — the real-cargo run is Phase 10
/// work. Phase 8's contract is "the serializer is deterministic
/// modulo `dur_ms`", which this test verifies by construction.
#[test]
fn two_runs_byte_equal_modulo_dur_ms() {
    let mut first = empty_envelope();
    first.mismatch_examples = vec![MismatchExample {
        fixture: "tests/x.rs".into(),
        mismatch_type: "snapshot_mismatch".into(),
        notes: "diff in line 42".into(),
    }];
    first.errors = vec![EnvelopeError {
        error_type: "snapshot_mismatch".into(),
        fixture: Some("tests/x.rs".into()),
        file: "tests/x.rs".into(),
        line: 42,
        detail: "expected -> actual".into(),
    }];
    first.results.baseline.dur_ms = 1234;
    first.results.lihaaf.dur_ms = 567;

    let mut second = first.clone();
    second.results.baseline.dur_ms = 9999;
    second.results.lihaaf.dur_ms = 8888;

    let (_tmp1, _path1, b1) = write_to_tmp(&mut first);
    let (_tmp2, _path2, b2) = write_to_tmp(&mut second);

    let s1 = std::str::from_utf8(&b1).expect("envelope must be valid UTF-8");
    let s2 = std::str::from_utf8(&b2).expect("envelope must be valid UTF-8");

    // Before strip, the two envelopes MUST differ (the dur_ms values
    // are different). This proves the test setup actually exercises
    // the strip — without it, a tautological pass would slip through.
    assert_ne!(
        s1, s2,
        "test setup invariant: pre-strip output must reflect the dur_ms difference"
    );

    let stripped1 = strip_dur_ms_lines(s1);
    let stripped2 = strip_dur_ms_lines(s2);
    assert_eq!(
        stripped1, stripped2,
        "two runs differing only in dur_ms must produce byte-equal envelopes after strip"
    );
}

/// Test 3 — `mismatch_examples` sorted by `fixture` (ASCII byte order)
/// regardless of caller insertion order. Reversed insertion proves
/// the writer's `canonicalize` step is actually invoked.
#[test]
fn mismatch_examples_sorted_by_fixture() {
    let mut env = empty_envelope();
    env.mismatch_examples = vec![
        MismatchExample {
            fixture: "tests/zeta.rs".into(),
            mismatch_type: "verdict_mismatch".into(),
            notes: String::new(),
        },
        MismatchExample {
            fixture: "tests/alpha.rs".into(),
            mismatch_type: "verdict_mismatch".into(),
            notes: String::new(),
        },
        MismatchExample {
            fixture: "tests/mu.rs".into(),
            mismatch_type: "verdict_mismatch".into(),
            notes: String::new(),
        },
    ];

    let (_tmp, _path, bytes) = write_to_tmp(&mut env);
    let text = std::str::from_utf8(&bytes).expect("UTF-8");
    let alpha_idx = text.find("alpha.rs").expect("alpha must be present");
    let mu_idx = text.find("mu.rs").expect("mu must be present");
    let zeta_idx = text.find("zeta.rs").expect("zeta must be present");
    assert!(
        alpha_idx < mu_idx && mu_idx < zeta_idx,
        "mismatch_examples must serialize in ASCII order of fixture: alpha < mu < zeta"
    );
    // The in-memory envelope must also be sorted after the write
    // (writer sorts in place; idempotent for repeat writes).
    assert_eq!(env.mismatch_examples[0].fixture, "tests/alpha.rs");
    assert_eq!(env.mismatch_examples[1].fixture, "tests/mu.rs");
    assert_eq!(env.mismatch_examples[2].fixture, "tests/zeta.rs");
}

/// Test 4 — `errors` sorted by `(file, line)` with `error_type` as the
/// tiebreak. The tiebreak matters because §3.3 allows multiple errors
/// at the same source location (e.g. two distinct
/// `discovery_unrecognized` reasons firing on the same line).
#[test]
fn errors_sorted_by_file_then_line() {
    let mut env = empty_envelope();
    env.errors = vec![
        EnvelopeError {
            error_type: "snapshot_mismatch".into(),
            fixture: None,
            file: "tests/zeta.rs".into(),
            line: 5,
            detail: String::new(),
        },
        EnvelopeError {
            error_type: "discovery_unrecognized".into(),
            fixture: None,
            file: "tests/alpha.rs".into(),
            line: 100,
            detail: String::new(),
        },
        EnvelopeError {
            error_type: "discovery_unrecognized".into(),
            fixture: None,
            file: "tests/alpha.rs".into(),
            line: 5,
            detail: String::new(),
        },
        // Two errors at the same (file, line) — tiebreak on
        // error_type lex order: `discovery_unrecognized` < `z_marker`.
        EnvelopeError {
            error_type: "z_marker".into(),
            fixture: None,
            file: "tests/alpha.rs".into(),
            line: 5,
            detail: String::new(),
        },
    ];

    let (_tmp, _path, _bytes) = write_to_tmp(&mut env);
    assert_eq!(env.errors[0].file, "tests/alpha.rs");
    assert_eq!(env.errors[0].line, 5);
    assert_eq!(
        env.errors[0].error_type, "discovery_unrecognized",
        "alpha:5 with `discovery_unrecognized` must precede alpha:5 with `z_marker`"
    );
    assert_eq!(env.errors[1].file, "tests/alpha.rs");
    assert_eq!(env.errors[1].line, 5);
    assert_eq!(env.errors[1].error_type, "z_marker");
    assert_eq!(env.errors[2].file, "tests/alpha.rs");
    assert_eq!(env.errors[2].line, 100);
    assert_eq!(env.errors[3].file, "tests/zeta.rs");
}

/// Test 5 — `excluded_fixtures` sorted by `fixture` ASCII byte order.
#[test]
fn excluded_fixtures_sorted_by_fixture() {
    let mut env = empty_envelope();
    env.excluded_fixtures = vec![
        ExcludedFixture {
            fixture: "tests/zeta.rs".into(),
            reason: "skip".into(),
        },
        ExcludedFixture {
            fixture: "tests/alpha.rs".into(),
            reason: "skip".into(),
        },
        ExcludedFixture {
            fixture: "tests/mu.rs".into(),
            reason: "skip".into(),
        },
    ];

    let (_tmp, _path, _bytes) = write_to_tmp(&mut env);
    assert_eq!(env.excluded_fixtures[0].fixture, "tests/alpha.rs");
    assert_eq!(env.excluded_fixtures[1].fixture, "tests/mu.rs");
    assert_eq!(env.excluded_fixtures[2].fixture, "tests/zeta.rs");
}

/// Test 6 — `generated_paths` sorted by `path` ASCII byte order.
///
/// The three paths below mirror the actual production output of a
/// compat run after the PR #34 redesign: a staged overlay under
/// `target/lihaaf-overlay/`, a converted-fixtures tree under
/// `target/lihaaf-compat-converted/`, and a snapshot file under
/// `tests/snapshots/`. The sort honors strict lexicographic byte
/// order — `target/lihaaf-compat-converted/` precedes
/// `target/lihaaf-overlay/Cargo.toml` because at byte position 14 a
/// `c` (0x63) precedes an `o` (0x6f), and `target/...` precedes
/// `tests/...` because at byte position 1 an `a` (0x61) precedes an
/// `e` (0x65).
#[test]
fn generated_paths_sorted_by_path() {
    let mut env = empty_envelope();
    env.generated_paths = vec![
        GeneratedPath {
            path: "tests/snapshots/foo.stderr".into(),
            class: "committed".into(),
        },
        GeneratedPath {
            path: "target/lihaaf-overlay/Cargo.toml".into(),
            class: "cleaned".into(),
        },
        GeneratedPath {
            path: "target/lihaaf-compat-converted/".into(),
            class: "kept".into(),
        },
    ];

    let (_tmp, _path, _bytes) = write_to_tmp(&mut env);
    assert_eq!(
        env.generated_paths[0].path,
        "target/lihaaf-compat-converted/"
    );
    assert_eq!(
        env.generated_paths[1].path,
        "target/lihaaf-overlay/Cargo.toml"
    );
    assert_eq!(env.generated_paths[2].path, "tests/snapshots/foo.stderr");
}

/// Test 7 — every path field surfaces as repo-relative + forward-slash.
/// Asserted by checking the serialized output contains no `\\\\` byte
/// pair (escaped backslash) and no leading-slash absolute paths.
///
/// Note: paths are caller-provided in canonical form per locked
/// decision §5 in the module header — the writer does not do path
/// conversion. So this test exercises the *envelope's* serialization
/// of correctly-prepared paths, not the conversion itself. A
/// regression where someone naively stuffed a `Path::display()` value
/// into the envelope (which on Windows would produce backslashes)
/// would be caught upstream of the writer; here we verify the
/// representation is the one the spec mandates.
#[test]
fn paths_are_repo_relative_forward_slash() {
    let mut env = empty_envelope();
    env.mismatch_examples = vec![MismatchExample {
        fixture: "tests/nested/dir/file.rs".into(),
        mismatch_type: "verdict_mismatch".into(),
        notes: String::new(),
    }];
    env.errors = vec![EnvelopeError {
        error_type: "discovery_unrecognized".into(),
        fixture: Some("tests/another.rs".into()),
        file: "tests/another.rs".into(),
        line: 1,
        detail: String::new(),
    }];
    env.excluded_fixtures = vec![ExcludedFixture {
        fixture: "tests/excluded.rs".into(),
        reason: "skip".into(),
    }];
    env.generated_paths = vec![GeneratedPath {
        path: "target/lihaaf-overlay/Cargo.toml".into(),
        class: "cleaned".into(),
    }];

    let (_tmp, _path, bytes) = write_to_tmp(&mut env);
    let text = std::str::from_utf8(&bytes).expect("UTF-8");

    // No escaped backslashes (JSON `\\` is a literal `\` in the path).
    // serde_json escapes a single backslash as `\\` so the on-disk
    // bytes look like `\\` (two ASCII chars). Searching for the literal
    // four-char string `\\\\` here would match the escape sequence in
    // Rust source, not the disk bytes; we want the two-byte sequence
    // (`\`, `\`) which appears in the file when a backslash was in
    // the original path.
    let backslash_pair = "\\\\"; // two-char Rust literal: `\\`
    assert!(
        !text.contains(backslash_pair),
        "no path field may serialize with backslashes; got envelope:\n{text}"
    );

    // No absolute-prefix paths. We assert this by checking that no
    // value position contains an absolute path. `"path": "/...` (the
    // POSIX absolute form) and `"path": "C:/...` (the Windows form
    // converted to forward-slash) are the two shapes the writer
    // forbids by spec.
    //
    // A naive scan for `: "/"` would false-positive on indent
    // whitespace inside JSON pretty-printing; we instead anchor on
    // the field name + value-open + literal forward-slash. The four
    // path fields are checked individually.
    for field in ["\"fixture\": \"/", "\"file\": \"/", "\"path\": \"/"] {
        assert!(
            !text.contains(field),
            "no path field may start with `/` (POSIX absolute); got envelope:\n{text}"
        );
    }
    // Windows absolute-form check: every uppercase ASCII letter
    // followed by `:/` would match a drive letter. The compact check
    // is `: \"X:/` for any `X` in A..=Z; we test a representative
    // subset to keep the test legible without enumerating 26 cases.
    // The realistic regression vector is a forgotten
    // `path.to_string_lossy()` on Windows; that produces forward-slash
    // but keeps the drive prefix.
    for drive in ["\"C:/", "\"D:/", "\"E:/"] {
        assert!(
            !text.contains(drive),
            "no path field may carry a drive-letter prefix; got envelope:\n{text}"
        );
    }
}

/// Test 8 — `schema_version` is exactly `1` in v0.1. Pins the value
/// so a refactor that bumps the schema without a documented migration
/// path is caught.
#[test]
fn schema_version_is_one() {
    let mut env = empty_envelope();
    assert_eq!(env.schema_version, 1);

    let (_tmp, _path, bytes) = write_to_tmp(&mut env);
    let text = std::str::from_utf8(&bytes).expect("UTF-8");
    assert!(
        text.contains("\"schema_version\": 1"),
        "envelope must serialize `schema_version` as `1`; got:\n{text}"
    );
}

/// Test 9 — `mode` is exactly `"compat"` in v0.1. Reserved literal for
/// the §3.3 schema; a rename would break every downstream consumer.
#[test]
fn mode_is_compat() {
    let mut env = empty_envelope();
    assert_eq!(env.mode, "compat");

    let (_tmp, _path, bytes) = write_to_tmp(&mut env);
    let text = std::str::from_utf8(&bytes).expect("UTF-8");
    assert!(
        text.contains("\"mode\": \"compat\""),
        "envelope must serialize `mode` as `\"compat\"`; got:\n{text}"
    );
}

/// Test 10 — additive field evolution: a JSON document with a v0.2-style
/// unknown field must deserialize back into the v0.1 struct without
/// error. This is the cross-version compatibility invariant the spec
/// guarantees so v0.1 consumers (CI gate scripts, dashboards) keep
/// working against v0.2 envelopes that add new fields.
#[test]
fn additive_field_evolution_compatibility() {
    let mut env = empty_envelope();
    let (_tmp, _path, bytes) = write_to_tmp(&mut env);
    let text = std::str::from_utf8(&bytes).expect("UTF-8");

    // Manually inject an unknown top-level field by inserting a JSON
    // key after `"schema_version": 1,`. Pretty-printed JSON has
    // predictable whitespace; we anchor on the literal substring.
    let injection_anchor = "\"schema_version\": 1,";
    let injection =
        format!("{injection_anchor}\n  \"future_v0_2_field\": {{\"nested\": [1, 2, 3]}},");
    let injected_text = text.replacen(injection_anchor, &injection, 1);
    assert_ne!(
        injected_text, text,
        "injection-anchor sanity check: the replace must change the text"
    );

    // Deserializing the injected JSON via the v0.1 struct must
    // succeed — serde's default policy ignores unknown fields, and
    // the envelope does NOT opt into `deny_unknown_fields`.
    let parsed: CompatEnvelope = serde_json::from_str(&injected_text).expect(
        "v0.1 struct must tolerate unknown future-v0.2 fields (no deny_unknown_fields opt-in)",
    );
    assert_eq!(
        parsed.schema_version, 1,
        "known fields must still parse around the injected unknown field"
    );
    assert_eq!(parsed.mode, "compat");
}

/// Test 11 — every field the §5 CI gate reads is present in the
/// serialized output. A rename or removal of any of these fields
/// silently breaks the pilot gate; this test bites before that ships.
///
/// §5 gate-read fields (per `docs/compatibility-plan.md`):
///   - `errors`
///   - `results.mismatch_count`
///   - `results.baseline.{pass, fail}`
///   - `results.lihaaf.{pass, fail}`
///   - `results.baseline.exit_code`
///   - `results.lihaaf.exit_code`
///   - `excluded_fixtures`
#[test]
fn gate_fields_present() {
    let mut env = empty_envelope();
    let (_tmp, _path, bytes) = write_to_tmp(&mut env);
    let text = std::str::from_utf8(&bytes).expect("UTF-8");

    for field in [
        "\"errors\":",
        "\"mismatch_count\":",
        "\"pass\":",
        "\"fail\":",
        "\"exit_code\":",
        "\"excluded_fixtures\":",
    ] {
        assert!(
            text.contains(field),
            "§5 gate field {field} must be present in the serialized envelope; got:\n{text}"
        );
    }

    // Structural sanity: `pass` / `fail` / `exit_code` must each
    // appear TWICE (baseline + lihaaf). A regression that collapses
    // the two sides into a single field would not be caught by the
    // substring check above.
    let count = |needle: &str| text.matches(needle).count();
    assert_eq!(
        count("\"pass\":"),
        2,
        "`pass` must appear in both baseline and lihaaf counts"
    );
    assert_eq!(
        count("\"fail\":"),
        2,
        "`fail` must appear in both baseline and lihaaf counts"
    );
    assert_eq!(
        count("\"exit_code\":"),
        2,
        "`exit_code` must appear in both baseline and lihaaf counts"
    );
}

/// Test 12 — empirical assertion of the locked decision §2 (in
/// `src/compat/report.rs` module header): `serde_json` serializes
/// struct fields in declaration order, regardless of the
/// `preserve_order` feature flag (which controls `Map<String, Value>`,
/// not struct fields). The spec's Risk callout says "asserted
/// empirically to bite if it ever changes" — this is that bite.
///
/// We assert the first 256 chars of the serialized envelope contain
/// `schema_version` before `mode`, and `mode` before `crate_name`,
/// matching the declared field order on `CompatEnvelope`.
#[test]
fn field_declaration_order_matches_on_disk_layout() {
    let mut env = empty_envelope();
    let (_tmp, _path, bytes) = write_to_tmp(&mut env);
    let text = std::str::from_utf8(&bytes).expect("UTF-8");

    let prefix = &text[..text.len().min(256)];
    let schema_pos = prefix
        .find("\"schema_version\":")
        .expect("schema_version must appear in the first 256 chars");
    let mode_pos = prefix
        .find("\"mode\":")
        .expect("mode must appear in the first 256 chars");
    let crate_pos = prefix
        .find("\"crate_name\":")
        .expect("crate_name must appear in the first 256 chars");
    assert!(
        schema_pos < mode_pos,
        "schema_version must precede mode in the on-disk layout; got prefix:\n{prefix}"
    );
    assert!(
        mode_pos < crate_pos,
        "mode must precede crate_name in the on-disk layout; got prefix:\n{prefix}"
    );

    // Top-level field order across the full document: every field
    // must appear in `CompatEnvelope` declaration order. We anchor on
    // the top-level indent (`\n  "field":`) so nested duplicates (e.g.
    // `results.lihaaf.toolchain`) do not collide with the top-level
    // `toolchain` field. Pretty-printed JSON indents top-level keys
    // by exactly two spaces.
    let expected = [
        "\n  \"schema_version\":",
        "\n  \"mode\":",
        "\n  \"crate_name\":",
        "\n  \"commit\":",
        "\n  \"commands\":",
        "\n  \"results\":",
        "\n  \"mismatch_examples\":",
        "\n  \"errors\":",
        "\n  \"excluded_fixtures\":",
        "\n  \"generated_paths\":",
        "\n  \"overlay\":",
        "\n  \"toolchain\":",
    ];
    let mut last_pos = 0usize;
    for field in expected {
        let pos = text
            .find(field)
            .unwrap_or_else(|| panic!("field {field} must appear in serialized envelope"));
        assert!(
            pos >= last_pos,
            "field {field} appears before a prior field in the declared order"
        );
        last_pos = pos;
    }
}

/// Bridge between cleanup-side and envelope-side `GeneratedPath`
/// types. The cleanup module produces absolute [`PathBuf`] entries
/// with a typed classification enum; the §3.3 envelope needs
/// repo-relative forward-slash strings with a stringly-typed class
/// field for v0.2-additive evolution. Phase 9 wires the conversion
/// into `compat::run`; this test pins the conversion contract until
/// then.
///
/// The four enum variants are checked explicitly so a renamed variant
/// would fail compilation here rather than silently producing
/// `"committed"` for a path that should be `"kept"`, etc.
#[test]
fn generated_path_from_cleanup_round_trip() {
    let compat_root = PathBuf::from("/tmp/compat-root");
    let cases = [
        (CleanupGeneratedPathClass::Committed, "committed"),
        (CleanupGeneratedPathClass::Ignored, "ignored"),
        (CleanupGeneratedPathClass::Cleaned, "cleaned"),
        (CleanupGeneratedPathClass::Kept, "kept"),
    ];
    for (cleanup_class, expected_label) in cases {
        let cleanup_entry = CleanupGeneratedPath {
            path: compat_root
                .join("target")
                .join("lihaaf-overlay")
                .join("Cargo.toml"),
            class: cleanup_class,
        };
        let envelope_entry = from_cleanup(&cleanup_entry, &compat_root);
        assert_eq!(envelope_entry.path, "target/lihaaf-overlay/Cargo.toml");
        assert_eq!(envelope_entry.class, expected_label);
    }

    // A nested path under `compat_root` is rendered with forward-slash
    // separators (verified on the produced string — even on Windows,
    // `relative_to` converts).
    let nested = compat_root.join("target").join("lihaaf-compat-converted");
    let cleanup_entry = CleanupGeneratedPath {
        path: nested,
        class: CleanupGeneratedPathClass::Cleaned,
    };
    let envelope_entry = from_cleanup(&cleanup_entry, &compat_root);
    assert!(
        !envelope_entry.path.contains('\\'),
        "envelope path must use forward-slash; got: {}",
        envelope_entry.path
    );
    assert_eq!(envelope_entry.path, "target/lihaaf-compat-converted");
}

/// Test 14 — `errors[].detail` has absolute `compat_root` paths stripped
/// before envelope serialization. Mirrors the production failure in
/// Actions run 25994537438, where `errors[0].detail` contained
/// `/home/runner/work/lihaaf/lihaaf/./Cargo.lihaaf.toml` from the cargo
/// invocation embedded in `DylibBuildFailed::Display`.
///
/// This test is the regression gate for FIX class V. It would FAIL
/// without the `normalize_error_detail_paths` call in `compat::run`
/// (or, equivalently, without calling `normalize_error_detail_paths`
/// before `write_envelope` here in the test).
///
/// Verification that the test actually bites: remove the
/// `normalize_error_detail_paths` call below and confirm the
/// `assert!(!text.contains(abs_root_str))` fires.
#[test]
fn error_detail_paths_stripped_before_write() {
    let abs_root = PathBuf::from("/home/runner/work/my-crate/my-crate");
    let abs_root_str = abs_root.to_string_lossy();

    // Simulate the `DylibBuildFailed` display string: cargo uses `{:?}`
    // for PathBuf, which wraps the path in double-quotes. Two absolute
    // paths appear in one detail string — exactly the shape from the
    // production failing run.
    let raw_detail = format!(
        "lihaaf: dylib build failed.\n  invocation: RUSTFLAGS=\"-C prefer-dynamic\" \
         cargo rustc -p anyhow --lib --release --crate-type=dylib \
         --message-format=json-render-diagnostics \
         --manifest-path \"{root}/target/lihaaf-overlay/Cargo.toml\" \
         --target-dir \"{root}/target/lihaaf-build\"\n  cargo stderr:\nerror[E0]: ...",
        root = abs_root_str
    );

    // Confirm the raw detail DOES contain the absolute root (test-setup
    // invariant: the assertion below is not vacuously true).
    assert!(
        raw_detail.contains(abs_root_str.as_ref()),
        "test setup: raw_detail must contain the absolute root before normalization"
    );

    let mut env = empty_envelope();
    env.errors = vec![EnvelopeError {
        error_type: "lihaaf_session_failed".into(),
        fixture: None,
        file: String::new(),
        line: 0,
        detail: raw_detail,
    }];

    // Apply normalization at the envelope boundary — this is what
    // `compat::run` does before calling `write_envelope`.
    normalize_error_detail_paths(&mut env, &abs_root);

    let (_tmp, _path, bytes) = write_to_tmp(&mut env);
    let text = std::str::from_utf8(&bytes).expect("envelope must be valid UTF-8");

    // The absolute root MUST NOT appear anywhere in the serialized
    // envelope — not in `detail`, not in any other field.
    assert!(
        !text.contains(abs_root_str.as_ref()),
        "errors[].detail must not contain the absolute compat_root after normalization; \
         got envelope:\n{text}"
    );

    // The repo-relative sub-paths MUST still appear — normalization
    // strips the prefix but not the rest of the path.
    assert!(
        text.contains("target/lihaaf-overlay/Cargo.toml"),
        "repo-relative manifest path must survive normalization; got:\n{text}"
    );
    assert!(
        text.contains("target/lihaaf-build"),
        "repo-relative target-dir path must survive normalization; got:\n{text}"
    );
}

/// Test 15 — cross-root byte determinism for `errors[].detail`. Two
/// envelopes built with the same logical error content but different
/// absolute `compat_root` values must produce byte-identical serialized
/// output after `normalize_error_detail_paths` and `dur_ms` stripping.
///
/// This is the §3.3 determinism property for error entries: different
/// CI runners (at different checkout roots) that both encounter the
/// same `DylibBuildFailed` error must produce identical envelope bytes.
///
/// The test would fail without the normalization step — the two
/// `abs_root_*` strings differ, so two raw detail strings differ, so
/// two unstripped envelopes differ outside `dur_ms` lines.
#[test]
fn error_detail_paths_cross_root_byte_determinism() {
    // Two hypothetical checkout roots — one for a GitHub Actions runner,
    // one for a local developer machine. Same relative layout, different
    // absolute prefix.
    let abs_root_a = PathBuf::from("/home/runner/work/my-crate/my-crate");
    let abs_root_b = PathBuf::from("/home/tarunvir/projects/my-crate");

    let make_detail = |root: &str| {
        format!(
            "lihaaf: dylib build failed.\n  invocation: RUSTFLAGS=\"-C prefer-dynamic\" \
             cargo rustc -p anyhow --lib --release --crate-type=dylib \
             --message-format=json-render-diagnostics \
             --manifest-path \"{root}/target/lihaaf-overlay/Cargo.toml\" \
             --target-dir \"{root}/target/lihaaf-build\"\n  cargo stderr:\nerror[E0]: ...",
        )
    };

    let mut env_a = empty_envelope();
    env_a.errors = vec![EnvelopeError {
        error_type: "lihaaf_session_failed".into(),
        fixture: None,
        file: String::new(),
        line: 0,
        detail: make_detail(&abs_root_a.to_string_lossy()),
    }];
    normalize_error_detail_paths(&mut env_a, &abs_root_a);

    let mut env_b = empty_envelope();
    env_b.errors = vec![EnvelopeError {
        error_type: "lihaaf_session_failed".into(),
        fixture: None,
        file: String::new(),
        line: 0,
        detail: make_detail(&abs_root_b.to_string_lossy()),
    }];
    normalize_error_detail_paths(&mut env_b, &abs_root_b);

    let (_tmp_a, _path_a, bytes_a) = write_to_tmp(&mut env_a);
    let (_tmp_b, _path_b, bytes_b) = write_to_tmp(&mut env_b);

    let text_a = std::str::from_utf8(&bytes_a).expect("envelope A must be valid UTF-8");
    let text_b = std::str::from_utf8(&bytes_b).expect("envelope B must be valid UTF-8");

    // Before strip: the two envelopes must be byte-equal even WITHOUT
    // dur_ms stripping, because neither root appears in the detail after
    // normalization and both `dur_ms` values are 0 (empty_envelope default).
    // We still use the strip helper for robustness against future timing
    // fields, but the pre-strip equality is the stronger property to assert.
    assert_eq!(
        text_a, text_b,
        "two runners with different compat_root values must produce byte-identical \
         envelopes after normalize_error_detail_paths; \
         runner A:\n{text_a}\nrunner B:\n{text_b}"
    );

    // Belt-and-braces via strip: the stripped form is also equal
    // (covers future cases where dur_ms is non-zero).
    let stripped_a = strip_dur_ms_lines(text_a);
    let stripped_b = strip_dur_ms_lines(text_b);
    assert_eq!(
        stripped_a, stripped_b,
        "stripped envelopes must be byte-equal; runner A:\n{stripped_a}\nrunner B:\n{stripped_b}"
    );

    // Neither absolute root must appear anywhere in the output.
    assert!(
        !text_a.contains("/home/runner/work/my-crate"),
        "runner-A root must not appear in serialized envelope; got:\n{text_a}"
    );
    assert!(
        !text_b.contains("/home/tarunvir/projects/my-crate"),
        "runner-B root must not appear in serialized envelope; got:\n{text_b}"
    );
}
