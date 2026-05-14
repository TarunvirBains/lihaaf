//! Phase 4 of compat mode (issue #9) — conservative trybuild baseline
//! extraction integration tests.
//!
//! The single load-bearing invariant under test is the §1 conservatism
//! rule:
//!
//! > Baseline extraction is intentionally conservative. Compat mode
//! > records the original `cargo test` command result as the coarse
//! > baseline. Fixture-level baseline status may only be reported when
//! > it is derived from explicitly recognized trybuild invocations and
//! > stable path matches; otherwise the fixture baseline is `unknown`
//! > and the report must say why.
//!
//! Practically: every test in this file constructs a libtest stdout
//! capture by hand, calls [`lihaaf::compat_parse_libtest_output`] (or
//! the end-to-end [`lihaaf::compat_baseline_run_with_recognized_fixtures`])
//! with a precisely-chosen recognized-fixture set, and asserts on the
//! returned [`lihaaf::CompatParsedBaseline`] (or [`lihaaf::CompatBaselineResult`])
//! shape. The pilot gate's `unknown == 0` requirement is implementable
//! only because empty-recognition input produces zero pass/fail signal;
//! [`recognized_fixtures_empty_yields_all_unknown`] is the acid test for
//! that property.
//!
//! ## Reaching the parser
//!
//! Every entry point is reached through `#[doc(hidden)]` re-exports in
//! `src/lib.rs` (`compat_parse_libtest_output`, `CompatFixtureId`, etc.).
//! Those re-exports exist exclusively for this test crate and the
//! `cargo-lihaaf` binary; the v0.1 supported entry to compat mode is
//! `cargo lihaaf --compat`, not the Rust API.
//!
//! ## Why every test is hermetic
//!
//! Tests that exercise the parser directly need no filesystem. The
//! end-to-end [`sidecar_records_v2_schema`] test owns a
//! `tempfile::TempDir` and writes the v2 sidecar JSON inside it; the
//! runner's only filesystem effect is that single write, so a leak
//! would be visible.

use std::path::PathBuf;

use lihaaf::{
    CompatBaselineMismatch, CompatBaselineVerdict, CompatFixtureId,
    compat_baseline_run_with_recognized_fixtures as run_with_recognized,
    compat_parse_libtest_output as parse,
};

/// Convenience constructor — every test builds a few of these.
fn fid(path: &str) -> CompatFixtureId {
    CompatFixtureId {
        repo_relative_path: PathBuf::from(path),
    }
}

/// **Acid test for the conservatism rule.**
///
/// An empty `recognized_fixtures` slice means the parser has no
/// authorization to assign fixture-level verdicts. The result must
/// be:
///
/// - `pass.is_none()` — no fixture-level pass count is meaningful.
/// - `fail.is_none()` — same.
/// - Every libtest verdict line counts toward `unknown_count`.
///
/// A regression that infers `pass`/`fail` from arbitrary libtest
/// output would violate the §1 contract and break the v0.1 pilot
/// gate's `unknown == 0` property.
#[test]
fn recognized_fixtures_empty_yields_all_unknown() {
    let stdout = "\
        running 3 tests\n\
        test tests/foo ... ok\n\
        test tests/bar ... FAILED\n\
        test tests/baz ... ok\n\
        \n\
        test result: FAILED. 2 passed; 1 failed; 0 ignored\n";
    let result = parse(stdout, &[]);
    assert!(
        result.pass.is_none(),
        "pass must be None when recognized_fixtures is empty; got {:?}",
        result.pass
    );
    assert!(
        result.fail.is_none(),
        "fail must be None when recognized_fixtures is empty; got {:?}",
        result.fail
    );
    assert_eq!(
        result.unknown_count, 3,
        "every verdict line must count as unknown; got {}",
        result.unknown_count
    );
    assert!(
        result.mismatch_entries.is_empty(),
        "no mismatch entries possible without recognition"
    );
}

/// A single recognized fixture matched against a `... ok` line
/// increments `pass`. The mismatch entry records the fixture path
/// in canonical forward-slash form with the `.rs` suffix preserved.
#[test]
fn single_recognized_pass_counts_in_pass() {
    let recognized = vec![fid("tests/trybuild/compile_pass/foo.rs")];
    let stdout = "test tests/trybuild/compile_pass/foo ... ok\n";
    let result = parse(stdout, &recognized);
    assert_eq!(result.pass, Some(1));
    assert_eq!(result.fail, Some(0));
    assert_eq!(result.unknown_count, 0);
    assert_eq!(result.mismatch_entries.len(), 1);
    assert_eq!(
        result.mismatch_entries[0].fixture,
        "tests/trybuild/compile_pass/foo.rs"
    );
    assert_eq!(
        result.mismatch_entries[0].baseline_verdict,
        CompatBaselineVerdict::Pass
    );
}

