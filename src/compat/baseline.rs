//! Phase 3 + Phase 4 of compat mode (issues #8 + #9) — argv-only
//! baseline command runner with conservative libtest-output parsing.
//!
//! Spawns the baseline `cargo test` invocation that fork CI compares
//! against. The exact argv vector is supplied by the caller (the
//! `--compat-cargo-test-argv` flag, parsed in Phase 1 and bundled into
//! [`crate::compat::cli::CompatArgs::compat_cargo_test_argv`]); this
//! module never tokenizes a string and never invokes a shell.
//!
//! ## Security invariant — no shell, ever
//!
//! The argv vector is handed directly to
//! [`std::process::Command::new`] + [`std::process::Command::args`], so
//! shell metacharacters (`$HOME`, `;`, `&&`, single quotes, backticks,
//! …) are passed through as literal bytes to the spawned program.
//! There is no path that constructs a single command-line string and
//! hands it to `sh -c`, `bash -c`, or `cmd /c`. The same guarantee
//! holds on Windows: [`std::process::Command`] dispatches via
//! `CreateProcess` directly rather than going through `cmd.exe`, and
//! `std`'s argv-joining round-trip uses the documented Microsoft C
//! runtime quoting so the child sees argv entries verbatim.
//!
//! See `docs/compatibility-plan.md` §3.1 — "no shell command line" is a
//! locked v0.1 invariant.
//!
//! ## Phase 3 scope vs. Phase 4 scope
//!
//! - **Phase 3** ([`run_baseline`]): captures the **coarse baseline**
//!   — argv, exit code, wall-clock, raw stdout / stderr bytes. The
//!   [`BaselineResult::pass`] and [`BaselineResult::fail`] fields stay
//!   [`None`]; [`BaselineResult::unknown_count`] stays `0`. Emits the
//!   sidecar at `schema_version == 1`.
//! - **Phase 4** ([`run_baseline_with_recognized_fixtures`]): runs the
//!   same capture, then funnels the libtest output through
//!   [`parse_libtest_output`] against a caller-supplied recognized-
//!   fixture set (Phase 6 will populate this from a syn AST walk of the
//!   target crate). Populates `pass` / `fail` / `unknown_count` /
//!   `mismatch_entries` per the `docs/compatibility-plan.md` §1
//!   conservatism rule: fixture-level pass/fail is only reported when
//!   the libtest test name correlates to an explicitly recognized
//!   fixture; otherwise the line goes to `unknown_count`. Emits the
//!   sidecar at `schema_version == 2`.
//!
//! ### Sidecar JSON shape
//!
//! v1 (Phase 3 only):
//!
//! ```json
//! {
//!   "schema_version": 1,
//!   "argv": ["cargo", "test", "..."],
//!   "exit_code": 0,
//!   "stdout": "<raw stdout text>",
//!   "stderr": "<raw stderr text>"
//! }
//! ```
//!
//! v2 (Phase 4, additive over v1):
//!
//! ```json
//! {
//!   "schema_version": 2,
//!   "argv": ["cargo", "test", "..."],
//!   "exit_code": 0,
//!   "stdout": "<raw stdout text>",
//!   "stderr": "<raw stderr text>",
//!   "pass": 120,
//!   "fail": 5,
//!   "unknown_count": 0,
//!   "mismatch_entries": [
//!     {"fixture": "tests/foo.rs", "baseline_verdict": "pass"},
//!     {"fixture": "tests/bar.rs", "baseline_verdict": "fail"}
//!   ]
//! }
//! ```
//!
//! `pass` and `fail` are `null` rather than absent when the parser ran
//! against an empty recognized-fixture set — `null` documents "the
//! parser ran but produced no recognized verdict", which is distinct
//! from "the parser was never invoked".
//!
//! ## Conservatism rule (§1)
//!
//! > Baseline extraction is intentionally conservative. Compat mode
//! > records the original `cargo test` command result as the coarse
//! > baseline. Fixture-level baseline status may only be reported when
//! > it is derived from explicitly recognized trybuild invocations and
//! > stable path matches; otherwise the fixture baseline is `unknown`
//! > and the report must say why. The v0.1 pilot gate may require
//! > `unknown == 0` for selected crates, but the implementation must
//! > not infer fixture-level truth from arbitrary libtest output.
//!
//! Practically: [`parse_libtest_output`] only emits `Some(pass)` /
//! `Some(fail)` counts when the caller supplied at least one
//! recognized fixture AND at least one libtest line correlated to it.
//! Every other libtest line (unrecognized test names, recognized
//! fixtures absent from output, garbled lines) is `unknown_count++`.
//! Empty recognized-fixture set ⇒ `pass.is_none() && fail.is_none()`,
//! and every parsed line counts as unknown. This is the property that
//! makes the §5 pilot gate's `unknown == 0` requirement meaningful.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Instant;

use crate::error::Error;
use crate::util;

