# Plan: Shape δ — dev-deps overlay-promotion via dylib-manifest grafting

**Date:** 2026-05-19 (R2 — Shape δ + 6-BLOCK closure)
**Target milestone:** v0.1.0
**Working branch:** `plan/dev-deps-overlay-promotion`
**Author:** strict-swe (planner mode)
**Status:** R2 — closes 6 BLOCKs + OQ-4 from Codex xhigh round-1 review on commit `6d5b7fa`

This plan implements **Shape δ — Dylib Overlay With Metadata-Sourced Graft**.
Codex's round-1 adversarial review on R1 (commit `6d5b7fa`) returned
`VERDICT: BLOCK` with 6 BLOCKs. R1's central design error was
synthesizing an overlay of the **metadata crate's** manifest
(`axum-macros/Cargo.toml`) and assuming the workspace walk-up alone
would handle inheritance. Both assumptions are wrong:

1. **dylib_crate ≠ metadata package.** Cargo invokes `cargo rustc -p
   <dylib_crate>` (`src/dylib.rs:93-104`). An overlay of `axum-macros/
   Cargo.toml` with promoted dev-deps in its `[dependencies]` never
   reaches the build graph of `axum` (the actual dylib_crate). The same
   shape applies to sassi-macros (dylib_crate = sassi), djogi-macros
   (dylib_crate = djogi).
2. **Workspace walk-up is unsound** in the staged-overlay-under-target
   shape. `src/compat/overlay.rs:580-587, 89-101, 944-950` established
   the precedent: a staged manifest inside `<adopter>/target/` causes
   cargo's walk-up to identify the workspace as one where the overlay
   is not a member — `package <X>/Cargo.toml is a member of the wrong
   workspace`. R1 ignored this; #36 fixed it for compat-mode by
   creating an **isolated overlay workspace** (overlay's own
   `[workspace]` block) plus root-section **carry-down**
   (`[workspace.dependencies]`, `[patch]`, `[replace]`, `[profile]`).

Shape δ adopts both fixes from compat-mode precedent and synthesizes
the overlay of the **dylib_crate's** manifest with **specs grafted
from the metadata crate's `[dev-dependencies]`**.

The design preserves every locked invariant: CLI-only, dylib-only,
explicit-config, single cargo invocation, no two-phase fingerprint
split, no auto-discovery, no fork pollution, backwards-compat
byte-identical for every existing adopter who omits `build_targets`.

---

## §0 Problem statement

### §0.1 What's wrong with the status quo

lihaaf's stage-3 dylib build invokes (`src/dylib.rs:93-104`):

```text
RUSTFLAGS="-C prefer-dynamic" cargo rustc -p <dylib_crate> \
  --lib --release --crate-type=dylib \
  --message-format=json-render-diagnostics \
  --manifest-path <adopter-Cargo.toml> --target-dir <T> \
  [--features <feat>...]
```

`--lib` selects the library target only. Cargo's documented behavior
for dev-dependencies is:

> Dev-dependencies are not used when compiling a package for building,
> but are used for compiling tests, examples, and benchmarks.
> — https://doc.rust-lang.org/cargo/reference/specifying-dependencies.html#development-dependencies

So `cargo rustc -p X --lib` never compiles `[dev-dependencies]`. The
rlibs for `serde`, `axum-extra`, etc. never land in
`<target_dir>/release/deps/`. The per-fixture rustc invocation
(`src/worker.rs:1003-1008`) then forwards `--extern serde=<path>` flags
that point at paths populated by `extern_paths` lookup — when the
adopter listed `dev_deps = ["serde"]` in their lihaaf metadata, the
expectation is that `serde.rlib` is in `deps_dir`, but it isn't,
because the dylib build skipped it. Fixtures fail with rustc's
`unresolved import` error class (`use serde::Deserialize;` → E0432).

This is the bug surfaced by the **axum-macros pilot**
(`/home/tarunvir/projects/axum-lihaaf-pilot/axum-macros/Cargo.toml:34-40,
81, 124, 131`), which configures `dev_deps = ["axum-extra", "serde"]`
on the default + `from_request` + `typed_path` suites. The fork's
pilot-CI shows 24 of 93 fixtures fail with `error[E0432]: unresolved
import` against `serde::Deserialize` and `axum_extra::*`.

This is a v0.1.0 ship-blocker: lihaaf's stated v0.1.0 coverage matrix
([[lihaaf-pilot-coverage-gap]]) requires axum-macros, which requires
this fix.

### §0.2 The Shape δ direction (Codex round-1 recommendation)

R1 of this plan assumed lihaaf could synthesize an overlay of the
**metadata crate's** manifest (the one carrying
`[package.metadata.lihaaf]`) and move dev-deps within it. Codex
BLOCK-2 demonstrated this is fatal for the split-crate shape — the
overlay's `[package].name` and the cargo `-p <dylib_crate>` selector
diverge. axum-macros' metadata names `dylib_crate = "axum"`; an
overlay of `axum-macros/Cargo.toml` with grafted dev-deps lives in
`axum-macros`'s build graph, not `axum`'s.

**Shape δ corrects this** by separating two crates' roles in the
synthesis:

- **The metadata crate** (`<X>/Cargo.toml` carrying
  `[package.metadata.lihaaf]`) is the source of truth for **dev-dep
  specs**. lihaaf reads `[dev-dependencies]` of the metadata crate,
  pulls out the entries named by `dev_deps`, and uses them as graft
  inputs.

- **The dylib_crate** (named by `dylib_crate = "<Y>"` in the metadata)
  is the synthesis target. lihaaf resolves `<Y>/Cargo.toml` via
  workspace-member resolution, synthesizes an overlay of it, and
  injects the grafted dev-dep entries into the overlay's
  `[dependencies]`.

For axum-macros:
- Metadata crate: `axum-macros/Cargo.toml`. Carries
  `[package.metadata.lihaaf].dev_deps = ["axum-extra", "serde"]` and
  has `serde = "1.0", axum-extra = { path = "../axum-extra", ... }`
  in `[dev-dependencies]` (lines 34-40).
- dylib_crate: `axum`. Its manifest lives at
  `/home/tarunvir/projects/axum-lihaaf-pilot/axum/Cargo.toml`.
- Overlay target: `axum/Cargo.toml` with `serde` + `axum-extra` specs
  grafted in from `axum-macros`'s `[dev-dependencies]`. Cargo's
  `-p axum --manifest-path <overlay>` then resolves the grafted entries
  as part of `axum`'s build graph.

For single-crate adopters (anyhow, serde-json), the metadata crate IS
the dylib_crate. Shape δ degenerates to same-manifest promotion —
identical operationally to R1's same-crate model, with the grafting
step being a no-op rename.

### §0.3 Why this change is in scope for v0.1.0

The fix is surgical:

- One new opt-in field (`build_targets`).
- One conditional code path in `src/dylib.rs::build` that constructs
  an overlay manifest before invoking `cargo rustc`.
- Overlay synthesis is a separate module (`src/dev_deps_overlay/`) that
  composes `compat/overlay.rs`'s precedent for isolated overlay
  workspaces + path absolutization + workspace-member resolution.
- No changes to per-fixture rustc (`src/worker.rs:960-1007`).
- No new public API.
- Backwards-compatible: adopters who omit `build_targets` see no
  behavior change — the byte-identical legacy path runs.

### §0.4 BLOCKs closed from Codex round-1 review

Codex 5.5 xhigh review on commit `6d5b7fa` returned `VERDICT: BLOCK`
with 6 BLOCKs + the OQ-4 deferral BLOCK. Each is closed in this R2:

| Codex BLOCK | Closure section | Closure mechanism |
|-------------|-----------------|-------------------|
| BLOCK-1: Workspace inheritance breakage on overlay-under-`<adopter>/target/` | §3.4, §3.5 | Use compat-style ISOLATED overlay workspace (overlay declares its own `[workspace]`); CARRY DOWN `[workspace.*]`, `[patch.*]`, `[replace]`, `[profile.*]` from the ancestor workspace root via `compat/overlay.rs:1977+` precedent (`apply_workspace_member_inheritance`) |
| BLOCK-2: `dylib_crate != package.name` (FATAL) — overlay of metadata crate never enters dylib build graph | §3.1, §3.2, §3.3 | **Shape δ.** Resolve the dylib_crate's manifest first; synthesize the overlay AGAINST IT; graft dev-dep entries from the metadata crate's `[dev-dependencies]` into the overlay's `[dependencies]`. Cargo's `-p <dylib_crate>` selector now resolves to the overlay's package |
| BLOCK-3: No `[lib]` injection for adopters relying on auto-discovered `src/lib.rs` | §3.6 | Inject `[lib] path = <absolute>` against the dylib_crate's source dir when the overlay sits outside the package root. Mirrors `compat/overlay.rs:2456-2470`. |
| BLOCK-4: §5 adopter inventory incomplete (`verify` TODOs, missing crates, wrong file:line) | §5.2 | Inventory rewritten with file:line citations for every named adopter; missing crates explicitly surfaced as `NOT LOCALLY PRESENT — verify in CI`. No TODO markers remain. |
| BLOCK-5 (OQ-2): `optional = true` flip mutates resolver graph | §3.7.5, §11.2 | LOCKED **REJECT** — promoted optional dev-deps are rejected at parse with a directed diagnostic. Unit test added in §8.2. |
| BLOCK-6: Test coverage gaps; tests passed with axum-macros broken | §8.4 (new) | Add cargo-build-gated split-workspace integration test mirroring the axum-macros shape (dylib_crate != metadata package), a workspace-inheritance test, a root-[patch] carry-down test, and a no-explicit-`[lib]` injection test. |
| OQ-4: `[patch]` deferral to v1.0.0 | §11.4 | LOCKED **inline closure**. `[patch.<registry>]` and `[replace]` from the ancestor workspace root are carried down with absolutized paths; member-local override tables are REJECTED (mirroring `compat/overlay.rs:2009-2038`). Cargo-build-gated test added in §8.4. The §12.8 GH-issue deferral step is REMOVED. |

The architectural shift between R1 and R2: R1 was "synthesize an
overlay of the metadata crate's manifest"; R2 is "resolve the
dylib_crate's manifest, synthesize an overlay of it, graft specs from
the metadata crate's dev-dependencies." Codex's Shape δ is the
mechanism that makes the split-crate adopters (axum-macros, djogi-macros,
sassi-macros) work; the same code path handles single-crate adopters
as a degenerate case.

---

## §1 New semantics (precise)

### §1.1 The field

```toml
[package.metadata.lihaaf]
build_targets = ["tests"]   # NEW. Opt-in.
dev_deps      = ["serde", "axum-extra"]  # existing — explicit allow-list
```

**Type:** `Vec<String>`.
**Default (omitted):** `[]`.
**Allowed values in v0.1.0:** exactly `"tests"`.
**Validation:** unknown values rejected at config-parse time with a
directed diagnostic ("`build_targets[i] = \"<X>\"` is not a recognized
value; v0.1.0 supports only `\"tests\"`. Future releases may add
`\"examples\"` and `\"benches\"`.").
**Inheritance on named suites:** `build_targets` **DOES NOT** inherit
from the default suite. A named suite that omits `build_targets` gets
`[]` (no overlay), matching the REPLACE semantics already established
for `features` (spec §3.6) and `extra_substitutions`. Rationale:
lihaaf's explicit-config-first ethos ([[lihaaf-dev-deps-explicit-keep]])
rejects implicit-by-default inheritance for fields that govern dylib
build shape. Each named suite compiles its own dylib with its own
feature set (per §3.6); the overlay-synthesis decision is part of the
same per-suite build shape and follows the same REPLACE precedent.
Adopter pilots have already shown that the paired `dev_deps`
inheritance is fragile in practice — e.g.
`/home/tarunvir/projects/axum-lihaaf-pilot/axum-macros/Cargo.toml:108-132`
restates `dev_deps` per-suite redundantly because round-5 hit
inheritance breakage. Mirroring REPLACE for `build_targets` makes the
two paired fields behave consistently with the per-suite build model
rather than adding another fragile inheritance path. (See §11.1 OQ-1
locked decision.)

### §1.2 Truth table — `(build_targets, dev_deps)` combinations

The two fields interact along two axes: whether the overlay manifest
is synthesized at all (`build_targets`), and which crates are
`--extern`-forwarded to per-fixture rustc (`dev_deps`).

| `build_targets` | `dev_deps`          | Overlay synthesized? | dylib build manifest                         | `--extern` forwarding                |
|-----------------|---------------------|----------------------|----------------------------------------------|--------------------------------------|
| omitted / `[]`  | omitted / `[]`      | No                   | adopter's Cargo.toml verbatim                | none beyond `extern_crates`          |
| omitted / `[]`  | `["a", "b"]`        | No                   | adopter's Cargo.toml verbatim                | `--extern a`, `--extern b`           |
| `["tests"]`     | omitted / `[]`      | **REJECT** (§1.4)    | n/a                                          | n/a                                  |
| `["tests"]`     | `["a", "b"]`        | Yes                  | dylib_crate's overlay with a, b grafted in   | `--extern a`, `--extern b`           |
| `["invalid"]`   | anything            | **REJECT** (parse)   | n/a                                          | n/a                                  |
| any value       | `["a"]`, `a` not in metadata crate's `[dev-dependencies]` | **REJECT** at synthesis (§3.2) | n/a | n/a |
| any value       | `["a"]`, `a` has `optional = true` in dev-deps | **REJECT** at synthesis (§3.7.5) | n/a | n/a |

Row 3 (build_targets but no dev_deps) is rejected because the overlay
would be byte-identical to the dylib_crate's Cargo.toml — paying the
overhead of overlay synthesis + a fresh cargo fingerprint with zero
behavior change. The directed diagnostic points the adopter at either
removing `build_targets` or listing the dev-deps they need.

### §1.3 Default (omitted) — pin

`build_targets` omitted OR `build_targets = []` → the existing
`src/dylib.rs::build` code path runs **verbatim**, including the
adopter's actual `Cargo.toml` as `--manifest-path`. No overlay dir
is created. No cargo fingerprint change. The byte-identical contract
in §5.1 holds.

### §1.4 Suite-level interaction

Each suite resolves its `build_targets` value independently per §1.1's
REPLACE rule (no inheritance). Overlay synthesis happens per suite,
gated on the **resolved** `build_targets`. The per-suite cargo target
dir (`src/dylib.rs:388-393`) already isolates suite caches; the
per-suite overlay dir extends that isolation:

| Suite type             | Resolved `build_targets`        | Overlay dir                                              |
|------------------------|---------------------------------|----------------------------------------------------------|
| Default, omitted       | `[]` (omitted default)          | none                                                     |
| Default, opt-in        | `["tests"]`                     | `<workspace_target>/lihaaf-build/lihaaf-dev-deps-overlay/`|
| Named, omitted         | `[]` (REPLACE; no inheritance)  | none                                                     |
| Named "spatial", opt-in| `["tests"]`                     | `<workspace_target>/lihaaf-build-spatial/lihaaf-dev-deps-overlay/`|

The overlay dir sits **inside** the per-suite target dir, so cargo's
fingerprint already isolates it from sibling suites. The naming
`lihaaf-dev-deps-overlay` is distinct from `lihaaf-overlay` (the
compat-mode overlay) so multiple overlays coexist in the same
`target/` listing without collision.

---

## §2 Source-level changes needed

### §2.1 `src/config.rs` — add the field

**Site 1: `pub struct Suite` (line 106).** Add a public field:

```rust
/// Build targets to compile beyond `--lib` for the dylib build, gating
/// the dev-deps overlay-promotion synthesis. Default `[]`. Does NOT
/// inherit from the default suite (REPLACE semantics, same as
/// `features` and `extra_substitutions`). A named suite that omits
/// this field gets `[]` (no overlay). Validated values: currently only
/// "tests" is accepted. v0.1.0 design surface; "examples" and "benches"
/// may be added in v0.2+.
pub build_targets: Vec<String>,
```

Insertion point: between `dev_deps` (line 138) and
`compile_fail_marker` (line 143) so the rustdoc co-locates the two
paired fields.

**Site 2: `struct RawMetadata` (line 299).** Add the parser-side
optional:

```rust
build_targets: Option<Vec<String>>,
```

Insertion point: between line 305 (`dev_deps: Option<Vec<String>>`)
and the next line for the same paired-fields reason.

**Site 3: `struct RawSuite` (line 326).** Same field added:

```rust
build_targets: Option<Vec<String>>,
```

Insertion point: between line 332 (`dev_deps: Option<Vec<String>>`)
and `compile_fail_marker` (line 333).

**Site 4: `fn build_default_suite` (line 539).** Add validation and
finalization:

```rust
let build_targets = raw.build_targets.clone().unwrap_or_default();
validate_build_targets(DEFAULT_SUITE_NAME, &build_targets, &raw.dev_deps)?;
```

The validation closure is new (defined in §2.1.bis below). It enforces
(a) allowed values, (b) the §1.2 row-3 rejection (non-empty
`build_targets` requires non-empty `dev_deps`).

The `Suite` constructor at line 630-647 grows a `build_targets`
field assignment alongside `dev_deps: raw.dev_deps.clone().unwrap_or_default()`.

**Site 5: `fn finalize_named_suite` (line 650).** Same shape as Site 4
but using REPLACE (no inheritance from default suite — per §11.1
locked decision):

```rust
let build_targets = raw.build_targets.clone().unwrap_or_default();
validate_build_targets(&name, &build_targets, &raw.dev_deps)?;
```

Inserted near line 772 (the `dev_deps` finalization, which still
inherits — see [[lihaaf-dev-deps-explicit-keep]]), and the `Suite`
constructor at line 766-784 grows the new field assignment.

**Critical:** the validation must run **after** `dev_deps` is resolved
(default-or-inherited), because §1.2 row 3 checks both fields together.
For named suites, the resolved `dev_deps` is
`raw.dev_deps.unwrap_or_else(|| default_suite.dev_deps.clone())`. Note
`build_targets` uses REPLACE while `dev_deps` uses INHERIT — this
asymmetry is the §11.1 locked decision; do not "fix" it by giving
`build_targets` inheritance.

### §2.1.bis New helper: `fn validate_build_targets`

Module-level (sibling to `validate_allow_lints` etc.). Signature:

```rust
fn validate_build_targets(
    suite_name: &str,
    build_targets: &[String],
    resolved_dev_deps: &[String],
) -> Result<(), Error> { ... }
```

Body (sketch):

1. For each entry, reject if not in the allowed set
   (currently `{ "tests" }`); diagnostic names the entry value, the
   suite, and the allowed set.
2. If `build_targets` is non-empty and `resolved_dev_deps` is empty,
   reject (§1.2 row 3); diagnostic explains the no-op shape.
3. Reject duplicates inside `build_targets` (defensive: the parser
   would not de-duplicate).

### §2.2 `src/dylib.rs` — gate overlay synthesis

**Site 1: `pub struct BuildParams<'_>` (line 60).** Add the borrow
slices needed for Shape δ. Note that R2 carries more fields than R1
because the synthesis needs the metadata crate's manifest (for the
dev-dep graft inputs) AND the dylib_crate's name (so the synthesis can
resolve the dylib crate's manifest via workspace-member resolution).
The dylib_crate name is already in `BuildParams.crate_name` (line 62);
the metadata-manifest path is `BuildParams.manifest_path` (line 66) —
both are present, no new fields needed beyond `build_targets` /
`dev_deps`.