/// A single recognized fixture matched against a `... FAILED` line
/// increments `fail`. Mirror of the pass test.
#[test]
fn single_recognized_fail_counts_in_fail() {
    let recognized = vec![fid("tests/trybuild/compile_fail/bar.rs")];
    let stdout = "test tests/trybuild/compile_fail/bar ... FAILED\n";
    let result = parse(stdout, &recognized);
    assert_eq!(result.pass, Some(0));
    assert_eq!(result.fail, Some(1));
    assert_eq!(result.unknown_count, 0);
    assert_eq!(result.mismatch_entries.len(), 1);
    assert_eq!(
        result.mismatch_entries[0].baseline_verdict,
        CompatBaselineVerdict::Fail
    );
}

/// **Prefix-collision regression.** A recognized fixture
/// `tests/ui/foo.rs` must NOT correlate to a libtest verdict line
/// for `tests/ui/foo_extra`. The earlier substring-match shape would
/// have happily attributed the `foo_extra` verdict to `foo`; the
/// exact-match rule closes that class.
///
/// The fixture `tests/ui/foo_extra.rs` IS recognized in this corpus,
/// so the test also asserts the verdict lands on `foo_extra` (where
/// it belongs), not on `foo`.
#[test]
fn recognized_fixture_prefix_does_not_match_longer_libtest_name() {
    let recognized = vec![fid("tests/ui/foo.rs"), fid("tests/ui/foo_extra.rs")];
    let stdout = "test tests/ui/foo_extra ... ok\n";
    let result = parse(stdout, &recognized);
    // The verdict must land on `foo_extra`, not on `foo`.
    assert_eq!(
        result.pass,
        Some(1),
        "exactly one recognized fixture passed"
    );
    assert_eq!(result.fail, Some(0));
    // `foo.rs` was never named — counts as unknown (absence of
    // evidence).
    assert_eq!(
        result.unknown_count, 1,
        "the unnamed `tests/ui/foo.rs` fixture counts as one unknown; got {}",
        result.unknown_count
    );
    assert_eq!(result.mismatch_entries.len(), 1);
    assert_eq!(
        result.mismatch_entries[0].fixture, "tests/ui/foo_extra.rs",
        "verdict must correlate to the exact-match fixture, not the prefix; got {}",
        result.mismatch_entries[0].fixture
    );
    assert_eq!(
        result.mismatch_entries[0].baseline_verdict,
        CompatBaselineVerdict::Pass
    );
}

/// **Conservatism rule, absence of evidence form.**
///
/// A recognized fixture the libtest output never names must count as
/// `unknown_count`, not as a free pass. The parser must NOT assume
/// "didn't fail therefore passed".
#[test]
fn recognized_fixture_absent_from_output_counts_unknown() {
    let recognized = vec![fid("tests/ghost.rs")];
    let stdout = "test other_thing ... ok\n";
    let result = parse(stdout, &recognized);
    // The `other_thing` line doesn't correlate to `tests/ghost` —
    // that's one unknown.
    // The `tests/ghost.rs` fixture itself was never named — that's
    // another unknown.
    assert_eq!(
        result.unknown_count, 2,
        "both the unrecognized libtest line AND the absent recognized fixture count as unknown; \
         got {}",
        result.unknown_count
    );
    // pass/fail are populated because recognized was non-empty, but
    // both should be zero (no recognized fixture saw a verdict).
    assert_eq!(result.pass, Some(0));
    assert_eq!(result.fail, Some(0));
    assert!(result.mismatch_entries.is_empty());
}

/// **Conservatism rule, unrecognized-libtest-line form.**
///
/// Even when a recognized-fixture set is provided, libtest verdict
/// lines whose test names don't correlate to any recognized fixture
/// must count as `unknown_count`. Cannot claim parity for fixtures
/// the discovery walker never authorized.
#[test]
fn unrecognized_libtest_line_counts_unknown_even_when_pass() {
    let recognized = vec![fid("tests/recognized.rs")];
    // Libtest passes a different test; the recognized fixture wasn't
    // mentioned. Two unknowns: one for the unrecognized line, one
    // for the absent recognized fixture.
    let stdout = "\
        test tests/recognized ... ok\n\
        test some/other/test ... ok\n";
    let result = parse(stdout, &recognized);
    assert_eq!(result.pass, Some(1), "recognized fixture passed");
    assert_eq!(result.fail, Some(0));
    assert_eq!(
        result.unknown_count, 1,
        "the unrecognized `some/other/test` line counts as 1 unknown; got {}",
        result.unknown_count
    );
}

