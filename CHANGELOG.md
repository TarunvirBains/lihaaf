# Changelog

All notable changes to lihaaf are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the
project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0-beta.10] — 2026-05-19

Delivers the `extra_substitutions` adopter-configuration framework (issue #45,
PR #68) with disjoint path-shape and banner-shape allowlists. The plan went
through 6 rounds of strict-swe Opus PLANNER + Codex 5.5 xhigh adversarial
review (3 adversarial cycles + 3 DOC cleanup rounds). The PR went through
Codex final-review BLOCK→FIX→ALLOW iteration: round-1 ALLOW with one
FIX_BEFORE_BETA (serde `Deserialize` bypass surface); round-2 BLOCK on
validation-ordering drift between TOML and serde paths; round-3 ALLOW.

### Added

- **`extra_substitutions` config key** (#45, PR #68): list of
  `{ from, to }` literal-substring substitutions applied per-line during
  snapshot normalization, AFTER built-in path placeholders and BEFORE
  TypeId collapse. Adopters supply environment-specific path mappings
  (NixOS store paths, vendored toolchains, Bazel sandboxes) without
  forking lihaaf's normalizer defaults. `from` is gated by `is_path_like`;
  `to` is constrained only by no-newline. Substitutions apply
  left-to-right in declared order; earlier rules can feed later rules.

- **`strip_lines` and `strip_line_prefixes` config keys** (#45, PR #68):
  per-line drop matching with exact-equality and prefix semantics
  respectively. Operate at line granularity (whole-line drop), distinct
  from `extra_substitutions`'s substring-replacement semantics. Both
  keys gate via the disjunction `is_path_like || is_banner_shape`, so
  adopters can drop either path-noise (NixOS / vendored / sandbox
  paths) OR banner lines (rustc explain footers, macro-origin trailers,
  error-count summaries, CI deprecation banners, vendored-toolchain
  version banners).

- **`is_path_like` allowlist predicate**: validates
  `extra_substitutions.from`. Requires `/`, `\`, or full-string match
  of `^\$[A-Z][A-Za-z0-9_]*$` (uppercase-anchored bare placeholder).
  Leading-`$` guard rejects `$lowercase/path`, `$1/path`, and similar
  bypass attempts via the `/` branch. Rule 4(c) is full-string anchored
  to reject `$DIR-`, `$DIR.`, `$A!` style trailing-junk patterns.
  Interior `$lowercase` within paths (e.g., `/path/$nix/sub`) is
  accepted as path text per the OQ-B leading-only clarification.

- **`is_banner_shape` allowlist predicate**: validates strip patterns
  via disjunction with `is_path_like`. Three-layer hybrid: (A) shared
  preconditions — len ≥ 20, no `\n`, no leading whitespace, no leading
  `^`/`=`/`|`; (B) 11-entry anti-prefix REJECT list (`expected `,
  `found `, `the trait `, `the type `, `cannot find `, `mismatched types`,
  `consider `, `help: `, `warning: `, `error[`, `  `); (C) disjunction
  of either (C.1) one of 5 enumerated rustc/tool banner prefixes
  (`For more information about this error`, `error: aborting due to `,
  `note: this error originates from `, `info: `, `linker version: `)
  OR (C.2) structural banner shape — len ≥ 40, ASCII uppercase first
  byte, contains space, contains at least one deprecation marker
  (`deprecated`, `deprecation`, `Please update`, `actions to use`,
  `EOL`, `end-of-life`).

- **Per-suite REPLACE semantics**: all three new keys live on `Suite`
  (not `Config`). Omission on a named suite resolves to empty `[]`
  (REPLACE, no inheritance from default suite). Mirrors the `features`
  precedent.

- **Serde `Deserialize` validation closure**: `Substitution` deserializes
  via `#[serde(try_from = "RawSubstitution")]`; strip keys deserialize
  via `StripPattern` newtype with `#[serde(try_from = "String")]`.
  Closes the public-API bypass route surfaced by Codex final review
  (the CLI TOML path was already safe via `validate_extra_substitutions`).
  Validation order across both paths now matches: `from` presence →
  `to` presence → `from` shape → newline-in-`to`.

- **Spec amendments** to `docs/spec/lihaaf-v0.1.md`: §3.2 (schema), §3.4
  (validation rules), §3.6 (per-suite inheritance non-list), §6.2
  (adopter extras bullet), §6.5 (determinism tuple), and new §6.6
  (~120 lines covering adopter-facing predicate contracts, per-suite
  REPLACE semantics, structural-banner-shape framing, compat-mode
  unsupported status, full-string-anchor convention for bare
  placeholders, interior `$lowercase` acceptance clarification).

### Notes

- **Compat mode for v0.1.0-beta.10:** the three new keys are documented
  as *unsupported in compat mode*. Adopter manifests using them with
  `cargo lihaaf --compat ...` are silently no-op'd at the overlay
  layer; this is the documented v0.1.0 contract, not an implementation
  gap. Compat-mode support is a v0.2 deliverable.

- **76 new tests** added (444 → 451 lib tests since beta.9; 76 new
  reflects the full plan §7 surface plus 6 round-1-fixup regression
  tests plus 1 ordering-parity test). Coverage includes: predicate
  matrices (8 acceptance + 11+ rejection classes for `is_path_like`;
  banner-shape acceptance + structural-banner non-CI + round-2 BLOCK
  regression guards for `is_banner_shape`); field-level wiring through
  both validators; composition + interaction with built-in placeholders,
  TypeId collapse, and compat short-CARGO; serde-bypass closure tests
  for `Substitution` + both strip keys; TOML/serde ordering-parity
  test for multiply-malformed inputs.

- **No predicate or default behavior change** for adopters who do not
  set any of the three new keys. Byte-identical normalizer output vs
  beta.9 in the absence of configuration. The 13 existing normalizer
  unit tests at `src/normalize.rs:493-840` and every in-tree fixture
  snapshot pass unchanged.

## [0.1.0-beta.9] — 2026-05-18

Delivers workspace-member entry via `--package` / `-p` flag (issue #53, PR #61),
unblocking Round-2 enrollment of axum-macros and similar workspace-member-shape
pilots. PR #61 also closes a 26-item post-merge punch list across four classes:
resolver path normalization (BLOCK-1, 12 sites), workspace-root non-table hard
rejection (BLOCK-2, 8 sites), multi-registry `[patch.<registry>]` carry-down
(BLOCK-3 / COUNTER_SIGNAL), and non-table member-local patch rejection.

### Added

- Compat-mode `--package <pkg>` / `-p <pkg>` flag for workspace-member entry (PR #61). When `--compat-root` points at a workspace root, `--package` resolves the upstream manifest to the named workspace member. The workspace's `[workspace.*]`, ALL `[patch.<registry>]` subtables (crates-io and alt registries), `[replace]`, and `[profile.*]` tables are carried down into the staged overlay so the member's `{ workspace = true }` references and patch resolution match baseline cargo's behavior. Closes #53 — unblocks Round-2 enrollment of axum-macros and similar workspace-member-shape pilots. PR #61 also includes: 12-site resolver path normalization (BLOCK-1), workspace-root non-table `[patch.<registry>]` hard rejection at 8 sites (BLOCK-2), multi-registry carry-down for all `[patch.<registry>]` subtables (BLOCK-3 / COUNTER_SIGNAL), and non-table member-local patch rejection for all registries.

  See `docs/compatibility-plan.md` §3.2.3 ("Workspace-member entry via `--package`") and `docs/spec/lihaaf-v0.1.md` §8.2 for the adopter-facing surface. v0.1.0 scope is virtual workspaces only (workspace root declares `[workspace]` without `[package]`); the package+workspace shape is deferred to v0.2 / v1.0 with a directed REJECT diagnostic.

## [0.1.0-beta.8] — 2026-05-18

Closes #40 (serde-json `ambiguous specification`) and #47 (cxx
`links = "cxxbridge1"` collision) via PR #56. Delivers the Option H
4-rule self-patch policy and a staged package-root mirror.

### Fixed

- Compat-mode now applies an intent-aware self-patch policy to `[patch.crates-io.<overlay-package-name>]` in the staged overlay (Option H, 4 rules):
  - Rule 1 (INJECT): if your upstream Cargo.toml does not self-patch the package-under-test, lihaaf injects `[patch.crates-io.<overlay-package-name>] = { path = "<staged-overlay-dir>" }`. Resolves the previously-failing serde-json case (`ambiguous specification`) and the family-completeness equivalents on anyhow-shape pilots.
  - Rule 2 (REMAP): if your upstream self-patches the package-under-test to a path that resolves to the upstream root crate (cxx-style `path = "."`), lihaaf rewrites the entry to point at the staged overlay directory. Resolves the previously-failing cxx case (`links = "cxxbridge1"` collision).
  - Rule 3: non-target `[patch.crates-io.<X>]` entries are preserved untouched.
  - Rule 4 (REJECT): if your upstream self-patches the package-under-test to a non-root path (vendored fork) or to a git source, lihaaf rejects with a clear error. The escape hatch (`--compat-allow-patch-override`) is deferred to v0.2/v1.1; if you hit this case, file an issue.

  See `docs/compatibility-plan.md` §3.2.3 for the adopter-facing rule table.

- Compat-mode now creates a staged package-root mirror in the overlay directory. After writing the overlay `Cargo.toml`, lihaaf creates symlinks (or copies on platforms where symlinks are unavailable) for each top-level entry in the upstream package directory into the staged overlay dir. This ensures that `build.rs` scripts which read package-root-relative files via `CARGO_MANIFEST_DIR` (cxx: `src/cxx.cc`, `include/cxx.h`) or via cwd probes (anyhow: `src/nightly.rs`; thiserror: `build/probe.rs`) find the correct files during the overlay build. Upstream entries excluded from the mirror: `target/`, `.git/`, `Cargo.toml` (overlay-generated), `Cargo.lock`. Without this fix, cxx builds fail with a hard I/O error; anyhow and thiserror builds silently use incorrect cfg flags (silent-false probe pattern).

  Issues #40 and #47.

## [0.1.0-beta.7] — 2026-05-18

Bundles the `allow_lints` feature (issue #43) plus a class-sweep fix for
NUL-byte rejection across argv-bound config string fields. The `allow_lints`
feature landed via PR #54 across two adversarial-review rounds; Codex round-1
flagged a single NUL gap in `validate_allow_lints`, and the subsequent
class-enumeration sweep surfaced three additional sibling instances of the
same gap (features, dylib_crate, test corpus). All four were closed in one
atomic follow-up commit before merge.

### Added
- **`allow_lints` config key** (#43, mirrors trybuild #302): new optional
  `Vec<String>` key in `[package.metadata.lihaaf]` and per-suite tables.
  Each entry is forwarded as `-A <lint>` to per-fixture rustc invocations,
  letting adopters suppress noisy lints (e.g. `unused_imports`, `dead_code`
  from bare rustc invocation paths) without per-fixture `#![allow(...)]`
  annotations. Inherits from the default suite when omitted on a named
  suite, mirroring the `dev_deps` precedent.
- **Compat-mode default `allow_lints = ["unexpected_cfgs"]`** in synthetic
  metadata as forward-only insurance: `unexpected_cfgs` is `--check-cfg`-
  gated and lihaaf does not currently pass `--check-cfg`, so the default
  is a no-op under v0.1.0 today. It will start mattering on the day
  lihaaf or rustc enables check-cfg-driven diagnostics. Adopters who want
  the opposite (i.e. surfacing `unexpected_cfgs` once enabled) must drop
  out of compat mode and use the v0.1 TOML-driven path.

### Fixed
- **NUL-byte rejection in argv-bound config strings** (`src/config.rs`):
  four config fields that flow to subprocess argv tokens (`allow_lints`,
  `features`, `dylib_crate`, plus their test corpus) now reject interior
  NUL bytes at config-parse time, surfacing as `CONFIG_INVALID` with a
  directed diagnostic. Previously these fields had no NUL check, so a
  NUL would pass validation and the spawn failure would surface as
  `WORKER_CRASHED` / `SubprocessSpawn` — actionable but misrouted.
  New `validate_features` and `validate_dylib_crate` private functions
  enforce the same structural rule. The `allow_lints` validator gains
  a NUL check ahead of its existing whitespace / quote / backslash
  rejection.

## [0.1.0-beta.6] — 2026-05-17

Targeted GA-blocker fix for compat-mode workspace identity. v0.1.0-beta.5
correctly resolved the manifest-name + envelope-determinism bugs, but the
post-publish refresh-pilots run
([Actions run 26000403851](https://github.com/TarunvirBains/lihaaf/actions/runs/26000403851))
revealed a NEW bug class: the staged overlay at
`<upstream>/target/lihaaf-overlay/Cargo.toml` collided with upstream
workspace identity for workspace-style pilots. Three of four Round-1
pilots (cxx, serde-json, thiserror) failed with
`package <X> is a member of the wrong workspace`; only anyhow
(single-crate, no `[workspace]`) succeeded. Tracked as issue #36; fixed
by PR #37 across four adversarial-panel rounds.

### Fixed
- **Workspace-identity collision in staged overlay** (`src/compat/overlay.rs`):
  the overlay-materialization pipeline now applies a 5-branch decision
  tree to the upstream manifest before writing the staged overlay:
  1. **Explicit member** (`[package].workspace = "<path>"`) → REJECT
     with `Error::Cli { clap_exit_code: 2, ... }` and a directed
     diagnostic naming the ancestor pointer and pointing the user at
     the workspace root.
  2. **Implicit ancestor member** (walk-up finds an ancestor
     `Cargo.toml` declaring `[workspace]`) → REJECT with a
     directed diagnostic naming the ancestor manifest path. This
     conservative check catches the silent-graph-divergence case where
     baseline cargo inherits ancestor `[patch.crates-io]` /
     `[replace]` / `resolver` / `[profile]` / `[workspace.dependencies]`
     state but the lihaaf overlay would terminate cargo's walk-up at
     `target/lihaaf-overlay/Cargo.toml` — producing differing dependency
     graphs and false pass/fail compat results.
  3. **Implicit member via inheritance refs** (no local `[workspace]`
     and at least one `{ workspace = true }` reference in
     `[package]` / `[dependencies]` / `[dev-dependencies]` /
     `[build-dependencies]` / `[target.<cfg>.<deps>]` / `[lints]`) →
     REJECT (would otherwise strand the inheritance reference at cargo
     parse time).
  4. **Workspace root** (manifest has its own `[workspace]` table) →
     clone, strip the membership keys (`members`, `exclude`,
     `default-members`), preserve all inheritance / configuration
     tables (`workspace.dependencies`, `workspace.package`,
     `workspace.lints`, `workspace.metadata`, `resolver`, and any
     unknown forward-compat sub-keys). The overlay declares no members,
     so the upstream's path-dep crates remain owned by the upstream
     workspace; the preserved tables keep every `{ workspace = true }`
     reference resolvable.
  5. **Standalone single-crate** (no workspace markers, no ancestor
     workspace) → inject an empty `[workspace] = {}` table to terminate
     cargo's walk-up at the staged overlay.
- **`detect_implicit_ancestor_workspace` helper** (`src/compat/overlay.rs`):
  walks parent directories from `parent_of(parent_of(upstream))` upward
  via lexical `Path::parent()` traversal (no `canonicalize` —
  intentional, see Known limitations). Returns `Some(ancestor_path)`
  on the first parseable `Cargo.toml` declaring `[workspace]`. Malformed
  ancestor manifests emit a non-fatal stderr warning and the walk
  continues; only hard I/O errors propagate.
- **`manifest_has_inheritance_reference` helper** (`src/compat/overlay.rs`):
  detects `{ workspace = true }` shapes across all 10+ inheritance
  families (package fields via the dotted-style + table-style, every
  dependency table including target-conditional and dev/build variants,
  the top-level `[lints]` form, and per-namespace
  `[lints.{rust,clippy,rustdoc}]`). Strictly checks
  `value.is_bool() == Some(true)` so false positives like
  `[package.metadata] workspace = "foo"` (string value) don't fire.

### Known limitations
- Issue #38 — ancestor `workspace.exclude` array not honored in
  implicit-ancestor detection. Conservative false-positive rejection
  on intentionally-excluded descendants; low likelihood. POST_BETA.
- Issue #39 — ancestor-walk doesn't follow symlinks. If upstream is
  reached via a symlink and the ancestor `[workspace]` lives only on
  the real path side, the walk misses it and the silent-divergence
  failure mode survives. Low likelihood. POST_BETA.
- Issue #40 — serde_json `specification serde_json is ambiguous`
  remains. A SEPARATE failure mode (resolution-time, not
  manifest-parse-time) not addressed by this PR. May be collateral-
  fixed by the new workspace handling; refresh-pilots against beta.6
  will reveal.
- Workspace-member case (lihaaf invoked from a sub-crate within a
  workspace, NOT from the workspace root) is explicitly rejected with
  a clean diagnostic. Ancestor-workspace inheritance flattening is
  out-of-scope for v0.1 (would require cross-manifest reads).
- Windows path portability deferred to v0.2 (carried over from beta.5).

### Tests
- `tests/compat/overlay_determinism.rs`: new
  `staged_overlay_overrides_upstream_workspace_inheritance` pins the
  workspace-root preservation contract (R2).
- `tests/compat/overlay_determinism.rs`: new
  `cargo_accepts_workspace_inheritance_reference_in_overlay`
  (LIHAAF_RUN_CARGO_BUILD_TESTS=1 gate) — constructs the exact
  Codex R1-BLOCK repro (`[workspace.dependencies] foo = { path = "..." }`
  + `[dependencies] foo = { workspace = true }`) and asserts cargo
  rustc succeeds end-to-end.
- `tests/compat/overlay_determinism.rs`: new
  `cargo_accepts_workspace_style_overlay_for_dylib_build`
  (LIHAAF_RUN_CARGO_BUILD_TESTS=1 gate) — constructs a real
  workspace-style upstream and asserts cargo rustc succeeds.
- `tests/compat/overlay_determinism.rs`: new
  `staged_overlay_rejects_workspace_member_manifest` and
  `staged_overlay_rejects_implicit_workspace_member_manifest` and
  `staged_overlay_rejects_manifest_with_ancestor_workspace` —
  variant-match assertions (not loose `format!("{err:?}").contains()`)
  so a future refactor swapping the error variant trips a panic
  instead of a silent test pass.
- `src/compat/overlay.rs::tests`: 17 new unit tests across all 5
  branches plus the new helper functions
  (`manifest_has_inheritance_reference_detects_every_family` covers
  10 inheritance shapes; `override_workspace_*` tests pin preservation,
  injection, idempotency, all rejection paths).
- Test counts at beta.6: 268 lib tests (was 251), 29
  `overlay_determinism` integration tests (was 22), 15
  `report_determinism` tests, 4 cargo-build-gated `cargo_accepts_*`
  tests (was 2).

### Changed
- **5-branch decision tree** in `override_workspace_inheritance`
  replaces the v0.1.0-beta.5 absolutization-only path. The
  absolutization pass for `[workspace] members/exclude/default-members`
  (carried over from beta.5) still runs but its output is harmlessly
  clobbered by the override; the unit tests on the absolutize pass
  call it directly so they remain pinned.
- **`src/compat/overlay.rs` module-level docs**: expanded with the
  R1 → R2 → R3 → R4 decision-tree rationale; explains why empty
  workspace was wrong (R1), why preserving inheritance tables matters
  (R2), why implicit-via-refs needs rejection (R3), why ancestor-walk
  is required for correctness (R4), and why the workspace-member case
  is intentionally out-of-scope for v0.1.

### Process
- 4-round adversarial-panel review (Codex xhigh + Gemini 3.1-pro-preview
  + strict-swe Opus); each BLOCK was investigated, fixed, and
  re-reviewed:
  - R1 (`1f6520b`) BLOCK by Codex + Gemini → R2 (`cc19ecc`)
  - R2 BLOCK by Codex with Gemini COUNTER → R3 (`83ca8d9`)
  - R3 BLOCK by Codex → R4 (`8d2517a`)
  - R4 triple-ALLOW with 3 informational COUNTERs filed as issues
    #38, #39, #40

## [0.1.0-beta.5] — 2026-05-17

Targeted GA-blocker fix for the compat-mode manifest staging path. The
v0.1.0-beta.4 release shipped the compat driver with the overlay
materialized as `<upstream>/Cargo.lihaaf.toml` (a sibling of the
upstream `Cargo.toml`). Cargo validates `--manifest-path` filenames at
startup and rejects any path whose last component is not literally
`Cargo.toml` — every stage-2 pilot fork (cxx, serde-json, anyhow,
thiserror) failed with `lihaaf_session_failed` / detail "the
manifest-path must be a path to a Cargo.toml file" on every CI run
([Actions run 25994537438](https://github.com/TarunvirBains/lihaaf/actions/runs/25994537438)).

### Fixed
- **Overlay staging path** (`src/compat/overlay.rs`): the materialized
  overlay now writes to `<upstream>/target/lihaaf-overlay/Cargo.toml`
  so cargo accepts `--manifest-path` without error. The `target/`
  subtree is treated as implicitly ignored by the cleanup classifier
  (existing `<target_root>/target/` short-circuit), so no fork-side
  `.gitignore` change is required.
- **Path-bearing TOML keys absolutized in the staged overlay**: cargo
  resolves every path-bearing manifest key against the parent
  directory of the manifest being parsed. The staged overlay lives
  two dirs deeper than the upstream `Cargo.toml`, so without
  absolutization cargo would search the empty staged dir for
  `src/lib.rs`, `build.rs`, path-deps, and workspace members. The
  overlay now absolutizes `[lib] path`, `[[bin]] path`,
  `[[example]] path`, `[[test]] path`, `[[bench]] path`,
  `[dependencies.X] path`, `[dev-dependencies.X] path`,
  `[build-dependencies.X] path`, `[target.*.<deps>] path`,
  `[workspace] members`, `[workspace] exclude`,
  `[workspace] default-members`, `[workspace.dependencies.X] path`,
  `[package].workspace`, `[package] build`, and
  `[patch.<registry>.X] path` (the `git`/`branch`/`tag`/`rev` fields
  in `[patch]` pass through verbatim; only `path`-form overrides are
  rewritten — fixing the cxx pilot pattern `cxx = { path = "." }`),
  and `[replace."<source-id>"] path` (the older soft-deprecated
  replacement form; same absolutization semantics and same family of
  failure as `[patch]` — R3 FIX class IV).
  Auto-discovery for `[[bin]]` / `[[example]]` / `[[test]]` /
  `[[bench]]` is explicitly disabled (`autobins = false`, etc.) so a
  future cargo version that hardens the empty-discovery case does not
  break the overlay.
- **`compat_root` absolutized at CLI entry boundary**: the production
  shape `--compat-root .` (used in `compat/templates/pilot-stage2.yml`)
  now receives a single-point absolutization in `CompatArgs::from_cli`
  via `current_dir().join()` before reaching any downstream consumer.
  Previously a relative `compat_root` caused every downstream `join`
  (converted-fixtures dir, overlay staging path, manifest path) to
  produce relative strings that cargo resolved against the staged
  manifest dir instead of the crate root, yielding a double-`target/`
  nonexistent-path failure on every real pilot run.
- **`fixture_dirs` resolution under the new staging path**
  (`src/compat/mod.rs`): the synthetic `[package.metadata.lihaaf]`
  block previously wrote `fixture_dirs` as repo-relative strings
  (`./target/lihaaf-compat-converted/{compile_pass,compile_fail}`).
  lihaaf's `discovery::collect` resolves relative paths against the
  manifest's parent dir, which under the new staging is
  `<compat_root>/target/lihaaf-overlay/` — a double-`target/` lookup
  that does not exist. The driver now absolutizes both paths against
  `<compat_root>` directly so the inner session sees the real
  on-disk locations regardless of where the overlay manifest is
  staged.
- **`commands.lihaaf` envelope field no longer leaks absolute paths**
  (`src/compat/mod.rs` `render_inner_command`): the `--manifest-path`
  argument was previously serialized via `overlay_manifest.to_string_lossy()`,
  embedding the runner-specific absolute checkout path (e.g.
  `/home/runner/work/my-crate/my-crate/target/lihaaf-overlay/Cargo.toml`).
  This violated the §3.3 determinism rule: two CI runners at different
  checkout roots produced non-identical envelope bytes. Fix: strip the
  `compat_root` prefix via `Path::strip_prefix` before serialization,
  producing the canonical repo-relative form `target/lihaaf-overlay/Cargo.toml`
  on every runner (R3 FIX class III).
- **`errors[].detail` envelope field no longer leaks absolute paths**
  (`src/compat/report.rs` `normalize_error_detail_paths`): infrastructure
  errors — in particular `DylibBuildFailed` — embed the cargo invocation
  in their `Display` output, which includes absolute `--manifest-path`
  and `--target-dir` values (cargo requires both to be absolute).
  Without normalization, a failure envelope from any stage-2 pilot run
  contained runner-specific paths (e.g.
  `/home/runner/work/lihaaf/lihaaf/target/lihaaf-build`) in
  `errors[0].detail`, violating §3.3 determinism. Fix: a new
  `normalize_error_detail_paths` step strips the `compat_root` prefix
  from every `errors[].detail` string at the envelope write boundary,
  mirroring the `commands.lihaaf` normalization pattern. Local terminal
  output is unaffected — the `Display` impl is unchanged (R5 FIX class V).
- **`mismatch_examples[].fixture` envelope field no longer leaks an
  absolute fallback path** (`src/util.rs` `relative_to`): structured
  path relativization now returns an error when the input is outside
  the expected base, forcing compat callers to choose an explicit
  non-absolute diagnostic rendering (`outside-base/...`) instead of
  silently serializing a runner-specific absolute path (R5 FIX class VI).

### Known limitations
- Windows path portability in `errors[].detail` and `mismatch_examples[].fixture` normalization is v0.2 work. v0.1 stage-2 runs ubuntu-24.04 only.

### Changed
- **`docs/compatibility-plan.md` §3.2.3** rewritten to describe the
  staged-target overlay shape, the path-absolutization sub-procedure,
  and the cleanup-classifier short-circuit that covers it. The
  "sibling vs rewrite-then-restore vs `[patch]`" rationale subsection
  now reads as "staged-target vs sibling vs rewrite-then-restore vs
  `[patch]`", with the sibling row added as the v0.1.0-beta.4
  approach this PR superseded.
- **`.gitignore`** no longer carries a dedicated `/Cargo.lihaaf.toml`
  entry — the staged overlay lives under `target/`, which the
  existing `/target` rule already covers.

### Tests
- `tests/compat/overlay_determinism.rs`: new
  `cargo_accepts_staged_overlay_for_dylib_build` test (gated behind
  `LIHAAF_RUN_CARGO_BUILD_TESTS=1`) invokes `cargo rustc` against the
  staged overlay with a synthetic `<upstream>/Cargo.toml` +
  `<upstream>/src/lib.rs` and asserts exit 0 — codifies the manual
  repro from the PR #34 adversarial review.
- `tests/compat/overlay_determinism.rs`:
  `staged_overlay_carries_absolute_lib_path` and
  `staged_overlay_absolutizes_path_dependencies` pin the
  path-absolutization contract at the byte level so every CI lane
  (without the cargo-build env-var gate) bites a regression that
  drops or downgrades the rewrite.
- `tests/compat/overlay_determinism.rs`:
  `absolutizes_patch_path_entries` (FIX class C) pins that
  `[patch.crates-io.X].path` entries are absolutized; regression for
  the cxx pilot `cxx = { path = "." }` / `cxx-build = { path =
  "gen/build" }` pattern that the Round-2 strict-swe Opus BLOCK found.
- `tests/compat/overlay_determinism.rs`:
  `staged_overlay_absolutizes_workspace_key_classes` (FIX class B) pins
  `[package].workspace`, `[workspace].default-members`, and
  `[workspace.dependencies.X].path` absolutization.
- `tests/compat/overlay_determinism.rs`:
  `cargo_accepts_rich_overlay_for_dylib_build` (FIX class D, gated
  behind `LIHAAF_RUN_CARGO_BUILD_TESTS=1`) exercises path-dep +
  `[patch.crates-io]` path entry in a single `cargo rustc` run — the
  richer production-failure shape the Round-2 panel surfaced that the
  minimal existing test would not have caught.
- `tests/compat/overlay_corpus/with_patch_section.{input,expected}.toml`
  updated to include a path-form patch entry (`demo-patched = { path =
  "." }`) alongside the existing `git`/`branch` entry, so the
  cross-binary determinism corpus bites any regression to `[patch.*.X]
  path` absolutization.
- `src/compat/overlay.rs` unit tests cover the explicit / implicit
  `[lib] path` injection, the `[target.*.dependencies.X] path`
  rewrite, the `[workspace] members` / `[workspace] exclude` rewrite,
  the `[package] build` injection rule (only when
  `<upstream>/build.rs` exists), the `autoX = false` disabling for
  non-lib targets, and the `[[bin]]` / `[[example]]` / `[[test]]` /
  `[[bench]]` `path =` rewrite.  Round-3 adds unit tests for all three
  FIX class B key classes (`absolutizes_package_workspace_pointer`,
  `absolutizes_workspace_default_members`,
  `absolutizes_workspace_dependencies_path`) and two unit tests for
  FIX class C (`absolutizes_patch_registry_path`,
  `absolutize_leaves_absolute_patch_path_unchanged`). Round-4 (R3
  panel): corrects the two failing class-B/C unit-test expectations to
  match `Path::join` semantics (no normalization — `..` and `.` are
  preserved), adds `absolutizes_replace_path` for FIX class IV
  (`[replace]`), and adds
  `render_inner_command_manifest_path_is_repo_relative` (FIX class III
  — §3.3 envelope determinism).
- `src/compat/cli.rs`: `from_cli_absolutizes_relative_compat_root`
  (R3 FIX class II) exercises `CompatArgs::from_cli` end-to-end with
  a relative `--compat-root` basename and asserts the resulting
  `compat_root` is absolute. Previously the test suite only checked
  absolutization implicitly via the overlay layer; this test bites a
  future regression that removes the `absolutize_required_path` call
  from `from_cli`.
- `tests/compat/overlay_determinism.rs`:
  `replace_paths_are_absolutized` (R3 FIX class IV) pins that
  `[replace."<source-id>"].path` entries are absolutized in the
  overlay.
- `tests/compat/overlay_corpus/with_replace_section.{input,expected}.toml`
  added to the cross-binary determinism corpus so any regression to
  `[replace]` path absolutization is caught by the corpus test.

## [0.1.0-beta.4] — 2026-05-16

Headline addition: **compat mode** — a fork-driven workflow letting
adopters compare their trybuild baseline against lihaaf's per-fixture
verdicts via a deterministic §3.3 JSON envelope. Adopters opt in with
`cargo lihaaf --compat --compat-root <DIR> --compat-report <PATH>`. The
companion `compat/baseline.toml` ceiling table + `lihaaf-compat-gate.yml`
workflow ship empty / dry-run for beta-4 — pilot crates enroll
post-cut by PR-ing rows to `baseline.toml`. Closes #8 #9 #10 #11 #12.

The `[package.metadata.lihaaf]` schema, §3.3 envelope shape, exit codes,
and snapshot byte format are unchanged from v0.1.0-beta.3 — compat
mode is a NEW surface, not a modification of existing ones.

### Added
- **Compat-mode driver** (`src/compat/`): 10-step pipeline reading
  upstream `Cargo.toml`, materializing a sibling overlay (`Cargo.lihaaf.toml`)
  with synthetic `[package.metadata.lihaaf]`, running an argv-only
  baseline `cargo test`, discovering trybuild fixtures via syn AST walk,
  converting fixtures to `<compat_root>/target/lihaaf-compat-converted/{compile_pass,compile_fail}/`,
  invoking `lihaaf::run` in-process for the inner session, capturing
  the active toolchain via `rustup show active-toolchain`, and writing
  the §3.3 envelope atomically.
- **§5 pilot gate primitives** (`src/compat/gate.rs`): `Ceiling`,
  `GateOutcome::{Allow, NotEnrolled, Block}`, `parse_baseline`,
  `load_baseline`, `check_gate`. Gate enforces 5 rules from
  `docs/compatibility-plan.md:239-244`: errors empty, mismatch_count
  ≤ N_<crate>, both baseline+lihaaf exit codes equal expected,
  per-side totals match (with `excluded_fixtures` accounting).
- **§5 baseline schema**: `compat/baseline.toml` (top-level table per
  crate with `n_max: u32` + optional `expected_exit_code: Option<i32>`).
  Ships empty for v0.1.0-beta.4.
- **`compat/KNOWN_DIFFS.md`**: documents tracked divergences and v0.2
  resolution paths (wrapper-vs-per-fixture totals; Windows read-only
  cleanup; baseline.toml schema versioning).
- **CI workflow `lihaaf-compat-gate.yml`** (dry-run for beta-4): runs
  on `pull_request_target` against `compat/**` changes; BASE-ref-only
  checkout via `${{ github.event.pull_request.base.sha }}`;
  `permissions: contents: read`; embedded Python `tomllib` validator
  asserts `baseline.toml` schema. No untrusted code execution in
  elevated context.
- **Doc-hidden re-exports** (`src/lib.rs`): `CompatEnvelope` + 9 nested
  types + `compat_check_gate` / `compat_parse_baseline` /
  `compat_load_baseline` / `CompatGateCeiling` / `CompatGateOutcome`
  for the in-tree `gate_smoke` integration test and future out-of-tree
  CI runners.
- **`compat::cli::CompatArgs`** + 7 compat-mode CLI flags
  (`--compat`, `--compat-root`, `--compat-report`, `--compat-manifest`,
  `--compat-commit`, `--compat-filter`, `--compat-trybuild-macro`,
  `--compat-cargo-test-argv`). `Cli::validate_mode_consistency` blocks
  cross-mode mistakes at parse time.
- **Race-free path removal** (`src/util.rs::remove_path_race_free`):
  shared `remove_file` → `remove_dir` → `remove_dir_all` cascade with
  `#[cfg(windows)]` arms for ACCESS_DENIED dispatch. Used by
  `compat::cleanup`, `dylib::copy_dylib`, `dylib::symlink_dylib`,
  `session` `--no-cache` cleanup.

### Changed
- **Normalizer** (`src/normalize.rs`): added `compat_short_cargo: bool`
  flag and `$CARGO/<crate>-<ver>/...` short-form rewrite (§3.2.2 spec).
  Plumbed through `pub(crate) Cli::inner_compat_normalize` (`#[arg(skip)]`)
  + `NormalizationContext::with_compat_short_cargo` builder; compat
  driver flips the flag for inner-session normalization.
- **`session.rs` `--no-cache`**: replaced two `if .exists() { let _ = ... }`
  stat-then-act blocks with `util::remove_path_race_free` calls;
  errors now propagate instead of being silently swallowed.
- **`dylib.rs::copy_dylib` / `symlink_dylib`**: stat-then-act removal
  pattern replaced with `util::remove_path_race_free`.

### Fixed
- TOCTOU race in `compat::cleanup::remove_path_best_effort` (the
  stat-then-branch pattern that could follow a freshly-planted symlink
  out of the intended target tree).
- Linux EACCES misclassified as Windows ACCESS_DENIED in the cleanup
  cascade — `PermissionDenied` arm now `#[cfg(windows)]`-only.
- Windows read-only non-empty directory regression in cleanup step 2
  (RemoveDirectoryW returns `PermissionDenied`; now falls through to
  step 3).
- `compat::run` no longer pushes `baseline_unknown` into
  `envelope.errors[]`; the §5 gate's errors-empty rule consequently
  doesn't fire on the libtest wrapper line that every trybuild adopter
  produces.
- `rustup show active-toolchain` failure no longer short-circuits the
  envelope write; failures flow into `envelope.errors[]` as
  `toolchain_capture_failed` and the envelope is still emitted.
- `compat::run` calls `rustup` with `current_dir(&compat_root)` so the
  pilot fork's `rust-toolchain.toml` resolves correctly.
- `fixture_convert::convert_fixtures` removes the prior converted tree
  before recreating, eliminating stale `.stderr` snapshots that could
  corrupt re-run verdicts.
- TOCTOU in `fixture_convert.rs` stderr snapshot copy: replaced
  `src_stderr.exists()` pre-check with `match remove_file → ErrorKind::NotFound`.
- §5 gate now honors `expected_exit_code: Option<i32>` per crate row —
  pilots with documented non-zero baseline exits can pass the gate.
- `fixture_dirs` in the synthetic metadata block now points at the
  `compile_pass/` and `compile_fail/` child directories (lihaaf's
  discovery is non-recursive) and emits repo-relative forward-slash
  strings (byte-deterministic across Linux/macOS/Windows). Previous
  code pointed at the parent and used `to_string_lossy()` (platform-
  dependent backslashes on Windows).

### Internal
- **5 rounds of adversarial review** converged on triple-ALLOW. Codex
  (xhigh) + Gemini 3.1-pro-preview (plan mode) + strict-swe (Opus,
  --effort max for round 3+; Sonnet for rounds 1-2). Round 1 surfaced
  8 critical bugs (incl. the fixture_dirs / non-recursive discovery
  mismatch that defeated the entire driver, and the `compat_short_cargo`
  flag never reaching the inner session); rounds 2-5 progressively
  surfaced smaller real bugs as family-completeness sweeps deepened.
- Strict-swe was escalated from Sonnet to Opus mid-cycle after Sonnet
  missed Codex's critical fixture_dirs finding in round 1 — the diff
  needed multi-file context Sonnet couldn't sustain.
- Codex round-3 caught the deepest finding of the cycle: the
  `unknown_count == 0` gate rule that was nominally removed in round-2
  commit `89fec16` was still firing via `errors.is_empty()` because
  `compat::run` was still pushing `baseline_unknown` into `errors[]`.
  Type-system fix in round-4 commit `e851d94`: `assemble_diagnostic_errors`
  helper's signature CANNOT take `unknown_count` as a parameter.
- Test parallelism: added `static SPAWN_LOCK: Mutex<()>` in
  `tests/compat/cli_mode_errors.rs` to serialize cargo-lihaaf
  subprocess spawns within the binary (root cause of WSL2 global OOM
  observed in the development cycle).
- Two `compat_run_accepts_*` tests in `cli_mode_errors.rs` rewritten
  at parser layer using `Cli::try_parse_from` — no longer spawn
  cargo-lihaaf against the lihaaf repo itself (Phase-1-stub-era
  assumption broken by the real Phase-10 driver).

## [0.1.0-beta.3] — 2026-05-14

Follow-up simplification pass closing the three deferred Codex SIBLING
findings from the v0.1.0-beta.2 adversarial review. No adopter-visible
behavior change — diagnostic text, public API surface, exit codes,
snapshot byte format, and `[package.metadata.lihaaf]` schema are
unchanged from v0.1.0-beta.2.

### Changed
- **Test consolidation (config):** Sixteen invalid-TOML tests in
  `src/config.rs::tests` now share an
  `assert_parse_rejects_with(toml, &[expected_substrings])` helper.
- **Test consolidation (normalize):** Eleven text-handling tests in
  `src/normalize.rs::tests` now share an
  `assert_normalizes(input, expected)` helper that uses the standard
  `/p` workspace + `/r` sysroot + `/p/x` fixture-directory triplet.
  Path-rewriting tests with custom context and long-type-note tests
  with multi-`contains` assertions retain their original setup.
- **Test consolidation (session):** All four `derive_crate_root_*`
  tests in `src/session.rs::tests` now share an
  `assert_derive_crate_root_equals(input, expected)` helper that
  also asserts the issue-14 empty-path invariant. Previously only
  two of the four tests carried the explicit guard; the helper
  uniformizes the family.

### Internal
- Codex's beta-2 round-2 adversarial review surfaced three
  informational SIBLING findings (config / normalize / session test
  parameterization opportunities). All three are now addressed
  across four atomic commits.
- Adversarial review for beta-3: Codex + Gemini 3.1-pro-preview +
  Sonnet-tier strict reviewer, all ALLOW on round 1.

## [0.1.0-beta.2] — 2026-05-13

Retro simplification pass against the v0.1.0-beta.1 surface. No
behavior change visible to adopters. Internal cleanup only — diagnostic
text, public API surface, exit codes, snapshot byte format, and
`[package.metadata.lihaaf]` schema are unchanged from v0.1.0-beta.1.

### Changed
- **Test consolidation:** Four freshness drift tests (`release_line`,
  `host`, `commit_hash`, `sysroot`) now share an
  `assert_only_field_drifts(field, mutate)` helper that anchors to
  live `rustc` and asserts both the named field appears in the
  changed-fields diagnostic prefix AND the other three do not. The
  `release_line` test previously used a placeholder toolchain and
  skipped the absence-assertion; the new shape strengthens its
  regression bite.
- **Test consolidation:** Four `toolchain::matches` comparator tests
  (`release_line`, `host`, `commit_hash`, `sysroot`) now share an
  `assert_field_mutation_differs(mutate)` helper, mirroring the
  freshness helper above so the same parameterization pattern applies
  to both layers of the four-field comparator.
- **Platform-duplicate test removed:** Windows-only
  `synchronous_release_on_windows_drop` deleted from `lock.rs`. The
  cross-platform `drop_releases_lock_for_same_process_reacquire`
  already bites the same regression on Windows CI; the survivor's
  rustdoc now explicitly documents the Windows-specific behavior.
- **Corpus macro dedup:** The `corpus_error` and
  `corpus_error_with_n_lines` procedural macros in the integration
  corpus now share an `emit_compile_error(body)` helper for the
  shared 4-token `compile_error!(<body>);` emission sequence.
- **Corpus loop clarity:** `corpus_oom_allocate` now uses
  `buf.chunks_mut(4096)` instead of a manual stride index for
  page-touching. LLVM-equivalent codegen; reads as the documented
  intent.

### Internal
- Dead `_ensure_dir_exists` helper deleted from `session.rs` (was
  `#[allow(dead_code)]` + `_`-prefixed double signal; zero callers).
- Broken comment fragment in `compute_parallelism` fixed.
- `FreshnessFailure::RustcDrift` now documents the `Box<Toolchain>`
  × 2 rationale (clippy `result_large_err` mitigation; unboxing
  re-trips the lint).
- 43 simplify-pass findings reviewed (Reuse 4 / Quality 27 /
  Efficiency 12). 8 high-certainty items applied across 4 atomic
  commits; 11 lower-certainty items deferred; 1 (`EFFICIENCY-1`)
  retained per the existing module-level rationale (defense-in-depth
  against in-session toolchain swap).
- Triple-reviewer adversarial panel: Codex + Gemini 3.1-pro-preview +
  Sonnet-tier strict reviewer. Family-completeness sweeps confirmed
  no remaining sibling sites in the crate.

## [0.1.0-beta.1] — 2026-05-13

First public beta. Adopters who pinned `0.1.0-alpha.4` should upgrade
straight to `0.1.0-beta.1`. The Rust library surface has narrowed (see
`### Changed`); adopters who imported internal module paths
(`lihaaf::dylib::*`, `lihaaf::worker::*`, etc.) must switch to the new
crate-root re-exports or to subprocess-spawning `cargo lihaaf`. The
CLI surface, exit codes, verdict catalog, snapshot byte format, and
`[package.metadata.lihaaf]` schema are unchanged from `0.1.0-alpha.4`.

### Added
- Local pre-commit guard (`scripts/scan-secrets.sh`) that scans staged diffs
  for credential-like patterns: database URLs with embedded credentials,
  environment-variable assignments with secret-shaped keys, private key
  blocks, and AWS access keys. Lines containing `<placeholder>` syntax are
  treated as documentation examples and skipped. Install per-clone via
  `scripts/install-pre-commit-hook.sh`; bypass for legitimate false positives
  with `git commit --no-verify`. `scripts/run-scan-tests.sh` exercises the
  scanner against 4 positive and 2 negative fixture files and is wired into
  CI as a step. `SECURITY.md` documents the pattern set, placeholder
  convention, bypass mechanism, and reporting contact.
- macOS and Windows RSS sampling for `MEMORY_EXHAUSTED` attribution
  (KR-5, FIX_BEFORE_BETA Spec C, issue #6). `sample_rss_kib` now uses
  `libc::proc_pidinfo(PROC_PIDTASKINFO)` on macOS and
  `OpenProcess` + `GetProcessMemoryInfo` on Windows, matching the
  existing Linux `/proc/<pid>/statm` semantics. The §5.4
  dynamic-parallelism cap reduction now fires on all three platforms
  after every harness-attributed `MEMORY_EXHAUSTED` kill; on other
  Unixes the OS OOMkiller backstops as before. Two additional Windows
  features (`Win32_System_Threading`, `Win32_System_ProcessStatus`)
  are added to the existing `windows-sys 0.59` dependency.
- Multi-suite configuration. Adopters can declare additional named
  feature-subset suites with `[[package.metadata.lihaaf.suite]]` array
  entries (each with `name`, `features`, `fixture_dirs`, and optional
  `extern_crates` / `dev_deps` / `edition` / `compile_fail_marker` /
  `fixture_timeout_secs` / `per_fixture_memory_mb` overrides), and
  each suite triggers an independent dylib build with that suite's
  feature set propagated to per-fixture rustc invocations. Per-suite
  manifests live at `target/lihaaf/manifest-<name>.json` and per-suite
  cargo target dirs live at `target/lihaaf-build-<name>/`. The default
  suite (built from the top-level `[package.metadata.lihaaf]` table)
  retains the legacy `target/lihaaf/manifest.json` and
  `target/lihaaf-build/` paths so adopters who never add a named suite
  see no cache-key change. New CLI flag `--suite NAME` (repeatable)
  limits the run to the named subset; without `--suite`, every defined
  suite runs in declared metadata order. Fixture directories must be
  disjoint across suites — validated at config parse time so two
  suites cannot collide on snapshot files. The new
  `Manifest::suite_name` field carries the suite identity for
  out-of-band tooling; manifests written by lihaaf <0.1.0-alpha.3
  default the field to `"default"` on read so legacy on-disk state
  keeps deserializing.
- Self-test corpus addition: a new `tests/lihaaf/compile_pass_suite_demo/`
  fixture and `[[package.metadata.lihaaf.suite]] name = "suite_demo"`
  entry in lihaaf's own Cargo.toml. The fixture references
  `lihaaf::SUITE_DEMO_MARKER` (a const exposed only when lihaaf is
  built with `--features suite_demo`); CI runs lihaaf against itself
  and any regression that drops feature propagation between the dylib
  build and the per-fixture rustc fails to link, biting the
  multi-suite invariant without needing a downstream adopter.

### Changed
- New CI gate: `tests/integration_corpus/`, a real-adopter-shaped
  integration corpus that exercises each lihaaf verdict class
  (`OK`, `SNAPSHOT_DIFF` with `LARGE_SNAPSHOT` warning,
  `SNAPSHOT_MISSING`, `TIMEOUT`, `MEMORY_EXHAUSTED`) end-to-end against
  a real proc-macro crate (FIX_BEFORE_BETA Spec D). The corpus uses a
  two-crate `serde + serde_derive` layout — a regular library
  (`integration_corpus`, which lihaaf builds as the `dylib_crate`)
  plus a sibling proc-macro crate (`integration_corpus_macros`,
  resolved out of the dylib's deps dir as an extra `--extern`). The
  proc-macro crate cannot itself be `dylib_crate` because cargo
  rejects `[lib] proc-macro = true` with `--crate-type=dylib`. The
  corpus is `publish = false` and is excluded from lihaaf's own
  package via the new root `[package].exclude` entry. CI runs the
  corpus after the existing self-test step and asserts each verdict
  label fires via four `grep -q` gates against the captured lihaaf
  output — any regression that renames or drops one of those labels
  (or the `LARGE_SNAPSHOT` warning) flips CI red here even if the
  rest of lihaaf still builds clean.
- Narrowed the public Rust library surface to a documented CLI-shape.
  The following modules — previously `pub mod` — are now `pub(crate)`:
  `diff`, `discovery`, `dylib`, `error`, `freshness`, `manifest`,
  `normalize`, `session`, `snapshot`, `toolchain`, `util`, `worker`.
  The `cli`, `config`, `exit`, and `verdict` modules remain `pub` (they
  define the v0.1 stable schema/catalog/argument-parsing contracts).
  Adopters who want a Rust-callable surface should use the new
  crate-root re-exports: `Cli`, `Config`, `Verdict`, `ExitCode`,
  `Error`, `Outcome`, `run`, `Report`. `Outcome` is part of the v0.1
  stable Rust surface because `Error::Session(Outcome)` is a public
  variant of the re-exported `Error` enum; Rust's E0446 rule (private
  type in public interface) makes `Outcome`'s public visibility
  load-bearing, so the re-export ratifies the de-facto contract.
  Pre-1.0 alpha precedent: v0.1.0-alpha.4 is the only published
  version and the CHANGELOG header already states the library API is
  non-stable across v0.1.x; adopters who imported internal module
  paths (`lihaaf::dylib::*`, `lihaaf::worker::*`, etc.) should switch
  to subprocess-spawning `cargo lihaaf` or to the crate-root
  re-exports above. Issue #3.
- The session reporter prints `lihaaf: === suite "<name>" ===` headers
  and per-suite aggregate lines (`lihaaf: suite "<name>": …`) when
  more than one suite runs in a session. Single-suite runs (adopters
  who never add a `[[suite]]` entry) keep their legacy output
  byte-identical: no header, no per-suite line, just the existing
  `lihaaf: <n> ok, …` final aggregate.
- `Err(Error::Session(outcome))` paths in `cargo-lihaaf` now print the
  outcome's diagnostic message to stderr before the exit-code mapping.
  Pre-v0.1.0-alpha.3 the binary exited with the right code but
  silently dropped the diagnostic body — adopters had to consult the
  exit-code table to interpret a session-level failure. The fix is
  behavior-only; the message bodies are unchanged from the existing
  `Display` impl on `Outcome`.
- `WorkerContext::new` signature changed: it now takes
  `dylib_crate: &str` and `suite: &Suite` in place of `&Config`.
  Library API (pre-1.0) — adopters who subprocess-spawn `cargo
  lihaaf` are unaffected. Adopters wiring lihaaf as a library
  (currently lihaaf's own bin and tests) update their call sites; the
  per-suite identity is what the worker now closes over.

### Fixed
- Concurrent `cargo lihaaf` invocations sharing a
  `CARGO_TARGET_DIR` are now serialized via a session-wide advisory
  file lock at `target/lihaaf/.session.lock`. Previously, two
  sessions could collide on `target/lihaaf/manifest-<suite>.json`,
  `target/lihaaf-build-<suite>/`, or the managed-dylib copy —
  most loudly when one side passed `--no-cache` and unconditionally
  deleted the other's mid-read state — and the race surfaced as
  intermittent `DylibBuildFailed`, `DylibNotFound`, spurious
  `FreshnessDrift` on `managed_dylib_path` / `dylib_sha256`, or
  `MalformedDiagnostic` on a partially-deleted snapshot. The lock
  is acquired after the `--list` short-circuit (so `--list` does
  not block) and before the `--no-cache` deletion sweep, and held
  for the remainder of the session. On contention, lihaaf emits
  `lihaaf: waiting for another lihaaf session to release <path>
  ...` to stderr BEFORE blocking, and (if the wait exceeded 50 ms)
  follows up with `lihaaf: acquired session lock after N ms` after
  the lock is granted; single-session runs hit the fast path and
  emit neither line. Cross-platform via `libc::flock(LOCK_EX)` on
  Unix (in an `EINTR` retry loop) and `LockFileEx` on Windows
  (with an explicit `UnlockFileEx` in `Drop` so a fast back-to-back
  same-process re-acquire does not race the OS's deferred
  `CloseHandle` release). Crash recovery is automatic: a process
  that dies holding the lock releases it via OS handle cleanup,
  no stale-lockfile removal needed. New Windows dependency
  `windows-sys 0.59` under `cfg(windows)` only; Unix uses the
  existing `libc` dep. Issue #1.
- Toolchain drift comparator now widens its key to
  `(release_line, host, commit_hash, sysroot)`. The previous
  comparator compared only `release_line`, so two materially different
  toolchains — e.g. rustup stable rustc 1.95.0 vs. a custom local
  build with the same release line, or the same release line on a
  different host triple — compared equal and bypassed the policy
  §4.5 hard-fail. The widening only catches MORE drift, never less,
  so existing pass cases stay passing. Known caveat: when `rustc` is
  a custom local build, `commit-hash:` is absent and `commit_hash`
  is the empty string; two such builds with the same other fields
  still compare equal on that field. Users running custom rustc
  builds operate outside the stable-channel safety net by design;
  the `sysroot` comparison usually catches this in practice. Issue
  #4.
- Session temp directory creation now creates the workspace target
  parent directory first. Clean CI checkouts that run lihaaf before any
  other crate-local Cargo command no longer fail with
  `No such file or directory` while creating
  `<crate>/target/lihaaf-session-*`.
- Per-fixture `rustc` invocations now set `CARGO_MANIFEST_DIR` to the
  consumer crate root (the directory containing the consumer's
  `Cargo.toml`). Cargo sets this automatically on `cargo build` /
  `cargo test`, but lihaaf's per-fixture rustc spawns bypass cargo,
  so it had to be supplied explicitly. Without it, any proc macro
  that calls `proc_macro_crate::crate_name("...")` (the dominant
  pattern for renamed-dependency resolution — used by `serde`,
  `inventory`, and most modern derive macros) failed at
  macro-expansion time with `` `CARGO_MANIFEST_DIR` env variable not
  set ``, blocking the compile-fail / compile-pass assertion before
  it could run. Issue #14.
- Relative `--manifest-path` values (notably the bare
  `--manifest-path Cargo.toml`) are now absolutized at session
  startup before the crate-root derivation runs. Previously,
  `Path::parent` of a single-component relative path returned
  `Some("")` instead of `None`, so the
  `.unwrap_or_else(|| ".".into())` fallback was bypassed and
  `CARGO_MANIFEST_DIR=""` could propagate to the per-fixture rustc —
  a path shape Cargo itself never emits. The fix matches Cargo's
  shape exactly (absolute, no symlink resolution). Issue #14 follow-up.

## [0.1.0-alpha.1] — 2026-05-11

First public release on crates.io. Pre-1.0 alpha — the CLI surface
(flag names, exit codes, verdict catalog, `manifest.json` schema, and
`[package.metadata.lihaaf]` schema) is the stable v0.1 contract; the
library API is non-stable and may shift across v0.1.x. Adopters
should subprocess-spawn `cargo lihaaf` rather than depend on
`lihaaf::*` paths from Rust.

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
- Spec §4.5 amended to acknowledge the v0.1 hard-fail policy on
  freshness divergence and to defer the in-session rebuild path to
  v0.2. The four §4.5 invariants now share §4.6's hard-fail behavior
  (exit code 67) explicitly. Previously the spec mandated rebuild
  while the implementation hard-failed; this brings the spec text in
  line with shipping behavior + the deferral note in
  `src/freshness.rs` rustdoc. (Codex delta-review A3.)

### Pending before v0.1.0 release
- (Resolved in `0.1.0-beta.1`) macOS / Windows RSS sampling APIs are
  not yet wired (KR-5).
