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
- The full v0.1 specification at `docs/spec/lihaaf-v0.1.md`.
- The inventory-on-dylib spike research artifact at
  `docs/research/2026-05-10-inventory-on-dylib-spike.md` (validated
  the dylib propagation path before any v0.1 code was written;
  outcome: GO_NATIVE).

### Pending before v0.1.0 release
- Codex Spark xhigh review identified four BLOCK findings and three
  lower-tier findings to address before the first tagged release: see
  the open follow-up commits and `docs/spec/lihaaf-v0.1.md` Known
  Risks (KR-1 through KR-5).
- macOS / Windows RSS sampling APIs are not yet wired (KR-5); on
  those platforms v0.1 falls back to the OS OOMkiller and surfaces
  runaway workers as `WORKER_CRASHED` rather than `MEMORY_EXHAUSTED`.
- `toml` crate bump from 0.8 to 1.x (one major version behind at
  initial commit).