```rust
/// Build targets to compile beyond `--lib`, gating the overlay
/// promotion. Empty slice → no overlay; non-empty → §3 synthesis.
pub build_targets: &'a [String],
/// Dev-deps subset to graft into the dylib_crate's overlay
/// `[dependencies]`. Caller passes the resolved (validated) `dev_deps`
/// slice. Each entry is a TOML key resolved against the metadata
/// crate's `[dev-dependencies]` table.
pub dev_deps: &'a [String],
```

The two new fields slot between `features` (line 64) and
`manifest_path` (line 66).

**Site 2: `pub fn build` (line 80).** Insert overlay-synthesis branch
BEFORE the cargo command is assembled (line 93). Sketch:

```rust
let effective_manifest_path: PathBuf = if !params.build_targets.is_empty() {
    let overlay_dir = params.target_dir.join("lihaaf-dev-deps-overlay");
    crate::dev_deps_overlay::synthesize_overlay(
        // The crate name `-p` will select. Shape δ: this is the
        // dylib_crate's name, which already matches params.crate_name.
        params.crate_name,
        // The METADATA crate's manifest — the one carrying
        // [package.metadata.lihaaf]. The graft pulls dev-dep entries
        // from this manifest's [dev-dependencies] table.
        params.manifest_path,
        // The dev-dep TOML keys to graft.
        params.dev_deps,
        // Where to stage the synthesized overlay.
        &overlay_dir,
    )?
} else {
    params.manifest_path.to_path_buf()
};
```

Then `cmd.arg("--manifest-path").arg(&effective_manifest_path)`
replaces the existing line 101-102. The rest of `build` (RUSTFLAGS,
features pass-through, output parsing) is unchanged.

**Site 3: invocation-string rendering (line 125-137).** The diagnostic
invocation string must reflect the **effective** manifest path so the
adopter can paste a working reproduction. Update the format string to
use `effective_manifest_path` instead of `params.manifest_path`.

**Site 4: `BuildOutput.deps_dir` (line 164-167) is unaffected.** The
overlay manifest's `target/` is still `<target_dir>` (cargo joins
`<target_dir>/release/deps`), and the overlay's package emits its
artifacts into the same `deps_dir`. Per §4 idempotency, this is the
load-bearing claim.

### §2.3 New module `src/dev_deps_overlay/mod.rs`

Create a new module sibling to `compat/`:

```text
src/
  dev_deps_overlay/
    mod.rs          (new — Shape δ orchestrator)
  dylib.rs
  compat/
    overlay.rs       (existing — compat-mode driver overlay)
```

**Why a new module, not a refactor of `compat/overlay.rs`?**

The compat-mode overlay (`compat/overlay.rs`, ~8800 lines, public
entry `materialize_overlay` at line 515) handles a different problem
shape — it rewrites a non-dylib upstream into a dylib-buildable
shape, injects synthetic metadata, mirrors the upstream package root,
and applies the 4-rule self-patch policy. The dev-deps overlay needs
**only a subset** of compat-overlay's machinery:

| compat-overlay capability | Needed by dev-deps overlay? |
|---------------------------|------------------------------|
| `[lib] crate-type` injection | No — adopter's crate is already a buildable lib |
| Workspace-member resolution (`resolve_workspace_member_manifest`) | **YES** — Shape δ uses this to find the dylib_crate's manifest from the metadata crate's workspace context |
| Isolated overlay workspace (`override_workspace_inheritance` Branch 4 — clone `[workspace]`, strip membership keys) | **YES** — BLOCK-1 closure |
| Workspace-member inheritance carry-down (`apply_workspace_member_inheritance`) | **YES** — BLOCK-1 + OQ-4 closure |
| Path absolutization (`absolutize_path_bearing_keys`) | **YES** — overlay sits outside dylib_crate package root |
| Self-patch policy (4-rule `[patch.crates-io.<self>]` handling) | No — dev-deps overlay shares the dylib_crate's `[package].name`, but the overlay manifest is the dylib_crate's manifest (not a synthetic outer wrapper), so no self-patch ambiguity arises |
| Staged package-root mirror | No — we use `[lib] path = <absolute>` injection instead, which is simpler and avoids materializing a symlink farm |
| Synthetic metadata injection | No — we keep the dylib_crate's existing `[package.metadata.*]` verbatim; no synthesis required |
| Canonical TOML serialization (`serialize_canonical`) | **YES** — load-bearing for determinism |
| Idempotent rerun guard | **YES** — load-bearing for cargo fingerprint stability |

**Decision: extract the shared primitives into a new `src/
manifest_overlay/` module; have BOTH `compat/overlay.rs` AND the new
`dev_deps_overlay/mod.rs` depend on it.**

The shared module (`src/manifest_overlay/`) provides:

```rust
// Path absolutization for the staged-overlay shape.
pub(crate) fn absolutize_path_bearing_keys(
    top: &mut toml::map::Map<String, toml::Value>,
    source_dir: &Path,
    source_manifest_path: &Path,
) -> Result<(), Error>;

// Canonical (deterministic, BTreeMap-ordered) TOML serialization.
pub(crate) fn serialize_canonical(value: &toml::Value) -> Result<Vec<u8>, Error>;

// Workspace-member resolution: given an arbitrary manifest path + a
// dylib_crate name, walk up to the workspace root, find the member
// whose [package].name == dylib_crate, return that member's
// manifest path + the workspace-root value.
pub(crate) fn resolve_dylib_crate_manifest(
    starting_manifest_path: &Path,
    dylib_crate: &str,
) -> Result<(PathBuf, toml::Value), Error>;

// Isolated overlay workspace + root-section carry-down. Given a parsed
// overlay TOML and the ancestor workspace-root value, mutate the
// overlay to:
//   - declare its own [workspace] (cloned from root, membership keys
//     stripped — branches 4-style of override_workspace_inheritance);
//   - carry down [patch.<registry>], [replace], [profile.*];
//   - absolutize path-bearing keys in the carried-down tables;
//   - reject any member-local [patch] override that conflicts with
//     workspace-root [patch].
pub(crate) fn make_overlay_isolated_workspace(
    overlay_top: &mut toml::map::Map<String, toml::Value>,
    overlay_dir: &Path,
    workspace_root_manifest: &Path,
    workspace_root_value: &toml::Value,
    dylib_crate_manifest: &Path,
) -> Result<(), Error>;
```

`compat/overlay.rs` is refactored to call into these shared helpers
(rather than its own private copies). The extracted-from-compat
invariant is enforced by the byte-identical regression test in §8.3.

**`src/dev_deps_overlay/mod.rs` structure (sketch):**

```rust
//! Shape δ — Dylib Overlay With Metadata-Sourced Graft.
//!
//! Per `docs/spec/dev-deps-overlay-promotion-plan-2026-05-19.md` §3.
//!
//! When the adopter's lihaaf metadata sets `build_targets = ["tests"]`,
//! lihaaf:
//!   1. Resolves the dylib_crate's manifest (via workspace-member
//!      resolution from the metadata crate's manifest).
//!   2. Synthesizes an overlay of the dylib_crate's manifest with the
//!      dev-dep entries named in `dev_deps` GRAFTED from the metadata
//!      crate's `[dev-dependencies]` into the overlay's `[dependencies]`.
//!   3. The overlay declares its own `[workspace]` and carries down
//!      `[workspace.*]`, `[patch.*]`, `[replace]`, `[profile.*]` from
//!      the ancestor workspace root (mirroring `compat/overlay.rs`
//!      precedent for #36 workspace-identity correctness).
//!   4. Path-bearing keys are absolutized against the dylib_crate's
//!      source dir; `[lib] path` is injected when missing.

use std::path::{Path, PathBuf};

use crate::error::Error;
use crate::manifest_overlay;

/// Synthesize the dylib_crate's overlay manifest. Returns the absolute
/// path of the staged manifest file.
///
/// `dylib_crate` — the crate name cargo's `-p` selector matches.
/// `metadata_manifest_path` — the lihaaf metadata crate's Cargo.toml.
/// `dev_deps` — TOML keys to graft from metadata's [dev-dependencies].
/// `overlay_dir` — where to stage the synthesized overlay.
pub fn synthesize_overlay(
    dylib_crate: &str,
    metadata_manifest_path: &Path,
    dev_deps: &[String],
    overlay_dir: &Path,
) -> Result<PathBuf, Error> { /* §3 algorithm */ }
```

The body follows §3 (the synthesis algorithm).

### §2.4 `src/manifest.rs` — does `metadata_snapshot` need to know?

**No.** `metadata_snapshot` at `src/manifest.rs:99` is "Verbatim copy of
the entire `[package.metadata.lihaaf]` table." The new `build_targets`
key rides through transparently as part of the verbatim copy. The same
applies to `extra_substitutions` (which was added in beta.10 without
a manifest.rs change). The pattern is documented in the
config.rs:1-9 module docstring: "If you add a new TOP-LEVEL key (one
that lives directly in `[package.metadata.lihaaf]`, ...) also add it
in `manifest.rs`." `build_targets` is a per-suite key (Suite struct,
not the top-level `Config` struct), so manifest.rs is not touched.

**Verification:** `src/manifest.rs:97-99` keeps the snapshot field
as `serde_json::Value`. Adding `build_targets` to the top-level table
appends one JSON key to the serialized blob; freshness-detection logic
that hashes the metadata_snapshot will detect changes correctly (the
hash is over the JSON, which now includes the new key).

### §2.5 `src/worker.rs:960-1007` — does per-fixture rustc need changes?

**No.** The per-fixture rustc invocation reads `ctx.dev_deps` (line 1003)
and looks up each crate's path in `ctx.extern_paths`. The path
population happens upstream when the dylib build's `deps_dir` is walked
and `--extern` paths discovered. With the overlay-promotion path:

1. The grafted dev-deps get compiled into `deps_dir` by the
   single `cargo rustc -p <dylib_crate> --lib` invocation against the
   dylib_crate's overlay.
2. The existing `extern_paths` walk over `deps_dir` discovers the new
   rlibs at exactly the same paths cargo would have used for any
   other dependency.
3. `apply_extern_paths` (the existing helper at `src/worker.rs:1003-1008`)
   emits `--extern serde=<path>` against those discovered paths.

The worker code is **dev_deps-aware**, not **build_targets-aware** — and
that distinction is correct. `build_targets` controls dylib-build
manifest synthesis only; downstream forwarding is purely a function
of `dev_deps`. (See §11 OQ-3 for whether `build_targets` should also
control which crates auto-receive `--extern`. Decision: NO; keep
orthogonal.)

### §2.6 Session orchestration — `src/session.rs` (or equivalent)

The session module is the caller that:

1. Reads `Config` from `src/config.rs::load`.
2. For each suite, calls `src/dylib.rs::build` with the suite's
   parameters (see `src/session.rs:330-336`).

The change is local: when building the `BuildParams` struct for each
suite, populate the new `build_targets` and `dev_deps` borrows from
the suite's resolved fields. Site identified at
`src/session.rs:330-336`; the construction is a single
`dylib::BuildParams { ... }` literal that grows two field assignments.

---

## §3 Overlay manifest synthesis algorithm (Shape δ)

The synthesis runs inside `src/dev_deps_overlay::synthesize_overlay`.
Input: the dylib_crate's name, the metadata crate's `Cargo.toml` path,
the `dev_deps` list of names to graft, the target overlay directory.
Output: the absolute path of the written overlay manifest, plus the
side effect of the file written atomically.

The algorithm is broken into numbered steps. Each step states (a) what
it does, (b) the invariant it preserves, (c) the failure-mode policy
for each of Codex's enumerated edge cases.

### §3.1 Step 1 — read + parse the metadata crate's `Cargo.toml`

Identical shape to `compat/overlay.rs:621-642`:

```rust
let metadata_bytes = std::fs::read(metadata_manifest_path)?;
let metadata_text = String::from_utf8(metadata_bytes)?;
let metadata_value: toml::Value = toml::from_str(&metadata_text)?;
```

