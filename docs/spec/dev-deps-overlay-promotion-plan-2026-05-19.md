# Plan: synthetic same-crate overlay promoting selected `dev_deps` to regular `[dependencies]`

**Date:** 2026-05-19
**Target milestone:** v0.1.0
**Working branch:** `plan/dev-deps-overlay-promotion`
**Author:** strict-swe (planner mode)
**Status:** draft — pending Codex 5.5 xhigh adversarial review

This plan implements Codex's Candidate E (overlay-promotion shape; user
approved 2026-05-19 with "perfect"). The shape replaces the rejected
auto-discovery design ([[plan/dev-deps-auto-discovery]] — `docs/spec/
dev-deps-auto-default-plan-2026-05-19.md`, on the sibling branch, BLOCKed
by Codex on source-citation errors and rejected same day by the user)
with an **opt-in**, **explicit** mechanism: a new `build_targets` field
that, when non-empty, tells lihaaf to synthesize an overlay manifest in
which the named `dev_deps` are moved from `[dev-dependencies]` into
`[dependencies]`. lihaaf then runs its existing release-mode dylib build
against the overlay rather than against the adopter's `Cargo.toml`
directly. This makes the named dev-deps part of the regular dependency
graph for the single `cargo rustc -p X --lib --release --crate-type=dylib`
invocation, so the dev-dep rlibs land in `deps_dir` and per-fixture rustc
can resolve them via the existing `--extern` shape.

The design preserves every locked invariant: CLI-only, dylib-only,
explicit-config, single cargo invocation, no two-phase fingerprint
split, no auto-discovery, no fork pollution, byte-identical backwards
compat for every existing adopter.

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
(`/home/tarunvir/projects/axum-lihaaf-pilot/axum-macros/Cargo.toml:81,
124, 131`), which configures `dev_deps = ["axum-extra", "serde"]` on
the default + `from_request` + `typed_path` suites. The fork's
pilot-CI shows 24 of 93 fixtures fail with `error[E0432]: unresolved
import` against `serde::Deserialize` and `axum_extra::*`. The same
class of failure will hit every future adopter whose fixtures `use`
crates that aren't transitively reachable through the dylib_crate's
**regular** `[dependencies]` tree.

The four Round-1 + two Round-2 pilots that work today
(anyhow, thiserror, cxx, serde_json, djogi, sassi, derive_more) work
**only because** every fixture in their corpora `use`s crates already
present in the consumer crate's regular `[dependencies]` tree. Once
an adopter writes a fixture that imports a crate from
`[dev-dependencies]` only, lihaaf is broken for that crate.

This is a v0.1.0 ship-blocker: lihaaf's stated v0.1.0 coverage matrix
([[lihaaf-pilot-coverage-gap]]) requires axum-macros, which requires
this fix.

### §0.2 Why this change is in scope for v0.1.0

The fix is surgical:

- One new opt-in field (`build_targets`).
- One conditional code path in `src/dylib.rs::build` that constructs
  an overlay manifest before invoking `cargo rustc`.
- Overlay synthesis reuses precedent from `src/compat/overlay.rs`
  (specifically: TOML round-trip, path absolutization, workspace
  membership handling).
- No changes to per-fixture rustc (`src/worker.rs:960-1007`).
- No new public API.
- Backwards-compatible: adopters who omit `build_targets` see no
  behavior change — the byte-identical legacy path runs.

### §0.3 The decision (locked, from Codex Candidate E)

Add an explicit opt-in `build_targets` field to both the top-level
`[package.metadata.lihaaf]` and each `[[package.metadata.lihaaf.suite]]`
entry. Default is omitted / `[]` → existing behavior unchanged. The only
permitted value in v0.1.0 is `"tests"`. When `build_targets = ["tests"]`,
lihaaf synthesizes an overlay manifest at
`<target_dir>/lihaaf-overlay-<suite>/Cargo.toml` that is a verbatim copy
of the dylib_crate's `Cargo.toml` with the entries named in `dev_deps`
**moved** from `[dev-dependencies]` into `[dependencies]`. The existing
`cargo rustc` invocation then runs against the overlay's manifest path.

`dev_deps` semantics remain unchanged: the field is still the explicit
allow-list of `--extern` forwardings to per-fixture rustc. The new
field is **orthogonal**: it controls overlay-promotion, not extern
forwarding. They are intentionally coupled in adopter UX (typically
both are set together) but decoupled in the code path so the lihaaf
self-test (`Cargo.toml:210-219`, no `dev_deps`, no `build_targets`)
keeps its zero-overhead model.

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
rather than adding another fragile inheritance path. The adopter who
wants overlay synthesis across all suites declares
`build_targets = ["tests"]` per suite — same shape as the existing
per-suite `features = ["macros"]` pattern. (See §11 OQ-1 for the
locked decision.)

### §1.2 Truth table — `(build_targets, dev_deps)` combinations

The two fields interact along two axes: whether the overlay manifest
is synthesized at all (`build_targets`), and which crates are
`--extern`-forwarded to per-fixture rustc (`dev_deps`).

| `build_targets` | `dev_deps`          | Overlay synthesized? | dylib build manifest                   | `--extern` forwarding                |
|-----------------|---------------------|----------------------|----------------------------------------|--------------------------------------|
| omitted / `[]`  | omitted / `[]`      | No                   | adopter's Cargo.toml verbatim          | none beyond `extern_crates`          |
| omitted / `[]`  | `["a", "b"]`        | No                   | adopter's Cargo.toml verbatim          | `--extern a`, `--extern b`           |
| `["tests"]`     | omitted / `[]`      | **REJECT** (§1.4)    | n/a                                    | n/a                                  |
| `["tests"]`     | `["a", "b"]`        | Yes                  | overlay (a, b moved to `[dependencies]`)| `--extern a`, `--extern b`           |
| `["invalid"]`   | anything            | **REJECT** (parse)   | n/a                                    | n/a                                  |
| any value       | `["a"]`, `a` not in adopter's `[dev-dependencies]` | **REJECT** at synthesis (§3.2)| n/a | n/a |

Row 3 (build_targets but no dev_deps) is rejected because the overlay
would be byte-identical to the adopter's Cargo.toml — paying the
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
| Default, opt-in        | `["tests"]`                     | `<target_dir>/lihaaf-build/lihaaf-overlay-default/`      |
| Named, omitted         | `[]` (REPLACE; no inheritance)  | none                                                     |
| Named "spatial", opt-in| `["tests"]`                     | `<target_dir>/lihaaf-build-spatial/lihaaf-overlay-spatial/`|

The overlay dir sits **inside** the per-suite target dir (not as a
sibling), so cargo's fingerprint already isolates it from sibling
suites. The naming `lihaaf-overlay-<suite>` is for human readability
in `target/` listings (multiple suites' overlays coexist without
collision).

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
For named suites, the resolved `dev_deps` is `raw.dev_deps.unwrap_or_else(|| default_suite.dev_deps.clone())`. Note `build_targets`
uses REPLACE while `dev_deps` uses INHERIT — this asymmetry is the
§11.1 locked decision; do not "fix" it by giving `build_targets`
inheritance.

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

**Site 1: `pub struct BuildParams<'_>` (line 60).** Add `build_targets`
and `dev_deps` borrow slices alongside the existing `features`:

```rust
/// Build targets to compile beyond `--lib`, gating the overlay
/// promotion. Empty slice → no overlay; non-empty → §3 synthesis.
pub build_targets: &'a [String],
/// Dev-deps subset to promote into the overlay's `[dependencies]`.
/// Caller passes the resolved (validated) `dev_deps` slice.
pub dev_deps: &'a [String],
```

The two new fields slot between `features` (line 64) and
`manifest_path` (line 66).

**Site 2: `pub fn build` (line 80).** Insert overlay-synthesis branch
BEFORE the cargo command is assembled (line 93). Sketch:

```rust
let effective_manifest_path: PathBuf = if !params.build_targets.is_empty() {
    let overlay_dir = params.target_dir.join("lihaaf-overlay");
    let overlay_manifest = synthesize_overlay_manifest(
        params.manifest_path,
        params.dev_deps,
        &overlay_dir,
    )?;
    overlay_manifest
} else {
    params.manifest_path.to_path_buf()
};
```

