# lihaaf

**lihaaf** ("quilt", Urdu) was inspired by
[Trybuild](https://github.com/dtolnay/trybuild) but driven by a need for
quick iteration: the compile-fail/compile-pass fixture style Trybuild made
practical, combined with a build model that keeps adding fixtures cheap.

`lihaaf` is a CLI proc-macro test harness for Rust. It builds the consumer
crate as a dynamic library once per session, then dispatches each fixture
to `rustc` individually, linking the prebuilt dylib via `--extern`. For
larger fixture suites this usually means seconds instead of minutes,
because fixtures share that one build.

There’s a short companion document in [`docs/spec/lihaaf-v0.1.md`](docs/spec/lihaaf-v0.1.md).
Most of the code aims to stay readable first, not process-centric.

## Measured adopter result

`djogi-macros` is the canonical adopter that drove the v0.1 design. In
the Phase 8.5 integration work, the same 237 proc-macro fixtures ran
through lihaaf successfully:

- `cargo lihaaf --list`: 237 fixtures discovered.
- `cargo lihaaf --filter compile_pass`: 99 OK in 27.5 seconds.
- `cargo lihaaf --filter compile_fail`: 138 OK in 15.0 seconds after
  blessing lihaaf-owned snapshots.
- Full lihaaf sweep: 237 OK in 31.8 seconds.

The existing trybuild fallback still passed on the same source corpus
(`trybuild_spatial_tests`: 1/1; `trybuild_tests`: 34/34), but the main
trybuild suite took 1942.56 seconds in the same validation pass. Exact
timings depend on hardware, target-dir state, and fixture shape; the
important result is that lihaaf preserves the compile-fail/compile-pass
workflow while making local iteration practical on large proc-macro
suites.

## Quick start

In the consumer crate's `Cargo.toml`:

```toml
[package.metadata.lihaaf]
dylib_crate = "consumer"
extern_crates = ["consumer", "consumer-macros"]
features = ["testing"]
dev_deps = ["serde", "serde_json"]
edition = "2021"
```

Layout:

```
your-crate/
├── Cargo.toml
├── src/lib.rs
└── tests/
    └── lihaaf/
        ├── compile_fail/
        │   ├── bad_attr.rs
        │   └── bad_attr.stderr
        └── compile_pass/
            └── happy_path.rs
```

Run:

```bash
cargo lihaaf
```

The first invocation builds the dylib (~10–20 s). Subsequent
invocations reuse the dylib and dispatch fixtures in parallel
(typically a 200-fixture sweep finishes in 10–20 seconds, even on a
laptop).

## Common flags

| Flag | Purpose |
|---|---|
| `--bless` | Overwrite `.stderr` snapshots whose normalized output differs. Equivalent: `LIHAAF_OVERWRITE=1`. |
| `--filter <substr>` | Run only fixtures whose relative path contains the substring (multiple flags OR'd). |
| `-j <n>` / `--jobs <n>` | Override worker parallelism. The harness still applies the RAM cap on top. |
| `--list` | Print the fixtures that would run and exit, without building the dylib. Composable with `--filter`; for CI sharding. |
| `--no-cache` | Force a fresh dylib build, ignoring any existing manifest. |
| `--manifest-path <path>` | Override the consumer `Cargo.toml` location. |
| `--quiet` / `-q` | Suppress per-fixture progress; only show non-OK verdicts. |
| `--verbose` / `-v` | Print each fixture's `rustc` command + captured stderr. |
| `--use-symlink` | Skip the lihaaf-managed dylib copy; symlink instead. Saves disk + time, but unsafe under concurrent cargo activity. |
| `--keep-output` | Preserve per-fixture work directories after verdict capture. Local-development debugging only — never set in CI. |

Flag behavior aligns with the v0.1 contract documented in the spec companion.

## Verdicts and exit codes

Every fixture produces exactly one verdict. The run exits with the most
severe code from the fixture verdicts and session-level outcomes.

| Verdict | Exit code |
|---|---|
| `OK` / `BLESSED` | 0 |
| `EXPECTED_FAIL_BUT_PASSED`, `EXPECTED_PASS_BUT_FAILED`, `SNAPSHOT_DIFF` | 1 |
| `SNAPSHOT_MISSING` | 2 |
| `TIMEOUT` | 3 |
| `MEMORY_EXHAUSTED` | 4 |
| `WORKER_CRASHED` | 5 |
| `SNAPSHOT_DIFF_TOO_LARGE` | 6 |
| `MALFORMED_DIAGNOSTIC` | 7 |
| `CLEANUP_RESIDUE` | 8 |
| `CONFIG_INVALID` | 64 |
| `DYLIB_BUILD_FAILED` | 65 |
| `DYLIB_NOT_FOUND` | 66 |
| `TOOLCHAIN_DRIFT` | 67 |

## What lihaaf is not

- **A `cargo test` replacement.** lihaaf ships as a separate `cargo
  lihaaf` subcommand. There is no `#[test]` integration yet;
  the `cargo test` scheduler would compromise lihaaf's parallelism +
  OOM containment + drift detection.
- **Coverage / multi-target / IDE / watch.** Those are deferred on
  purpose. Each cut has a concrete reason and a future-trigger or
  explicit "never" classification.

## Tradeoffs and choices

A few implementation spots are intentionally open. The paths below
felt easiest to keep stable and debuggable in day-to-day use:

- **Cargo invocation for the dylib build**:
  `cargo rustc -p <crate> --lib --release --crate-type=dylib
  --message-format=json-render-diagnostics --target-dir=<lihaaf-build>`
  with `RUSTFLAGS="-C prefer-dynamic"`. Validated end-to-end by the
  inventory-on-dylib spike (verdict `GO_NATIVE`). A dedicated target dir
  (`target/lihaaf-build/`) avoids thrashing the normal
  `cargo build` cache, since `RUSTFLAGS` is part of cargo's
  fingerprint.

- **File copy primitive**: `std::fs::copy`. It’s plain and predictable:
  POSIX semantics on Linux/macOS, `CopyFileW` on Windows. Reflink is
  still deferred for v0.2.

- **Per-platform RSS sampling**:
  - Linux: `/proc/<pid>/statm` (2nd field × `sysconf(_SC_PAGESIZE)`).
    Verified against rustc child processes on `rustc 1.95.0` on Linux
    6.x x86_64.
  - macOS / Windows: returns `None` from the sampler. The OS
    OOMkiller / jetsam still backstops; a runaway worker on those
    platforms surfaces as `WORKER_CRASHED` rather than
    `MEMORY_EXHAUSTED`. v0.x lands proper macOS / Windows sampling
    APIs.

- **Sampling interval**: 100 ms. Short enough to catch a
  runaway monomorphization before the OS OOMkiller fires; long enough
  that the sampler thread stays out of the worker's way.

- **Termination signal pair**: SIGTERM, then SIGKILL after a 2-second
  grace.

- **Diff algorithm**: hand-rolled Myers diff (Eugene W. Myers,
  1986). Worst-case O((N+M)·D) where D is the edit-script length; for
  proc-macro stderr (10s–100s of lines, low edit distance when
  something changed) this is microseconds.

## Dependencies

- `clap` 4 — CLI parsing.
- `toml` 1 — `[package.metadata.lihaaf]` parsing.
- `serde` + `serde_json` — manifest write/read.
- `sha2` — dylib SHA-256 for stale-state checks.
- `tempfile` — per-session temporary directory.
- `libc` 0.2 (Unix only) — `kill(2)` for worker termination and
  `sysconf(_SC_PAGESIZE)` for RSS unit conversion. The canonical curated
  source for POSIX FFI signatures.

## Stability

The CLI surface, exit codes, and snapshot byte format are part of the
v0.1 stable contract.
Adding new flags / verdicts / normalization rules is non-breaking
across minor versions; removals or semantics changes are reserved for
v1.0.

The library API (`pub` items in this crate) is pre-1.0 and may evolve
before v1.0. Adopters who want to drive lihaaf from Rust today should
subprocess-spawn `cargo lihaaf`.

## License

Dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE)
at your option, the standard Rust ecosystem convention.
