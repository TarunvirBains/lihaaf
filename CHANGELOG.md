# Changelog

All notable changes to lihaaf are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the
project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
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
- macOS / Windows RSS sampling APIs are not yet wired (KR-5); on
  those platforms v0.1 falls back to the OS OOMkiller and surfaces
  runaway workers as `WORKER_CRASHED` rather than `MEMORY_EXHAUSTED`.