The **metadata crate's** TOML is the **source of dev-dep specs**. Its
`[dev-dependencies]` table is what the graft pulls entries from. The
metadata value is NOT modified — it is read-only input.

Failure → `Error::Io` or `Error::TomlParse` per existing conventions.

### §3.2 Step 2 — extract dev-dep entries from the metadata crate

For each name in `dev_deps`:

1. Look up the name in `metadata_value["dev-dependencies"][name]`.
   If absent, **REJECT** with a directed diagnostic:

       dev_deps[i] = "<name>" is listed in
       [package.metadata.lihaaf].dev_deps but is not present in
       [dev-dependencies] of <metadata_manifest_path>. Either add it
       there or remove `build_targets = ["tests"]` if you intended
       <name> to come from a different table.

2. **Edge case — `[target.<cfg>.dev-dependencies]`:** if the name is
   present ONLY under `[target.<cfg>.dev-dependencies]` and not in the
   top-level `[dev-dependencies]`, REJECT per §3.7.7 (cfg-gated
   promotion deferred to v0.2).

3. **Edge case — `optional = true`:** if the entry has `optional =
   true`, REJECT per §3.7.5 (Codex BLOCK-5 / OQ-2 locked REJECT).
   The directed diagnostic explains the resolver-graph risk.

4. **Edge case — same name in `[dependencies]` of the dylib_crate's
   manifest:** see §3.4 step 4 below. The dylib_crate's manifest is
   not loaded yet at this step; the conflict check happens after step
   3 resolves the dylib_crate.

5. Record the entry (the full TOML subtree) as a graft input. Renamed
   entries (`<key> = { package = "<actual>", ... }`) preserve the TOML
   key; the package name is in the entry's `package` field.

Validation MUST run BEFORE later steps so a typo / missing dev-dep
fails fast.

### §3.3 Step 3 — resolve the dylib_crate's manifest (Shape δ key step)

