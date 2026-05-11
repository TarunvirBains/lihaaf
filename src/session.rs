//! Session lifecycle (spec §4.1 stages 1–9).
//!
//! ## Orchestration
//!
//! `run` walks the spec's stages in order. Any stage's failure is
//! terminal — we do not skip ahead. The eight stages map onto:
//!
//! 1. Configuration load → [`crate::config::load`].
//! 2. Toolchain capture → [`crate::toolchain::capture`].
//! 3. Dylib build → [`crate::dylib::build`].
//! 4. Dylib copy → [`crate::dylib::copy_dylib`] / `symlink_dylib`.
//! 5. Manifest refresh → [`crate::manifest::Manifest::write`].
//! 6. Fixture discovery → [`crate::discovery::collect`].
//! 7. Worker pool dispatch → [`crate::worker::dispatch_pool`].
//! 8. Result aggregation → [`Report`].
//! 9. Exit → caller maps [`Report::exit_code`] to a process exit code.
//!
//! ## Why a single function (and not a builder)
//!
//! v0.1's caller is the binary. There's exactly one entry point, the
//! arguments come from clap. A builder would over-design the surface
//! for one consumer. v0.x revisits if a second consumer materializes.

use std::path::{Path, PathBuf};

use crate::cli::Cli;
use crate::config;
use crate::discovery;
use crate::dylib;
use crate::error::{Error, Outcome};
use crate::exit::ExitCode;
use crate::manifest::Manifest;
use crate::normalize::NormalizationContext;
use crate::toolchain;
use crate::util;
use crate::verdict::FixtureResult;
use crate::worker::{self, WorkerContext};

/// Aggregate result of a session run.
#[derive(Debug)]
pub struct Report {
    /// Per-fixture results in deterministic (lexicographic) order.
    pub results: Vec<FixtureResult>,
    /// True when at least one cleanup error accumulated. Maps onto the
    /// `CLEANUP_RESIDUE` outcome.
    pub cleanup_residue: bool,
    /// Total wall-clock for the worker pool, in milliseconds.
    pub wall_ms: u64,
}

impl Report {
    /// Compute the binary's exit code per spec §10.3 (max-severity rule).
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
    // Stage 1 — configuration load.
    let manifest_path = resolve_manifest_path(&cli)?;
    let crate_root = manifest_path
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."));
    let config = config::load(&manifest_path)?;

    // Stage 2 — toolchain capture.
    let toolchain = toolchain::capture()?;

    // List mode short-circuits before the dylib build (spec §8.2).
    if cli.list {
        let fixtures = discovery::collect(&config, &crate_root, &cli.filter)?;
        for f in &fixtures {
            println!("{}", f.relative_path);
        }
        return Ok(Report {
            results: Vec::new(),
            cleanup_residue: false,
            wall_ms: 0,
        });
    }

    let workspace_target = dylib::workspace_target_dir(&manifest_path);
    let lihaaf_build_dir = workspace_target.join("lihaaf-build");

    // Stage 3 — dylib build.
    let build_started = std::time::Instant::now();
    let build_out = dylib::build(&dylib::BuildParams {
        crate_name: &config.dylib_crate,
        features: &config.features,
        manifest_path: &manifest_path,
        target_dir: &lihaaf_build_dir,
        toolchain: &toolchain,
    })?;
    if !cli.quiet {
        let secs = build_started.elapsed().as_secs_f64();
        eprintln!("lihaaf: built {} dylib in {:.1}s", config.dylib_crate, secs);
    }

    // Stage 4 — dylib copy (or symlink).
    let managed_path = dylib::managed_dylib_path(&workspace_target, &build_out.cargo_dylib_path);
    if cli.use_symlink {
        dylib::symlink_dylib(&build_out.cargo_dylib_path, &managed_path)?;
    } else {
        dylib::copy_dylib(&build_out.cargo_dylib_path, &managed_path)?;
    }

