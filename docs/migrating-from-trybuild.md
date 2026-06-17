# Migrating from trybuild

This guide walks existing trybuild users through converting a compile-fail and/or
compile-pass fixture suite to lihaaf. It is based on three real-world conversions:
sassi ([`c143f373`](https://github.com/TarunvirBains/sassi/commit/c143f373)),
djogi ([`fbc80b52`](https://github.com/TarunvirBains/djogi/commit/fbc80b52)),
and the anyhow conversion branch (`lihaaf-converted`).

## Who this guide is for

You have a proc-macro crate (or a regular crate with proc-macro dependencies) that
uses trybuild to gate compile-fail and/or compile-pass fixtures in CI. Your fixture
count is large enough that trybuild's per-fixture `cargo` invocations are becoming
slow to iterate on locally or in CI — or you want a harness that will stay fast as
the suite grows. This guide assumes you know your way around `Cargo.toml` and a
basic GitHub Actions workflow.

If your fixture count is small (say, fewer than 10) and wall-clock time is not a
concern, be aware that lihaaf requires an upfront dylib build (~10–20 s) that
amortizes less over a tiny fixture set. The migration is still valid — the workflow
is cleaner — but the speed benefit is most pronounced on suites of 50+ fixtures.

## What changes at a glance

The core workflow stays the same: you have `.rs` fixture files and `.stderr`
snapshot files, and CI fails if the actual compiler output doesn't match the
snapshot. What changes is the mechanism:

| Aspect | trybuild | lihaaf |
|---|---|---|
| Harness invocation | `cargo test --test compile_fail` | `cargo lihaaf` |
| Cargo.toml dep | `trybuild = "1"` in `[dev-dependencies]` | `[package.metadata.lihaaf]` block (no library dep) |
| Fixture directory | `tests/ui/` (trybuild default) | `tests/compile_fail/` (recommended) |
| Snapshot normalizer | trybuild uses `$DIR`, `$RUST` | lihaaf uses `$DIR`, `$WORKSPACE`, `$RUST`, `$CARGO/registry/` |
| CI install step | no install step (it's a dep) | `cargo install lihaaf --version X.Y.Z --locked` |

The fixtures themselves (`.rs` files) require no changes. The `.stderr` snapshots
need a one-time re-bless because lihaaf's normalizer applies slightly different
substitutions.

## Pre-migration validation: Paving the way with Compat Mode

Before doing the actual migration (changing configuration files and moving directories), you can run `cargo lihaaf` in compatibility mode. Compat mode helps you assess how close lihaaf's diagnostic output is to your existing trybuild baseline. 

This is particularly useful for measuring baseline behavior across different Rust compiler versions before making any code modifications.

### What compat mode does

When you run with `--compat`, `lihaaf`:
1. Executes your existing trybuild suite via `cargo test` to gather baseline results.
2. Performs a static AST walk of your test files to discover fixture files, copying them and their existing `.stderr` snapshots to a staging directory.
3. Generates a temporary, staged manifest overlay in the cargo target directory (avoiding any edits to your upstream `Cargo.toml`).
4. Dispatches the staged fixtures through lihaaf.
5. Produces a byte-deterministic comparison report (`compat-report.json`) detailing fixture parity, mismatches, and exclusions.

### Example invocation

```bash
cargo lihaaf \
  --compat \
  --compat-root /path/to/target-crate \
  --compat-report compat-report.json
```

Use compat mode on your stable and nightly toolchains to see the normalizer
differences, including lihaaf's foreign-span body suppression (which trybuild
does not apply). This gives you a clear diff before you fully commit to the
migration steps below.

## Step 1: Add lihaaf's `[package.metadata.lihaaf]` block

lihaaf is driven entirely by `[package.metadata.lihaaf]` in the crate's
`Cargo.toml`. There is no library import — you run `cargo lihaaf` as a standalone
binary. The full key reference is in
[`docs/spec/lihaaf-v0.2.md` §3.2](spec/lihaaf-v0.2.md).

**Minimal example** — single crate, fixtures only import from the crate itself
(like anyhow: 7 fixtures, `use anyhow::...`, no macro sibling):

```toml
[package.metadata.lihaaf]
dylib_crate   = "anyhow"
extern_crates = ["anyhow"]
features      = []
edition       = "2021"
fixture_dirs  = ["tests/compile_fail"]
```

`dylib_crate` is the workspace member (or current crate) built as a dylib.
`extern_crates[0]` **must** equal `dylib_crate` — this is enforced at startup.
`fixture_dirs` overrides the spec default (`tests/lihaaf/compile_fail` and
`tests/lihaaf/compile_pass`); the recommended no-infix layout uses
`tests/compile_fail` and `tests/compile_pass` directly (see the [layout section](#layout-choice-no-infix-vs-testslihaaf-infix) below).

**Fuller example** — proc-macro crate with a sibling macros crate, extra dev-deps,
and feature-gated test helpers (like djogi-macros: 237 fixtures, uses
`djogi::prelude::*` which re-exports `djogi_macros::*`):

```toml
[package.metadata.lihaaf]
dylib_crate = "djogi"
extern_crates = ["djogi", "djogi-macros"]
features      = []
dev_deps      = ["serde", "serde_json", "sassi"]
edition       = "2024"
fixture_dirs  = ["tests/compile_fail", "tests/compile_pass"]
```

Key points:

- `extern_crates` lists every crate whose items fixtures refer to with `use`.
  `extern_crates[0]` must be `dylib_crate`. The remaining entries are resolved
  from the dependency graph and forwarded as additional `--extern` flags.
- `dev_deps` lists extra crates that fixtures import directly but that are not
  in `extern_crates` — commonly `serde`, `serde_json`, or test-utility crates.
  Each entry is resolved via `cargo metadata` and forwarded as `--extern`.
- Most migrations do not need `build_targets`. If a split-crate migration fails
  to make a metadata-side dev-dep available to the per-fixture
  `rustc` loop, use the staged workspace shape described in the axum-macros
  grouped-layout section below.
- `features` controls which Cargo features are enabled for both the dylib build
  and every per-fixture `rustc` invocation. Use it when test helpers are gated
  behind a feature (e.g. `features = ["testing"]`).
- `edition` should match the edition of your fixtures. Defaults to `"2021"`;
  valid values are `"2015"`, `"2018"`, `"2021"`, `"2024"`.

The `dylib_crate` / `extern_crates` separation matters when your crate is a
proc-macro wrapper: if fixtures use the *consumer* crate's public API and the
proc-macros are loaded transitively (e.g. via `pub use my_macros::*`), set
`dylib_crate = "my-crate"` and include `"my-macros"` in `extern_crates`. The
dylib is the crate whose items fixtures directly name; the proc-macro crate is
loaded as a proc-macro plugin during that dylib build.

## Step 2: Move fixture files

trybuild conventionally looks for fixtures in `tests/ui/`. Move them to
`tests/compile_fail/` (or `tests/compile_pass/` for pass fixtures):

```bash
# In the crate root where your Cargo.toml lives:
git mv tests/ui/foo.rs tests/compile_fail/foo.rs
git mv tests/ui/foo.stderr tests/compile_fail/foo.stderr
# ... repeat for each fixture pair
```

You do **not** need to rename files. Hyphens in fixture filenames are valid
identifiers for lihaaf — `chained-comparison.rs` stays `chained-comparison.rs`.
The anyhow conversion kept every upstream filename verbatim.

Once all files are moved, remove the now-empty directory:

```bash
git rm -r tests/ui
```

If your repository had trybuild-owned snapshots in a different location (e.g.
`tests/compile_fail/` alongside the driver), move those too. The destination
directory name is what lihaaf uses to classify fixtures as `compile_fail` or
`compile_pass` — any directory whose path contains the string `compile_fail`
(the `compile_fail_marker` default) is treated as a failure-expected suite.

## Step 3: Re-bless `.stderr` snapshots

lihaaf's normalizer applies these substitution tokens to snapshot output:

| Token | Replaced text |
|---|---|
| `$DIR` | Path to the fixture's containing directory |
| `$WORKSPACE` | Workspace root (the directory containing `Cargo.toml`) |
| `$RUST` | Rust sysroot (e.g. `~/.rustup/toolchains/nightly-x86_64-unknown-linux-gnu`) |
| `$CARGO/registry/` | Cargo registry cache path |

trybuild uses `$DIR` and `$RUST` but not `$WORKSPACE` or `$CARGO`. If your
existing snapshots already use `$DIR` (trybuild does emit it), the common
substitution is a path correction — trybuild's `$DIR` points at the workspace
root by default on many projects, while lihaaf's `$DIR` points at the fixture's
immediate containing directory.

**Quick fix for simple cases** — if your snapshots use a path like
`tests/ui/foo.rs` in error spans, update them to `$DIR/foo.rs`:

```bash
# Example: snapshots referenced the old ui/ directory by a relative path.
# After moving files, adjust any hardcoded path segments:
sed -i 's|tests/ui/|$DIR/|g' tests/compile_fail/*.stderr
```

For the common case where trybuild emitted the full path and lihaaf's `$DIR`
will substitute it correctly, you can just delete the old snapshots and re-bless
from scratch:

```bash
cargo lihaaf --bless
```

### Understanding `--bless`

The `--bless` flag runs every fixture, captures the compiler output, normalizes
it, and writes (or overwrites) the corresponding `.stderr` file. The term "bless"
traces back to the Rust compiler's own UI testing workflow: when rustc developers
change error formatting across thousands of test fixtures, they pass `--bless` to
the compiler's build system to trust the new compiler output and overwrite the old
snapshots. Snapshot-testing libraries in the ecosystem—like trybuild and insta—
adopted this design. When you pass `--bless`, you are acting as the authority,
consecrating the current compiler output as the new golden master.

**Hygiene check**: Always run `git diff` after blessing. Ensure the text changes
align with your migration edits (file paths, marker changes, etc.), rather than
masking an unintended compiler regression.

Commit the resulting snapshot files alongside your source changes. If your
snapshots reference stdlib spans (lines like `--> $RUST/core/src/fmt/mod.rs`),
the `$RUST` token will appear in the re-blessed output automatically — no manual
editing needed. See the [CI section](#step-5-update-ci) for the `rust-src`
component requirement that goes with this.

After re-blessing, run `cargo lihaaf` without `--bless` to confirm all fixtures
pass with the committed snapshots.

## Step 4: Remove trybuild infrastructure

Delete the trybuild test driver file. It is typically named `tests/compile_fail.rs`,
`tests/compiletest.rs`, or similar:

```bash
git rm tests/compiletest.rs    # adjust to your actual filename
```

In `Cargo.toml`, drop `trybuild` from `[dev-dependencies]` and any explicit
`[[test]]` entry that points at the old driver:

```diff
 [dev-dependencies]
 futures = { version = "0.3", default-features = false }
 rustversion = "1.0.6"
 syn = { version = "2.0", features = ["full"] }
-trybuild = "1"
 thiserror = "2"

-[[test]]
-name = "compiletest"
-path = "tests/compiletest.rs"
```

**`rustversion` — keep or drop?** Drop it only if it was used exclusively by the
trybuild gate (e.g. `#[rustversion::attr(stable, test)]` wrapping the trybuild
test function). The anyhow conversion kept `rustversion` because `test_ensure.rs` and
`test_backtrace.rs` use `#[rustversion::...]` attributes directly, independently
of the trybuild driver. When in doubt, check if any non-fixture test file imports
`rustversion` before removing it.

If trybuild was a workspace-level dependency (`trybuild.workspace = true` in
per-crate manifests), also remove it from the workspace `Cargo.toml`:

```diff
 [workspace.dependencies]
 syn = { version = "2", features = ["full"] }
-trybuild = { version = "1", features = ["diff"] }
```

Run `cargo check` after these edits to confirm the manifest parses correctly.

## Step 5: Update CI

Replace the trybuild-driven step with a lihaaf install + run. The pattern is the
same whether you're on GitHub Actions or another CI system.

```diff
+      - name: Install cargo-lihaaf
+        run: cargo install lihaaf --version 0.1.1 --locked

       - name: Compile-fixture suite
-        run: cargo test --test compiletest
+        run: cargo lihaaf
```

Pin the version to whatever was used to bless the committed snapshots. `--locked`
makes the install deterministic across runs (uses lihaaf's published `Cargo.lock`).
When you upgrade lihaaf, bump the pin and re-bless snapshots in the same commit.

**`rust-src` component** — keep it if any committed `.stderr` snapshot contains
`$RUST/` (a stdlib span reference), because lihaaf needs the sysroot sources to
resolve those paths. Drop it if no snapshot references `$RUST/`. The sassi
conversion dropped `rust-src` (its fixtures don't reference stdlib internals);
djogi and the anyhow conversion kept it (their snapshots reference `$RUST/core/...`).

```diff
       - uses: dtolnay/rust-toolchain@master
         with:
           toolchain: nightly
-          components: rust-src
+          # Remove rust-src only if no snapshot references $RUST/
+          components: rust-src
```

Run `grep -r '\$RUST' tests/compile_fail/` to check before dropping the component.

For workspace repos with multiple lihaaf-enabled crates, pass
`--manifest-path <crate>/Cargo.toml` to target each crate in turn:

```bash
cargo lihaaf --manifest-path sassi-macros/Cargo.toml
cargo lihaaf --manifest-path djogi-macros/Cargo.toml
```

> [!TIP]
> **Performance optimization for large suites:** If you are migrating a large test suite and want to maximize speed and save disk space, you can pass the `--use-symlink` flag. This replaces the default dynamic library copy with a symbolic link, saving ~30 MB and several hundred milliseconds per run. However, ensure that no concurrent cargo builds are running (e.g., IDE background tasks) while this flag is active to avoid linker errors or compiler crashes.

## Step 6: Verify

Run these three commands in order:

```bash
# 1. Confirm the manifest parses and deps resolve.
cargo check

# 2. Regenerate snapshots from scratch (first time or after diagnostic changes).
cargo lihaaf --bless

# 3. Confirm the gate passes with committed snapshots — this is what CI runs.
cargo lihaaf
```

If `cargo lihaaf` exits 0 and `git status` shows a clean tree after `--bless`,
the conversion is complete. Run `cargo test` as well to verify your non-fixture
unit tests still pass independently.

## Filtering and Test Selection

In a trybuild setup, filtering to run a specific subset of tests or a single test is typically done via the `cargo test` filter:
```bash
cargo test --test compile_fail my_specific_fixture
```
This performs a substring match on the generated test case name.

With `lihaaf`, there is no integration with `cargo test`'s harness filter because `lihaaf` runs as a standalone CLI tool. Instead, you filter fixtures directly on the command line using the `--filter` flag:
```bash
cargo lihaaf --filter my_specific_fixture
```
The filter matches case-sensitively against the relative path of the `.rs` fixture file.

### Migration-Specific Filter Patterns

- **Targeting a group of tests:** If you have organized your tests into subdirectories (e.g., `tests/compile_fail/debug_handler/`), you can run only that group:
  ```bash
  cargo lihaaf --filter debug_handler
  ```
- **Running a single specific test:** To isolate a single failing fixture:
  ```bash
  cargo lihaaf --filter bad_attribute.rs
  ```
- **OR behavior (combining filters):** You can specify multiple `--filter` flags to run fixtures matching any of the substrings:
  ```bash
  cargo lihaaf --filter debug_handler --filter from_ref
  ```

## Layout choice: no-infix vs `tests/lihaaf/` infix

The spec's documented default fixture directories are
`tests/lihaaf/compile_fail` and `tests/lihaaf/compile_pass`. This guide
recommends overriding them with the **no-infix layout**:

```
your-crate/
├── Cargo.toml
└── tests/
    ├── compile_fail/
    │   ├── bad_attr.rs
    │   └── bad_attr.stderr
    └── compile_pass/
        └── happy_path.rs
```

Set `fixture_dirs` accordingly:

```toml
fixture_dirs = ["tests/compile_fail", "tests/compile_pass"]
```

The `tests/lihaaf/` infix (`tests/lihaaf/compile_fail/`) is a transitional shape
that is useful when you are running trybuild and lihaaf **in parallel** during
migration — the infix namespace keeps the two harnesses' file trees from
colliding. Once trybuild is removed, the `lihaaf/` infix is dead namespace that
serves no purpose. The no-infix layout is cleaner and matches how you would lay
out `tests/compile_fail/` for any other testing tool.

Note that the lihaaf spec documents the infix shape as the default. This guide
recommends the no-infix shape for completed migrations; set `fixture_dirs`
explicitly when using it.

## Adoption pattern: single-commit vs two-step

**Single-commit** — convert everything in one commit: add the metadata block,
move files, re-bless snapshots, delete trybuild, update CI. Sassi used this
pattern. It works well for small to medium repos where the full conversion can be
reviewed as a unit and the snapshot re-bless is fast.

**Two-step** — add the lihaaf harness first (parallel with trybuild), then excise
trybuild in a follow-up. Djogi and the anyhow conversion both used this pattern:

1. **Step A** — add `[package.metadata.lihaaf]`, move or symlink fixtures, bless
   lihaaf snapshots. Both trybuild and lihaaf pass CI. The lihaaf fixtures use
   the infix layout (`tests/lihaaf/compile_fail/`) temporarily, so they don't
   stomp trybuild's snapshots.
2. **Step B** — remove trybuild driver, dev-dep, and CI steps. Move fixtures
   out of the infix layout into `tests/compile_fail/` (the no-infix layout). CI
   now gates on lihaaf alone.

The two-step pattern is recommended for repos with large fixture corpora or with
multiple team members committing against the suite, because it lets CI verify
the lihaaf snapshots are stable before trybuild is removed. If Step A CI is green
for a day or two and no snapshot drift appears, Step B excision is low-risk.

The infix layout is only needed during Step A. After Step B, collapse it back to
no-infix.

## Edge cases

### Workspace inheritance

Workspace deps (`dep.workspace = true`) work normally in crates that carry a
`[package.metadata.lihaaf]` block. lihaaf calls `cargo metadata` to resolve the
full dependency graph — workspace inheritance is handled by Cargo before lihaaf
ever sees the resolved paths. No special configuration is needed.

If your crate has a dev-dep cycle (common for proc-macro crates: `crate-macros`
dev-deps `crate`, which regularly depends on `crate-macros`), Cargo permits the
cycle in dev-dependencies because they are excluded from the published graph.
Path-only dev-deps (`djogi = { path = "../djogi" }` with no `version`) are also
fine — lihaaf resolves them through `cargo metadata`.

### Named suites

If you need a subset of fixtures to compile against a different feature set, use
the `[[package.metadata.lihaaf.suite]]` array-of-tables. Djogi uses a `spatial`
suite for fixtures that require the `spatial` feature:

```toml
[package.metadata.lihaaf]
dylib_crate   = "djogi"
extern_crates = ["djogi", "djogi-macros"]
features      = []
edition       = "2024"
fixture_dirs  = ["tests/compile_fail", "tests/compile_pass"]

# Spatial compile-pass fixtures build the dylib with features = ["spatial"].
# They live in their own directory so the default suite stays feature-neutral.
[[package.metadata.lihaaf.suite]]
name         = "spatial"
features     = ["spatial"]
fixture_dirs = ["tests/compile_pass_spatial"]
```

`cargo lihaaf` runs all suites in declared order. `cargo lihaaf --suite spatial`
runs only the named suite. `cargo lihaaf --suite default` runs only the implicit
default suite.

Fields not specified on a named suite (`extern_crates`, `dev_deps`, `edition`,
`compile_fail_marker`, `fixture_timeout_secs`, `per_fixture_memory_mb`,
`allow_lints`) inherit from the top-level `[package.metadata.lihaaf]` table.

### Grouped per-group layouts (upstream `tests/<group>/{fail,pass}/`)

Some upstream crates organize fixtures into per-group subdirectories rather than
the flat `tests/ui/` shape. The axum-macros conversion keeps the upstream layout:

```
axum-macros/tests/
  debug_handler/{fail,pass}/
  debug_middleware/{fail,pass}/
  from_ref/{fail,pass}/
  from_request/{fail,pass}/
  typed_path/{fail,pass}/
```

Two spec rules from §3.6 shape how to configure this:

1. The implicit **default suite** (top-level `[package.metadata.lihaaf]` table)
   must have a non-empty `fixture_dirs` resolving to ≥1 existing directory.
2. `fixture_dirs` across all suites must be **disjoint** — no fixture directory
   may appear in two suites.

Combining these: you cannot have an empty default + every group as a named
suite. The correct shape is **one group as the default suite; remaining groups
as named suites**. axum-macros picks `debug_handler` (the largest group) as
the default:

```toml
[package.metadata.lihaaf]
dylib_crate          = "axum"
extern_crates        = ["axum", "axum-macros"]
features             = ["macros"]
dev_deps             = ["axum-extra", "serde"]
build_targets        = ["tests"]
edition              = "2021"
compile_fail_marker  = "fail"
fixture_dirs         = ["tests/debug_handler/fail", "tests/debug_handler/pass"]

[[package.metadata.lihaaf.suite]]
name         = "debug_middleware"
features     = ["macros"]
dev_deps     = []
fixture_dirs = ["tests/debug_middleware/fail", "tests/debug_middleware/pass"]

[[package.metadata.lihaaf.suite]]
name         = "from_ref"
features     = ["macros"]
dev_deps     = []
fixture_dirs = ["tests/from_ref/fail", "tests/from_ref/pass"]

[[package.metadata.lihaaf.suite]]
name          = "from_request"
features      = ["macros"]
dev_deps      = ["axum-extra", "serde"]
build_targets = ["tests"]
fixture_dirs  = ["tests/from_request/fail", "tests/from_request/pass"]

[[package.metadata.lihaaf.suite]]
name          = "typed_path"
features      = ["macros"]
dev_deps      = ["axum-extra", "serde"]
build_targets = ["tests"]
fixture_dirs  = ["tests/typed_path/fail", "tests/typed_path/pass"]
```

Two extra patterns this surfaces:

- **`compile_fail_marker = "fail"`** — upstream uses `fail/` and `pass/` rather
  than `compile_fail/` and `compile_pass/`. The marker is a substring match
  (default `"compile_fail"`); setting it to `"fail"` makes directories ending in
  `/fail` classify as compile_fail and `/pass` as compile_pass. Named suites
  inherit this via the §3.6 inheritance rule.

- **Default-suite naming** — `cargo lihaaf --suite default` runs the chosen
  default group (`debug_handler` here), not a "default" alias. If you want all
  groups callable by their natural group name, you'd have to duplicate the
  default group as a named suite — which the disjoint rule rejects. Pick the
  default group based on how often it'll be invoked alone (largest, most
  failure-prone, fastest to compile — whatever fits your workflow).

- **`build_targets` is per-suite** — `dev_deps` inherits from the default suite,
  but `build_targets` does not. In the axum-macros conversion, suites whose fixtures
  import `serde` or `axum-extra` need `build_targets = ["tests"]`; suites that do
  not need those crates should set `dev_deps = []` so they stay on the default
  dylib-only build path. This distinction is what keeps the grouped layout from
  compiling against an incomplete dev-dep graph.

### Toolchain-pinned fixtures

If your trybuild driver was wrapped in a `#[rustversion::attr(nightly, test)]`
gate (or similar), you simply run `cargo lihaaf` on the same toolchain the gate
was targeting. lihaaf inherits whatever toolchain the invoking shell has on PATH
— the toolchain selection is Cargo's job, not lihaaf's. In CI, install the
matching toolchain before the `cargo lihaaf` step. The anyhow conversion's nightly-only
fixtures work this way: the CI step uses `dtolnay/rust-toolchain@nightly` and
then runs `cargo lihaaf`.

### `rust-src` component

Keep the `rust-src` component in your CI toolchain install if any committed
`.stderr` snapshot contains `$RUST/`. Drop it if none do. To check:

```bash
grep -r '\$RUST' tests/compile_fail/ tests/compile_pass/
```

If the grep is empty, drop `rust-src`. If it returns hits, keep it — lihaaf
needs the sysroot sources to normalize those paths at bless time.

### `dev_deps` resolution

The `dev_deps` key lists crates that fixtures `use` directly beyond `extern_crates`.
lihaaf resolves each entry via `cargo metadata` and forwards it as a `--extern`
flag on the per-fixture `rustc` invocation. If a fixture fails with
`error[E0432]: unresolved import` for a crate that is in `[dev-dependencies]`
but not in `extern_crates` or `dev_deps`, add it to `dev_deps`.

When the default dylib build already leaves the needed rlibs in Cargo's deps dir,
`dev_deps` is usually enough. If the fixture still fails to resolve a
metadata-side dev-dep after adding it to `dev_deps`, opt that suite into
`build_targets = ["tests"]`. Named suites inherit `dev_deps` when they omit it,
but they do not inherit `build_targets`, so add the opt-in to each named suite
that needs the staged dev-dep collector.

## Troubleshooting and Debugging with `--keep-output`

During migration, you might encounter compilation errors, unresolved imports, or unexpected snapshot mismatches. The `--keep-output` flag acts as an escape hatch to help you diagnose these migration-specific scenarios:

- **Inspecting staged manifests:** In compat mode, `lihaaf` generates a staged manifest overlay inside `target/lihaaf-overlay/Cargo.toml`. If a compat run complains about workspace dependencies or ambiguous specifications, you can pass `--keep-output` to inspect this generated file and check if path-dependencies, workspaces, or patches were absolutized correctly.
- **Investigating failing compiler invocations:** When a fixture fails with unexpected compiler output, you can find its preserved work directory in the system's temporary folder (the exact path will be logged). This directory contains the exact compiled objects, dependency files, and stderr records. You can navigate into the directory and manually run the `rustc` command (print it via `cargo lihaaf -v`) to tweak arguments, isolate failures, or check header linkages.
- **Reviewing staged/copied fixtures:** If AST-based fixture discovery in compat mode behaves unexpectedly, `--keep-output` preserves the copied test fixture files and their snapshots. This lets you confirm that the AST traversal mapped trybuild's `t.compile_fail(...)` calls to the correct directory structures.

## Benchmarking your conversion

The anyhow conversion branch includes a reference benchmark workflow at
`.github/workflows/benchmark.yml` (branch `lihaaf-converted`). It runs a matrix
job — one branch with trybuild, one with lihaaf — and uploads a timing JSON with
wall-clock milliseconds and peak RSS for each. Use it as a template for timing
your own conversion.

Record your own before/after numbers in the conversion change or release notes.
Fixture count, target-dir state, and enabled features affect the ratio enough
that the benchmark workflow is a template, not a universal benchmark.

Note that the dylib build amortizes over the full fixture count. For very small
fixture sets the upfront dylib build dominates and the per-fixture savings are
modest. For suites of 50+ fixtures — and especially for large suites like
djogi-macros (237 fixtures) — the amortized cost is negligible.

## Verifying the conversion shipped correctly

Before merging, confirm:

- [ ] `cargo lihaaf` exits 0 with no snapshot mismatches.
- [ ] `git status` is clean after re-blessing (`cargo lihaaf --bless` produced no
      new diffs, or all diffs have been staged and committed).
- [ ] CI is green on the change — both the normal `cargo test` gate and the new
      `cargo lihaaf` step.
- [ ] `grep -r trybuild .` returns no actionable references (only narrative
      migration notes are acceptable, not unconverted driver calls or live deps).
- [ ] Any downstream crates that used the old trybuild driver as an integration
      test entry point still pass their own test suites.

## Further reading

- [`docs/spec/lihaaf-v0.2.md`](spec/lihaaf-v0.2.md) — full specification: metadata
  schema, normalizer rules, verdict types, named suites, workspace-member entry.
- README [lihaaf vs trybuild](#lihaaf-vs-trybuild) section — rationale and measured
  timing from the djogi-macros conversion (237 fixtures).
- Real-world conversions:
  - sassi [`c143f373`](https://github.com/TarunvirBains/sassi/commit/c143f373) —
    single-commit big-bang, no-infix layout (infix used during conversion, then
    collapsed), no `rust-src`.
  - djogi [`fbc80b52`](https://github.com/TarunvirBains/djogi/commit/fbc80b52) —
    two-step excision, no-infix layout (after infix transitional phase), kept
    `rust-src`.
  - anyhow (`lihaaf-converted` branch) — two-step, no-infix, kept `rust-src`,
    nightly-only, minimal single-crate with no proc-macro sibling.