/// One captured baseline run.
///
/// `pub` (with the parent module pinned at `pub(crate)`) so the crate
/// root can [`#[doc(hidden)]`] re-export this for the integration test
/// crate. Not part of any v0.1 stability contract — the supported entry
/// to compat mode is `cargo lihaaf --compat`, not the Rust API.
///
/// Fields are documented in `docs/compatibility-plan.md` §3.3 (the
/// `results.baseline` subset) plus the §3.3 envelope's
/// `commands.baseline` field for `argv`.
#[derive(Debug)]
// Fields are unread in Phase 3 — the §3.3 envelope writer (Phase 7)
// reads them when wiring `results.baseline` and `commands.baseline`.
// Carrying them today keeps the §3.3 envelope schema additive across
// Phase 3 → Phase 7 (no field renames mid-implementation) and lets the
// integration tests in `tests/compat/argv_baseline_no_shell.rs` assert
// the captured shape.
#[allow(dead_code)]
pub struct BaselineResult {
    /// Number of fixtures libtest reported as passing. Populated only
    /// when fixture-level baseline is RECOGNIZED per §1 — Phase 4
    /// (issue #9) wires the conservative parser. The Phase 3 entry
    /// point [`run_baseline`] always returns `None`; Phase 4's
    /// [`run_baseline_with_recognized_fixtures`] returns `Some(n)` only
    /// when at least one recognized fixture correlated to a libtest
    /// verdict.
    pub pass: Option<u32>,
    /// Number of fixtures libtest reported as failing. Same nullable
    /// rule as [`Self::pass`].
    pub fail: Option<u32>,
    /// Number of fixtures whose libtest output didn't match a
    /// recognized trybuild invocation. Always populated. The Phase 3
    /// entry point [`run_baseline`] always returns `0`. Phase 4's
    /// [`run_baseline_with_recognized_fixtures`] increments this for
    /// every unrecognized libtest line, every recognized fixture
    /// absent from libtest output, and every garbled verdict line —
    /// see [`parse_libtest_output`] for the full classification rule.
    pub unknown_count: u32,
    /// Exit code from the child process. On a signal-terminated child
    /// (no real exit code; `ExitStatus::code()` returns [`None`]) this
    /// is `-1` — the §3.3 envelope renders the signal in `errors[]`
    /// rather than overloading the exit code field.
    pub exit_code: i32,
    /// Wall-clock for the baseline run, milliseconds. EXCLUDED from
    /// determinism checks per §3.3 (timing is not byte-stable across
    /// machines).
    pub dur_ms: u64,
    /// Path to the libtest output sidecar JSON the runner wrote.
    /// Always populated even on a non-zero exit so the §3.3 envelope
    /// writer can point adopters at the raw bytes for diagnosis.
    pub sidecar_path: PathBuf,
    /// Resolved argv that was actually executed. Recorded so the §3.3
    /// envelope's `commands.baseline` field can render the exact
    /// invocation. This is a byte-for-byte copy of the input slice —
    /// no quoting, no shell-escape normalization.
    pub argv: Vec<String>,
    /// Per-fixture mismatch records produced by [`parse_libtest_output`]
    /// in Phase 4. Always sorted by `fixture` (forward-slash ASCII
    /// byte order). Empty in the Phase 3 entry point ([`run_baseline`])
    /// and in any Phase 4 call where no recognized fixture correlated
    /// to a libtest verdict; the §3.3 envelope writer (Phase 8)
    /// transforms these into the `mismatch_examples` array.
    pub mismatch_entries: Vec<BaselineMismatch>,
}

/// Sidecar JSON schema version emitted by the Phase 3 entry point
/// [`run_baseline`]. Adopters parsing the v1 sidecar should gate on
/// this exact integer; the v2 schema is additive over v1 (no field is
/// renamed or retyped) but the bump is the explicit hook adopters
/// switch on.
const SIDECAR_SCHEMA_VERSION_V1: u32 = 1;

/// Sidecar JSON schema version emitted by the Phase 4 entry point
/// [`run_baseline_with_recognized_fixtures`]. Layers `pass`, `fail`,
/// `unknown_count`, and `mismatch_entries` over the v1 shape. Bumping
/// here keeps the Phase 3 sidecar's v1 stamp untouched and lets the
/// Phase 8 envelope writer key off the version without inspecting
/// fields.
const SIDECAR_SCHEMA_VERSION_V2: u32 = 2;

/// Sentinel exit code used when the child terminated via a signal and
/// no real OS-level exit code is available. `-1` is chosen because
/// every real POSIX exit code is in `0..=255`, and Windows
/// [`std::process::ExitStatus::code`] only returns `None` on a
/// signal-style termination (rare on that platform).
const SIGNAL_TERMINATED_EXIT_SENTINEL: i32 = -1;

/// One trybuild fixture that Phase 6 (syn AST discovery) recognized in
/// the target crate's test sources. Passed into
/// [`run_baseline_with_recognized_fixtures`] and
/// [`parse_libtest_output`] so the parser knows which fixtures are
/// authorized to contribute to fixture-level pass/fail counts.
///
/// In Phase 4 callers (the conservative-baseline integration tests)
/// construct this directly. Phase 6 produces a richer
/// `DiscoveredFixture` type with the originating `(call_site,
/// fixture_path)` pair; this type carries only the repo-relative
/// fixture path because that is the single field
/// [`parse_libtest_output`] needs.
///
/// `repo_relative_path` is stored as a [`PathBuf`] (platform-native
/// separators) but [`parse_libtest_output`] internally compares
/// against a forward-slash projection so Windows checkouts work the
/// same as POSIX.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FixtureId {
    /// Repo-relative path to the fixture file, e.g.
    /// `tests/trybuild/compile_fail/foo.rs`. Comparison is
    /// forward-slash-normalized before matching against libtest
    /// output so a path that happens to use `\` separators on
    /// Windows still correlates correctly.
    pub repo_relative_path: PathBuf,
}

/// The §3.3 envelope's `MismatchExample` raw input for one recognized
/// fixture. The Phase 8 envelope writer sorts these by `fixture` and
/// transforms them into the final `mismatch_examples` array; this
/// module produces them unsorted (Phase 4) and exposes them on
/// [`BaselineResult::mismatch_entries`] for that downstream step.
///
/// One `BaselineMismatch` is emitted per recognized fixture for which
/// the parser saw a libtest verdict line. Phase 4 carries the raw
/// libtest verdict (`Pass` / `Fail`) plus the fixture path; Phase 8
/// joins this with the lihaaf-side outcome to decide the §3.3
/// `mismatch_type` (`baseline_only_fail`, `lihaaf_only_fail`, etc.).
///
/// `baseline_verdict` is the libtest reading; Phase 4 deliberately
/// does **not** synthesize a comparison against lihaaf here (that is
/// Phase 7/8's job).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BaselineMismatch {
    /// Repo-relative, forward-slash path to the fixture. Sorted on
    /// this field by the §3.3 envelope writer.
    pub fixture: String,
    /// What libtest said about this fixture, classified into the
    /// conservative `Pass` / `Fail` bucket. Unrecognized verdict
    /// shapes never reach this struct — they go to
    /// [`BaselineResult::unknown_count`] instead.
    pub baseline_verdict: BaselineVerdict,
}

