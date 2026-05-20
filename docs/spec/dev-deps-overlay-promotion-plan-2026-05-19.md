# Plan: staged suite workspace collector for fixture dev-deps

**Date:** 2026-05-20
**Target milestone:** v0.1.0
**Working branch:** `plan/dev-deps-overlay-promotion`
**Status:** locked design; pre-implementation review fixes are
incorporated

This plan replaces the prior dylib crate mutation design. Opted-in
suites no longer add `dev_deps` to the dylib crate manifest. Instead,
lihaaf synthesizes a temporary suite workspace and asks Cargo to build
the staged dylib package and a synthetic collector package in one
resolver graph for that suite.

The locked goals are:

- no promoted `dev_deps` are inserted into the dylib crate manifest;
- the staged dylib package emits both `rlib` and `dylib` from the real
  source via an absolute `[lib].path`;
- the collector package depends on explicit metadata-side
  `[dev-dependencies]` entries named by the resolved suite `dev_deps`;
- path dependencies that point back to `dylib_crate` resolve to the
  staged dylib member, so fixtures see one `dylib_crate` identity;
- each opted-in suite gets one Cargo resolver graph, never one Cargo
  build per fixture;
- when `build_targets` is omitted or empty, the existing direct dylib
  path is byte-stable.

This plan reflects the current implementation direction for the
`build_targets` suite workspace path.

---

## §0 Problem

### §0.1 Why current lihaaf misses fixture dev-deps

The existing stage-4 dylib build uses `src/dylib.rs::build`:

```text
RUSTFLAGS="-C prefer-dynamic" cargo rustc -p <dylib_crate> \
  --lib --release --crate-type=dylib \
  --message-format=json-render-diagnostics \
  --manifest-path <metadata-Cargo.toml> --target-dir <suite-target>
```

Cargo does not build `[dev-dependencies]` for `cargo rustc --lib`.
Fixtures can still list `dev_deps` in lihaaf metadata, and the worker
will later try to forward those crates as `--extern`, but the rlibs are
missing from `<suite-target>/release/deps`.

The v0.1.0 blocker is the axum-macros pilot:

- metadata crate: `axum-macros/Cargo.toml` in the axum-lihaaf-pilot repo;
- `dylib_crate = "axum"`;
- default suite has `dev_deps = ["axum-extra", "serde"]`;
- named suites `from_request` and `typed_path` restate the same
  `dev_deps`;
- fixtures import `serde::Deserialize` and `axum_extra::*`.

Building only `axum` as a dylib skips `axum-macros`'s dev-deps, so the
fixture rustc phase cannot resolve them.

### §0.2 Why dylib crate mutation is rejected

The rejected design put metadata-side dev-deps into the staged dylib
manifest. That fails two axum-shaped classes:

1. `serde` already exists in `axum` as an optional regular dependency
   with `dep:serde` feature edges. Replacing it with a non-optional
   metadata-side dev-dep can make Cargo reject the manifest.
2. `axum-extra` has feature edges that can depend back on `axum`.
   Making `axum` depend on `axum-extra` creates a back-edge into the
   dylib crate and can produce separate incompatible `axum` identities.

The new plan does not try to make fixture-only crates normal
dependencies of the dylib crate. It builds them as dependencies of a
collector package that lives beside a staged dylib package in the same
temporary workspace graph.

### §0.3 Locked design summary

For a suite with resolved `build_targets = ["tests"]`:

1. Compute the suite target dir exactly as today:
   `target/lihaaf-build/` for the default suite and
   `target/lihaaf-build-<suite>/` for named suites.
2. Synthesize a temporary workspace under that target dir:
   `<suite-target>/lihaaf-suite-workspace/`.
3. Add a staged member for `dylib_crate`. Its manifest is based on the
   real dylib manifest, but its library target points at the real source
   with absolute `[lib].path` and `crate-type = ["rlib", "dylib"]`.
4. Add a synthetic collector member. Its normal `[dependencies]` table
   contains exactly the resolved suite `dev_deps`, copied from the
   metadata crate's top-level `[dev-dependencies]` table.
5. Stage or rewrite path dependencies as needed so any selected
   dependency path that targets `dylib_crate` points at the staged
   dylib member.
6. Run one Cargo build for that suite, selecting both packages in the
   staged workspace.
7. Parse the dylib artifact from Cargo JSON output, copy or symlink it
   to the managed lihaaf path, and keep using the staged target
   `release/deps` directory for worker extern resolution.

