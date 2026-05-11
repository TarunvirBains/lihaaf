# lihaaf v0.1 — design specification

> **lihaaf** ("quilt", Urdu) — a Rust test harness purpose-built for fast,
> parallel, non-flaky compile-fail and compile-pass testing of proc-macros
> and macro-emitted code. Named after Ismat Chughtai's 1942 short story.

Status: design spec. No code yet. Target: a single-binary `cargo lihaaf`
subcommand publishable to crates.io from v0.1.

This document stands alone. It does not require any prior conversation or
internal note to be understood, and it is the authoritative design source
for the v0.1 implementation.

---

## 1. Goal and lens

### 1.1 Three adjectives

The harness exists to be **fast**, **parallel**, and **non-flaky** for one
specific workload: validating proc-macros by compiling small fixtures
against a real consumer crate and asserting either compile-time success
or a specific compile-time error.

**Bounded scope of "generic."** Lihaaf is generic *for proc-macro
testing* — it knows nothing about its consumers and can serve any
consumer crate that ships proc-macros and trybuild-shaped fixture
files. It is NOT generic for arbitrary test types. Specifically out of
scope (with anchored deferrals in Section 11):

- Runtime-behavior testing of compiled fixtures (Section 11.6 — lihaaf
  asserts compile outcome, not runtime behavior)
- Non-Rust fixture languages (Section 11.6 — `.rhai`, `.lua`, etc.
  belong to consumer-specific harnesses)
- Doc-test extraction
- Coverage instrumentation (Section 11.2)
- Multi-target / cross-compilation (Section 11.1)

The architecture and CLI defaults are tuned for the proc-macro
fixture shape (small files, one main, snapshot-based assertion).
Adopters needing other test types should compose lihaaf with the
right tool for that workload, not stretch lihaaf to fit.

- **Fast** — a 200-fixture run finishes in seconds on a developer laptop,
  not minutes. The architectural lever (Section 2) gets us there; nothing
  else does.
- **Parallel** — fixtures are mutually isolated and the harness
  saturates available cores up to a RAM-derived cap (Section 5).
- **Non-flaky** — silent wrong results are worse than loud failures.
  Mid-session toolchain drift, OOM, and any other ambiguity hard-fail
  with a clear verdict (Section 10). No retry loops that mask
  intermittent issues. The "non-flaky" claim is bounded by what the
  harness controls: deterministic verdict assignment for a fixture
  that runs to completion. KR-1 acknowledges that scheduler-sensitive
  edge cases (rapid OOM where the OS kills before lihaaf's RSS
  sampling fires, OOMkiller targeting a sibling process) can produce
  environment-dependent verdicts that lihaaf cannot eliminate without
  cgroups-level enforcement (v0.x). The promise is "no flake from
  lihaaf's own machinery"; OS-level resource pressure remains an
  honest external variable.

### 1.2 Decision lens

When tradeoffs surface, the harness chooses in this order:

1. **Scalability** — the architecture must handle 1000+ fixtures, allow
   parallelism limited only by available RAM, and stay fast as adopter
   crates grow.
2. **Production stability** — never produce a silent wrong result;
   hard-fail on ambiguity; CI failures must be reproducible byte-for-byte
   on a developer laptop with the same toolchain.
3. **Idiomatic Rust** — prefer stdlib primitives, hand-roll over heavy
   deps when the algorithm is well-understood and ~200 LOC, keep the
   dep tree small.
4. **Simple to use** — ergonomics matter, but never drive an
   architectural decision. Ergonomics is the residual after 1–3 are
   satisfied.

If the lens points one way and convenience the other, the lens wins.
Where this happens the spec calls it out.

### 1.3 Design assumptions

#### Concurrent cargo activity is the default environment

Lihaaf assumes that concurrent cargo activity in the same `target/`
directory is the **default operating environment**, not an edge case.

Agentic-development workflows — multiple Claude/Codex/AI agents working
in worktrees, IDE `cargo check` loops running in the background, manual
`cargo build` in a second terminal, parallel CI jobs sharing a warm
`target/` cache — make multi-writer `target/` directories the norm.
Tools that live in `target/` and assume single-writer semantics are
broken in this era.

Worktrees mitigate the problem (each worktree gets its own `target/`),
but not all setups use them, and even within a single worktree an IDE
check loop runs independently of the test harness. Lihaaf makes
architectural choices (Section 4.3, Section 5.3) that hold correctness
regardless of what cargo does to shared artifacts between fixture
dispatches.

Every decision in this spec that touches the dylib path, the manifest,
or per-fixture output directories is made with the "concurrent cargo is
normal" assumption active. When the spec creates a stable managed copy
and cleans up per-fixture artifacts, it is doing so because those are
the only choices consistent with this assumption. Symlink-default and
keep-output-default would be the wrong defaults for the same reason.

### 1.4 Non-goals

The harness does NOT try to:

- Replace `cargo test` for unit/integration tests on the consumer.
- Provide a workshop/REPL/watch loop (Section 11.4).
- Integrate with `#[test]` (Section 11.5).
- Cross-compile or test multiple targets in one invocation
  (Section 11.1).
- Produce HTML/JSON test reports (Section 11.3).
- Carry a regex-engine dependency (Section 6.1).

These cuts are explicit, anchored, and enable the architectural
simplifications the rest of the spec leans on.

---

## 2. Architecture overview

### 2.1 The single architectural choice

The dominant prior art (`dtolnay/trybuild`) treats each fixture as an
independent cargo project. cargo's per-project rebuild machinery
dominates wall-clock; the per-fixture rustc invocation pays full cost
for resolving the consumer crate's metadata; cache reuse with the parent
workspace is defeated because trybuild adds `--cfg trybuild` to
rustflags, which perturbs cargo's fingerprint hash. Per-fixture
parallelism is limited because each rustc rebuilds the consumer crate.
For a 200+ fixture corpus this routinely costs 5–15 minutes wall-clock
with high RAM pressure and lock-contention flake.

lihaaf's win comes from one architectural choice:

**The consumer crate is built ONCE as a Rust dynamic library at
session-startup. Each fixture is then compiled by invoking rustc directly
with `--extern <crate>=path/to/lib<crate>.so`, bypassing cargo entirely
for the fixture path.**

Fixtures don't rebuild the consumer; they link to it. Per-fixture cost
drops to seconds because the fixture is tiny (typically 5–30 lines) and
the consumer's full type/trait surface is already materialized in the
dylib. The bulk cost moves out of the per-fixture inner loop and into a
single up-front build cargo already knows how to do well.

### 2.2 Dataflow

```
user → cargo lihaaf
        │
        v
   ┌──────────────────────────────────────────────────────┐
   │ Session startup                                      │
   │  1. Read [package.metadata.lihaaf]                   │
   │  2. Capture rustc --version, sysroot, host triple    │
   │  3. cargo rustc consumer as dylib                    │
   │     Parse --message-format=json → dylib path         │
   │  4. Copy dylib → target/lihaaf/lib<crate>-current-*  │
   │     (--use-symlink: symlink instead; see §4.3)       │
   │  5. Refresh target/lihaaf/manifest.json              │
   │  6. Enumerate fixture .rs files                      │
   └──────────────────────────┬───────────────────────────┘
                              v
   ┌──────────────────────────────────────────────────────┐
   │ Worker pool                                          │
   │  parallelism = min(--j, RAM/per_fixture_memory_mb)   │
   │  per worker:                                         │
   │    rustc --edition <ed> --crate-type bin             │
   │          --error-format=json -L <deps>               │
   │          --extern <crate>=<managed_dylib> [--cfg …]  │
   │          fixture.rs                                  │
   │  capture stderr → parse JSON → normalize → diff      │
   │  per worker: RSS sampling, OOM containment           │
   │  after verdict: delete per-fixture workdir           │
   │  (--keep-output: skip deletion; see §5.3)            │
   └──────────────────────────┬───────────────────────────┘
                              v
   ┌──────────────────────────────────────────────────────┐
   │ Reporter                                             │
   │  per-fixture verdict line; aggregate counts;         │
   │  exit code per Section 10                            │
   └──────────────────────────────────────────────────────┘
```

### 2.3 Why a dylib (and not an `.rlib` with `--extern`)

`rustc` accepts `--extern <crate>=<path>` for both `.rlib` (static) and
`.so`/`.dylib`/`.dll` (dynamic). Dynamic wins on three grounds:

- **Linker-step cost.** With an rlib, each fixture re-links the
  consumer's monomorphized code. With a dylib, fixtures reference the
  consumer by symbol; the consumer's code lives in one place on disk
  and one place in RAM. For fixtures that touch a small slice of the
  consumer's surface (the common case), this is a measurable
  per-fixture win and a large RAM win across the worker pool.
- **Inventory propagation.** `inventory::submit!` and similar
  registration patterns rely on linker section attributes the dynamic
  linker collects at `dlopen` time. With per-fixture rlib linking
  every fixture re-links the consumer's full registration set; with a
  dylib it's authored once. This matches the property the parallel
  Rhai-shell consumer also depends on (Section 2.4).
- **Cache locality across fixtures.** A single `.so` is warm-cached
  after the first fixture; subsequent fixtures pay only the
  fixture-side rustc cost. Per-fixture rlib linking fights the OS
  page cache for the consumer's bytes on every link.

The cost of going dynamic is a build-time `crate-type = "dylib"` flip
and the spike property described next.

### 2.4 The two-consumer dylib model

lihaaf is one of two planned consumers of the "consumer crate as dylib"
pattern. The other is a Rhai-based interactive shell (a separate
project, not in scope here). Both depend on the same property: when the
consumer crate is built as a dynamic library, runtime registration
mechanisms (`inventory::submit!` and similar `dlopen`-friendly patterns)
must propagate items registered inside the dylib to consumers that link
the dylib.

A research spike validated this property end-to-end on 2026-05-10 and
returned the `GO_NATIVE` outcome: `cargo rustc --crate-type=dylib`
override works AND inventory propagates natively across the dylib
boundary, with no consumer-side `Cargo.toml` change required.

The spike's research note is at
`docs/research/2026-05-10-inventory-on-dylib-spike.md`.

Section 13 retains the full contingency catalog for revalidation
cadence and for adopters whose consumer crates differ from the
validated canonical adopter. None of the non-`GO_NATIVE` contingencies
affect the rest of the spec as written; they expand the configuration
surface or shift one step of the session lifecycle.

### 2.5 Disk footprint: lihaaf vs trybuild

The disk arithmetic motivates several v0.1 decisions — copy-default
for the dylib (Section 4.3), and per-fixture artifact cleanup regardless
of outcome (Section 5.3).

**Trybuild model.** Each fixture is compiled as a standalone cargo
project that statically links the consumer crate. On a typical ORM or
framework consumer, each fixture binary is roughly 50–100 MB. For a
237-fixture corpus this totals approximately 12–24 GB of persistent
disk from a single trybuild run. That is large enough to exhaust GitHub
Actions' default runner disk in one CI job — hence the "Free disk space"
step that large-corpus projects add to their GHA workflows.

