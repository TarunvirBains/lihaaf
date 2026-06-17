# lihaaf

`lihaaf` is a fast compile-fail and compile-pass test harness for Rust proc
macros — a faster `trybuild`-style workflow for proc-macro crates with many
compile-fail / compile-pass fixtures.

## Why lihaaf?

`lihaaf` ("quilt"; Urdu and Punjabi, also written ਲਿਹਾਫ਼ in Gurmukhi) was inspired by
[Trybuild](https://github.com/dtolnay/trybuild) but driven by a need for
quick iteration: the compile-fail/compile-pass fixture style `trybuild` made
practical, combined with a build model that keeps adding fixtures cheap.

`lihaaf` is a CLI proc-macro test harness for Rust. It builds the consumer
crate as a dynamic library once per session, then dispatches each fixture to
`rustc` individually, linking the prebuilt dylib via `--extern`. For larger
fixture suites this usually means seconds instead of minutes, because fixtures
share that one build.

There's a short companion document in [`docs/spec/lihaaf-v0.1.md`](docs/spec/lihaaf-v0.1.md).
Most of the code aims to stay readable first, not process-centric.

## lihaaf vs trybuild

`trybuild` is the mature default for compile-fail / compile-pass testing in
Rust. `lihaaf` is for proc-macro authors who want faster iteration when their
fixture corpus grows — it builds the macro crate into a dylib once and
dispatches per-fixture rustc invocations against it, so adding the N+1th
fixture stays cheap.

**Measured adopter result** (`djogi-macros`, 237 fixtures):

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
workflow while making local iteration practical on large proc-macro suites.

> **Migrating an existing trybuild suite?** See [`docs/migrating-from-trybuild.md`](docs/migrating-from-trybuild.md) for the step-by-step playbook.

## Compile-fail tests

Place fixtures in `tests/lihaaf/compile_fail/`. Each `.rs` file is a
standalone Rust snippet expected to fail compilation. lihaaf compares the
normalized stderr output against a `.stderr` snapshot file with the same
stem. Run `cargo lihaaf --bless` the first time to write the snapshots.

## Compile-pass tests

Place fixtures in `tests/lihaaf/compile_pass/`. Each `.rs` file must compile
successfully when linked against the consumer crate's dylib. lihaaf reports
`OK` for each one that does and `EXPECTED_PASS_BUT_FAILED` for any that does
not.

## Proc macro test harness

lihaaf is purpose-built for proc-macro crates. The consumer crate is compiled
once as a `dylib` (`cargo rustc --crate-type=dylib`); every fixture
invocation then links that prebuilt dylib via `--extern`. This means:

- Proc-macro expansion runs against the real compiled macro implementation —
  no mocking, no simulation.
- Adding a new fixture does not re-invoke the macro-crate build.
- Multi-suite support lets you test the same macro with different Cargo
  feature subsets in a single session.

## Quick start

In the consumer crate's `Cargo.toml`:

```toml
[package.metadata.lihaaf]
dylib_crate = "consumer"
extern_crates = ["consumer", "consumer-macros"]
features = ["testing"]
dev_deps = ["serde", "serde_json"]
# Optional. Use when fixtures import crates from `dev_deps` directly and
# those crates must be built before the per-fixture rustc loop.
build_targets = ["tests"]
edition = "2021"
# Suppress rustc lints on each per-fixture invocation (forwarded as `-A <lint>`).
# Common entries under v0.1: unused_imports and dead_code, which fire under
# lihaaf's bare-rustc invocation when fixtures share scaffolding with the
# main crate. Other entries (e.g. unexpected_cfgs) become active when
# rustc is invoked with --check-cfg; today lihaaf does not pass --check-cfg.
allow_lints = ["unused_imports", "dead_code"]
# Note: allow_lints inherits from the default suite when omitted on a named suite.
# Optional — literal-substring substitutions applied after built-in path placeholders.
# extra_substitutions = [{ from = "/nix/store/...", to = "$NIX_STORE" }]
# Optional — drop full lines that exactly match any of these strings.
# strip_lines = ["For more information about this error, try `rustc --explain E0308`"]
# Optional — drop full lines that begin with any of these prefixes.
# strip_line_prefixes = ["note: this error originates from"]
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
| `--suite <NAME>` | Limit the run to the named suite(s) (repeatable). Without `--suite`, every defined suite runs in declared metadata order. See "Multi-suite". |
| `--list` | Print the fixtures that would run and exit, without building the dylib. Composable with `--filter`; for CI sharding. |
| `--no-cache` | Force a fresh dylib build, ignoring any existing manifest (across every suite). |
| `--manifest-path <path>` | Override the consumer `Cargo.toml` location. |
| `--quiet` / `-q` | Suppress per-fixture progress; only show non-OK verdicts. |
| `--verbose` / `-v` | Print each fixture's `rustc` command + captured stderr. |
| `--use-symlink` | Skip the lihaaf-managed dylib copy; symlink instead. Saves disk + time, but unsafe under concurrent cargo activity. |
| `--keep-output` | Preserve per-fixture work directories after verdict capture. Local-development debugging only — never set in CI. |
| `--package <NAME>` / `-p <NAME>` | Compat-mode workspace-member selector. Required when `--compat-root` points at a workspace root that declares `[workspace]` without `[package]`. Single package per invocation. |
## Filtering fixtures

You can use the `--filter` flag to select a subset of fixtures to run by path substring. The filter matches case-sensitively against the relative path of the fixtures.

### Examples

- **Single filter:** Run only fixtures whose path contains `compile_fail`:
  ```bash
  cargo lihaaf --filter compile_fail
  ```
- **Multiple filters (OR behavior):** Run fixtures whose path contains either `pass` or `fail` (multiple `--filter` flags are OR'd together):
  ```bash
  cargo lihaaf --filter pass --filter fail
  ```
- **Filter and list:** Combine `--filter` with `--list` to print all matched fixtures without building the dylib or invoking rustc:
  ```bash
  cargo lihaaf --filter parse --list
  ```

## Symlink execution and safety (`--use-symlink`)

By default, `lihaaf` copies the prebuilt dynamic library (dylib) of the consumer crate into a dedicated temporary directory before running the per-fixture test loop. This isolates test execution from concurrent modifications in your target directory.

Setting the `--use-symlink` flag replaces this copy operation with a symbolic link:

- **Disk/Performance Trade-off:** Saves approximately 30 MB of disk space per test session and shaves off a few hundred milliseconds of overhead.
- **Safety Requirement:** When enabled, the caller guarantees that **no concurrent Cargo activity** (e.g. background IDE compilation, automatic `cargo check` on save, concurrent CI jobs, or manual builds in other terminals) will modify or rebuild items in the `target/` directory.

> [!WARNING]
> If Cargo modifies the dylib in `target/` while the test loop is actively running, the symbolic link will resolve to a half-compiled or missing library, causing link errors or undefined behavior in parallel worker threads. Do not use `--use-symlink` in environments with active file watchers or concurrent build processes.
## Preserving output for debugging (`--keep-output`)

By default, `lihaaf` removes all temporary compilation work directories and generated staging files as soon as a fixture is evaluated. This prevents cluttering the disk.

If you pass `--keep-output`, `lihaaf` disables this automatic cleanup:

- **What is preserved:**
  - **Standard Mode:** The temporary compilation directory for each fixture (containing object files, dependency info, and build artifacts) is preserved under the system's temporary directory.
  - **Compat Mode:** The staged manifest overlay (under `target/lihaaf-overlay/`), sidecar files, and copied/converted test fixtures are preserved in the worktree.
- **Debugging Use Cases:** If a test fails in an unexpected way, you can navigate to the preserved directories, inspect the compiled outputs, check dependency configurations, or manually copy and execute the `rustc` command-line invocation to diagnose the issue.

> [!CAUTION]
> `--keep-output` is designed as a local-development escape hatch only. **Never enable `--keep-output` in CI systems or automated scripts**, as it prevents cleanup and will leak temporary files and build artifacts, eventually exhausting disk space.

## Blessing snapshots

When you intentionally change a procedural macro's error output—or when a new `rustc` toolchain version alters how compiler diagnostics are formatted—your snapshot tests will fail because your saved `.stderr` files no longer match.

To overwrite your existing snapshots with the current compiler output:

```bash
cargo lihaaf --bless
```

**Why "bless"?** The term traces back to the Rust compiler's own UI testing. When rustc developers change error formatting across thousands of test fixtures, they don't edit them by hand. They pass `--bless` to the compiler's build system (`./x test --bless`), which instructs the test harness to trust the new compiler output and overwrite the old snapshots. Snapshot-testing libraries in the ecosystem—like trybuild and insta—adopted this design philosophy. lihaaf explicitly exposes `--bless` to keep workflows idiomatic for Rust developers. When you pass the flag, you are acting as the authority—consecrating the current compiler output as the new golden master.

**Hygiene check**: Always run `git diff` after blessing. Ensure the changed text aligns with your macro edits, rather than masking an unintended compiler regression or panic.

## Compat mode

`cargo lihaaf --compat` is a migration and validation workflow for proc-macro crates that already have a [trybuild](https://github.com/dtolnay/trybuild) fixture corpus.

### What it does

Compat mode runs both halves of the comparison:
- **Baseline capture:** It runs the existing trybuild suite via `cargo test` and captures the outcome.
- **lihaaf execution:** It discovers trybuild fixtures from the AST, copies them to a temporary staging area, creates an overlay manifest under `target/` without touching your code or `Cargo.toml`, runs `cargo lihaaf`, and captures the verdicts.
- **Parity comparison:** It aggregates the results and outputs a byte-deterministic comparison report (`compat-report.json`) detailing matching outcomes, mismatches, errors, and exclusions.

### When to use it

- **Pre-migration validation:** Before modifying your codebase, use compat mode to check how closely lihaaf's diagnostic output matches trybuild.
- **Cross-toolchain validation:** Run the compat comparison across multiple Rust compiler versions (e.g. stable, nightly) to measure and verify diagnostic stability before fully converting.
- **CI Gating:** Run compat mode in a pull request or CI environment to guarantee that no behavioral or diagnostic changes are introduced during migration.

### Basic workflow

1. **Invoke the compat runner:** Run `cargo lihaaf` with the `--compat` flag, specifying the root of the target crate and the path where you want the JSON report written:
   ```bash
   cargo lihaaf \
     --compat \
     --compat-root <PATH-TO-CRATE-CHECKOUT> \
     --compat-report compat-report.json
   ```
2. **Handle workspace members:** If the target crate is a member of a virtual Cargo workspace, you must specify the workspace root as the root, and select the package via the `-p` / `--package` flag:
   ```bash
   cargo lihaaf \
     --compat \
     --compat-root /path/to/workspace-root \
     --package my-crate \
     --compat-report compat-report.json
   ```
3. **Inspect the comparison report:** Parse and inspect the generated `compat-report.json`. Check for any entries in `mismatch_examples` or `errors` to see if there are minor formatting differences or unrecognized AST patterns.
4. **Iterate:** If there are accepted diagnostic differences, you can use the standard `--bless` flag in combination with `--compat` to update or record them:
   ```bash
   cargo lihaaf --compat --compat-root . --compat-report compat-report.json --bless
   ```

See `docs/compatibility-plan.md` for the full compat-mode design.

## Multi-suite

Some adopters need to compile a SUBSET of their fixtures against a
DIFFERENT Cargo-feature set than the rest of the corpus — e.g. a
handful of `#[cfg(feature = "spatial")]`-gated compile-pass fixtures
that need `--features spatial` enabled, while the other ~233 fixtures
must continue to compile against the default no-feature set so
unrelated diagnostics stay deterministic.

Lihaaf supports this with named suites. The top-level
`[package.metadata.lihaaf]` table is always the implicit "default"
suite. Adopters add additional suites with
`[[package.metadata.lihaaf.suite]]` entries:

```toml
[package.metadata.lihaaf]
dylib_crate = "consumer"
extern_crates = ["consumer", "consumer-macros"]
fixture_dirs = ["tests/lihaaf/compile_fail", "tests/lihaaf/compile_pass"]
features = []  # default suite: no Cargo features

[[package.metadata.lihaaf.suite]]
name = "spatial"
features = ["spatial"]
fixture_dirs = ["tests/lihaaf/compile_pass_spatial"]
```

Each suite triggers an independent dylib build with its own feature
set. Per-suite manifest paths (`target/lihaaf/manifest-<name>.json`)
and per-suite cargo target dirs (`target/lihaaf-build-<name>/`) keep
incremental caches isolated. `cargo lihaaf` (no `--suite`) runs every
defined suite in declared order; `cargo lihaaf --suite spatial`
restricts to the named subset.

`build_targets = ["tests"]` is the opt-in path for fixtures that import
crates listed in `dev_deps` directly. For that suite, lihaaf synthesizes
a temporary isolated suite workspace: the staged dylib package points at
the real source with an absolute `[lib].path` and emits both `rlib` and
`dylib`, while a synthetic collector package depends on the explicit
`dev_deps`. This is one Cargo resolver graph per suite. Suites that omit
`build_targets` keep the default dylib-only build path.

Constraints (validated at config parse time):
- Suite names must be `[A-Za-z0-9_-]` and not equal to `default`.
- `fixture_dirs` must be disjoint across suites (no shared snapshot files).
- `dylib_crate` is not per-suite — one consumer crate per session.
- `features` does not inherit (a named suite that omits `features`
  gets `[]`, not the default suite's features).
- `build_targets` does not inherit. A named suite that omits it gets
  `[]`, even if the default suite opts into `["tests"]`.
- Other keys (`extern_crates`, `dev_deps`, `edition`,
  `compile_fail_marker`, `fixture_timeout_secs`,
  `per_fixture_memory_mb`, `allow_lints`) inherit from the top-level
  table when omitted on a named suite.

See `docs/spec/lihaaf-v0.1.md` §3.6 for the full design.

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
  lihaaf` subcommand. There is no `#[test]` integration;
  the `cargo test` scheduler would compromise lihaaf's parallelism,
  OOM containment, and drift detection.
- **Coverage / multi-target / IDE / watch.** Those are deferred on
  purpose. Each cut has a concrete reason and a future-trigger or
  explicit "never" classification.

## Tradeoffs and choices

A few implementation spots are intentionally open. The paths below
felt easiest to keep stable and debuggable in day-to-day use:

- **Cargo invocation for the dylib build**:
  `cargo rustc -p <crate> --lib --release --crate-type=dylib
  --message-format=json-render-diagnostics --target-dir=<lihaaf-build>`
  with `RUSTFLAGS="-C prefer-dynamic"` when `build_targets` is omitted.
  With `build_targets = ["tests"]`, lihaaf instead runs `cargo build`
  against the staged suite workspace described above, still using a
  dedicated suite target dir. The default path remains byte-stable for
  adopters that do not opt in.

- **File copy primitive**: `std::fs::copy`. It's plain and predictable:
  POSIX semantics on Linux/macOS, `CopyFileW` on Windows. Reflink is
  still deferred for v0.2.

- **Per-platform RSS sampling**:
  - Linux: `/proc/<pid>/statm` (2nd field × `sysconf(_SC_PAGESIZE)`).
    Verified against rustc child processes on `rustc 1.95.0` on Linux
    6.x x86_64.
  - macOS: `libc::proc_pidinfo(PROC_PIDTASKINFO)` — same semantics as
    the Linux path. Runaway workers correctly surface as
    `MEMORY_EXHAUSTED`.
  - Windows: `OpenProcess` + `GetProcessMemoryInfo` (via `windows-sys`).
    Runaway workers correctly surface as `MEMORY_EXHAUSTED`.

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
- `libc` 0.2 (Unix only) — `kill(2)` for worker termination,
  `sysconf(_SC_PAGESIZE)` for RSS unit conversion, and
  `proc_pidinfo(PROC_PIDTASKINFO)` for macOS RSS sampling. The canonical
  curated source for POSIX FFI signatures.
- `windows-sys` 0.59 (Windows only) — `OpenProcess` + `GetProcessMemoryInfo`
  for RSS sampling; `LockFileEx` / `UnlockFileEx` for the session lock.
- `syn` 2 — compat-mode fixture discovery via AST analysis (spec §3.2.1).
- `proc-macro2` 1 — span-location support for compat-mode discovery
  (line-number citations in `discovery_unrecognized` entries).

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