Non-opted suites do none of this and continue through the existing
`src/dylib.rs::build` path.

---

## §1 User-Facing Semantics

### §1.1 New field

```toml
[package.metadata.lihaaf]
dylib_crate = "axum"
extern_crates = ["axum", "axum-macros"]
features = ["macros"]
dev_deps = ["axum-extra", "serde"]
build_targets = ["tests"]
```

**Type:** array of strings.
**Default:** `[]`.
**Allowed values in v0.1.0:** exactly `"tests"`.

`build_targets = ["tests"]` means: "for this suite, build the
fixture-only dependency graph into the suite's lihaaf target dir before
fixture rustc runs." It does not mean: "make these crates dependencies
of the dylib crate."

### §1.2 Suite inheritance

`dev_deps` keeps the current INHERIT semantics. A named suite that
omits `dev_deps` receives the default suite's resolved `dev_deps`.

`build_targets` uses REPLACE semantics. A named suite that omits
`build_targets` receives `[]`, even if the default suite opts in. This
matches fields that control per-suite build shape, such as `features`.

Validation must run after suite resolution:

- first build `Suite` values with inherited `dev_deps`;
- then validate each suite's final `(build_targets, dev_deps)` pair;
- never validate an opted-in named suite against raw
  `RawSuite.dev_deps`, because omitted raw `dev_deps` may still resolve
  to non-empty inherited values.

### §1.3 Truth table

| Final `build_targets` | Final `dev_deps` | Behavior |
|-----------------------|------------------|----------|
| omitted or `[]` | omitted or `[]` | default dylib build; no collector |
| omitted or `[]` | `["a", "b"]` | default dylib build; worker still forwards `--extern a` and `--extern b` from the existing deps dir |
| `["tests"]` | omitted or `[]` | reject after suite resolution; there is no dependency graph to collect |
| `["tests"]` | `["a", "b"]` | synthesize and build the staged suite workspace once for the suite |
| any unknown value | any | reject with a directed config diagnostic |
| `["tests"]` | name absent from metadata crate `[dev-dependencies]` | reject before any Cargo build |

### §1.4 Explicit allow-list

The collector only depends on names present in the final suite
`dev_deps` array. Each name must be present in the metadata crate's
top-level `[dev-dependencies]`. lihaaf does not scan fixture source and
does not auto-promote every dev-dependency.

For v0.1.0, only the top-level `[dev-dependencies]` table is in scope.
If a name is present only under `[target.'cfg(...)'.dev-dependencies]`,
reject it with a diagnostic naming the unsupported target-specific
table. Target-specific collector policy is a post-v0.1.0 extension.

If a dependency is renamed with `package = "..."`, the metadata key is
the `dev_deps` name and the package name is used only for Cargo
resolution. The implementation must add tests before enabling renamed
dev-deps in v0.1.0; otherwise reject renamed dev-deps with a diagnostic
that says extern alias support is not implemented for the collector.

---

## §2 Source-Level Changes

### §2.1 `src/config.rs`

Add `build_targets: Vec<String>` to `Suite`.

Add `build_targets: Option<Vec<String>>` to both raw metadata structs:

- `RawMetadata` for the default suite;
- `RawSuite` for named suites.

Construction rules:

- default suite: `raw.build_targets.clone().unwrap_or_default()`;
- named suite: `raw.build_targets.unwrap_or_default()`;
- do not inherit `build_targets` from the default suite.

Add `validate_build_targets_for_suites(&suites)` after:

- `build_default_suite`;
- every `finalize_named_suite`;
- `validate_unique_suite_names`;
- `validate_disjoint_fixture_dirs`.

The validator checks the final resolved `Suite` values:

- every target is exactly `"tests"`;
- duplicate target names are rejected;
- `build_targets` non-empty with `dev_deps` empty is rejected;
- `build_targets` empty with `dev_deps` non-empty is allowed for
  default-suite compatibility.

This placement closes the raw-vs-resolved validation drift called out
by review.

Validation must also cover direct serde construction paths. Either make
`Suite` deserialize through a checked raw shape, or introduce a
`BuildTargets` newtype with `#[serde(try_from = "Vec<String>")]` so
unknown or duplicate targets cannot bypass the parser by constructing a
`Suite` directly.

### §2.2 `src/session.rs`

Keep the current suite target-dir calculation:

```rust
let lihaaf_build_dir = dylib::build_dir_for_suite(workspace_target, &suite.name);
```

