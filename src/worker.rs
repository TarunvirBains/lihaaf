//! Per-fixture worker dispatch (spec §5).
//!
//! ## Implementer choices recorded here
//!
//! - **Per-platform RSS sampling API** (KR-5): on Linux we read
//!   `/proc/<pid>/statm` (2nd field × page-size). This is the live
//!   per-process RSS for a running child — verified by hand on
//!   `rustc 1.95` on Linux 6.x x86_64. macOS and Windows are documented
//!   stubs (return `None`); without correct sampling those platforms
//!   would silently fail KR-5's mitigation requirement, so v0.1 turns
//!   off the RSS-ceiling check on those platforms and the OOM
//!   attribution heuristic falls back to the OS OOMkiller path
//!   (verdict: `WORKER_CRASHED`). v0.x adds proper macOS / Windows
//!   sampling.
//!
//! - **Sampling interval** (§5.4 — implementer chooses): 100 ms on
//!   Linux. Short enough to catch a runaway monomorphization before
//!   the OS OOMkiller fires (which typically takes seconds of
//!   sustained pressure on a desktop kernel), long enough that the
//!   sampler thread stays out of the worker's way.
//!
//! - **Termination signal pair** (§5.4): SIGTERM, then SIGKILL after
//!   a 2-second grace. SIGTERM lets rustc clean up its temp files;
//!   SIGKILL is the backstop for ICEs that ignore SIGTERM.
//!
//! ## Determinism (§5.7)
//!
//! Within a single invocation with `-j 1`, fixture verdicts are
//! emitted in lexicographic order. With `-j > 1`, verdicts are emitted
//! in completion order; the final aggregate report sorts them back.
//! This module's [`dispatch_serial`] is the reference path; the
//! parallel [`dispatch_pool`] uses `std::thread` with a bounded
//! channel.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use crate::config::Config;
use crate::discovery::{Fixture, FixtureKind};
use crate::error::Error;
use crate::freshness::{self, FreshnessFailure, FreshnessSnapshot};
use crate::normalize::{self, NormalizationContext};
use crate::snapshot;
use crate::verdict::{CleanupFailure, FixtureResult, FixtureWarning, MalformedSource, Verdict};

/// Per-session worker context. Shared (read-only) across all worker
/// dispatches.
#[derive(Debug, Clone)]
pub struct WorkerContext {
    /// Resolved consumer-crate root (parent of Cargo.toml).
    pub crate_root: PathBuf,
    /// Path to the lihaaf-managed dylib (spec §4.3).
    pub managed_dylib: PathBuf,
    /// `target/release/deps` containing the rest of the link tree.
    pub deps_dir: PathBuf,
    /// `dylib_crate` from the metadata.
    pub dylib_crate: String,
    /// `extern_crates` from the metadata, sans `dylib_crate` (which is
    /// linked separately via the managed dylib).
    pub extra_extern_crates: Vec<String>,
    /// `dev_deps` from the metadata.
    pub dev_deps: Vec<String>,
    /// `features` from the metadata. Each becomes a `--cfg
    /// feature="<f>"` flag.
    pub features: Vec<String>,
    /// Edition.
    pub edition: String,
    /// Per-fixture timeout in seconds.
    pub timeout_secs: u32,
    /// Per-fixture RSS ceiling in MB.
    pub memory_mb_ceiling: u32,
    /// Bless mode active?
    pub bless: bool,
    /// Verbose mode active?
    pub verbose: bool,
    /// Keep per-fixture work directories?
    pub keep_output: bool,
    /// Per-session temp directory parent (one workdir per fixture
    /// underneath).
    pub session_temp: PathBuf,
    /// Map from non-dylib extern crate name → resolved artifact path.
    /// Built by [`resolve_extern_paths`] before dispatch.
    pub extern_paths: HashMap<String, PathBuf>,
    /// Normalization context (re-used per fixture; cheap to clone).
    pub norm_ctx: NormalizationContext,
    /// Sysroot lib dir for `LD_LIBRARY_PATH`. Required when the dylib
    /// uses `-C prefer-dynamic` (it depends on `libstd.so` from the
    /// toolchain).
    pub sysroot_lib_dir: PathBuf,
    /// Snapshot of the four spec §4.5 invariants captured at session
    /// startup. Re-checked before every per-fixture dispatch via
    /// [`crate::freshness::check`]; on drift, [`dispatch_pool`] /
    /// [`dispatch_serial`] short-circuit and bubble back a
    /// [`FreshnessFailure`] for [`crate::session::run`] to convert
    /// into an `Outcome::FreshnessDrift` exit.
    pub freshness_snapshot: FreshnessSnapshot,
}

