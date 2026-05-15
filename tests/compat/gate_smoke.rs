//! Integration smoke test for the §5 pilot-gate logic (Phase 10).
//!
//! Exercises [`lihaaf::compat_check_gate`] / [`lihaaf::compat_parse_baseline`] /
//! [`lihaaf::compat_load_baseline`] through the `#[doc(hidden)]`
//! re-exports at the crate root. The integration boundary matters: the
//! supported entry to compat mode is `cargo lihaaf --compat`, but the
//! §5 gate is also reachable from out-of-tree CI runners that depend on
//! the re-exports staying linkable. This binary asserts the re-export
//! surface compiles and the typed primitives behave per spec.
//!
//! Each test synthesizes a [`lihaaf::CompatEnvelope`] in memory and
//! calls [`lihaaf::compat_check_gate`] against a constructed ceiling
//! map; no filesystem state is involved unless the test explicitly
//! exercises [`lihaaf::compat_load_baseline`].
//!
//! Spec references:
//! - `docs/compatibility-plan.md:239-244` — the four field groups the
//!   gate reads.
//! - `docs/superpowers/plans/2026-05-13-compat-mode-implementation-plan.md`
//!   Phase 10 — locked decisions about baseline.toml location, gate
//!   placement, and the dry-run shape.

use std::collections::BTreeMap;
use std::path::Path;

use lihaaf::{
    CompatBaselineCounts, CompatCommands, CompatEnvelope, CompatEnvelopeError,
    CompatExcludedFixture, CompatGateCeiling, CompatGateOutcome, CompatLihaafCounts,
    CompatOverlayMetadata, CompatResults, compat_check_gate, compat_load_baseline,
    compat_parse_baseline,
};