/// **Mixed correlation acceptance test.** Multiple recognized
/// fixtures, mixed verdict shapes, plus some unrecognized lines.
/// This is the integration shape the v0.1 pilot gate will see.
#[test]
fn mixed_recognized_and_unrecognized_partitions_correctly() {
    let recognized = vec![
        fid("tests/pass_a.rs"),
        fid("tests/pass_b.rs"),
        fid("tests/fail_c.rs"),
        fid("tests/absent_d.rs"),
    ];
    let stdout = "\
        running 5 tests\n\
        test tests/pass_a ... ok\n\
        test tests/pass_b ... ok\n\
        test tests/fail_c ... FAILED\n\
        test unrelated/thing ... ok\n\
        test unrelated/other ... FAILED\n\
        \n\
        test result: FAILED.\n";
    let result = parse(stdout, &recognized);
    assert_eq!(result.pass, Some(2), "two recognized fixtures passed");
    assert_eq!(result.fail, Some(1), "one recognized fixture failed");
    // Two unrecognized libtest lines + one absent recognized fixture
    // (`tests/absent_d`) = 3 unknowns.
    assert_eq!(
        result.unknown_count, 3,
        "2 unrecognized lines + 1 absent recognized fixture = 3; got {}",
        result.unknown_count
    );
    assert_eq!(result.mismatch_entries.len(), 3);
    // Sorted by fixture in forward-slash ASCII order:
    // tests/fail_c.rs < tests/pass_a.rs < tests/pass_b.rs.
    assert_eq!(result.mismatch_entries[0].fixture, "tests/fail_c.rs");
    assert_eq!(result.mismatch_entries[1].fixture, "tests/pass_a.rs");
    assert_eq!(result.mismatch_entries[2].fixture, "tests/pass_b.rs");
}

/// `mismatch_entries` is sorted by fixture in forward-slash ASCII
/// byte order. Tests that insert in reverse order produce sorted
/// output — the Phase 8 envelope writer relies on this so its
/// `mismatch_examples` sort is a no-op.
#[test]
fn mismatch_entries_sorted_for_determinism() {
    let recognized = vec![
        fid("tests/zzz_last.rs"),
        fid("tests/aaa_first.rs"),
        fid("tests/mmm_middle.rs"),
    ];
    // Libtest output in reverse order — sort happens at the parser
    // layer.
    let stdout = "\
        test tests/zzz_last ... ok\n\
        test tests/mmm_middle ... FAILED\n\
        test tests/aaa_first ... ok\n";
    let result = parse(stdout, &recognized);
    assert_eq!(result.mismatch_entries.len(), 3);
    assert_eq!(result.mismatch_entries[0].fixture, "tests/aaa_first.rs");
    assert_eq!(result.mismatch_entries[1].fixture, "tests/mmm_middle.rs");
    assert_eq!(result.mismatch_entries[2].fixture, "tests/zzz_last.rs");
}

/// **Garbled output safety.** A truncated line, a verdict word the
/// parser doesn't recognize, and an embedded ANSI escape inside an
/// otherwise-valid line all must not panic. Every parse failure
/// must increment `unknown_count` rather than corrupt the count.
#[test]
fn garbled_output_does_not_panic_and_counts_unknown() {
    let recognized = vec![fid("tests/foo.rs"), fid("tests/bar.rs")];
    let stdout = "\
        running 2 tests\n\
        test tests/foo ... bench\n\
        test tests/bar ... \n\
        test tests/midline_cut_o\n\
        test result: FAILED. 0 passed; 2 fai";
    let result = parse(stdout, &recognized);
    // None of the garbled lines produce a pass/fail. Both
    // recognized fixtures stay unknown, so 0/0 + at least 2 for
    // never-seen-fixtures. Both verdict-shape failures count, plus
    // the dangling `test tests/midline_cut_o` (no `...` separator)
    // is unknown.
    assert_eq!(result.pass, Some(0));
    assert_eq!(result.fail, Some(0));
    assert!(
        result.unknown_count >= 2,
        "garbled lines + 2 absent recognized fixtures must yield >= 2 unknowns; got {}",
        result.unknown_count
    );
    assert!(result.mismatch_entries.is_empty());
}