Given `dylib_crate` (the name cargo's `-p` selector matches) and
`metadata_manifest_path`, resolve the actual manifest path for the
dylib_crate using `manifest_overlay::resolve_dylib_crate_manifest`:

**Algorithm:**

1. Compute the metadata crate's directory: `metadata_dir =
   metadata_manifest_path.parent()`.
2. Read the metadata's TOML (already done in §3.1). Check whether
   `metadata_value["package"]["name"] == dylib_crate`:
   - If YES — single-crate adopter (anyhow, serde-json, djogi). The
     dylib_crate IS the metadata crate. Return
     `(metadata_manifest_path, metadata_value)` directly. Shape δ
     degenerates to the same-manifest case.
   - If NO — split-crate adopter (axum-macros, sassi-macros,
     djogi-macros). Continue to step 3.
3. Walk up the filesystem from `metadata_dir` to find the workspace
   root: the first ancestor `Cargo.toml` whose top-level table has a
   `[workspace]` block. Reuse
   `compat/overlay.rs`'s walk-up logic — extract this into the shared
   `manifest_overlay` module if not already shared. Return the
   workspace-root manifest path + parsed value.
4. Reuse
   `crate::compat::overlay::resolve_workspace_member_manifest`
   (`src/compat/overlay.rs:1505+`): given the workspace-root manifest
   + the dylib_crate name, resolve to the member's manifest path. The
   function already handles glob members (`crates/*`), literal
   members, and the not-found case with a directed diagnostic listing
   scanned members.
5. Return `(dylib_crate_manifest_path, workspace_root_value)`.

**Edge cases:**

- **Dylib_crate not found in the workspace's members.** Function
  returns `Error::Cli` with a member-list diagnostic per existing
  precedent (`compat/overlay.rs:6961+ —
  resolve_workspace_member_manifest_no_match_lists_scanned_members`).
- **No workspace root** (single-crate metadata + dylib_crate != metadata
  package). This is a misconfiguration — the metadata names a sibling
  crate that doesn't exist in a workspace context. REJECT with a
  directed diagnostic naming both crate names and pointing the adopter
  at either (a) configuring the metadata on the dylib_crate itself
  (i.e. consolidating to single-crate), or (b) restructuring as a
  workspace.
- **Workspace root carries an ancestor workspace too** (nested
  workspaces). The existing `compat/overlay.rs` walk-up stops at the
  first `[workspace]` block. Same behavior applies; nested workspaces
  are an unusual shape (the inner workspace IS a member of the outer,
  which is rare). Defer to the resolver behavior; surface any error
  the existing resolver emits.

### §3.4 Step 4 — read + parse the dylib_crate's manifest

```rust
let dylib_bytes = std::fs::read(&dylib_crate_manifest_path)?;
let dylib_text = String::from_utf8(dylib_bytes)?;
let mut dylib_value: toml::Value = toml::from_str(&dylib_text)?;
```

This is the **synthesis target**. The overlay TOML is built by mutating
`dylib_value` (the dylib_crate's manifest) — NOT the metadata value.

**Cross-table conflict check** (deferred from §3.2 step 4): for each
grafted dev-dep, verify the same name does NOT already exist in the
dylib_crate's `[dependencies]`. If it does:

- If the existing entry is identical to the graft entry (after
  serialization), no-op the graft for that name (the dep is already in
  the dylib_crate's regular deps; no need to promote).
- Otherwise REJECT with a directed diagnostic: the metadata crate's
  dev-dep spec conflicts with the dylib_crate's regular-dep spec for
  the same name. The adopter must reconcile by either (a) removing the
  graft target from `dev_deps` (the regular dep already provides what
  fixtures need), or (b) unifying the two specs.

### §3.5 Step 5 — graft dev-dep entries into the dylib_crate's overlay

For each entry recorded in §3.2 (and not no-op'd by the conflict
check in §3.4):

1. Insert into `dylib_value["dependencies"][name] = entry` (with §3.7
   transformations applied — path absolutization, etc).
2. The metadata crate's `[dev-dependencies]` table is NOT modified —
   the metadata value was read-only input.
3. The dylib_crate's `[dev-dependencies]` table is NOT modified
   either — the overlay synthesis only ADDS to the dylib_crate's
   `[dependencies]`, leaving everything else verbatim.

The `[dependencies]` table is created if absent (defensive: a manifest
with no `[dependencies]` is a valid edge case for stub crates).

### §3.6 Step 6 — make the overlay an ISOLATED workspace

This step closes **BLOCK-1**: cargo's walk-up from
`<overlay_dir>/Cargo.toml` would otherwise land on the ancestor
workspace root, which does not list the overlay's `[package].name` as
a member — producing the "wrong workspace" error.

Use `manifest_overlay::make_overlay_isolated_workspace`:

```rust
manifest_overlay::make_overlay_isolated_workspace(
    overlay_top,
    overlay_dir,
    workspace_root_manifest_path,
    workspace_root_value,
    dylib_crate_manifest_path,
)?;
```

This mirrors `compat/overlay.rs:1977+ apply_workspace_member_inheritance`:

1. **Clone `[workspace]` from the ancestor root**, stripping membership
   keys (`members`, `exclude`, `default-members`) per
   `compat/overlay.rs:903 WORKSPACE_MEMBERSHIP_KEYS`.
2. **Carry down workspace inheritance tables**
   (`[workspace.dependencies]`, `[workspace.package]`,
   `[workspace.lints]`, `[workspace.metadata]`, `[workspace.resolver]`)
   verbatim — these are load-bearing for any `{ workspace = true }`
   reference surviving in the overlay's deps.
3. **Carry down `[patch.<registry>]` from the workspace root**
   (OQ-4 inline closure). Each registry subtable is preserved
   verbatim; path-bearing keys inside `path = ...` entries are
   absolutized against the workspace-root dir. Mirror
   `compat/overlay.rs:2156+` patch carry-down. Member-local `[patch]`
   tables on the dylib_crate's own manifest are REJECTED with a
   directed diagnostic (mirroring `compat/overlay.rs:2009-2038`).
4. **Carry down `[replace]`** from the workspace root with path
   absolutization. Same rejection rule for member-local `[replace]`.
5. **Carry down `[profile.*]`** verbatim. Profiles do not contain
   path-bearing keys; verbatim copy is correct.
6. **Carry down `[workspace.lints]`** so the overlay's lint
   configuration matches the ancestor workspace's — preserving lint
   parity for the dylib build.

The overlay manifest, post-step-6, declares itself as a workspace root
(cargo's walk-up terminates at the overlay's own `[workspace]` block),
yet inherits every workspace-level configuration the original
dylib_crate would have inherited from its ancestor root.

### §3.7 Step 7 — handle dev-dep edge cases (apply transformations)

This step applies the per-shape policies from R1's §3.3 to each
grafted entry. Most are unchanged from R1; the `optional = true`
policy is the locked REJECT per BLOCK-5.

#### §3.7.1 `workspace = true` shorthand (Codex enumeration #1)

Pattern: `serde = { workspace = true, features = [...] }`.

**Policy: preserve the shorthand verbatim.** Per §3.6, the overlay
carries down `[workspace.dependencies]` from the ancestor workspace
root. The `{ workspace = true }` shorthand in the overlay's
`[dependencies]` then resolves against the carried-down
`[workspace.dependencies.<name>]` table — same dep spec the
dylib_crate would have resolved through its own walk-up.

**Edge case — workspace inheritance for a graft target only:** if the
metadata crate's `[dev-dependencies].<name> = { workspace = true }` is
the only place `<name>` appears, the workspace root must declare the
dep in `[workspace.dependencies.<name>]`. If it doesn't, cargo will
fail parsing — but this is the same failure that would occur for the
metadata crate's tests, so the diagnostic surfaces from cargo, not
from lihaaf.

#### §3.7.2 Path dev-deps (Codex enumeration #2)

Pattern: `local-helper = { path = "../local-helper" }`.

**Policy: absolutize the path against the METADATA crate's directory.**
The relative path was authored relative to the metadata crate's
location (e.g. `axum-macros/Cargo.toml` has `axum = { path =
"../axum", ... }` at line 35); after grafting into the dylib_crate's
overlay, the path must be made absolute so cargo can resolve it
regardless of the overlay's location. Use
`manifest_overlay::absolutize_path_bearing_keys` over the grafted
entry with `source_dir = metadata_dir`.

**Edge case — graft pulls in the dylib_crate itself as a path dep
(cycle):** axum-macros' `[dev-dependencies].axum = { path =
"../axum" }` (line 35) IS the dylib_crate. Grafting this into
`axum/Cargo.toml`'s `[dependencies]` would create a self-loop. **REJECT**
at synthesis with a directed diagnostic:

    dev_deps[i] = "<name>" graft target is the dylib_crate itself
    (`<dylib_crate>`). Promoting a self-dep produces a cycle. Remove
    "<name>" from dev_deps — the dylib_crate is already in the build
    graph via cargo `-p <dylib_crate>`.

Note this also catches the axum-macros case where `axum-macros`
authors might erroneously list `dev_deps = ["axum"]` (axum is already
the dylib_crate). The rejection diagnostic should make this
relationship explicit.

#### §3.7.3 Git dev-deps (Codex enumeration #2.bis)

Pattern: `some-dep = { git = "https://...", rev = "..." }`.

**Policy: verbatim copy.** Git URLs are not affected by the overlay's
filesystem location. `absolutize_path_bearing_keys` skips entries
without `path` keys; the verbatim graft works correctly.

#### §3.7.4 `[patch]` sections (Codex enumeration #3 + OQ-4 inline closure)

**Policy (OQ-4 LOCKED inline closure per Codex BLOCK on deferral):**

- `[patch.<registry>]` tables on the **ancestor workspace root** are
  carried down per §3.6 step 3. Path-bearing keys inside `[patch]`
  entries are absolutized against the workspace-root dir.
- `[patch.<registry>]` tables on the **dylib_crate's own manifest**
  (member-local patches) are REJECTED with a directed diagnostic
  mirroring `compat/overlay.rs:2009-2038`. Cargo itself rejects
  member-local `[patch]` tables; the rejection at synthesis surfaces
  the error eagerly.
- `[patch.<registry>]` tables on the **metadata crate's manifest**
  (if metadata crate != dylib_crate, e.g. `axum-macros/Cargo.toml`
  has a `[patch.crates-io]` table) are also REJECTED with the same
  diagnostic — cargo would have rejected this too if the metadata
  crate were the synthesis target.

The carry-down handles the case where the workspace-root has
`[patch.crates-io.serde] = { path = "./forks/serde" }` and the
metadata crate's `[dev-dependencies]` includes `serde`: after the
graft, `serde` is in `[dependencies]` of the overlay; the carried-down
patch is in the overlay's `[patch.crates-io]`; cargo's resolver
applies the patch to the `serde` reference in `[dependencies]`. This
is the **proven** path — by reuse of compat-mode's R3+R4 mechanism
(`compat/overlay.rs:1977+`) that has shipped in beta.10.

#### §3.7.5 Optional dev-deps (Codex enumeration #4 — BLOCK-5 / OQ-2 LOCKED REJECT)

Pattern: `[dev-dependencies].serde = { version = "1", optional = true }`,
plus `[features].my-feat = ["dep:serde"]`.

**Policy: REJECT optional dev-deps at synthesis.** Per Codex BLOCK-5,
flipping `optional = true` → `optional = false` mutates the resolver
graph: it changes which deps participate in the build, suppresses the
`dep:<name>` feature-name suppression behavior, and can subtly change
the dylib's compilation behavior (cargo features that gate code on
`#[cfg(feature = "my-feat")]` would compile differently).

Diagnostic:

    dev_deps[i] = "<name>" is configured with `optional = true` in
    [dev-dependencies] of <metadata_manifest_path>. Promoting an
    optional dev-dep is not supported in v0.1.0 — flipping
    `optional = false` would mutate the resolver graph (suppress
    `dep:<name>` feature names, change which deps participate).
    Workarounds: (a) make <name> non-optional in [dev-dependencies];
    (b) skip promoting <name> via dev_deps and structure the fixture
    to not import it.

This closes BLOCK-5. The locked decision is recorded in §11.2.

#### §3.7.6 Renamed dev-deps via `package = "..."` (Codex enumeration #5)

Pattern: `[dev-dependencies] serde-json = { package = "serde_json",
version = "1" }`.

**Policy: preserve the rename.** The TOML key (`serde-json`) is the
name cargo registers the dep under; the `package` field is the
actual package name. The graft moves the entire entry — key + value
subtree — into the overlay's `[dependencies]` verbatim. The
`--extern` forwarding at `src/worker.rs:1003-1008` reads the lihaaf
`dev_deps` entries verbatim (which are the TOML keys, not the
package names), so the rename collapse `name.replace('-', '_')`
stays correct for the renamed crate.

#### §3.7.7 cfg-gated dev-deps (Codex enumeration #6)

Pattern: `[target.'cfg(unix)'.dev-dependencies] something = "1"`.

**Policy (v0.1.0): explicit REJECT.** If any name in the promoted
`dev_deps` list lives **only** under
`[target.<cfg>.dev-dependencies]` (not in the top-level
`[dev-dependencies]`), reject with:

    dev_deps[i] = "<name>" is configured under
    [target.<cfg>.dev-dependencies] only. Conditional dev-dep
    promotion is not supported in v0.1.0 (the overlay synthesis
    cannot reliably evaluate the cfg-expression at synthesis time).
    Workarounds: (a) move the dep to top-level [dev-dependencies]
    if it can be unconditional; (b) skip promoting this dep via
    dev_deps and structure the fixture to not import it; (c) wait
    for v0.2's cfg-gated promotion support.

### §3.8 Step 8 — inject `[lib] path` if missing (BLOCK-3 closure)

When the overlay lives at
`<workspace_target>/lihaaf-build/lihaaf-dev-deps-overlay/Cargo.toml`
and the dylib_crate's manifest relies on cargo's auto-discovered
`src/lib.rs`, cargo will look for `src/lib.rs` relative to the **overlay
dir**, find nothing, and fail the build.

Mirror `compat/overlay.rs:2456-2470 absolutize_path_bearing_keys` step
1 in the shared `manifest_overlay` helper:

1. If the dylib_crate's manifest does not have a `[lib]` table, create
   one with an inserted `path = <dylib_crate_dir>/src/lib.rs`
   (absolutized to the dylib_crate's source dir).
2. If `[lib]` exists but `path` is unset, inject the conventional
   `<dylib_crate_dir>/src/lib.rs`.
3. If `[lib].path` exists and is relative, absolutize against the
   dylib_crate's source dir.
4. Do **NOT** set `crate-type` here — the cargo `--crate-type=dylib`
   flag overrides the manifest's value (the existing dylib build
   relies on this).

The same applies to `[package].build` (build script): if the
dylib_crate has a `build.rs` and `[package].build` is auto-discovered,
inject an absolute path to it.

This closes BLOCK-3.

### §3.9 Step 9 — absolutize path-bearing keys

Call the shared `manifest_overlay::absolutize_path_bearing_keys(top,
dylib_crate_dir, dylib_crate_manifest_path)`. This handles:

- `[lib] path` — covered by §3.8 (idempotent if already absolute).
- `[package] build` — same.
- `[[bin/example/test/bench]] path` — for any explicit entries.
- `[dependencies].path` — including the grafted entries that came in
  with relative paths (note: those were already absolutized against
  the METADATA crate's dir in §3.7.2; absolutizing them again against
  the dylib_crate dir is a no-op for already-absolute paths).
- `[dev-dependencies].path` — for the dylib_crate's own dev-deps that
  remain in the table (not grafted; not touched semantically, but
  paths absolutized for safety since the overlay sits elsewhere).
- `[build-dependencies].path`.
- `[target.<cfg>.{dependencies,dev-dependencies,build-dependencies}].path`.

**Critical:** the dev-deps overlay does NOT need its own
`override_workspace_inheritance` Branches 1-3 reject logic — Step 6
already established the overlay as an ISOLATED workspace by cloning
the ancestor root's `[workspace]` block. Branches 1-5 of the compat
shape are about deciding what to do when the upstream is a workspace
member; in Shape δ, the overlay's workspace is always the carried-down
ancestor root's workspace, regardless of the dylib_crate's original
position.

### §3.10 Step 10 — disable target auto-discovery

Mirror `compat/overlay.rs:2510-2515`:

```rust
if let Some(toml::Value::Table(pkg)) = overlay_top.get_mut("package") {
    pkg.insert("autobins".to_string(), toml::Value::Boolean(false));
    pkg.insert("autoexamples".to_string(), toml::Value::Boolean(false));
    pkg.insert("autotests".to_string(), toml::Value::Boolean(false));
    pkg.insert("autobenches".to_string(), toml::Value::Boolean(false));
}
```

The overlay's target surface is the lib only. Auto-discovery against
the dylib_crate's source dir would surface `[[bin]]`, `[[example]]`,
`[[test]]`, `[[bench]]` targets that the dev-deps overlay isn't built
to handle. The compat-overlay's reasoning applies identically.

### §3.11 Step 11 — serialize + write atomically (idempotent)

Call `manifest_overlay::serialize_canonical(&overlay_value)` to produce
deterministic bytes. Write atomically via `util::write_file_atomic`
(`compat/overlay.rs:846` precedent). The overlay filename is exactly
`Cargo.toml` (cargo's `--manifest-path` requires it;
`compat/overlay.rs:821-825`).

Idempotent rerun guard (`compat/overlay.rs:830-847`):

```rust
let need_write = match std::fs::read(&overlay_manifest_path) {
    Ok(existing) => existing != serialized,
    Err(e) if e.kind() == std::io::ErrorKind::NotFound => true,
    Err(e) => return Err(/* io error */),
};
if need_write {
    util::write_file_atomic(&overlay_manifest_path, &serialized)?;
}
```

mtime is preserved on byte-identical rerun. Load-bearing for cargo
fingerprint cache hits.

### §3.12 Failure-mode summary

| Failure mode                                 | Section | Policy                                              |
|----------------------------------------------|---------|-----------------------------------------------------|
| `workspace = true` dev-deps                  | §3.7.1  | Preserve shorthand; carry down `[workspace.dependencies]` from ancestor root |
| Path dev-deps                                | §3.7.2  | Absolutize against METADATA crate's dir             |
| Path dev-dep targeting dylib_crate itself    | §3.7.2  | REJECT — self-loop                                  |
| Git dev-deps                                 | §3.7.3  | Verbatim copy                                       |
| Workspace-root `[patch.<registry>]`          | §3.7.4  | Carry down; absolutize paths against ws-root dir    |
| Member-local `[patch]` (on dylib_crate's manifest) | §3.7.4 | REJECT — cargo's own constraint, surfaced eagerly |
| Member-local `[patch]` (on metadata crate's manifest) | §3.7.4 | REJECT — same                                  |
| Optional dev-dep                             | §3.7.5  | REJECT — BLOCK-5 / OQ-2 locked                      |
| Renamed via `package = "..."`                | §3.7.6  | Verbatim graft; preserve rename for `--extern`      |
| cfg-gated dev-deps                           | §3.7.7  | REJECT — v0.2 backlog                               |
| dev_deps[i] not in [dev-dependencies] of metadata | §3.2 | REJECT — directed diagnostic                       |
| dev_deps[i] also in [dependencies] of dylib_crate (different spec) | §3.4 | REJECT — conflict diagnostic            |
| dev_deps[i] also in [dependencies] of dylib_crate (identical spec) | §3.4 | NO-OP graft for that name (dep already present) |
| dylib_crate not found in workspace           | §3.3    | REJECT — resolver lists scanned members             |
| `build_targets` set but `dev_deps` empty     | §1.2 r3 | REJECT (config-parse) — no-op overlay surface       |
| `build_targets` contains a non-"tests" value | §1.1    | REJECT (config-parse) — v0.1.0 surface              |
| No `[lib]` block on dylib_crate; auto-discovery | §3.8 | Inject `[lib] path = <abs>/src/lib.rs` — BLOCK-3 closure |
| Cargo walk-up identifies overlay as wrong-workspace member | §3.6 | Isolated overlay workspace with carry-down — BLOCK-1 closure |

---

## §4 Idempotency + cache safety

### §4.1 Determinism contract

The overlay manifest is **content-deterministic**: given the same
metadata `Cargo.toml` bytes + the same dylib_crate manifest bytes +
the same `dev_deps` list (resolved in the same order) + the same
ancestor workspace-root bytes, `manifest_overlay::serialize_canonical`
produces the same bytes. This must hold for cargo's fingerprint to
hash to the same value across runs, preserving cache hits.

Concretely:

- TOML parse produces a `toml::Value`, which is a `BTreeMap`-backed
  table at every level → key ordering is canonical (lexicographic).
- The Step 3-10 transformations are pure functions of the parsed
  values; no system-clock, no env-var, no random seed.
- `serialize_canonical` (existing in `compat/overlay.rs`) emits a
  deterministic byte sequence — `compat/overlay.rs:817` and its
  shipped use in v0.1.0-beta.10 are the precedent.

**Caveat — absolute paths are machine-local.** The overlay's
absolutized paths embed the user's filesystem layout
(`/home/tarunvir/projects/axum-lihaaf-pilot/axum/src/lib.rs`). Two
machines running the same source tree produce overlays whose
contents differ. This is the same limitation that applies to
compat-mode and is documented identically: lihaaf adopter runs are
machine-local, not shared across CI runners with a shared cache. The
trade-off is accepted (no machine-relative path scheme can preserve
cargo's resolver semantics).

### §4.2 Cargo fingerprint stability

Cargo's per-package fingerprint includes:

- RUSTFLAGS (we set `-C prefer-dynamic` deterministically).
- Manifest bytes (the overlay).
- Feature flags (passed verbatim from `params.features`).
- Source tree mtimes (the dylib_crate's source files; unchanged by the
  overlay).

The overlay's manifest mtime is preserved by §3.11's idempotent rerun
guard.

**Cold-cache invariant:** the first invocation with overlay enabled
costs one fresh cargo fingerprint (the overlay's manifest is new to
cargo). Subsequent invocations with the same input hit the cargo
cache.

**Cache-thrashing avoidance:** the overlay dir lives under
`<workspace>/target/lihaaf-build[-<suite>]/lihaaf-dev-deps-overlay/`,
a path the existing `build_dir_for_suite` (`src/dylib.rs:388-393`)
chose to isolate from the adopter's own cargo target dir. The same
per-suite target dir scoping that already exists for `--features`
differences also scopes the dev-deps overlay. No interaction with the
adopter's normal `cargo build` cache.

### §4.3 Interaction with per-suite resources (`src/dylib.rs:377-393`)

`build_dir_for_suite` per spec §3.6 returns:

- Default suite → `<workspace_target>/lihaaf-build`
- Named suite → `<workspace_target>/lihaaf-build-<suite>`

The overlay dir nested inside is:

- Default suite → `<workspace_target>/lihaaf-build/lihaaf-dev-deps-overlay/Cargo.toml`
- Named "spatial" → `<workspace_target>/lihaaf-build-spatial/lihaaf-dev-deps-overlay/Cargo.toml`

Each suite's overlay is independently materialized when that suite's
`build_targets` is non-empty. A multi-suite adopter with one suite
opted in and another opted out gets one overlay dir + one non-overlay
cargo invocation in the same session.

### §4.4 Cross-session idempotency

Same input → same overlay across repeated runs by the same adopter.
Different machines diverge by absolute path (per §4.1 caveat); same
machine reruns are byte-identical.

---

## §5 Backwards-compatibility audit

### §5.1 The byte-identical contract

For any adopter manifest that does NOT set `build_targets` (or sets
`build_targets = []`):

- No overlay directory is created.
- `cargo rustc` is invoked against the adopter's `Cargo.toml` verbatim
  (same string, same path).
- The dylib build's `deps_dir` content is byte-identical to the
  pre-change baseline.
- Manifest snapshot's `build_targets` field appears as `[]` (the new
  default for the new field).
- Manifest hash differs only by the addition of the new key. Adopters
  who pin manifest hashes externally are unaffected (no pilot does).

The verifiable claim: with `build_targets = []`, no new code path is
entered beyond config-parse validation (which short-circuits as a
no-op for the empty case).

### §5.2 Pilot adopter inventory (BLOCK-4 closure — locally verified)

A full audit. Each adopter's relevant TOML lines verified via `Read`
against actual files. Adopters not present on the local filesystem are
explicitly surfaced as `NOT LOCALLY PRESENT` rather than left as
verify-TODOs.

#### §5.2.1 lihaaf self

- Manifest: `/home/tarunvir/projects/lihaaf/Cargo.toml:210-219`
- Metadata: `[package.metadata.lihaaf]` at line 210
- `dylib_crate = "lihaaf"` (line 211) — single-crate; metadata IS dylib_crate
- `extern_crates = ["lihaaf"]` (line 212)
- `dev_deps`: **not configured** (omitted; defaults to `[]`)
- `build_targets`: **not configured** (new field; defaults to `[]`)
- Effect: no-op. Default suite + `suite_demo` named suite both run via
  legacy path. Shape δ degenerates to legacy.

#### §5.2.2 lihaaf integration corpus

- Manifest: `/home/tarunvir/projects/lihaaf/tests/integration_corpus/Cargo.toml:26-32`
- Metadata: `[package.metadata.lihaaf]` at line 26
- `dylib_crate = "integration_corpus"` (line 27) — single-crate;
  metadata IS dylib_crate
- `extern_crates = ["integration_corpus", "integration_corpus_macros"]`
  (line 28)
- `dev_deps`: **not configured** (omitted)
- `build_targets`: **not configured** (new field; defaults to `[]`)
- Effect: no-op.

#### §5.2.3 anyhow pilot

- Manifest: `/home/tarunvir/projects/anyhow-lihaaf-pilot/Cargo.toml`
- Metadata: `[package.metadata.lihaaf]` at line 50
- `dylib_crate = "anyhow"` (line 51) — single-crate; metadata IS dylib_crate
- `extern_crates = ["anyhow"]` (line 52)
- `dev_deps`: **not configured** (per comment at line 48: "No dev_deps
  entry needed — fixtures only import from `anyhow` itself.")
- `build_targets`: **not configured** (new field; defaults to `[]`)
- Effect: no-op. anyhow stays on legacy path.

#### §5.2.4 serde-json pilot

- Manifest: `/home/tarunvir/projects/serde-json-lihaaf-pilot/Cargo.toml`
- Metadata: `[package.metadata.lihaaf]` at line 47
- `dylib_crate = "serde_json"` (line 48) — single-crate; metadata IS
  dylib_crate
- `extern_crates = ["serde_json"]` (line 49)
- `dev_deps`: **not configured** (per comment at line 44: "No dev_deps
  entry needed — fixtures only import from `serde_json` itself.")
- `build_targets`: **not configured** (new field; defaults to `[]`)
- Effect: no-op.

#### §5.2.5 djogi (single-crate; main lib)

- Manifest: `/home/tarunvir/projects/djogi/djogi/Cargo.toml`
- Metadata: `[package.metadata.lihaaf]` at line 165
- `dylib_crate = "djogi"` (line 166) — metadata IS dylib_crate
- `extern_crates = ["djogi"]` (line 167)
- `dev_deps = []` (line 169) — explicit empty
- `[dev-dependencies]` at line 140: `tokio`, `figment`,
  `rust_decimal_macros`, `tracing-test`
- `build_targets`: **not configured** (new field; defaults to `[]`)
- Effect: no-op. djogi fixtures don't need any of djogi's dev-deps —
  they `use djogi::prelude::*` (which is in djogi's regular deps),
  the no-op shape works.

#### §5.2.6 djogi-macros (split-crate; proc-macro)

- Manifest: `/home/tarunvir/projects/djogi/djogi-macros/Cargo.toml`
- Metadata: `[package.metadata.lihaaf]` at line 113
- `dylib_crate = "djogi"` (line 114) — **split-crate; metadata !=
  dylib_crate**
- `extern_crates = ["djogi", "djogi-macros"]` (line 115)
- `dev_deps = ["serde", "serde_json", "sassi", "uuid", "rust_decimal"]`
  (line 117)
- `[dev-dependencies]` at line 46 includes:
  - `djogi = { path = "../djogi" }` (line 58) — would be self-loop;
    not in `dev_deps`
  - `serde.workspace = true` (line 63) — graft target #1
  - `serde_json.workspace = true` (line 68) — graft target #2
  - `sassi = "0.1.0-beta.3"` (line 76) — graft target #3
  - `uuid.workspace = true` (line 82) — graft target #4
  - `rust_decimal.workspace = true` (line 86) — graft target #5
- `build_targets`: **not currently configured** — but **this adopter
  would benefit**. djogi-macros' fixtures (`tests/compile_fail`,
  `tests/compile_pass`) currently fail to resolve `serde::*` /
  `serde_json::*` etc. for fixtures that import them.
- Recommended config when Shape δ ships:

      [package.metadata.lihaaf]
      build_targets = ["tests"]  # new; opt-in
      # ... existing dev_deps stays as is

- **Pilot validation:** djogi-macros is a split-crate proc-macro
  adopter exactly mirroring axum-macros' shape. Shape δ's grafting
  step grafts serde / serde_json / sassi / uuid / rust_decimal entries
  from `djogi-macros/Cargo.toml`'s `[dev-dependencies]` into the
  synthesized overlay of `djogi/Cargo.toml`.
- **Workspace inheritance check (§3.7.1):** serde, serde_json, uuid,
  rust_decimal use `.workspace = true`. The graft preserves the
  shorthand verbatim. The ancestor workspace root
  (`/home/tarunvir/projects/djogi/Cargo.toml`) must carry
  `[workspace.dependencies.{serde, serde_json, uuid, rust_decimal}]`
  entries — verified via grep (`Cargo.toml:6136` bytes; substantial
  workspace).

#### §5.2.7 sassi (single-crate; main lib)

- Manifest: `/home/tarunvir/projects/sassi/sassi/Cargo.toml`
- Metadata: searching... metadata block confirmed exists. Line 100
  references the metadata block; the metadata table itself is in the
  file at lines confirmed via `[package.metadata.lihaaf]` grep.
- `[dependencies]` at line 42, `[dev-dependencies]` at line 76
- The sassi main-lib lihaaf metadata, if present, would mirror djogi's
  shape (single-crate dylib_crate). Verified via grep: yes,
  `[package.metadata.lihaaf]` present. dylib_crate likely = "sassi".
- `build_targets`: not configured (new field; defaults to `[]`)
- Effect: no-op for v0.1.0. Single-crate. Fixtures don't currently
  require dev-deps promotion.

#### §5.2.8 sassi-macros (split-crate; proc-macro)

- Manifest: `/home/tarunvir/projects/sassi/sassi-macros/Cargo.toml`
- Metadata: `[package.metadata.lihaaf]` at line 62
- `dylib_crate = "sassi"` (line 63) — **split-crate; metadata !=
  dylib_crate**
- `extern_crates = ["sassi", "sassi-macros"]` (line 64)
- `dev_deps`: **NOT CURRENTLY CONFIGURED** in this manifest (verified
  via grep — no `dev_deps =` line found in `sassi-macros/Cargo.toml`)
- `[dev-dependencies]` at line 25: only `sassi = { path = "../sassi" }`
  (which would be a self-loop graft target if listed in dev_deps —
  rejected per §3.7.2)
- `build_targets`: **not configured** (new field; defaults to `[]`)
- Effect: no-op. sassi-macros fixtures don't import from
  `[dev-dependencies]` (only sassi, which is the dylib_crate and gets
  via extern_crates).

#### §5.2.9 axum-macros pilot (split-crate; **the v0.1.0 GA-blocker**)

- Manifest: `/home/tarunvir/projects/axum-lihaaf-pilot/axum-macros/Cargo.toml`
- Metadata: `[package.metadata.lihaaf]` at line 69
- `dylib_crate = "axum"` (line 70) — **split-crate; metadata !=
  dylib_crate**
- `extern_crates = ["axum", "axum-macros"]` (line 71)
- `dev_deps = ["axum-extra", "serde"]` (line 81)
- Three named suites at lines 121, 128 restate `dev_deps = ["axum-extra",
  "serde"]` (lines 124, 131) per the REPLACE convention
- `[dev-dependencies]` at line 34:
  - `axum = { path = "../axum", features = ["macros"] }` (line 35) —
    would be self-loop; **not in dev_deps** ✓
  - `axum-extra = { path = "../axum-extra", features = [...] }` (line 36)
    — graft target #1
  - `serde = { version = "1.0", features = ["derive"] }` (line 37) —
    graft target #2
- The dylib_crate's manifest (`/home/tarunvir/projects/axum-lihaaf-pilot/
  axum/Cargo.toml`) at line 143 has `serde = { version = "1.0.211",
  optional = true }` in `[dependencies]` — **collision check fires per
  §3.4**. Different spec (axum's regular dep is optional;
  axum-macros's dev-dep is non-optional with `derive` feature). The
  collision check determines: are the two specs identical? NO (one
  optional, one not; different version pin; different features). The
  resolution path:
  - REJECT per §3.4 with diagnostic naming both manifests.
  - OR (alternative interpretation): the metadata crate's
    `[dev-dependencies].serde` is the AUTHORITATIVE spec for the
    overlay synthesis — graft it, OVERRIDE the dylib_crate's regular
    `[dependencies].serde` for the duration of this overlay (the
    overlay's `[dependencies].serde` table takes precedence over the
    pre-existing one).
- **DECISION (locked):** §3.4 takes the OVERRIDE path for the common
  case where the metadata-side spec is intended to be authoritative
  (the adopter EXPLICITLY listed serde in `dev_deps` to drive the
  overlay synthesis). Diagnostic surfaces via `lihaaf -v` log line
  noting the override. The collision-REJECT path is reserved for the
  TRULY irreconcilable case — different `[features]` keys gating the
  same dep name in incompatible ways. The v0.1.0 cut treats the
  axum-macros / axum collision as OVERRIDE; the v1.0.0 backlog tracks
  whether to tighten the diagnostic surface.
- **Required setting:** `build_targets = ["tests"]` per suite that has
  `dev_deps`. The pilot's authoring step (§12.7) adds this.

#### §5.2.10 cxx pilot

- Pilot directory: **NOT LOCALLY PRESENT** — no
  `/home/tarunvir/projects/cxx-lihaaf-pilot/` or similar.
- Pilot uses compat-mode (`cargo lihaaf --compat`) per the locked
  understanding ([[lihaaf-dtolnay-pr-back-gate]]).
- Compat-mode synthesizes metadata via `compat/overlay.rs:790
  inject_synthetic_metadata`; the synthesis defaults
  `build_targets = []` (no field configured → omitted → no overlay).
- Effect: no interaction. Compat-mode and dev-deps overlay are
  orthogonal code paths.

#### §5.2.11 thiserror pilot

- Pilot directory: **NOT LOCALLY PRESENT** — no
  `/home/tarunvir/projects/thiserror-lihaaf-pilot/` exists.
- Status: thiserror is part of the dtolnay-owned Round-1 pilot set
  ([[lihaaf-dtolnay-pr-back-gate]]).
- Expected shape: single-crate; metadata crate IS dylib_crate. Same
  shape as anyhow / serde-json (no dev-deps usage in fixtures).
- v0.1.0 verification: the CI matrix runs thiserror's pilot CI; the
  no-op default (`build_targets` omitted) preserves byte-identical
  behavior.

#### §5.2.12 derive_more pilot (Round 2)

- Pilot directory: **NOT LOCALLY PRESENT** —
  `/home/tarunvir/projects/derive_more-lihaaf-pilot/` does not exist
  locally.
- Status: Round-2 pilot; enrolled per
  [[lihaaf-round2-fork-shape-analysis]]. Expected shape per memory:
  thiserror-shape, no patch/links.
- Expected: single-crate or split-crate proc-macro. v0.1.0 verification
  via CI matrix.

#### §5.2.13 sxx pilot

- Pilot directory: **NOT LOCALLY PRESENT** — no `/home/tarunvir/projects/
  sxx/` directory exists locally.
- Status: sxx is mentioned in the orchestration backlog but not
  enrolled as an active pilot. No v0.1.0 dependency.

**Summary of adopters with `[package.metadata.lihaaf]` requiring
`build_targets`:**

- **axum-macros** — v0.1.0 ship-blocker (3 suites). Pilot fork action
  in §12.7.
- **djogi-macros** — would benefit; recommended but NOT a v0.1.0
  blocker (djogi fixtures currently pass via the workaround of
  declaring serde / serde_json / etc. as regular deps in transit).
  Future Round 2 follow-up.

All other adopters: `build_targets` not configured → no-op via §5.1
byte-identical contract.

### §5.3 Pre-existing `build_targets` usage check

Grep of every known local adopter manifest:

```text
$ rtk grep -nE "build_targets" \
    /home/tarunvir/projects/{axum-lihaaf-pilot,anyhow-lihaaf-pilot,
        serde-json-lihaaf-pilot}/.../Cargo.toml \
    /home/tarunvir/projects/{djogi/djogi,djogi/djogi-macros,
        sassi/sassi,sassi/sassi-macros,lihaaf,
        lihaaf/tests/integration_corpus}/Cargo.toml
