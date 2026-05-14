# Changelog

All notable changes to lihaaf are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the
project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0-beta.3] — 2026-05-14

Follow-up simplification pass closing the three deferred Codex SIBLING
findings from the v0.1.0-beta.2 adversarial review. No adopter-visible
behavior change — diagnostic text, public API surface, exit codes,
snapshot byte format, and `[package.metadata.lihaaf]` schema are
unchanged from v0.1.0-beta.2.

### Changed
- **Test consolidation (config):** Sixteen invalid-TOML tests in
  `src/config.rs::tests` now share an
  `assert_parse_rejects_with(toml, &[expected_substrings])` helper.
- **Test consolidation (normalize):** Eleven text-handling tests in
  `src/normalize.rs::tests` now share an
  `assert_normalizes(input, expected)` helper that uses the standard
  `/p` workspace + `/r` sysroot + `/p/x` fixture-directory triplet.
  Path-rewriting tests with custom context and long-type-note tests
  with multi-`contains` assertions retain their original setup.
- **Test consolidation (session):** All four `derive_crate_root_*`
  tests in `src/session.rs::tests` now share an
  `assert_derive_crate_root_equals(input, expected)` helper that
  also asserts the issue-14 empty-path invariant. Previously only
  two of the four tests carried the explicit guard; the helper
  uniformizes the family.

### Internal
- Codex's beta-2 round-2 adversarial review surfaced three
  informational SIBLING findings (config / normalize / session test
  parameterization opportunities). All three are now addressed
  across four atomic commits.
- Adversarial review for beta-3: Codex + Gemini 3.1-pro-preview +
  Sonnet-tier strict reviewer, all ALLOW on round 1.

## [0.1.0-beta.2] — 2026-05-13

Retro simplification pass against the v0.1.0-beta.1 surface. No
behavior change visible to adopters. Internal cleanup only — diagnostic
text, public API surface, exit codes, snapshot byte format, and
`[package.metadata.lihaaf]` schema are unchanged from v0.1.0-beta.1.

### Changed
- **Test consolidation:** Four freshness drift tests (`release_line`,
  `host`, `commit_hash`, `sysroot`) now share an
  `assert_only_field_drifts(field, mutate)` helper that anchors to
  live `rustc` and asserts both the named field appears in the
  changed-fields diagnostic prefix AND the other three do not. The
  `release_line` test previously used a placeholder toolchain and
  skipped the absence-assertion; the new shape strengthens its
  regression bite.
- **Test consolidation:** Four `toolchain::matches` comparator tests
  (`release_line`, `host`, `commit_hash`, `sysroot`) now share an
  `assert_field_mutation_differs(mutate)` helper, mirroring the
  freshness helper above so the same parameterization pattern applies
  to both layers of the four-field comparator.
- **Platform-duplicate test removed:** Windows-only
  `synchronous_release_on_windows_drop` deleted from `lock.rs`. The
  cross-platform `drop_releases_lock_for_same_process_reacquire`
  already bites the same regression on Windows CI; the survivor's
  rustdoc now explicitly documents the Windows-specific behavior.
- **Corpus macro dedup:** The `corpus_error` and
  `corpus_error_with_n_lines` procedural macros in the integration
  corpus now share an `emit_compile_error(body)` helper for the
  shared 4-token `compile_error!(<body>);` emission sequence.
- **Corpus loop clarity:** `corpus_oom_allocate` now uses
  `buf.chunks_mut(4096)` instead of a manual stride index for
  page-touching. LLVM-equivalent codegen; reads as the documented
  intent.

### Internal
- Dead `_ensure_dir_exists` helper deleted from `session.rs` (was
  `#[allow(dead_code)]` + `_`-prefixed double signal; zero callers).
- Broken comment fragment in `compute_parallelism` fixed.
- `FreshnessFailure::RustcDrift` now documents the `Box<Toolchain>`
  × 2 rationale (clippy `result_large_err` mitigation; unboxing
  re-trips the lint).
- 43 simplify-pass findings reviewed (Reuse 4 / Quality 27 /
  Efficiency 12). 8 high-certainty items applied across 4 atomic
  commits; 11 lower-certainty items deferred; 1 (`EFFICIENCY-1`)
  retained per the existing module-level rationale (defense-in-depth
  against in-session toolchain swap).