impl WorkerContext {
    /// Build a context from the validated config + dylib build output +
    /// session-startup state.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        crate_root: PathBuf,
        managed_dylib: PathBuf,
        deps_dir: PathBuf,
        config: &Config,
        bless: bool,
        verbose: bool,
        keep_output: bool,
        session_temp: PathBuf,
        norm_ctx: NormalizationContext,
        sysroot: &Path,
        freshness_snapshot: FreshnessSnapshot,
    ) -> Self {
        let extra_extern_crates = config
            .extern_crates
            .iter()
            .skip(1)
            .cloned()
            .collect::<Vec<_>>();
        Self {
            crate_root,
            managed_dylib,
            deps_dir: deps_dir.clone(),
            dylib_crate: config.dylib_crate.clone(),
            extra_extern_crates,
            dev_deps: config.dev_deps.clone(),
            features: config.features.clone(),
            edition: config.edition.clone(),
            timeout_secs: config.fixture_timeout_secs,
            memory_mb_ceiling: config.per_fixture_memory_mb,
            bless,
            verbose,
            keep_output,
            session_temp,
            extern_paths: HashMap::new(),
            norm_ctx,
            sysroot_lib_dir: sysroot.join("lib"),
            freshness_snapshot,
        }
    }
}

/// Result of a worker-pool dispatch.
///
/// Carries the per-fixture results PLUS an optional freshness failure
/// — if any fixture's per-dispatch §4.5 check fails, the pool stops
/// dispatching new work, drains the in-flight fixtures, and bubbles
/// the failure back to [`crate::session::run`] for conversion to a
/// session-level `Outcome::FreshnessDrift` exit.
#[derive(Debug)]
pub struct DispatchOutcome {
    /// Per-fixture results in deterministic (lexicographic) order.
    /// Will be a partial set if `freshness_failure` is Some.
    pub results: Vec<FixtureResult>,
    /// `Some` if a per-dispatch §4.5 invariant drifted; `None` on a
    /// clean run.
    pub freshness_failure: Option<FreshnessFailure>,
}

/// Resolve `--extern` paths for crates other than the dylib_crate.
///
/// Looks under `deps_dir` for an `.rlib` matching each name. The
/// search prefers `lib<name>-<hash>.rlib` (cargo's normal layout) and
/// falls back to `<name>-<hash>.rlib` (proc-macro crates). Multiple
/// matches → the most recently modified file wins (matches cargo's
/// own "newest artifact" rule).
pub fn resolve_extern_paths(
    deps_dir: &Path,
    crate_names: &[String],
) -> Result<HashMap<String, PathBuf>, Error> {
    let mut out = HashMap::new();
    if crate_names.is_empty() {
        return Ok(out);
    }
    let entries = std::fs::read_dir(deps_dir).map_err(|e| {
        Error::io(
            e,
            "reading deps dir for extern resolution",
            Some(deps_dir.to_path_buf()),
        )
    })?;
    let mut all_files: Vec<PathBuf> = Vec::new();
    for entry in entries {
        let entry =
            entry.map_err(|e| Error::io(e, "iterating deps dir", Some(deps_dir.to_path_buf())))?;
        let p = entry.path();
        if p.is_file() {
            all_files.push(p);
        }
    }
    for name in crate_names {
        // Cargo's lib name uses `_` for `-` in crate names.
        let normalized = name.replace('-', "_");
        let mut candidates: Vec<PathBuf> = Vec::new();
        for f in &all_files {
            let stem = match f.file_stem().and_then(|s| s.to_str()) {
                Some(s) => s,
                None => continue,
            };
            let ext = f.extension().and_then(|s| s.to_str()).unwrap_or("");
            if ext != "rlib" && ext != "so" && ext != "dylib" && ext != "dll" {
                continue;
            }
            let with_lib = format!("lib{normalized}-");
            let with_lib_no_hash = format!("lib{normalized}");
            if stem.starts_with(&with_lib) || stem == with_lib_no_hash {
                candidates.push(f.clone());
                continue;
            }
            // proc-macro crates: no `lib` prefix, e.g.
            // `consumer_macros-abc.so`.
            if stem.starts_with(&format!("{normalized}-")) || stem == normalized {
                candidates.push(f.clone());
            }
        }
        candidates.sort_by(|a, b| {
            std::fs::metadata(b)
                .and_then(|m| m.modified())
                .ok()
                .cmp(&std::fs::metadata(a).and_then(|m| m.modified()).ok())
        });
        if let Some(best) = candidates.into_iter().next() {
            out.insert(name.clone(), best);
        }
    }
    Ok(out)
}

/// Run all fixtures serially. Returns one [`FixtureResult`] per
/// fixture in the input order, plus an optional freshness failure if
/// the spec §4.5 per-dispatch check tripped on any fixture (in which
/// case the iteration short-circuits and the result vector is
/// partial).
///
/// Used both for `-j 1` and as the fallback when parallelism reduces
/// to 1 mid-session.
pub fn dispatch_serial(
    fixtures: &[Fixture],
    ctx: &WorkerContext,
    progress: impl Fn(&FixtureResult),
) -> DispatchOutcome {
    let mut results: Vec<FixtureResult> = Vec::with_capacity(fixtures.len());
    for fx in fixtures {
        // Spec §4.5: re-check the four invariants before each dispatch.
        if let Err(failure) = freshness::check(&ctx.freshness_snapshot) {
            return DispatchOutcome {
                results,
                freshness_failure: Some(failure),
            };
        }
        let r = run_one(fx, ctx);
        progress(&r);
        results.push(r);
    }
    DispatchOutcome {
        results,
        freshness_failure: None,
    }
}