Then branch on the resolved suite:

- if `suite.build_targets.is_empty()`, call `dylib::build` exactly as
  today;
- if `suite.build_targets == ["tests"]`, call the new staged workspace
  builder and receive the same `BuildOutput` shape.

The staged builder returns:

- `cargo_dylib_path`: the dylib artifact emitted by Cargo from the
  staged dylib package;
- `deps_dir`: `<suite-target>/release/deps`;
- `invocation`: the exact Cargo command and relevant env for
  diagnostics.

Everything after the build result stays the same:

- managed dylib copy or symlink;
- manifest refresh;
- fixture discovery;
- `worker::resolve_extern_paths(&build_out.deps_dir, extra_names)`;
- worker pool dispatch.

This keeps the behavior change scoped to opted-in suites and preserves
the default-path byte shape when `build_targets` is absent.

### §2.3 New module `src/suite_workspace`

Add a module responsible for:

- reading metadata and dylib manifests;
- resolving the dylib crate manifest;
- synthesizing the temporary suite workspace;
- staging package manifests and path dependency rewrites;
- running Cargo for opted-in suites;
- parsing the staged Cargo JSON output into `dylib::BuildOutput`.

Suggested public entry point:

```rust
pub struct BuildParams<'a> {
    pub dylib_crate: &'a str,
    pub suite: &'a Suite,
    pub metadata_manifest_path: &'a Path,
    pub target_dir: &'a Path,
    pub toolchain: &'a Toolchain,
}

pub fn build(params: &BuildParams<'_>) -> Result<dylib::BuildOutput, Error>;
```

Do not modify `src/dylib.rs::build` for the default path unless a shared
artifact parser is extracted without changing command shape or
diagnostics for non-opted suites.

### §2.4 `src/worker.rs`

No first-order worker change is intended. The worker already receives:

- the managed dylib path;
- the deps dir;
- `extern_crates`;
- `dev_deps`.

The collector path exists to ensure the deps dir contains rlibs for the
explicit dev-deps before `resolve_extern_paths` runs.

### §2.5 Manifest snapshots

`build_targets` is a per-suite metadata key, like `dev_deps`, `edition`,
and `allow_lints`. It is preserved via the `raw_metadata` round-trip and
does not require a dedicated `Manifest` struct field in
`src/manifest.rs`. Snapshot tests must cover:

- omitted field remains omitted in existing fixtures;
- opted-in field round-trips in `metadata_snapshot`.

---

## §3 Staged Suite Workspace Algorithm

### §3.1 Entry guard

The staged path runs only when the final suite has
`build_targets = ["tests"]`. Otherwise return to the current
`dylib::build` path before creating any directories, parsing extra
manifests, or touching target artifacts.

### §3.2 Read metadata-side dev-dep specs

Read the metadata crate's `Cargo.toml`, the same manifest path lihaaf
already receives from CLI resolution.

For each final `suite.dev_deps` name:

- find an entry in top-level `[dev-dependencies]`;
- clone the dependency spec as the collector dependency source;
- absolutize any `path` value against the metadata crate directory;
- preserve version, registry, git, branch, tag, rev, features,
  default-features, and package fields;
- preserve `workspace = true` only if the staged workspace also carries
  the needed `[workspace.dependencies.<name>]` entry from the ancestor
  workspace root;
- reject names missing from top-level `[dev-dependencies]`;
- reject names that exist only in target-specific dev-dependency tables;
- reject renamed dev-deps (`package = "..."`) until extern alias lookup
  is designed and tested;
- reject `optional = true` until collector feature-forcing is designed
  and tested.

The collector dependency table is metadata-sourced. It is not derived
from the dylib crate manifest.

### §3.3 Resolve the real dylib manifest

Resolution must be honest about the current helper scope:

- If the metadata package name equals `dylib_crate`, the metadata
  manifest is the dylib manifest.
- If they differ, resolve `dylib_crate` from the ancestor workspace.
- The existing `compat::overlay::resolve_workspace_member_manifest`
  only accepts a virtual workspace root: `[workspace]` without
  `[package]`.
- Do not claim package+workspace root support unless the implementation
  extends the resolver with tests. For v0.1.0, either use the helper
  within its virtual-workspace scope or reject unsupported workspace
  roots with a directed diagnostic.

The axum-macros path is a split-crate case: metadata lives in
`axum-macros`, while the dylib manifest is `axum/Cargo.toml`.