- Triple-reviewer adversarial panel: Codex + Gemini 3.1-pro-preview +
  Sonnet-tier strict reviewer. Family-completeness sweeps confirmed
  no remaining sibling sites in the crate.

## [0.1.0-beta.1] — 2026-05-13

First public beta. Adopters who pinned `0.1.0-alpha.4` should upgrade
straight to `0.1.0-beta.1`. The Rust library surface has narrowed (see
`### Changed`); adopters who imported internal module paths
(`lihaaf::dylib::*`, `lihaaf::worker::*`, etc.) must switch to the new
crate-root re-exports or to subprocess-spawning `cargo lihaaf`. The
CLI surface, exit codes, verdict catalog, snapshot byte format, and
`[package.metadata.lihaaf]` schema are unchanged from `0.1.0-alpha.4`.

### Added
- Local pre-commit guard (`scripts/scan-secrets.sh`) that scans staged diffs
  for credential-like patterns: database URLs with embedded credentials,
  environment-variable assignments with secret-shaped keys, private key
  blocks, and AWS access keys. Lines containing `<placeholder>` syntax are
  treated as documentation examples and skipped. Install per-clone via
  `scripts/install-pre-commit-hook.sh`; bypass for legitimate false positives
  with `git commit --no-verify`. `scripts/run-scan-tests.sh` exercises the
  scanner against 4 positive and 2 negative fixture files and is wired into
  CI as a step. `SECURITY.md` documents the pattern set, placeholder
  convention, bypass mechanism, and reporting contact.