/// Run all fixtures in a worker pool of up to `parallelism` threads.
/// Verdicts emit in completion order via `progress`; the returned vec
/// is sorted lexicographically by `relative_path` for the deterministic
/// final report (spec §5.7).
///
/// Each worker thread re-checks the four spec §4.5 invariants
/// (existence / mtime / SHA-256 / rustc release) before pulling its
/// next fixture from the queue. On the first detected failure, the
/// failure is recorded into a shared `Mutex<Option<FreshnessFailure>>`
/// and the queue is drained — every worker observes the failure on
/// its next loop iteration and exits without dispatching new work.
/// In-flight rustc invocations are NOT cancelled (their results are
/// reported normally); the contract is "no NEW dispatches once the
/// drift is detected." The first failure to reach the mutex wins;
/// subsequent failures (e.g., a different invariant tripped on a
/// different worker) are silently dropped, matching the §4.5 "ANY
/// divergence" framing.
pub fn dispatch_pool(
    fixtures: &[Fixture],
    ctx: &WorkerContext,
    parallelism: usize,
    progress: impl Fn(&FixtureResult) + Send + Sync + 'static,
) -> DispatchOutcome {
    if parallelism <= 1 {
        return dispatch_serial(fixtures, ctx, |r| progress(r));
    }
    let (tx, rx) = mpsc::channel::<FixtureResult>();
    let queue: Arc<Mutex<std::collections::VecDeque<Fixture>>> =
        Arc::new(Mutex::new(fixtures.iter().cloned().collect()));
    let freshness_failure: Arc<Mutex<Option<FreshnessFailure>>> = Arc::new(Mutex::new(None));
    let ctx = Arc::new(ctx.clone());
    let mut handles = Vec::with_capacity(parallelism);
    for _ in 0..parallelism {
        let q = Arc::clone(&queue);
        let c = Arc::clone(&ctx);
        let t = tx.clone();
        let ff = Arc::clone(&freshness_failure);
        let h = thread::spawn(move || {
            loop {
                // Spec §4.5: re-check before pulling. If a peer worker
                // already recorded a failure, exit promptly without
                // pulling more work.
                if ff.lock().unwrap().is_some() {
                    break;
                }
                if let Err(failure) = freshness::check(&c.freshness_snapshot) {
                    let mut slot = ff.lock().unwrap();
                    if slot.is_none() {
                        *slot = Some(failure);
                    }
                    // Drain the queue so other workers don't pull
                    // anything new after observing the failure.
                    q.lock().unwrap().clear();
                    break;
                }
                let next = {
                    let mut g = q.lock().unwrap();
                    g.pop_front()
                };
                match next {
                    Some(fx) => {
                        let r = run_one(&fx, &c);
                        if t.send(r).is_err() {
                            break;
                        }
                    }
                    None => break,
                }
            }
        });
        handles.push(h);
    }
    drop(tx);

    let mut results: Vec<FixtureResult> = Vec::with_capacity(fixtures.len());
    for r in rx {
        progress(&r);
        results.push(r);
    }
    for h in handles {
        let _ = h.join();
    }
    results.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
    let failure = freshness_failure.lock().unwrap().take();
    DispatchOutcome {
        results,
        freshness_failure: failure,
    }
}

/// Run one fixture: spawn rustc, monitor RSS + timeout, capture
/// stderr, classify, normalize, diff, optionally bless. Cleanup is
/// unconditional (spec §5.3) unless `keep_output` is set.
fn run_one(fx: &Fixture, ctx: &WorkerContext) -> FixtureResult {
    let started = Instant::now();
    let workdir = ctx.session_temp.join(fixture_workdir_name(fx));
    if let Err(e) = std::fs::create_dir_all(&workdir) {
        return FixtureResult {
            relative_path: fx.relative_path.clone(),
            verdict: Verdict::WorkerCrashed {
                cause: format!("could not create workdir: {e}"),
            },
            cleanup_failure: None,
            wall_ms: 0,
            warning: None,
        };
    }

    // First attempt.
    let outcome = spawn_and_monitor(fx, ctx, &workdir, false);
    let mut wall_ms = started.elapsed().as_millis() as u64;
    let (mut verdict, mut warning) = match outcome.kind {
        MonitorKind::Exited { ok, stderr } => classify_exit(fx, ctx, ok, &stderr),
        MonitorKind::HarnessKilledMemory => {
            // Per §5.4, retry serially once before declaring
            // MEMORY_EXHAUSTED.
            let _ = std::fs::remove_dir_all(&workdir);
            let _ = std::fs::create_dir_all(&workdir);
            let retry = spawn_and_monitor(fx, ctx, &workdir, true);
            wall_ms = started.elapsed().as_millis() as u64;
            match retry.kind {
                MonitorKind::HarnessKilledMemory => (Verdict::MemoryExhausted, None),
                MonitorKind::Exited { ok, stderr } => classify_exit(fx, ctx, ok, &stderr),
                MonitorKind::Timeout => (Verdict::Timeout, None),
                MonitorKind::ExternalKill { cause } => (Verdict::WorkerCrashed { cause }, None),
            }
        }
        MonitorKind::Timeout => (Verdict::Timeout, None),
        MonitorKind::ExternalKill { cause } => (Verdict::WorkerCrashed { cause }, None),
    };

    // Bless path: if we have a SnapshotDiff verdict and bless is on,
    // overwrite and emit Blessed. The bless transition does not affect
    // any LARGE_SNAPSHOT warning that may already be attached — the
    // warning is about the input shape, not the verdict.
    if ctx.bless {
        if let Verdict::SnapshotDiff { .. } = &verdict
            && let Some(actual) = compute_actual_normalized(fx, ctx)
            && let Ok(p) = snapshot::write(&fx.path, &actual)
        {
            verdict = Verdict::Blessed { snapshot_path: p };
        }
        if let Verdict::SnapshotMissing { actual } = &verdict
            && let Ok(p) = snapshot::write(&fx.path, actual)
        {
            verdict = Verdict::Blessed { snapshot_path: p };
            // SnapshotMissing carries no LARGE_SNAPSHOT signal — drop
            // any incidental warning the prior path may have produced
            // (in practice none is set on the SnapshotMissing branch).
            warning = None;
        }
    }

    // Cleanup. Per §5.3 + cleanup-failure policy.
    let cleanup_failure = if ctx.keep_output {
        None
    } else {
        match std::fs::remove_dir_all(&workdir) {
            Ok(()) => None,
            Err(e) => Some(CleanupFailure {
                path: workdir.clone(),
                message: e.to_string(),
            }),
        }
    };

    FixtureResult {
        relative_path: fx.relative_path.clone(),
        verdict,
        cleanup_failure,
        wall_ms,
        warning,
    }
}

