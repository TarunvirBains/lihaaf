//! Session lifecycle and command flow.
//!
//! ## Orchestration
//!
//! `run` follows a simple staged flow. Failure in any stage stops
//! immediately with a report; this keeps behavior predictable.
//! The flow maps roughly onto:
//!
//! 1. Configuration load → [`crate::config::load`].
//! 2. Toolchain capture → [`crate::toolchain::capture`].
//! 3. Suite selection — apply `--suite` filtering across `config.suites`.
//! 4. For each selected suite (in declared metadata order):
//!    a. Dylib build for the suite's feature set → [`crate::dylib::build`].
//!    b. Dylib copy → [`crate::dylib::copy_dylib`] / `symlink_dylib`.
//!    c. Manifest refresh — per-suite manifest path under
//!    `target/lihaaf/manifest{,-<suite>}.json`.
//!    d. Fixture discovery → [`crate::discovery::collect`].
//!    e. Worker pool dispatch → [`crate::worker::dispatch_pool`].
//!    f. Per-suite result aggregation.
//! 5. Cross-suite result aggregation → [`Report`].
//! 6. Exit → caller maps [`Report::exit_code`] to a process exit code.
//!
//! ## Multi-suite
//!
//! A session always runs at least one suite (the implicit "default"
//! suite from the top-level `[package.metadata.lihaaf]` table). Adopters
//! that declare `[[package.metadata.lihaaf.suite]]` entries get an
//! independent dylib build per suite — `cargo lihaaf` without
//! `--suite` runs every defined suite in declared metadata order, and
//! `--suite NAME` (repeatable) restricts the run to the named subset.
//!
//! Each suite's build artifacts live under a suite-namespaced
//! `target/lihaaf-build{-<name>}` cargo target dir so different feature
//! sets do not thrash each other's incremental cache. The default suite
//! retains the legacy `target/lihaaf-build/` and `target/lihaaf/manifest.json`
//! paths so adopters who never add a named suite see no cache-key change.
//!
//! ## Why one function (and not a builder)
//!
//! The binary is the main caller and clap is the argument source, so
//! a single entry point is kept. A builder would be extra complexity for
//! the current usage model. A second consumer would warrant revisiting this.

use std::path::{Path, PathBuf};

use crate::cli::Cli;
use crate::config::{self, Config, Suite};
use crate::discovery;
use crate::dylib;
use crate::error::{Error, Outcome};
use crate::exit::ExitCode;
use crate::freshness::FreshnessSnapshot;
use crate::lock;
use crate::manifest::{self, Manifest};
use crate::normalize::NormalizationContext;
use crate::toolchain::{self, Toolchain};
use crate::util;
use crate::verdict::FixtureResult;
use crate::worker::{self, WorkerContext};

/// Aggregate result of a session run.
#[derive(Debug)]
pub struct Report {
    /// Per-fixture results across every suite that ran. Ordered by
    /// suite-execution order, then lexicographically within each suite.
    pub results: Vec<FixtureResult>,
    /// True when at least one cleanup error accumulated. Maps onto the
    /// `CLEANUP_RESIDUE` outcome.
    pub cleanup_residue: bool,
    /// Total wall-clock for the worker pool dispatch loop across every
    /// suite that ran, in milliseconds. Excludes per-suite dylib build
    /// time and discovery so the value is comparable to the legacy
    /// single-suite report.
    pub wall_ms: u64,
    /// Names of suites that actually ran, in execution order. Useful
    /// for tests and CI scripts that want to confirm a `--suite NAME`
    /// invocation routed correctly.
    pub suites_run: Vec<String>,
}

impl Report {
    /// Compute the binary's exit code per the policy (max-severity rule).
    pub fn exit_code(&self) -> ExitCode {
        let mut code = ExitCode::Ok;
        for r in &self.results {
            code = code.merge(r.verdict.exit_code());
        }
        if self.cleanup_residue {
            code = code.merge(ExitCode::CleanupResidue);
        }
        code
    }
}