Then `cmd.arg("--manifest-path").arg(&effective_manifest_path)`
replaces the existing line 101-102. The rest of `build` (RUSTFLAGS,
features pass-through, output parsing) is unchanged.

The choice to put the overlay dir at `<target_dir>/lihaaf-overlay`
(not `<target_dir>/lihaaf-overlay-<suite>`) is deliberate: the suite
namespacing is **already** baked into `target_dir` itself
(`build_dir_for_suite`, line 388-393). The default suite's overlay
lives at `<workspace>/target/lihaaf-build/lihaaf-overlay/Cargo.toml`,
the "spatial" suite's at `<workspace>/target/lihaaf-build-spatial/lihaaf-overlay/Cargo.toml`. No cross-suite collision.

**Site 3: invocation-string rendering (line 125-137).** The diagnostic
invocation string must reflect the **effective** manifest path so the
adopter can paste a working reproduction. Update the format string to
use `effective_manifest_path` instead of `params.manifest_path`.

**Site 4: `BuildOutput.deps_dir` (line 164-167) is unaffected.** The
overlay manifest's `target/` is still `<target_dir>` (cargo joins
`<target_dir>/release/deps`), and the overlay's package emits its
artifacts into the same `deps_dir`. Per §4 idempotency, this is the
load-bearing claim.

### §2.3 New module `src/dev_deps_overlay.rs`

Create a sibling-to-`dylib.rs` module:

```text
src/
  dev_deps_overlay.rs   (new)
  dylib.rs
  compat/
    overlay.rs           (existing — compat-mode driver overlay)
```

**Why a new module, not a refactor of `compat/overlay.rs`?**
The compat-mode overlay (`compat/overlay.rs`, 8751 lines, public entry
`materialize_overlay` at line 515) is built for a very different
problem:

- Compat-mode overlay **inserts a `[lib] crate-type = ["dylib"]`** to
  rewrite a non-dylib upstream into a dylib-buildable shape
  (`compat/overlay.rs:697-710`).
- Compat-mode handles workspace-root-vs-workspace-member resolution
  via `--package` (`compat/overlay.rs:1505+` — `resolve_workspace_member_manifest`).
- Compat-mode injects synthetic `[package.metadata.lihaaf]` for the
  inner session driver (`compat/overlay.rs:790`).
- Compat-mode populates a structural mirror of the upstream package
  root for build scripts (`compat/overlay.rs:849-869`).
- Compat-mode applies the 4-rule self-patch policy for path-conflict
  handling (`compat/overlay.rs:725-787, 2966+ — apply_self_patch_policy`).

Almost none of that applies to v0.1 dev-deps overlay:

- The adopter's `[lib] crate-type` is already correct (they're a
  lihaaf adopter; their `Cargo.toml` already declares the dylib_crate
  as a normal lib).
- The adopter's manifest already lives in the workspace they want
  cargo to resolve against (no compat-root resolution).
- No synthetic metadata is injected (the adopter's metadata is what
  drives the inner session).