/// Conservative classification of a single libtest verdict line.
/// Only two variants are possible because the parser is
/// fail-closed: anything that does not match a documented verdict
/// shape (`... ok`, `... FAILED`) is dropped into `unknown_count`
/// rather than coerced into one of these buckets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BaselineVerdict {
    /// Libtest line ended with `... ok`.
    Pass,
    /// Libtest line ended with `... FAILED`.
    Fail,
}

impl BaselineVerdict {
    /// Stable string form for the v2 sidecar JSON. Stays lowercase
    /// so the on-disk shape is `jq`-friendly and matches the §3.3
    /// envelope's lowercase enum convention.
    fn as_str(self) -> &'static str {
        match self {
            BaselineVerdict::Pass => "pass",
            BaselineVerdict::Fail => "fail",
        }
    }
}

/// Output of [`parse_libtest_output`]. Mirrors the four Phase 4 fields
/// of [`BaselineResult`] (`pass`, `fail`, `unknown_count`,
/// `mismatch_entries`) but is a standalone return type so the parser
/// is unit-testable without spawning a child process.
///
/// Determinism: `mismatch_entries` is sorted by `fixture` (forward-
/// slash ASCII byte order) before being returned. Phase 8's envelope
/// writer asserts the same ordering on its own output; producing it
/// sorted here means the Phase 8 sort is a no-op against this input,
/// and downstream consumers reading the sidecar JSON see a stable
/// layout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedBaseline {
    /// Number of recognized fixtures the parser saw passing in
    /// libtest output. `None` when the recognized-fixture set was
    /// empty AND no recognized matches happened — that case is
    /// distinct from `Some(0)` (recognized fixtures were supplied
    /// but all of them either failed or were absent from output).
    pub pass: Option<u32>,
    /// Number of recognized fixtures the parser saw failing. Same
    /// nullable rule as [`Self::pass`].
    pub fail: Option<u32>,
    /// Number of libtest lines that could not be assigned a
    /// fixture-level verdict. Always populated. Increments for:
    ///
    /// 1. Every verdict line when the recognized-fixture set is empty.
    /// 2. Verdict lines whose libtest test name does not equal any
    ///    recognized fixture's stem (forward-slash form of
    ///    `repo_relative_path` minus its `.rs` extension; exact
    ///    match — substring matching would let fixture
    ///    `tests/ui/foo.rs` collide with libtest line
    ///    `tests/ui/foo_extra`).
    /// 3. Recognized fixtures the parser never saw in output (the
    ///    "absence of evidence is not pass" rule).
    /// 4. Verdict lines with malformed shape after ANSI stripping
    ///    (truncated, unexpected token order, etc.).
    pub unknown_count: u32,
    /// Per-fixture entries the §3.3 envelope writer (Phase 8) will
    /// turn into `mismatch_examples`. Sorted by `fixture` (forward-
    /// slash ASCII byte order). Populated only for recognized
    /// fixtures the parser correlated to a libtest verdict.
    pub mismatch_entries: Vec<BaselineMismatch>,
}