- macOS and Windows RSS sampling for `MEMORY_EXHAUSTED` attribution
  (KR-5, FIX_BEFORE_BETA Spec C, issue #6). `sample_rss_kib` now uses
  `libc::proc_pidinfo(PROC_PIDTASKINFO)` on macOS and
  `OpenProcess` + `GetProcessMemoryInfo` on Windows, matching the
  existing Linux `/proc/<pid>/statm` semantics. The §5.4
  dynamic-parallelism cap reduction now fires on all three platforms
  after every harness-attributed `MEMORY_EXHAUSTED` kill; on other
  Unixes the OS OOMkiller backstops as before. Two additional Windows
  features (`Win32_System_Threading`, `Win32_System_ProcessStatus`)
  are added to the existing `windows-sys 0.59` dependency.
- Multi-suite configuration. Adopters can declare additional named
  feature-subset suites with `[[package.metadata.lihaaf.suite]]` array
  entries (each with `name`, `features`, `fixture_dirs`, and optional
  `extern_crates` / `dev_deps` / `edition` / `compile_fail_marker` /
  `fixture_timeout_secs` / `per_fixture_memory_mb` overrides), and
  each suite triggers an independent dylib build with that suite's
  feature set propagated to per-fixture rustc invocations. Per-suite
  manifests live at `target/lihaaf/manifest-<name>.json` and per-suite
  cargo target dirs live at `target/lihaaf-build-<name>/`. The default
  suite (built from the top-level `[package.metadata.lihaaf]` table)
  retains the legacy `target/lihaaf/manifest.json` and
  `target/lihaaf-build/` paths so adopters who never add a named suite
  see no cache-key change. New CLI flag `--suite NAME` (repeatable)
  limits the run to the named subset; without `--suite`, every defined
  suite runs in declared metadata order. Fixture directories must be
  disjoint across suites — validated at config parse time so two
  suites cannot collide on snapshot files. The new
  `Manifest::suite_name` field carries the suite identity for
  out-of-band tooling; manifests written by lihaaf <0.1.0-alpha.3
  default the field to `"default"` on read so legacy on-disk state
  keeps deserializing.
- Self-test corpus addition: a new `tests/lihaaf/compile_pass_suite_demo/`
  fixture and `[[package.metadata.lihaaf.suite]] name = "suite_demo"`
  entry in lihaaf's own Cargo.toml. The fixture references
  `lihaaf::SUITE_DEMO_MARKER` (a const exposed only when lihaaf is
  built with `--features suite_demo`); CI runs lihaaf against itself
  and any regression that drops feature propagation between the dylib
  build and the per-fixture rustc fails to link, biting the
  multi-suite invariant without needing a downstream adopter.

### Changed
- New CI gate: `tests/integration_corpus/`, a real-adopter-shaped
  integration corpus that exercises each lihaaf verdict class
  (`OK`, `SNAPSHOT_DIFF` with `LARGE_SNAPSHOT` warning,
  `SNAPSHOT_MISSING`, `TIMEOUT`, `MEMORY_EXHAUSTED`) end-to-end against
  a real proc-macro crate (FIX_BEFORE_BETA Spec D). The corpus uses a
  two-crate `serde + serde_derive` layout — a regular library
  (`integration_corpus`, which lihaaf builds as the `dylib_crate`)
  plus a sibling proc-macro crate (`integration_corpus_macros`,
  resolved out of the dylib's deps dir as an extra `--extern`). The
  proc-macro crate cannot itself be `dylib_crate` because cargo
  rejects `[lib] proc-macro = true` with `--crate-type=dylib`. The
  corpus is `publish = false` and is excluded from lihaaf's own
  package via the new root `[package].exclude` entry. CI runs the
  corpus after the existing self-test step and asserts each verdict
  label fires via four `grep -q` gates against the captured lihaaf
  output — any regression that renames or drops one of those labels
  (or the `LARGE_SNAPSHOT` warning) flips CI red here even if the
  rest of lihaaf still builds clean.
- Narrowed the public Rust library surface to a documented CLI-shape.
  The following modules — previously `pub mod` — are now `pub(crate)`:
  `diff`, `discovery`, `dylib`, `error`, `freshness`, `manifest`,
  `normalize`, `session`, `snapshot`, `toolchain`, `util`, `worker`.
  The `cli`, `config`, `exit`, and `verdict` modules remain `pub` (they
  define the v0.1 stable schema/catalog/argument-parsing contracts).
  Adopters who want a Rust-callable surface should use the new
  crate-root re-exports: `Cli`, `Config`, `Verdict`, `ExitCode`,
  `Error`, `Outcome`, `run`, `Report`. `Outcome` is part of the v0.1
  stable Rust surface because `Error::Session(Outcome)` is a public
  variant of the re-exported `Error` enum; Rust's E0446 rule (private
  type in public interface) makes `Outcome`'s public visibility
  load-bearing, so the re-export ratifies the de-facto contract.
  Pre-1.0 alpha precedent: v0.1.0-alpha.4 is the only published
  version and the CHANGELOG header already states the library API is
  non-stable across v0.1.x; adopters who imported internal module
  paths (`lihaaf::dylib::*`, `lihaaf::worker::*`, etc.) should switch
  to subprocess-spawning `cargo lihaaf` or to the crate-root
  re-exports above. Issue #3.
- The session reporter prints `lihaaf: === suite "<name>" ===` headers
  and per-suite aggregate lines (`lihaaf: suite "<name>": …`) when
  more than one suite runs in a session. Single-suite runs (adopters
  who never add a `[[suite]]` entry) keep their legacy output
  byte-identical: no header, no per-suite line, just the existing
  `lihaaf: <n> ok, …` final aggregate.
- `Err(Error::Session(outcome))` paths in `cargo-lihaaf` now print the
  outcome's diagnostic message to stderr before the exit-code mapping.
  Pre-v0.1.0-alpha.3 the binary exited with the right code but
  silently dropped the diagnostic body — adopters had to consult the
  exit-code table to interpret a session-level failure. The fix is
  behavior-only; the message bodies are unchanged from the existing
  `Display` impl on `Outcome`.
- `WorkerContext::new` signature changed: it now takes
  `dylib_crate: &str` and `suite: &Suite` in place of `&Config`.
  Library API (pre-1.0) — adopters who subprocess-spawn `cargo
  lihaaf` are unaffected. Adopters wiring lihaaf as a library
  (currently lihaaf's own bin and tests) update their call sites; the
  per-suite identity is what the worker now closes over.

### Fixed
- Concurrent `cargo lihaaf` invocations sharing a
  `CARGO_TARGET_DIR` are now serialized via a session-wide advisory
  file lock at `target/lihaaf/.session.lock`. Previously, two
  sessions could collide on `target/lihaaf/manifest-<suite>.json`,
  `target/lihaaf-build-<suite>/`, or the managed-dylib copy —
  most loudly when one side passed `--no-cache` and unconditionally
  deleted the other's mid-read state — and the race surfaced as
  intermittent `DylibBuildFailed`, `DylibNotFound`, spurious
  `FreshnessDrift` on `managed_dylib_path` / `dylib_sha256`, or
  `MalformedDiagnostic` on a partially-deleted snapshot. The lock
  is acquired after the `--list` short-circuit (so `--list` does
  not block) and before the `--no-cache` deletion sweep, and held
  for the remainder of the session. On contention, lihaaf emits
  `lihaaf: waiting for another lihaaf session to release <path>
  ...` to stderr BEFORE blocking, and (if the wait exceeded 50 ms)
  follows up with `lihaaf: acquired session lock after N ms` after
  the lock is granted; single-session runs hit the fast path and
  emit neither line. Cross-platform via `libc::flock(LOCK_EX)` on
  Unix (in an `EINTR` retry loop) and `LockFileEx` on Windows
  (with an explicit `UnlockFileEx` in `Drop` so a fast back-to-back
  same-process re-acquire does not race the OS's deferred
  `CloseHandle` release). Crash recovery is automatic: a process
  that dies holding the lock releases it via OS handle cleanup,
  no stale-lockfile removal needed. New Windows dependency
  `windows-sys 0.59` under `cfg(windows)` only; Unix uses the
  existing `libc` dep. Issue #1.
- Toolchain drift comparator now widens its key to
  `(release_line, host, commit_hash, sysroot)`. The previous
  comparator compared only `release_line`, so two materially different
  toolchains — e.g. rustup stable rustc 1.95.0 vs. a custom local
  build with the same release line, or the same release line on a
  different host triple — compared equal and bypassed the policy
  §4.5 hard-fail. The widening only catches MORE drift, never less,
  so existing pass cases stay passing. Known caveat: when `rustc` is
  a custom local build, `commit-hash:` is absent and `commit_hash`
  is the empty string; two such builds with the same other fields
  still compare equal on that field. Users running custom rustc
  builds operate outside the stable-channel safety net by design;
  the `sysroot` comparison usually catches this in practice. Issue
  #4.
- Session temp directory creation now creates the workspace target
  parent directory first. Clean CI checkouts that run lihaaf before any
  other crate-local Cargo command no longer fail with
  `No such file or directory` while creating
  `<crate>/target/lihaaf-session-*`.
- Per-fixture `rustc` invocations now set `CARGO_MANIFEST_DIR` to the
  consumer crate root (the directory containing the consumer's
  `Cargo.toml`). Cargo sets this automatically on `cargo build` /
  `cargo test`, but lihaaf's per-fixture rustc spawns bypass cargo,
  so it had to be supplied explicitly. Without it, any proc macro
  that calls `proc_macro_crate::crate_name("...")` (the dominant
  pattern for renamed-dependency resolution — used by `serde`,
  `inventory`, and most modern derive macros) failed at
  macro-expansion time with `` `CARGO_MANIFEST_DIR` env variable not
  set ``, blocking the compile-fail / compile-pass assertion before
  it could run. Issue #14.
- Relative `--manifest-path` values (notably the bare
  `--manifest-path Cargo.toml`) are now absolutized at session
  startup before the crate-root derivation runs. Previously,
  `Path::parent` of a single-component relative path returned
  `Some("")` instead of `None`, so the
  `.unwrap_or_else(|| ".".into())` fallback was bypassed and
  `CARGO_MANIFEST_DIR=""` could propagate to the per-fixture rustc —
  a path shape Cargo itself never emits. The fix matches Cargo's
  shape exactly (absolute, no symlink resolution). Issue #14 follow-up.

## [0.1.0-alpha.1] — 2026-05-11

First public release on crates.io. Pre-1.0 alpha — the CLI surface
(flag names, exit codes, verdict catalog, `manifest.json` schema, and
`[package.metadata.lihaaf]` schema) is the stable v0.1 contract; the
library API is non-stable and may shift across v0.1.x. Adopters
should subprocess-spawn `cargo lihaaf` rather than depend on
`lihaaf::*` paths from Rust.

### Added
- Initial v0.1 implementation per `docs/spec/lihaaf-v0.1.md`:
  cargo subcommand `cargo-lihaaf`, session-startup dylib build via
  `cargo rustc --crate-type=dylib --message-format=json-render-diagnostics`,
  per-fixture rustc dispatch with `--extern` linking, hand-rolled
  Myers diff (no regex), stdlib-only stderr normalizer, RAM-aware
  worker pool with OOM containment (Linux: `/proc/<pid>/statm`
  sampling), snapshot bless mode, `[package.metadata.lihaaf]` config
  parsing, fixture discovery with `compile_fail_marker` directory
  classification, full rustdoc on every public item.
- The full v0.1 specification at `docs/spec/lihaaf-v0.1.md`. The
  underlying mechanism (inventory propagation across the dylib
  boundary) was validated end-to-end in a research spike before any
  v0.1 code was written; outcome `GO_NATIVE`. The spec's §13 appendix
  records the contingency catalog for revalidation cadence.
- Self-test corpus under `tests/lihaaf/{compile_pass,compile_fail}/`
  with `[package.metadata.lihaaf]` self-references; CI runs
  `cargo lihaaf` against this corpus end-to-end on every push.
- New `freshness` module: per-dispatch re-check of the four spec
  §4.5 invariants (managed-dylib existence / mtime / SHA-256 /
  rustc release line). Drift surfaces as
  `Outcome::FreshnessDrift { invariant, detail }` mapped to exit
  code 67 (same class as `TOOLCHAIN_DRIFT`).
- New `ParallelismGate` permit pool driving the spec §5.4 dynamic
  parallelism reduction. Every harness-attributed OOM kill drops
  the cap by 1 (floor: 1) for all subsequent dispatches across
  every worker.
- New `FixtureWarning::LargeSnapshot { expected_lines, actual_lines }`
  variant + `lihaaf: LARGE_SNAPSHOT <path> (<expected>/<actual> lines)`
  reporter line for spec §7.2 complexity-ceiling soft-warning case.
- `--no-cache` (spec §8.2) is now wired: removes
  `target/lihaaf/manifest.json` and `target/lihaaf-build/` before
  stage 3 to fully bypass any prior cache + cargo's incremental.
- §3.3 aggregate counts line emitted alongside the wall-clock line:
  `lihaaf: <n> ok, <n> failed, <n> timeout, <n> memory_exhausted`.

### Changed
- Stderr UTF-8 validation is now strict per spec §7.2: invalid bytes
  surface as `MALFORMED_DIAGNOSTIC` with the precise byte offset
  (`Utf8Error::valid_up_to()`) instead of being silently substituted
  via `from_utf8_lossy`. Snapshot files validated the same way; the
  zero-offset placeholder in the snapshot path is replaced with the
  real offset.
- Normalizer no longer drops `error: aborting due to N previous
  error[s]` or `For more information about this error, try \`rustc
  --explain ...\``. Spec §6.3 preserves diagnostic text byte-for-
  byte; the prior trybuild-mimicking drops violated the §6.2 / §6.3
  enumeration. Existing adopters need to re-bless once.
- `-j 0` is now a clap parse error per spec §5.2 ("explicit is
  better"). The previous silent coercion to `-j 1` is gone along
  with the defensive `compute_parallelism` `.max(1)`.
- Session-temp parent directory is now preserved on
  `CLEANUP_RESIDUE` (spec §5.3 / §10.2), not just on
  `--keep-output`. The path emits on stderr at session end so
  adopters can find the residue.
- `toml` crate bumped 0.8 → 1.x.
- Unix `kill(2)` and `sysconf(_SC_PAGESIZE)` route through the
  `libc` crate instead of hand-rolled `extern "C"` blocks.
- Spec §4.5 amended to acknowledge the v0.1 hard-fail policy on
  freshness divergence and to defer the in-session rebuild path to
  v0.2. The four §4.5 invariants now share §4.6's hard-fail behavior
  (exit code 67) explicitly. Previously the spec mandated rebuild
  while the implementation hard-failed; this brings the spec text in
  line with shipping behavior + the deferral note in
  `src/freshness.rs` rustdoc. (Codex delta-review A3.)

### Pending before v0.1.0 release
- (Resolved in `0.1.0-beta.1`) macOS / Windows RSS sampling APIs are
  not yet wired (KR-5).