    // Stage 5 — manifest refresh.
    let dylib_sha = util::sha256_file(&managed_path)?;
    let dylib_mtime = dylib::mtime_unix_secs(&managed_path)?;
    let metadata_snapshot = toml_value_to_json(&config.raw_metadata);
    let manifest = Manifest {
        lihaaf_version: crate::VERSION.to_string(),
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
        features: config.features.clone(),
        extern_crates: config.extern_crates.clone(),
        edition: config.edition.clone(),
        metadata_snapshot,
    };
    let manifest_dest = workspace_target.join("lihaaf/manifest.json");
    manifest.write(&manifest_dest)?;
    // `--no-cache` forces a fresh dylib build but does not change any
    // other behavior; the rebuild already happened above. The flag is
    // honored simply by not consulting any prior manifest.
    let _ = cli.no_cache;

    // Stage 6 — fixture discovery.
    let fixtures = discovery::collect(&config, &crate_root, &cli.filter)?;
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

    // Parallelism cap (spec §5.2).
    let parallelism = compute_parallelism(&cli, &config);
    if !cli.quiet {
        eprintln!("lihaaf: parallelism = {parallelism}");
    }

    // Per-session temp dir.
    let session_temp = tempfile::Builder::new()
        .prefix("lihaaf-session-")
        .tempdir_in(&workspace_target)
        .map_err(|e| {
            Error::io(
                e,
                "creating session temp dir",
                Some(workspace_target.clone()),
            )
        })?;
    let session_temp_path = session_temp.path().to_path_buf();

    // Build the worker context.
    let norm_ctx = NormalizationContext::new(crate_root.clone(), toolchain.sysroot.clone());
    let mut worker_ctx = WorkerContext::new(
        crate_root.clone(),
        managed_path.clone(),
        build_out.deps_dir.clone(),
        &config,
        cli.effective_bless(),
        cli.verbose,
        cli.keep_output,
        session_temp_path.clone(),
        norm_ctx,
        &toolchain.sysroot,
    );

    // Resolve `--extern` paths for the non-dylib crates.
    let mut extra_names = worker_ctx.extra_extern_crates.clone();
    extra_names.extend(worker_ctx.dev_deps.iter().cloned());
    worker_ctx.extern_paths = worker::resolve_extern_paths(&build_out.deps_dir, &extra_names)?;

    // Mid-session toolchain drift check (spec §4.6). We compare the
    // captured rustc release against a fresh capture. Cheap; cost is
    // dwarfed by the per-fixture rustc.
    let post_capture = toolchain::capture()?;
    if !toolchain::matches(&toolchain, &post_capture) {
        return Err(Error::Session(Outcome::ToolchainDrift {
            original: toolchain.release_line.clone(),
            current: post_capture.release_line.clone(),
        }));
    }

    // Stage 7 — worker pool dispatch.
    let dispatch_start = std::time::Instant::now();
    let progress_quiet = cli.quiet;
    let results = worker::dispatch_pool(&fixtures, &worker_ctx, parallelism, move |r| {
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
        // quiet mode). Spec §7.2 mandates the LARGE_SNAPSHOT line.
        emit_fixture_warnings(r);
    });
    let wall_ms = dispatch_start.elapsed().as_millis() as u64;

    // Stage 8 — result aggregation.
    let cleanup_residue = results.iter().any(|r| r.cleanup_failure.is_some());

    // Print the aggregate report.
    print_aggregate(&results, wall_ms, cleanup_residue);

    // Preserve the per-session temp directory in two cases:
    //
    // 1. `--keep-output` is set (local-development escape hatch — spec
    //    §5.3 / §8.2 #--keep-output).
    // 2. Any fixture's per-fixture workdir cleanup failed (spec §5.3:
    //    "the session-temp parent directory is NOT removed at session
    //    end if it contains residue from a CLEANUP_FAILED fixture —
    //    leaving the residue visible is more useful than silently
    //    retrying a removal that already failed once."). Re-emitted as
    //    spec §10.2 `CLEANUP_RESIDUE` outcome's preserve-on-disk side
    //    of the contract.
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
        results,
        cleanup_residue,
        wall_ms,
    })
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

    // Spec §3.3 worked-example aggregate line. The bucketed counts in
    // `summary` above carry every verdict label; this line re-projects
    // the four buckets the spec calls out by name into the
    // adopter-friendly shape `<n> ok, <n> failed, <n> timeout, <n>
    // memory_exhausted` so CI greps and dashboards have a single fixed
    // line to anchor on regardless of which exotic verdicts a run
    // produced. `failed` aggregates everything that is neither pass
    // nor a "spec-named" bucket (timeout / memory_exhausted) — matches
    // the worked-example wording in §3.3.
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