/// Strip ANSI CSI escape sequences from `s` in place, returning a new
/// owned [`String`]. Used by [`parse_libtest_output`] so a libtest
/// build that emits `\x1b[31mFAILED\x1b[0m` (color-on terminal) is
/// indistinguishable from the CI/cargo-test-with-`--color=never`
/// form.
///
/// Implementation is byte-level (per §6.1 — no regex). Walks the
/// input forward, skipping any run that starts with `\x1b[` and
/// continues until the first byte in `0x40..=0x7e` (the SGR final
/// byte range per ECMA-48 §5.4), inclusive of that byte. Lone
/// `\x1b` not followed by `[` are passed through verbatim — they are
/// not CSI escapes and may be legitimate output content.
///
/// The function is `pub(crate)` so the test module can unit-test it.
pub(crate) fn strip_ansi(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        // CSI: ESC '[' ... <final byte 0x40..=0x7e>.
        if bytes[i] == 0x1b && i + 1 < bytes.len() && bytes[i + 1] == b'[' {
            // Skip past ESC '['.
            i += 2;
            // Eat parameter and intermediate bytes, then the final
            // byte. ECMA-48 final-byte range is 0x40..=0x7e.
            while i < bytes.len() {
                let b = bytes[i];
                i += 1;
                if (0x40..=0x7e).contains(&b) {
                    break;
                }
            }
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    // `bytes` started life as valid UTF-8; we only ever dropped a
    // pure-ASCII subsequence (`ESC [ ... final`) which itself is
    // 7-bit-clean, so what remains is still valid UTF-8. The
    // `from_utf8_unchecked` path is tempting but `from_utf8` is the
    // safe call and the cost is a single linear scan.
    String::from_utf8(out).unwrap_or_default()
}

/// Conservative parser for libtest stdout. See module-level docs for
/// the conservatism rule (§1).
///
/// Inputs:
///
/// - `stdout`: the raw stdout text captured from the baseline
///   `cargo test` run. ANSI escape sequences are stripped before
///   parsing so color-on and color-off output produce identical
///   results.
/// - `recognized_fixtures`: the Phase 6 discovery output, identifying
///   which trybuild fixtures the parser is authorized to assign
///   fixture-level verdicts to. An empty slice triggers the maximally
///   conservative path: every verdict line becomes
///   `unknown_count++`, and `pass` / `fail` stay [`None`].
///
/// Output: [`ParsedBaseline`]. See its docs for the field semantics.
///
/// ### Parsing shape
///
/// Libtest emits one verdict per test in the shape:
///
/// ```text
/// test <test_name> ... ok
/// test <test_name> ... FAILED
/// ```
///
/// Lines not starting with `"test "` are ignored unconditionally
/// (this skips `running N tests`, `test result: ...`, `failures:`,
/// stack traces, and every other shape libtest emits around the
/// per-test lines). Lines starting with `"test "` but missing the
/// `" ... "` separator are counted as `unknown_count` — they are
/// either malformed or a libtest variant the parser does not
/// recognize, and the safe answer is "unknown" per §1.
///
/// ### Fixture-to-test-name correlation
///
/// Libtest reports the **test function name** (e.g. `tests::trybuild`),
/// not the individual fixture path. Phase 6's discovery output maps
/// `(call_site, fixture_path)` pairs; the parser here uses an
/// **exact match** against the forward-slash form of
/// `repo_relative_path` minus the `.rs` extension. Miss ⇒
/// `unknown_count++`. Exact equality (rather than substring) is the
/// load-bearing rule that closes a prefix-collision class — a
/// recognized fixture `tests/ui/foo.rs` must not also match a
/// libtest line `test tests/ui/foo_extra ... ok`.
pub fn parse_libtest_output(stdout: &str, recognized_fixtures: &[FixtureId]) -> ParsedBaseline {
    // Pre-compute the forward-slash + stem-stripped form of every
    // recognized fixture. Sorted lookup table for stable matching
    // semantics — `iter().find` is O(n) per verdict line, but n is
    // bounded by the recognized-fixture count which is the syn AST
    // walk's output (typically tens to hundreds of fixtures per
    // crate; constant-time lookup adds complexity that buys little).
    let normalized: Vec<(String, &FixtureId)> = recognized_fixtures
        .iter()
        .map(|fid| {
            let raw = fid.repo_relative_path.to_string_lossy().into_owned();
            // Forward-slash normalize. `util::to_forward_slash`
            // already handles the `\` → `/` case and is the spec's
            // single canonicalizer.
            let forward = util::to_forward_slash(&raw);
            // Strip the `.rs` extension so a libtest test name
            // doesn't have to include `.rs` (libtest test names
            // never do).
            let stem = forward
                .strip_suffix(".rs")
                .map(str::to_string)
                .unwrap_or(forward);
            (stem, fid)
        })
        .collect();

    // Strip ANSI before line splitting — color codes can wrap the
    // entire verdict (`\x1b[32mtest foo ... ok\x1b[0m`) or just the
    // verdict word (`test foo ... \x1b[31mFAILED\x1b[0m`); stripping
    // once at the front means the line-parser sees plain bytes
    // regardless of which shape cargo emitted.
    let cleaned = strip_ansi(stdout);

    let mut pass_count: u32 = 0;
    let mut fail_count: u32 = 0;
    let mut unknown_count: u32 = 0;
    // Track which recognized fixtures were correlated so we can
    // count the ones libtest never named.
    let mut matched_indices: Vec<bool> = vec![false; normalized.len()];
    let mut mismatch_entries: Vec<BaselineMismatch> = Vec::new();

    for raw_line in cleaned.lines() {
        let line = raw_line.trim_start();
        // Only `test ` (with the single trailing space) is a verdict
        // line. The `tests::` Rust module path would be `test `
        // too, but libtest prefixes the line with the literal word
        // `test` followed by the test name; the `s` would be part
        // of the test name token, not the prefix.
        if !line.starts_with("test ") {
            continue;
        }

        // Find the ` ... ` separator. `split_once` is `O(n)` per
        // line; libtest verdict lines are short (typically < 200
        // bytes) so this is fine. No regex (§6.1).
        let after_prefix = &line["test ".len()..];
        let Some((test_name, verdict_part)) = after_prefix.split_once(" ... ") else {
            // Malformed verdict line. Could be `test result: ok.
            // 12 passed; ...` — that starts with `test ` but lacks
            // the ` ... ` separator. Conservative answer: unknown.
            // We deliberately don't bump unknown_count for these
            // summary lines (they are NOT per-test verdicts), so
            // distinguish via the additional check below.
            //
            // Heuristic: if the candidate test_name contains `:`
            // (e.g. `test result:`) or starts with `result`, this
            // is a summary line, not a per-test verdict. Skip.
            if after_prefix.starts_with("result") || after_prefix.starts_with("result:") {
                continue;
            }
            // Otherwise this is a malformed per-test verdict. Per
            // the conservatism rule, count as unknown.
            unknown_count = unknown_count.saturating_add(1);
            continue;
        };

        // Verdict classification. Libtest emits exactly one of
        // `ok`, `FAILED`, `ignored`, or `bench` (the last two are
        // not relevant for trybuild output, but we tolerate them by
        // dropping into unknown). The verdict token may be followed
        // by additional text (timing info on `--report-time`).
        let verdict_token = verdict_part.split_whitespace().next().unwrap_or("");
        let verdict = match verdict_token {
            "ok" => BaselineVerdict::Pass,
            "FAILED" => BaselineVerdict::Fail,
            // `ignored`, `bench`, unknown verdict words — count as
            // unknown rather than coercing into pass/fail.
            _ => {
                unknown_count = unknown_count.saturating_add(1);
                continue;
            }
        };

        // Correlate the test name against the recognized fixtures.
        // Empty recognized set ⇒ every verdict line is unknown
        // (the conservatism rule's strongest form).
        if normalized.is_empty() {
            unknown_count = unknown_count.saturating_add(1);
            continue;
        }

        // Exact match against the fixture stem. Forward-slash
        // normalize the test name first so `tests\foo` (unlikely
        // but possible on Windows-built test binaries) matches the
        // same fixture as `tests/foo`. The earlier substring shape
        // had a prefix-collision class: a recognized fixture
        // `tests/ui/foo.rs` would also match a libtest verdict line
        // `test tests/ui/foo_extra ... ok` because `foo_extra`
        // contains the substring `foo`. Exact equality closes that
        // class and is sufficient for every Phase 4 test corpus
        // (their libtest test names match the stem byte-for-byte).
        let test_name_norm = util::to_forward_slash(test_name);
        let matched: Option<usize> = normalized
            .iter()
            .position(|(stem, _)| test_name_norm == stem.as_str());

        let Some(idx) = matched else {
            // Libtest named a test the parser couldn't correlate
            // to a recognized fixture. Conservative answer: unknown.
            unknown_count = unknown_count.saturating_add(1);
            continue;
        };

        matched_indices[idx] = true;
        match verdict {
            BaselineVerdict::Pass => pass_count = pass_count.saturating_add(1),
            BaselineVerdict::Fail => fail_count = fail_count.saturating_add(1),
        }
        mismatch_entries.push(BaselineMismatch {
            fixture: normalized[idx].0.clone() + ".rs",
            baseline_verdict: verdict,
        });
    }

    // Recognized fixtures the parser never saw a verdict for: the
    // "absence of evidence is not pass" rule. No mismatch entry is
    // emitted for these — there's nothing for the §3.3 envelope to
    // mismatch against (the baseline verdict is genuinely unknown).
    for seen in &matched_indices {
        if !*seen {
            unknown_count = unknown_count.saturating_add(1);
        }
    }

    // Determinism: sort by fixture forward-slash ASCII byte order.
    mismatch_entries.sort_by(|a, b| a.fixture.cmp(&b.fixture));

    // `pass` / `fail` populated only when the parser had a recognized
    // set to work with. Empty recognized set ⇒ both `None` even if
    // `unknown_count > 0`. This is the key distinction the §5 pilot
    // gate keys off — `Some(0)` (recognized fixtures, none passed)
    // is meaningfully different from `None` (no fixtures recognized).
    let (pass, fail) = if normalized.is_empty() {
        (None, None)
    } else {
        (Some(pass_count), Some(fail_count))
    };

    ParsedBaseline {
        pass,
        fail,
        unknown_count,
        mismatch_entries,
    }
}

/// Capture from one argv-only child process spawn. Shared by the
/// Phase 3 and Phase 4 entry points; the only behavioral divergence
/// between them is the post-capture parse + sidecar shape.
struct SpawnCapture {
    exit_code: i32,
    dur_ms: u64,
    stdout: String,
    stderr: String,
}

/// Argv-only child spawn + capture. Centralizes the empty-argv guard,
/// the `Command::new().args().current_dir()` build, the piped I/O
/// setup, the wall-clock measurement, and the
/// signal-terminated-exit-code sentinel handling.
///
/// No shell, no `sh -c`, no `cmd /c` — the first argv element is the
/// program; the remaining elements are direct argv entries the OS
/// hands to the child without interpretation.
fn spawn_and_capture(argv: &[String], cwd: &Path) -> Result<SpawnCapture, Error> {
    if argv.is_empty() {
        return Err(Error::Cli {
            clap_exit_code: 2,
            message: "error: `--compat-cargo-test-argv` must contain at least one argument \
                      (the program to spawn, e.g. `\"cargo\"`)"
                .to_string(),
        });
    }

    let program = &argv[0];
    let args = &argv[1..];

    let started = Instant::now();
    // Capture stdout/stderr so the sidecar can record them. Inherit env
    // by default — the child needs PATH so `cargo` / `rustc` resolve,
    // RUSTUP_TOOLCHAIN so the +toolchain selector continues to work,
    // and CARGO_HOME so adopters with non-default cargo state get the
    // same view their `cargo test` sees outside lihaaf.
    let output = Command::new(program)
        .args(args)
        .current_dir(cwd)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| Error::SubprocessSpawn {
            program: program.clone(),
            source: e,
        })?;

    let dur_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);

    // `ExitStatus::code()` is `None` on Unix signal-terminated children
    // (`SIGKILL`, `SIGTERM`, …). The §3.3 envelope's `errors[]` field
    // is where signal detail belongs; the integer exit-code slot uses
    // a sentinel so adopters consuming only the bare integer still see
    // a non-zero value rather than a misleading 0.
    let exit_code = output
        .status
        .code()
        .unwrap_or(SIGNAL_TERMINATED_EXIT_SENTINEL);

    // Libtest stdout is well-formed UTF-8 in practice (cargo + rustc
    // emit it as such, and the `--format=json` mode if used would
    // round-trip cleanly). Lossy decode tolerates a binary fixture
    // that emits non-UTF-8 noise — those bytes are lost in the
    // sidecar but the rest of the capture stays readable. The
    // alternative (base64) would add a dependency the v0.1 surface
    // does not need.
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();

    Ok(SpawnCapture {
        exit_code,
        dur_ms,
        stdout,
        stderr,
    })
}