/// Run the full session.
///
/// On success, returns a [`Report`] (potentially with non-OK verdicts
/// that the caller maps to a non-zero exit code). On a session-level
/// failure (config invalid, dylib build failed, etc.), returns
/// [`Error::Session`].
pub fn run(cli: Cli) -> Result<Report, Error> {
    // Mode-consistency validation. This is also enforced in
    // `cli::parse_from`, but a Rust caller that builds `Cli` via
    // direct field initialization (or via `Cli::try_parse_from`) would
    // otherwise bypass it — `--compat-root` without `--compat` is a
    // silent no-op in the non-compat session path, which is exactly
    // the shape the validator exists to reject. Calling here closes
    // the gap on both entry points. Idempotent: a second invocation
    // (after `parse_from` already validated) returns `Ok(())`.
    cli.validate_mode_consistency()?;

    // Stage 1 — configuration load.
    let manifest_path = resolve_manifest_path(&cli)?;
    let crate_root = derive_crate_root(&manifest_path);
    let config = config::load(&manifest_path)?;

    // Stage 2 — toolchain capture.
    let toolchain = toolchain::capture()?;

    // Stage 3 — suite selection.
    let selected_indexes = select_suites_by_cli(&config.suites, &cli.suite)?;
    let multi_suite = selected_indexes.len() > 1;

    // List mode short-circuits before the dylib build (the policy).
    // Multi-suite list mode walks each selected suite and prints its
    // fixtures; the per-suite header is only emitted when more than
    // one suite is selected so the single-suite output stays
    // byte-identical to v0.1.0-alpha.2.
    if cli.list {
        for &idx in &selected_indexes {
            let suite = &config.suites[idx];
            if multi_suite {
                eprintln!("# suite \"{}\"", suite.name);
            }
            let fixtures = discovery::collect(suite, &crate_root, &cli.filter)?;
            for f in &fixtures {
                println!("{}", f.relative_path);
            }
        }
        return Ok(Report {
            results: Vec::new(),
            cleanup_residue: false,
            wall_ms: 0,
            suites_run: selected_indexes
                .iter()
                .map(|&i| config.suites[i].name.clone())
                .collect(),
        });
    }

    let workspace_target = dylib::workspace_target_dir(&manifest_path);

    // Session lock (FIX_BEFORE_BETA Spec B, issue #1). Held for the
    // remainder of `run` to serialize concurrent `cargo lihaaf`
    // sessions that share a `CARGO_TARGET_DIR`. Acquired AFTER the
    // `--list` short-circuit above so read-only `--list` invocations
    // do not block on a busy target dir. Acquired BEFORE the
    // `--no-cache` deletion block below so a second session cannot
    // observe `target/lihaaf/` mid-delete. The underscore prefix
    // keeps clippy happy about the unused-binding ("used only for
    // its `Drop`") — releasing on session-end is the entire point.
    let _session_lock = lock::SessionLock::acquire(&workspace_target)?;

    // `--no-cache` (the policy): force a fresh dylib build by removing
    // every per-suite manifest AND every per-suite lihaaf-build target
    // dir BEFORE stage 4. Removing across all suites (not just the
    // selected ones) keeps `--no-cache` semantics simple: the user asked
    // for a clean slate, they get a clean slate.
    if cli.no_cache {
        for suite in &config.suites {
            let manifest_dest = manifest::manifest_path_for_suite(&workspace_target, &suite.name);
            if manifest_dest.exists() {
                let _ = std::fs::remove_file(&manifest_dest);
            }
            let build_dir = dylib::build_dir_for_suite(&workspace_target, &suite.name);
            if build_dir.exists() {
                let _ = std::fs::remove_dir_all(&build_dir);
            }
        }
        if !cli.quiet {
            let n = config.suites.len();
            let plural = if n == 1 { "" } else { "s" };
            eprintln!(
                "lihaaf: --no-cache: removed any prior manifest + lihaaf-build dir across {n} suite{plural}"
            );
        }
    }

    // Per-session temp dir (shared across suites). Each fixture's
    // workdir lives underneath; cleanup-residue / --keep-output behavior
    // applies to the whole session-temp parent, not per-suite.
    let session_temp = create_session_temp_dir(&workspace_target)?;
    let session_temp_path = session_temp.path().to_path_buf();

    let total_dispatch_start = std::time::Instant::now();
    let mut all_results: Vec<FixtureResult> = Vec::new();
    let mut suites_run: Vec<String> = Vec::with_capacity(selected_indexes.len());

    for &idx in &selected_indexes {
        let suite = &config.suites[idx];
        if multi_suite && !cli.quiet {
            eprintln!("lihaaf: === suite \"{}\" ===", suite.name);
        }
        let suite_results = run_one_suite(SuiteRunInput {
            suite,
            config: &config,
            cli: &cli,
            crate_root: &crate_root,
            manifest_path: &manifest_path,
            workspace_target: &workspace_target,
            toolchain: &toolchain,
            session_temp: &session_temp_path,
        })?;
        if multi_suite {
            print_per_suite_aggregate(suite, &suite_results);
        }
        all_results.extend(suite_results);
        suites_run.push(suite.name.clone());
    }

    let wall_ms = total_dispatch_start.elapsed().as_millis() as u64;

    let cleanup_residue = all_results.iter().any(|r| r.cleanup_failure.is_some());

    // Print the cross-suite aggregate report. Same shape as the v0.1.0-alpha.2
    // single-suite aggregate so CI scripts that grep on the existing
    // `lihaaf: N ok, N failed, N timeout, N memory_exhausted` line keep
    // working.
    print_aggregate(&all_results, wall_ms, cleanup_residue);

    // Preserve the per-session temp directory in two cases:
    //
    // 1. `--keep-output` is set (local-development escape hatch).
    // 2. Any fixture's per-fixture workdir cleanup failed (`CLEANUP_RESIDUE` case).
    //
    // `tempfile::TempDir` removes the directory on drop; `std::mem::
    // forget` is the documented way to suppress that drop while
    // keeping the path alive on disk.
    if cli.keep_output {
        eprintln!(
            "lihaaf: --keep-output set; per-fixture workdirs preserved under {}",
            session_temp_path.display()
        );
        std::mem::forget(session_temp);
    } else if cleanup_residue {
        eprintln!(
            "lihaaf: CLEANUP_RESIDUE detected; session-temp parent preserved at {}",
            session_temp_path.display()
        );
        std::mem::forget(session_temp);
    }

    Ok(Report {
        results: all_results,
        cleanup_residue,
        wall_ms,
        suites_run,
    })
}

fn create_session_temp_dir(workspace_target: &std::path::Path) -> Result<tempfile::TempDir, Error> {
    std::fs::create_dir_all(workspace_target).map_err(|e| {
        Error::io(
            e,
            "creating session temp parent dir",
            Some(workspace_target.to_path_buf()),
        )
    })?;

    tempfile::Builder::new()
        .prefix("lihaaf-session-")
        .tempdir_in(workspace_target)
        .map_err(|e| {
            Error::io(
                e,
                "creating session temp dir",
                Some(workspace_target.to_path_buf()),
            )
        })
}

#[cfg(test)]
mod session_temp_parent_tests {
    use super::create_session_temp_dir;