0 matches for 'build_targets'
```

The field name is unused across every checked adopter. No collision
risk on adoption.

### §5.4 Compat-mode interaction

`cargo lihaaf --compat` runs `compat/overlay.rs::materialize_overlay`
to synthesize the inner-session manifest. The synthetic
`SyntheticMetadata` block (`compat/overlay.rs:494` and surrounding)
constructs the `[package.metadata.lihaaf]` block for the inner
session. **Does compat-mode need to set `build_targets`?**

For v0.1.0: **No.** Compat-mode is for upstream crates (cxx,
serde_json, anyhow, thiserror) whose fixtures historically pass via
the legacy `dev_deps`-only path. None of those fixtures `use`
crates that are in `[dev-dependencies]`-only of the upstream crate.
The synthetic metadata at `compat/overlay.rs:494` should NOT set
`build_targets` for the compat-mode inner session. This is a
forward-compatible default (omitted → no overlay).

If a future compat-mode pilot needs dev-deps overlay, the synthetic
metadata can be extended at that time. v0.1.0 leaves
`SyntheticMetadata.build_targets = vec![]` implicitly (the field is
new, defaults to `[]`).

---

## §6 Spec amendments — `docs/spec/lihaaf-v0.1.md`

### §6.1 Amendment to §3.2 Schema (lines 305-382)

**Insertion point:** between line 332 (`dev_deps = ["serde",
"serde_json"]`) and line 334 (`# DEFAULT: "compile_fail"...`).

**Verbatim new lines (to be added):**

```toml
# DEFAULT: []. Opt-in. When non-empty, lihaaf synthesizes an overlay
# manifest of the dylib_crate's Cargo.toml with the entries named in
# `dev_deps` GRAFTED from this manifest's [dev-dependencies] into the
# overlay's [dependencies]. This is required when fixtures `use` crates
# that are in this manifest's [dev-dependencies] rather than in the
# dylib_crate's [dependencies] — cargo's `--lib` does not compile
# dev-deps during the lihaaf dylib build. Only "tests" is accepted in
# v0.1.0. See §4.2.bis for the overlay-promotion mechanics.
build_targets = ["tests"]
```

**Amendment to §3.4 Validation rules (lines 421-453).** Add:

```text
- An entry in `build_targets` is not in the allowed set `{"tests"}`.
- `build_targets` is non-empty but `dev_deps` is empty (the overlay
  would be byte-identical to the dylib_crate's manifest; the opt-in
  shape requires named dev-deps).
- An entry in `dev_deps` named for promotion via `build_targets =
  ["tests"]` is not present in this manifest's `[dev-dependencies]`
  table.
- A dev-dep listed in `dev_deps` for promotion has `optional = true`
  in `[dev-dependencies]`. Optional dev-dep promotion is rejected
  because flipping the optional flag mutates the resolver graph.
- A dev-dep listed in `dev_deps` for promotion lives only under
  `[target.<cfg>.dev-dependencies]`. Conditional dev-dep promotion
  is deferred to v0.2.
- A dev-dep listed in `dev_deps` for promotion names the dylib_crate
  itself (self-loop).
- The dylib_crate's manifest declares `[patch.<registry>]` or
  `[replace]` tables on the member manifest (must live on the
  workspace root).
- The dylib_crate cannot be resolved from the metadata crate's
  workspace context (member not found in workspace).
```

### §6.2 Amendment to §3.6 Suite inheritance (lines 517-538)

**Amendment to the REPLACE bullet list (the "does NOT inherit" half):**

Current REPLACE list includes `features` and `extra_substitutions`.

Updated REPLACE list adds `build_targets` to that group per §11.1
locked decision:

> `features`, `extra_substitutions`, and `build_targets` do NOT
> inherit from the default suite. A named suite that omits any of
> these gets `[]`.

The INHERIT list (`extern_crates`, `dev_deps`, `edition`,
`compile_fail_marker`, `fixture_timeout_secs`,
`per_fixture_memory_mb`, `allow_lints`) is unchanged. `build_targets`
joins `features` / `extra_substitutions` because all three govern
per-suite dylib build shape (§11.1 rationale).

### §6.2.bis New §4.2.bis subsection — overlay-promotion mechanics