/// Recompute the normalized stderr for the bless path. Returns `None`
/// if rustc surprisingly succeeded between the first run and now (the
/// adopter's source changed under our feet) or if the rustc output
/// failed UTF-8 validation — in either case we leave the verdict alone
/// rather than blessing whatever bytes happened to land.
fn compute_actual_normalized(fx: &Fixture, ctx: &WorkerContext) -> Option<String> {
    let workdir = ctx.session_temp.join(fixture_workdir_name(fx));
    let _ = std::fs::create_dir_all(&workdir);
    let outcome = spawn_and_monitor(fx, ctx, &workdir, false);
    if let MonitorKind::Exited { stderr, .. } = outcome.kind {
        // UTF-8 validation parity with `classify_exit`. A bless that
        // silently used `from_utf8_lossy` here could write a snapshot
        // file with replacement characters that no rerun would ever
        // match — refuse to bless if the diagnostic stream is malformed.
        let s = std::str::from_utf8(&stderr).ok()?;
        Some(normalize_stderr(s, fx, ctx))
    } else {
        None
    }
}

fn fixture_workdir_name(fx: &Fixture) -> String {
    let mut s = String::with_capacity(fx.relative_path.len());
    for ch in fx.relative_path.chars() {
        match ch {
            '/' | '\\' | ':' => s.push('_'),
            _ => s.push(ch),
        }
    }
    s
}

/// Classify a worker's exit into a verdict plus an optional warning.
///
/// The warning (currently only `LARGE_SNAPSHOT`) rides alongside the
/// verdict — it does not change the exit-code aggregation. See spec
/// §7.2 complexity ceiling for the soft / hard thresholds.
///
/// UTF-8 validation is performed here, exactly once, on the raw
/// rustc-emitted bytes. Spec §7.2 ("Non-UTF-8 / binary-content
/// handling"): any byte sequence that fails UTF-8 validation surfaces
/// as `MALFORMED_DIAGNOSTIC` with the precise byte offset of the first
/// invalid byte (returned by [`std::str::Utf8Error::valid_up_to`]).
/// `from_utf8_lossy` is deliberately NOT used as a fallback — the
/// malformed signal IS the verdict.
fn classify_exit(
    fx: &Fixture,
    ctx: &WorkerContext,
    ok: bool,
    stderr_bytes: &[u8],
) -> (Verdict, Option<FixtureWarning>) {
    // Spec §7.2: a non-UTF-8 byte in rustc's diagnostic stream IS the
    // verdict. We do not silently substitute replacement characters
    // and continue — that would erase the signal that adopters need
    // in order to debug whatever produced the bad bytes.
    let stderr = match std::str::from_utf8(stderr_bytes) {
        Ok(s) => s,
        Err(e) => {
            return (
                Verdict::MalformedDiagnostic {
                    byte_offset: e.valid_up_to(),
                    source: MalformedSource::RustcRendered,
                },
                None,
            );
        }
    };
    let normalized = normalize_stderr(stderr, fx, ctx);
    match (fx.kind, ok) {
        (FixtureKind::CompilePass, true) => (Verdict::Ok, None),
        (FixtureKind::CompilePass, false) => {
            (Verdict::ExpectedPassButFailed { stderr: normalized }, None)
        }
        (FixtureKind::CompileFail, true) => (Verdict::ExpectedFailButPassed, None),
        (FixtureKind::CompileFail, false) => {
            // Diff against snapshot.
            match snapshot::try_read(&fx.path) {
                Ok(snapshot::ReadOutcome::Found(expected)) => {
                    // Snapshot lines pre-normalized to LF on read; we
                    // count post-split lines on the actual side too so
                    // both numbers match what the diff algorithm sees.
                    let expected_lines = expected.lines().count();
                    let actual_lines = normalized.lines().count();
                    let result = crate::diff::unified_diff(&expected, &normalized);
                    // Capture LARGE_SNAPSHOT before consuming the
                    // result via diff_to_verdict — the `warn` flag is
                    // the only place this spec §7.2 condition surfaces.
                    let warning = match &result {
                        crate::diff::DiffResult::Diff { warn: true, .. } => {
                            Some(FixtureWarning::LargeSnapshot {
                                expected_lines,
                                actual_lines,
                            })
                        }
                        _ => None,
                    };
                    let verdict = match crate::diff::diff_to_verdict(result) {
                        Some(v) => v,
                        None => Verdict::Ok,
                    };
                    (verdict, warning)
                }
                Ok(snapshot::ReadOutcome::Missing) => {
                    (Verdict::SnapshotMissing { actual: normalized }, None)
                }
                Ok(snapshot::ReadOutcome::Malformed { byte_offset, .. }) => (
                    Verdict::MalformedDiagnostic {
                        byte_offset,
                        source: MalformedSource::Snapshot,
                    },
                    None,
                ),
                Err(_) => (
                    Verdict::MalformedDiagnostic {
                        byte_offset: 0,
                        source: MalformedSource::Snapshot,
                    },
                    None,
                ),
            }
        }
    }
}