    #[test]
    fn creates_missing_target_parent_before_session_tempdir() {
        let tmp = tempfile::tempdir().unwrap();
        let target = tmp.path().join("fixture-crate").join("target");

        let session_temp = create_session_temp_dir(&target).unwrap();

        assert!(target.is_dir());
        assert!(session_temp.path().starts_with(&target));
    }
}

/// Bundle of inputs for [`run_one_suite`]. Grouped into a struct so the
/// signature stays readable as the per-suite stage list grows.
struct SuiteRunInput<'a> {
    suite: &'a Suite,
    config: &'a Config,
    cli: &'a Cli,
    crate_root: &'a Path,
    manifest_path: &'a Path,
    workspace_target: &'a Path,
    toolchain: &'a Toolchain,
    session_temp: &'a Path,
}

/// Stages 4a–4f for one suite. Returns the per-suite fixture results.
/// Session-level failures (dylib build, freshness drift, etc.) bubble
/// up via `Result::Err` and short-circuit the outer suite loop.
fn run_one_suite(input: SuiteRunInput<'_>) -> Result<Vec<FixtureResult>, Error> {
    let SuiteRunInput {
        suite,
        config,
        cli,
        crate_root,
        manifest_path,
        workspace_target,
        toolchain,
        session_temp,
    } = input;

    // Per-suite cargo target dir. The default suite uses the legacy
    // `target/lihaaf-build/` for cache-key stability; named suites get
    // `target/lihaaf-build-<suite>/` so different feature sets do not
    // thrash each other's incremental cache.
    let lihaaf_build_dir = dylib::build_dir_for_suite(workspace_target, &suite.name);

    // Stage 4a — dylib build with this suite's features.
    let build_started = std::time::Instant::now();
    let build_out = dylib::build(&dylib::BuildParams {
        crate_name: &config.dylib_crate,
        features: &suite.features,
        manifest_path,
        target_dir: &lihaaf_build_dir,
        toolchain,
    })?;
    if !cli.quiet {
        let secs = build_started.elapsed().as_secs_f64();
        eprintln!("lihaaf: built {} dylib in {:.1}s", config.dylib_crate, secs);
    }

    // Stage 4b — dylib copy (or symlink).
    let managed_path = dylib::managed_dylib_path(workspace_target, &build_out.cargo_dylib_path);
    if cli.use_symlink {
        dylib::symlink_dylib(&build_out.cargo_dylib_path, &managed_path)?;
    } else {
        dylib::copy_dylib(&build_out.cargo_dylib_path, &managed_path)?;
    }

    // Stage 4c — manifest refresh. Per-suite path so different suite
    // builds do not stomp on each other's cached state.
    let dylib_sha = util::sha256_file(&managed_path)?;
    let dylib_mtime = dylib::mtime_unix_secs(&managed_path)?;
    let metadata_snapshot = toml_value_to_json(&config.raw_metadata);
    let manifest_obj = Manifest {
        lihaaf_version: crate::VERSION.to_string(),
        suite_name: suite.name.clone(),
        rustc_release: toolchain.release_line.clone(),
        rustc_commit_hash: toolchain.commit_hash.clone(),
        host_triple: toolchain.host.clone(),
        sysroot: toolchain.sysroot.clone(),
        dylib_crate: config.dylib_crate.clone(),
        cargo_dylib_path: build_out.cargo_dylib_path.clone(),
        managed_dylib_path: managed_path.clone(),
        dylib_sha256: dylib_sha.clone(),
        dylib_mtime_unix_secs: dylib_mtime,
        use_symlink: cli.use_symlink,
        features: suite.features.clone(),
        extern_crates: suite.extern_crates.clone(),
        edition: suite.edition.clone(),
        metadata_snapshot,
    };
    let manifest_dest = manifest::manifest_path_for_suite(workspace_target, &suite.name);
    manifest_obj.write(&manifest_dest)?;

    // Capture the four policy invariants from per-suite startup state.
    // Re-checked per-fixture inside the worker pool dispatch loop. The
    // snapshot is per-suite because each suite has its own managed dylib
    // (different feature sets produce different SHA-256s).
    let freshness_snapshot = FreshnessSnapshot {
        managed_dylib_path: managed_path.clone(),
        original_mtime_unix_secs: dylib_mtime,
        original_sha256: dylib_sha.clone(),
        original_toolchain: toolchain.clone(),
    };

    // Stage 4d — fixture discovery for this suite.
    let fixtures = discovery::collect(suite, crate_root, &cli.filter)?;
    if !cli.quiet {
        let cf = fixtures
            .iter()
            .filter(|f| matches!(f.kind, discovery::FixtureKind::CompileFail))
            .count();
        let cp = fixtures.len() - cf;
        eprintln!(
            "lihaaf: {} fixtures discovered (compile_fail: {cf}, compile_pass: {cp})",
            fixtures.len()
        );
    }

    // Parallelism cap (the policy). Recomputed per-suite because the
    // RAM cap is derived from the suite's `per_fixture_memory_mb`, which
    // adopters may set higher for a heavier feature set.
    //
    // v0.1.0-alpha.3 intentionally does not carry `ParallelismGate`
    // OOM reductions across suite boundaries: each suite gets a fresh
    // dispatch pool and a fresh gate. Sharing that state would require
    // plumbing a mutable gate through `worker::dispatch_pool`; defer
    // until a real adopter reports cross-suite OOM cascades.
    let parallelism = compute_parallelism(cli, suite);
    if !cli.quiet {
        eprintln!("lihaaf: parallelism = {parallelism}");
    }

    // Build the worker context for this suite. The compat driver
    // (`compat::mod::build_inner_cli`) sets `inner_compat_normalize =
    // true` on the inner Cli so this session emits trybuild-shaped
    // short-form `$CARGO/<crate>-<ver>/...` snapshots per §3.2.2. Non-
    // compat callers leave the flag at its default `false` and observe
    // byte-identical v0.1 normalizer output.
    let norm_ctx = NormalizationContext::new(crate_root.to_path_buf(), toolchain.sysroot.clone())
        .with_compat_short_cargo(cli.inner_compat_normalize);
    let mut worker_ctx = WorkerContext::new(
        crate_root.to_path_buf(),
        managed_path.clone(),
        build_out.deps_dir.clone(),
        &config.dylib_crate,
        suite,
        cli.effective_bless(),
        cli.verbose,
        cli.keep_output,
        session_temp.to_path_buf(),
        norm_ctx,
        &toolchain.sysroot,
        freshness_snapshot,
    );

    // Resolve `--extern` paths for the non-dylib crates.
    let mut extra_names = worker_ctx.extra_extern_crates.clone();
    extra_names.extend(worker_ctx.dev_deps.iter().cloned());
    worker_ctx.extern_paths = worker::resolve_extern_paths(&build_out.deps_dir, &extra_names)?;

    // Mid-suite toolchain drift check (the policy): the captured
    // rustc identity is compared against a fresh capture across the
    // four-field key (release_line, host, commit_hash, sysroot). Cheap;
    // cost is dwarfed by the per-fixture rustc. The rendered Outcome
    // carries the full key string for both sides so an adopter seeing
    // a host/commit_hash/sysroot drift gets a diagnostic that names the
    // dimension, not two identical release lines.
    let post_capture = toolchain::capture()?;
    if !toolchain::matches(toolchain, &post_capture) {
        return Err(Error::Session(Outcome::ToolchainDrift {
            original: toolchain::format_drift_key(toolchain),
            current: toolchain::format_drift_key(&post_capture),
        }));
    }

    // Stage 4e — worker pool dispatch.
    let progress_quiet = cli.quiet;
    let dispatch_outcome = worker::dispatch_pool(&fixtures, &worker_ctx, parallelism, move |r| {
        if progress_quiet {
            if !r.verdict.is_pass() {
                eprintln!("lihaaf: {} {}", r.verdict.label(), r.relative_path);
            }
        } else {
            eprintln!(
                "lihaaf: {:>26} {} ({} ms)",
                r.verdict.label(),
                r.relative_path,
                r.wall_ms
            );
        }
        // Emit any non-fatal per-fixture warning on a separate line,
        // independent of `--quiet` (a warning that the adopter asked
        // for visibility into shouldn't be hidden by a verdict-only
        // quiet mode). the policy mandates the LARGE_SNAPSHOT line.
        emit_fixture_warnings(r);
    });

    // the policy: a per-dispatch freshness failure is a session-level
    // hard fail. Convert to the typed outcome and let main() map it
    // onto exit code 67 (same as the policy TOOLCHAIN_DRIFT).
    if let Some(failure) = dispatch_outcome.freshness_failure {
        return Err(Error::Session(Outcome::FreshnessDrift {
            invariant: failure.invariant_label().to_string(),
            detail: failure.detail(),
        }));
    }

    Ok(dispatch_outcome.results)
}

