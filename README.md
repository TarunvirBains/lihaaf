# lihaaf

> **lihaaf** ("quilt", Urdu) — a Rust test harness purpose-built for
> fast, parallel, non-flaky compile-fail and compile-pass testing of
> proc-macros and macro-emitted code. Named after Ismat Chughtai's 1942
> short story.

`lihaaf` builds the consumer crate **once** as a Rust dynamic library
at session startup, then dispatches each fixture as a per-fixture
`rustc` invocation that links the dylib via `--extern`. Per-fixture
cost drops from cargo's full per-project rebuild (5–15 minutes on a
200-fixture corpus) to seconds because fixtures don't rebuild the
consumer; they link to it.

The full v0.1 specification lives at [`docs/spec/lihaaf-v0.1.md`](docs/spec/lihaaf-v0.1.md).

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

Full flag reference: spec §8.

## Verdicts and exit codes

Per spec §10. Every fixture produces exactly one verdict. The binary's
exit code is the maximum (most severe) of all per-fixture verdicts
plus session-level outcomes.

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
  lihaaf` subcommand. There is no `#[test]` integration (spec §11.5);
  the `cargo test` scheduler would compromise lihaaf's parallelism +
  OOM containment + drift detection.
- **Coverage / multi-target / IDE / watch.** All anchored deferrals in
  spec §11. Each cut has a concrete reason and a future-trigger or
  "never" classification.
- **A regex-engine consumer.** Zero regex-engine deps, ever (spec
  §6.1). The normalizer is hand-rolled byte-level matching.

## Implementer choices recorded in v0.1

The spec deliberately softened four sections so the implementer could
make calls. Here is what landed:

- **Cargo invocation for the dylib build** (§4.2):
  `cargo rustc -p <crate> --lib --release --crate-type=dylib
  --message-format=json-render-diagnostics --target-dir=<lihaaf-build>`
  with `RUSTFLAGS="-C prefer-dynamic"`. Validated end-to-end by the
  inventory-on-dylib spike (verdict `GO_NATIVE`; see spec §13). A
  dedicated target dir
  (`target/lihaaf-build/`) avoids thrashing the adopter's normal
  `cargo build` cache, since `RUSTFLAGS` is part of cargo's
  fingerprint.

- **File copy primitive** (§4.3): `std::fs::copy`. POSIX semantics on
  Linux/macOS, `CopyFileW` on Windows. The v0.2 reflink optimization
  remains an anchored deferral (spec §4.3 final paragraph).

- **Per-platform RSS sampling API** (§5.4 / KR-5):
  - Linux: `/proc/<pid>/statm` (2nd field × `sysconf(_SC_PAGESIZE)`).
    Verified against rustc child processes on `rustc 1.95.0` on Linux
    6.x x86_64.
  - macOS / Windows: returns `None` from the sampler. The OS
    OOMkiller / jetsam still backstops; a runaway worker on those
    platforms surfaces as `WORKER_CRASHED` rather than
    `MEMORY_EXHAUSTED`. v0.x lands proper macOS / Windows sampling
    APIs.

- **Sampling interval** (§5.4): 100 ms. Short enough to catch a
  runaway monomorphization before the OS OOMkiller fires; long enough
  that the sampler thread stays out of the worker's way.

- **Termination signal pair** (§5.4): SIGTERM, then SIGKILL after a
  2-second grace.

- **Diff algorithm** (§7.2): hand-rolled Myers diff (Eugene W. Myers,
  1986). Worst-case O((N+M)·D) where D is the edit-script length; for
  proc-macro stderr (10s–100s of lines, low edit distance when
  something changed) this is microseconds.

## Dependencies

- `clap` 4 — CLI parsing.
- `toml` 1 — `[package.metadata.lihaaf]` parsing.
- `serde` + `serde_json` — manifest write/read.
- `sha2` — dylib SHA-256 for the freshness check (spec §4.5).
- `tempfile` — per-session temporary directory.
- `libc` 0.2 (Unix only) — `kill(2)` for spec §5.4 worker
  termination and `sysconf(_SC_PAGESIZE)` for spec §5.4 RSS unit
  conversion. The canonical curated source for POSIX FFI signatures.

No regex engine. No diff library.

## Stability

The CLI surface (every flag in spec §8.2), exit codes (§10.3), and
snapshot byte format (§7.4) are part of the v0.1 stable surface.
Adding new flags / verdicts / normalization rules is non-breaking
across minor versions; removals or semantics changes are reserved for
v1.0.

The library API (`pub` items in this crate) is pre-1.0 and may evolve
before v1.0. Adopters who want to drive lihaaf from Rust today should
subprocess-spawn `cargo lihaaf`.

## License

Dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE)
at your option, the standard Rust ecosystem convention.