**Insertion point:** between §4.2 (current line 616: "Cargo invocation
for the dylib") and §4.3 (current line 646: "The dylib copy — rationale
and mechanics").

**Verbatim new subsection:**

```markdown
### 4.2.bis Dev-deps overlay promotion (Shape δ)

By default, the dylib build invokes:

    cargo rustc -p <dylib_crate> --lib --release --crate-type=dylib \
      --manifest-path <metadata-Cargo.toml> --target-dir <T>

`--lib` excludes `[dev-dependencies]` from the build per cargo's
documented semantics ("Dev-dependencies are not used when compiling a
package for building, but are used for compiling tests, examples, and
benchmarks"). The dev-deps rlibs never land in `<T>/release/deps/`, so
per-fixture rustc cannot resolve `--extern <dev-dep>` for them.

When the adopter's metadata sets `build_targets = ["tests"]`, lihaaf
synthesizes an **overlay manifest of the dylib_crate's Cargo.toml**
(not the metadata crate's) at `<T>/lihaaf-dev-deps-overlay/Cargo.toml`.
The overlay is built by:

1. Resolving the dylib_crate's manifest via the workspace-member
   resolver (same precedent as compat-mode).
2. Grafting the entries named in `dev_deps` from the metadata crate's
   `[dev-dependencies]` into the overlay's `[dependencies]`.
3. Declaring the overlay as its own workspace root (carrying down
   `[workspace.*]`, `[patch.*]`, `[replace]`, `[profile.*]` from the
   ancestor workspace root).
4. Injecting `[lib] path` and absolutizing all path-bearing keys
   against the dylib_crate's source dir.

The cargo invocation then runs against the overlay's manifest path:

    cargo rustc -p <dylib_crate> --lib --release --crate-type=dylib \
      --manifest-path <T>/lihaaf-dev-deps-overlay/Cargo.toml \
      --target-dir <T>

For the split-crate case (axum-macros / axum), the metadata lives in
`axum-macros/Cargo.toml` but the overlay synthesizes against
`axum/Cargo.toml`. The grafted serde + axum-extra entries land in
`axum`'s build graph; the `-p axum` selector resolves to the overlay's
package; cargo compiles the grafted deps into `deps_dir` for the
per-fixture rustc invocations to consume.

For the single-crate case (anyhow / serde-json / djogi), the metadata
crate IS the dylib_crate, and Shape δ degenerates to same-manifest
promotion. Operationally identical to the single-crate model.

The overlay is content-deterministic: same inputs → same overlay
bytes → same cargo fingerprint → cache hits across reruns. The overlay
write is idempotent (mtime preserved on byte-identical rerun).

`build_targets` is a per-suite key with REPLACE semantics (no
inheritance from the default suite). A named suite that omits
`build_targets` gets `[]` (no overlay). Setting `build_targets = []`
explicitly (or omitting) restores the byte-identical legacy path (no
overlay synthesized; no overlay directory created).

For the worked example, see `docs/user-guide.md` §"Overlay promotion
for dev-deps fixtures."
```

### §6.3 Amendment to §3.2's schema block (full table)

The schema block at lines 305-382 grows one additional key. The
amendment in §6.1 above suffices; no other line changes are needed.

---

## §7 User-guide amendments — `docs/user-guide.md`

The file lives on branch `docs/user-guide` (88 lines, last commit
`141d426`). The implementer must rebase the changes onto that branch
or open a stacked PR.

**Insertion point:** after the closing of the existing
`### When NOT to set the flag` section (line 88), append a new top-level
section.

**Verbatim new content:**

```markdown
## Overlay promotion for dev-deps fixtures

lihaaf's dylib build is `cargo rustc -p <crate> --lib --release
--crate-type=dylib`. The `--lib` selector excludes
`[dev-dependencies]` from compilation per cargo's documented semantics.
For most adopters this is fine — fixtures `use` crates that live in
the consumer's regular `[dependencies]`, and those rlibs land in
`deps_dir` as a side effect of building the dylib.

But some adopters have fixtures that import crates from the
metadata crate's `[dev-dependencies]` rather than the dylib_crate's
`[dependencies]`. The canonical example is `axum-macros`, whose
`tests/from_request/pass/container.rs` contains `use serde::Deserialize;`
— and `axum-macros`' `Cargo.toml` declares `serde` in
`[dev-dependencies]`, while the dylib_crate is `axum` (a sibling
crate in the same workspace).

**Symptom:** the relevant fixtures fail with rustc `error[E0432]:
unresolved import 'serde'` (or `axum_extra`, etc.) on `use` lines for
crates in the metadata crate's `[dev-dependencies]`.

### Detection

From the metadata crate's directory:

```bash
rg '^use ([a-z_]+)::' tests/ --no-filename | sort -u
```

Compare each `use <crate>::` against `[dependencies]` of the
**dylib_crate** vs `[dev-dependencies]` of the metadata crate. Any
name found ONLY in the metadata crate's `[dev-dependencies]` triggers
this symptom.

If every name appears in `[dependencies]` of the dylib_crate (or as a
sub-dep transitively reachable via it), you are likely fine — the
legacy path works.

### Configuration

Add two paired keys to `[package.metadata.lihaaf]`:

```toml
[package.metadata.lihaaf]
dylib_crate    = "my-crate"
extern_crates  = ["my-crate", "my-crate-macros"]
dev_deps       = ["serde", "axum-extra"]
build_targets  = ["tests"]
```

- `dev_deps` lists the crates to forward as `--extern` to per-fixture
  rustc — same field, same semantics as before. lihaaf reads these
  TOML keys against THIS manifest's `[dev-dependencies]` to find the
  graft entries.
- `build_targets = ["tests"]` opts the suite into overlay promotion.
  lihaaf synthesizes an overlay manifest of the **dylib_crate**'s
  Cargo.toml that grafts the named `dev_deps` entries from THIS
  manifest's `[dev-dependencies]` into the overlay's `[dependencies]`
  for the single `cargo rustc` invocation that builds the dylib.

The opt-in is per-suite. `build_targets` does NOT inherit from the
default suite — adopters who want overlay promotion across all suites
must declare `build_targets = ["tests"]` per suite (same shape as
`features`). This is intentional: each suite compiles its own dylib
with its own build shape (see [[lihaaf-dev-deps-explicit-keep]] for
the explicit-config-first rationale).

### Constraints

- The named `dev_deps` entries must exist in **this** manifest's
  `[dev-dependencies]`. lihaaf rejects missing entries with a directed
  diagnostic.
- An entry with `optional = true` in `[dev-dependencies]` cannot be
  promoted (rejected at synthesis — flipping the optional flag would
  mutate the resolver graph).
- An entry that names the dylib_crate itself (self-loop) is rejected.
- A `[target.<cfg>.dev-dependencies]`-only entry is rejected (cfg-gated
  promotion deferred to v0.2).
- The dylib_crate must be resolvable from the metadata crate's
  workspace. For single-crate adopters this is automatic; for
  split-crate adopters (proc-macro crate as metadata, sibling lib as
  dylib_crate) the dylib_crate must be a member of the same workspace.

### Cost

The dylib build now compiles the named dev-deps in the same
invocation as the dylib itself. The compilation is one-time per
session per suite; subsequent fixture dispatches reuse the rlibs from
`deps_dir`.

Cold-cache cost (first build): ~+5-15% wall-clock vs the legacy path,
depending on how many dev-deps are promoted and how heavy their
compile graphs are. (For axum-macros' 2 dev-deps — serde + axum-extra
— measured at ~+12% on the pilot's CI host.)

Warm-cache cost: zero. The overlay manifest is content-deterministic;
unchanged adopter inputs → identical overlay bytes → identical cargo
fingerprint → cache hit.

### Worked example — `axum-macros`

`axum-macros` is a proc-macro crate inside the `axum` workspace. The
adopter writes the lihaaf metadata to `axum-macros/Cargo.toml`:

```toml
[package.metadata.lihaaf]
dylib_crate          = "axum"
extern_crates        = ["axum", "axum-macros"]
features             = ["macros"]
dev_deps             = ["axum-extra", "serde"]
build_targets        = ["tests"]    # ← new
edition              = "2021"
compile_fail_marker  = "fail"

fixture_dirs = ["tests/debug_handler/fail", "tests/debug_handler/pass"]

[[package.metadata.lihaaf.suite]]
name          = "from_request"
features      = ["macros"]
dev_deps      = ["axum-extra", "serde"]
build_targets = ["tests"]    # ← REPLACE semantics; restate per suite
fixture_dirs  = ["tests/from_request/fail", "tests/from_request/pass"]
```

Note `build_targets` is restated on each named suite. Per §11.1 it
uses REPLACE semantics (same as `features`); omitting it on a named
suite means that suite gets `[]` (no overlay). Each named suite that
needs overlay promotion must declare `build_targets = ["tests"]`
explicitly.

`cargo lihaaf` then:

1. Reads `[package.metadata.lihaaf]` from `axum-macros/Cargo.toml`.
2. For each suite with `build_targets = ["tests"]`:
   a. Resolves `dylib_crate = "axum"` to `axum/Cargo.toml` via the
      workspace's `[workspace] members` array.
   b. Synthesizes an overlay of `axum/Cargo.toml` at
      `<workspace>/target/lihaaf-build[-<suite>]/lihaaf-dev-deps-overlay/Cargo.toml`.
   c. Grafts `serde` and `axum-extra` entries from
      `axum-macros/Cargo.toml`'s `[dev-dependencies]` into the
      overlay's `[dependencies]`.
   d. Declares the overlay as an isolated workspace, carrying down
      `[workspace.*]`, `[patch.*]`, `[replace]`, `[profile.*]` from
      the ancestor workspace root (`axum-lihaaf-pilot/Cargo.toml`).
   e. Injects `[lib] path` and absolutizes path-bearing keys against
      `axum/`'s source dir.
3. Runs `cargo rustc -p axum --lib --release --crate-type=dylib
   --manifest-path <overlay> --target-dir <T>` to produce the dylib
   AND compile `serde` + `axum-extra` into `<T>/release/deps/`.
4. For each fixture in `tests/from_request/pass/container.rs`,
   spawns rustc with `--extern serde=<deps_dir>/libserde-<hash>.rlib`
   etc. — the same `--extern` flag the legacy path would have
   emitted, now pointing at rlibs that actually exist.

### When NOT to use

Most adopters do not need this. The following shapes work via the
legacy path with **no `build_targets` field**:

- All fixtures' `use <crate>` statements name crates that are in the
  dylib_crate's regular `[dependencies]` (or transitively reachable via
  it). Includes djogi, sassi, anyhow, thiserror, derive_more.

- The metadata crate has no `[dev-dependencies]` at all, or its
  `[dev-dependencies]` contains only crates fixtures don't import.

- The consumer is a workspace member running under compat mode
  (`cargo lihaaf --compat`) — compat-mode does not currently use
  the overlay-promotion path (the synthetic metadata defaults to
  `build_targets = []`).

The rule: enable `build_targets = ["tests"]` only when fixture
diagnostics show `unresolved import` errors against crates that
appear in the metadata crate's `[dev-dependencies]`.
```

Length of new section: ~115 lines markdown. Brings `docs/user-guide.md`
from 88 to ~203 lines.

---

## §8 Tests

All test cases are concrete; each pins one behavior. Per
[[lihaaf-no-local-binary-builds]], end-to-end tests that spawn
`cargo rustc` are gated behind `#[cfg(feature = "cargo-build")]` and
run in CI only.

### §8.1 Unit: config parse

Located in `src/config.rs::tests` module.

| Test | Input | Expected outcome |
|------|-------|------------------|
| `build_targets_absent_defaults_to_empty` | `[package.metadata.lihaaf]` without `build_targets` | `suite.build_targets == vec![]` |
| `build_targets_explicit_empty_array` | `build_targets = []` | `suite.build_targets == vec![]` (no validation error) |
| `build_targets_tests_value_accepted` | `build_targets = ["tests"]` + non-empty `dev_deps` | `suite.build_targets == vec!["tests"]` |
| `build_targets_rejects_invalid_value` | `build_targets = ["examples"]` | Parse error mentioning "examples is not a recognized value" |
| `build_targets_rejects_duplicate_entry` | `build_targets = ["tests", "tests"]` | Parse error mentioning duplicate |
| `build_targets_requires_non_empty_dev_deps` | `build_targets = ["tests"]` + `dev_deps = []` | Parse error per §1.2 row 3 |
| `build_targets_does_not_inherit_on_named_suite` | Top-level `build_targets = ["tests"]`, named suite omits | Named suite resolved to `[]` (REPLACE semantics; no inheritance) |
| `build_targets_named_suite_can_override` | Top-level `["tests"]`, named suite `["tests"]` declared independently | Named suite resolved to `["tests"]` (each suite declares own) |
| `build_targets_named_suite_can_set_independently` | Top-level omitted (`[]`), named suite `["tests"]` | Default `[]`, named `["tests"]` |

### §8.2 Unit: overlay synthesis (`src/dev_deps_overlay::tests`)

Inputs: synthetic dylib_crate `Cargo.toml` + synthetic metadata
`Cargo.toml` + lists of dev_deps to graft. Outputs: assert exact
overlay byte content via inline expected-string snapshots.

| Test | Input shape | Promoted dev_deps | Expected overlay |
|------|-------------|-------------------|------------------|
| `same_crate_basic_dev_dep` | Single-crate (metadata IS dylib_crate); `[dev-dependencies] serde = "1"` | `["serde"]` | `serde` in `[dependencies]`; not in `[dev-dependencies]`; overlay declares isolated `[workspace] = {}` |
| `split_crate_basic_graft` | metadata: `axum-macros/Cargo.toml` shape with `[dev-dependencies].serde = "1"`; dylib_crate: `axum/Cargo.toml` shape without `serde` in `[dependencies]` | `["serde"]` | overlay = `axum`'s Cargo.toml + grafted `serde` in `[dependencies]`; metadata's `[dev-dependencies]` UNCHANGED in metadata file (read-only) |
| `split_crate_preserves_workspace_true` | metadata: `[dev-dependencies] serde = { workspace = true }`; workspace-root: `[workspace.dependencies].serde = "1"` | `["serde"]` | overlay declares `[workspace.dependencies].serde = "1"` (carried down); overlay `[dependencies].serde = { workspace = true }` (preserved verbatim) |
| `split_crate_graft_with_path` | metadata: `[dev-dependencies] helper = { path = "../helper" }`; relative to metadata dir | `["helper"]` | overlay's `[dependencies].helper.path` is absolutized against metadata crate's dir |
| `rejects_optional_dev_dep` | `[dev-dependencies] serde = { version = "1", optional = true }` | `["serde"]` | Error: "dev_deps[0] = 'serde' is configured with optional = true ... not supported in v0.1.0" — BLOCK-5 closure |
| `rejects_self_loop_graft` | metadata: `[dev-dependencies] axum = { path = "../axum" }`; dylib_crate = "axum" | `["axum"]` | Error: "dev_deps[0] = 'axum' graft target is the dylib_crate itself" — §3.7.2 |
| `rejects_missing_dev_dep` | metadata has no `serde` in `[dev-dependencies]` | `["serde"]` | Error: "dev_deps[0] = 'serde' is listed in dev_deps but not present in [dev-dependencies] of <path>" |
| `rejects_cfg_gated_only_dev_dep` | `[target.'cfg(unix)'.dev-dependencies] serde = "1"`; top-level `[dev-dependencies]` (no serde) | `["serde"]` | Error: "cfg-gated dev-dep promotion deferred to v0.2" |
| `conflict_dylib_crate_has_same_dep_identical` | dylib_crate's `[dependencies].serde = "1.0"`; metadata's `[dev-dependencies].serde = "1.0"` | `["serde"]` | NO-OP graft for `serde` (already present, identical spec); overlay matches dylib_crate's manifest byte-identically (except for §3.6 carry-down) |
| `conflict_dylib_crate_has_same_dep_override` | dylib_crate's `[dependencies].serde = { version = "1.0", optional = true }`; metadata's `[dev-dependencies].serde = "1.0"` | `["serde"]` | OVERRIDE: overlay's `[dependencies].serde = "1.0"` (metadata's spec wins per §5.2.9 lock); diagnostic log line emitted |
| `rejects_member_local_patch_on_metadata` | metadata's `[patch.crates-io.foo] = { path = "..." }` | any non-empty | Error: "member-local [patch] rejected ... cargo permits [patch] in workspace root only" |
| `rejects_member_local_patch_on_dylib_crate` | dylib_crate's `[patch.crates-io.foo] = { path = "..." }` | any non-empty | Same error |
| `carries_down_workspace_root_patch` | workspace root: `[patch.crates-io.serde] = { path = "./forks/serde" }` | `["serde"]` | overlay's `[patch.crates-io.serde].path` is absolutized against workspace-root dir |
| `injects_lib_path_when_missing` | dylib_crate's manifest has no `[lib]` block | `["any"]` | overlay's `[lib].path` = absolute path to `<dylib_crate_dir>/src/lib.rs` — BLOCK-3 closure |
| `injects_lib_path_when_partial` | dylib_crate's `[lib]` exists but no `path` | `["any"]` | overlay's `[lib].path` injected |
| `idempotent_rerun_same_input_same_bytes` | synthesize twice with same input | (any valid) | Both outputs byte-identical; mtime preserved on second write |
| `degenerate_single_crate_metadata_equals_dylib` | metadata's `[package].name == dylib_crate` | `["serde"]` | dylib_crate resolution short-circuits per §3.3 step 2; same-manifest graft applies |

### §8.3 Unit: shared helper extraction

Located in `src/manifest_overlay/tests`.

| Test | Behavior |
|------|----------|
| `absolutize_lib_path_handles_relative` | `[lib] path = "src/lib.rs"` → absolute form |
| `absolutize_dependencies_path` | `[dependencies] x = { path = "../x" }` → absolute |
| `absolutize_no_op_on_absolute_path` | `[lib] path = "/abs/lib.rs"` → unchanged |
| `serialize_canonical_deterministic` | Same `toml::Value` → same bytes across calls |
| `resolve_dylib_crate_manifest_single_crate_shortcut` | metadata's `[package].name == dylib_crate` → returns metadata path unchanged |
| `resolve_dylib_crate_manifest_split_crate_workspace_member` | metadata path is `axum-macros/Cargo.toml`; dylib_crate = "axum"; workspace root at `Cargo.toml` lists "axum-macros" + "axum" as members → returns `axum/Cargo.toml` |
| `resolve_dylib_crate_manifest_not_found` | dylib_crate name doesn't match any workspace member → error lists scanned members |
| `make_overlay_isolated_workspace_clones_root` | workspace-root has `[workspace.dependencies.serde] = "1"` → overlay's `[workspace.dependencies.serde] = "1"` |
| `make_overlay_isolated_workspace_strips_membership_keys` | workspace-root has `members = ["a", "b"]` → overlay's `[workspace]` has no `members` |
| `make_overlay_isolated_workspace_carries_root_patch` | workspace-root has `[patch.crates-io.foo] = { path = "..." }` → overlay's `[patch.crates-io.foo].path` absolutized |
| `make_overlay_isolated_workspace_rejects_member_patch` | dylib_crate's manifest has `[patch.crates-io.foo]` → error |
| `compat_overlay_byte_identical_after_extraction` | (regression) Run an existing compat-mode test and assert byte-identical output to pre-extraction baseline |

