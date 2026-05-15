//! Phase 10 of compat mode — §5 pilot gate logic.
//!
//! Reads a `compat/baseline.toml` ceiling table and validates a §3.3
//! [`CompatEnvelope`](crate::compat::report::CompatEnvelope) against the
//! per-crate `N_<crate>` ceiling. See
//! `docs/compatibility-plan.md` §5 for the contract this implements.
//!
//! ## What this module owns
//!
//! - [`Ceiling`] — one per-crate ceiling row parsed from
//!   `compat/baseline.toml`.
//! - [`parse_baseline`] — parse a TOML byte slice into the ceiling map.
//! - [`check_gate`] — evaluate a [`CompatEnvelope`] against the loaded
//!   ceiling and return [`GateOutcome`].
//!
//! ## What this module does NOT own
//!
//! - The CI workflow YAML. The shell-level "fail the PR" mapping lives
//!   in `.github/workflows/lihaaf-compat-gate.yml`. This module is the
//!   typed primitive the workflow invokes (directly via a CLI flag or
//!   indirectly via `jq` in the dry-run period); the workflow is the
//!   policy.
//! - Envelope construction. The envelope is produced by the compat
//!   driver ([`crate::compat::run`]); this module only consumes it.
//!
//! ## Pilot gate (§5) rules
//!
//! The gate accepts an envelope when EVERY rule below is satisfied,
//! mirroring the field list in `docs/compatibility-plan.md:239-244`:
//!
//! 1. `crate_name` exists in `baseline.toml` (otherwise the pilot is not
//!    enrolled and the gate is a NO-OP — the workflow shows a "no
//!    ceiling configured" message and lets the PR proceed).
//! 2. `errors` is empty (§5: `errors == []` — any envelope-recorded
//!    error invalidates the run).
//! 3. `results.mismatch_count <= N_<crate>` (the shrinking-only rule).
//! 4. `results.baseline.unknown_count == 0` (no unrecognized libtest
//!    lines — the §1 conservatism rule must produce a clean signal).
//! 5. `results.baseline.exit_code == 0` (the baseline `cargo test`
//!    succeeded; a failed baseline produces meaningless deltas).
//! 6. `results.lihaaf.exit_code == 0` (the inner lihaaf run produced
//!    a clean session).
//! 7. `baseline.pass + baseline.fail == lihaaf.pass + lihaaf.fail +
//!    excluded_fixtures.len()` (§5: the per-side totals must match
//!    unless the `excluded_fixtures` set accounts for the delta).
//!
//! Any rule violation produces [`GateOutcome::Block`] with a directed
//! diagnostic naming the offending field and threshold; otherwise
//! [`GateOutcome::Allow`].
//!
//! ## v0.1.0-beta.4 dry-run note
//!
//! The shipped `compat/baseline.toml` at the repo root is empty for
//! v0.1.0-beta.4 — no pilot crates are enrolled. The gate is a NO-OP
//! until pilot PRs add entries; the workflow YAML wires up the dry-run
//! shape so subsequent PRs can populate the table without the workflow
//! itself being net-new.

use std::collections::BTreeMap;
use std::path::Path;

use crate::compat::report::CompatEnvelope;
use crate::error::Error;

/// One per-crate ceiling row.
///
/// `n_max` is the §5 "shrinking-only" cap: the number of mismatch
/// entries the pilot crate is currently allowed to ship. The number
/// MAY DECREASE in subsequent PRs (a pilot recording a stable run
/// reduces its ceiling); the gate REJECTS any PR that increases the
/// ceiling without explicit review (enforced in PR review, not by this
/// module — the gate only reads the current ceiling).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ceiling {
    /// Maximum number of `results.mismatch_count` entries the gate
    /// allows. Per §5 the value shrinks over time as the pilot stabilizes.
    ///
    /// The crate name is the BTreeMap key — not duplicated here.
    pub n_max: u32,
}

/// Outcome of a single [`check_gate`] invocation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GateOutcome {
    /// The envelope satisfies every rule. Workflow exit code 0.
    Allow,
    /// The crate is not enrolled in `baseline.toml` — the gate is a
    /// NO-OP. Workflow exit code 0 with a "no ceiling configured"
    /// message; the PR proceeds.
    NotEnrolled,
    /// The envelope violates at least one rule. Workflow exit code 1
    /// (or the specific exit code the workflow maps to a "block").
    /// `reason` is human-readable and names the offending field +
    /// threshold.
    Block(String),
}