/// Resolve the user's `--suite` selection against the parsed
/// [`Config::suites`] list. Returns indices in declared metadata order
/// — explicitly NOT in CLI argument order, so a multi-suite invocation
/// produces deterministic suite ordering regardless of how the user
/// arranged their `--suite` flags.
///
/// An empty CLI selection (the user passed no `--suite`) selects every
/// suite. An unknown name (no match in `config.suites`) is rejected
/// with the list of valid names so the adopter can fix the typo.
pub fn select_suites_by_cli(
    suites: &[Suite],
    cli_selection: &[String],
) -> Result<Vec<usize>, Error> {
    if cli_selection.is_empty() {
        return Ok((0..suites.len()).collect());
    }
    // Build the set of requested names (de-duplicated) and verify each
    // matches a known suite. Iteration order is metadata-declared order.
    let mut requested: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
    for name in cli_selection {
        if !suites.iter().any(|s| s.name == *name) {
            let known: Vec<String> = suites.iter().map(|s| format!("\"{}\"", s.name)).collect();
            return Err(Error::Session(Outcome::ConfigInvalid {
                message: format!(
                    "--suite \"{name}\" does not match any suite in [package.metadata.lihaaf]. Known suites: [{}].\nWhy this matters: lihaaf only runs suites that are declared in metadata; --suite cannot create a new one.",
                    known.join(", ")
                ),
            }));
        }
        requested.insert(name.as_str());
    }
    Ok(suites
        .iter()
        .enumerate()
        .filter(|(_, s)| requested.contains(s.name.as_str()))
        .map(|(i, _)| i)
        .collect())
}