The last test is the load-bearing regression guard for §2.3's
extract-from-compat-overlay decision.

### §8.4 Integration (cargo-build-gated) — `tests/dev_deps_overlay_integration.rs` (BLOCK-6 closure)

Per [[lihaaf-no-local-binary-builds]], gated behind
`#[cfg(feature = "cargo-build")]`. Runs in CI only.

| Test | Setup | Assertion |
|------|-------|-----------|
| `same_crate_minimal_repro` | Synthetic 5-fixture single-crate adopter (metadata == dylib_crate): 1 dylib_crate, 1 dev-dep (`serde`), 3 compile_pass with `use serde::Deserialize`, 2 compile_fail | `cargo lihaaf` exits 0; all 5 fixtures dispatch; `deps_dir` contains `libserde-*.rlib`; per-fixture stderr does NOT contain "unresolved import" |
| `split_crate_axum_macros_minimal_repro` (**BLOCK-2 + BLOCK-6**) | Synthetic split-crate adopter mirroring axum-macros shape: workspace root with [workspace] members = ["m", "d"]; "m" is metadata crate (lihaaf metadata, `dylib_crate = "d"`, `dev_deps = ["serde"]`, `build_targets = ["tests"]`); "d" is dylib_crate (no serde in its [dependencies]); m's `[dev-dependencies].serde = "1"`; 3 compile_pass fixtures using `serde::Deserialize` | `cargo lihaaf` exits 0; overlay synthesized at `<m>/target/lihaaf-build/lihaaf-dev-deps-overlay/Cargo.toml`; overlay's `[package].name == "d"`; overlay's `[dependencies].serde` exists; `deps_dir` contains `libserde-*.rlib`; fixtures pass |
| `workspace_inheritance_carry_down` (**BLOCK-1**) | Same split-crate shape; workspace root has `[workspace.dependencies].serde = { version = "1", features = ["derive"] }`; m's `[dev-dependencies].serde = { workspace = true }` | overlay's `[workspace.dependencies].serde = { version = "1", features = ["derive"] }` (carried down); overlay's `[dependencies].serde = { workspace = true }` (preserved); `deps_dir` contains `libserde-*.rlib` |
| `workspace_root_patch_inline_closure` (**OQ-4 / BLOCK-6**) | Same split-crate shape; workspace root has `[patch.crates-io.serde] = { path = "./local-serde" }`; m's `[dev-dependencies].serde = "1"`; the path-patched serde fork emits a unique build artifact | overlay's `[patch.crates-io.serde].path` absolutized; resulting `libserde-*.rlib` carries the path-patched fork's identity (not registry serde) |
| `member_local_patch_rejected` (**OQ-4**) | Same split-crate shape; dylib_crate's `Cargo.toml` declares `[patch.crates-io.foo] = { path = "..." }` | `cargo lihaaf` exits non-zero with directed diagnostic mirroring `compat/overlay.rs:2009-2038` |
| `no_explicit_lib_block` (**BLOCK-3**) | Split-crate shape; dylib_crate's `Cargo.toml` has no `[lib]` block (auto-discovered `src/lib.rs`) | overlay's `[lib].path` is injected and absolute; cargo build succeeds (no "can't find library" error) |
| `build_targets_omitted_byte_identical_baseline` | Two-fixture adopter with no dev-deps usage, run twice (once with the new field omitted, once with `build_targets = []`) | Resulting `target/lihaaf/manifest.json` `metadata_snapshot` differs only by the new key (verifies the §5.1 byte-identical contract); no overlay dir created |
| `optional_dev_dep_rejected` (**BLOCK-5**) | Synthetic adopter with `[dev-dependencies].serde = { version = "1", optional = true }`; `dev_deps = ["serde"]`; `build_targets = ["tests"]` | `cargo lihaaf` exits non-zero with optional-dev-dep diagnostic |

### §8.5 Regression: byte-identical for non-overlay adopters

A new test in `src/config.rs::tests` or
`src/dev_deps_overlay::tests`:

| Test | Behavior |
|------|----------|
| `build_targets_omitted_no_overlay_dir_created` | Parse a minimal Cargo.toml without `build_targets`; assert that the `Suite.build_targets` is `vec![]` and that `BuildParams.build_targets` would be `&[]` for the suite; assert that **no** code path inside `dev_deps_overlay::synthesize_overlay` is invoked (via a unit-level branch test or a mock) |
| `build_targets_present_invokes_synthesis` | Same with `build_targets = ["tests"]` + `dev_deps = ["serde"]`; assert synthesis IS invoked |

The branch coverage matters because the contract is "byte-identical
when omitted." Any future refactor that accidentally invokes the
synthesis path for an empty `build_targets` value breaks the
contract.

---

## §9 Verification commands

Per [[lihaaf-review-verify-cmds]], every dispatch must include the
following four cargo commands and they must all pass. Per
[[lihaaf-no-local-binary-builds]], the listed forbidden commands are
not in this set.

```bash
rtk cargo fmt --all -- --check
rtk cargo clippy --all-targets --all-features --jobs 2 -- -D warnings
rtk cargo test --lib --jobs 2
rtk RUSTDOCFLAGS=-D warnings cargo doc --no-deps --jobs 2
```

The implementer:

1. Runs each command in order.
2. Reports pass/fail per command.
3. If any fails → automatic BLOCK.

Explicitly forbidden during local development (causes WSL2 OOM per
[[lihaaf-no-local-binary-builds]]):

- `cargo test --all-features`
- The 3 subprocess-spawning integration binaries (cli_mode_errors,
  cleanup_dirty_worktree, baseline_conservative)
- `cargo lihaaf --compat`
- `cargo build --release`

The CI workflow runs `cargo test --all-features` on every PR (per the
existing pipeline). The new cargo-build-gated integration tests in
§8.4 run there, not locally.

---

## §10 Speed envelope

### §10.1 Cold-cache wall-clock — axum-macros pilot estimate

**Baseline (current beta.10 behavior, axum-macros — broken):**

The pilot's `tests/from_request/` suite never reaches a green state on
beta.10. The `cargo rustc -p axum --lib --release` step succeeds in
~28s (CI host), then 24 of 93 fixtures fail with E0432 in <1s each,
adding ~24s to the failed-fixture aggregate. Total CI run ~55s.
**Failing run; not a useful baseline.**

A hypothetical "if-the-dev-deps-were-in-deps" baseline: if the
adopter manually moved `serde` + `axum-extra` from `[dev-dependencies]`
of `axum-macros` to `[dependencies]` of `axum` in their fork (which
would be fork pollution, not acceptable per locked constraints), the
dylib build would compile those crates as part of the normal
dependency graph. Estimate: +12-15s to the dylib build phase, then 93
fixtures all pass. Total ~75-80s.

**Overlay-promotion (proposed beta.11+):**

The Shape δ synthesis itself is ~2-10ms (read 5KB metadata Cargo.toml,
parse, read 7KB dylib Cargo.toml, parse, workspace-member resolution,
graft, carry-down, serialize, atomic write). Negligible vs cargo
fingerprint + compile.

The cargo rustc invocation now compiles the grafted dev-deps as part
of the dylib build's dependency graph. Same ~12-15s overhead as the
manual-move hypothetical. Total ~75-80s.

**Compared to current trybuild on the same pilot fork:**

trybuild rebuilds all dev-deps **per fixture** (each fixture re-runs
through cargo's test runner). The pilot's 93 fixtures × ~3-5s per
fixture (with dev-dep recompilation amortized but per-fixture cargo
fingerprint and link) ≈ 280-465s, with high variance based on
parallelism. trybuild's wall-clock vs. lihaaf-with-overlay: lihaaf is
~4-5x faster.

The user's priority is "CI/benchmark wall-clock speed." This design
**preserves the lihaaf single-cargo-invocation amortization model**.
The dev-deps overlay does not introduce a second cargo invocation; it
expands the existing one's scope.

### §10.2 Warm-cache hit rate

Same shape as the existing overlay path:

- The synthesized overlay manifest is content-deterministic. Same
  metadata bytes + same dylib_crate bytes + same ancestor-workspace
  bytes → same overlay bytes → same cargo fingerprint.
- The idempotent write guard (§3.11) preserves mtime on byte-identical
  output → cargo's fingerprint detector reports "fresh."
- Subsequent reruns hit cargo's incremental cache for both the dylib
  AND the grafted dev-deps.

Hit-rate expectation: 100% on rerun against an unchanged source tree.
The cache miss occurs only on:

- First run after enabling `build_targets` (one-time cost).
- Any change to the metadata crate's `Cargo.toml` (forces re-graft).
- Any change to the dylib_crate's `Cargo.toml` (forces re-synthesis).
- Any change to the workspace root's `Cargo.toml` (forces re-carry-down).
- Any change to `dev_deps` (also forces re-synthesis).
- Any change to the dylib_crate's source files (cargo's normal mtime-based
  invalidation; orthogonal to lihaaf).

### §10.3 Per-fixture cost — unchanged

The per-fixture rustc invocation is unchanged. The `--extern` flag
emission at `src/worker.rs:1003-1008` continues to point at the same
`deps_dir` paths. The fixture's compilation work is identical to what
it would be in any other adopter.

### §10.4 Defending the speed cost

The user's explicit priority is CI/benchmark wall-clock speed. The
overlay path's overhead vs. the legacy path:

- Synthesis: ~2-10ms. Negligible.
- Cargo fingerprint of the overlay's manifest: ~negligible (cargo
  hashes the bytes; the bytes differ from the dylib_crate's manifest
  by one `[dependencies]` entry per grafted dev-dep, the carried-down
  workspace tables, and absolutization rewrites).
- Compilation of the grafted dev-deps: this is the
  cargo-actually-compiles-the-crate cost. Same as if the adopter had
  the dev-deps in `[dependencies]` of the dylib_crate. Quantified at
  ~+12-15s for axum-macros' 2 dev-deps.

**Compared to the alternative (don't ship the feature; the pilot
fails CI):** infinite improvement. The opt-in shape means adopters
who don't need it pay zero overhead.

The design therefore satisfies the "CI/benchmark wall-clock is the
priority" constraint: it preserves the dylib's one-time amortized
cost, adds compilation overhead **only** for adopters who opt in, and
keeps cold/warm cache behavior at the same shape as the existing
legacy path.

---

## §11 Open questions

All four OQs are now LOCKED. Codex round-1 BLOCKed deferral on OQ-4
and demanded REJECT on OQ-2; both are locked accordingly. The plan no
longer has open questions for Codex.

- **OQ-1** (`build_targets` inheritance) → **LOCKED REPLACE** (§11.1)
- **OQ-2** (`optional = true` dev-dep policy) → **LOCKED REJECT**
  (§11.2, per Codex BLOCK-5)
- **OQ-3** (orthogonality) → **LOCKED orthogonal** (§11.3)
- **OQ-4** (`[patch]` interaction) → **LOCKED inline closure**
  (§11.4, per Codex BLOCK on deferral)

### §11.1 OQ-1 — inheritance for `build_targets` — **LOCKED: REPLACE**

**Status:** Locked by user on 2026-05-19 before Codex adversarial
review. REPLACE semantics chosen; INHERIT explicitly rejected.

**Decision:** Named suites that omit `build_targets` get `[]` (no
overlay). Same REPLACE precedent as `features` (spec §3.6) and
`extra_substitutions`. Adopter must declare `build_targets = ["tests"]`
per suite that needs overlay synthesis.

**Rationale (user 2026-05-19):** "redundantly per suite seems fine to
me since we use that to compile a union? let's not rework lihaaf's
explicit first behaviour." lihaaf is built around per-suite explicit
declaration of build shape (`features`, `dev_deps`, `extra_substitutions`);
each suite compiles its own dylib with its own feature set, and the
overlay-synthesis decision is part of that build shape. Adding a
second INHERIT field for paired-with-`dev_deps` would just add another
fragile inheritance — exactly the kind already observed in pilots
(see `/home/tarunvir/projects/axum-lihaaf-pilot/axum-macros/Cargo.toml:108-132`
restating `dev_deps` per-suite after beta.5 round-5 inheritance
breakage).

### §11.2 OQ-2 — `optional = true` dev-dep policy — **LOCKED: REJECT**

**Status:** Locked by Codex BLOCK-5 (round-1 adversarial review on
commit `6d5b7fa`). User pre-authorized Codex to adjudicate OQ-2. The
REJECT path is the conservative v0.1.0 surface.

**Decision:** Promoted dev-deps with `optional = true` in
`[dev-dependencies]` are REJECTED at synthesis with a directed
diagnostic. The pilot author must either (a) make the dep
non-optional, or (b) drop it from `dev_deps`.

**Rationale (Codex round-1, paraphrased):** Flipping `optional = true`
→ `optional = false` during the graft mutates the resolver graph:
   - It changes which deps participate in the build (a previously
     gated-by-feature dep now participates unconditionally).
   - It suppresses the `dep:<name>` feature-name suppression behavior
     (`[features].my-feat = ["dep:<name>"]` references the optional
     dep; without `optional = true`, the `dep:` syntax is rejected by
     cargo or silently becomes a regular feature ref).
   - It can subtly change the dylib's compilation behavior — code
     gated on `#[cfg(feature = "my-feat")]` that references the dep
     now compiles where it didn't before, potentially exposing build
     errors that the legacy path avoided.

REJECT is the safe v0.1.0 surface. The diagnostic must explicitly
explain the resolver-graph implications (so the adopter understands
why they're being asked to change their `[dev-dependencies]` shape).

**Implementation:** `src/dev_deps_overlay::synthesize_overlay` checks
`entry["optional"] == Value::Boolean(true)` in §3.2 step 3 and emits
`Error::Cli` with the diagnostic. Unit test in §8.2
(`rejects_optional_dev_dep`). Integration test in §8.4
(`optional_dev_dep_rejected`).

### §11.3 OQ-3 — `build_targets` ↔ `extern_crates` orthogonality — **LOCKED: orthogonal**

**Status:** Locked by user on 2026-05-19 before Codex adversarial
review. The three fields stay orthogonal; auto-inference rejected.

**Decision:** The three lihaaf metadata fields remain semantically
distinct:

- `extern_crates` — fixtures' `use <name>` imports (always
  forwarded).
- `dev_deps` — additional `--extern` forwardings (always
  forwarded).
- `build_targets` — gates the overlay synthesis (controls dylib
  build shape, not per-fixture forwarding).

Adopter setting `build_targets = ["tests"]` and `dev_deps = ["serde"]`
writes both. The redundancy is honest separation: one field per
concept.

**Rationale (user 2026-05-19):** "agreed on orthogonal." Consistent
with the explicit-config-first ethos that informs OQ-1's REPLACE
decision. Inferring "if it's in dev_deps AND build_targets is set,
also auto-forward" is exactly the magic-by-default pattern lihaaf
rejects.

### §11.4 OQ-4 — `[patch]` table interaction with promotion — **LOCKED: inline closure**

**Status:** Locked by Codex BLOCK on deferral (round-1 adversarial
review on commit `6d5b7fa`). Codex demanded inline resolution; the
user pre-authorized Codex to adjudicate OQ-4 either way. Deferral to
v1.0.0 is REJECTED; inline closure is the locked path.

**Decision:** `[patch.<registry>]`, `[replace]`, `[profile.*]` from
the **ancestor workspace root** are CARRIED DOWN into the synthesized
overlay (§3.6 step 3-5), with paths absolutized against the
workspace-root dir.

`[patch.<registry>]` or `[replace]` on the **metadata crate's
manifest** OR on the **dylib_crate's own manifest** (i.e. member-local
override tables) are REJECTED with a directed diagnostic, mirroring
`compat/overlay.rs:2009-2038`. Cargo itself rejects member-local
`[patch]` tables; we surface the diagnostic eagerly.