- No package-root mirror needed (the dylib build uses the adopter's
  source tree directly; the overlay's `[lib] path` points at the
  adopter's existing `src/lib.rs`).
- No self-patch policy (the dev-deps overlay isn't aliasing
  `crates-io.X` source-ids; it only moves entries between
  `[dependencies]` and `[dev-dependencies]`).

**The shared primitive is path absolutization.** The
`absolutize_path_bearing_keys` function at `compat/overlay.rs:2370+`
handles `[lib] path`, `[package] build`, `[[bin/example/test/bench]] path`,
`[dependencies/dev-dependencies/build-dependencies/target.<cfg>.deps].path`,
and `[workspace.dependencies.X].path`. The dev-deps overlay needs
**exactly the same logic** because the staged overlay sits at
`<target_dir>/lihaaf-build/lihaaf-overlay/Cargo.toml`, which is two
levels deeper than the adopter's `<adopter>/Cargo.toml`, so every
relative path-bearing key must be absolutized against the adopter's
dir.

**Decision:** extract `absolutize_path_bearing_keys` into a shared
internal helper crate-local module `src/manifest_overlay/mod.rs` with
exactly two exported helpers:

```rust
pub(crate) fn absolutize_path_bearing_keys(
    top: &mut toml::map::Map<String, toml::Value>,
    source_dir: &Path,
    source_manifest_path: &Path,
) -> Result<(), Error>;

pub(crate) fn serialize_canonical(value: &toml::Value) -> Result<Vec<u8>, Error>;
```

Both modules — `compat/overlay.rs` and the new `dev_deps_overlay.rs` —
call into `manifest_overlay::absolutize_path_bearing_keys` and
`manifest_overlay::serialize_canonical`. This:

- Eliminates the duplication risk (a bug fix in one path-handling
  routine flows to both).
- Preserves the compat-overlay's invariants: the compat path
  continues to call `absolutize_path_bearing_keys` exactly as today.
- Confines the shared surface to two function signatures.

The shared module is **not** a refactor that touches the existing
compat-overlay logic. The existing functions are moved to the new
module location and `compat/overlay.rs` becomes a caller. The
extracted-as-is invariant is enforced by tests:
`compat/overlay.rs` integration tests continue to pass byte-identically.

**`src/dev_deps_overlay.rs` structure (sketch):**

```rust
//! Synthetic same-crate overlay for dev-deps promotion, per
//! `docs/spec/dev-deps-overlay-promotion-plan-2026-05-19.md`.
//!
//! When the adopter's lihaaf config sets `build_targets = ["tests"]`,
//! lihaaf synthesizes an overlay `Cargo.toml` that is a verbatim copy
//! of the adopter's manifest with the named `dev_deps` MOVED from
//! `[dev-dependencies]` into `[dependencies]`. The single-cargo-invocation
//! shape compiles those crates' rlibs into `deps_dir` so per-fixture
//! rustc can resolve them via `--extern`.

use std::path::{Path, PathBuf};

use crate::error::Error;
use crate::manifest_overlay::{absolutize_path_bearing_keys, serialize_canonical};

/// Synthesize the overlay manifest. Returns the absolute path of the
/// staged manifest file.
pub fn synthesize_overlay_manifest(
    adopter_manifest_path: &Path,
    promoted_dev_deps: &[String],
    overlay_dir: &Path,
) -> Result<PathBuf, Error> { ... }
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

1. The overlay-promoted dev-deps get compiled into `deps_dir` by the
   single `cargo rustc -p X --lib` invocation against the overlay.
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
   parameters.

The change is local: when building the `BuildParams` struct for each
suite, populate the new `build_targets` and `dev_deps` borrows from
the suite's resolved fields. Site identified by grepping for current
`BuildParams { ... }` construction in `src/session.rs` (and any other
caller; there's only one production call site, plus tests).

---

## §3 Overlay manifest synthesis algorithm

The synthesis algorithm runs inside `src/dev_deps_overlay.rs::
synthesize_overlay_manifest`. Input: the adopter's `Cargo.toml` path,
the `dev_deps` list of names to promote, the target overlay directory.
Output: the absolute path of the written overlay manifest, plus the
side effect of the file written atomically.

The algorithm is broken into numbered steps. Each step states (a) what
it does, (b) the invariant it preserves, (c) the failure-mode policy
for each of Codex's enumerated edge cases.

### §3.1 Step 1 — read + parse the adopter's `Cargo.toml`

Identical shape to `compat/overlay.rs:621-642`:

```rust
let raw_bytes = std::fs::read(adopter_manifest_path)?;
let raw_text = String::from_utf8(raw_bytes)?;
let mut value: toml::Value = toml::from_str(&raw_text)?;
```

Failure → `Error::Io` or `Error::TomlParse` per existing conventions.
The adopter dir is `adopter_manifest_path.parent()` for downstream
absolutization.

### §3.2 Step 2 — validate the promoted `dev_deps` exist in `[dev-dependencies]`

For each name in `promoted_dev_deps`:

1. Look up the name in `value["dev-dependencies"][name]`. If absent,
   **REJECT** with a directed diagnostic: `dev_deps[i] = "<name>"`
   is listed in `[package.metadata.lihaaf].dev_deps` but is not
   present in `[dev-dependencies]` of `<adopter_manifest_path>`. Did
   you mean to add it to `[dev-dependencies]`? Or remove the
   `build_targets = ["tests"]` opt-in if you intended `<name>` to
   come from `[dependencies]`.
2. Also look up the name in `value["dependencies"][name]`. If
   **also present**, REJECT: a single dev-dep name cannot be in
   both. (Cargo itself rejects this with E0464 or similar; we
   eagerly surface a directed diagnostic.)
3. Record the dep's value (the entire TOML subtree) for promotion in
   Step 4.

Renamed dev-deps: cargo's `package = "serde_json"` rename mechanism
means the **TOML key** is the rename (e.g., `serde-json`); the
package name is in the entry's `package` field. The adopter's
`dev_deps = ["serde-json"]` matches the **TOML key**, not the package
name, per the existing dev_deps convention (`src/worker.rs:1003-1006`
uses the name verbatim for `--extern serde_json=path` after the
rename-collapse `replace('-', '_')`). The lookup in
`[dev-dependencies]` uses the same TOML key. **Policy:** renamed
dev-deps work transparently; the overlay simply moves the renamed
entry verbatim. The `--extern` name in the per-fixture rustc
invocation continues to use the renamed key (the existing rename
collapse stays correct).

Validation must happen BEFORE Step 4 (move) so a typo in `dev_deps`
fails fast instead of producing a half-built overlay.

### §3.3 Step 3 — apply Codex-enumerated edge-case policies

For each promoted dev-dep entry (the TOML subtree gathered in §3.2),
process it through the per-shape policies BEFORE moving it to
`[dependencies]`:

#### §3.3.1 `workspace = true` shorthand (Codex enumeration #1)

Pattern: `serde = { workspace = true, features = [...] }`. The
shorthand tells cargo to inherit the dep's version+features from the
ancestor `[workspace.dependencies.serde]` table.

**Policy: preserve the shorthand verbatim.** The overlay manifest
sits at `<adopter>/target/lihaaf-build/lihaaf-overlay/Cargo.toml`. The
cargo walk-up will land on the same workspace root the adopter's
original `Cargo.toml` resolves to (because cargo walks up from the
overlay's location, which is **inside** the adopter's source tree).
The `{ workspace = true }` shorthand in `[dependencies]` resolves the
same way it does in `[dev-dependencies]`: cargo looks up the dep name
in `[workspace.dependencies]`.

**Required invariant:** the overlay's workspace must be the **same**
workspace as the adopter's. This holds by construction: the overlay
lives under `<adopter>/target/`, so cargo's walk-up first hits the
adopter's `Cargo.toml` (which already has either `[workspace]` or
`[package].workspace = "..."`). The overlay is a **sibling** of the
adopter manifest in the workspace-resolver sense, not a child. (See
§3.4 invariant on the overlay's relationship to the parent workspace.)

**Note for adversarial review:** this is the key place this design
diverges from compat-mode overlay. Compat-mode overrides the
workspace (because it's a synthetic outer workspace driving an inner
session). Dev-deps overlay **preserves** the workspace inheritance.
Mechanically: dev-deps overlay does **NOT** call
`override_workspace_inheritance`. It MUST NOT — the workspace
inheritance is load-bearing for the `{ workspace = true }` shorthand.

#### §3.3.2 Path dev-deps (Codex enumeration #2)

Pattern: `local-helper = { path = "../local-helper" }`. Relative
path resolved against the adopter manifest's directory.

**Policy: absolutize the path against the adopter dir.** This is
exactly what `absolutize_path_bearing_keys` already does for
`[dev-dependencies].path` (`compat/overlay.rs:2524`). The shared
extracted helper handles this transparently — the path is rewritten
to absolute form BEFORE Step 4 moves the entry into
`[dependencies]`. Cargo accepts forward-slash paths on every
platform per the compat-overlay precedent.

#### §3.3.3 Git dev-deps (Codex enumeration #2.bis)

Pattern: `some-dep = { git = "https://...", rev = "..." }`. No
filesystem path; cargo fetches into the workspace's registry cache.

**Policy: verbatim copy.** Git URLs are not affected by the
overlay's filesystem location. The dep entry is moved as-is.
`absolutize_path_bearing_keys` skips entries without `path` keys.

#### §3.3.4 `[patch]` sections (Codex enumeration #3)

Pattern: the adopter's `Cargo.toml` (or the ancestor workspace
root's `Cargo.toml`) carries `[patch.crates-io.serde] = { path =
"./forks/serde" }`.

**Policy: do not touch `[patch]`.** The patch sections are inherited
from the workspace root via cargo's walk-up, exactly as for the
adopter's original manifest. The overlay sits inside the same
workspace, so `[patch]` continues to resolve correctly without any
intervention.

If the adopter's `Cargo.toml` carries a `[patch]` section directly
(rare for non-workspace-root manifests; cargo emits a warning), the
absolutization helper at `compat/overlay.rs:2674+ — absolutize_patch_paths` handles the rewrite. The shared
extracted helper inherits that.

**Adversarial-review trip-wire:** the `[patch]` table itself is NOT
moved or copied; the overlay just inherits via cargo walk-up. If
the adopter's `Cargo.toml` has `[patch.crates-io.<X>] = { path =
"..." }` for some dep X that is **also** in the promoted
`dev_deps` list, the patch fires for the overlay's `[dependencies]`
entry the same way it fired for `[dev-dependencies]`. This is the
desired behavior — the adopter's patch declaration is intent
("use this fork everywhere"); promoting the dev-dep into the
regular graph just makes the patch apply to one more reference of
the same source-id.

#### §3.3.5 `dep:` feature syntax + optional dev-deps (Codex enumeration #4)

Pattern: `[dev-dependencies] serde = { version = "1", optional =
true }`, plus `[features] my-feature = ["dep:serde"]`.

**Policy (v0.1.0):** when the dev-dep entry has `optional = true`,
**flip it to `optional = false`** during the move into
`[dependencies]`. Rationale: the adopter explicitly listed this
crate in `dev_deps` for promotion. The opt-out path (don't list it
in `dev_deps`) is available. Once promoted, `optional = true` would
require the corresponding feature to be active, which is not
something lihaaf's existing `features` field model handles cleanly
(features in lihaaf's metadata gate the dylib_crate's features,
not arbitrary dep activation).

The `dep:serde` feature reference in `[features]` continues to
work after promotion: cargo's resolver treats the moved entry as a
regular `[dependencies]` member, and `dep:serde` resolves to the
same dep node.

**Trip-wire (adversarial-review note):** if a `[features].X =
["dep:serde"]` ALSO appears in the adopter's `[dev-dependencies]`
side as a `serde = { version = "1", optional = true }` entry, the
feature wiring may break in subtle ways when the promotion flips
the optional bit. The reject-and-document path is captured as
§11 OQ-2 — the current recommendation is "promote with
optional=false; trip an adversarial pilot before promoting to a
guaranteed-safe transformation."

#### §3.3.6 Renamed dev-deps via `package = "..."` (Codex enumeration #5)

Pattern: `[dev-dependencies] serde-json = { package = "serde_json",
version = "1" }`.

**Policy: preserve the rename.** The TOML key (`serde-json`) is the
name cargo registers the dep under; the `package` field is the
actual package name. The promotion moves the entire entry — key +
value subtree — into `[dependencies]` verbatim. The `--extern`
forwarding at `src/worker.rs:1003-1008` reads the lihaaf
`dev_deps` entries verbatim (which are the TOML keys, not the
package names), so the rename collapse `name.replace('-', '_')`
stays correct for the renamed crate.

**Cross-check:** the adopter's `dev_deps = ["serde-json"]` (the
TOML key, not the package name) is the convention the spec already
documents (`docs/spec/lihaaf-v0.1.md:332`: `dev_deps = ["serde",
"serde_json"]` — both are TOML keys and package names because no
rename is in play in the example). If the adopter writes the
package name instead of the TOML key, the Step 2 validation fires
(no matching `[dev-dependencies]` entry).

#### §3.3.7 cfg-gated dev-deps (Codex enumeration #6)

Pattern: `[target.'cfg(unix)'.dev-dependencies] something = "1"`.

**Policy (v0.1.0): explicit REJECT.** If any name in the promoted
`dev_deps` list lives **only** under a `[target.<cfg>.dev-dependencies]`
table (not in the top-level `[dev-dependencies]`), reject with:

> `dev_deps[i] = "<name>"` is configured under
> `[target.<cfg>.dev-dependencies]` only. Conditional dev-dep
> promotion is not supported in v0.1.0 (the overlay synthesis
> cannot reliably evaluate the cfg-expression at synthesis time).
> Workarounds: (a) move the dep to top-level `[dev-dependencies]`
> if it can be unconditional; (b) skip promoting this dep via
> `dev_deps` and structure the fixture to not import it; (c) wait
> for v0.2's cfg-gated promotion support.

This matches the design decision recorded as v0.2+ backlog. The
synthesis routine does **not** silently include or exclude — both
would produce surprising fixture-resolution behavior. Hard reject
is the safe v0.1.0 surface.

If a dep is in **both** top-level `[dev-dependencies]` AND
`[target.<cfg>.dev-dependencies]` (cargo allows this; the cfg
table is merged on top for matching targets), the promotion uses
the top-level entry and leaves the cfg-table entry alone. This is
deferred behavior — the merged-spec semantics are subtle enough to
warrant deferral until a pilot needs it.

### §3.4 Step 4 — move the entries

For each name validated in §3.2 + processed in §3.3:

1. Read `value["dev-dependencies"][name]` → `entry`.
2. Insert `value["dependencies"][name] = entry` (with any §3.3
   transformations applied).
3. Remove `value["dev-dependencies"][name]`.

This preserves the entry's subtree verbatim except for §3.3
transformations (path absolutization, `optional = false` flip).

The `[dependencies]` table is created if absent (defensive: a
manifest with no `[dependencies]` is a valid edge case for
proc-macro crates).

### §3.4.bis Step 4.bis — overlay-vs-workspace invariant

After Step 4, the overlay's TOML is structurally:

```toml
[package]  # verbatim from adopter
name = "..."  # the same name as the adopter
version = "..."  # same

[lib]
# verbatim from adopter (no [lib] crate-type rewrite — already correct)

[dependencies]
# adopter's [dependencies] + the promoted entries

# [dev-dependencies] — adopter's, minus the promoted entries

# every other key — verbatim
```

**Invariant:** the overlay's `[package].name` is **identical** to the
adopter's. This is intentional. The overlay is a same-crate
re-shaping, not a sibling crate. Cargo's `-p <name>` selector in
`src/dylib.rs:95-96` resolves to the overlay's package (which has
the same `name`); the adopter's package becomes uninvocable from the
overlay-resolved workspace ONLY for the duration of this `cargo
rustc` invocation, which is scoped to a single dylib build. No other
`cargo` invocation in the session uses the overlay's manifest.

**Why this works (cargo workspace resolver):** cargo's workspace
walk-up from `<adopter>/target/lihaaf-build/lihaaf-overlay/Cargo.toml`
lands on the adopter's `Cargo.toml` or its ancestor workspace root.
The overlay's `[package].name` is the same as the adopter's. Cargo's
workspace `members` array (if the ancestor is a workspace) lists the
adopter — not the overlay — but cargo's `-p` selector resolves by
**name** match, not by path identity, and the overlay is consulted
because of the `--manifest-path` flag. The overlay's manifest path
is the explicit target; cargo uses it directly.

**Mechanically verified by:** the compat-mode overlay
(`compat/overlay.rs:556-878`) uses the exact same pattern — staged
overlay at `<target>/lihaaf-overlay/Cargo.toml` with the same
`[package].name` as upstream, then `cargo rustc -p <name>
--manifest-path <staged>` — and that ships in v0.1.0-beta.10 (CI
green across cxx/serde_json/anyhow/thiserror pilots).

### §3.5 Step 5 — absolutize path-bearing keys

Call the shared `manifest_overlay::absolutize_path_bearing_keys(top,
adopter_dir, adopter_manifest_path)`. This handles:

- `[lib] path` — adopter's `src/lib.rs` etc.
- `[package] build` — adopter's `build.rs` if present.
- `[[bin/example/test/bench]] path` — adopter's auto-discovered targets.
- `[dependencies].path` (now including the promoted entries that came
  in with relative paths) — see §3.3.2.
- `[dev-dependencies].path` (those that did NOT get promoted).
- `[build-dependencies].path`.
- `[target.<cfg>.{dependencies,dev-dependencies,build-dependencies}].path`.
- `[workspace] members / exclude / default-members` (no-op for the
  dev-deps overlay's adopter manifest since these only appear on
  workspace roots; the helper is no-op when the keys are absent).

**Critical:** the dev-deps overlay does **NOT** call
`override_workspace_inheritance` (the compat-overlay's Branch 1-5
workspace-resolution logic). The dev-deps overlay inherits the
adopter's workspace transparently — see §3.3.1 and §3.4.bis.

### §3.6 Step 6 — serialize + write atomically

Call `manifest_overlay::serialize_canonical(&value)` to produce
deterministic bytes. Write atomically using `util::write_file_atomic`
(the same helper compat-overlay uses, `compat/overlay.rs:846`).

The overlay path is `overlay_dir.join("Cargo.toml")` (cargo requires
the filename to be exactly `Cargo.toml`; see `compat/overlay.rs:821-825`).
`write_file_atomic` calls `create_dir_all` on the parent, so the
overlay directory is created lazily on first use.

### §3.7 Step 7 — idempotent rerun guard

Identical to `compat/overlay.rs:830-847`:

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

The mtime is preserved on a clean-rerun. This is load-bearing for the
§4 cache-safety contract.

### §3.8 Failure-mode summary

Each Codex-enumerated failure mode → §3 step + policy:

| Failure mode                                 | §  Section | Policy                                              |
|----------------------------------------------|-----------|-----------------------------------------------------|
| `workspace = true` dev-deps                  | §3.3.1   | Preserve shorthand; inherit through cargo walk-up   |
| Path dev-deps                                | §3.3.2   | Absolutize via shared helper                        |
| Git dev-deps                                 | §3.3.3   | Verbatim copy                                       |
| `[patch]` sections                           | §3.3.4   | Don't touch; inherit via workspace walk-up          |
| `dep:` feature syntax + optional             | §3.3.5   | Promote with `optional = false`                     |
| Renamed via `package = "..."`                | §3.3.6   | Verbatim move; preserve rename for `--extern`       |
| cfg-gated dev-deps                           | §3.3.7   | Explicit REJECT v0.1.0; v0.2 backlog                |
| Adopter `[dev-dependencies]` typo            | §3.2     | REJECT — named in lihaaf but missing in manifest     |
| Adopter has dep in BOTH `[dependencies]` AND `[dev-dependencies]` | §3.2 | REJECT — cargo rejects this; we surface eagerly |
| `build_targets` set but `dev_deps` empty     | §1.2 r3 (config-parse) | REJECT — no-op overlay surface       |
| `build_targets` contains a non-"tests" value | §1.1 (config-parse) | REJECT — v0.1.0 surface                |

---

## §4 Idempotency + cache safety

### §4.1 Determinism contract

The overlay manifest is **content-deterministic**: given the same
adopter `Cargo.toml` bytes + the same `dev_deps` list (resolved in
the same order), `manifest_overlay::serialize_canonical` produces the
same bytes. This must hold for cargo's fingerprint to hash to the
same value across runs, preserving cache hits.

Concretely:

- TOML parse produces a `toml::Value`, which is a `BTreeMap`-backed
  table at every level → key ordering is canonical (lexicographic).
- The Step 3-4 transformations are pure functions of the parsed
  value; no system-clock, no env-var, no random seed.
- `serialize_canonical` (existing in `compat/overlay.rs`) emits a
  deterministic byte sequence — `compat/overlay.rs:817` and its
  shipped use in v0.1.0-beta.10 are the precedent.

### §4.2 Cargo fingerprint stability

Cargo's per-package fingerprint includes:

- RUSTFLAGS (we set `-C prefer-dynamic` deterministically).
- Manifest bytes (the overlay).
- Feature flags (passed verbatim from `params.features`).
- Source tree mtimes (the adopter's source files; unchanged by the
  overlay).

The overlay's manifest mtime is preserved by §3.7's idempotent rerun
guard. The first invocation writes the overlay; subsequent
invocations skip the write when bytes match, so mtime is preserved.

**Cold-cache invariant:** the first invocation with overlay
enabled costs one fresh cargo fingerprint (the overlay's manifest is
new to cargo). Subsequent invocations with the same input hit the
cargo cache.

**Cache-thrashing avoidance:** the overlay dir lives under
`<workspace>/target/lihaaf-build[-<suite>]/lihaaf-overlay/`, a path
the existing `build_dir_for_suite` (`src/dylib.rs:388-393`) chose to
isolate from the adopter's own cargo target dir. The same per-suite
target dir scoping that already exists for `--features` differences
also scopes the dev-deps overlay. No interaction with the adopter's
normal `cargo build` cache.

### §4.3 Interaction with per-suite resources (`src/dylib.rs:377-393`)

`build_dir_for_suite` per spec §3.6 returns:

- Default suite → `<workspace_target>/lihaaf-build`
- Named suite → `<workspace_target>/lihaaf-build-<suite>`

The overlay dir nested inside is:

- Default suite → `<workspace_target>/lihaaf-build/lihaaf-overlay/Cargo.toml`
- Named "spatial" → `<workspace_target>/lihaaf-build-spatial/lihaaf-overlay/Cargo.toml`

Each suite's overlay is independently materialized when that suite's
`build_targets` is non-empty. A multi-suite adopter with one
suite opted in and another opted out gets one overlay dir + one
non-overlay cargo invocation in the same session.

### §4.4 Cross-session idempotency

The same input produces the same overlay across:

- Repeated runs by the same adopter.
- Different machines (the path absolutization uses the adopter's
  local dir, so the absolute paths differ — but cargo's fingerprint
  is over manifest content, which differs only by path; we accept
  this as a documented limitation since lihaaf adopter runs are
  always machine-local, not shared across CI runners with a shared
  cache).
- Suite re-runs (the per-suite resource isolation is preserved).

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

### §5.2 Pilot adopter inventory

A full audit. Each adopter's relevant TOML lines verified.

| Adopter | Manifest path | `dev_deps` configured? | `build_targets` configured? | Effect on adopter |
|---------|--------------|------------------------|-----------------------------|--------------------|
| **lihaaf self** | `/home/tarunvir/projects/lihaaf/Cargo.toml:210-219` | No (omitted) | No (new field) | No-op. Default suite + `suite_demo` named suite both run via legacy path. |
| **lihaaf integration corpus** | `/home/tarunvir/projects/lihaaf/tests/integration_corpus/Cargo.toml:26-32` | No (omitted) | No (new field) | No-op. |
| **anyhow pilot** | `/home/tarunvir/projects/anyhow-lihaaf-pilot/Cargo.toml:48` | No ("fixtures only import from anyhow itself") | No (new field) | No-op. |
| **serde-json pilot** | `/home/tarunvir/projects/serde-json-lihaaf-pilot/Cargo.toml:44` | No ("fixtures only import from serde_json itself") | No (new field) | No-op. |
| **djogi** | `/home/tarunvir/projects/djogi/djogi-macros/Cargo.toml:117` | Yes (`["serde", "serde_json", "sassi", "uuid", "rust_decimal"]`) | No (new field) | **NO-OP.** The crates listed in `dev_deps` are all in djogi-macros' regular `[dependencies]` already (not `[dev-dependencies]`); djogi works via the legacy `dev_deps`-forwarding-only path. Verify: `grep -A2 "^\[dependencies\]" /home/tarunvir/projects/djogi/djogi-macros/Cargo.toml` should list these. **Implementer must verify in §12 step 1.** |
| **sassi** | `/home/tarunvir/projects/sassi/sassi-macros/Cargo.toml` | (verify) | No (new field) | Presumed no-op. **Implementer must verify in §12 step 1.** |
| **axum-macros pilot** | `/home/tarunvir/projects/axum-lihaaf-pilot/axum-macros/Cargo.toml:81, 124, 131` | Yes (`["axum-extra", "serde"]` on 3 suites) | No (new field, but THIS pilot needs to set it) | **REQUIRES setting `build_targets = ["tests"]` to land green.** This is the v0.1.0 GA-blocker pilot. Setting the new field on each affected suite (default, from_request, typed_path) unblocks the 24-fixture failure. |
| **cxx pilot** | (workspace-style; uses compat-mode in v0.1.0) | n/a (compat mode) | n/a | No interaction. Compat-mode driver writes synthetic metadata via `compat/overlay.rs:790`; that synthesis path is separate and unaffected. |
| **derive_more pilot** | (Round-2 pilot) | (verify) | No (new field) | Presumed no-op. **Implementer must verify in §12 step 1.** |

Adopters who SHOULD NOT set `build_targets`: djogi, sassi, anyhow,
serde_json, lihaaf-self, integration_corpus, derive_more. Setting
`build_targets = ["tests"]` for these adopters would be a no-op
(promoted entries are already in `[dependencies]`) but pays the
overlay-synthesis overhead unnecessarily. The user guide (§7) MUST
include a "when NOT to use" section.

### §5.3 Pre-existing `build_targets` usage check

Grep of every known adopter manifest:

```text
$ rtk grep -nE "build_targets" \
    /home/tarunvir/projects/{axum-lihaaf-pilot,anyhow-lihaaf-pilot,
        serde-json-lihaaf-pilot}/.../Cargo.toml \
    /home/tarunvir/projects/{djogi/djogi-macros,sassi/sassi-macros,
        lihaaf,lihaaf/tests/integration_corpus}/Cargo.toml
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
# DEFAULT: []. Opt-in. When non-empty, lihaaf synthesizes a same-crate
# overlay manifest that promotes the entries named in `dev_deps` from
# `[dev-dependencies]` into `[dependencies]`. This is required when
# fixtures `use` crates that are in the adopter's `[dev-dependencies]`
# rather than its `[dependencies]` — cargo's `--lib` does not compile
# dev-deps during the lihaaf dylib build. Only "tests" is accepted in
# v0.1.0. See §4.2.bis for the overlay-promotion mechanics.
build_targets = ["tests"]
```

**Amendment to §3.4 Validation rules (lines 421-453).** Add:

```text
- An entry in `build_targets` is not in the allowed set `{"tests"}`.
- `build_targets` is non-empty but `dev_deps` is empty (the overlay
  would be byte-identical to the adopter's manifest; the opt-in
  shape requires named dev-deps).
- An entry in `dev_deps` named for promotion via `build_targets =
  ["tests"]` is not present in the adopter's `[dev-dependencies]`
  table.
- A dev-dep listed in both `[dependencies]` and `[dev-dependencies]`
  (cargo itself rejects this; lihaaf surfaces a directed diagnostic).
- A dev-dep listed in `dev_deps` for promotion lives only under
  `[target.<cfg>.dev-dependencies]`. Conditional dev-dep promotion
  is deferred to v0.2.
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
### 4.2.bis Dev-deps overlay promotion

By default, the dylib build invokes:

    cargo rustc -p <dylib_crate> --lib --release --crate-type=dylib \
      --manifest-path <adopter-Cargo.toml> --target-dir <T>

`--lib` excludes `[dev-dependencies]` from the build per cargo's
documented semantics ("Dev-dependencies are not used when compiling a
package for building, but are used for compiling tests, examples, and
benchmarks"). The dev-deps rlibs never land in `<T>/release/deps/`, so
per-fixture rustc cannot resolve `--extern <dev-dep>` for them.

When the adopter's metadata sets `build_targets = ["tests"]`, lihaaf
synthesizes an **overlay manifest** at `<T>/lihaaf-overlay/Cargo.toml`
that is a verbatim copy of the adopter's `Cargo.toml` with the entries
named in `dev_deps` moved from `[dev-dependencies]` into
`[dependencies]`. The cargo invocation then runs against the overlay's
manifest path:

    cargo rustc -p <dylib_crate> --lib --release --crate-type=dylib \
      --manifest-path <T>/lihaaf-overlay/Cargo.toml --target-dir <T>

The overlay's `[package].name` matches the adopter's exactly; the
overlay sits inside the adopter's workspace via cargo's walk-up from
`<T>/lihaaf-overlay/`, so workspace inheritance (`{ workspace = true }`
deps, `[patch]` tables, `[workspace.dependencies]`) continues to
resolve.

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

But some adopters have fixtures that import crates from
`[dev-dependencies]` rather than `[dependencies]`. The canonical
example is `axum-macros`, whose `tests/from_request/pass/container.rs`
contains `use serde::Deserialize;` — and `axum-macros`' `Cargo.toml`
declares `serde` in `[dev-dependencies]`, not `[dependencies]`.

**Symptom:** the relevant fixtures fail with rustc `error[E0432]:
unresolved import 'serde'` (or `axum_extra`, etc.) on `use` lines for
crates in the consumer's `[dev-dependencies]`.

### Detection

From the consumer crate's root:

```bash
rg '^use ([a-z_]+)::' tests/ --no-filename | sort -u
```

Compare each `use <crate>::` against `[dependencies]` vs
`[dev-dependencies]` in `Cargo.toml`. Any name found ONLY in
`[dev-dependencies]` triggers this symptom.

If every name appears in `[dependencies]` (or as a sub-dep
transitively reachable via the dylib_crate), you are likely fine —
the legacy path works.

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
  rustc — same field, same semantics as before.
- `build_targets = ["tests"]` opts the suite into overlay promotion.
  lihaaf synthesizes an overlay manifest that moves the named
  `dev_deps` entries from `[dev-dependencies]` into `[dependencies]`
  for the single `cargo rustc` invocation that builds the dylib.

The opt-in is per-suite. `build_targets` does NOT inherit from the
default suite — adopters who want overlay promotion across all suites
must declare `build_targets = ["tests"]` per suite (same shape as
`features`). This is intentional: each suite compiles its own dylib
with its own build shape (see [[lihaaf-dev-deps-explicit-keep]] for
the explicit-config-first rationale).

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
2. For each suite with `build_targets = ["tests"]`, synthesizes an
   overlay manifest at `<workspace>/target/lihaaf-build[-<suite>]/lihaaf-overlay/Cargo.toml`.
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
  consumer's regular `[dependencies]` (or transitively reachable via
  it). Includes djogi, sassi, anyhow, thiserror, derive_more.

- The consumer crate has no `[dev-dependencies]` at all, or its
  `[dev-dependencies]` contains only crates fixtures don't import.

- The consumer is a workspace member running under compat mode
  (`cargo lihaaf --compat`) — compat-mode does not currently use
  the overlay-promotion path (the synthetic metadata defaults to
  `build_targets = []`).

The rule: enable `build_targets = ["tests"]` only when fixture
diagnostics show `unresolved import` errors against crates that
appear in `[dev-dependencies]` of the consumer.
```

Length of new section: ~75 lines markdown. Brings `docs/user-guide.md`
from 88 to ~163 lines.

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

### §8.2 Unit: overlay synthesis (`src/dev_deps_overlay.rs::tests`)

Inputs: synthetic adopter `Cargo.toml` strings + lists of dev_deps to
promote. Outputs: assert exact overlay byte content via inline
expected-string snapshots.

| Test | Input Cargo.toml shape | Promoted dev_deps | Expected overlay |
|------|------------------------|-------------------|------------------|
| `roundtrip_basic_dev_dep` | `[package] name = "X"; version = "0.1.0"; [dependencies] foo = "1"; [dev-dependencies] serde = "1"` | `["serde"]` | `foo + serde` both in `[dependencies]`; no `[dev-dependencies]` |
| `roundtrip_preserves_unpromoted_dev_deps` | Same + extra `tokio = "1"` in dev-deps | `["serde"]` | `serde` promoted; `tokio` remains in `[dev-dependencies]` |
| `roundtrip_promotes_renamed_dep` | `[dev-dependencies] serde-json = { package = "serde_json", version = "1" }` | `["serde-json"]` | Renamed entry moved verbatim |
| `roundtrip_promotes_path_dev_dep` | `[dev-dependencies] helper = { path = "../helper" }` | `["helper"]` | `path` absolutized against adopter dir |
| `roundtrip_promotes_workspace_true` | `[dev-dependencies] serde = { workspace = true }` | `["serde"]` | Shorthand `{ workspace = true }` preserved verbatim in `[dependencies]` |
| `roundtrip_promotes_optional_dep_flips_optional` | `[dev-dependencies] serde = { version = "1", optional = true }` | `["serde"]` | Entry moved with `optional = false` |
| `rejects_missing_dev_dep` | `[dev-dependencies] (none); [dependencies] foo = "1"` | `["serde"]` | Error: dev_deps[0]="serde" missing from [dev-dependencies] |
| `rejects_dep_in_both_tables` | `[dependencies] serde = "1"; [dev-dependencies] serde = "1"` | `["serde"]` | Error: serde in both tables (eager surface of cargo's own reject) |
| `rejects_cfg_gated_only_dev_dep` | `[target.'cfg(unix)'.dev-dependencies] serde = "1"; [dev-dependencies] (no serde)` | `["serde"]` | Error: cfg-gated dev-dep promotion deferred to v0.2 |
| `idempotent_rerun_same_input_same_bytes` | Synthesize twice with same input | (any valid) | Both outputs byte-identical |

### §8.3 Unit: shared helper extraction

Located in `src/manifest_overlay/mod.rs::tests` (the extracted shared
helper).

| Test | Behavior |
|------|----------|
| `absolutize_lib_path_handles_relative` | `[lib] path = "src/lib.rs"` → absolute form |
| `absolutize_dependencies_path` | `[dependencies] x = { path = "../x" }` → absolute |
| `absolutize_no_op_on_absolute_path` | `[lib] path = "/abs/lib.rs"` → unchanged |
| `serialize_canonical_deterministic` | Same `toml::Value` → same bytes across calls |
| `compat_overlay_byte_identical_after_extraction` | (regression) Run an existing compat-mode test and assert byte-identical output to pre-extraction baseline |

The last test is the load-bearing regression guard for §2.3's
extract-from-compat-overlay decision.

### §8.4 Integration (cargo-build-gated) — `tests/dev_deps_overlay_integration.rs`

Per [[lihaaf-no-local-binary-builds]], gated behind
`#[cfg(feature = "cargo-build")]`. Runs in CI only.

| Test | Setup | Assertion |
|------|-------|-----------|
| `axum_macros_minimal_repro` | Synthetic 5-fixture adopter that mirrors axum-macros' shape: 1 dylib_crate, 1 dev-dep (`serde`), 3 compile_pass fixtures with `use serde::Deserialize`, 2 compile_fail fixtures with intentional errors | `cargo lihaaf` exits 0; all 5 fixtures dispatch; the synthesis-driven `cargo rustc` invocation completes; `deps_dir` contains `libserde-*.rlib`; per-fixture stderr does NOT contain "unresolved import" |
| `build_targets_omitted_byte_identical_baseline` | Two-fixture adopter with no dev-deps usage, run twice (once with the new field omitted, once with `build_targets = []`) | Resulting `target/lihaaf/manifest.json` `metadata_snapshot` differs only by the new key (verifies the §5.1 byte-identical contract) |

### §8.5 Regression: byte-identical for non-overlay adopters

A new test in `src/config.rs::tests` or
`src/dev_deps_overlay.rs::tests`:

| Test | Behavior |
|------|----------|
| `build_targets_omitted_no_overlay_dir_created` | Parse a minimal Cargo.toml without `build_targets`; assert that the `Suite.build_targets` is `vec![]` and that `BuildParams.build_targets` would be `&[]` for the suite; assert that **no** code path inside `dev_deps_overlay::synthesize_overlay_manifest` is invoked (via a unit-level branch test or a mock) |
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
rtk cargo clippy --all-features --jobs 2 -- -D warnings
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
- The 3 subprocess-spawning integration binaries
- `cargo lihaaf --compat`
- `cargo build --release`

The CI workflow runs `cargo test --all-features` on every PR (per the
existing pipeline). The new cargo-build-gated integration test in §8.4
runs there, not locally.

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
to `[dependencies]` in their fork (which would be fork pollution, not
acceptable per locked constraints), the dylib build would compile
those crates as part of the normal dependency graph. Estimate: +12-15s
to the dylib build phase, then 93 fixtures all pass. Total ~75-80s.

**Overlay-promotion (proposed beta.11+):**

The synthesis itself is ~1-5ms (read 5KB Cargo.toml, parse, two
`BTreeMap` mutations, serialize, atomic write). Negligible.

The cargo rustc invocation now compiles the promoted dev-deps as part
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
  adopter inputs → same overlay bytes → same cargo fingerprint.
- The idempotent write guard (§3.7) preserves mtime on byte-identical
  output → cargo's fingerprint detector reports "fresh."
- Subsequent reruns hit cargo's incremental cache for both the dylib
  AND the promoted dev-deps.

Hit-rate expectation: 100% on rerun against an unchanged source tree.
The cache miss occurs only on:

- First run after enabling `build_targets` (one-time cost).
- Any change to the adopter's `Cargo.toml` (forces re-synthesis).
- Any change to `dev_deps` (also forces re-synthesis).
- Any change to the adopter's source files (cargo's normal mtime-based
  invalidation; orthogonal to lihaaf).