fn print_aggregate(results: &[FixtureResult], wall_ms: u64, cleanup_residue: bool) {
    use crate::verdict::Verdict;
    let mut counts: std::collections::BTreeMap<&'static str, usize> =
        std::collections::BTreeMap::new();
    for r in results {
        *counts.entry(r.verdict.label()).or_insert(0) += 1;
    }
    let mut summary = String::new();
    for (label, n) in &counts {
        if !summary.is_empty() {
            summary.push_str(", ");
        }
        summary.push_str(&format!("{n} {label}"));
    }
    if results.is_empty() {
        summary.push_str("0 results");
    }
    eprintln!("lihaaf: {summary}");

    // The aggregate line is kept to four buckets (`ok`, `failed`,
    // `summary` above carry every verdict label; this line re-projects
    // the four buckets reported as `ok`, `failed`, `timeout`,
    // adopter-friendly shape `<n> ok, <n> failed, <n> timeout, <n>
    // memory_exhausted` so CI greps and dashboards have a single fixed
    // line to anchor on regardless of which exotic verdicts a run
    // produced. `failed` aggregates everything that is neither pass
    // nor the named timeout/memory_exhausted buckets.
    let aggregate = aggregate_counts(results);
    eprintln!(
        "lihaaf: {} ok, {} failed, {} timeout, {} memory_exhausted",
        aggregate.ok, aggregate.failed, aggregate.timeout, aggregate.memory_exhausted
    );

    eprintln!("lihaaf: total wall-clock: {:.1}s", wall_ms as f64 / 1000.0);
    if cleanup_residue {
        eprintln!("lihaaf: CLEANUP_RESIDUE — one or more workdirs could not be removed:");
        for r in results {
            if let Some(c) = &r.cleanup_failure {
                eprintln!(
                    "  {} (path={}, error={})",
                    r.relative_path,
                    c.path.display(),
                    c.message
                );
            }
        }
    }

    // Render diff verdicts in full so adopters see the change.
    for r in results {
        match &r.verdict {
            Verdict::SnapshotDiff { diff } => {
                eprintln!("\n=== {} (SNAPSHOT_DIFF) ===\n{diff}", r.relative_path);
            }
            Verdict::SnapshotMissing { actual } => {
                eprintln!(
                    "\n=== {} (SNAPSHOT_MISSING) ===\n--- actual normalized stderr ---\n{actual}",
                    r.relative_path
                );
            }
            Verdict::ExpectedFailButPassed => {
                eprintln!(
                    "\n=== {} (EXPECTED_FAIL_BUT_PASSED) ===\nfixture compiled successfully but is in a compile_fail dir",
                    r.relative_path
                );
            }
            Verdict::ExpectedPassButFailed { stderr } => {
                eprintln!(
                    "\n=== {} (EXPECTED_PASS_BUT_FAILED) ===\n{stderr}",
                    r.relative_path
                );
            }
            Verdict::SnapshotDiffTooLarge {
                actual_lines,
                expected_lines,
                actual_head,
                expected_head,
            } => {
                eprintln!(
                    "\n=== {} (SNAPSHOT_DIFF_TOO_LARGE) ===\nactual: {actual_lines} lines, expected: {expected_lines} lines",
                    r.relative_path
                );
                eprintln!("--- actual head ---\n{actual_head}");
                eprintln!("--- expected head ---\n{expected_head}");
            }
            _ => {}
        }
    }
}

/// One-line per-suite aggregate emitted between suites in a multi-suite
/// run. Skipped on single-suite runs so the legacy output stays
/// byte-identical for adopters who never declare a named suite.
///
/// Format pinned to:
/// `lihaaf: suite "<name>": <n> ok, <n> failed, <n> timeout, <n> memory_exhausted`
fn print_per_suite_aggregate(suite: &Suite, results: &[FixtureResult]) {
    let agg = aggregate_counts(results);
    eprintln!(
        "lihaaf: suite \"{}\": {} ok, {} failed, {} timeout, {} memory_exhausted",
        suite.name, agg.ok, agg.failed, agg.timeout, agg.memory_exhausted
    );
}

/// Render any non-fatal warnings attached to a fixture result. Today
/// this is only `LARGE_SNAPSHOT` (the policy complexity ceiling), but
/// the emit point is generic so additional warning kinds slot in
/// without touching the dispatch loop.
///
/// Format pinned by the policy:
/// `lihaaf: LARGE_SNAPSHOT <path> (<expected>/<actual> lines)`.
/// The line is separate from the verdict line and does NOT alter the
/// fixture's verdict or the session exit code.
fn emit_fixture_warnings(r: &FixtureResult) {
    use crate::verdict::FixtureWarning;
    if let Some(w) = &r.warning {
        match w {
            FixtureWarning::LargeSnapshot {
                expected_lines,
                actual_lines,
            } => {
                eprintln!(
                    "lihaaf: LARGE_SNAPSHOT {} ({}/{} lines)",
                    r.relative_path, expected_lines, actual_lines
                );
            }
        }
    }
}

/// Bucketed counts for the policy aggregate line. Captures the four
/// names the worked example calls out (`ok`, `failed`, `timeout`,
/// `memory_exhausted`); every other verdict folds into `failed` so the
/// line stays at four buckets regardless of how exotic a run got.
#[derive(Debug, Default, Clone, Copy)]
struct AggregateCounts {
    ok: usize,
    failed: usize,
    timeout: usize,
    memory_exhausted: usize,
}

/// Bucket per-fixture verdicts into the four the policy aggregate names.
///
/// `Ok` and `Blessed` count as `ok` (verdict-table footnote: "Treated as
/// OK for exit-code purposes"). `Timeout` and `MemoryExhausted` are
/// dedicated buckets per the policy. Everything else (`SnapshotDiff`,
/// `SnapshotMissing`, `WorkerCrashed`, `MalformedDiagnostic`,
/// `SnapshotDiffTooLarge`, `ExpectedFailButPassed`,
/// `ExpectedPassButFailed`) folds into `failed` — the policy worked
/// example's "failed" bucket is the catch-all for "fixture did not pass
/// for a reason other than timeout or memory_exhausted."
fn aggregate_counts(results: &[FixtureResult]) -> AggregateCounts {
    use crate::verdict::Verdict;
    let mut a = AggregateCounts::default();
    for r in results {
        match &r.verdict {
            Verdict::Ok | Verdict::Blessed { .. } => a.ok += 1,
            Verdict::Timeout => a.timeout += 1,
            Verdict::MemoryExhausted => a.memory_exhausted += 1,
            _ => a.failed += 1,
        }
    }
    a
}