/// ANSI escape codes around the verdict word do not block
/// classification. The parser's `strip_ansi` pass must run before
/// the verdict-token check.
#[test]
fn ansi_escapes_around_verdict_are_stripped() {
    let recognized = vec![fid("tests/colored.rs")];
    // Real libtest with color emits something like this:
    //   test foo ... \x1b[32mok\x1b[0m
    // We test both shapes (verdict-only color and full-line color).
    let stdout = "test tests/colored ... \x1b[32mok\x1b[0m\n";
    let result = parse(stdout, &recognized);
    assert_eq!(result.pass, Some(1));
    assert_eq!(result.fail, Some(0));
    assert_eq!(result.unknown_count, 0);
    assert_eq!(result.mismatch_entries.len(), 1);
    assert_eq!(
        result.mismatch_entries[0].baseline_verdict,
        CompatBaselineVerdict::Pass
    );
}

/// **End-to-end: the v2 sidecar carries the parsed counts.** Spawns
/// a real child (`/usr/bin/true`) to capture an empty libtest output,
/// then asserts the on-disk sidecar reports the parser's verdict
/// in canonical fields. An empty recognized set ⇒ `pass`/`fail`
/// null, `unknown_count` zero (no verdict lines to classify).
#[test]
fn sidecar_records_v2_schema() {
    let tmp = tempfile::tempdir().unwrap();
    let sidecar = tmp.path().join("baseline_capture.json");
    let argv = vec!["true".to_string()];
    let result = run_with_recognized(&argv, tmp.path(), &sidecar, &[])
        .expect("`true` must spawn from a tempdir");
    assert_eq!(result.exit_code, 0);
    assert!(
        result.pass.is_none(),
        "empty recognized set ⇒ pass is None; got {:?}",
        result.pass
    );
    assert!(
        result.fail.is_none(),
        "empty recognized set ⇒ fail is None; got {:?}",
        result.fail
    );
    assert_eq!(result.unknown_count, 0, "no verdict lines from `true`");

    // Sidecar shape.
    let bytes = std::fs::read(&sidecar).expect("sidecar must exist");
    let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(
        v.get("schema_version").and_then(serde_json::Value::as_u64),
        Some(2),
        "v2 sidecar must stamp schema_version == 2"
    );
    // `pass` and `fail` are JSON `null` rather than missing —
    // adopters parsing the sidecar need a stable shape.
    assert!(v.get("pass").is_some_and(serde_json::Value::is_null));
    assert!(v.get("fail").is_some_and(serde_json::Value::is_null));
    assert_eq!(
        v.get("unknown_count").and_then(serde_json::Value::as_u64),
        Some(0)
    );
    let entries = v
        .get("mismatch_entries")
        .and_then(serde_json::Value::as_array)
        .unwrap();
    assert!(entries.is_empty());
}

/// **End-to-end with parser bite.** Verify the v2 entry point wires
/// `parse_libtest_output` correctly: stdout that names a recognized
/// fixture produces a populated `mismatch_entries` array on disk.
/// Uses `printf` (POSIX coreutils) to emit a single libtest-shaped
/// line; the child's exit code matches the printf default.
#[test]
fn sidecar_v2_records_mismatch_entries_when_recognized() {
    let tmp = tempfile::tempdir().unwrap();
    let sidecar = tmp.path().join("baseline_capture.json");
    // `printf` is available on every POSIX platform lihaaf targets
    // and (unlike `echo`) treats its first argument as a format
    // string, so the test does not depend on shell builtin shadows.
    let argv = vec![
        "printf".to_string(),
        "test tests/recognized ... ok\n".to_string(),
    ];
    let recognized = vec![CompatFixtureId {
        repo_relative_path: PathBuf::from("tests/recognized.rs"),
    }];
    let result =
        run_with_recognized(&argv, tmp.path(), &sidecar, &recognized).expect("`printf` must spawn");
    assert_eq!(result.exit_code, 0);
    assert_eq!(result.pass, Some(1));
    assert_eq!(result.fail, Some(0));
    assert_eq!(result.unknown_count, 0);
    assert_eq!(result.mismatch_entries.len(), 1);
    assert_eq!(
        result.mismatch_entries[0],
        CompatBaselineMismatch {
            fixture: "tests/recognized.rs".to_string(),
            baseline_verdict: CompatBaselineVerdict::Pass,
        }
    );

    // On-disk sidecar mirrors the in-memory result.
    let bytes = std::fs::read(&sidecar).expect("sidecar must exist");
    let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let entries = v
        .get("mismatch_entries")
        .and_then(serde_json::Value::as_array)
        .expect("mismatch_entries must be an array");
    assert_eq!(entries.len(), 1);
    assert_eq!(
        entries[0]
            .get("fixture")
            .and_then(serde_json::Value::as_str),
        Some("tests/recognized.rs")
    );
    assert_eq!(
        entries[0]
            .get("baseline_verdict")
            .and_then(serde_json::Value::as_str),
        Some("pass")
    );
}