/// Parse a `compat/baseline.toml` byte slice into a ceiling map keyed by
/// crate name.
///
/// The schema is a flat top-level table where each key is a crate name
/// and the value is a table with an `n_max` integer. Example:
///
/// ```toml
/// [serde-json]
/// n_max = 12
///
/// [anyhow]
/// n_max = 3
/// ```
///
/// Empty input (the v0.1.0-beta.4 default) produces an empty map.
///
/// **Errors.** Returns [`Error::TomlParse`] on malformed TOML or
/// non-integer / negative `n_max` values.
pub fn parse_baseline(
    toml_bytes: &[u8],
    source: &Path,
) -> Result<BTreeMap<String, Ceiling>, Error> {
    let text = std::str::from_utf8(toml_bytes).map_err(|e| Error::TomlParse {
        path: source.to_path_buf(),
        message: format!("baseline.toml is not valid UTF-8: {e}"),
    })?;
    let value: toml::Value =
        toml::from_str(text).map_err(|e: toml::de::Error| Error::TomlParse {
            path: source.to_path_buf(),
            message: format!("baseline.toml: {e}"),
        })?;
    let Some(top) = value.as_table() else {
        return Err(Error::TomlParse {
            path: source.to_path_buf(),
            message: "baseline.toml top level must be a table".into(),
        });
    };

    let mut out: BTreeMap<String, Ceiling> = BTreeMap::new();
    for (crate_name, entry) in top {
        let Some(sub) = entry.as_table() else {
            return Err(Error::TomlParse {
                path: source.to_path_buf(),
                message: format!("baseline.toml entry for crate `{crate_name}` must be a table"),
            });
        };
        let n_max_value = sub.get("n_max").ok_or_else(|| Error::TomlParse {
            path: source.to_path_buf(),
            message: format!(
                "baseline.toml entry for crate `{crate_name}` is missing required key `n_max`"
            ),
        })?;
        let n_max_i64 = n_max_value.as_integer().ok_or_else(|| Error::TomlParse {
            path: source.to_path_buf(),
            message: format!("baseline.toml `{crate_name}.n_max` must be a non-negative integer"),
        })?;
        if n_max_i64 < 0 {
            return Err(Error::TomlParse {
                path: source.to_path_buf(),
                message: format!(
                    "baseline.toml `{crate_name}.n_max = {n_max_i64}` must be non-negative"
                ),
            });
        }
        let n_max = u32::try_from(n_max_i64).map_err(|_| Error::TomlParse {
            path: source.to_path_buf(),
            message: format!(
                "baseline.toml `{crate_name}.n_max = {n_max_i64}` exceeds the u32 range"
            ),
        })?;
        out.insert(crate_name.clone(), Ceiling { n_max });
    }
    Ok(out)
}

/// Evaluate `envelope` against the loaded ceiling map.
///
/// See the module-level docs for the §5 rules this enforces.
///
/// **No side effects.** This function does not touch the filesystem
/// and does not write to stdout/stderr; the caller renders the
/// [`GateOutcome`] for the workflow log.
pub fn check_gate(baseline: &BTreeMap<String, Ceiling>, envelope: &CompatEnvelope) -> GateOutcome {
    let Some((crate_name, ceiling)) = baseline.get_key_value(&envelope.crate_name) else {
        return GateOutcome::NotEnrolled;
    };

    if !envelope.errors.is_empty() {
        return GateOutcome::Block(format!(
            "envelope.errors carries {} entry/entries (must be empty; the §5 gate refuses to \
             score a run that recorded an error). First error: type=`{}` detail=`{}`",
            envelope.errors.len(),
            envelope.errors[0].error_type,
            envelope.errors[0].detail,
        ));
    }

    if envelope.results.baseline.unknown_count != 0 {
        return GateOutcome::Block(format!(
            "baseline.unknown_count = {} (must be 0; the §1 conservatism rule requires every \
             libtest verdict to correlate to a recognized fixture before the pilot gate is \
             meaningful)",
            envelope.results.baseline.unknown_count,
        ));
    }

    if envelope.results.baseline.exit_code != 0 {
        return GateOutcome::Block(format!(
            "baseline.exit_code = {} (must be 0; the baseline `cargo test` invocation must \
             succeed before deltas are meaningful)",
            envelope.results.baseline.exit_code,
        ));
    }

    if envelope.results.lihaaf.exit_code != 0 {
        return GateOutcome::Block(format!(
            "lihaaf.exit_code = {} (must be 0; the inner lihaaf compat run must produce a \
             clean session)",
            envelope.results.lihaaf.exit_code,
        ));
    }

    if envelope.results.mismatch_count > ceiling.n_max {
        return GateOutcome::Block(format!(
            "mismatch_count = {} exceeds ceiling `{}.n_max = {}` (the §5 shrinking-only rule: \
             a PR may decrease but not increase the per-crate ceiling)",
            envelope.results.mismatch_count, crate_name, ceiling.n_max,
        ));
    }

    let baseline_total =
        u64::from(envelope.results.baseline.pass) + u64::from(envelope.results.baseline.fail);
    let lihaaf_total =
        u64::from(envelope.results.lihaaf.pass) + u64::from(envelope.results.lihaaf.fail);
    let excluded_count = envelope.excluded_fixtures.len() as u64;
    if baseline_total != lihaaf_total + excluded_count {
        return GateOutcome::Block(format!(
            "per-side totals diverge: baseline.pass+fail = {} but lihaaf.pass+fail = {} and \
             excluded_fixtures.len() = {} (§5 rule: baseline total must equal lihaaf total \
             plus excluded count)",
            baseline_total, lihaaf_total, excluded_count,
        ));
    }

    GateOutcome::Allow
}