fn compute_parallelism(cli: &Cli, suite: &Suite) -> usize {
    let cpu_cap: usize = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1);
    let ram_cap: usize = match util::total_ram_mb() {
        Some(total) => {
            let cap = total / suite.per_fixture_memory_mb as u64;
            cap.max(1) as usize
        }
        None => cpu_cap, // honest fallback when the platform doesn't expose RAM
    };
    // `cli.jobs` is guaranteed positive: the clap value parser rejects
    // `-j 0` per the policy. The platform-derived `cpu_cap` and `ram_cap`
    // both clamp to >= 1 above. No defensive `max(1)` here — policy
    // forbids the silent-zero coercion.
    let cli_cap: usize = cli.jobs.map(|n| n as usize).unwrap_or(cpu_cap);
    cli_cap.min(ram_cap)
}

fn resolve_manifest_path(cli: &Cli) -> Result<PathBuf, Error> {
    if let Some(p) = &cli.manifest_path {
        if !p.is_file() {
            return Err(Error::Session(Outcome::ConfigInvalid {
                message: format!(
                    "--manifest-path={} does not point at an existing file.\nWhy this matters: lihaaf needs the consumer's Cargo.toml.",
                    p.display()
                ),
            }));
        }
        // Absolutize relative manifest paths so downstream
        // [`derive_crate_root`] never falls into the
        // single-component-relative trap: `Path::parent` of
        // `PathBuf::from("Cargo.toml")` is `Some("")`, not `None`, so
        // a naive `parent().unwrap_or_else(|| ".".into())` would set
        // `CARGO_MANIFEST_DIR=""` for the bare `--manifest-path
        // Cargo.toml` invocation, which is not Cargo-equivalent.
        // Cargo's own `CARGO_MANIFEST_DIR` is absolute, so we match
        // that shape by joining against `current_dir()` (lexical
        // absolutization, no symlink resolution — Cargo itself does
        // not canonicalize). Issue #14, gpt-5.5 xhigh PR-15 review.
        return absolutize_manifest_path(p);
    }
    // Walk up from CWD looking for Cargo.toml. `current_dir()` is
    // always absolute on the platforms we target, so candidates
    // produced here are already absolute and need no further
    // normalization.
    let mut dir =
        std::env::current_dir().map_err(|e| Error::io(e, "reading current directory", None))?;
    loop {
        let candidate = dir.join("Cargo.toml");
        if candidate.is_file() {
            return Ok(candidate);
        }
        if !dir.pop() {
            return Err(Error::Session(Outcome::ConfigInvalid {
                message:
                    "no Cargo.toml found in the current directory or any parent.\nWhy this matters: lihaaf reads `[package.metadata.lihaaf]` from the consumer crate.\nFix: cd into a crate directory or pass --manifest-path."
                        .into(),
            }));
        }
    }
}

/// Lexically absolutize a manifest path supplied via `--manifest-path`.
///
/// If the path is already absolute, return it unchanged. Otherwise,
/// join it against the process's current working directory. Symlinks
/// are not resolved — Cargo's own manifest-path handling is lexical,
/// and resolving symlinks here would produce a `CARGO_MANIFEST_DIR`
/// that diverges from what `cargo build --manifest-path …` would set.
fn absolutize_manifest_path(p: &Path) -> Result<PathBuf, Error> {
    if p.is_absolute() {
        return Ok(p.to_path_buf());
    }
    let cwd =
        std::env::current_dir().map_err(|e| Error::io(e, "reading current directory", None))?;
    Ok(cwd.join(p))
}

/// Derive the consumer-crate root from a (preferably absolute)
/// manifest path.
///
/// Production callers route through [`resolve_manifest_path`], which
/// absolutizes `--manifest-path` inputs, so `manifest_path.parent()`
/// is always non-empty on that path. The defensive `is_empty` guard
/// below pins the issue-14 invariant for direct callers and for any
/// future refactor that bypasses the absolutize step: a single-component
/// relative path like `PathBuf::from("Cargo.toml")` has
/// `parent() == Some("")` (not `None`), so the bare
/// `.unwrap_or_else(|| ".".into())` fallback never fires and the env
/// var would be set to the empty string. Falling back to `.` keeps
/// downstream `cmd.env("CARGO_MANIFEST_DIR", &crate_root)` working
/// against the rustc child's current working directory rather than
/// silently emitting an empty value — itself a non-Cargo-equivalent
/// path shape. Issue #14, gpt-5.5 xhigh PR-15 review.
fn derive_crate_root(manifest_path: &Path) -> PathBuf {
    match manifest_path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
        _ => PathBuf::from("."),
    }
}