### §3.4 Create the temporary workspace

Create:

```text
<suite-target>/lihaaf-suite-workspace/
  Cargo.toml
  dylib/
    Cargo.toml
  collector/
    Cargo.toml
    src/lib.rs
  staged-path-deps/
    ...
```

The root `Cargo.toml` is a virtual workspace with `members` set to the
staged member directories. It carries resolver and workspace-root state
according to §3.8.

The suite workspace lives under the suite target dir, but Cargo's
target dir remains the suite target dir itself:

```text
cargo build ... \
  --manifest-path <suite-target>/lihaaf-suite-workspace/Cargo.toml \
  --target-dir <suite-target>
```

Artifacts therefore land in `<suite-target>/release/deps`, which is
the deps dir the worker already expects.

### §3.5 Stage the dylib package

The staged dylib manifest is based on the real dylib manifest, with
these transformations:

- keep `[package].name` equal to `dylib_crate`;
- preserve package version, edition, rust-version, features,
  dependencies, build-dependencies, target dependencies, lints, and
  package metadata unless a transformation below says otherwise;
- do not add metadata-side `dev_deps` to `[dependencies]`;
- make the library target explicit:

```toml
[lib]
path = "/absolute/path/to/real/dylib/src/lib.rs"
crate-type = ["rlib", "dylib"]
```

- if the real manifest already has `[lib]`, preserve its name and other
  non-conflicting fields, but replace or add `path` and ensure
  `crate-type` contains both `rlib` and `dylib`;
- disable auto-discovery for examples, tests, benches, and bins unless
  a future test proves they are needed for this build path;
- strip explicit `[[bin]]`, `[[example]]`, `[[test]]`, and `[[bench]]`
  array-of-tables from staged manifests. The staged member compiles only
  the library target lihaaf sets explicitly;
- absolutize path-bearing dependency fields against the real dylib
  package root unless they are rewritten to staged members.

v0.1.0 rejects staged packages that need build scripts. Reject if the
real manifest declares `build = "..."` or if the package root contains a
default `build.rs`; the diagnostic must name the offending manifest path
and explain that build-script staging is not implemented. Do not
silently drop a build script.

### §3.6 Stage the collector package

The collector package is synthetic and private:

```toml
[package]
name = "__lihaaf_dev_deps_collector"
version = "0.0.0"
edition = "2021"
publish = false

[lib]
path = "src/lib.rs"

[dependencies]
# exactly the resolved suite.dev_deps specs, metadata-sourced
```

`src/lib.rs` can be empty. Building the collector compiles its normal
dependencies, which is enough to populate the deps dir with rlibs for
fixture extern resolution.

If a metadata-side dev-dep spec contains `optional = true`, reject it
for v0.1.0 unless implementation adds a tested collector feature policy
that forces the dependency to build. The rejection is collector-local;
there is no dylib optional-dependency mutation in this design.

### §3.7 Rewrite path deps and back-edges

The identity invariant:

> Any rlib passed to fixtures that depends on `dylib_crate` must have
> been compiled against the same staged `dylib_crate` package that also
> emitted the managed dylib.

To enforce it:

1. Build a path-dependency staging plan from the collector dependency
   specs and the staged dylib manifest.
2. Conservatively stage every path dependency transitively reachable
   from the collector dependency specs, staged dylib manifest, or
   carried `[workspace.dependencies]` entries referenced by
   `workspace = true`, regardless of whether a feature or target cfg
   will activate it in the current build. Do not hand-roll a feature
   resolver in v0.1.0.
3. For every staged path dependency, read its manifest and inspect
   `[dependencies]`, `[build-dependencies]`, and target-specific
   dependency tables for further path dependencies.
4. If a path dependency entry resolves to a package whose
   `[package].name` equals `dylib_crate`, rewrite that entry to the
   staged dylib member.
5. If a path dependency manifest needs any rewrite, stage that package
   as a workspace member using the same source-path strategy as §3.5,
   then point dependents at the staged member.
6. Continue until no selected path dependency can point back to the
   original dylib package.

Registry and git dependencies are not rewritten. If a registry or git
dependency key, or its explicit `package = "..."`, names `dylib_crate`
inside the staged graph, reject it for v0.1.0 with a diagnostic naming
the manifest and dependency key. A path dependency that cannot be
analyzed safely is rejected with a diagnostic naming the manifest and
dependency key.