/// Render the rustc `--error-format=json` JSON stream as plain text by
/// concatenating each diagnostic's `rendered` field, then run the
/// stderr normalizer.
fn normalize_stderr(json_stderr: &str, fx: &Fixture, ctx: &WorkerContext) -> String {
    let rendered = render_json_diagnostics(json_stderr);
    let fixture_dir = fx.path.parent().unwrap_or(Path::new("."));
    normalize::normalize(&rendered, &ctx.norm_ctx, fixture_dir)
}

/// Pull the `rendered` field out of every JSON diagnostic on stderr,
/// concatenating in input order. Lines that aren't valid JSON pass
/// through verbatim (rustc occasionally interleaves plain text).
fn render_json_diagnostics(stderr: &str) -> String {
    let mut out = String::with_capacity(stderr.len());
    for line in stderr.lines() {
        if !line.starts_with('{') {
            out.push_str(line);
            out.push('\n');
            continue;
        }
        // Manually pull "rendered":"<…>" without serde_json's full
        // parse to avoid allocating a Value tree per diagnostic line.
        if let Some(rendered) = extract_rendered(line) {
            out.push_str(&rendered);
            // rustc's `rendered` typically ends in '\n' already; we add
            // one only if missing.
            if !rendered.ends_with('\n') {
                out.push('\n');
            }
        }
    }
    out
}

/// Extract the value of `"rendered":"…"` from a JSON line. Returns
/// `None` if the field is absent or unparseable. Hand-rolled JSON-string
/// decoder; only the substring after `"rendered":` is examined.
fn extract_rendered(line: &str) -> Option<String> {
    // Find the key token. Two shapes are possible: `"rendered":"…"` and
    // `"rendered" : "…"`. We scan for the literal key including the
    // leading quote.
    let key = "\"rendered\"";
    let key_idx = line.find(key)?;
    let mut i = key_idx + key.len();
    let bytes = line.as_bytes();
    // Skip optional whitespace, then ':', whitespace, then '"'.
    while i < bytes.len() && (bytes[i] == b' ' || bytes[i] == b'\t') {
        i += 1;
    }
    if i >= bytes.len() || bytes[i] != b':' {
        return None;
    }
    i += 1;
    while i < bytes.len() && (bytes[i] == b' ' || bytes[i] == b'\t') {
        i += 1;
    }
    if i >= bytes.len() || bytes[i] != b'"' {
        // Could be `null` — return None.
        return None;
    }
    i += 1;
    let mut out = String::new();
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'"' {
            return Some(out);
        }
        if b == b'\\' && i + 1 < bytes.len() {
            let next = bytes[i + 1];
            match next {
                b'n' => out.push('\n'),
                b't' => out.push('\t'),
                b'r' => out.push('\r'),
                b'\\' => out.push('\\'),
                b'/' => out.push('/'),
                b'"' => out.push('"'),
                b'u' => {
                    if i + 5 < bytes.len() {
                        let hex = std::str::from_utf8(&bytes[i + 2..i + 6]).ok()?;
                        let code = u32::from_str_radix(hex, 16).ok()?;
                        if let Some(c) = char::from_u32(code) {
                            out.push(c);
                        }
                        i += 6;
                        continue;
                    } else {
                        return None;
                    }
                }
                _ => out.push(next as char),
            }
            i += 2;
        } else {
            // Single UTF-8 char.
            let mut j = i + 1;
            while j < bytes.len() && (bytes[j] & 0xC0) == 0x80 {
                j += 1;
            }
            out.push_str(&line[i..j]);
            i = j;
        }
    }
    None
}

/// Outcome of a single rustc spawn + monitor pass.
struct MonitorOutcome {
    kind: MonitorKind,
}