/// Build a §3.3 envelope with the named `crate_name` and otherwise
/// neutral defaults (all counts zero, no errors, no excluded fixtures).
/// Tests mutate the relevant fields after construction.
fn neutral_envelope(crate_name: &str) -> CompatEnvelope {
    CompatEnvelope {
        schema_version: 1,
        mode: "compat".into(),
        crate_name: crate_name.into(),
        commit: String::new(),
        commands: CompatCommands {
            baseline: "cargo test".into(),
            lihaaf: "cargo lihaaf --compat --compat-root .".into(),
        },
        results: CompatResults {
            baseline: CompatBaselineCounts {
                pass: 0,
                fail: 0,
                unknown_count: 0,
                exit_code: 0,
                dur_ms: 0,
            },
            lihaaf: CompatLihaafCounts {
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
        overlay: CompatOverlayMetadata {
            generated: true,
            dropped_comments: Vec::new(),
            upstream_already_has_dylib: false,
        },
        toolchain: "rustc 1.95.0".into(),
    }
}

fn one_crate_baseline(name: &str, n_max: u32) -> BTreeMap<String, CompatGateCeiling> {
    let mut m = BTreeMap::new();
    m.insert(name.to_string(), CompatGateCeiling { n_max });
    m
}

#[test]
fn parses_empty_baseline_toml() {
    let map = compat_parse_baseline(b"", Path::new("baseline.toml"))
        .expect("empty baseline.toml must parse");
    assert!(
        map.is_empty(),
        "an empty baseline.toml must produce zero ceilings; got {} entries",
        map.len(),
    );
}

#[test]
fn parses_baseline_toml_with_one_crate() {
    let toml = b"[example-crate]\nn_max = 7\n";
    let map =
        compat_parse_baseline(toml, Path::new("baseline.toml")).expect("baseline.toml must parse");
    assert_eq!(map.len(), 1);
    // The crate name is the BTreeMap key; the entry only carries `n_max`.
    let ceiling = &map["example-crate"];
    assert_eq!(ceiling.n_max, 7);
}

#[test]
fn load_baseline_reads_from_disk() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let path = tmp.path().join("baseline.toml");
    std::fs::write(&path, b"[some-crate]\nn_max = 3\n").expect("write baseline.toml");
    let map = compat_load_baseline(&path).expect("load_baseline must succeed");
    assert_eq!(map.len(), 1);
    assert_eq!(map["some-crate"].n_max, 3);
}

#[test]
fn unenrolled_crate_is_noop() {
    let baseline = BTreeMap::new();
    let env = neutral_envelope("not-enrolled");
    assert_eq!(
        compat_check_gate(&baseline, &env),
        CompatGateOutcome::NotEnrolled,
    );
}

#[test]
fn under_ceiling_allows() {
    let baseline = one_crate_baseline("demo", 5);
    let mut env = neutral_envelope("demo");
    env.results.mismatch_count = 3;
    assert_eq!(compat_check_gate(&baseline, &env), CompatGateOutcome::Allow,);
}

#[test]
fn at_ceiling_allows() {
    let baseline = one_crate_baseline("demo", 5);
    let mut env = neutral_envelope("demo");
    env.results.mismatch_count = 5;
    assert_eq!(compat_check_gate(&baseline, &env), CompatGateOutcome::Allow,);
}

#[test]
fn over_ceiling_blocks() {
    let baseline = one_crate_baseline("demo", 5);
    let mut env = neutral_envelope("demo");
    env.results.mismatch_count = 6;
    match compat_check_gate(&baseline, &env) {
        CompatGateOutcome::Block(msg) => {
            assert!(msg.contains("mismatch_count"), "diagnostic: {msg}");
            assert!(msg.contains("n_max"), "diagnostic: {msg}");
            assert!(msg.contains("6"), "diagnostic: {msg}");
            assert!(msg.contains("5"), "diagnostic: {msg}");
        }
        other => panic!("expected Block, got {other:?}"),
    }
}

#[test]
fn envelope_errors_block() {
    let baseline = one_crate_baseline("demo", 5);
    let mut env = neutral_envelope("demo");
    env.errors.push(CompatEnvelopeError {
        error_type: "discovery_unrecognized".into(),
        fixture: None,
        file: "tests/trybuild.rs".into(),
        line: 7,
        detail: "non-literal arg".into(),
    });
    match compat_check_gate(&baseline, &env) {
        CompatGateOutcome::Block(msg) => {
            assert!(msg.contains("envelope.errors"), "diagnostic: {msg}");
            assert!(
                msg.contains("discovery_unrecognized"),
                "diagnostic should name the first error_type: {msg}",
            );
        }
        other => panic!("expected Block, got {other:?}"),
    }
}

#[test]
fn baseline_exit_code_nonzero_blocks() {
    let baseline = one_crate_baseline("demo", 5);
    let mut env = neutral_envelope("demo");
    env.results.baseline.exit_code = 101;
    match compat_check_gate(&baseline, &env) {
        CompatGateOutcome::Block(msg) => {
            assert!(msg.contains("baseline.exit_code"), "diagnostic: {msg}");
            assert!(msg.contains("101"), "diagnostic: {msg}");
        }
        other => panic!("expected Block, got {other:?}"),
    }
}

#[test]
fn lihaaf_exit_code_nonzero_blocks() {
    let baseline = one_crate_baseline("demo", 5);
    let mut env = neutral_envelope("demo");
    env.results.lihaaf.exit_code = 1;
    match compat_check_gate(&baseline, &env) {
        CompatGateOutcome::Block(msg) => {
            assert!(msg.contains("lihaaf.exit_code"), "diagnostic: {msg}");
        }
        other => panic!("expected Block, got {other:?}"),
    }
}

#[test]
fn baseline_unknown_count_blocks() {
    let baseline = one_crate_baseline("demo", 5);
    let mut env = neutral_envelope("demo");
    env.results.baseline.unknown_count = 2;
    match compat_check_gate(&baseline, &env) {
        CompatGateOutcome::Block(msg) => {
            assert!(msg.contains("unknown_count"), "diagnostic: {msg}");
        }
        other => panic!("expected Block, got {other:?}"),
    }
}

#[test]
fn totals_divergence_without_excluded_blocks() {
    let baseline = one_crate_baseline("demo", 5);
    let mut env = neutral_envelope("demo");
    env.results.baseline.pass = 10;
    env.results.lihaaf.pass = 8;
    // 10 != 8 + 0 — no excluded_fixtures to account for the delta.
    match compat_check_gate(&baseline, &env) {
        CompatGateOutcome::Block(msg) => {
            assert!(msg.contains("per-side totals"), "diagnostic: {msg}");
        }
        other => panic!("expected Block, got {other:?}"),
    }
}

#[test]
fn totals_divergence_with_matching_excluded_allows() {
    let baseline = one_crate_baseline("demo", 5);
    let mut env = neutral_envelope("demo");
    env.results.baseline.pass = 10;
    env.results.lihaaf.pass = 8;
    env.excluded_fixtures.push(CompatExcludedFixture {
        fixture: "tests/ui/a.rs".into(),
        reason: "compat limitation".into(),
    });
    env.excluded_fixtures.push(CompatExcludedFixture {
        fixture: "tests/ui/b.rs".into(),
        reason: "compat limitation".into(),
    });
    // 10 == 8 + 2 — delta accounted for.
    assert_eq!(compat_check_gate(&baseline, &env), CompatGateOutcome::Allow,);
}

#[test]
fn parse_baseline_rejects_negative_n_max() {
    let toml = b"[foo]\nn_max = -1\n";
    let err = compat_parse_baseline(toml, Path::new("baseline.toml"))
        .expect_err("negative n_max must reject");
    let msg = format!("{err:?}");
    assert!(msg.contains("non-negative"), "diagnostic: {msg}");
}

#[test]
fn parse_baseline_rejects_missing_n_max() {
    let toml = b"[foo]\nother = 1\n";
    let err = compat_parse_baseline(toml, Path::new("baseline.toml"))
        .expect_err("missing n_max must reject");
    let msg = format!("{err:?}");
    assert!(msg.contains("n_max"), "diagnostic: {msg}");
}

#[test]
fn parse_baseline_rejects_non_table_entry() {
    let toml = b"foo = 1\n";
    let err = compat_parse_baseline(toml, Path::new("baseline.toml"))
        .expect_err("non-table entry must reject");
    let msg = format!("{err:?}");
    assert!(msg.contains("must be a table"), "diagnostic: {msg}");
}