/// Render any non-fatal warnings attached to a fixture result. Today
/// this is only `LARGE_SNAPSHOT` (spec §7.2 complexity ceiling), but
/// the emit point is generic so additional warning kinds slot in
/// without touching the dispatch loop.
///
/// Format pinned by spec §7.2:
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

/// Bucketed counts for the spec §3.3 aggregate line. Captures the four
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

/// Bucket per-fixture verdicts into the four §3.3 aggregate names.
///
/// `Ok` and `Blessed` count as `ok` (verdict-table footnote: "Treated as
/// OK for exit-code purposes"). `Timeout` and `MemoryExhausted` are
/// dedicated buckets per §3.3. Everything else (`SnapshotDiff`,
/// `SnapshotMissing`, `WorkerCrashed`, `MalformedDiagnostic`,
/// `SnapshotDiffTooLarge`, `ExpectedFailButPassed`,
/// `ExpectedPassButFailed`) folds into `failed` — the §3.3 worked
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

fn compute_parallelism(cli: &Cli, config: &config::Config) -> usize {
    let cpu_cap: usize = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1);
    let ram_cap: usize = match util::total_ram_mb() {
        Some(total) => {
            let cap = total / config.per_fixture_memory_mb as u64;
            cap.max(1) as usize
        }
        None => cpu_cap, // honest fallback when the platform doesn't expose RAM
    };
    // `cli.jobs` is guaranteed positive: the clap value parser rejects
    // `-j 0` per spec §5.2. The platform-derived `cpu_cap` and `ram_cap`
    // both clamp to >= 1 above. No defensive `max(1)` here — the spec
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
        return Ok(p.clone());
    }
    // Walk up from CWD looking for Cargo.toml.
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

#[allow(dead_code)]
fn _ensure_dir_exists(p: &Path) -> Result<(), Error> {
    std::fs::create_dir_all(p).map_err(|e| Error::io(e, "creating dir", Some(p.to_path_buf())))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    fn cfg(per_fixture_mb: u32) -> Config {
        Config {
            dylib_crate: "x".into(),
            extern_crates: vec!["x".into()],
            fixture_dirs: vec![],
            features: vec![],
            edition: "2021".into(),
            dev_deps: vec![],
            compile_fail_marker: "compile_fail".into(),
            fixture_timeout_secs: 90,
            per_fixture_memory_mb: per_fixture_mb,
            raw_metadata: toml::Value::Table(toml::map::Map::new()),
        }
    }

    #[test]
    fn parallelism_respects_explicit_jobs() {
        let mut cli = Cli {
            bless: false,
            filter: vec![],
            jobs: Some(2),
            no_cache: false,
            manifest_path: None,
            list: false,
            quiet: true,
            verbose: false,
            use_symlink: false,
            keep_output: false,
        };
        let p = compute_parallelism(&cli, &cfg(1024));
        assert!(p <= 2);
        cli.jobs = Some(1);
        let p2 = compute_parallelism(&cli, &cfg(1024));
        assert_eq!(p2, 1);
    }

    #[test]
    fn parallelism_is_at_least_one() {
        let cli = Cli {
            bless: false,
            filter: vec![],
            jobs: None,
            no_cache: false,
            manifest_path: None,
            list: false,
            quiet: true,
            verbose: false,
            use_symlink: false,
            keep_output: false,
        };
        // Even with an absurd per-fixture cap, we must not return 0.
        let p = compute_parallelism(&cli, &cfg(u32::MAX / 2));
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
}