enum MonitorKind {
    /// rustc finished. `stderr` is the raw bytes — UTF-8 validation is
    /// the caller's responsibility. Spec §7.2 mandates that any
    /// non-UTF-8 sequence in the diagnostic stream surfaces as a
    /// `MALFORMED_DIAGNOSTIC` verdict with the precise byte offset of
    /// the first invalid byte; lossy decoding here would erase that
    /// signal.
    Exited {
        ok: bool,
        stderr: Vec<u8>,
    },
    HarnessKilledMemory,
    Timeout,
    ExternalKill {
        cause: String,
    },
}

fn spawn_and_monitor(
    fx: &Fixture,
    ctx: &WorkerContext,
    workdir: &Path,
    _is_retry: bool,
) -> MonitorOutcome {
    let bin_path = workdir.join(&fx.stem);
    let mut cmd = Command::new("rustc");
    cmd.arg("--edition")
        .arg(&ctx.edition)
        .arg("--crate-type=bin")
        .arg("--error-format=json")
        .arg("-C")
        .arg("prefer-dynamic")
        .arg("-o")
        .arg(&bin_path)
        .arg("-L")
        .arg(format!("dependency={}", ctx.deps_dir.display()))
        .arg("-L")
        .arg(format!("native={}", ctx.sysroot_lib_dir.display()))
        // dylib comes through as the canonical extern.
        .arg("--extern")
        .arg(format!(
            "{}={}",
            ctx.dylib_crate.replace('-', "_"),
            ctx.managed_dylib.display()
        ));

    // Other extern crates.
    for name in &ctx.extra_extern_crates {
        if let Some(path) = ctx.extern_paths.get(name) {
            cmd.arg("--extern")
                .arg(format!("{}={}", name.replace('-', "_"), path.display()));
        }
    }
    for name in &ctx.dev_deps {
        if let Some(path) = ctx.extern_paths.get(name) {
            cmd.arg("--extern")
                .arg(format!("{}={}", name.replace('-', "_"), path.display()));
        }
    }

    // Features as `--cfg feature="<f>"`.
    for feat in &ctx.features {
        cmd.arg("--cfg").arg(format!("feature=\"{feat}\""));
    }

    cmd.arg(&fx.path);

    cmd.stderr(Stdio::piped());
    cmd.stdout(Stdio::null());

    // LD_LIBRARY_PATH so the dylib's `-C prefer-dynamic` link can find
    // libstd.so. The sysroot lib dir contains
    // `rustlib/<host>/lib/libstd-<hash>.so`.
    let host_lib = ctx
        .sysroot_lib_dir
        .join(format!("rustlib/{}/lib", host_triple_or_default()));
    let mut ld_paths = std::env::var_os("LD_LIBRARY_PATH")
        .map(|s| std::env::split_paths(&s).collect::<Vec<_>>())
        .unwrap_or_default();
    ld_paths.insert(0, host_lib);
    ld_paths.insert(0, ctx.deps_dir.clone());
    if let Some(parent) = ctx.managed_dylib.parent() {
        ld_paths.insert(0, parent.to_path_buf());
    }
    if let Ok(joined) = std::env::join_paths(ld_paths) {
        cmd.env("LD_LIBRARY_PATH", joined);
    }

    if ctx.verbose {
        eprintln!("lihaaf: rustc invocation:\n  {cmd:?}");
    }

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            return MonitorOutcome {
                kind: MonitorKind::ExternalKill {
                    cause: format!("could not spawn rustc: {e}"),
                },
            };
        }
    };
    let pid = child.id();
    let stderr_handle = child.stderr.take();

    // Spawn a stderr reader on its own thread so a slow draining child
    // doesn't deadlock the monitor.
    let stderr_join: thread::JoinHandle<Vec<u8>> = thread::spawn(move || {
        if let Some(mut h) = stderr_handle {
            let mut buf = Vec::new();
            let _ = std::io::Read::read_to_end(&mut h, &mut buf);
            buf
        } else {
            Vec::new()
        }
    });

    let timeout = Duration::from_secs(ctx.timeout_secs as u64);
    let ceiling_kib = (ctx.memory_mb_ceiling as u64) * 1024;
    let start = Instant::now();
    let mut harness_killed_memory = false;

    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let stderr = stderr_join.join().unwrap_or_default();
                if harness_killed_memory {
                    return MonitorOutcome {
                        kind: MonitorKind::HarnessKilledMemory,
                    };
                }
                if status.success() {
                    return MonitorOutcome {
                        kind: MonitorKind::Exited { ok: true, stderr },
                    };
                }
                // rustc returns 1 for compilation errors, 101 for ICEs.
                // We treat everything except a signal-kill as a normal
                // exit; a signal-kill (when not harness-initiated) is
                // crash territory per §5.5.
                #[cfg(unix)]
                let signal = std::os::unix::process::ExitStatusExt::signal(&status);
                #[cfg(not(unix))]
                let signal: Option<i32> = None;
                match (status.code(), signal) {
                    (Some(1), _) => {
                        return MonitorOutcome {
                            kind: MonitorKind::Exited { ok: false, stderr },
                        };
                    }
                    (Some(0), _) => {
                        return MonitorOutcome {
                            kind: MonitorKind::Exited { ok: true, stderr },
                        };
                    }
                    (Some(code), _) => {
                        return MonitorOutcome {
                            kind: MonitorKind::ExternalKill {
                                cause: format!("exit code: {code}"),
                            },
                        };
                    }
                    (None, Some(sig)) => {
                        return MonitorOutcome {
                            kind: MonitorKind::ExternalKill {
                                cause: format!("signal: {sig}"),
                            },
                        };
                    }
                    (None, None) => {
                        return MonitorOutcome {
                            kind: MonitorKind::ExternalKill {
                                cause: "process exited without code or signal".into(),
                            },
                        };
                    }
                }
            }
            Ok(None) => {
                // Still running.
                if start.elapsed() >= timeout {
                    terminate(&mut child);
                    let _ = stderr_join.join();
                    return MonitorOutcome {
                        kind: MonitorKind::Timeout,
                    };
                }
                if let Some(rss_kib) = sample_rss_kib(pid)
                    && rss_kib > ceiling_kib
                {
                    harness_killed_memory = true;
                    terminate(&mut child);
                    // Loop back; the next try_wait will pick up the
                    // exit and we'll classify HarnessKilledMemory.
                }
                thread::sleep(Duration::from_millis(100));
            }
            Err(e) => {
                return MonitorOutcome {
                    kind: MonitorKind::ExternalKill {
                        cause: format!("wait failed: {e}"),
                    },
                };
            }
        }
    }
}