/// Read a `baseline.toml` file from disk and call [`parse_baseline`].
/// Convenience entry for tests and the (future) CI runner; the gate
/// itself ([`check_gate`]) accepts the already-parsed map so the test
/// suite can synthesize ceilings without writing to disk.
pub fn load_baseline(path: &Path) -> Result<BTreeMap<String, Ceiling>, Error> {
    let bytes = std::fs::read(path)
        .map_err(|e| Error::io(e, "reading compat baseline.toml", Some(path.to_path_buf())))?;
    parse_baseline(&bytes, path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compat::report::{
        BaselineCounts, Commands, CompatEnvelope, LihaafCounts, OverlayMetadata, Results,
    };

    fn envelope_with(
        crate_name: &str,
        mismatch_count: u32,
        baseline_unknown: u32,
        baseline_exit: i32,
        lihaaf_exit: i32,
    ) -> CompatEnvelope {
        CompatEnvelope {
            schema_version: 1,
            mode: "compat".into(),
            crate_name: crate_name.into(),
            commit: String::new(),
            commands: Commands {
                baseline: "cargo test".into(),
                lihaaf: "cargo lihaaf --compat".into(),
            },
            results: Results {
                baseline: BaselineCounts {
                    pass: 0,
                    fail: 0,
                    unknown_count: baseline_unknown,
                    exit_code: baseline_exit,
                    dur_ms: 0,
                },
                lihaaf: LihaafCounts {
                    pass: 0,
                    fail: 0,
                    exit_code: lihaaf_exit,
                    dur_ms: 0,
                    toolchain: "rustc 1.95.0".into(),
                },
                mismatch_count,
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
    fn parse_baseline_accepts_empty_input() {
        let map = parse_baseline(b"", Path::new("baseline.toml")).expect("empty must parse");
        assert!(map.is_empty());
    }

    #[test]
    fn parse_baseline_reads_one_crate() {
        let toml = b"[serde-json]\nn_max = 12\n";
        let map = parse_baseline(toml, Path::new("baseline.toml")).expect("must parse");
        assert_eq!(map.len(), 1);
        assert_eq!(map["serde-json"].n_max, 12);
    }

    #[test]
    fn parse_baseline_rejects_negative_n_max() {
        let toml = b"[foo]\nn_max = -1\n";
        let err = parse_baseline(toml, Path::new("baseline.toml")).expect_err("must reject");
        let msg = format!("{err:?}");
        assert!(msg.contains("non-negative"), "got: {msg}");
    }

    #[test]
    fn parse_baseline_rejects_missing_n_max() {
        let toml = b"[foo]\nother = 1\n";
        let err = parse_baseline(toml, Path::new("baseline.toml")).expect_err("must reject");
        let msg = format!("{err:?}");
        assert!(msg.contains("n_max"), "got: {msg}");
    }

    #[test]
    fn check_gate_unenrolled_crate_is_noop() {
        let baseline = BTreeMap::new();
        let env = envelope_with("not-listed", 99, 0, 0, 0);
        assert_eq!(check_gate(&baseline, &env), GateOutcome::NotEnrolled);
    }

    #[test]
    fn check_gate_under_ceiling_passes() {
        let mut baseline = BTreeMap::new();
        baseline.insert("demo".into(), Ceiling { n_max: 5 });
        let env = envelope_with("demo", 3, 0, 0, 0);
        assert_eq!(check_gate(&baseline, &env), GateOutcome::Allow);
    }

    #[test]
    fn check_gate_at_ceiling_passes() {
        let mut baseline = BTreeMap::new();
        baseline.insert("demo".into(), Ceiling { n_max: 5 });
        let env = envelope_with("demo", 5, 0, 0, 0);
        assert_eq!(check_gate(&baseline, &env), GateOutcome::Allow);
    }

    #[test]
    fn check_gate_over_ceiling_blocks() {
        let mut baseline = BTreeMap::new();
        baseline.insert("demo".into(), Ceiling { n_max: 5 });
        let env = envelope_with("demo", 6, 0, 0, 0);
        match check_gate(&baseline, &env) {
            GateOutcome::Block(msg) => {
                assert!(msg.contains("mismatch_count"), "got: {msg}");
                assert!(msg.contains("n_max"), "got: {msg}");
            }
            other => panic!("expected Block, got {other:?}"),
        }
    }

    #[test]
    fn check_gate_blocks_on_baseline_unknown_count() {
        let mut baseline = BTreeMap::new();
        baseline.insert("demo".into(), Ceiling { n_max: 5 });
        let env = envelope_with("demo", 0, 1, 0, 0);
        match check_gate(&baseline, &env) {
            GateOutcome::Block(msg) => assert!(msg.contains("unknown_count"), "got: {msg}"),
            other => panic!("expected Block, got {other:?}"),
        }
    }

    #[test]
    fn check_gate_blocks_on_baseline_exit_code_nonzero() {
        let mut baseline = BTreeMap::new();
        baseline.insert("demo".into(), Ceiling { n_max: 5 });
        let env = envelope_with("demo", 0, 0, 1, 0);
        match check_gate(&baseline, &env) {
            GateOutcome::Block(msg) => assert!(msg.contains("baseline.exit_code"), "got: {msg}"),
            other => panic!("expected Block, got {other:?}"),
        }
    }

    #[test]
    fn check_gate_blocks_on_lihaaf_exit_code_nonzero() {
        let mut baseline = BTreeMap::new();
        baseline.insert("demo".into(), Ceiling { n_max: 5 });
        let env = envelope_with("demo", 0, 0, 0, 1);
        match check_gate(&baseline, &env) {
            GateOutcome::Block(msg) => assert!(msg.contains("lihaaf.exit_code"), "got: {msg}"),
            other => panic!("expected Block, got {other:?}"),
        }
    }

    #[test]
    fn check_gate_blocks_when_errors_nonempty() {
        let mut baseline = BTreeMap::new();
        baseline.insert("demo".into(), Ceiling { n_max: 5 });
        let mut env = envelope_with("demo", 0, 0, 0, 0);
        env.errors.push(crate::compat::report::EnvelopeError {
            error_type: "discovery_unrecognized".into(),
            fixture: None,
            file: "tests/trybuild.rs".into(),
            line: 42,
            detail: "unrecognized test pattern".into(),
        });
        match check_gate(&baseline, &env) {
            GateOutcome::Block(msg) => {
                assert!(msg.contains("envelope.errors"), "got: {msg}");
                assert!(msg.contains("discovery_unrecognized"), "got: {msg}");
            }
            other => panic!("expected Block, got {other:?}"),
        }
    }

    #[test]
    fn check_gate_blocks_when_totals_diverge_without_excluded() {
        let mut baseline = BTreeMap::new();
        baseline.insert("demo".into(), Ceiling { n_max: 5 });
        let mut env = envelope_with("demo", 0, 0, 0, 0);
        env.results.baseline.pass = 10;
        env.results.baseline.fail = 0;
        env.results.lihaaf.pass = 8;
        env.results.lihaaf.fail = 0;
        // 10 != 8 + 0 — divergence not accounted for.
        match check_gate(&baseline, &env) {
            GateOutcome::Block(msg) => {
                assert!(msg.contains("per-side totals"), "got: {msg}");
                assert!(msg.contains("10"), "got: {msg}");
                assert!(msg.contains("8"), "got: {msg}");
            }
            other => panic!("expected Block, got {other:?}"),
        }
    }

    #[test]
    fn check_gate_allows_when_excluded_accounts_for_delta() {
        let mut baseline = BTreeMap::new();
        baseline.insert("demo".into(), Ceiling { n_max: 5 });
        let mut env = envelope_with("demo", 0, 0, 0, 0);
        env.results.baseline.pass = 10;
        env.results.baseline.fail = 0;
        env.results.lihaaf.pass = 8;
        env.results.lihaaf.fail = 0;
        env.excluded_fixtures
            .push(crate::compat::report::ExcludedFixture {
                fixture: "tests/ui/skip_a.rs".into(),
                reason: "compat limitation".into(),
            });
        env.excluded_fixtures
            .push(crate::compat::report::ExcludedFixture {
                fixture: "tests/ui/skip_b.rs".into(),
                reason: "compat limitation".into(),
            });
        // 10 == 8 + 2 — divergence accounted for.
        assert_eq!(check_gate(&baseline, &env), GateOutcome::Allow);
    }
}