### §10.3 Per-fixture cost — unchanged

The per-fixture rustc invocation is unchanged. The `--extern` flag
emission at `src/worker.rs:1003-1008` continues to point at the same
`deps_dir` paths. The fixture's compilation work is identical to what
it would be in any other adopter.

### §10.4 Comparison to the rejected POC's two-phase approach

The POC at `/tmp/lihaaf-poc-phase0/src/dylib.rs:89-141` ran TWO
cargo invocations: `cargo build --tests` (phase 0) followed by
`cargo rustc --lib` (phase 1). The two-invocation shape has a
documented fingerprint hazard (phase 0's `--tests` includes harness
test-runner overhead that the phase 1 `--lib` doesn't, so the
fingerprint computation across the two phases is subtly different,
producing inconsistent cache state across reruns).

Candidate E avoids the two-phase shape entirely: **one cargo
invocation, one resolver graph, one RUSTFLAGS value, one
fingerprint.** Cold-cache cost is comparable to the POC's
two-invocation path (both compile the same dev-deps); warm-cache
cost is strictly lower (Candidate E has zero fingerprint
inconsistency, so the second-rerun cache hit is reliable; the POC's
cache hit was probabilistic).

### §10.5 Defending the speed cost

The user's explicit priority is CI/benchmark wall-clock speed. The
overlay path's overhead vs. the legacy path:

- Synthesis: ~1-5ms. Negligible.
- Cargo fingerprint of the overlay's manifest: ~negligible (cargo
  hashes the bytes; the bytes differ from the adopter's manifest by
  one `[dependencies]` entry per promoted dev-dep).
- Compilation of the promoted dev-deps: this is the
  cargo-actually-compiles-the-crate cost. Same as if the adopter had
  the dev-deps in `[dependencies]` (which is the conceptual baseline).
  Quantified at ~+12-15s for axum-macros' 2 dev-deps.

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

Pre-Codex user decisions (locked 2026-05-19 before dispatch):

- **OQ-1** (`build_targets` inheritance) → **LOCKED REPLACE** (§11.1)
- **OQ-2** (`optional = true` dev-dep policy) → **OPEN for Codex** (§11.2)
- **OQ-3** (orthogonality) → **LOCKED orthogonal** (§11.3)
- **OQ-4** (`[patch]` interaction) → **CODEX DECIDES; user-preferred path DEFERRED to v1.0.0** (§11.4, GH-tracked per §12.8)

Codex round-1 will critique all four. The locked decisions are
user-authorized design choices; if Codex pushes back on a locked
decision, the planner / orchestrator escalates to the user before
revising. The OPEN OQ-2 is genuinely open and Codex's preferred
shape should be adopted (subject to user veto). OQ-4 is also Codex-
adjudicated: user prefers v0.1.0 ships with warning + GH-tracked
v1.0.0 follow-up (see §12.8), but Codex may BLOCK the deferral and
demand inline resolution. Either Codex outcome on OQ-4 is acceptable
to the user.

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