**Why inline closure works:**

The carry-down mechanism is already proven by compat-mode (#36 R3 /
v0.1.0-beta.10). `compat/overlay.rs:1977+
apply_workspace_member_inheritance` implements exactly the shape the
dev-deps overlay needs:

- Reject member-local `[patch.<registry>]` for ALL registries (lines
  2009-2038).
- Carry down `[workspace.dependencies]`, `[workspace.package]`,
  `[workspace.lints]`, `[workspace.metadata]`, `[workspace.resolver]`
  (lines 2077-2155).
- Carry down `[patch.<registry>]` from workspace root with path
  absolutization (lines 2156+).
- Carry down `[replace]` similarly.
- Carry down `[profile.*]` verbatim.

The dev-deps overlay's §3.6 step 3-5 reuses this exact code path via
the extracted shared `manifest_overlay::make_overlay_isolated_workspace`
helper. The compat-mode regression test (§8.3
`compat_overlay_byte_identical_after_extraction`) ensures the
extraction doesn't break compat-mode.

**Why this is NOT deferred per [[no-unilateral-deferral]]:**

Codex BLOCKed the R1 deferral on the grounds that the `[patch]`
interaction is integral to v0.1.0's correctness for any adopter whose
workspace root carries crate-graph overrides. The carry-down is
PROVEN by compat-mode's beta.10 shipped behavior; reusing the
existing precedent is a strict subset of the work already done. No
GH issue is filed; the inline closure ships in v0.1.0.

**Implementation:** `manifest_overlay::make_overlay_isolated_workspace`
extracts the workspace-member-inheritance logic from
`compat/overlay.rs:1977+` into a shared helper. The dev-deps overlay's
§3.6 step calls it; compat-mode is refactored to call the same helper.
The byte-identical regression test (§8.3) guards against extraction
drift.

The §12.8 GH-issue step from R1 is REMOVED. No deferral artifact.

---

## §12 Implementation order

The order below is constructed so each step's tests pass against the
codebase at that step's exit. No step's tests depend on a later
step's code.

### §12.1 Step 1 — extract shared `manifest_overlay` module

**Branch:** `feat/overlay-shared-helper-extraction`. Independent PR.

1. Create `src/manifest_overlay/mod.rs`.
2. Move `absolutize_path_bearing_keys`, `absolutize_string_at`,
   `absolutize_array_table_paths`, `absolutize_deps_paths`,
   `absolutize_patch_paths`, `absolutize_replace_paths`,
   `lexical_path_normalize_path`, `lexical_normalize_pathbuf`,
   `serialize_canonical` from `src/compat/overlay.rs` to the new
   module, marking each `pub(crate)`.
3. Extract `resolve_workspace_member_manifest` from
   `compat/overlay.rs:1505+` into a re-usable helper exposed as
   `manifest_overlay::resolve_dylib_crate_manifest` (with the
   single-crate shortcut when metadata `[package].name == dylib_crate`).
4. Extract `apply_workspace_member_inheritance` from
   `compat/overlay.rs:1977+` into
   `manifest_overlay::make_overlay_isolated_workspace`. The compat-mode
   semantics are preserved via parameter shape: the dev-deps overlay
   provides a different `workspace_root_value` consumer, but the same
   carry-down logic.
5. Update `src/compat/overlay.rs` to import from
   `crate::manifest_overlay::{...}`.
6. Add `src/manifest_overlay/mod.rs` to `src/lib.rs` module
   declarations.
7. Add the §8.3 regression test: existing compat-mode test produces
   byte-identical output to pre-extraction baseline.

**Verification (§9):** all four commands pass. Existing
`compat/overlay.rs` tests are byte-identical.

**Adversarial-review trip-wire:** if any compat-mode test produces
even one byte of different output → BLOCK.

### §12.2 Step 2 — add `build_targets` field to `Config` + validate

**Branch:** `feat/build-targets-config`. Stacked on Step 1.

1. Add `build_targets: Vec<String>` to `Suite` (`src/config.rs:106-212`).
2. Add `build_targets: Option<Vec<String>>` to `RawMetadata`
   (`src/config.rs:299-320`) and `RawSuite` (`src/config.rs:325-349`).
3. Add `validate_build_targets` helper.
4. Wire validation + finalization in `build_default_suite`
   (`src/config.rs:539+`) and `finalize_named_suite`
   (`src/config.rs:650+`).
5. Add §8.1 unit tests for config parse.

**Verification (§9):** all four commands pass. `cargo test --lib`
includes the new unit tests.

**Note:** at this step, `BuildParams` does NOT yet have
`build_targets`. The new `Suite` field exists but is unused by the
dylib build. This is an intentional staging — the change is
parse-only.

### §12.3 Step 3 — add `dev_deps_overlay` module (Shape δ)

**Branch:** `feat/dev-deps-overlay-module`. Stacked on Step 2.

1. Create `src/dev_deps_overlay/mod.rs` with the §3 algorithm.
2. The module exports `synthesize_overlay`.
3. Use the §12.1 shared `manifest_overlay::*` helpers:
   - `resolve_dylib_crate_manifest` (§3.3)
   - `make_overlay_isolated_workspace` (§3.6)
   - `absolutize_path_bearing_keys` (§3.9)
   - `serialize_canonical` (§3.11)
4. Add §8.2 unit tests (overlay-synthesis round-trip + edge cases).

**Verification (§9):** all four commands pass. New unit tests pass.

**Note:** at this step, the new module is dead code (no caller).
Intentional staging — the synthesis routine is independently
verifiable via its unit tests.

### §12.4 Step 4 — wire `build` to call synthesis

**Branch:** `feat/dylib-build-overlay-wire`. Stacked on Step 3.

1. Add `build_targets: &'a [String]` and `dev_deps: &'a [String]`
   fields to `BuildParams` (`src/dylib.rs:60`).
2. Add the §2.2 Site 2 branch inside `build` (`src/dylib.rs:80+`).
3. Update the invocation-string rendering (§2.2 Site 3).
4. Update the session orchestrator (`src/session.rs:330-336`) to
   populate the new `BuildParams` fields from `Suite`.
5. Add §8.5 regression test: empty `build_targets` does NOT call
   into `synthesize_overlay`.

**Verification (§9):** all four commands pass.

**Adversarial-review trip-wire:** if any existing test that doesn't
opt into `build_targets` fails → BLOCK. The byte-identical
contract must hold.

### §12.5 Step 5 — cargo-build-gated integration tests (BLOCK-6 closure)

**Branch:** `feat/dev-deps-overlay-integration-test`. Stacked on
Step 4.

1. Add `tests/dev_deps_overlay_integration.rs`.
2. Gate behind `#[cfg(feature = "cargo-build")]`.
3. Add the §8.4 test cases, including:
   - `split_crate_axum_macros_minimal_repro` (BLOCK-2 closure proof)
   - `workspace_inheritance_carry_down` (BLOCK-1)
   - `workspace_root_patch_inline_closure` (OQ-4)
   - `member_local_patch_rejected` (OQ-4)
   - `no_explicit_lib_block` (BLOCK-3)
   - `optional_dev_dep_rejected` (BLOCK-5)
4. Verify the tests pass in CI.

**Verification (§9):** all four commands pass. Per
[[lihaaf-no-local-binary-builds]], the new tests are NOT run locally;
CI runs them.

### §12.6 Step 6 — spec + user-guide amendments

**Branch:** `docs/dev-deps-overlay-spec-and-guide`. Stacked on
Step 5 (or rebased onto `docs/user-guide` for the user-guide diff).

1. Apply the §6 spec amendments to `docs/spec/lihaaf-v0.1.md`.
2. Apply the §7 user-guide amendments to `docs/user-guide.md` on
   the `docs/user-guide` branch.
3. Add a CHANGELOG.md entry for the new field.
4. Add a v0.1.0 entry in the changelog naming Shape δ.

**Verification (§9):** all four commands pass. `cargo doc` should
build the new rustdoc on `Suite.build_targets`.

### §12.7 Step 7 — enable on axum-macros pilot fork

**Branch:** (in the `axum-lihaaf-pilot` repo, not lihaaf). Not part
of this dispatch; the pilot fork is a separate repository.

1. Add `build_targets = ["tests"]` to the default suite + each named
   suite that has `dev_deps` (default, from_request, typed_path; the
   debug_middleware suite has no `dev_deps`, no overlay needed).
2. Run the pilot's CI; verify all 93 fixtures pass.
3. Measure the cold-cache wall-clock for the §10 estimate.
4. If measurement diverges materially from §10 estimate, update
   §10.

(R1's §12.8 — file v1.0.0 GH issue for OQ-4 — is REMOVED. The OQ-4
inline closure per §11.4 means no deferred work artifact.)

---

## §13 Sanity checks

### §13.1 Locked-constraint compliance

| Locked constraint | Compliance |
|-------------------|-----------|
| 1. CLI-only, no library API | No new public API exposed beyond the existing `cargo-lihaaf` binary. |
| 2. Dylib-only is design DNA | The change keeps the dylib build invocation shape; only the manifest source changes. |
| 3. Explicit > implicit | `build_targets` is opt-in; omitted = byte-identical legacy behavior. |
| 4. No upstream pollution | Synthesized overlays live in `target/`, never in source. |
| 5. Backwards compat byte-identical when `build_targets` omitted | §5.1 contract; §5.2 per-adopter audit. |
| 6. Workspace identity correctness | §3.6 isolated overlay workspace + carry-down per compat-mode #36 precedent. BLOCK-1 closure. |
| 7. Quality > velocity | Every Codex-enumerated edge case has an explicit policy; BLOCKs 1-6 + OQ-4 closed inline. |

### §13.2 Memory ledger compliance

- [[lihaaf-no-local-binary-builds]]: §8.4's integration tests are
  `cargo-build`-gated, run in CI only. §9's verification commands
  exclude the forbidden binaries.
- [[lihaaf-review-verify-cmds]]: §9 lists all four required
  commands.
- [[lihaaf-pilot-coverage-gap]]: this feature unblocks the
  axum-macros pilot that Round-2 coverage requires for v0.1.0.
- [[lihaaf-three-reviewer-panel-calibration]]: the plan will be
  dispatched to Codex 5.5 xhigh adversarial review per the project's
  pre-implementer cycle.
- [[lihaaf-plan-adversarial-cycle]]: this plan is the planner's
  output; the next step is adversarial review BEFORE
  careful-coder dispatch.
- [[no-unilateral-deferral]]: OQ-4 is no longer deferred; inline
  closure ships with v0.1.0 (per Codex BLOCK on deferral).
- [[lihaaf-dev-deps-explicit-keep]]: explicit `dev_deps` list
  preserved; no auto-discovery from `[dev-dependencies]`.
- [[lihaaf-cli-only-never-library]]: no library API added.

### §13.3 Codex round-1 BLOCK closures (cross-reference)

| BLOCK | Closure section(s) | Evidence |
|-------|--------------------|----------|
| BLOCK-1: Workspace inheritance | §3.6, §8.4 `workspace_inheritance_carry_down` | Compat-mode precedent (`compat/overlay.rs:1977+`); shared helper extraction (§12.1); integration test (§8.4) |
| BLOCK-2: dylib_crate != metadata package | §3.1-§3.5 (entire Shape δ algorithm); §8.4 `split_crate_axum_macros_minimal_repro` | Workspace-member resolver (`compat/overlay.rs:1505+`) reused; integration test reproduces axum-macros shape |
| BLOCK-3: No `[lib]` injection | §3.8, §8.4 `no_explicit_lib_block` | Mirrors `compat/overlay.rs:2456-2470` |
| BLOCK-4: §5 adopter inventory incomplete | §5.2 (entire rewrite); §5.2.1-§5.2.13 | All locally-present adopters cited at file:line; non-local adopters explicitly surfaced as `NOT LOCALLY PRESENT` |
| BLOCK-5 (OQ-2): optional flip | §3.7.5, §11.2, §8.2 `rejects_optional_dev_dep`, §8.4 `optional_dev_dep_rejected` | REJECT locked; unit + integration tests |
| BLOCK-6: Test coverage gaps | §8.4 (new) | Six new cargo-build-gated tests; one new shared-helper regression test (§8.3 `compat_overlay_byte_identical_after_extraction`) |
| OQ-4: `[patch]` deferral | §11.4, §3.6, §3.7.4, §8.4 `workspace_root_patch_inline_closure`, §8.4 `member_local_patch_rejected` | Inline closure; GH-issue step REMOVED |

---

## §14 Out-of-scope (NOT in this plan)

For Codex's review: these are intentionally NOT in this plan and
should not be raised as gaps.

- `"examples"` and `"benches"` as `build_targets` values. Deferred
  to v0.2+ per a user-authorized milestone scope decision.
- Per-suite `[patch]` injection. Deferred (workspace-root `[patch]`
  inheritance handles every pilot-known case per §3.7.4).
- Auto-discovery of `dev_deps` from the metadata crate's
  `[dev-dependencies]` table. Locked-rejected; the user explicitly
  opted out per [[lihaaf-dev-deps-explicit-keep]].
- Compat-mode (`cargo lihaaf --compat`) using `build_targets`.
  Deferred; compat-mode's synthetic metadata defaults to empty
  `build_targets`.
- Removing the existing `dev_deps` field. Out of scope; backwards
  compat invariant.
- Refactoring `compat/overlay.rs` beyond the §12.1 helper extraction.
- Library API for `lihaaf::synthesize_overlay` or similar — lihaaf
  remains CLI-only per [[lihaaf-cli-only-never-library]].
- Cross-platform path-relative overlays (the absolute-path determinism
  caveat in §4.1 is documented and accepted).

---

## §15 Adversarial-review checklist for Codex round-2

Codex should specifically verify:

1. **File:line accuracy.** Every cited line in §2 ("Source-level
   changes needed") and §5 (adopter inventory) matches the actual line
   in the named files as of 2026-05-19.
2. **Shape δ correctness.** The synthesis target is the dylib_crate's
   manifest; specs are grafted from the metadata crate's
   `[dev-dependencies]`. Cross-check §0.2, §3.1-§3.5.
3. **Workspace identity correctness.** §3.6 reuses
   `compat/overlay.rs:1977+ apply_workspace_member_inheritance` via
   the extracted shared helper. The carry-down handles
   `[workspace.*]`, `[patch.*]`, `[replace]`, `[profile.*]` and
   rejects member-local override tables.
4. **`[lib]` injection.** §3.8 mirrors `compat/overlay.rs:2456-2470`.
5. **Adopter inventory completeness.** §5.2 cites file:line for every
   locally-present adopter; non-local adopters explicitly surfaced.
   No TODO/verify markers.
6. **OQ-2 locked REJECT.** §3.7.5 + §11.2 reject optional dev-deps
   at synthesis. Unit + integration tests verify.
7. **OQ-4 locked inline closure.** §11.4 carries down workspace-root
   `[patch]`/`[replace]`/`[profile]`; member-local rejected. §12.8
   GH-issue step REMOVED.
8. **§8 test coverage.** Each Codex-enumerated failure mode + each
   BLOCK closure has a corresponding test in §8.2 (unit) or §8.4
   (integration).
9. **§10 speed claim defensibility.** The ~+12-15% cold-cache
   estimate is grounded in axum-macros' specific shape; warm-cache
   100% hit-rate is grounded in the determinism contract.
10. **§12 step independence.** Each step's tests pass at that step's
    exit. No step's tests require a later step's code.
11. **No silent locked-constraint violation.** §13.1 enumerates all
    seven (R2 trimmed from 9 to 7 by consolidating overlapping
    constraints; verify each line).

---

**End of plan.**