**Lihaaf with copy-default.** The lihaaf-managed dylib copy is
approximately the size of one such binary (~30 MB for a typical consumer
crate). Cargo retains its own copy of the same artifact in
`target/release/deps/` (~30 MB). Together: ~60 MB of persistent disk,
across all fixtures, for the entire session. Per-fixture binaries are
deleted immediately after the verdict is captured (Section 5.3), so
the transient disk high-water mark during a 237-fixture run with
parallelism 8 is approximately 8 × a few MB for the tiny fixture
binaries — well under 100 MB total.

Net: roughly a 200–400x reduction in persistent disk versus trybuild's
static-link model. The GHA "Free disk space" step becomes unnecessary.

### 2.6 What the harness explicitly doesn't own

- A query/diff/snapshot-mgmt UI beyond the CLI (Section 8 + Section 7).
- Any opinion on the consumer's domain (the harness knows nothing about
  ORMs, web frameworks, parsers, or any other category).
- Any cross-process coordination — the harness is one process per
  invocation; workers are short-lived child rustc processes.
- The consumer's `Cargo.toml` content beyond the
  `[package.metadata.lihaaf]` table (Section 3) and the dylib build
  profile (Section 4).

---

## 3. Configuration surface

### 3.1 Where configuration lives

Adopters declare lihaaf configuration in the consumer crate's
`Cargo.toml` under `[package.metadata.lihaaf]`. There is no
`lihaaf.toml`, no `.lihaaf.json`, no env-only configuration path, no
auto-discovery from heuristics. If the table is missing, lihaaf
hard-errors:

```
error: lihaaf needs `[package.metadata.lihaaf]` to know what to build.
       Add the table to your Cargo.toml. See the lihaaf README for the
       minimum required keys.
```

This is deliberate. Auto-discovery hides build-graph decisions in
non-obvious places and produces "works on my machine" pathologies the
moment two adopters have slightly different layouts. Explicit
configuration is the cost of the architectural simplifications elsewhere.

### 3.2 Schema

```toml
[package.metadata.lihaaf]

# REQUIRED. Workspace member crate to build as the dylib.
dylib_crate = "consumer"

# REQUIRED. Crate names fixtures may `use ...::*` from. One
# `--extern <name>=<path>` flag is emitted per entry.
# extern_crates[0] MUST equal dylib_crate.
extern_crates = ["consumer", "consumer-macros"]

# DEFAULT: ["tests/lihaaf/compile_fail", "tests/lihaaf/compile_pass"].
# Directories scanned for *.rs fixtures (non-recursive within each).
fixture_dirs = ["tests/lihaaf/compile_fail", "tests/lihaaf/compile_pass"]

# DEFAULT: []. Cargo features enabled for both dylib build and per-
# fixture rustc invocation. Unifies the test-helper-behind-feature
# pattern (Section 3.5).
features = ["testing"]

# DEFAULT: "2021". Edition passed as `--edition <value>` to fixture
# rustc. One of "2015", "2018", "2021", "2024".
edition = "2021"

# DEFAULT: []. Extra crates beyond extern_crates that fixtures import
# directly (e.g., serde, serde_json). Resolved via cargo metadata and
# forwarded as `--extern` flags.
dev_deps = ["serde", "serde_json"]

# DEFAULT: "compile_fail". A fixture is compile_fail if its enclosing
# directory name (relative to crate root) contains this string;
# otherwise compile_pass. Directory-based to force one expectation
# per file via filesystem layout, not per-line annotations.
compile_fail_marker = "compile_fail"

# DEFAULT: 90. Per-fixture rustc wall-clock timeout in seconds.
# Exceeded → TIMEOUT verdict (Section 10), no retry.
fixture_timeout_secs = 90

# DEFAULT: platform-derived (Section 5.4). Max RSS in MB any single
# rustc worker may consume before being killed.
per_fixture_memory_mb = 1024
```

### 3.3 Worked example

For an ORM crate `consumer` with a sibling `consumer-macros` proc-macro
crate, fixtures live under `tests/lihaaf/compile_{fail,pass}/`, and a
test-only feature exposes private constructors:

```toml
[package.metadata.lihaaf]
dylib_crate = "consumer"
extern_crates = ["consumer", "consumer-macros"]
features = ["testing"]
dev_deps = ["serde", "serde_json"]
edition = "2021"
```

Adopter then runs:

```
cargo lihaaf
```

Output (success path):

```
lihaaf: building consumer dylib (cargo build --release)…  done in 4.2s
lihaaf: 237 fixtures discovered (compile_fail: 138, compile_pass: 99)
lihaaf: parallelism = 4 (RAM cap)
lihaaf: ........................................[40]
lihaaf: ........................................[80]
lihaaf: ........................................[120]
lihaaf: ........................................[160]
lihaaf: ........................................[200]
lihaaf: .....................................[237]
lihaaf: 237 ok, 0 failed, 0 timeout, 0 memory_exhausted
lihaaf: total wall-clock: 12.7s
```

### 3.4 Validation rules

The harness validates `[package.metadata.lihaaf]` at startup before
doing any work. Any of these conditions hard-error with a non-zero exit:

- `dylib_crate` missing or empty.
- `dylib_crate` does not name a workspace member or the current crate.
- `extern_crates` missing, empty, or `extern_crates[0] != dylib_crate`.
- `fixture_dirs` resolves to zero existing directories.
- `edition` not in the allowed set.
- `fixture_timeout_secs` not a positive integer.
- `per_fixture_memory_mb` (if set) not a positive integer.

Validation messages name the offending key, the allowed shape, and a
one-line "Why this matters" hint.

### 3.5 The `#[cfg(test)]` access pattern

cargo sets `cfg(test)` only for the crate-under-test, not its
dependents. Fixtures link the dylib as a regular dependency, so
`cfg(test)`-gated items in the consumer are unreachable from fixtures.

The convention: adopters gate test-only helpers behind a Cargo feature
(commonly `feature = "testing"`), list it in `features`. lihaaf builds
the dylib with the feature enabled and propagates the same `--cfg`
fragment to each fixture's rustc invocation.

lihaaf does NOT auto-detect a `testing` feature. Adopters must opt in
explicitly. Auto-enabling would split the dylib into two cache lines
(with and without the feature) and double build time the moment a
lihaaf run interleaves with a normal `cargo test` run.

---

## 4. Session lifecycle

### 4.1 Stages

A `cargo lihaaf` invocation runs these stages in order. Any stage's
failure is terminal — lihaaf does not skip ahead.

1. **Configuration load** — read and validate
   `[package.metadata.lihaaf]` (Section 3).
2. **Toolchain capture** — capture `rustc --version --verbose`:
   release string, host triple, sysroot path, commit hash. Persist
   for cross-stage equality checks.
3. **Dylib build** — build the consumer crate as a release-mode Rust
   dynamic library, using whichever cargo subcommand the implementer
   determines is most reliable, in a way that emits cargo's JSON
   message stream so that `compiler-artifact` messages can be parsed
   to recover the artifact path. See Section 4.2 for behavioral
   requirements; Section 13 records the spike-validated invocation.
4. **Dylib copy** — copy the cargo-emitted dylib from
   `target/<triple>/<profile>/deps/lib<crate>-<hash>.so` to a
   lihaaf-managed stable path `target/lihaaf/lib<crate>-current-<hash>.so`.
   See Section 4.3 for the full rationale and behavioral requirements.
   All subsequent fixture workers reference the COPY, never the
   cargo-managed original.
5. **Manifest refresh** — write `target/lihaaf/manifest.json`
   atomically (write `.tmp`, rename) capturing both the cargo dylib
   path and the lihaaf-managed copy path, SHA-256, mtime, rustc
   release, host triple, features, and a verbatim snapshot of
   `[package.metadata.lihaaf]`.
6. **Fixture discovery** — walk `fixture_dirs` non-recursively,
   collect `*.rs`, classify via `compile_fail_marker`, sort
   lexicographically for deterministic output.
7. **Worker pool dispatch** — spawn per Section 5.
8. **Result aggregation** — collect verdicts, render the report.
9. **Exit** — Section 10 dictates the code.

### 4.2 Cargo invocation for the dylib

The dylib build must satisfy these behavioral requirements:

- The consumer crate is built in release profile as a Rust dynamic
  library (`.so` on Linux, `.dylib` on macOS, `.dll` on Windows),
  without requiring any edit to the consumer's `Cargo.toml`.
- Cargo's JSON message stream (`--message-format=json` or equivalent)
  is captured so that `compiler-artifact` messages are available for
  artifact-path recovery.
- lihaaf finds the `compiler-artifact` message whose `target.name`
  equals `dylib_crate` and whose `target.kind` includes `"dylib"`,
  reads the `filenames` array, and selects the first entry matching
  the platform's dynamic-library extension.