This is the class-level fix for the axum-extra back-edge: collector
`axum-extra` must not compile against the original `../axum` package
while fixtures link the managed staged `axum` dylib.

### §3.8 Workspace inheritance and override tables

The staged suite workspace must reuse compat-mode behavior accurately,
not the stale helper claims from the previous plan.

`apply_workspace_member_inheritance` behavior to reuse or extract:

- carries `[workspace.dependencies]`, `[workspace.package]`,
  `[workspace.lints]`, `[workspace.metadata]`, and resolver into the
  staged workspace view;
- carries workspace-root `[replace]` and `[profile.*]`;
- absolutizes workspace-root path-bearing entries against the
  workspace root;
- rejects member-local `[patch.*]` for every registry.

`[workspace.dependencies]` path entries are back-edge-rewritten only
when they are selected by a staged dependency via `workspace = true`.
Unused carried workspace dependency entries remain inert Cargo metadata;
v0.1.0 does not recursively analyze every unused path in the ancestor
workspace root.

Workspace-root `[patch.*]` carry-down must not call
`apply_self_patch_policy` wholesale. That helper is for compat-overlay
self-patching and may inject or remap `[patch.crates-io.<self>]`; the
suite workspace already has an explicit staged member and v0.1.0 rejects
registry/git self-edges instead of patching them.

Extract or implement a narrower helper that only:

- carries workspace-root patch registries into the staged workspace;
- absolutizes patch path entries against the workspace root;
- preserves non-self patch entries for every registry;
- rejects any patch entry whose key, or explicit `package = "..."`, is
  `dylib_crate` until a staged-workspace self-patch policy is designed
  and tested.

The path-back-edge rewrite in §3.7 is the only v0.1.0 mechanism that
preserves `dylib_crate` identity.

Member-local `[replace]` must receive an explicit policy before
implementation. The conservative v0.1.0 policy is to reject
member-local `[replace]` in staged members unless tests prove parity
with Cargo and compat-mode behavior.

### §3.9 Cargo invocation

The opted-in suite build uses one Cargo resolver graph:

```text
RUSTFLAGS="<prior> -C prefer-dynamic" cargo build \
  -p <dylib_crate> \
  -p __lihaaf_dev_deps_collector \
  --release \
  --message-format=json-render-diagnostics \
  --manifest-path <suite-workspace>/Cargo.toml \
  --target-dir <suite-target> \
  [--features <dylib_crate>/<suite-feature> ...]
```

Use package-qualified feature names when building from the virtual
workspace root so suite features apply to the staged dylib package.
Collector dependency features come from the metadata-side dev-dep specs.

Parse Cargo JSON messages and select the compiler artifact for
`dylib_crate` whose target crate types include `dylib`. Use that path
as `BuildOutput.cargo_dylib_path`.

### §3.10 Lockfile and package collision policy

The temporary workspace must not write the adopter's original
`Cargo.lock`. If an ancestor lockfile exists, copy it into the staged
workspace as a starting point; if Cargo must update it, updates stay
inside the temporary workspace.