/// Best-effort host triple guess for the LD_LIBRARY_PATH sysroot
/// rustlib subdir. Falls back to the build-time host triple. The
/// canonical case is the lihaaf host = the dylib host. Adopters with
/// cross toolchains override LD_LIBRARY_PATH externally.
fn host_triple_or_default() -> &'static str {
    if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        "x86_64-unknown-linux-gnu"
    } else if cfg!(all(target_os = "linux", target_arch = "aarch64")) {
        "aarch64-unknown-linux-gnu"
    } else if cfg!(all(target_os = "macos", target_arch = "x86_64")) {
        "x86_64-apple-darwin"
    } else if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        "aarch64-apple-darwin"
    } else if cfg!(all(target_os = "windows", target_arch = "x86_64")) {
        "x86_64-pc-windows-msvc"
    } else {
        "x86_64-unknown-linux-gnu"
    }
}

/// Send SIGTERM, wait briefly, then SIGKILL.
fn terminate(child: &mut Child) {
    #[cfg(unix)]
    {
        let pid = child.id() as i32;
        // SIGTERM (15).
        unsafe {
            libc_kill(pid, 15);
        }
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline {
            match child.try_wait() {
                Ok(Some(_)) => return,
                _ => thread::sleep(Duration::from_millis(50)),
            }
        }
        // SIGKILL.
        let _ = child.kill();
    }
    #[cfg(not(unix))]
    {
        let _ = child.kill();
    }
}

/// Send a signal to a Unix process via `libc::kill`.
///
/// Wrapper kept for call-site clarity — the signal numbers (SIGTERM=15
/// at the call site above) stay close to the spec §5.4 termination
/// contract rather than scattering `libc::SIGTERM` across the module.
#[cfg(unix)]
unsafe fn libc_kill(pid: i32, sig: i32) {
    unsafe { libc::kill(pid, sig) };
}

/// Sample per-process RSS in KiB. Linux-only in v0.1; returns `None`
/// on other platforms (see KR-5 documentation at module top).
fn sample_rss_kib(pid: u32) -> Option<u64> {
    #[cfg(target_os = "linux")]
    {
        let path = format!("/proc/{pid}/statm");
        let text = std::fs::read_to_string(&path).ok()?;
        // statm: size resident shared text lib data dt — all in pages.
        let mut tokens = text.split_whitespace();
        tokens.next()?; // size
        let resident_pages: u64 = tokens.next()?.parse().ok()?;
        // Page size: assume 4096 unless overridden via env (testing).
        let page_kib = page_size_kib();
        Some(resident_pages * page_kib)
    }
    #[cfg(not(target_os = "linux"))]
    {
        // KR-5: per-platform sampling is implementer's responsibility.
        // We disable the ceiling check on platforms where we don't have
        // a verified live-RSS API. The OS OOMkiller still backs us up;
        // a runaway worker surfaces as WORKER_CRASHED rather than
        // MEMORY_EXHAUSTED. Documented in the worker module's preamble.
        let _ = pid;
        None
    }
}