If multiple `compiler-artifact` messages match, the last one wins
(cargo's normal "newest artifact" rule). If none match, lihaaf
hard-errors, printing both the cargo invocation used and the JSON
output verbatim so the adopter can reproduce the failure.

The implementer chooses the specific cargo subcommand (`cargo rustc`,
`cargo build`, or another) that most reliably satisfies the above
requirements. Section 13 records the spike-validated invocation as a
recommended starting point.

Release profile is non-negotiable for v0.1. Debug-mode dylibs are
faster on first build but slower on every steady-state run because of
larger size, longer initial linker step, and worse per-fixture rustc
time. v0.1 optimizes steady-state.

### 4.3 The dylib copy — rationale and mechanics

At session startup, immediately after cargo emits the dylib (stage 3),
lihaaf copies it to a lihaaf-managed path before any fixture dispatches.

**Why copy, not symlink.** Concurrent cargo activity in the same
`target/` directory is the default operating environment (Section 1.3).
A developer's IDE runs `cargo check` in a loop; a second terminal runs
`cargo build`; a parallel CI job shares the target directory. Any of
these can replace, delete, or partially overwrite the cargo-managed
dylib at `target/release/deps/lib<crate>-<hash>.so` between the moment
lihaaf builds it and the moment a fixture worker links it. A symlink
exposes every fixture in the session to this race — a fixture mid-link
when cargo replaces the original gets a torn read or a load-time error.
A copy isolates the session completely: lihaaf's fixture workers all
reference the same stable bytes regardless of what cargo does to its
own artifacts.

**Copy mechanics.** The copy destination is
`target/lihaaf/lib<crate>-current-<hash>.so` where `<hash>` is the
filename hash cargo embedded in the artifact name. The destination
directory is `target/lihaaf/`, created if absent. The copy is
unconditional on every session start — lihaaf does not check whether
the file at the destination is already identical. The implementer
chooses the file-copy primitive.

**Cost.** For a typical release dylib, the copy cost is on the order
of a few hundred milliseconds on a laptop with a warm page cache, and
disk usage roughly doubles the single-dylib footprint (cargo-managed
copy plus lihaaf-managed copy). These are characteristic calibration
anchors for reviewers, not behavioral commitments; actual numbers
depend on the platform, filesystem, and dylib size. The safety win
is accepted as worth this cost; the disk math comparison in Section
2.5 shows why absolute disk usage remains well below the trybuild
baseline regardless.

**`--use-symlink` opt-in.** When the caller can assert that no
concurrent cargo activity will modify `target/` during the session
(typical on single-process CI runners with dedicated build directories),
`--use-symlink` skips the copy and creates a symbolic link instead.
This saves the copy cost and the disk doubling. The safety contract is
on the caller: lihaaf will NOT detect concurrent cargo activity when
`--use-symlink` is set, and mid-session dylib replacement will produce
`WORKER_CRASHED` or wrong verdicts. The flag documents this constraint
explicitly. Never set in CI unless you can guarantee serial cargo access.

**v0.2 optimization (not in v0.1).** CoW platforms (Linux on btrfs or
XFS with reflink support, macOS APFS, Windows ReFS) support clone
operations via `ioctl(FICLONE)` / `clonefile()` /
`FSCTL_DUPLICATE_EXTENTS_TO_FILE` that give copy semantics at symlink
cost — the kernel deduplicates physical pages until one side is
modified. v0.1 uses a plain file copy for simplicity and broad platform support.
v0.2 may detect CoW support and use it when available.

**What lihaaf does NOT do.** It does not acquire an exclusive flock on
the cargo-managed dylib, because cargo does not honor advisory flock
for its output files on Linux. The copy-default architecture makes the
lock approach unnecessary — the session's fixture workers never touch
the cargo-managed original after the copy completes.

### 4.4 Manifest schema

`target/lihaaf/manifest.json`:

```json
{
  "lihaaf_version": "0.1.0",
  "rustc_release": "rustc 1.85.0 (0123456789 2026-01-15)",
  "rustc_commit_hash": "0123456789",
  "host_triple": "x86_64-unknown-linux-gnu",
  "sysroot": "/home/u/.rustup/toolchains/1.85.0-x86_64-unknown-linux-gnu",
  "dylib_crate": "consumer",
  "cargo_dylib_path": "/path/to/target/release/deps/libconsumer-abc123.so",
  "managed_dylib_path": "/path/to/target/lihaaf/libconsumer-current-abc123.so",
  "dylib_sha256": "abc123...",
  "dylib_mtime_unix_secs": 1746883200,
  "use_symlink": false,
  "features": ["testing"],
  "extern_crates": ["consumer", "consumer-macros"],
  "edition": "2021",
  "metadata_snapshot": {
    "...": "verbatim copy of [package.metadata.lihaaf] for drift detection"
  }
}
```

`cargo_dylib_path` is the cargo-emitted original. `managed_dylib_path`
is the lihaaf-managed copy (or symlink when `--use-symlink` is active).
Fixture workers always receive `managed_dylib_path`.

### 4.5 Freshness validation

Before each fixture worker dispatches, the harness re-checks four
invariants against the in-memory manifest captured at startup:

- The lihaaf-managed dylib file at `managed_dylib_path` exists.
- Its mtime has not moved backward (a backward jump implies clock
  skew or external file replacement of the managed copy itself).
- Its SHA-256 still matches `dylib_sha256` (defensive against
  accidental modification of the managed copy).
- `rustc --version --verbose` still produces the same release line.

ANY divergence → blow the cache, re-run from stage 3 (dylib build),
re-copy, re-validate, then proceed. No "try anyway" fallback.
Re-validation is cheap (stat + SHA-256 over a page-cache-warm artifact
+ a short subprocess) — the blast radius of a stale dylib (silent ABI
mismatch producing wrong test results) makes paying that cost on every
dispatch the right call. For a typical 10–50 MB dylib the SHA-256
takes ~30 ms on a laptop because the bytes saturate memory bandwidth.

Note: the freshness check covers the MANAGED copy, not the
cargo-managed original. Cargo may freely replace its own artifact
between fixture dispatches — that is the whole point of the copy.

### 4.6 Hard-fail on rustc drift

If `rustc --version --verbose` at fixture-dispatch time differs from
the version captured during dylib build (Section 4.1 stage 2), lihaaf
MUST refuse to run any further fixtures and exit with the
`TOOLCHAIN_DRIFT` failure mode (Section 10). The exit message names
both versions and recommends a fresh `cargo lihaaf` invocation.

Rustc does not promise ABI stability across versions. A fixture binary
linking a dylib built by a different rustc has two failure modes:
load-time crash (loud, survivable) or silent miscompilation (quiet,
catastrophic). Hard-fail is the only correct policy; the JVM and
CPython landed on the same answer for `dlopen` ABI mismatch decades
ago for the same reason.

### 4.7 What lihaaf does NOT cache

lihaaf does not cache:

- Per-fixture results across runs. Every invocation runs every
  selected fixture from scratch. (Future work, anchored: see
  Section 11.)
- Diagnostic JSON parses across runs.
- Normalized stderr across runs (the dylib could change; normalization
  is cheap anyway).

The single thing lihaaf caches across runs is the dylib (cargo's
caching, not lihaaf's).

---

## 5. Worker dispatch

### 5.1 Model

Each fixture is one worker. A worker is one rustc child process invoked
by lihaaf, plus the harness-side bookkeeping needed to capture stderr,
parse the JSON diagnostic stream, sample RSS, enforce the timeout, and
emit the verdict. Workers do not share any in-process mutable state
with the harness or with each other.

### 5.2 Parallelism

The default parallelism cap is the smaller of two values:

- **CPU cap** — `std::thread::available_parallelism()`.
- **RAM cap** — `total_ram / per_fixture_memory_mb`, with `total_ram`
  read from `/proc/meminfo` on Linux, `sysctl hw.memsize` on macOS,
  and `GlobalMemoryStatusEx` on Windows.

Defaults are deliberately conservative. A runaway worker cohort
triggering OOMkiller (Linux) or jetsam (macOS) produces cross-fixture
flake — fixtures that "should pass" are killed for being adjacent to
a fixture with a runaway template instantiation, taking hours to
diagnose. The CPU cap protects RAM-rich machines; the RAM cap
protects RAM-poor machines.

Adopters override with `-j <n>` on the CLI. `-j 0` is rejected (no
implicit "cargo-style" "use all cores" semantics; explicit is better).

### 5.3 Per-fixture invocation and artifact cleanup

The rustc invocation must include the following flags, each pinned to
a behavioral requirement:

```
rustc
    --edition <edition>
    --crate-type bin
    --error-format=json
    -o <per_fixture_workdir>/<fixture_stem>
    -L dependency=<deps_dir>
    --extern <crate1>=<managed_dylib_path>
    --extern <crate2>=<rlib_or_dylib_path>
    [--cfg feature="<feat1>"]
    [--cfg feature="<feat2>"]
    <fixture.rs>
```

Each flag carries a specific behavioral requirement that makes it
non-negotiable: `--edition` matches the consumer's declared edition;
`--crate-type bin` compiles the fixture as an executable so rustc
performs full type-checking (not just syntax); `--error-format=json`
is required so lihaaf receives structured diagnostics rather than
plain text (see the "Where:" bullets below for the others). The
implementer composes the invocation from these requirements — the
block above is not a script to transcribe.

Where:

- `<managed_dylib_path>` is the lihaaf-managed copy from Section 4.3,
  NOT the cargo-managed original. All fixture workers use the same
  stable copy; cargo can freely touch its own artifact without
  affecting any in-progress fixture.
- `<per_fixture_workdir>` is a per-fixture subdirectory under a
  per-session temporary directory, created before the rustc spawn.
- `<deps_dir>` is `target/release/deps` (cargo populates this during
  the dylib build).
- Each `--extern` after the first resolves through `cargo metadata`
  output captured during stage 3 — for each crate name in
  `extern_crates ++ dev_deps`, find the matching `Resolve` node, read
  its compiled artifact path (`.rlib` or `.so`).
- `--error-format=json` is non-negotiable. It gives lihaaf structured
  diagnostics with stable spans, allowing normalization without text
  parsing. The plain-text rendering (which is what the snapshot stores)
  is reconstructed from the JSON `rendered` field per diagnostic.

**Artifact cleanup is immediate and unconditional.** After each
fixture's verdict is captured — regardless of whether the fixture
passed, failed, timed out, or crashed — lihaaf immediately removes the
per-fixture work directory, including the fixture binary, intermediate
object files, the fixture's `.fingerprint` data, and any other
rustc-emitted files. This is not a best-effort cleanup at session exit;
it happens per-fixture, right after the verdict is emitted.

The reason cleanup is unconditional and immediate is that the failure
cases are when disk pressure matters most. A CI run with 50 failing
fixtures that retains their binaries leaves behind the same disk
footprint as the trybuild model lihaaf exists to avoid. The verdict
printed to stdout (and captured in any future `--report` output) is
the persistent output. The binary is throwaway.

**Cleanup-failure policy.** When the per-fixture `fs::remove_dir_all`
call returns an error (permission denied, filesystem error, foreign
process holding a file open, transient I/O failure), lihaaf:

1. Records the cleanup error against that fixture's verdict slot — the
   fixture's pass/fail outcome is unchanged, but a `CLEANUP_FAILED`
   diagnostic is appended (path of the workdir + the OS error).
2. Continues the run. A cleanup failure on one fixture does NOT abort
   the session.
3. At session end, if any cleanup errors accumulated, lihaaf prints
   the surviving paths and exits with a distinct exit code
   (`CLEANUP_RESIDUE` — Section 10) so CI can flag the residue without
   confusing it with a fixture-verdict failure. The fixture verdicts
   themselves still determine pass/fail; the cleanup-residue exit
   code is OR'd in.
4. The session-temp parent directory is NOT removed at session end if
   it contains residue from a `CLEANUP_FAILED` fixture — leaving the
   residue visible is more useful than silently retrying a removal
   that already failed once.

This policy makes a leaky filesystem visible without distorting the
verdict signal that adopters consume.

**`--keep-output` opt-in.** When `--keep-output` is set, lihaaf
preserves all per-fixture work directories for the duration of the run.
This is a local-development escape hatch for when an engineer needs to
inspect a binary or re-invoke rustc with different flags on a specific
fixture. Never set in CI — it defeats the disk footprint guarantee.
The preserved directories live under the per-session temporary
directory and are printed to stderr at session end so the engineer
knows where to look.

The per-session temporary directory (parent of all per-fixture work
directories) is removed unconditionally at session exit in all cases
EXCEPT when `--keep-output` is set.

### 5.4 OOM containment (v0.1 requirement)

OOM containment is in v0.1 because deferring it would cause exactly
the cross-fixture flake Section 5.2 warns about, on the very runs
where adopters set `-j` aggressively for throughput.

Mechanism:

- After spawn, lihaaf samples each worker's per-process resident set
  size at a short interval suited to catching runaway allocation before
  the OS OOMkiller fires. The interval must be short enough to be
  meaningful relative to Section 5.2's cross-fixture-flake threshold;
  the implementer chooses the specific value and the platform API used
  to read per-process RSS on each target OS. Per-platform API selection
  requires care — for example, an API that returns only cumulative
  statistics for already-terminated children rather than live per-child
  RSS would silently fail to enforce the ceiling (see KR-5). The
  implementer is responsible for validating that the chosen API returns
  live per-process RSS on each target platform.
- If RSS exceeds `per_fixture_memory_mb`, the worker is terminated via
  the platform's graceful-then-forceful signal pair (the implementer
  picks the mechanism per platform). At the moment the termination
  signal is sent, lihaaf records a per-worker `harness_initiated_kill =
  true` flag — this flag is the load-bearing piece of attribution data
  for the verdict classification below.
- The fixture is marked needs-retry; parallelism is dynamically reduced
  (floor: 1); the fixture re-dispatches serially.
- If the serial retry also OOMs (i.e., the harness sent a kill signal
  again), the verdict is `MEMORY_EXHAUSTED` (Section 10) and the run
  continues at the reduced parallelism.

**OOM attribution heuristic.** The kill-retry path fires only when
lihaaf's own RSS-ceiling check triggered the kill
(`harness_initiated_kill == true` on the worker that exited). When a
worker exits with a kill signal that lihaaf did NOT initiate — external
`SIGKILL` from the OS OOMkiller (Linux), `jetsam` (macOS), parent shell
`kill -9`, scheduler-OOM targeting a sibling process, or any other
source — the verdict is `WORKER_CRASHED` (Section 5.5), NOT
`MEMORY_EXHAUSTED`, and the kill-retry path does NOT fire. This is the
honest classification: lihaaf knows it killed the worker only when it
sent the signal itself; everything else is crash territory whose root
cause lihaaf cannot reliably attribute to memory pressure. KR-1
documents the resulting gap (rapid OOM where the OS kills before
lihaaf's 100 ms sampling can fire surfaces as `WORKER_CRASHED` with no
lihaaf-side ceiling event); the v0.x cgroups-based memory limits on
Linux are the path to closing it.

Default `per_fixture_memory_mb` is a platform-derived sane default
for typical proc-macro fixtures (see Section 3.2). The implementer
chooses a formula that scales appropriately with available RAM and the
active parallelism level — for orientation, a value in the range of
several hundred MB to a few GB is typical. Adopters override the
config key when their fixtures legitimately need more (generic-heavy
fixtures whose monomorphization expands hugely).

### 5.5 Non-OOM crash isolation

If a worker process exits with a signal (SIGSEGV, SIGABRT) or a
non-rustc exit code, the fixture is marked with a `WORKER_CRASHED`
verdict naming the signal/code. Other workers continue uninterrupted —
the harness explicitly does NOT abort the run on a single worker
crash. This is the same posture as `cargo test` and avoids one buggy
fixture poisoning a 200-fixture run.

### 5.6 Timeout

`fixture_timeout_secs` (default 90) bounds wall-clock time for the
worker process. On exceedance, lihaaf sends `SIGTERM` then `SIGKILL`
exactly as the OOM path does. Verdict: `TIMEOUT`. No retry — timeouts
are usually load-bearing signals about a fixture's structure (e.g.,
infinite trait recursion), not noise to be papered over.