fn toml_value_to_json(v: &toml::Value) -> serde_json::Value {
    match v {
        toml::Value::String(s) => serde_json::Value::String(s.clone()),
        toml::Value::Integer(i) => serde_json::Value::Number((*i).into()),
        toml::Value::Float(f) => serde_json::Number::from_f64(*f)
            .map(serde_json::Value::Number)
            .unwrap_or(serde_json::Value::Null),
        toml::Value::Boolean(b) => serde_json::Value::Bool(*b),
        toml::Value::Datetime(d) => serde_json::Value::String(d.to_string()),
        toml::Value::Array(a) => {
            serde_json::Value::Array(a.iter().map(toml_value_to_json).collect())
        }
        toml::Value::Table(t) => {
            let mut map = serde_json::Map::new();
            for (k, v) in t {
                map.insert(k.clone(), toml_value_to_json(v));
            }
            serde_json::Value::Object(map)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::DEFAULT_SUITE_NAME;

    fn suite(name: &str, per_fixture_mb: u32) -> Suite {
        Suite {
            name: name.into(),
            extern_crates: vec!["x".into()],
            fixture_dirs: vec![],
            features: vec![],
            edition: "2021".into(),
            dev_deps: vec![],
            compile_fail_marker: "compile_fail".into(),
            fixture_timeout_secs: 90,
            per_fixture_memory_mb: per_fixture_mb,
        }
    }

    /// Helper: build a `Cli` with every flag in its non-compat default
    /// posture, then let each test override the specific fields it
    /// cares about. Keeps unit tests insulated from purely-additive
    /// struct extensions (Phase 1 compat-mode fields, etc.).
    fn default_test_cli() -> Cli {
        Cli {
            bless: false,
            compat: false,
            compat_cargo_test_argv: None,
            compat_commit: None,
            compat_filter: vec![],
            compat_manifest: None,
            compat_report: None,
            compat_root: None,
            compat_trybuild_macro: vec![],
            filter: vec![],
            jobs: None,
            suite: vec![],
            no_cache: false,
            manifest_path: None,
            list: false,
            quiet: true,
            verbose: false,
            use_symlink: false,
            keep_output: false,
            inner_compat_normalize: false,
        }
    }

    #[test]
    fn parallelism_respects_explicit_jobs() {
        let mut cli = default_test_cli();
        cli.jobs = Some(2);
        let p = compute_parallelism(&cli, &suite(DEFAULT_SUITE_NAME, 1024));
        assert!(p <= 2);
        cli.jobs = Some(1);
        let p2 = compute_parallelism(&cli, &suite(DEFAULT_SUITE_NAME, 1024));
        assert_eq!(p2, 1);
    }

    #[test]
    fn parallelism_is_at_least_one() {
        let cli = default_test_cli();
        // Even with an absurd per-fixture cap, the result must not be 0.
        let p = compute_parallelism(&cli, &suite(DEFAULT_SUITE_NAME, u32::MAX / 2));
        assert!(p >= 1);
    }

    #[test]
    fn fixture_warnings_default_to_none_on_construction() {
        // The dispatch loop has a hard invariant: no FixtureResult is
        // constructed in worker.rs without an explicit `warning` field.
        // This test exercises the default-None case so a future change
        // that drops the field would fail to compile, signaling intent.
        use crate::verdict::{FixtureResult, Verdict};
        let r = FixtureResult {
            relative_path: "x".into(),
            verdict: Verdict::Ok,
            cleanup_failure: None,
            wall_ms: 0,
            warning: None,
        };
        assert!(r.warning.is_none());
    }

    #[test]
    fn aggregate_counts_buckets_per_section_3_3() {
        use crate::verdict::{FixtureResult, Verdict};
        let r = |label: &str, v: Verdict| FixtureResult {
            relative_path: label.to_string(),
            verdict: v,
            cleanup_failure: None,
            wall_ms: 0,
            warning: None,
        };
        let results = vec![
            r("a", Verdict::Ok),
            r(
                "b",
                Verdict::Blessed {
                    snapshot_path: PathBuf::from("/tmp/x.stderr"),
                },
            ),
            r("c", Verdict::Timeout),
            r("d", Verdict::Timeout),
            r("e", Verdict::MemoryExhausted),
            r(
                "f",
                Verdict::SnapshotDiff {
                    diff: "diff".into(),
                },
            ),
            r("g", Verdict::ExpectedFailButPassed),
            r(
                "h",
                Verdict::WorkerCrashed {
                    cause: "signal: 11".into(),
                },
            ),
        ];
        let agg = aggregate_counts(&results);
        // Ok + Blessed → 2 ok. SnapshotDiff + ExpectedFailButPassed +
        // WorkerCrashed → 3 failed. Two Timeout, one MemoryExhausted.
        assert_eq!(agg.ok, 2);
        assert_eq!(agg.failed, 3);
        assert_eq!(agg.timeout, 2);
        assert_eq!(agg.memory_exhausted, 1);
    }

    /// Run `derive_crate_root` against `input` and assert:
    /// 1. The returned path equals `PathBuf::from(expected)`.
    /// 2. The returned path is NEVER the empty path — this pins the
    ///    issue #14 / xhigh PR-15 invariant: a bare `Cargo.toml` (single
    ///    segment) has `parent() == Some("")`, so the original
    ///    `.unwrap_or_else(|| ".".into())` fallback was bypassed and
    ///    `CARGO_MANIFEST_DIR=""` propagated to the per-fixture rustc.
    ///    `derive_crate_root` keeps the invariant intact even when callers
    ///    skip the absolutize step.
    fn assert_derive_crate_root_equals(input: &str, expected: &str) {
        let root = derive_crate_root(&PathBuf::from(input));
        assert_eq!(root, PathBuf::from(expected));
        assert!(
            !root.as_os_str().is_empty(),
            "crate root must never be the empty path (issue #14 / xhigh PR-15 review)"
        );
    }

    /// Regression for the issue-14 follow-up (gpt-5.5 xhigh PR-15 review):
    /// the single-component-relative manifest path
    /// `PathBuf::from("Cargo.toml")` must not yield an empty
    /// crate-root. `Path::parent()` returns `Some("")` for such inputs,
    /// not `None`, so the original
    /// `.parent().unwrap_or_else(|| ".".into())` fallback was bypassed
    /// and `CARGO_MANIFEST_DIR=""` propagated to the per-fixture rustc.
    /// `derive_crate_root` keeps the invariant intact even when callers
    /// skip the absolutize step.
    #[test]
    fn derive_crate_root_handles_bare_cargo_toml() {
        assert_derive_crate_root_equals("Cargo.toml", ".");
    }

    /// A `parent()` of `None` (the original assumed case) must also
    /// resolve to `.`. The empty path itself is the trivial input that
    /// produces this shape.
    #[test]
    fn derive_crate_root_falls_back_for_parentless_input() {
        assert_derive_crate_root_equals("", ".");
    }

    /// Absolute manifest paths produce absolute crate roots — the
    /// Cargo-equivalent shape adopters expect.
    #[test]
    fn derive_crate_root_returns_parent_of_absolute_manifest() {
        #[cfg(unix)]
        assert_derive_crate_root_equals("/abs/pkg/Cargo.toml", "/abs/pkg");
        #[cfg(windows)]
        assert_derive_crate_root_equals(r"C:\abs\pkg\Cargo.toml", r"C:\abs\pkg");
    }

    /// Multi-component relative paths already had a non-empty
    /// `parent()`, but the invariant is pinned here so a future
    /// refactor cannot regress them either.
    #[test]
    fn derive_crate_root_returns_parent_of_relative_workspace_member() {
        assert_derive_crate_root_equals("member/Cargo.toml", "member");
    }

    /// `resolve_manifest_path` must absolutize a relative
    /// `--manifest-path` so the derived crate root has Cargo's
    /// absolute shape rather than a relative `.`/empty fragment.
    /// The session-startup path then feeds an absolute
    /// `CARGO_MANIFEST_DIR` to every per-fixture rustc — matching what
    /// `cargo build --manifest-path Cargo.toml` would set.
    #[test]
    fn absolutize_manifest_path_promotes_bare_cargo_toml_to_absolute() {
        let abs = absolutize_manifest_path(&PathBuf::from("Cargo.toml"))
            .expect("current_dir is readable in this test");
        assert!(
            abs.is_absolute(),
            "absolutized manifest path must be absolute, got {abs:?}"
        );
        let root = derive_crate_root(&abs);
        assert!(
            root.is_absolute(),
            "derived crate root must be absolute, got {root:?}"
        );
        assert!(
            !root.as_os_str().is_empty(),
            "derived crate root must never be empty"
        );
        // The crate root is precisely the cwd in which the test runs,
        // because `Cargo.toml` is a single-segment file name.
        let cwd = std::env::current_dir().unwrap();
        assert_eq!(root, cwd);
    }

    /// An already-absolute `--manifest-path` round-trips unchanged.
    #[test]
    fn absolutize_manifest_path_preserves_absolute_input() {
        #[cfg(unix)]
        let input = PathBuf::from("/etc/Cargo.toml");
        #[cfg(windows)]
        let input = PathBuf::from(r"C:\etc\Cargo.toml");
        let out = absolutize_manifest_path(&input).unwrap();
        assert_eq!(out, input);
    }

    #[test]
    fn toml_to_json_round_trips_table_shape() {
        let toml_text = r#"
            a = 1
            b = "two"
            c = [3, 4]
            [d]
            e = true
        "#;
        // toml 1.x: parse a document via `from_str`, not `parse()`
        // (the latter now parses a single value).
        let v: toml::Value = toml::from_str(toml_text).unwrap();
        let j = toml_value_to_json(&v);
        assert_eq!(j["a"], serde_json::json!(1));
        assert_eq!(j["b"], serde_json::json!("two"));
        assert_eq!(j["c"], serde_json::json!([3, 4]));
        assert_eq!(j["d"]["e"], serde_json::json!(true));
    }

    // ---- Suite selection ----

    fn suites_named(names: &[&str]) -> Vec<Suite> {
        names.iter().map(|n| suite(n, 1024)).collect()
    }

    #[test]
    fn select_suites_empty_cli_returns_all_in_declared_order() {
        let suites = suites_named(&["default", "spatial", "extra"]);
        let idx = select_suites_by_cli(&suites, &[]).unwrap();
        assert_eq!(idx, vec![0, 1, 2]);
    }

    #[test]
    fn select_suites_filters_to_named_subset_in_declared_order() {
        let suites = suites_named(&["default", "spatial", "extra"]);
        // CLI argument order is "extra" then "spatial" but execution
        // order is metadata-declared order (spatial before extra).
        let idx =
            select_suites_by_cli(&suites, &["extra".to_string(), "spatial".to_string()]).unwrap();
        assert_eq!(idx, vec![1, 2]);
    }

    #[test]
    fn select_suites_dedups_repeated_cli_names() {
        let suites = suites_named(&["default", "spatial"]);
        let idx = select_suites_by_cli(
            &suites,
            &[
                "spatial".to_string(),
                "spatial".to_string(),
                "default".to_string(),
            ],
        )
        .unwrap();
        assert_eq!(idx, vec![0, 1]);
    }

    #[test]
    fn select_suites_unknown_name_rejected_with_known_list() {
        let suites = suites_named(&["default", "spatial"]);
        let err = select_suites_by_cli(&suites, &["unknown".to_string()]).unwrap_err();
        match err {
            Error::Session(Outcome::ConfigInvalid { message }) => {
                assert!(message.contains("\"unknown\""));
                assert!(message.contains("\"default\""));
                assert!(message.contains("\"spatial\""));
            }
            other => panic!("expected ConfigInvalid, got {other:?}"),
        }
    }

    #[test]
    fn select_suites_default_is_addressable_by_cli_name() {
        let suites = suites_named(&["default", "spatial"]);
        let idx = select_suites_by_cli(&suites, &["default".to_string()]).unwrap();
        assert_eq!(idx, vec![0]);
    }
}