v0.1.0 does not add `--offline`, `--frozen`, or `--locked` implicitly.
The staged builder inherits Cargo's ambient network and lock policy from
the user's environment and Cargo config, matching the default `cargo
rustc` path. If ambient offline mode fails because the staged collector
needs uncached registry data, surface Cargo's failure with a directed
lihaaf diagnostic that says the copied lockfile was kept inside the
temporary suite workspace and that the adopter must warm Cargo's cache
or disable the collector for that suite.

Avoid package collisions by ensuring the staged workspace graph sees
only one package source for `dylib_crate`: the staged dylib member.
Any path dependency that would introduce the original dylib package is
rewritten or rejected before Cargo runs.

### §3.11 Failure summary

Reject before Cargo build when:

- `build_targets` contains anything other than `"tests"`;
- final suite has `build_targets = ["tests"]` but empty `dev_deps`;
- a `dev_deps` name is absent from metadata-side `[dev-dependencies]`;
- a `dev_deps` name exists only in a target-specific dev-dependency
  table;
- a renamed dev-dep is encountered before alias support is implemented;
- an optional metadata-side dev-dep is encountered before collector
  force-build semantics are implemented;
- a staged package has `build = "..."` or default `build.rs`;
- split-crate resolution needs package+workspace support that has not
  been implemented;
- a path dependency back-edge cannot be analyzed or rewritten safely;
- a registry or git dependency in the staged graph names `dylib_crate`;
- member-local `[patch.*]` or unsupported member-local `[replace]`
  appears in a staged member.

---

## §4 Idempotency, Cache, and Speed

### §4.1 Default-path byte-stability

When `suite.build_targets.is_empty()`, the code must:

- not create `lihaaf-suite-workspace`;
- not parse metadata dev-dep specs for collector use;
- not alter the cargo command, env, message parsing, artifact copy, or
  deps-dir calculation;
- preserve existing manifest snapshot output when the new field is
  omitted.

This is the backwards-compatibility contract for every adopter that
does not opt in.

### §4.2 Opted-in determinism

The staged workspace writer must be deterministic:

- stable member directory names;
- sorted synthetic TOML tables where local serializers already sort;
- atomic write via temp file plus rename;
- stale staged members removed when the suite's dependency set shrinks;
- no writes outside the suite target dir.

### §4.3 Speed envelope

Opted-in suites pay for one suite-level Cargo graph build. They do not
pay per fixture.

For axum-macros, the expected shape is:

- default suite: one staged Cargo build if `build_targets = ["tests"]`;
- `debug_middleware`: no staged build unless it explicitly opts in;
- `from_ref`: no staged build unless it explicitly opts in;
- `from_request`: one staged Cargo build if opted in;
- `typed_path`: one staged Cargo build if opted in.

Warm-cache behavior should remain close to the existing per-suite cargo
cache model because the target dir is still
`target/lihaaf-build[-suite]`.

---

## §5 Backwards-Compatibility and Adopter Inventory

### §5.1 Local lihaaf manifests

Current repo facts:

- `Cargo.toml` defines lihaaf self metadata with
  `dylib_crate = "lihaaf"` and `extern_crates = ["lihaaf"]`.
- `tests/integration_corpus/Cargo.toml` defines integration corpus
  metadata with `dylib_crate = "integration_corpus"` and
  `extern_crates = ["integration_corpus", "integration_corpus_macros"]`.
- Neither local manifest currently declares `dev_deps` or
  `build_targets`.

Both stay on the byte-stable direct dylib path.

### §5.2 Axum-macros pilot

Key facts from the axum-lihaaf-pilot repo (`axum-macros/Cargo.toml`):

- metadata manifest: `axum-macros/Cargo.toml`;
- `dylib_crate = "axum"`;
- `extern_crates = ["axum", "axum-macros"]`;
- `features = ["macros"]`;
- default `dev_deps = ["axum-extra", "serde"]`;
- only two named suites restate dev_deps:
  `from_request` and `typed_path`;
- `debug_middleware` and `from_ref` do not restate dev_deps;
- `[dev-dependencies].axum-extra` is a path dependency;
- `[dev-dependencies].serde` has `features = ["derive"]`;
- `axum-extra` has feature edges that can depend back on `axum`.

Because `dev_deps` inherits, `debug_middleware` and `from_ref` have
non-empty final `dev_deps` even though they omit the raw field. Pilot
enablement must choose explicitly for every suite:

- add `build_targets = ["tests"]` to suites whose final `dev_deps`
  should be collected; or
- set `dev_deps = []` on suites that should use the direct dylib path.

Do not edit the pilot repo until lihaaf implementation and local review
have passed.

### §5.3 Djogi pilot facts

Key facts from the djogi repo manifests:

- `djogi-macros/Cargo.toml` has
  `dev_deps = ["serde", "serde_json", "sassi", "uuid", "rust_decimal"]`.
- `djogi/Cargo.toml` has `serde`, `serde_json`, `uuid`, and
  `rust_decimal` as dev dependencies.
- `djogi/Cargo.toml` additionally has `sassi` as a dev dependency.
- Workspace-level dependencies are declared in the root `Cargo.toml`.

### §5.4 Sassi pilot facts

Key facts from the sassi repo manifests:

- `sassi/Cargo.toml` has no `[package.metadata.lihaaf]`;
- `sassi-macros/Cargo.toml` is the relevant metadata manifest for
  sassi macro fixtures.

### §5.5 Non-local pilots

For `thiserror` and `derive_more`, keep only:

```text
NOT LOCALLY PRESENT / verify in CI
```

Do not claim their exact lihaaf shape from memory in this plan.

### §5.6 Summary

Known local adopters without `build_targets` remain unchanged.
Known split-crate adopters with fixture-only imports can opt in suite by
suite. The v0.1.0 gate remains axum-macros.

---

## §6 Spec and Documentation Amendments

Update `docs/spec/lihaaf-v0.1.md` after implementation details are
reviewed:

- schema table: add `build_targets`;
- suite inheritance section: `build_targets` REPLACE,
  `dev_deps` INHERIT;
- build pipeline: opted-in suites use a staged suite workspace,
  non-opted suites use the default dylib build;
- dev-deps explanation: explicit collector allow-list, no dylib crate
  dependency mutation;
- compatibility notes: default path byte-stable when omitted.

Update any public user-facing docs that describe lihaaf metadata. Do
not reference a non-existent guide path without adding or locating the
actual doc.

---

## §7 Tests

### §7.1 Config tests

Add unit tests for:

- default suite parses omitted `build_targets` as `[]`;
- named suite omits `build_targets` and receives `[]`;
- named suite omits `dev_deps` and inherits default `dev_deps`;
- validation runs after inheritance:
  a named suite with omitted raw `dev_deps`, inherited non-empty
  `dev_deps`, and `build_targets = ["tests"]` is accepted;
- `build_targets = ["tests"]` with final empty `dev_deps` is rejected;
- unknown and duplicate targets are rejected;
- existing direct serde/JSON construction paths cannot bypass
  validation, either through checked `Suite` deserialization or a checked
  `BuildTargets` newtype.

### §7.2 Suite workspace unit tests

Add tests for:

- collector dependencies are copied from metadata-side
  `[dev-dependencies]`;
- missing dev-dep spec rejects before Cargo build;
- dev-dep name present only under target-specific
  `[target.'cfg(...)'.dev-dependencies]` rejects before Cargo build;
- renamed dev-dep with `package = "..."` rejects before Cargo build and
  names the offending key;
- optional dev-dep rejects before Cargo build and names the offending
  key;
- path dev-dep paths are absolutized against metadata manifest dir;
- `workspace = true` dev-deps require carried
  `[workspace.dependencies]`;
- selected `[workspace.dependencies]` path entries are staged and
  back-edge-rewritten when referenced through `workspace = true`;
- staged dylib manifest has absolute `[lib].path` and
  `crate-type = ["rlib", "dylib"]`;
- staged dylib manifest strips explicit `[[bin]]`, `[[example]]`,
  `[[test]]`, and `[[bench]]` targets;
- staged package with `build = "..."` or default `build.rs` rejects;
- promoted dev-deps are absent from the staged dylib `[dependencies]`;
- path dependency back-edge to `dylib_crate` is rewritten to the staged
  dylib member;
- registry or git dependency that names `dylib_crate` rejects;
- conservative path staging finds a `dylib_crate` path back-edge even
  when the dependency is optional or target-specific;
- member-local `[patch.*]` is rejected;
- workspace-root non-self `[patch.*]` entries are carried with path
  absolutization;
- workspace-root `[patch.*]` self entries for `dylib_crate` reject;
- workspace-root `[replace]` and `[profile.*]` carry down;
- member-local `[replace]` follows the locked policy.

### §7.3 Cargo-spawning integration tests

Use `LIHAAF_RUN_CARGO_BUILD_TESTS=1`. Do not add a Cargo feature named
`cargo-build` for these tests.

Required gated integration cases:

- negative identity probe: separate collector build in same target dir
  produces incompatible dylib/rlib identity for a feature back-edge;
- positive same-graph probe: staged workspace builds `a` as rlib+dylib
  and collector `h`, fixture links dylib `a` plus rlib `b`, and the
  binary compiles/runs;
- explicit feature back-edge via `dep:x` activation;
- feature back-edge via `x/feature` activation;
- default-feature back-edge;
- axum-extra-shaped minimal repro;
- multi-suite isolation: each opted-in suite gets one staged workspace
  and one target dir, with no per-fixture Cargo build;
- direct-path byte-stability when `build_targets` is omitted.

### §7.4 Axum pilot gate

After implementation and local lihaaf verification, run the axum-lihaaf-pilot
suite all-green without reverting or editing its dirty generated snapshots
unless explicitly instructed.

---

## §8 Verification Commands

Plan rewrite verification:

```bash
rtk git diff --check -- \
  docs/spec/dev-deps-overlay-promotion-plan-2026-05-19.md \
  docs/spec/v010-precompact.md