### 5.7 Determinism

Within a single invocation, with `-j 1`, fixture verdicts are emitted
in lexicographic relative-path order. With `-j > 1`, verdicts are
emitted in completion order, but the final aggregate report sorts them
back to lexicographic order. Adopters who need wall-clock-deterministic
ordering (e.g., for CI-side log diffing) use `-j 1`.

---

## 6. Stderr normalization

### 6.1 The no-regex constraint

The harness MUST NOT depend on any Rust regex engine — `regex`,
`regex-lite`, `fancy-regex`, `regex-automata`, or any wrapper. Adopter
projects routinely carry no-regex policies that extend through dev-dep
trees, and lihaaf is intended to land in dev-deps; the constraint
preserves that compatibility from the start.

The constraint also stands on its own merits independent of any
adopter policy. Compile-fail testing is a tight inner loop and the
normalization step runs on every fixture's stderr; pulling in a regex
engine for what amounts to fixed-string substitution and a handful of
well-shaped scans bloats the binary, the build time, and the audit
surface for negligible gain.

### 6.2 What gets normalized

rustc stderr contains environment-dependent text that would otherwise
make snapshots non-portable across machines, toolchain versions, and
working directories. The normalizer rewrites these to placeholders.
The categories the harness handles in v0.1:

- **Absolute paths to the fixture's directory** → `$DIR`. Any path
  prefix matching the fixture directory (the directory holding the
  fixture `.rs` file) becomes `$DIR/`.
- **Workspace-root paths** → `$WORKSPACE`. Any path prefix matching
  the consumer crate's workspace root.
- **rustc sysroot paths** → `$RUST`. Any path prefix matching the
  captured sysroot (Section 4.1 stage 2).
- **Cargo registry paths** → `$CARGO/registry/`. Any path prefix
  matching `<CARGO_HOME>/registry/`.
- **Backslashes in paths** → forward slashes. Windows path output is
  rewritten to POSIX form so snapshots are byte-identical across OS.
- **Line endings** — `\r\n` → `\n` everywhere.
- **TypeId hashes** in error messages (the `#0`-suffixed hash that
  rustc emits in some `expected … found …` messages) → the literal
  `$TYPEID`.
- **Trailing whitespace** on each line → stripped.
- **Multiple blank lines** → collapsed to one.

### 6.3 What does NOT get normalized

- **Diagnostic text** — error codes, message text, span pointers
  (`^^^`), help text. Preserved byte-for-byte; a change in rustc's
  wording IS a snapshot change and adopters use `--bless` to accept
  it. Hiding wording drift would weaken the snapshot signal.
- **Line and column numbers in spans** — pinned by fixture content.
  Editing a fixture shifts line numbers and the snapshot
  legitimately needs re-blessing.
