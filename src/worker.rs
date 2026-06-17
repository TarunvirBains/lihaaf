//! Per-fixture worker dispatch.
//!
//! ## Dispatch choices worth calling out
//!
//! - **RSS sampling (Linux / macOS / Windows):** per-platform live RSS
//!   is sampled for running worker children via the platform's native API.
//!   Linux reads `/proc/<pid>/statm` (2nd field × page-size); macOS calls
//!   `libc::proc_pidinfo(PROC_PIDTASKINFO)` and reads `pti_resident_size`;
//!   Windows calls `OpenProcess` + `GetProcessMemoryInfo` and reads
//!   `WorkingSetSize`. On other Unixes (FreeBSD, NetBSD, etc.) the
//!   sampler returns `None` and the OS OOMkiller backstops, with runaway
//!   workers surfacing as `WORKER_CRASHED` rather than `MEMORY_EXHAUSTED`.
//!
//! - **Sampling interval:** 100 ms on Linux. This is short enough to catch
//!   runaway work before the OS OOMkiller usually triggers, while staying
//!   out of the worker's critical path.
//!
//! - **Termination signal pair:** SIGTERM, then SIGKILL after
//!   a 2-second grace. SIGTERM gives rustc a chance to clean up, while
//!   SIGKILL is the hard fallback.
//!
//! ## Determinism
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
use std::sync::{Arc, Condvar, Mutex, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use crate::config::Suite;
use crate::discovery::{Fixture, FixtureKind};
use crate::error::Error;
use crate::freshness::{self, FreshnessFailure, FreshnessSnapshot};
use crate::normalize::{self, NormalizationContext};
use crate::snapshot;
use crate::util;
use crate::verdict::{CleanupFailure, FixtureResult, FixtureWarning, MalformedSource, Verdict};

/// Per-session worker context. Shared (read-only) across all worker
/// dispatches.
#[derive(Debug, Clone)]
pub struct WorkerContext {
    /// Resolved consumer-crate root (parent of Cargo.toml).
    pub crate_root: PathBuf,
    /// Path to the lihaaf-managed dylib (the policy).
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
    /// `allow_lints` from the suite. Each becomes a `-A <lint>` flag on
    /// the per-fixture rustc invocation.
    pub allow_lints: Vec<String>,
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
    /// Snapshot of the four the policy invariants captured at session
    /// startup. Re-checked before every per-fixture dispatch via
    /// [`crate::freshness::check`]; on drift, [`dispatch_pool`] /
    /// [`dispatch_serial`] short-circuit and bubble back a
    /// [`FreshnessFailure`] for [`crate::session::run`] to convert
    /// into an `Outcome::FreshnessDrift` exit.
    pub freshness_snapshot: FreshnessSnapshot,
}

impl WorkerContext {
    /// Build a context from the validated suite + global dylib_crate +
    /// dylib build output + session-startup state.
    ///
    /// `dylib_crate` is owned by the top-level [`Config`] (one per
    /// session), not the suite — see `config::Config` rustdoc for why
    /// `dylib_crate` is intentionally NOT a per-suite key. Everything
    /// else (`features`, `extern_crates`, `edition`, `dev_deps`,
    /// timeout, memory ceiling, `allow_lints`, `extra_substitutions`,
    /// `strip_lines`, `strip_line_prefixes`) is per-suite and read
    /// from `suite`.
    ///
    /// The three adopter-defined normalizer override fields
    /// (`extra_substitutions`, `strip_lines`, `strip_line_prefixes`)
    /// are forwarded into `norm_ctx` here via the dedicated builders
    /// so the caller's `NormalizationContext` arrives carrying the
    /// suite's per-suite override values. When the suite has all
    /// three at default (empty), the builders are no-ops and no
    /// adopter overrides are applied.
    ///
    /// [`Config`]: crate::config::Config
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        crate_root: PathBuf,
        managed_dylib: PathBuf,
        deps_dir: PathBuf,
        dylib_crate: &str,
        suite: &Suite,
        bless: bool,
        verbose: bool,
        keep_output: bool,
        session_temp: PathBuf,
        norm_ctx: NormalizationContext,
        sysroot: &Path,
        freshness_snapshot: FreshnessSnapshot,
    ) -> Self {
        let extra_extern_crates = suite
            .extern_crates
            .iter()
            .skip(1)
            .cloned()
            .collect::<Vec<_>>();
        let norm_ctx = norm_ctx
            .with_extra_substitutions(suite.extra_substitutions.clone())
            .with_strip_lines(
                suite
                    .strip_lines
                    .iter()
                    .map(|p| p.as_str().to_owned())
                    .collect(),
            )
            .with_strip_line_prefixes(
                suite
                    .strip_line_prefixes
                    .iter()
                    .map(|p| p.as_str().to_owned())
                    .collect(),
            );
        Self {
            crate_root,
            managed_dylib,
            deps_dir: deps_dir.clone(),
            dylib_crate: dylib_crate.to_string(),
            extra_extern_crates,
            dev_deps: suite.dev_deps.clone(),
            features: suite.features.clone(),
            allow_lints: suite.allow_lints.clone(),
            edition: suite.edition.clone(),
            timeout_secs: suite.fixture_timeout_secs,
            memory_mb_ceiling: suite.per_fixture_memory_mb,
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
/// — if any fixture's per-dispatch the policy check fails, the pool stops
/// dispatching new work, drains the in-flight fixtures, and bubbles
/// the failure back to [`crate::session::run`] for conversion to a
/// session-level `Outcome::FreshnessDrift` exit.
#[derive(Debug)]
pub struct DispatchOutcome {
    /// Per-fixture results in deterministic (lexicographic) order.
    /// Will be a partial set if `freshness_failure` is Some.
    pub results: Vec<FixtureResult>,
    /// `Some` if a per-dispatch the policy invariant drifted; `None` on a
    /// clean run.
    pub freshness_failure: Option<FreshnessFailure>,
}

/// Permit pool for the policy dynamic-parallelism rule.
///
/// the policy: "If RSS exceeds `per_fixture_memory_mb`, the worker is
/// terminated … The fixture is marked needs-retry; parallelism is
/// dynamically reduced (floor: 1); the fixture re-dispatches
/// serially. If the serial retry also OOMs … the verdict is
/// `MEMORY_EXHAUSTED` and the run continues at the reduced
/// parallelism."
///
/// The gate is a counted semaphore. Workers acquire a permit before
/// pulling a fixture; release after running it. On a harness-initiated
/// OOM kill, the worker calls [`Self::reduce`] which permanently
/// removes one permit from the pool (down to a floor of 1). After the
/// reduction, peer workers that try to acquire find no permit
/// available and block on the condvar; once enough workers have
/// finished their current task, the pool runs at the new (lower) cap.
///
/// Why a gate rather than killing extra worker threads: live worker
/// threads holding permits can be in the middle of a fixture, and
/// killing the OS thread mid-rustc would orphan a child process.
/// Letting them complete naturally and re-blocking on permit
/// acquisition is cheap and correct.
#[derive(Debug)]
struct ParallelismGate {
    inner: Mutex<GateInner>,
    cv: Condvar,
}

#[derive(Debug)]
struct GateInner {
    /// Permits currently available for acquisition. Starts at `cap`,
    /// decreases on `acquire`, increases on `release`, and decreases
    /// permanently on `reduce` (which also drops `cap`).
    available: usize,
    /// Current parallelism cap. Permanently reduced by `reduce` on
    /// OOM, never increased. Floor: 1.
    cap: usize,
    /// True once the producer (dispatch loop) closes the gate, so
    /// blocked workers wake up and exit.
    closed: bool,
}

impl ParallelismGate {
    /// Create a gate with `n` initial permits. `n.max(1)` enforces the
    /// floor at construction time.
    fn new(n: usize) -> Self {
        let n = n.max(1);
        Self {
            inner: Mutex::new(GateInner {
                available: n,
                cap: n,
                closed: false,
            }),
            cv: Condvar::new(),
        }
    }

    /// Acquire a permit, blocking if none is available. Returns
    /// `false` if the gate has been closed (the dispatch loop is
    /// shutting down). The caller must call [`Self::release`] after
    /// each successful acquire.
    fn acquire(&self) -> bool {
        let mut g = self.inner.lock().unwrap();
        loop {
            if g.closed {
                return false;
            }
            if g.available > 0 {
                g.available -= 1;
                return true;
            }
            g = self.cv.wait(g).unwrap();
        }
    }

    /// Release a permit. Wakes one waiting acquirer.
    fn release(&self) {
        let mut g = self.inner.lock().unwrap();
        // Only credit the release if it doesn't push `available` past
        // `cap`. A `reduce` between this acquire/release pair burns the
        // permit; it is not restored.
        if g.available < g.cap {
            g.available += 1;
            self.cv.notify_one();
        }
    }

    /// Permanently reduce the cap by 1 (floor: 1). the policy: "first
    /// OOM, parallelism drops by 1 (floor: 1) for ALL subsequent
    /// dispatches."
    ///
    /// Returns the new cap after the reduction. Idempotent at the
    /// floor — calling `reduce` on a gate already at cap=1 is a no-op
    /// and returns 1.
    fn reduce(&self) -> usize {
        let mut g = self.inner.lock().unwrap();
        if g.cap > 1 {
            g.cap -= 1;
            // The permit count tracks `available` independently of
            // `cap`; if there's an unreleased permit at the moment of
            // reduction (i.e., a worker is still running its fixture),
            // `available` is left alone — `release` will refuse to
            // credit it back if it would exceed `cap`.
            if g.available > g.cap {
                g.available = g.cap;
            }
            self.cv.notify_all();
        }
        g.cap
    }

    /// Close the gate. Wakes all blocked acquirers; subsequent
    /// `acquire` calls return `false` immediately.
    fn close(&self) {
        let mut g = self.inner.lock().unwrap();
        g.closed = true;
        self.cv.notify_all();
    }

    /// Snapshot current cap, for tests + diagnostics.
    #[cfg(test)]
    fn current_cap(&self) -> usize {
        self.inner.lock().unwrap().cap
    }
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
    let deps_dir_real = deps_dir.canonicalize().map_err(|e| {
        Error::io(
            e,
            "canonicalizing deps dir for extern resolution",
            Some(deps_dir.to_path_buf()),
        )
    })?;
    let entries = std::fs::read_dir(&deps_dir_real).map_err(|e| {
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
        let file_type = match entry.file_type() {
            Ok(t) => t,
            Err(_) => continue,
        };
        if !file_type.is_file() {
            continue;
        }
        let p = entry.path();
        let p_real = match util::canonicalize_within_base(&deps_dir_real, &p) {
            Ok(cp) => cp,
            Err(_) => continue,
        };
        all_files.push(p_real);
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
/// the policy per-dispatch check tripped on any fixture (in which
/// case the iteration short-circuits and the result vector is
/// partial).
///
/// Used both for `-j 1` and as the fallback when parallelism reduces
/// to 1 mid-session. Serial mode is already at the policy floor;
/// OOM events still observe the retry path inside `run_one` but the
/// "reduce parallelism by 1" rule is a no-op at the floor.
pub fn dispatch_serial(
    fixtures: &[Fixture],
    ctx: &WorkerContext,
    progress: impl Fn(&FixtureResult),
) -> DispatchOutcome {
    let mut results: Vec<FixtureResult> = Vec::with_capacity(fixtures.len());
    for fx in fixtures {
        // the policy: re-check the four invariants before each dispatch.
        if let Err(failure) = freshness::check(&ctx.freshness_snapshot) {
            return DispatchOutcome {
                results,
                freshness_failure: Some(failure),
            };
        }
        let outcome = run_one(fx, ctx);
        progress(&outcome.result);
        results.push(outcome.result);
    }
    DispatchOutcome {
        results,
        freshness_failure: None,
    }
}

/// Run all fixtures in a worker pool of up to `parallelism` threads.
/// Verdicts emit in completion order via `progress`; the returned vec
/// is sorted lexicographically by `relative_path` for the deterministic
/// final report (the policy).
///
/// Each worker thread re-checks the four the policy invariants
/// (existence / mtime / SHA-256 / rustc release) before pulling its
/// next fixture from the queue. On the first detected failure, the
/// failure is recorded into a shared `Mutex<Option<FreshnessFailure>>`
/// and the queue is drained — every worker observes the failure on
/// its next loop iteration and exits without dispatching new work.
/// In-flight rustc invocations are NOT cancelled (their results are
/// reported normally); the contract is "no NEW dispatches once the
/// drift is detected."
///
/// the policy dynamic-parallelism reduction: the pool is governed by an
/// internal permit gate. On every harness-attributed OOM kill (the
/// worker observes a harness-initiated kill on the initial attempt —
/// NOT external OS OOMkills, which surface as `WORKER_CRASHED` per
/// the policy attribution heuristic), the gate's cap drops by 1
/// permanently (floor: 1). Subsequent dispatches across all workers
/// run at the reduced cap. The reduction is on the FIRST OOM, not
/// the double-OOM `MEMORY_EXHAUSTED` case — this matches
/// "parallelism is dynamically reduced (floor: 1); the fixture
/// re-dispatches serially."
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
    let gate = Arc::new(ParallelismGate::new(parallelism));
    let ctx = Arc::new(ctx.clone());
    let mut handles = Vec::with_capacity(parallelism);
    for _ in 0..parallelism {
        let q = Arc::clone(&queue);
        let c = Arc::clone(&ctx);
        let t = tx.clone();
        let ff = Arc::clone(&freshness_failure);
        let g = Arc::clone(&gate);
        let h = thread::spawn(move || {
            loop {
                // Permit acquisition is the dynamic parallelism gate.
                // If the cap shrank below the number of running
                // workers, this acquire blocks until peers finish
                // their current task; if the gate closes (the
                // dispatch loop is shutting down), exit cleanly.
                if !g.acquire() {
                    break;
                }
                // the policy: re-check before pulling. If a peer worker
                // already recorded a failure, exit promptly without
                // pulling more work.
                if ff.lock().unwrap().is_some() {
                    g.release();
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
                    g.release();
                    break;
                }
                let next = {
                    let mut g_q = q.lock().unwrap();
                    g_q.pop_front()
                };
                match next {
                    Some(fx) => {
                        let outcome = run_one(&fx, &c);
                        // the policy: every harness-attributed OOM
                        // event reduces the cap by 1, on every
                        // subsequent dispatch across all workers.
                        if outcome.harness_oom_observed {
                            g.reduce();
                        }
                        if t.send(outcome.result).is_err() {
                            g.release();
                            break;
                        }
                        g.release();
                    }
                    None => {
                        g.release();
                        break;
                    }
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
    // Wake any worker still blocked on permit acquisition (e.g., a
    // worker that observed `g.cap` shrink below the number of live
    // threads and was waiting for a permit that will never come).
    gate.close();
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

/// Per-fixture run outcome — the published `FixtureResult` plus an
/// internal `harness_oom_observed` flag the dispatch loop consumes to
/// drive the policy dynamic-parallelism reduction.
struct RunOneOutcome {
    result: FixtureResult,
    /// True iff the harness initiated an OOM kill on this fixture (on
    /// either the initial attempt or the serial retry). the policy
    /// requires parallelism to drop by 1 (floor: 1) on EVERY OOM-
    /// attributed kill, not just on `MEMORY_EXHAUSTED` (the "double
    /// OOM" case). External kills (OS OOMkiller, signal from outside,
    /// etc.) do NOT set this flag — they surface as `WORKER_CRASHED`
    /// per the OOM-attribution heuristic in the policy.
    harness_oom_observed: bool,
}

/// Run one fixture: spawn rustc, monitor RSS + timeout, capture
/// stderr, classify, normalize, diff, optionally bless. Cleanup is
/// unconditional (the policy) unless `keep_output` is set.
fn run_one(fx: &Fixture, ctx: &WorkerContext) -> RunOneOutcome {
    let started = Instant::now();
    let workdir = ctx.session_temp.join(fixture_workdir_name(fx));
    if let Err(e) = std::fs::create_dir_all(&workdir) {
        return RunOneOutcome {
            result: FixtureResult {
                relative_path: fx.relative_path.clone(),
                verdict: Verdict::WorkerCrashed {
                    cause: format!("could not create workdir: {e}"),
                },
                cleanup_failure: None,
                wall_ms: 0,
                warning: None,
            },
            harness_oom_observed: false,
        };
    }

    // First attempt.
    let outcome = spawn_and_monitor(fx, ctx, &workdir, false);
    let mut wall_ms = started.elapsed().as_millis() as u64;
    let mut harness_oom_observed = false;
    let (mut verdict, mut warning) = match outcome.kind {
        MonitorKind::Exited { ok, stderr } => classify_exit(fx, ctx, ok, &stderr),
        MonitorKind::HarnessKilledMemory => {
            // the policy first OOM. Mark the OOM observation BEFORE
            // the retry so the dispatch loop reduces parallelism
            // regardless of whether the retry succeeds, fails, or
            // double-OOMs. The reduction is on the first OOM, not the
            // double-OOM case.
            harness_oom_observed = true;
            // Per the policy, retry serially once before declaring
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

    // Bless path: a SnapshotDiff verdict with bless on causes
    // overwrite and Blessed emission. The bless transition does not affect
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

    // Cleanup. Per the policy + cleanup-failure policy.
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

    RunOneOutcome {
        result: FixtureResult {
            relative_path: fx.relative_path.clone(),
            verdict,
            cleanup_failure,
            wall_ms,
            warning,
        },
        harness_oom_observed,
    }
}

/// Recompute the normalized stderr for the bless path. Returns `None`
/// if rustc surprisingly succeeded between the first run and now (the
/// adopter's source changed between runs) or if the rustc output
/// failed UTF-8 validation — in either case the verdict is left alone
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
/// verdict — it does not change the exit-code aggregation. See
/// the policy complexity ceiling for the soft / hard thresholds.
///
/// UTF-8 validation is performed here, exactly once, on the raw
/// rustc-emitted bytes. the policy ("Non-UTF-8 / binary-content
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
    // the policy: a non-UTF-8 byte in rustc's diagnostic stream IS the
    // verdict. Silently substituting replacement characters and
    // continuing is refused — that would erase the signal adopters
    // need to debug whatever produced the bad bytes.
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
                    // Snapshot lines pre-normalized to LF on read; post-split
                    // lines are counted on the actual side too so both
                    // numbers match what the diff algorithm sees.
                    let expected_lines = expected.lines().count();
                    let actual_lines = normalized.lines().count();
                    let result = crate::diff::unified_diff(&expected, &normalized);
                    // Capture LARGE_SNAPSHOT before consuming the
                    // result via diff_to_verdict — the `warn` flag is
                    // the only place this the policy condition surfaces.
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
        if let Some(rendered) = extract_rendered(line) {
            out.push_str(&rendered);
            // rustc's `rendered` typically ends in '\n' already; one
            // is added only if missing.
            if !rendered.ends_with('\n') {
                out.push('\n');
            }
        }
    }
    out
}

/// Extract the value of the OUTERMOST `"rendered"` string field from a
/// JSON line. Returns `None` if the field is absent, `null`, or
/// non-string, or if the line is not valid JSON.
///
/// rustc emits each diagnostic as one JSON object. That object's
/// `message` contains a `children` array (help/note sub-diagnostics,
/// each carrying its own `rendered` field that is typically `null`)
/// AND a top-level `rendered` string. Both the rustc-direct shape
/// (`{"$message_type":"diagnostic","rendered":"…",…}`) and the
/// cargo-wrapped shape
/// (`{"reason":"compiler-message","message":{"rendered":"…",…}}`)
/// are accepted; the rustc direct form is what `cargo-lihaaf` itself
/// produces (see `spawn_and_monitor`) but adopters may pipe in either.
///
/// Implementation uses `serde_json` (already a hard dep via
/// `manifest.json`) rather than a hand-rolled substring scan — the
/// previous scan found the first `"rendered"` token textually and so
/// latched onto a `null` from `children[0]` whenever children were
/// serialized before the parent's own `rendered`, silently dropping
/// the parent diagnostic.
///
/// Spec §6.1's no-regex rule is preserved: `serde_json` is not a
/// regex engine.
fn extract_rendered(line: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(line).ok()?;
    // Cargo `--message-format=json` wrap: the diagnostic object is
    // nested under `message`, and `message.rendered` is the parent
    // rendered text. Note that this branch never matches the bare
    // rustc shape because in that shape `message` is a string
    // ("error[E…]: …"), not an object — `Value::get` on a string
    // returns `None`.
    if let Some(s) = v
        .get("message")
        .and_then(|m| m.get("rendered"))
        .and_then(serde_json::Value::as_str)
    {
        return Some(s.to_owned());
    }
    // Bare rustc `--error-format=json` form (and the "single object
    // with `rendered` at the top level" shape that existing tests cover).
    v.get("rendered")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
}

/// Outcome of a single rustc spawn + monitor pass.
struct MonitorOutcome {
    kind: MonitorKind,
}

enum MonitorKind {
    /// rustc finished. `stderr` is the raw bytes — UTF-8 validation is
    /// the caller's responsibility. the policy mandates that any
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

/// Apply the per-fixture rustc env block to `cmd`.
///
/// Two variables are propagated:
///
/// * **`LD_LIBRARY_PATH`** — the dylib is linked with `-C
///   prefer-dynamic`, so the loader needs the directory holding the
///   managed dylib, `target/release/deps`, and the sysroot's
///   `rustlib/<host>/lib` (for `libstd-<hash>.so`). Pre-existing
///   `LD_LIBRARY_PATH` entries are preserved at the back so the parent
///   process's env is not lost.
///
/// * **`CARGO_MANIFEST_DIR`** — proc macros that call
///   [`proc_macro_crate::crate_name`] read this at expansion time to
///   locate the consumer's `Cargo.toml` and resolve renamed
///   dependencies. `cargo` sets it automatically during `cargo build` /
///   `cargo test`, but because lihaaf's per-fixture rustc invocations
///   bypass cargo, it must be supplied explicitly here. Without it the
///   macro fails with `` `CARGO_MANIFEST_DIR` env variable not set ``
///   before the compile-fail / compile-pass assertion can be evaluated.
///   The consumer crate root (the directory containing the consumer's
///   `Cargo.toml`, resolved in [`crate::session::run`]) is the value
///   cargo would set; it is already tracked in `ctx.crate_root`.
///
/// Extracted as a discrete helper so the regression test drives the
/// same code path as production rather than asserting against a
/// hand-rolled copy — removing or mutating either `cmd.env` call
/// below trips the unit test.
///
/// [`proc_macro_crate::crate_name`]: https://docs.rs/proc-macro-crate/latest/proc_macro_crate/fn.crate_name.html
fn apply_rustc_env(cmd: &mut Command, ctx: &WorkerContext) {
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

    // CARGO_MANIFEST_DIR — see function-level rustdoc.
    cmd.env("CARGO_MANIFEST_DIR", &ctx.crate_root);
}

fn apply_feature_cfgs(cmd: &mut Command, features: &[String]) {
    for feat in features {
        cmd.arg("--cfg").arg(format!("feature=\"{feat}\""));
    }
}

/// Append `-A <lint>` flag pairs for every entry in `lints`.
///
/// Each lint is forwarded verbatim as a single argv token via
/// `Command::arg` — no shell expansion occurs. The function is
/// a no-op when `lints` is empty.
///
/// Placement in `spawn_and_monitor`: called after `apply_feature_cfgs`
/// and before `cmd.arg(&fx.path)`, so `-A` flags come after `--cfg`
/// entries and before the source file — matching the existing pattern
/// of "build up all flags, then the source file last."
fn apply_allow_lints(cmd: &mut Command, lints: &[String]) {
    for lint in lints {
        cmd.arg("-A").arg(lint);
    }
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

    // Features as `--cfg feature="<f>"`. Extracted helper so the
    // multi-suite regression guard (the `suite_demo` fixture and
    // `tests/lihaaf_fixture_shape.rs`) drives the same code path as
    // production — removing the loop, dropping the `--cfg` arg shape,
    // or stripping the quoted-feature-name format trips the
    // `apply_feature_cfgs_*` unit tests without needing to spawn rustc.
    apply_feature_cfgs(&mut cmd, &ctx.features);
    // lint suppressions as `-A <lint>` pairs. Placement: after feature
    // cfgs and before the source file, matching the "all flags then
    // source file last" ordering. Trips the `apply_allow_lints_*` unit
    // tests when absent — ensuring this call is not accidentally dropped.
    apply_allow_lints(&mut cmd, &ctx.allow_lints);

    cmd.arg(&fx.path);

    cmd.stderr(Stdio::piped());
    cmd.stdout(Stdio::null());

    apply_rustc_env(&mut cmd, ctx);

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
                // Everything except a signal-kill is treated as a normal
                // exit; a signal-kill (when not harness-initiated) is
                // crash territory per the policy.
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
                    // exit; the next try_wait will classify HarnessKilledMemory.
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
/// at the call site above) stay close to the policy termination
/// contract rather than scattering `libc::SIGTERM` across the module.
#[cfg(unix)]
unsafe fn libc_kill(pid: i32, sig: i32) {
    unsafe { libc::kill(pid, sig) };
}

/// Sample per-process RSS in KiB. Dispatches to the platform-native
/// implementation. Returns `None` on unsupported platforms (FreeBSD,
/// NetBSD, etc.) — the OS OOMkiller backstops and the worker surfaces
/// as `WORKER_CRASHED` rather than `MEMORY_EXHAUSTED` on those targets.
fn sample_rss_kib(pid: u32) -> Option<u64> {
    #[cfg(target_os = "linux")]
    {
        sample_rss_kib_linux(pid)
    }
    #[cfg(target_os = "macos")]
    {
        sample_rss_kib_macos(pid)
    }
    #[cfg(windows)]
    {
        sample_rss_kib_windows(pid)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    {
        // Other Unixes (FreeBSD, NetBSD, OpenBSD, Solaris, etc.) —
        // still unsupported. The OS OOMkiller backstops; runaway worker
        // surfaces as WORKER_CRASHED, not MEMORY_EXHAUSTED.
        let _ = pid;
        None
    }
}

#[cfg(target_os = "linux")]
fn sample_rss_kib_linux(pid: u32) -> Option<u64> {
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

#[cfg(target_os = "linux")]
fn page_size_kib() -> u64 {
    // Most Linux platforms run a 4 KiB page. ARM64 servers occasionally
    // use 16 KiB or 64 KiB. The live value is read via `libc::sysconf`,
    // falling back to 4 if anything goes wrong.
    let raw = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    if raw <= 0 { 4 } else { (raw as u64) / 1024 }
}

#[cfg(target_os = "macos")]
fn sample_rss_kib_macos(pid: u32) -> Option<u64> {
    use std::mem::{MaybeUninit, size_of};
    let mut info: MaybeUninit<libc::proc_taskinfo> = MaybeUninit::zeroed();
    let size = size_of::<libc::proc_taskinfo>() as i32;
    // SAFETY: libc::proc_pidinfo writes into info; size matches the buffer.
    // On success returns the number of bytes written (== size); on failure
    // returns -1 with errno set. We treat anything other than `size` as
    // unsamplable and return None.
    let rc = unsafe {
        libc::proc_pidinfo(
            pid as i32,
            libc::PROC_PIDTASKINFO,
            0,
            info.as_mut_ptr().cast(),
            size,
        )
    };
    if rc != size {
        return None;
    }
    // SAFETY: rc == size implies proc_pidinfo fully populated the buffer.
    let info = unsafe { info.assume_init() };
    Some(info.pti_resident_size / 1024)
}

#[cfg(windows)]
fn sample_rss_kib_windows(pid: u32) -> Option<u64> {
    use std::mem::{MaybeUninit, size_of};
    use windows_sys::Win32::Foundation::{CloseHandle, FALSE};
    use windows_sys::Win32::System::ProcessStatus::{
        GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS,
    };
    use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};

    // SAFETY: pid is u32 (PID type on Windows is DWORD); OpenProcess
    // returns a null handle on failure for this access mask, which we
    // check below. `windows-sys` 0.59 types `HANDLE` as a raw pointer
    // (`*mut c_void`), not an integer, so we must use `.is_null()`
    // rather than integer comparisons; same pattern as Spec B's
    // `src/lock.rs` (which uses `as _` casts for the FFI).
    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, FALSE, pid) };
    if handle.is_null() {
        return None;
    }

    // `cb` MUST be set to the struct size BEFORE GetProcessMemoryInfo
    // per the Microsoft documented contract. Zero-initializing the
    // buffer leaves `cb = 0`, which the kernel uses as a version /
    // size discriminator and rejects with FALSE — silently disabling
    // RSS sampling on Windows if we forget. Initialize the field
    // explicitly.
    let mut counters: MaybeUninit<PROCESS_MEMORY_COUNTERS> = MaybeUninit::zeroed();
    let size = size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
    // SAFETY: `counters.as_mut_ptr()` is a valid pointer to a
    // zero-initialized buffer of exactly `PROCESS_MEMORY_COUNTERS` size;
    // writing the primitive `u32` `cb` field is sound.
    unsafe {
        std::ptr::addr_of_mut!((*counters.as_mut_ptr()).cb).write(size);
    }

    // SAFETY: handle is non-null and obtained from OpenProcess above;
    // counters is sized correctly and has its `cb` field set; size
    // matches the struct.
    let rc = unsafe { GetProcessMemoryInfo(handle, counters.as_mut_ptr(), size) };
    // SAFETY: handle is non-null and owned by us; CloseHandle returns
    // non-zero on success. Return value intentionally ignored — the
    // sample succeeded or failed already, and this is a cleanup path
    // with no error-propagation lane.
    let _ = unsafe { CloseHandle(handle) };

    if rc == 0 {
        return None;
    }
    // SAFETY: rc != 0 implies GetProcessMemoryInfo fully populated the buffer.
    let counters = unsafe { counters.assume_init() };
    Some((counters.WorkingSetSize as u64) / 1024)
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
    fn extract_rendered_preserves_parent_when_child_rendered_is_null_bare_rustc() {
        // rustc `--error-format=json` emits one object per diagnostic;
        // `children` (help/note sub-diagnostics whose own `rendered` is
        // typically `null`) is serialized BEFORE the parent's `rendered`.
        // A naive "find the first `\"rendered\"` token" scan latches onto
        // the child's `null` and drops the entire parent diagnostic. The
        // fix must return the outermost (parent) `rendered`, not the first
        // one it sees textually.
        let line = concat!(
            r#"{"$message_type":"diagnostic",""#,
            r#"message":"not all trait items implemented","#,
            r#""code":{"code":"E0046","explanation":null},"#,
            r#""level":"error","#,
            r#""spans":[],"#,
            r#""children":[{"message":"`bar` from trait","code":null,"#,
            r#""level":"help","spans":[],"children":[],"rendered":null}],"#,
            r#""rendered":"error[E0046]: not all trait items implemented\n   --> a.rs:1:1\n"}"#,
        );
        let r = extract_rendered(line)
            .expect("parent rendered must survive a null child rendered field");
        assert!(
            r.contains("E0046"),
            "expected parent E0046 rendered text, got: {r:?}",
        );
        assert!(
            r.contains("not all trait items implemented"),
            "expected parent diagnostic body, got: {r:?}",
        );
    }

    #[test]
    fn extract_rendered_preserves_parent_when_child_rendered_is_null_cargo_wrap() {
        // Same regression, cargo `--message-format=json` wrapped form:
        // the diagnostic object is nested under `message`. Both shapes
        // are covered by `extract_rendered`'s existing tests, so both
        // shapes must survive the fix.
        let line = concat!(
            r#"{"reason":"compiler-message","message":{"#,
            r#""message":"no method named `foo` found","#,
            r#""children":[{"message":"consider using ...","level":"help","rendered":null}],"#,
            r#""rendered":"error[E0599]: no method named `foo` found\n"}}"#,
        );
        let r = extract_rendered(line)
            .expect("cargo-wrapped parent rendered must survive a null child rendered field");
        assert!(
            r.contains("E0599"),
            "expected parent E0599 rendered text, got: {r:?}",
        );
    }

    #[test]
    fn render_json_diagnostics_does_not_reduce_primary_to_abort_only_footer() {
        // End-to-end shape: rustc stream of a compile_fail fixture that
        // produces a primary diagnostic with a help child (whose
        // `rendered` is null), followed by the standard
        // "aborting due to N previous error" + `rustc --explain` footer
        // lines. Pre-fix, the primary was silently dropped and only the
        // footer survived; `--bless` would write a snapshot containing
        // ONLY the footer. Post-fix, the primary must survive in the
        // rendered output so that `.stderr` snapshots stay honest.
        let stderr = concat!(
            r#"{"$message_type":"diagnostic","message":"primary","#,
            r#""code":{"code":"E0046","explanation":null},"#,
            r#""level":"error","children":[{"#,
            r#""message":"help","code":null,"level":"help",""#,
            r#"spans":[],"children":[],"rendered":null}],"#,
            r#""rendered":"error[E0046]: not all trait items implemented\n   --> a.rs:1:1\n"}"#,
            "\n",
            r#"{"$message_type":"diagnostic","message":"aborting due to 1 previous error","#,
            r#""level":"error","rendered":"error: aborting due to 1 previous error\n"}"#,
            "\n",
            r#"{"$message_type":"diagnostic","message":"For more information about this error, try `rustc --explain E0046`.","#,
            r#""level":"failure-note","rendered":"For more information about this error, try `rustc --explain E0046`.\n"}"#,
            "\n",
        );
        let out = render_json_diagnostics(stderr);
        assert!(
            out.contains("E0046"),
            "primary E0046 diagnostic must be preserved, got: {out:?}",
        );
        assert!(
            out.contains("not all trait items implemented"),
            "primary rendered body must be preserved, got: {out:?}",
        );
        // Defensive: catch a regression where only the rustc footer
        // ("aborting due to" + `rustc --explain`) survives. Concretely,
        // if the primary is dropped the entire `out` reduces to those
        // two trailing lines.
        let abort_only_footer = "error: aborting due to 1 previous error\nFor more information about this error, try `rustc --explain E0046`.\n";
        assert_ne!(
            out, abort_only_footer,
            "rendered output must not be reduced to the abort+explain footer",
        );
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

    #[test]
    fn parallelism_gate_starts_at_cap() {
        let g = ParallelismGate::new(4);
        assert_eq!(g.current_cap(), 4);
    }

    #[test]
    fn parallelism_gate_reduce_drops_cap_with_floor_one() {
        let g = ParallelismGate::new(4);
        assert_eq!(g.reduce(), 3);
        assert_eq!(g.reduce(), 2);
        assert_eq!(g.reduce(), 1);
        // Floor: subsequent reduce calls are no-ops at cap=1.
        assert_eq!(g.reduce(), 1);
        assert_eq!(g.current_cap(), 1);
    }

    #[test]
    fn parallelism_gate_acquire_release_round_trips() {
        let g = Arc::new(ParallelismGate::new(2));
        assert!(g.acquire());
        assert!(g.acquire());
        // Cap is 2 — third acquire would block. The release
        // path is tested by releasing twice and re-acquiring.
        g.release();
        g.release();
        assert!(g.acquire());
        assert!(g.acquire());
    }

    #[test]
    fn parallelism_gate_close_unblocks_waiters() {
        let g = Arc::new(ParallelismGate::new(1));
        assert!(g.acquire()); // hold the only permit
        let g2 = Arc::clone(&g);
        let waiter = thread::spawn(move || g2.acquire());
        // Give the waiter a moment to block, then close the gate.
        thread::sleep(Duration::from_millis(50));
        g.close();
        // The waiter must have woken with `false`.
        assert!(!waiter.join().unwrap());
    }

    #[test]
    fn parallelism_gate_reduce_burns_in_flight_permit() {
        // Scenario: cap=2, both permits acquired; cap is then reduced.
        // Currently-acquired permits are not credited back beyond the
        // new cap, so after both `release` calls only `cap=1` permit
        // remains available.
        let g = ParallelismGate::new(2);
        assert!(g.acquire());
        assert!(g.acquire());
        let new_cap = g.reduce();
        assert_eq!(new_cap, 1);
        g.release();
        g.release();
        // After both releases, available <= cap. Acquire once more
        // succeeds, second would block — the second is not attempted to
        // avoid a hang in the test.
        assert!(g.acquire());
        assert_eq!(g.current_cap(), 1);
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
            allow_lints: vec![],
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
                compat_short_cargo: false,
                extra_substitutions: Vec::new(),
                strip_lines: Vec::new(),
                strip_line_prefixes: Vec::new(),
                keep_foreign_span_bodies: false,
            },
            sysroot_lib_dir: PathBuf::from("/r/lib"),
            freshness_snapshot: FreshnessSnapshot {
                managed_dylib_path: PathBuf::from("/p/target/lihaaf/lib.so"),
                original_mtime_unix_secs: 0,
                original_sha256: "0".repeat(64),
                original_toolchain: crate::toolchain::Toolchain {
                    release_line: "rustc 1.95.0 (test 2026-01-01)".into(),
                    release: "1.95.0".into(),
                    host: "x86_64-unknown-linux-gnu".into(),
                    commit_hash: "0000000000000000000000000000000000000000".into(),
                    sysroot: PathBuf::from("/r"),
                },
            },
        }
    }

    #[test]
    fn classify_exit_emits_malformed_diagnostic_with_correct_offset() {
        // the policy: a non-UTF-8 byte in rustc's stderr surfaces as
        // MALFORMED_DIAGNOSTIC with the precise byte offset of the
        // first invalid byte. A minimal WorkerContext is constructed
        // (no rustc spawn — classify_exit is pure given its inputs)
        // and fed bytes containing `0xFE` after 3 valid prefix
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

    #[test]
    fn sample_rss_returns_some_for_self() {
        // Every supported platform — Linux, macOS, Windows — must return
        // Some(kib) for our own PID.
        let pid = std::process::id();
        let kib = sample_rss_kib(pid);
        #[cfg(any(target_os = "linux", target_os = "macos", windows))]
        {
            let rss = kib.expect("expected Some(rss) on supported platforms (Linux/macOS/Windows)");
            assert!(rss > 0, "self RSS must be > 0; got {rss}");
        }
        #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
        assert!(kib.is_none(), "unsupported platforms still return None");
    }

    #[test]
    fn sample_rss_returns_none_for_invalid_pid() {
        // u32::MAX is virtually guaranteed to be a non-existent PID on
        // every supported platform. The sampler must return None rather
        // than panicking, returning 0, or surfacing an error.
        assert_eq!(sample_rss_kib(u32::MAX), None);
    }

    /// Per-fixture `rustc` invocations must carry `CARGO_MANIFEST_DIR`
    /// set to the consumer crate root. Any proc macro that calls
    /// `proc_macro_crate::crate_name("foo")` reads this variable at
    /// macro-expansion time to locate the consumer's `Cargo.toml` and
    /// resolve renamed dependencies. Without it, the macro fails with
    /// "`CARGO_MANIFEST_DIR` env variable not set" before the
    /// compile-fail / compile-pass assertion can be evaluated.
    ///
    /// The test invokes [`apply_rustc_env`] — the same helper that
    /// `spawn_and_monitor` calls before every per-fixture spawn — so
    /// any future regression that drops the `CARGO_MANIFEST_DIR` line
    /// trips here without needing to spawn rustc. The test also
    /// asserts that `LD_LIBRARY_PATH` is still set, guarding the
    /// dylib-loader invariant from accidental removal in the same edit.
    #[test]
    fn fixture_rustc_cmd_carries_cargo_manifest_dir() {
        let ctx = unit_test_ctx();
        let mut cmd = Command::new("rustc");
        apply_rustc_env(&mut cmd, &ctx);

        // Collect every explicitly-set env var on this command.
        let envs: Vec<(std::ffi::OsString, Option<std::ffi::OsString>)> = cmd
            .get_envs()
            .map(|(k, v)| (k.to_os_string(), v.map(|s| s.to_os_string())))
            .collect();

        // CARGO_MANIFEST_DIR must be present and equal to crate_root.
        let manifest_dir = envs
            .iter()
            .find(|(k, _)| k == "CARGO_MANIFEST_DIR")
            .and_then(|(_, v)| v.as_ref())
            .expect("CARGO_MANIFEST_DIR must be set on every per-fixture rustc command");
        assert_eq!(
            std::path::Path::new(manifest_dir),
            ctx.crate_root.as_path(),
            "CARGO_MANIFEST_DIR must equal the consumer crate root"
        );

        // LD_LIBRARY_PATH must also be present (pre-existing invariant).
        let ld_present = envs
            .iter()
            .any(|(k, v)| k == "LD_LIBRARY_PATH" && v.is_some());
        assert!(
            ld_present,
            "LD_LIBRARY_PATH must still be set (regression guard)"
        );
    }

    /// `apply_rustc_env` must overwrite a stale `CARGO_MANIFEST_DIR`
    /// that an earlier caller or the inherited parent environment may
    /// have set to a wrong value. This is the "earlier-leg" companion
    /// to [`apply_rustc_env_does_not_lock_out_later_overrides`], which
    /// only pins the later-write-wins precedence. Without this guard
    /// a future refactor that conditionally skips the
    /// `cmd.env("CARGO_MANIFEST_DIR", …)` line (e.g. "only set it when
    /// the parent process hasn't already") would silently break: the
    /// per-fixture rustc would inherit a `CARGO_MANIFEST_DIR` pointing
    /// at the lihaaf binary's own crate root, not the consumer's, and
    /// `proc_macro_crate::crate_name` would resolve against the wrong
    /// manifest.
    #[test]
    fn apply_rustc_env_overwrites_inherited_cargo_manifest_dir() {
        let ctx = unit_test_ctx();
        let mut cmd = Command::new("rustc");
        // Simulate a wrong value an earlier hop (parent process,
        // inherited env, prior helper) might have left on the command.
        cmd.env("CARGO_MANIFEST_DIR", "/the/wrong/crate");
        apply_rustc_env(&mut cmd, &ctx);
        let manifest_dir = cmd
            .get_envs()
            .find_map(|(k, v)| (k == "CARGO_MANIFEST_DIR").then_some(v))
            .flatten()
            .expect("CARGO_MANIFEST_DIR must remain set after apply_rustc_env");
        assert_eq!(
            std::path::Path::new(manifest_dir),
            ctx.crate_root.as_path(),
            "apply_rustc_env must overwrite a stale CARGO_MANIFEST_DIR with the consumer crate root"
        );
    }

    /// Per-fixture rustc env propagation must not silently clear an
    /// explicit override the parent process injected.
    ///
    /// `Command::env` last-write-wins, so a higher-precedence override
    /// (such as one a future feature flag may set after
    /// [`apply_rustc_env`]) is honored. The test confirms that
    /// behavior by re-setting `CARGO_MANIFEST_DIR` after the helper
    /// runs and verifying the post-helper value wins. This pins the
    /// invariant so a future "set the env once, freeze the command"
    /// refactor cannot regress it without surfacing here.
    #[test]
    fn apply_rustc_env_does_not_lock_out_later_overrides() {
        let ctx = unit_test_ctx();
        let mut cmd = Command::new("rustc");
        apply_rustc_env(&mut cmd, &ctx);

        let override_path = std::path::PathBuf::from("/tmp/lihaaf-override");
        cmd.env("CARGO_MANIFEST_DIR", &override_path);

        let manifest_dir = cmd
            .get_envs()
            .find_map(|(k, v)| (k == "CARGO_MANIFEST_DIR").then_some(v))
            .flatten()
            .expect("CARGO_MANIFEST_DIR still present after override");
        assert_eq!(std::path::Path::new(manifest_dir), override_path.as_path());
    }

    #[test]
    fn apply_feature_cfgs_emits_quoted_feature_cfgs() {
        let mut cmd = Command::new("rustc");
        apply_feature_cfgs(&mut cmd, &["suite_demo".to_string(), "spatial".to_string()]);
        let args: Vec<_> = cmd
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            args,
            vec![
                "--cfg",
                "feature=\"suite_demo\"",
                "--cfg",
                "feature=\"spatial\""
            ]
        );
    }

    #[test]
    fn apply_feature_cfgs_is_noop_for_empty_feature_set() {
        let mut cmd = Command::new("rustc");
        apply_feature_cfgs(&mut cmd, &[]);
        assert_eq!(cmd.get_args().count(), 0);
    }

    #[test]
    fn apply_allow_lints_emits_dash_a_per_lint() {
        let mut cmd = Command::new("rustc");
        apply_allow_lints(
            &mut cmd,
            &["unexpected_cfgs".to_string(), "dead_code".to_string()],
        );
        let args: Vec<_> = cmd
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            args,
            vec!["-A", "unexpected_cfgs", "-A", "dead_code"],
            "each lint must produce exactly one `-A <lint>` pair on the argv"
        );
    }

    #[test]
    fn apply_allow_lints_is_noop_for_empty_slice() {
        let mut cmd = Command::new("rustc");
        apply_allow_lints(&mut cmd, &[]);
        assert_eq!(
            cmd.get_args().count(),
            0,
            "empty allow_lints must not append any flags"
        );
    }

    #[test]
    fn apply_allow_lints_handles_namespaced_lint() {
        let mut cmd = Command::new("rustc");
        apply_allow_lints(&mut cmd, &["clippy::needless_collect".to_string()]);
        let args: Vec<_> = cmd
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            args,
            vec!["-A", "clippy::needless_collect"],
            "namespaced lint must be forwarded verbatim as a single argv token"
        );
    }

    #[cfg(unix)]
    #[test]
    fn resolve_extern_paths_skips_symlink_escape() {
        use std::os::unix::fs::symlink;

        let tmp = tempfile::tempdir().unwrap();
        let deps_dir = tmp.path().join("deps");
        std::fs::create_dir(&deps_dir).unwrap();

        let outside = tmp.path().join("outside");
        std::fs::create_dir(&outside).unwrap();
        let outside_rlib = outside.join("libdemo-abc.rlib");
        std::fs::write(&outside_rlib, b"stub").unwrap();
        symlink(&outside_rlib, deps_dir.join("libdemo-abc.rlib")).unwrap();

        let resolved = resolve_extern_paths(&deps_dir, &["demo".to_string()]).unwrap();
        assert!(
            !resolved.contains_key("demo"),
            "symlinked artifacts escaping deps dir must be ignored"
        );
    }
}