**Codex note:** if Codex pushes for INHERIT during adversarial review,
the rationale to defend REPLACE is the explicit-config-first ethos
([[lihaaf-dev-deps-explicit-keep]]) — not a craft question, a
user-locked architectural decision.

### §11.2 OQ-2 — `optional = true` dev-dep policy

§3.3.5 recommends flipping `optional = true` to `optional = false` on
promotion. But this changes the semantics of a manifest that uses
`[features].my-feat = ["dep:serde"]` patterns. Specifically:

- Pre-promotion: `serde` is enabled only when `my-feat` is active.
- Post-promotion: `serde` is enabled unconditionally (because
  `optional = false` and it's in `[dependencies]`).

This **may** subtly change the dylib's compilation behavior if the
adopter's code uses `#[cfg(feature = "my-feat")]` to gate code
referencing `serde`. The promoted-unconditional shape would compile
the `serde`-dependent code; the original wouldn't (when `my-feat`
is off).

**Alternative policy:** REJECT optional dev-deps when promoted. The
adopter must promote a non-optional shape, or skip promotion for the
optional dep entirely.

**Recommendation:** v0.1.0 ships the `optional = false` flip with a
loud warning in §6.1 / §7 ("optional dev-deps are unconditionally
enabled when promoted; verify your `cfg(feature = ...)` gates").
v0.2 may add a REJECT path if a pilot surfaces breakage.

**For Codex:** is there a third option I missed? A version-bound
"flip only if no `dep:<name>` feature reference exists in
`[features]`"?

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

**Codex note:** if Codex pushes for inference / single-field
consolidation during adversarial review, the rationale to defend
orthogonality is the same as OQ-1 — explicit-config-first is a
user-locked architectural decision.

### §11.4 OQ-4 — `[patch]` table interaction with promotion — **CODEX DECIDES; user-preferred path DEFERRED to v1.0.0**

**Status:** User on 2026-05-19 prefers deferral to v1.0.0 roadmap but
delegates final adjudication to Codex adversarial review. Per
[[no-unilateral-deferral]] this is an explicit user-authorized
deferral path, not a unilateral shelve.

**Two outcomes possible from Codex review:**

1. **Codex ALLOWs the deferral** → plan ships as written with §12.8
   creating the GH issue + §7 user-guide warning. v0.1.0 cannot ship
   without §12.8's GH issue filed.
2. **Codex BLOCKs the deferral** (i.e. demands "carry verbatim"
   proven or REJECT-path added pre-v0.1.0) → planner re-dispatch to
   close OQ-4 in this plan body; §12.8 may collapse into the relevant
   implementation step.

**The deferred risk:** `[patch]` table on adopter's `Cargo.toml`
interacting with overlay promotion. §3.3.4 ships "carry verbatim"
policy — don't touch `[patch]`, let cargo's walk-up resolve it. This
works for the common case (adopter doesn't patch promoted dev-deps).

```toml
# adopter's Cargo.toml (the deferred-risk shape)
[dependencies]
foo = "1.0"

[dev-dependencies]
serde = "1.0"

[patch.crates-io]
serde = { path = "./forks/serde" }
```

Pre-promotion: cargo's resolver applies the patch to `serde`
references in `[dev-dependencies]`. The patch fires.

Post-promotion (overlay): same patch fires for `serde` references in
`[dependencies]`. The patch SHOULD fire identically.

**Both should work — but the patch correctness invariant is not
proven** for the case where the same crates-io.X source-id appears
across `[dependencies]` (overlay) AND `[dev-dependencies]` (baseline)
of the same package with a `[patch.crates-io]` redirect.

**v0.1.0 mitigation:** the user guide (§7) MUST document that adopters
with `[patch.crates-io]` patches on their adopter manifest targeting
crates listed in `dev_deps` should validate the patch fires correctly
against their dylib build before relying on `build_targets = ["tests"]`.

**v1.0.0 work tracked in GH issue (per §12.8):**

- Reproduce the deferred-risk shape with a synthetic pilot or
  `derive_more` if applicable.
- Cite cargo's documented source-id resolution rules.
- Either (a) prove the "carry verbatim" policy correct, or (b) add a
  REJECT path at synthesis for adopter-local `[patch]` sections
  targeting promoted dev-deps.

**Rationale (user 2026-05-19):** "sure codex decides. I am comfortable
deferring to post v0.1.0 as our v1.0.0 roadmap can capture it." Cargo
resolver behavior here is unproven; user-preferred path is v0.1.0
ships with documented user-guide warning + GH-tracked v1.0.0 follow-up.
If Codex ALLOWs that shape, deferral stands; if Codex BLOCKs, planner
closes OQ-4 inline.

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
3. Update `src/compat/overlay.rs` to import from
   `crate::manifest_overlay::{...}`.
4. Add `src/manifest_overlay/mod.rs` to `src/lib.rs` module
   declarations.
5. Add the §8.3 regression test: existing compat-mode test produces
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

### §12.3 Step 3 — add `dev_deps_overlay` module

**Branch:** `feat/dev-deps-overlay-module`. Stacked on Step 2.

1. Create `src/dev_deps_overlay.rs` with the §3 synthesis algorithm.
2. The module exports `synthesize_overlay_manifest`.
3. Use the §12.1 shared `manifest_overlay::*` helpers.
4. Add §8.2 unit tests (overlay-synthesis round-trip).

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
4. Update the session orchestrator (`src/session.rs` or equivalent)
   to populate the new `BuildParams` fields from `Suite`.
5. Add §8.5 regression test: empty `build_targets` does NOT call
   into `synthesize_overlay_manifest`.

**Verification (§9):** all four commands pass.

**Adversarial-review trip-wire:** if any existing test that doesn't
opt into `build_targets` fails → BLOCK. The byte-identical
contract must hold.

### §12.5 Step 5 — cargo-build-gated integration test

**Branch:** `feat/dev-deps-overlay-integration-test`. Stacked on
Step 4.

1. Add `tests/dev_deps_overlay_integration.rs`.
2. Gate behind `#[cfg(feature = "cargo-build")]`.
3. Add the §8.4 test cases.
4. Verify the test passes in CI.

**Verification (§9):** all four commands pass. Per
[[lihaaf-no-local-binary-builds]], the new test is NOT run locally;
CI runs it.

### §12.6 Step 6 — spec + user-guide amendments

**Branch:** `docs/dev-deps-overlay-spec-and-guide`. Stacked on
Step 5 (or rebased onto `docs/user-guide` for the user-guide diff).

1. Apply the §6 spec amendments to `docs/spec/lihaaf-v0.1.md`.
2. Apply the §7 user-guide amendments to `docs/user-guide.md` on
   the `docs/user-guide` branch.
3. Add a CHANGELOG.md entry for the new field.
4. Add a v0.1.0 entry in the changelog naming the feature.

**Verification (§9):** all four commands pass. `cargo doc` should
build the new rustdoc on `Suite.build_targets`.

### §12.7 Step 7 — enable on axum-macros pilot fork

**Branch:** (in the `axum-lihaaf-pilot` repo, not lihaaf). Not part
of this dispatch; the pilot fork is a separate repository.

1. Add `build_targets = ["tests"]` to the default suite + each named
   suite that has `dev_deps`.
2. Run the pilot's CI; verify all 93 fixtures pass.
3. Measure the cold-cache wall-clock for the §10 estimate.
4. If measurement diverges materially from §10 estimate, update
   §10.

### §12.8 Step 8 — file v1.0.0 GH issue for OQ-4 `[patch]` deferral

**MANDATORY: v0.1.0 cannot ship without this issue filed.** Per
[[no-unilateral-deferral]] this is the GH-tracked record of the
user-authorized §11.4 deferral.

1. Open a GH issue on `lihaaf-rs/lihaaf` titled exactly:
   `v1.0.0: prove or reject [patch.crates-io] interaction with overlay-promoted dev_deps`
2. Issue body MUST include:
   - Link to `docs/spec/dev-deps-overlay-promotion-plan-2026-05-19.md#114-oq-4--patch-table-interaction-with-promotion--deferred-to-v100`
   - The deferred-risk shape from §11.4 (verbatim toml example).
   - The three sub-tasks from §11.4: (a) reproduce risk shape with
     synthetic pilot or `derive_more`, (b) cite cargo's documented
     source-id resolution rules, (c) either prove "carry verbatim"
     correct OR add a REJECT path at synthesis.
   - Label: `v1.0.0`, `deferred-from-v0.1.0`, `overlay-promotion`.
3. Reference the issue number back into §11.4 as a final paragraph:
   "Tracked in GH issue #<N>."
4. Commit the §11.4 backlink update as a follow-up commit on the
   same plan branch.

**Verification (§9):** the GH issue exists and is reachable; the
plan doc has been updated with the issue number.

**Adversarial-review trip-wire:** if v0.1.0 ships without this GH
issue filed → BLOCK at release-gate.

---

## §13 Sanity checks

### §13.1 Locked-constraint compliance

| Locked constraint | Compliance |
|-------------------|-----------|
| 1. CLI-only, no library API | No new public API exposed beyond the existing `cargo-lihaaf` binary. |
| 2. Dylib-only is design DNA | The change keeps the dylib build invocation shape; only the manifest source changes. |
| 3. Explicit > implicit | `build_targets` is opt-in; omitted = byte-identical legacy behavior. |
| 4. CI/benchmark wall-clock priority | §10 quantifies; the change preserves the single-cargo-invocation amortization. |
| 5. UNION in single dylib | Single cargo rustc; no two-phase fingerprint. |
| 6. `dev_deps` semantics unchanged | `dev_deps` is still an explicit allow-list; `build_targets` is orthogonal. |
| 7. Backwards-compat for existing adopters | §5.2 verified per-adopter; all listed adopters remain no-op. |
| 8. No fork pollution | Adopters configure on their own lihaaf-converted branch; no upstream Cargo.toml edits. |
| 9. Quality > velocity | Every Codex-enumerated edge case has an explicit policy. |

### §13.2 Memory ledger compliance

- [[lihaaf-no-local-binary-builds]]: §8.4's integration test is
  `cargo-build`-gated, runs in CI only. §9's verification commands
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

---

## §14 Out-of-scope (NOT in this plan)

For Codex's review: these are intentionally NOT in this plan and
should not be raised as gaps.

- `"examples"` and `"benches"` as `build_targets` values. Deferred
  to v0.2+.
- Per-suite `[patch]` injection. Deferred (workspace-root `[patch]`
  inheritance handles every pilot-known case).
- Auto-discovery of `dev_deps` from the adopter's `[dev-dependencies]`
  table. Locked-rejected; the user explicitly opted out.
- Compat-mode (`cargo lihaaf --compat`) using `build_targets`.
  Deferred; compat-mode's synthetic metadata defaults to empty
  `build_targets`.
- Removing the existing `dev_deps` field. Out of scope; backwards
  compat invariant.
- Refactoring `compat/overlay.rs` beyond the §12.1 helper extraction.

---

## §15 Adversarial-review checklist for Codex

Codex should specifically verify:

1. **File:line accuracy.** Every cited line in §2 ("Source-level
   changes needed") matches the actual line in `main` HEAD as of
   2026-05-19. (Previous plan was BLOCKed on this.)
2. **§3 algorithm completeness.** Every Codex-enumerated failure
   mode has a documented policy.
3. **§5 adopter inventory completeness.** All known adopters checked,
   none would break.
4. **§8 test coverage.** Each behavioral claim has a corresponding
   unit or integration test.
5. **§10 speed claim defensibility.** The ~+12-15% cold-cache
   estimate is grounded in axum-macros' specific shape; warm-cache
   100% hit-rate is grounded in the determinism contract.
6. **§11 OQs are real.** Not fishing-expedition placeholders; each
   names a design choice the planner could not unilaterally lock.
7. **§12 step independence.** Each step's tests pass at that step's
   exit. No step's tests require a later step's code.
8. **No silent locked-constraint violation.** §13.1 enumerates all
   nine; verify each line.

---

**End of plan.**