- **Suggestion text** ("help: there is a method `foo` with a similar
  name") — preserved. Part of a fixture's observable behavior;
  adopters often pin the suggestion shape.

### 6.4 The byte-level approach

The normalizer is a behavioral contract over the categories in §6.2
and §6.3. The implementation must:

- Use stdlib primitives only — no regex engine of any kind (see §6.1).
- Enforce a **longest-prefix-wins** rule: when multiple path-prefix
  matchers could fire on the same substring, the longest matching
  prefix takes priority. For example, a substring beginning with
  `$WORKSPACE/target/release/deps/` must resolve to its own
  placeholder before the shorter `$WORKSPACE/` matcher would. This
  rule is behavioral — it determines which placeholder appears in the
  output when prefix matchers overlap.
- §6.2 and §6.3 are the contract for which substrings are rewritten
  and which are not. The implementer chooses data structures,
  iteration order, and matching strategy subject to these contracts.

The TypeId rewrite must replace every occurrence of `#` followed by
one or more ASCII digits on a given line with the literal `$TYPEID`.
This is the required behavior; the implementer chooses the
traversal.

### 6.5 Determinism contract

For a `(fixture, dylib SHA-256, rustc release, OS)` tuple, the
normalized stderr is byte-deterministic. The normalizer has no
hidden state, no env-dependent paths it doesn't capture, and no
wall-clock dependency. Snapshots survive cross-machine reruns when
the toolchain matches.

The OS qualifier is honest: rustc messages can mention `\\` inside
strings (a fixture containing `\\` in source) and the normalizer
does NOT rewrite those — only path-shaped substrings on `--> ` /
`::: ` lines. v0.1 documents this; v0.x adds per-platform snapshot
files (`.stderr.linux`, `.stderr.macos`, `.stderr.windows`) if
adopters need true cross-OS portability for fixtures that lean on
paths inside strings.

---

## 7. Snapshot diff and bless

### 7.1 The snapshot file

Each compile_fail fixture has a sibling `.stderr` file with the
expected normalized stderr. Compile_pass fixtures have no `.stderr`
sibling; the assertion is "rustc exits 0" and the rendered diagnostics
(if any — warnings) are not compared against a snapshot. Compile_pass
fixtures with warnings still pass; warning-aware compile_pass is a
v0.x extension (Section 11).

A missing `.stderr` for a compile_fail fixture is treated as
"snapshot empty"; the diff will produce a full insertion and the
adopter is expected to bless. The harness does NOT auto-create a
`.stderr` from the first run without `--bless`; that would silently
codify whatever rustc happened to emit on the first invocation.

### 7.2 The diff algorithm

When normalized stderr diverges from the snapshot, the harness prints
a unified diff and exits non-zero (Section 10).

The diff algorithm must satisfy these behavioral requirements:

- **Line granularity only** — never word- or character-granularity.
- **GNU-compatible unified diff output** — headers in the form
  `--- expected\n+++ actual\n@@ -<a>,<b> +<c>,<d> @@\n`, with `-`
  /`+`/` ` line prefixes. Adopters pipe through their normal diff
  pretty-printers; this header format is the contract for the
  diff-rendering pipeline adopters consume.
- **No regex dependency.** No transitive dependency on a heavy diff
  crate; the behavioral motivation is keeping the dep tree small and
  build times low.
- **Worst-case runtime bounded** per the §7.2 complexity ceiling
  (10K soft / 100K hard line ceilings, specified below).

The algorithm choice is the implementer's. A hand-rolled Myers diff
(Eugene W. Myers, 1986) is likely to remain the right pick given the
line-count range of typical proc-macro stderr, but the spec does not
foreclose simpler approaches that meet the contracts above.

**Complexity ceiling.** Myers diff worst-case time is O(N·D) where N
is the total line count and D is the edit distance. For typical
proc-macro stderr (10s to low hundreds of lines, low edit distance
when something changed), this is microseconds. To prevent a
pathological fixture (a multi-thousand-line stderr from a deeply
recursive macro expansion or a generated-code dump) from holding the
worker pool, lihaaf enforces a soft input ceiling of 10,000 lines per
side and a hard ceiling of 100,000 lines:

- At or below the soft ceiling: full Myers diff runs normally.
- Between soft and hard ceiling: the diff still runs, but the harness
  emits a `LARGE_SNAPSHOT` warning naming the fixture and the line
  count.
- Above the hard ceiling: the diff is skipped; the verdict is
  `SNAPSHOT_DIFF_TOO_LARGE` (Section 10) and the harness emits the
  first 100 lines of each side plus the line counts. Adopters with
  legitimate use for snapshots that large should split the fixture
  rather than tune lihaaf to handle them.

**Non-UTF-8 / binary-content handling.** Rustc's `--error-format=json`
emits well-formed UTF-8 strings for the `rendered` field; the
upstream invariant is that diagnostic text is UTF-8 with the BOM
stripped. Lihaaf treats any byte sequence in the `rendered` field
that fails UTF-8 validation as a fixture-side anomaly: the verdict is
`MALFORMED_DIAGNOSTIC` (Section 10), the diff step is skipped, and
the raw byte count + first invalid byte offset are included in the
diagnostic. The same policy applies to the snapshot file — if it
fails to parse as UTF-8 (e.g., a stray control byte was committed),
lihaaf rejects the comparison rather than silently doing a byte-wise
diff that would produce nonsense output. Mixed line endings
(`CRLF` / `LF` / `CR`) are normalized to `LF` before line-splitting,
matching the snapshot byte-determinism rule in Section 7.4.

### 7.3 `--bless` semantics

With `--bless`, the harness:

1. Runs every selected fixture normally.
2. For each compile_fail fixture whose normalized stderr differs from
   the snapshot, OVERWRITES the snapshot.
3. For each compile_pass fixture that now fails compilation, DOES NOT
   bless — there is no snapshot to overwrite, and silently flipping
   "pass" to "fail" is wrong. The verdict is the normal compile_pass
   failure.
4. Reports each blessed file by relative path on stderr.

`LIHAAF_OVERWRITE=1` is exactly equivalent to `--bless`. Both
accepted; both have the same effect. The env-var form exists for
CI-side scripts that prefer injecting env to rewriting invocations.

`--bless` is destructive — it overwrites checked-in `.stderr` files
without confirmation. The harness assumes adopters have version
control and review diffs before committing. There is no "bless into a
sidecar for review" mode in v0.1; straightforward to add later.

### 7.4 Snapshot byte-determinism

Snapshots are written with LF line endings on every platform and a
final newline. They are rewritten in full (no append; no in-place
edit). This guarantees adopter `.stderr` files are byte-identical
across reruns and across OS, making them safe to commit and review in
PRs without git's CRLF normalization fighting back.

### 7.5 What about compile_pass diagnostics?

Compile_pass fixtures may emit warnings. v0.1 ignores these — the
verdict is purely "rustc exited 0". v0.1 omits warning-aware
compile_pass because the dominant macro-testing workload is "macro
emits valid Rust that type-checks," and adopters who want to pin
warning text today can write a compile_fail fixture with
`#![deny(...)]` at the top.

If demand surfaces, v0.x adds a `compile_pass_warnings_*` directory
convention parallel to compile_fail with the same snapshot-and-bless
flow.

---

## 8. CLI

### 8.1 Invocation

The harness ships one binary, `cargo-lihaaf`, conforming to the cargo
subcommand convention (the binary is invoked as `cargo lihaaf` and
cargo passes `lihaaf` as the first argument; the binary strips it).

```
cargo lihaaf [OPTIONS]
```

There are no subcommands in v0.1. Every operation is a flag on the
single top-level invocation. This keeps the surface small and the
semver story simple: a flag may be added across minor versions; a
flag may be deprecated (with warning) but not removed across minor
versions; semantics of a flag may change only across major versions.

### 8.2 Flags

Each flag below is part of the v0.1 stable surface and follows the
semver discipline above.

#### `--bless`

Overwrite `.stderr` snapshots whose normalized output differs from
disk. See Section 7.3. Equivalent env: `LIHAAF_OVERWRITE=1`.

#### `--filter <substr>`

Run only fixtures whose relative path (from crate root) contains the
literal substring. Multiple `--filter` flags are OR'd. Substring
match (not glob, not regex), case-sensitive. The match is against the
full relative path, not just the file stem — `--filter phase7/`
selects a phase directory, `--filter phase7_jsonb` selects a subset.

#### `-j <n>`, `--jobs <n>`

Override the worker parallelism cap. `<n>` must be a positive
integer. The harness uses `min(n, RAM_cap)` — the explicit override
does NOT bypass the RAM cap (Section 5.2). Adopters with genuine RAM
headroom should raise `per_fixture_memory_mb`, which lifts the cap.

#### `--no-cache`

Force a fresh dylib build, ignoring any existing manifest. Equivalent
to deleting `target/lihaaf/manifest.json` before invocation.

#### `--manifest-path <path>`

Override the consumer `Cargo.toml` location. Default is cargo's
normal "current directory + parent walk" lookup. Useful in CI.

#### `--list`

Print the fixtures the harness would run, one relative path per line,
and exit 0. Does not build the dylib, does not invoke rustc.
Composable with `--filter`. For CI-side sharding.

#### `--quiet`, `-q`

Suppress per-fixture progress. Only the aggregate report and
non-`OK` verdict lines print.

#### `--verbose`, `-v`

Print each fixture's rustc command before running it, plus captured
stderr regardless of normalization outcome.

#### `--use-symlink`

Skip the lihaaf-managed dylib copy (Section 4.3) and create a symbolic
link instead. This saves the copy cost (~few hundred ms, ~30 MB disk)
at the cost of safety: lihaaf will NOT detect concurrent cargo activity
when this flag is set. If a sibling cargo invocation replaces the
cargo-managed dylib between fixture dispatches, workers may receive a
torn read, a different ABI version, or an absent file, producing
`WORKER_CRASHED` or wrong verdicts with no clear diagnostic.

Safety contract: the caller asserts that no concurrent cargo build will
modify `target/` for the duration of the lihaaf session. Typical safe
context: a single-process CI runner with a dedicated `target/` directory
that has no concurrent jobs. Never set when an IDE `cargo check` loop or
a second terminal `cargo build` might be active.

#### `--keep-output`

Preserve per-fixture work directories after verdict capture, for the
duration of the run. Default is immediate cleanup after each verdict
(Section 5.3). Use this to inspect a fixture binary, re-invoke rustc
with different flags, or diagnose a `WORKER_CRASHED` verdict by
examining the partial output.

Work directories are printed to stderr at session end so the engineer
can find them. The per-session temporary parent directory is also kept
when `--keep-output` is set.

Local-development escape hatch only. Never set in CI — it defeats the
disk footprint guarantee that is lihaaf's reason for existing.

#### `--help`, `-h`, `--version`, `-V`

Standard.

### 8.3 Exit codes

See Section 10. The CLI's exit code is the maximum (most severe) of
all per-fixture verdicts plus the session-level outcomes.

### 8.4 What the CLI does NOT do

These are conscious omissions, not oversights:

- **No `--watch` mode.** Section 11.4.
- **No `--coverage` flag.** Section 11.2.
- **No `--report html|json|junit` flag.** Section 11.3. v0.1 emits
  text only.
- **No interactive bless.** Section 7.3.
- **No `--target <triple>` flag.** Section 11.1. v0.1 always uses the
  host triple.
- **No `--profile <name>` flag.** v0.1 always uses release. The
  decision (Section 4.2) is conscious.

### 8.5 Semver commitment

For v0.1.x, lihaaf commits to:

- **Flag stability.** No flag listed in 8.2 disappears or changes
  semantics across patches or minor versions.
- **Exit-code stability.** The codes in Section 10 are part of the
  stable surface. Adding a new failure mode produces a new code; an
  existing code's meaning does not change.
- **Snapshot format stability.** Normalized `.stderr` snapshot bytes
  produced by lihaaf 0.1.x are also acceptable to lihaaf 0.1.y for
  any y ≥ x, modulo new normalization rules added in y that an
  adopter explicitly opts into. (The default normalization set is
  fixed across minor versions.)

A v0.2 release may add flags, add normalization rules, and add
verdicts/exit codes. It MUST NOT remove flags or change existing exit
codes; that requires a v1.0.

---

## 9. Validation strategy

### 9.1 Side-by-side parity with the prior art

Before lihaaf v0.1 ships, the harness MUST be validated against the
canonical adopter's existing trybuild corpus by running the two
side-by-side and diffing outputs. The "canonical adopter" here is the
framework crate that drove lihaaf's design: a Rust ORM/framework with
~237 trybuild-style fixtures. lihaaf is NOT coupled to that adopter;
the generic harness happens to have a richly populated reference
corpus available.

The validation procedure:

1. Pin the toolchain to a single rustc release (the adopter's
   `rust-toolchain.toml`).
2. Run `cargo test --test trybuild_tests` in the adopter. Capture
   per-fixture pass/fail and total wall-clock.
3. Run `cargo lihaaf` in the same adopter against the same source
   files. Capture per-fixture pass/fail and total wall-clock.
4. For each fixture, compare verdicts. Each disagreement resolves to
   one of:
   - lihaaf is wrong → fix lihaaf before v0.1 ships.
   - trybuild is wrong → record, file the trybuild issue, document.
   - Fixture is ambiguous (relies on undocumented diagnostic wording)
     → fix the fixture.
5. Compare wall-clock and parallelism. Success criterion: lihaaf
   full-sweep wall-clock ≤ 25% of trybuild full-sweep on the same
   hardware.

### 9.2 What "agreement" means

For a fixture, agreement requires:

- Both harnesses produce the same pass/fail verdict.
- For compile_fail fixtures, the normalized stderr matches the
  snapshot in BOTH harnesses (i.e., neither harness flags a snapshot
  diff). lihaaf and trybuild use different normalization
  implementations; the spec's success criterion is that both
  implementations agree on the canonical adopter's checked-in
  snapshots after Section 6.2's normalization rules are applied.

If lihaaf normalizes more aggressively than trybuild and produces a
shorter normalized output, the snapshot would need re-blessing under
lihaaf. That is expected and acceptable — it's a one-time migration
step adopters take when switching. The validation just needs to
confirm the verdict (pass/fail) agrees.

**Scope and bounds of this criterion.** The criterion above is a
release-gate signal for the canonical adopter's corpus. It is NOT a
cross-project fidelity proof, and lihaaf does NOT claim equivalence
to trybuild for arbitrary consumer crates. Specifically, the
"agreement" definition can pass in the presence of:

- **Diagnostic-quality drift** that preserves pass/fail outcome —
  changed column numbers, reordered notes, altered help-text wording,
  added/removed `note:` lines that the snapshot didn't capture.
- **Semantic regressions in error reporting** that don't flip the
  pass/fail verdict — for instance, a fixture that expected error
  E0277 emitting E0599 instead would pass agreement if both harnesses
  see the new error and the snapshot is re-blessed for both.
- **Noise differences from environmental factors** that lihaaf's
  normalizer handles differently from trybuild — temp paths, line
  endings, concurrent-process artifacts.

Adopters using lihaaf on consumer crates other than the canonical
adopter should expect to do their own validation pass before relying
on lihaaf for release gating. The validation strategy here is sized to
catch gross divergence on one well-characterized corpus, not to prove
fidelity across the long tail of possible proc-macro patterns.

### 9.3 Wall-clock and parallelism observations to record

For the validation report:

- Total wall-clock (each harness, full sweep).
- Wall-clock per phase bucket (the canonical adopter's trybuild driver
  is split into ~22 per-phase `#[test]` functions; lihaaf has no such
  split, so this is approximated by `--filter <phase>_`).
- Peak RSS (sum across worker pool, sampled at 100 ms intervals).
- Number of cargo invocations (trybuild: one per per-phase test fn ×
  one per fixture; lihaaf: one per session).

The numbers are reported in the v0.1 release notes, not in the spec
itself (the spec describes the validation procedure, not the
results).

### 9.4 Why this validation matters

A new test harness producing different verdicts from the prior art is
a footgun. Adopters won't catch a subtle difference until later, when
it surfaces as a regression escape that "the old harness would have
caught." The side-by-side run flushes those out before they become
production lore.

---

## 10. Failure modes

### 10.1 Per-fixture verdicts

Every fixture produces exactly one verdict. The verdict is the
authoritative result for that fixture in the run.

| Verdict | Meaning |
|---|---|
| `OK` | Fixture matched expectation (compile_pass: rustc exited 0; compile_fail: normalized stderr equals snapshot). |
| `EXPECTED_FAIL_BUT_PASSED` | compile_fail fixture, but rustc exited 0. |
| `EXPECTED_PASS_BUT_FAILED` | compile_pass fixture, but rustc exited non-zero. |
| `SNAPSHOT_DIFF` | compile_fail fixture, rustc failed as expected, normalized stderr differs from snapshot. |
| `SNAPSHOT_MISSING` | compile_fail fixture, rustc failed as expected, no `.stderr` file on disk. |
| `BLESSED` | `--bless` was set and the snapshot was overwritten. (Treated as OK for exit-code purposes.) |
| `TIMEOUT` | Worker exceeded `fixture_timeout_secs`. |
| `MEMORY_EXHAUSTED` | Worker exceeded `per_fixture_memory_mb` on both initial and serial retry. |
| `WORKER_CRASHED` | Worker exited via signal or non-rustc exit code (e.g., SIGSEGV in rustc, ICE) without lihaaf-initiated kill. The verdict line names the signal/code. |
| `SNAPSHOT_DIFF_TOO_LARGE` | compile_fail fixture, normalized stderr or snapshot exceeds the 100,000-line hard ceiling (Section 7.2). Diff is skipped; the verdict line includes both line counts and the first 100 lines of each side. |
| `MALFORMED_DIAGNOSTIC` | Non-UTF-8 bytes detected in the rustc `--error-format=json` output's `rendered` field, or in the snapshot file. Diff is skipped; the verdict line names the byte offset where validation failed (Section 7.2). |
| `CLEANUP_FAILED` | The fixture's pass/fail outcome is captured normally, but the per-fixture work directory could not be removed (filesystem error, permission denied, foreign process holding a file open). Appended to the fixture's verdict slot rather than replacing it; surfaces at session end as the `CLEANUP_RESIDUE` session outcome. |

### 10.2 Session-level outcomes

Independent of per-fixture verdicts, the session itself can fail
before fixtures run. These are sticky — a session-level failure means
no per-fixture verdicts are reported.

| Outcome | Meaning |
|---|---|
| `CONFIG_INVALID` | `[package.metadata.lihaaf]` missing or invalid (Section 3.4). |
| `DYLIB_BUILD_FAILED` | The dylib build (Section 4.2) returned non-zero. lihaaf prints the cargo invocation and the captured stderr verbatim. |
| `DYLIB_NOT_FOUND` | cargo succeeded but no `compiler-artifact` message named the dylib. |
| `TOOLCHAIN_DRIFT` | rustc version at fixture-dispatch differs from the version at dylib-build (Section 4.6). |
| `MANIFEST_CORRUPT` | `target/lihaaf/manifest.json` is unreadable or schema-invalid. lihaaf treats this as a stale-cache event and rebuilds the dylib without further error. |
| `CLEANUP_RESIDUE` | One or more fixtures had `CLEANUP_FAILED` diagnostics. The session's per-fixture verdicts still determine pass/fail; this outcome is OR'd into the exit code so CI can flag the residue without confusing it with a fixture-verdict failure. The session-temp parent directory is NOT removed at session end if `CLEANUP_FAILED` residue exists — leaving it visible is more useful than silently retrying a removal that already failed. |

### 10.3 Exit codes

```
0   all fixtures OK (or BLESSED)
1   one or more EXPECTED_FAIL_BUT_PASSED, EXPECTED_PASS_BUT_FAILED, or SNAPSHOT_DIFF
2   one or more SNAPSHOT_MISSING (without --bless)
3   one or more TIMEOUT
4   one or more MEMORY_EXHAUSTED
5   one or more WORKER_CRASHED
6   one or more SNAPSHOT_DIFF_TOO_LARGE
7   one or more MALFORMED_DIAGNOSTIC
8   CLEANUP_RESIDUE (one or more CLEANUP_FAILED diagnostics; OR'd with above)
64  CONFIG_INVALID
65  DYLIB_BUILD_FAILED
66  DYLIB_NOT_FOUND
67  TOOLCHAIN_DRIFT
```

When multiple verdicts are present in a single run, the exit code is
the maximum (most severe) one (1 < 2 < 3 < 4 < 5 < 6 < 7 < 8 < 64 <
65 < 66 < 67). `CLEANUP_RESIDUE` (8) sits between fixture failures
and session-level "couldn't even start" outcomes — disk hygiene is
worse than a normal fixture failure but doesn't invalidate the
verdict signal. The exact ordering is part of the stable surface
(Section 8.5).

The 64+ block uses `<sysexits.h>` numbering for session-level "couldn't
even start" failures, by analogy with the convention. The 1–5 block is
ad-hoc but stable.

### 10.4 What lihaaf does NOT do on failure

- **No retry of failed fixtures across runs.** Each invocation runs
  the selected fixtures once; failures stick.
- **No partial commit on `--bless` after some fixtures fail.** The
  bless action is independent per fixture: if fixture A's snapshot
  was blessed but fixture B's verdict is `WORKER_CRASHED`, fixture A's
  `.stderr` is still updated. Bless is per-fixture, not all-or-nothing.
- **No automatic `--bless` on snapshot-missing.** A missing snapshot is
  always a `SNAPSHOT_MISSING` verdict unless the adopter explicitly
  passes `--bless`.

### 10.5 Reproducibility floor

For a fixed `(toolchain release, dylib SHA-256, fixture content,
snapshot content, OS)` tuple, lihaaf's verdict for a given fixture is
deterministic. The harness has no nondeterminism in the verdict path —
no env-dependent normalization, no wall-clock-dependent decisions.

The wall-clock determinism floor is necessarily weaker because the
worker pool's RSS sampling and OS scheduler decisions affect
TIMEOUT/MEMORY_EXHAUSTED outcomes. Adopters whose CI is on the edge of
the timeout should raise it explicitly rather than rely on best-case
scheduling.

---

## 11. Anchored deferrals

Every cut feature below has a specific anchor — a concrete reason and
a specific future-trigger or "never" classification. None of these are
"we'll get to it eventually" promises; each is either truly never, or
truly waiting for a concrete signal.

### 11.1 Multi-target / cross-compilation

**Status:** deferred to v0.x.

**Anchor:** The set of adopter targets driving v0.1 is
host-Linux + host-macOS. Cross-compiling proc-macros for testing has
the additional complication that the target dylib must be loadable on
the host (proc-macro resolution happens on the host, not the target).

**Future trigger:** an adopter requests Windows-target or
embedded-target proc-macro testing AND has a host capable of running
those binaries. At that point, lihaaf adds a `--target <triple>` flag
and gates dylib linking through cargo's existing target-triple
machinery.

### 11.2 Coverage instrumentation

**Status:** deferred to v0.x.

**Anchor:** `cargo-llvm-cov` drives cargo invocations end-to-end;
lihaaf bypasses cargo for the fixture path. Integration would require
forwarding `-C instrument-coverage` to per-fixture rustc and
collecting per-process profiles. Demand signal is currently zero
because fixtures mostly type-check rather than run code.

**Future trigger:** an adopter requests coverage data for fixture-side
code execution. At that point, lihaaf adds `--coverage` that sets
per-fixture RUSTFLAGS and emits profiles into `target/lihaaf/coverage/`.

### 11.3 HTML / JSON / JUnit reporters

**Status:** deferred to v0.x.

**Anchor:** v0.1 emits text + exit codes. CI parses text logs fine
for compile-fail/compile-pass; JUnit XML is over-spec'd for
"fixture passed/failed" granularity.

**Future trigger:** an adopter has a structured-output pipeline
(Allure, custom dashboards). At that point, lihaaf adds `--report
json` (schema in the docs) and possibly `--report junit`. HTML never —
`cargo lihaaf | aha > out.html` is a one-liner.

### 11.4 Workshop / watch mode

**Status:** **NEVER**.

**Anchor:** The audience for a watch-mode REPL is fixture authors, and
their iteration loop is already sub-second: `cargo lihaaf --filter
<name>` with the dylib warm in cache. End-users of the adopter crate
iterate on queries via the parallel Rhai-shell consumer (Section 2.4),
not on `.rs` fixtures.

A watch mode would also undermine the "session is one process" model
that Sections 4 and 5 lean on for determinism, OOM containment, and
freshness validation. A long-running daemon with mutable cached
state is a different architecture.

If the use case ever materializes, a separate `lihaaf-watch` tool
wrapping `cargo lihaaf` with `notify`-based filesystem watching is the
right shape — not a flag on the core harness.

### 11.5 Custom test runners (`#[test]` integration)

**Status:** **NEVER**.

**Anchor:** Integrating with `#[test]` (the trybuild
`TestCases::new()` pattern) would hand parallelism control to cargo's
test scheduler (defeating Section 5.2's RAM cap), tie lihaaf's
process lifecycle to a test-binary lifecycle (complicating Section
5.4's OOM containment and Section 4.6's drift detection), and
produce two competing CLI shapes for the same workload.

Adopters who must run lihaaf via `cargo test` for organizational
reasons can write a small wrapper:

```rust
#[test]
fn lihaaf() {
    let status = std::process::Command::new("cargo")
        .arg("lihaaf")
        .status()
        .expect("cargo lihaaf");
    assert!(status.success(), "cargo lihaaf failed");
}
```

That snippet is not part of lihaaf. The harness itself ships only as a
cargo subcommand.

### 11.6 `.rhai` fixture support

**Status:** deferred to v0.x with a strong "probably never" lean.

**Anchor:** A different consumer (the Rhai-based interactive shell
mentioned in Section 2.4) has its own thin test harness for `.rhai`
content. Extending lihaaf to drive `.rhai` would couple the harness
to the Rhai consumer's domain, contradicting "lihaaf is generic."

**Future trigger:** sustained demand from multiple adopters for
non-Rust fixture types AND a clean abstraction for "compile language
X via tool Y, capture output, snapshot." If that ever surfaces, a
separate harness (possibly reusing lihaaf's worker-pool primitives)
is the right shape, not a flag on this one.

### 11.7 Workspace-scope invocation

**Status:** deferred to v0.x.

**Anchor:** v0.1 is per-crate (`cargo lihaaf` from inside the consumer
crate, or `--manifest-path` pointing to it). Workspaces with multiple
crates each carrying `[package.metadata.lihaaf]` would need traversal.

**Future trigger:** an adopter has multiple workspace members with
fixture corpora and wants to drive them all in one invocation. At
that point, lihaaf adds `--workspace`, `--exclude <pkg>`, and
`--package <pkg>` matching cargo's selectors.

### 11.8 IDE integration

**Status:** deferred to v0.x.

**Anchor:** IDEs hook into `cargo test` and rust-analyzer's test
discovery. lihaaf-as-subcommand doesn't fit that machinery. The CLI
surface (Section 8) needs to stabilize before IDE-plugin work.

**Future trigger:** the v0.1 CLI ships, real adopters use it, and an
IDE-integration request lands with a concrete protocol (LSP
diagnostics, Test Explorer adapter). The hook gets designed with
knowledge of which IDE and which protocol — not speculatively.

### 11.9 Per-fixture result caching across runs

**Status:** deferred to v0.x.

**Anchor:** v0.1 re-runs every selected fixture on every invocation.
With the dylib cached, a 200-fixture sweep finishes in seconds.
Caching requires fingerprinting `(fixture content, dylib SHA-256,
rustc release)` and the bookkeeping is not free.

**Future trigger:** an adopter's corpus grows past ~2000 fixtures and
per-fixture rustc cost dominates wall-clock. At that point, lihaaf
adds a fingerprint-keyed verdict cache under
`target/lihaaf/verdicts/` and a `--no-verdict-cache` opt-out.

### 11.10 Built-in shard splitter

**Status:** deferred to v0.x.

**Anchor:** v0.1 supports CI-side sharding via `--list` + external
`split` + `--filter`. A native `--shard <i>/<n>` is convenient, not
load-bearing.

**Future trigger:** adopters report enough boilerplate around the
`--list`+`split`+`--filter` pattern. At that point, lihaaf adds
`--shard` with deterministic hashing of paths into `n` buckets.

---

## 12. Counter-arguments

The lens demands honesty about what gives the design pause. The
following are the strongest cases against the choices in this spec,
phrased as if a thoughtful skeptic were challenging it. Each is met
with the reasoning that nonetheless drove the decision in.

### 12.1 "The dylib model is fragile and hard to debug"

**The challenge.** Dynamic linking introduces failure modes absent
from the rlib world: linker version skew, `dlopen`-time symbol-
resolution failures with inscrutable messages, ABI surprises when
monomorphizations diverge between dylib build and fixture build, and
a long history of `LD_LIBRARY_PATH` pain on Linux + glibc. Trybuild's
"each fixture is a standalone cargo project" model trades wall-clock
to avoid all of this — a defensible engineering call.

**The response.** The fragility is narrower in this design and the
remaining failure modes are loud:

- ABI skew between dylib and fixture is impossible because BOTH are
  built by the SAME rustc invocation — the build captures the
  rustc release at stage 2 and any drift hard-fails before fixtures
  dispatch (Section 4.6).
- Symbol-resolution failures at fixture link time produce rustc
  diagnostics ("undefined reference to ...") that are themselves the
  test signal: either the fixture is wrong or the dylib is wrong, and
  both are observable.
- `LD_LIBRARY_PATH` is not on the runtime path — the fixture binary
  is produced but never executed. lihaaf only cares about rustc's
  exit code and stderr. This sidesteps the largest dylib pain class
  entirely.

The design accepts the narrower fragility surface in exchange for
substantial performance and parallelism wins.

### 12.2 "Hard-fail on rustc drift is too aggressive"

**The challenge.** A developer running `cargo lihaaf`, then `rustup
update`, then re-running `cargo lihaaf` hits TOOLCHAIN_DRIFT and has to
wait through a full dylib rebuild. A permissive design could detect
the drift and rebuild transparently. The spec optimizes for a rare bug
(silent ABI miscompile) at the cost of common workflow friction.

**The response.** The fail-vs-rebuild distinction lands on fail for
two reasons:

- TOOLCHAIN_DRIFT is rare in practice; the friction cost is paid
  rarely while the safety gain (a clear error pointing at what
  changed, versus a transparent rebuild that hides the toolchain
  transition from CI logs) is paid every time.
- A transparent rebuild changes the dylib SHA-256 and can flip
  verdicts for fixtures sensitive to rustc-version-specific diagnostic
  wording (suggestion text, similarly-named-method hints). An adopter
  seeing "PASSED" then "SNAPSHOT_DIFF" without a "your toolchain
  changed" signal would file a bug against lihaaf for what is
  actually a toolchain transition.

The friction is real. v0.x can soften with a `--rebuild-on-drift`
opt-in that does the rebuild loudly. The default stays hard-fail.

### 12.3 "RAM-aware parallelism with OOM containment is over-engineered for v0.1"

**The challenge.** Per-rustc RSS sampling, OOM containment, back-off,
and serial-retry logic in v0.1 add code volume and edge-case surface.
A simpler v0.1 (no sampling, fixed `-j num_cpus / 4`) would ship
faster and stress-test the architecture sooner.

**The response.** The over-engineering charge is fair on volume, but
the inclusion is justified by the failure mode prevented:

- Without OOM containment, a single fixture with runaway template
  instantiation (an easy-to-write proc-macro bug) gets the entire
  worker cohort killed by OOMkiller, producing flaky verdicts on
  unrelated fixtures. The diagnostic burden lands on adopters as
  "lihaaf killed my CI" and damages the harness's reputation hard
  before there's any production track record to lean on.
- The RAM cap (Section 5.2) is the cheapest possible defense — read
  total RAM once, set `-j` so OOM cannot happen. OOM containment
  catches the residual case of one bad fixture exceeding its share.
  Both features are small relative to the rest of the harness.

If validation (Section 9) shows the OOM path never triggers on real
adopters, v0.x can simplify. The v0.1 default ships the defense and
observes.

### 12.4 "Refusing to support `#[test]` integration locks out real adopters"

**The challenge.** A nontrivial fraction of teams have a CI policy
like "the only command CI runs is `cargo test`." Refusing `#[test]`
integration means lihaaf is either skipped or wrapped via Section
11.5's snippet — a small barrier the prior art doesn't have.

**The response.** The barrier is accepted. Section 11.5's reasoning
(tying lihaaf's process model to cargo's test scheduler would
compromise parallelism, OOM containment, and drift detection) is
load-bearing on the architectural commitments. The lens prioritizes
scalability + stability over adopter ergonomics here.

If the wrapper snippet turns out to be unacceptable in practice, v0.x
can ship a `lihaaf-test` shim crate whose sole purpose is a
`#[test]`-friendly entry point that subprocess-spawns `cargo lihaaf`
— smallest possible footprint without compromising the architecture.

### 12.5 "No regex is purist; it'll cost you a clean implementation"

**The challenge.** Normalization and diff are easier with a regex
engine. Hand-rolling forces a slower, audit-heavier byte-level
implementation with its own bug surface (off-by-one in `split_once`
logic, missed Unicode edge cases).

**The response.** The constraint is partly inherited from adopter
policies (Section 6.1), but also good design:

- The normalizer's substitutions (Section 6.2) are all fixed-string
  replacements with known prefixes — no backtracking, alternation, or
  character classes needed.
- The Myers diff (Section 7.2) does not need regex at all.
- The `regex` crate adds ~500 KB to the binary and ~10 seconds to
  clean-build time. For a tool run many times a day, that compounds.

The hand-rolled normalizer's bug surface is bounded — each
substitution maps to a fixture, and the Section 9 validation runs a
diverse real-world corpus through it.

### 12.6 "Building the dylib on every invocation is too much overhead"

**The challenge.** Stage 3 (Section 4.1) runs `cargo build` every
invocation. Even with cargo's incremental cache hot, that's a
multi-second cost. A smarter design would skip stage 3 when
`dylib_sha256` matches the on-disk artifact.

**The response.** Fair optimization opportunity, deferred to v0.x.
v0.1's posture is:

- Always run cargo. cargo itself decides whether anything needs
  rebuilding; if nothing does, the invocation is fast (a few hundred
  ms in steady state) because cargo just walks its fingerprint DB.
- The freshness check (Section 4.5) re-checks SHA-256 cheaply.

Skipping cargo entirely requires verifying lihaaf's freshness model
is strictly conservative versus cargo's. v0.1 errs toward "let cargo
handle it" to avoid shipping a stale-cache bug. The overhead is real;
the v0.1 budget accepts it.

---

## 13. Spike contingency appendix

### 13.1 Spike scope and resolution status

The spike validated end-to-end whether:

- (a) `cargo rustc --crate-type=dylib` can override the consumer
  crate's `[lib]` declaration to produce a dylib without requiring
  any change to the consumer's `Cargo.toml`.
- (b) `inventory::submit!` registrations defined inside the dylib
  propagate to consumers that link the dylib at runtime, with no
  consumer-side workaround.

**Spike status: resolved 2026-05-10 with outcome `GO_NATIVE`.**

Both (a) and (b) succeed. `cargo rustc -p <consumer> --lib --release
--crate-type=dylib` works, and inventory submissions propagate across
the dylib boundary natively. The spec's main body holds verbatim.

This appendix retains the full contingency catalog for two purposes:
(1) revalidation cadence — if the dylib ABI or inventory behavior
changes in a future Rust release, the team has a reference for which
outcome applies; (2) completeness for adopters whose consumer crates
differ from the validated canonical adopter.

### 13.2 Outcome 1 — `GO_NATIVE`

(a) and (b) both succeed natively.

**No changes.** The spec's main body holds verbatim. Stage 3's cargo
invocation is `cargo rustc -p <dylib_crate> --lib --release
--crate-type=dylib --message-format=json`. Adopters do not edit their
`Cargo.toml`'s `[lib]` table.

**This is the confirmed outcome as of 2026-05-10.**

**Linking-mode prerequisites the spike validated.** GO_NATIVE depends
on two consumer-side build properties beyond (a) and (b). The spike
artifact validated both for the canonical adopter; lihaaf MUST
re-validate for each new adopter the same way:

- **`-C prefer-dynamic` for compile-time-link consumers.** When lihaaf
  invokes the dylib build, it sets `RUSTFLAGS="-C prefer-dynamic"` so
  the resulting dylib links its own dependencies dynamically rather
  than statically baking them in. Without this flag the dylib is
  self-contained (~1.5 MB larger per the spike's measurements) and
  compile-time-link consumers (lihaaf's own per-fixture rustc
  invocations) end up with duplicate copies of stdlib-adjacent
  crates. With this flag the dylib is smaller and consumers must have
  the same dynamic dependencies available at link time. Lihaaf
  accepts this — the harness controls both sides of the link.
- **Platform loader compatibility.** The dylib's `.init_array` (Linux),
  module initializers (macOS), or DllMain entry points (Windows) must
  successfully run when the dylib is loaded. The spike validated this
  on Linux x86_64 via `libloading::Library::new`. macOS and Windows
  inherit the inventory-crate guarantees but have NOT been validated
  by the spike directly; an adopter targeting those platforms MUST
  re-run the spike's runtime smoke test before relying on
  GO_NATIVE.

A spike outcome that fails ONLY one of these prerequisites — say,
self-contained dylib works for runtime-load consumers (Phase 9 shell)
but `-C prefer-dynamic` produces a dylib that compile-time-link
consumers (lihaaf) can't use — sits between GO_NATIVE and
RUNTIME_INCOMPATIBLE. The honest classification is: the dylib is
valid for SOME consumer styles but not others. Lihaaf's per-fixture
path requires compile-time-link to work; if `-C prefer-dynamic` fails,
lihaaf falls back to the self-contained dylib (~1.5 MB tax per
fixture link, no functional change) and notes the cost in the
manifest. The Phase 9 shell, which uses runtime `dlopen`, is unaffected
by this prerequisite.

### 13.3 Outcome 2 — `GO_WITH_MANIFEST`

(a) fails: `cargo rustc --crate-type=dylib` does NOT override
`[lib]`'s `crate-type`. Adopters must declare `crate-type = ["lib",
"dylib"]` in the consumer's `Cargo.toml`.

(b) succeeds: with the manifest declaration in place, inventory
propagates natively.

**Changes required:**

- Section 3 documents an adopter-side requirement: the consumer's
  `[lib]` table must declare `crate-type = ["lib", "dylib"]`. The
  `lib` entry preserves the default rlib build for normal `cargo
  build`; the `dylib` entry is what lihaaf consumes.
- Section 4.2 simplifies to `cargo build -p <dylib_crate> --lib
  --release --message-format=json`. lihaaf still parses
  `compiler-artifact` messages but now selects the `target.kind`
  containing `"dylib"` (the build produces both an rlib and a dylib).
- Section 3.4 adds a validation: if the consumer's `[lib]` does NOT
  include `"dylib"` in `crate-type`, lihaaf hard-errors with a clear
  "add this to your Cargo.toml" remediation hint.

Adopter-friction cost: one line of Cargo.toml. Acceptable.

### 13.4 Outcome 3 — `GO_WITH_WORKAROUND`

(a) fails OR succeeds (irrelevant — workaround applies regardless).

(b) fails: inventory does not propagate across the dylib boundary
without help. Items registered inside the dylib are not visible to
consumers that link the dylib at runtime.

**v0.1 scope reduction.** If this outcome applies on revalidation,
v0.1 ships WITHOUT inventory propagation support — the architectural
simplification doesn't survive the workaround. Fixtures whose macros
emit `inventory::submit!` calls won't observe those registrations at
runtime. This is acceptable for v0.1 because the dominant fixture
workload is "compile and type-check," not "run and observe registered
items." Adopters whose fixtures DO need runtime inventory inspection
fall back to the prior art (trybuild) until v0.x ships a workaround.

**v0.x workaround sketch.** Before locking in any
`lihaaf_inventory_collect_<T>()` workaround, the implementation team
must evaluate alternate registration mechanisms (`linkme`, `ctor`,
manual init) to determine whether one provides cleaner propagation
semantics across the dylib boundary. Only if none does should the
explicit collector pattern be adopted:

```rust
// In the consumer crate, behind a `lihaaf` (or similarly-named) flag:
#[cfg(feature = "lihaaf")]
pub fn lihaaf_inventory_collect_<T>() -> Vec<&'static T> {
    inventory::iter::<T>().collect()
}
```

A required `inventory_collectors: ["fn1", "fn2"]` config key names
each collector. lihaaf invokes them at fixture-build time via a
generated thunk and re-emits registrations in a generated stub the
fixture pulls in.

The adopter-friction cost is significant; the spec doesn't pretend
this outcome is a clean degradation. If revalidation returns it, the
orchestrator gets a clear "v0.1 scope narrows; amend before
implementation" signal.

### 13.5 Outcome 4 — `RUNTIME_INCOMPATIBLE`

(a) succeeds: the dylib builds successfully.

(b) partially fails: the dylib builds and links, but fails at runtime
due to TLS initialization races, dynamic loader compatibility issues,
or global-initialization ordering problems when the fixture binary
`dlopen`s the dylib. The failure manifests as a runtime crash or a
hang during dylib loading rather than a compile-time error.

This outcome is distinct from `GO_WITH_WORKAROUND`: the mechanism
(inventory propagation) may actually work in isolation, but the dylib
cannot be loaded reliably at all. The spike's API smoke step —
actually loading the dylib and invoking a registered item — is the
test that distinguishes `RUNTIME_INCOMPATIBLE` from `GO_NATIVE` or
`GO_WITH_WORKAROUND`.

**Changes required.** The remediation mirrors `NO_GO` in scope: the
dylib-per-session architecture needs to be evaluated for a fallback
that does not rely on runtime loading. Alternate approaches include
`linkme` or `ctor` for registration, or a compile-only harness that
never executes fixture binaries (acceptable if the workload is purely
"does this compile" rather than "does this run with correct registered
items"). The orchestrator should dispatch a focused spike on the
specific loader failure before choosing a remediation path.

### 13.6 Outcome 5 — `NO_GO`

(a) and (b) both fail in ways that cannot be worked around without
upstream changes to either cargo or the inventory crate.

**Changes required:** lihaaf cannot ship in the form this spec
describes. The dylib architecture is the load-bearing choice; without
it, the harness has no advantage over trybuild and the spec's main
body needs to be substantially redrafted.

The spike is unlikely to return `NO_GO` — both `cargo rustc` and
`inventory` have load-bearing public APIs that strongly suggest the
property holds at least in the `GO_WITH_MANIFEST` shape. But the spec
acknowledges the possibility for honesty, and `NO_GO` is what the
orchestrator gets if neither (a) nor (b) survives investigation.

---

## Known risks

A handful of design choices in this spec carry residual risk that
no amount of self-review can eliminate. They are documented here so
the implementation team and reviewers can engage with them directly.

### KR-1 — RSS sampling resolution on Linux

Section 5.4's 100 ms RSS sampling interval may be too coarse to catch
a fixture whose memory footprint balloons in the milliseconds between
spawn and OOM. The mitigation is the OS-level OOMkiller (which still
exists as a backstop), but lihaaf's verdict in that case will be
`WORKER_CRASHED`, not `MEMORY_EXHAUSTED`. Adopters reading the report
may mis-classify the cause. v0.x can move to `cgroups`-based memory
limits on Linux for accurate enforcement; v0.1 documents this gap.

### KR-2 — Atomicity of the manifest rewrite

Section 4.1 stage 5 writes `manifest.json` atomically (write-temp +
rename). On non-POSIX filesystems (some Windows configurations), the
rename may not be atomic, leaving a window in which a concurrent
lihaaf invocation could read a half-written manifest. The mitigation
is Section 10.2's `MANIFEST_CORRUPT` outcome, which treats unreadable
manifests as stale-cache events and rebuilds. Two concurrent lihaaf
invocations against the same target dir will produce extra cargo work
but not wrong verdicts.

### KR-3 — Workspace-shared `target/` with concurrent cargo activity

The design assumption (Section 1.3) is that concurrent cargo activity
is normal. The copy-default architecture (Section 4.3) directly
addresses this: fixture workers reference lihaaf's stable managed copy
of the dylib, not the cargo-managed original. A concurrent `cargo
build` that replaces or deletes the original mid-session has no effect
on the in-progress fixture workers.

The residual risk is narrow: the managed copy could itself be modified
externally (e.g., a script that cleans `target/lihaaf/`), which the
freshness check (Section 4.5) catches before the next fixture
dispatches. This is a low-likelihood administrative accident, not a
normal concurrent-cargo scenario.

When `--use-symlink` is active, the original concurrent-cargo race
returns in full: the symlink points at the cargo-managed artifact, and
a concurrent `cargo build` can replace it mid-session. This is the
documented tradeoff of `--use-symlink`; callers who set it accept the
race. flock-based mitigation is not viable here — cargo does not honor
advisory flock for its output files on Linux.

### KR-4 — `--bless` racing with editor save loops

Section 7.3's `--bless` overwrites `.stderr` files in the working
copy. If an adopter has the `.stderr` open in an editor that auto-saves
(VS Code's autosave, vim's `autowriteall`, etc.), a `--bless` mid-edit
can produce conflicting writes. lihaaf does not coordinate with editors;
the convention is "bless is destructive; close editors first." v0.x
could write to a sibling `.stderr.new` and require the user to confirm,
but this is a workflow issue rather than a correctness issue and v0.1
opts for the simpler "overwrite, trust git" path.

### KR-5 — Per-platform RSS sampling API selection

Section 5.4 requires sampling per-process resident set size on each
target platform, but the correct API varies per OS and some candidates
that appear suitable are not. The correctness risk is silent failure to
enforce the memory ceiling: an implementation that picks the wrong API
can report no ceiling exceedance on a fixture that is actually running
out of memory, causing `WORKER_CRASHED` verdicts (from the OS
OOMkiller) rather than the expected `MEMORY_EXHAUSTED` verdict and
back-off behavior.

The canonical example is `getrusage(RUSAGE_CHILDREN)` on macOS: this
call returns cumulative statistics accumulated from already-terminated
children, not the live resident set size of a currently running child
process. An implementation that chose this API would silently fail to
enforce the Section 5.4 ceiling on macOS — the sampler would report
the wrong value for live workers and the ceiling check would never
fire.

**Mitigation requirement.** The implementer is expected to validate,
before shipping v0.1, that the chosen per-platform API actually returns
live per-process RSS for a running child on each target OS. Validation
must be confirmed against at least one fixture corpus that exercises
the ceiling on each target OS (i.e., a fixture that actually exceeds
`per_fixture_memory_mb` must produce `MEMORY_EXHAUSTED`, not
`WORKER_CRASHED`). v0.1 does not ship until this per-platform
sampling has been verified.

---

## Glossary

- **Consumer crate** — the crate whose proc-macros are under test;
  the crate lihaaf builds as a dylib.
- **Fixture** — a `.rs` file in `fixture_dirs` that the harness
  compiles to validate one specific assertion (compile_fail,
  compile_pass).
- **Snapshot** — the `.stderr` sibling file for a compile_fail
  fixture, holding the expected normalized rustc stderr.
- **Bless** — overwrite a snapshot with the current normalized rustc
  stderr, conventionally done after intentional changes to either the
  fixture or the consumer crate's diagnostic output.
- **Dylib** — Rust dynamic library (`.so` on Linux, `.dylib` on
  macOS, `.dll` on Windows).
- **Worker** — one rustc child process invoked by the harness to
  compile one fixture.
- **Verdict** — the harness's final classification for a fixture
  (Section 10.1).
- **Outcome** — the harness's session-level classification (Section
  10.2), independent of per-fixture verdicts.