/// Run the baseline `cargo test` invocation.
///
/// **Argv-only.** No shell, no `sh -c`, no `cmd /c`. The first element
/// of `argv` is the program; the remaining elements are direct argv
/// entries the OS hands to the child without interpretation.
///
/// **Errors.** Returns:
///
/// - [`Error::Cli`] when `argv` is empty. The diagnostic names the
///   `--compat-cargo-test-argv` flag so the adopter knows which input
///   was malformed even when this function is called through the
///   default `["cargo", "test"]` path.
/// - [`Error::SubprocessSpawn`] when the OS refuses to spawn the
///   program (binary not found, permission denied, …). Distinct from
///   a non-zero exit, which is a normal session outcome captured in
///   [`BaselineResult::exit_code`].
/// - [`Error::Io`] on failure to wait on the child or to write the
///   sidecar JSON.
/// - [`Error::JsonParse`] when the sidecar JSON cannot be serialized.
///   In practice this is unreachable — the input is `String`s,
///   integers, and a vector of `String`s, all of which `serde_json`
///   serializes infallibly — but the error path is wired in defensively
///   so a future schema bump can fail loudly rather than panicking.
///
/// **Side effects.** Writes the sidecar JSON to `sidecar_path` via
/// `crate::util::write_file_atomic`. Creates the sidecar's parent
/// directory if it doesn't exist (matching the atomic-write helper's
/// own semantics).
pub fn run_baseline(
    argv: &[String],
    cwd: &Path,
    sidecar_path: &Path,
) -> Result<BaselineResult, Error> {
    let SpawnCapture {
        exit_code,
        dur_ms,
        stdout,
        stderr,
    } = spawn_and_capture(argv, cwd)?;

    write_sidecar(sidecar_path, argv, exit_code, &stdout, &stderr)?;

    Ok(BaselineResult {
        pass: None,
        fail: None,
        unknown_count: 0,
        exit_code,
        dur_ms,
        sidecar_path: sidecar_path.to_path_buf(),
        argv: argv.to_vec(),
        mismatch_entries: Vec::new(),
    })
}

