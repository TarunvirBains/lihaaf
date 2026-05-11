# Changelog

All notable changes to lihaaf are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the
project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased] — v0.1.0 work-in-progress

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

### Pending before v0.1.0 release
- macOS / Windows RSS sampling APIs are not yet wired (KR-5); on
  those platforms v0.1 falls back to the OS OOMkiller and surfaces
  runaway workers as `WORKER_CRASHED` rather than `MEMORY_EXHAUSTED`.