```

Implementation verification, after review allows coding:

```bash
rtk cargo test
rtk env LIHAAF_RUN_CARGO_BUILD_TESTS=1 cargo test --test dev_deps_collector_integration
```

The second command name is provisional; use the actual test target name
chosen during implementation. The env var is not provisional.

---

## §9 Locked Decisions and Remaining Questions

### §9.1 Locked: staged suite workspace

Opted-in suites use a temporary isolated workspace and one Cargo graph
that selects both the staged dylib package and the collector package.
Separate "build dylib first, collector second" is rejected unless a
future proof handles the crate-identity trap.

### §9.2 Locked: no dylib dependency mutation

Metadata-side `dev_deps` never become dependencies of the staged dylib
package. Optional dependency semantics in the real dylib crate are
therefore preserved.

### §9.3 Locked: validation after suite resolution

`build_targets` validation happens against final `Suite` values, not
raw TOML fields.

### §9.4 Locked: test gate

Cargo-spawning tests use `LIHAAF_RUN_CARGO_BUILD_TESTS=1`.

### §9.5 Locked: workspace helper reality

OQ-4 is closed around actual compat behavior:

- workspace inheritance carry-down and `[replace]` / `[profile.*]`
  carry-down live in `apply_workspace_member_inheritance`;
- member-local `[patch.*]` rejection lives there too;
- workspace-root `[patch.*]` carry-down needs a narrower helper than
  `apply_self_patch_policy`; carry non-self patches with path
  absolutization, but do not inject or remap a crates-io self patch for
  the staged suite workspace.

### §9.6 Locked: conservative path-dependency staging breadth

The locked invariant is clear: no dependency rlib may compile against a
different `dylib_crate` identity. v0.1.0 stages every path dependency
transitively reachable from the collector specs, staged dylib manifest,
or selected `workspace = true` dependency specs before Cargo runs,
without trying to reproduce Cargo's feature resolver. Unused staged
members are acceptable; missed identity back-edges are not.

### §9.7 Locked: build scripts in staged packages

v0.1.0 rejects staged packages that declare `build = "..."` or contain a
default `build.rs`. Package-root mirroring or explicit build-script path
support is out of scope until a separate design and test matrix exists.

### §9.8 Locked: lockfile and network policy

The staged builder copies an ancestor lockfile into the temporary suite
workspace when one exists, but it does not add `--offline`, `--frozen`,
or `--locked` by default. Cargo's ambient network policy remains the
source of truth.

---

## §10 Implementation Order

1. Add `Suite.build_targets`, raw fields, resolved validation, and
   config tests.
2. Add `src/suite_workspace` with pure synthesis functions and unit
   tests for staged manifests, collector deps, workspace carry-down,
   and back-edge rewrites.
3. Add the staged Cargo build runner and artifact parser.
4. Wire `src/session.rs` to branch to the staged builder only for
   opted-in suites.
5. Add env-gated cargo-spawning integration tests.
6. Update `docs/spec/lihaaf-v0.1.md` and user-facing docs.
7. Run local verification.
8. After implementation and local review pass, enable the axum pilot
   without touching dirty generated snapshots:
   - top-level `[package.metadata.lihaaf]`: add
     `build_targets = ["tests"]`;
   - suite `from_request`: add `build_targets = ["tests"]`;
   - suite `typed_path`: add `build_targets = ["tests"]`;
   - suite `debug_middleware`: add `dev_deps = []`;
   - suite `from_ref`: add `dev_deps = []`;
   - verify `cargo lihaaf --package axum-macros` from the pilot
     worktree.
9. Run final release review.
10. Only after implementation, pilot verification, and release review
    pass, dispatch the publisher with explicit package, registry,
    version, working directory, CI gate, and target commit.

---

## §11 Review Checklist

Fresh plan review should focus on:

- crate identity between managed dylib and collector-built rlibs;
- path dependency rewrite and Cargo package collision behavior;
- workspace resolver scope, especially virtual workspace versus
  package+workspace roots;
- workspace inheritance, `[replace]`, `[profile.*]`, and `[patch.*]`
  behavior;
- optional and renamed dev-dep policies;
- target-dir and lockfile side effects;
- suite-level speed invariant;
- default-path byte-stability when `build_targets` is omitted.

## §12 Out of Scope

- publishing v0.1.0;
- pilot worktree edits during plan rewrite;
- auto-discovering fixture imports;
- per-fixture Cargo builds;
- expanding `build_targets` beyond `"tests"`;
- changing compat-mode behavior outside shared helpers needed for the
  staged suite workspace.