/// Run the baseline `cargo test` invocation with conservative
/// fixture-level libtest parsing layered on top (Phase 4 of compat
/// mode, issue #9).
///
/// Same argv-only / no-shell guarantees as [`run_baseline`]. After
/// the capture completes, [`parse_libtest_output`] is called against
/// `stdout` with `recognized_fixtures`; the parsed counts and
/// mismatch entries are written into the returned [`BaselineResult`].
///
/// Side effects: writes the v2 sidecar JSON (see module-level docs)
/// to `sidecar_path`.
///
/// **Conservatism.** When `recognized_fixtures.is_empty()`,
/// [`BaselineResult::pass`] and [`BaselineResult::fail`] stay [`None`]
/// and every verdict line in `stdout` increments
/// [`BaselineResult::unknown_count`]. This is the §1 rule — the
/// v0.1 pilot gate's `unknown == 0` requirement is implementable
/// because absence of recognition produces zero pass/fail signal.
///
/// Errors: same shape as [`run_baseline`] — `Cli` on empty argv,
/// `SubprocessSpawn` on OS spawn refusal, `Io` on wait/write, and
/// `JsonParse` on sidecar serialization failure.
pub fn run_baseline_with_recognized_fixtures(
    argv: &[String],
    cwd: &Path,
    sidecar_path: &Path,
    recognized_fixtures: &[FixtureId],
) -> Result<BaselineResult, Error> {
    let SpawnCapture {
        exit_code,
        dur_ms,
        stdout,
        stderr,
    } = spawn_and_capture(argv, cwd)?;

    // Phase 4: parse libtest output conservatively.
    let parsed = parse_libtest_output(&stdout, recognized_fixtures);

    write_sidecar_v2(sidecar_path, argv, exit_code, &stdout, &stderr, &parsed)?;

    Ok(BaselineResult {
        pass: parsed.pass,
        fail: parsed.fail,
        unknown_count: parsed.unknown_count,
        exit_code,
        dur_ms,
        sidecar_path: sidecar_path.to_path_buf(),
        argv: argv.to_vec(),
        mismatch_entries: parsed.mismatch_entries,
    })
}

/// Build the v1-shaped envelope keys: `schema_version`, `argv`,
/// `exit_code`, `stdout`, `stderr`, in that insertion order. The v2
/// writer appends additional keys on top of this base.
///
/// Insertion order is preserved by the `preserve_order` feature on
/// `serde_json` (enabled at the crate level), so adopters reading the
/// file with `jq` see a stable shape across runs.
fn build_v1_envelope(
    schema_version: u32,
    argv: &[String],
    exit_code: i32,
    stdout: &str,
    stderr: &str,
) -> serde_json::Map<String, serde_json::Value> {
    let mut envelope = serde_json::Map::new();
    envelope.insert(
        "schema_version".to_string(),
        serde_json::Value::from(schema_version),
    );
    envelope.insert(
        "argv".to_string(),
        serde_json::Value::Array(
            argv.iter()
                .map(|s| serde_json::Value::String(s.clone()))
                .collect(),
        ),
    );
    envelope.insert("exit_code".to_string(), serde_json::Value::from(exit_code));
    envelope.insert(
        "stdout".to_string(),
        serde_json::Value::String(stdout.to_string()),
    );
    envelope.insert(
        "stderr".to_string(),
        serde_json::Value::String(stderr.to_string()),
    );
    envelope
}

/// Serialize the v1 sidecar JSON and write it atomically. Used by
/// the Phase 3 entry point [`run_baseline`]; the Phase 4 entry point
/// calls [`write_sidecar_v2`] instead.
///
/// Split out from [`run_baseline`] for testability — the inline unit
/// test in this module exercises the serializer shape without
/// spawning a child.
fn write_sidecar(
    sidecar_path: &Path,
    argv: &[String],
    exit_code: i32,
    stdout: &str,
    stderr: &str,
) -> Result<(), Error> {
    let envelope = build_v1_envelope(SIDECAR_SCHEMA_VERSION_V1, argv, exit_code, stdout, stderr);

    let mut bytes =
        serde_json::to_vec_pretty(&serde_json::Value::Object(envelope)).map_err(|e| {
            Error::JsonParse {
                context: "serializing compat baseline sidecar".into(),
                message: e.to_string(),
            }
        })?;
    // Trailing newline so `cat` output reads cleanly.
    bytes.push(b'\n');

    util::write_file_atomic(sidecar_path, &bytes)
}