#[cfg(target_os = "linux")]
fn page_size_kib() -> u64 {
    // Most Linux platforms run a 4 KiB page. ARM64 servers occasionally
    // use 16 KiB or 64 KiB. We read the live value via `libc::sysconf`,
    // falling back to 4 if anything goes wrong.
    let raw = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    if raw <= 0 { 4 } else { (raw as u64) / 1024 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_rendered_basic_message() {
        let line = r#"{"reason":"compiler-message","message":{"rendered":"error: oops\n  --> foo.rs:1:1\n"}}"#;
        let r = extract_rendered(line).unwrap();
        assert!(r.starts_with("error: oops"));
    }

    #[test]
    fn extract_rendered_handles_escape_sequences() {
        let line = r#"{"rendered":"a\\nb\nc"}"#;
        let r = extract_rendered(line).unwrap();
        assert_eq!(r, "a\\nb\nc");
    }

    #[test]
    fn extract_rendered_returns_none_on_null() {
        let line = r#"{"rendered":null}"#;
        assert!(extract_rendered(line).is_none());
    }

    #[test]
    fn extract_rendered_returns_none_when_field_absent() {
        let line = r#"{"message":"no rendered key"}"#;
        assert!(extract_rendered(line).is_none());
    }

    #[test]
    fn render_json_diagnostics_concatenates_in_order() {
        let stderr = "\
{\"rendered\":\"error: alpha\\n\"}
{\"rendered\":\"error: beta\\n\"}
plain text line
";
        let out = render_json_diagnostics(stderr);
        assert!(out.contains("error: alpha"));
        assert!(out.contains("error: beta"));
        assert!(out.contains("plain text line"));
    }

    /// Minimal `WorkerContext` for unit tests of pure-function logic
    /// (e.g., `classify_exit`). Synthetic paths; no rustc spawn.
    fn unit_test_ctx() -> WorkerContext {
        WorkerContext {
            crate_root: PathBuf::from("/p"),
            managed_dylib: PathBuf::from("/p/target/lihaaf/lib.so"),
            deps_dir: PathBuf::from("/p/target/release/deps"),
            dylib_crate: "consumer".into(),
            extra_extern_crates: vec![],
            dev_deps: vec![],
            features: vec![],
            edition: "2021".into(),
            timeout_secs: 90,
            memory_mb_ceiling: 1024,
            bless: false,
            verbose: false,
            keep_output: false,
            session_temp: PathBuf::from("/tmp/lihaaf-session"),
            extern_paths: HashMap::new(),
            norm_ctx: NormalizationContext {
                workspace_root: PathBuf::from("/p"),
                sysroot: PathBuf::from("/r"),
                cargo_registry: None,
            },
            sysroot_lib_dir: PathBuf::from("/r/lib"),
            freshness_snapshot: FreshnessSnapshot {
                managed_dylib_path: PathBuf::from("/p/target/lihaaf/lib.so"),
                original_mtime_unix_secs: 0,
                original_sha256: "0".repeat(64),
                original_rustc_release_line: "rustc 1.95.0 (test 2026-01-01)".into(),
            },
        }
    }

    #[test]
    fn classify_exit_emits_malformed_diagnostic_with_correct_offset() {
        // Spec §7.2: a non-UTF-8 byte in rustc's stderr surfaces as
        // MALFORMED_DIAGNOSTIC with the precise byte offset of the
        // first invalid byte. We construct a minimal WorkerContext
        // (no rustc spawn — classify_exit is pure given its inputs)
        // and feed it bytes containing `0xFE` after 3 valid prefix
        // bytes. Expected: byte_offset == 3.
        let ctx = unit_test_ctx();
        let fx = Fixture {
            path: PathBuf::from("/p/tests/lihaaf/compile_fail/foo.rs"),
            relative_path: "tests/lihaaf/compile_fail/foo.rs".into(),
            stem: "foo".into(),
            kind: FixtureKind::CompileFail,
        };
        let mut bytes: Vec<u8> = b"abc".to_vec();
        bytes.push(0xFE);
        bytes.extend_from_slice(b"def");
        let (verdict, warning) = classify_exit(&fx, &ctx, false, &bytes);
        assert!(warning.is_none(), "malformed input has no warning");
        match verdict {
            Verdict::MalformedDiagnostic {
                byte_offset,
                source,
            } => {
                assert_eq!(byte_offset, 3, "first invalid byte is at offset 3");
                assert!(matches!(source, MalformedSource::RustcRendered));
            }
            other => panic!("expected MalformedDiagnostic, got {other:?}"),
        }
    }

    #[test]
    fn classify_exit_malformed_at_offset_zero_when_first_byte_invalid() {
        let ctx = unit_test_ctx();
        let fx = Fixture {
            path: PathBuf::from("/p/tests/lihaaf/compile_fail/x.rs"),
            relative_path: "tests/lihaaf/compile_fail/x.rs".into(),
            stem: "x".into(),
            kind: FixtureKind::CompileFail,
        };
        let bytes = vec![0xFE];
        let (verdict, _) = classify_exit(&fx, &ctx, false, &bytes);
        match verdict {
            Verdict::MalformedDiagnostic { byte_offset, .. } => assert_eq!(byte_offset, 0),
            other => panic!("expected MalformedDiagnostic, got {other:?}"),
        }
    }

    #[test]
    fn fixture_workdir_name_replaces_separators() {
        let fx = Fixture {
            path: PathBuf::from("/p/tests/lihaaf/compile_fail/foo.rs"),
            relative_path: "tests/lihaaf/compile_fail/foo.rs".into(),
            stem: "foo".into(),
            kind: FixtureKind::CompileFail,
        };
        let n = fixture_workdir_name(&fx);
        assert_eq!(n, "tests_lihaaf_compile_fail_foo.rs");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn sample_rss_returns_some_for_self() {
        let pid = std::process::id();
        let kib = sample_rss_kib(pid);
        assert!(kib.is_some(), "self RSS must be readable on Linux");
        assert!(kib.unwrap() > 0);
    }
}