/// Serialize the v2 sidecar JSON and write it atomically. Used by
/// the Phase 4 entry point [`run_baseline_with_recognized_fixtures`].
/// Layers `pass`, `fail`, `unknown_count`, and `mismatch_entries`
/// over the v1 shape; the canonical key order is
/// `schema_version`, `argv`, `exit_code`, `stdout`, `stderr`,
/// `pass`, `fail`, `unknown_count`, `mismatch_entries`. v1 readers
/// that ignore unknown fields keep working against v2 sidecars; v2
/// readers gate on `schema_version == 2` and read the additional
/// fields.
fn write_sidecar_v2(
    sidecar_path: &Path,
    argv: &[String],
    exit_code: i32,
    stdout: &str,
    stderr: &str,
    parsed: &ParsedBaseline,
) -> Result<(), Error> {
    let mut envelope =
        build_v1_envelope(SIDECAR_SCHEMA_VERSION_V2, argv, exit_code, stdout, stderr);
    envelope.insert(
        "pass".to_string(),
        match parsed.pass {
            Some(n) => serde_json::Value::from(n),
            None => serde_json::Value::Null,
        },
    );
    envelope.insert(
        "fail".to_string(),
        match parsed.fail {
            Some(n) => serde_json::Value::from(n),
            None => serde_json::Value::Null,
        },
    );
    envelope.insert(
        "unknown_count".to_string(),
        serde_json::Value::from(parsed.unknown_count),
    );
    let mismatch_array: Vec<serde_json::Value> = parsed
        .mismatch_entries
        .iter()
        .map(|m| {
            let mut obj = serde_json::Map::new();
            obj.insert(
                "fixture".to_string(),
                serde_json::Value::String(m.fixture.clone()),
            );
            obj.insert(
                "baseline_verdict".to_string(),
                serde_json::Value::String(m.baseline_verdict.as_str().to_string()),
            );
            serde_json::Value::Object(obj)
        })
        .collect();
    envelope.insert(
        "mismatch_entries".to_string(),
        serde_json::Value::Array(mismatch_array),
    );

    let mut bytes =
        serde_json::to_vec_pretty(&serde_json::Value::Object(envelope)).map_err(|e| {
            Error::JsonParse {
                context: "serializing compat baseline sidecar (v2)".into(),
                message: e.to_string(),
            }
        })?;
    bytes.push(b'\n');

    util::write_file_atomic(sidecar_path, &bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    /// The empty-argv guard fires before any process is spawned. The
    /// diagnostic must name `--compat-cargo-test-argv` so the adopter
    /// can find the flag in `cargo lihaaf --help`.
    #[test]
    fn empty_argv_is_rejected_with_directed_message() {
        let tmp = tempdir().unwrap();
        let sidecar = tmp.path().join("baseline_capture.json");
        let err = run_baseline(&[], tmp.path(), &sidecar).expect_err("empty argv must be rejected");
        match err {
            Error::Cli { message, .. } => {
                assert!(
                    message.contains("--compat-cargo-test-argv"),
                    "diagnostic must name the flag; got: {message}"
                );
                assert!(
                    message.contains("at least one argument"),
                    "diagnostic must spell out the requirement; got: {message}"
                );
            }
            other => panic!("expected Error::Cli, got {other:?}"),
        }
    }

    /// Sidecar JSON keys land in the documented order:
    /// `schema_version`, `argv`, `exit_code`, `stdout`, `stderr`. A
    /// reorder would silently break adopter `jq` pipelines that pull
    /// fields by position; the `preserve_order` feature on `serde_json`
    /// is the underlying guarantee.
    #[test]
    fn sidecar_shape_is_canonical() {
        let tmp = tempdir().unwrap();
        let path = tmp.path().join("capture.json");
        let argv = vec!["foo".to_string(), "bar".to_string()];
        write_sidecar(&path, &argv, 0, "out", "err").unwrap();
        let bytes = std::fs::read(&path).unwrap();
        let text = std::str::from_utf8(&bytes).unwrap();

        let i_schema = text
            .find("\"schema_version\"")
            .expect("schema_version key must be present");
        let i_argv = text.find("\"argv\"").expect("argv key must be present");
        let i_exit = text
            .find("\"exit_code\"")
            .expect("exit_code key must be present");
        let i_stdout = text.find("\"stdout\"").expect("stdout key must be present");
        let i_stderr = text.find("\"stderr\"").expect("stderr key must be present");

        assert!(
            i_schema < i_argv && i_argv < i_exit && i_exit < i_stdout && i_stdout < i_stderr,
            "sidecar JSON keys must appear in canonical order: schema_version, argv, \
             exit_code, stdout, stderr; got:\n{text}"
        );
    }

    /// Sidecar schema_version is the documented integer (`1`). A bump
    /// requires a deliberate code change; the test bites if a refactor
    /// accidentally drops the version constant or flips its type.
    #[test]
    fn sidecar_schema_version_is_one() {
        let tmp = tempdir().unwrap();
        let path = tmp.path().join("capture.json");
        write_sidecar(&path, &["x".to_string()], 0, "", "").unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        let v: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(
            v.get("schema_version").and_then(serde_json::Value::as_u64),
            Some(1)
        );
    }

    /// `strip_ansi` removes a single CSI escape sequence and leaves
    /// surrounding bytes alone. Acid test for the byte-level walker;
    /// a regression that off-by-one's the final-byte detection would
    /// either leak the trailing letter (`m`, `K`, etc.) into output
    /// or consume part of the legitimate following text.
    #[test]
    fn strip_ansi_removes_single_csi_sequence() {
        assert_eq!(strip_ansi("\x1b[31mFAILED\x1b[0m"), "FAILED");
        assert_eq!(strip_ansi("plain text"), "plain text");
        assert_eq!(strip_ansi(""), "");
    }

    /// `strip_ansi` handles a CSI in the middle of a longer string
    /// without dropping bytes around it. Also confirms a `[`-only
    /// (without leading `\x1b`) is treated as literal text.
    #[test]
    fn strip_ansi_handles_mixed_content() {
        assert_eq!(
            strip_ansi("test foo ... \x1b[32mok\x1b[0m"),
            "test foo ... ok"
        );
        // A standalone `[` is not a CSI start.
        assert_eq!(strip_ansi("[bracketed]"), "[bracketed]");
    }

    /// `strip_ansi` does not panic on a truncated CSI (no final
    /// byte). The conservative answer is to consume to the end and
    /// emit nothing for the partial sequence — same effect as if the
    /// sequence had completed.
    #[test]
    fn strip_ansi_tolerates_truncated_csi() {
        // `\x1b[31` with no terminator — walker should consume to
        // the end of input and emit only the leading prefix.
        let s = "prefix\x1b[31";
        let out = strip_ansi(s);
        assert_eq!(out, "prefix");
    }

    /// `parse_libtest_output` on an empty recognized-fixture set
    /// counts every verdict line as unknown and leaves pass/fail
    /// `None`. This is the conservatism rule in its strongest form.
    #[test]
    fn parse_empty_recognized_set_yields_all_unknown() {
        let stdout = "test foo ... ok\ntest bar ... FAILED\n";
        let result = parse_libtest_output(stdout, &[]);
        assert_eq!(result.pass, None);
        assert_eq!(result.fail, None);
        assert_eq!(result.unknown_count, 2);
        assert!(result.mismatch_entries.is_empty());
    }

    /// `parse_libtest_output` correlates a libtest line that
    /// substring-matches a recognized fixture and bumps the right
    /// counter.
    #[test]
    fn parse_recognized_pass_correlates() {
        let recognized = vec![FixtureId {
            repo_relative_path: PathBuf::from("tests/foo.rs"),
        }];
        // The libtest test name contains the fixture's stem
        // (`tests/foo`).
        let stdout = "test tests/foo ... ok\n";
        let result = parse_libtest_output(stdout, &recognized);
        assert_eq!(result.pass, Some(1));
        assert_eq!(result.fail, Some(0));
        assert_eq!(result.unknown_count, 0);
        assert_eq!(result.mismatch_entries.len(), 1);
        assert_eq!(result.mismatch_entries[0].fixture, "tests/foo.rs");
        assert_eq!(
            result.mismatch_entries[0].baseline_verdict,
            BaselineVerdict::Pass
        );
    }

    /// `parse_libtest_output` ignores summary lines and stack
    /// traces — anything not starting with `"test "` plus a
    /// `" ... "` separator is dropped on the floor (not counted as
    /// unknown, not counted as pass/fail).
    #[test]
    fn parse_ignores_summary_and_non_verdict_lines() {
        let stdout = "\n\
            running 5 tests\n\
            test tests/foo ... ok\n\
            test result: ok. 1 passed; 0 failed; 0 ignored\n\
            \n\
            failures:\n\
            \n";
        let recognized = vec![FixtureId {
            repo_relative_path: PathBuf::from("tests/foo.rs"),
        }];
        let result = parse_libtest_output(stdout, &recognized);
        assert_eq!(result.pass, Some(1));
        assert_eq!(result.fail, Some(0));
        // Note: 0 unknowns here despite many non-verdict lines.
        assert_eq!(result.unknown_count, 0);
    }

    /// The v2 sidecar carries the documented field order and
    /// schema version. v1 readers can still parse it (they would
    /// ignore the additional fields); v2 readers gate on
    /// `schema_version == 2`.
    #[test]
    fn sidecar_v2_shape_is_canonical() {
        let tmp = tempdir().unwrap();
        let path = tmp.path().join("capture.json");
        let parsed = ParsedBaseline {
            pass: Some(3),
            fail: Some(1),
            unknown_count: 0,
            mismatch_entries: vec![BaselineMismatch {
                fixture: "tests/a.rs".to_string(),
                baseline_verdict: BaselineVerdict::Pass,
            }],
        };
        write_sidecar_v2(
            &path,
            &["cargo".to_string(), "test".to_string()],
            0,
            "stdout",
            "stderr",
            &parsed,
        )
        .unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        let v: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(
            v.get("schema_version").and_then(serde_json::Value::as_u64),
            Some(2)
        );
        assert_eq!(v.get("pass").and_then(serde_json::Value::as_u64), Some(3));
        assert_eq!(v.get("fail").and_then(serde_json::Value::as_u64), Some(1));
        assert_eq!(
            v.get("unknown_count").and_then(serde_json::Value::as_u64),
            Some(0)
        );
        let entries = v
            .get("mismatch_entries")
            .and_then(serde_json::Value::as_array)
            .unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(
            entries[0]
                .get("fixture")
                .and_then(serde_json::Value::as_str),
            Some("tests/a.rs")
        );
        assert_eq!(
            entries[0]
                .get("baseline_verdict")
                .and_then(serde_json::Value::as_str),
            Some("pass")
        );

        // Key order: schema_version, argv, exit_code, stdout,
        // stderr, pass, fail, unknown_count, mismatch_entries.
        let i_schema = text.find("\"schema_version\"").unwrap();
        let i_argv = text.find("\"argv\"").unwrap();
        let i_exit = text.find("\"exit_code\"").unwrap();
        let i_stdout = text.find("\"stdout\"").unwrap();
        let i_stderr = text.find("\"stderr\"").unwrap();
        let i_pass = text.find("\"pass\"").unwrap();
        let i_fail = text.find("\"fail\"").unwrap();
        let i_unknown = text.find("\"unknown_count\"").unwrap();
        let i_mismatch = text.find("\"mismatch_entries\"").unwrap();
        assert!(
            i_schema < i_argv
                && i_argv < i_exit
                && i_exit < i_stdout
                && i_stdout < i_stderr
                && i_stderr < i_pass
                && i_pass < i_fail
                && i_fail < i_unknown
                && i_unknown < i_mismatch,
            "v2 sidecar JSON keys must appear in canonical order; got:\n{text}"
        );
    }

    /// v2 sidecar emits JSON `null` for `pass` and `fail` when the
    /// parser ran with an empty recognized set. This is the
    /// machine-readable signal Phase 8's envelope writer keys off
    /// to decide whether to emit fixture-level counts.
    #[test]
    fn sidecar_v2_pass_fail_null_when_recognition_empty() {
        let tmp = tempdir().unwrap();
        let path = tmp.path().join("capture.json");
        let parsed = ParsedBaseline {
            pass: None,
            fail: None,
            unknown_count: 5,
            mismatch_entries: Vec::new(),
        };
        write_sidecar_v2(&path, &["x".to_string()], 0, "", "", &parsed).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        let v: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert!(v.get("pass").is_some_and(serde_json::Value::is_null));
        assert!(v.get("fail").is_some_and(serde_json::Value::is_null));
        assert_eq!(
            v.get("unknown_count").and_then(serde_json::Value::as_u64),
            Some(5)
        );
    }
}
