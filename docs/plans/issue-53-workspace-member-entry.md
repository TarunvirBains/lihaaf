# ⚠️ TEMPORARY ARTIFACT — DELETE AFTER POST-IMPLEMENTATION REVIEW-ALLOW

This is a **pre-implementer-dispatch design plan**, NOT durable repository documentation.

- Lives on branch `docs/v01-plan-artifacts-53` for the duration of the implementer-dispatch + adversarial-review cycle for lihaaf #53.
- The implementer's PR (target: `careful-coder` Opus) MUST `rm docs/plans/issue-53-workspace-member-entry.md` as part of its diff so this file does not land on `main`.
- If the implementer's PR is reviewed and ALLOWed but this file is still present, the post-merge cleanup must remove it before the next release branch cuts.
- This plan is currently at R2 awaiting adversarial review (R1 returned Codex `VERDICT: BLOCK` with a 30-item class-enumeration sweep on 2026-05-18). Codex xhigh (or equivalent) ALLOW required before implementer dispatch.

---

# Plan: lihaaf #53 — workspace-MEMBER subdirectory entry (`-p <package>`)

Revision: R2 (2026-05-18, post-Codex-R1-BLOCK + class-enumeration sweep)

Issue: https://github.com/TarunvirBains/lihaaf/issues/53 (compat: implicit-ancestor REJECT fires on workspace-MEMBER subdirectory entry — axum-macros blocker)

Status entering: beta.8 + #40+#47 landed (Option H 4-rule self-patch policy + staged package-root mirror at commit `cb5d5bf`; see PR #56). PR #37 R4 implicit-ancestor REJECT is the over-broad guard that blocks the legitimate workspace-member entry shape.
Status target: v0.1.0 (per [[lihaaf-v01-ga-gate]] 2026-05-18 — #53 is the final v0.1.0 blocker after #40, #43, #47; closes Round-2 enrollment of axum-macros).

Target implementer: `careful-coder` Opus, max effort. Reason: change spans CLI surface (`src/cli.rs`), compat-args projection (`src/compat/cli.rs`), compat driver wiring (`src/compat/mod.rs`), overlay resolver (`src/compat/overlay.rs` — new function + REJECT-branch interaction), spec amendment, compat plan amendment, integration tests, corpus expansion, and CHANGELOG. Sonnet variant is wrong tier — this touches the entry-point boundary and the REJECT-branch interaction is design-judgment-heavy.

Target branch (for the implementer to cut): `feat/issue-53-workspace-member-entry` from `main` at `cb5d5bf` or later (PR #56 #40+#47 Option H + staged-mirror MUST be on `main` first — pre-R2 was uncertain about ordering; R2 confirms #40+#47 are landed and removes the conditional branch in §11.9).

Working dir: `/home/tarunvir/projects/lihaaf`.

---

## Revision history

- **R1 (2026-05-18, initial)** — drafted by strict-swe Opus planner. Sections 1-12 written together; sent to Codex xhigh for adversarial review (per [[lihaaf-plan-adversarial-cycle]]).
- **R2 (2026-05-18, post-Codex-R1-BLOCK + sweep-after-review)** — Codex R1 returned `VERDICT: BLOCK` with 6 HARD_BLOCKs + 4 FIX_BEFORE_IMPL findings; the subsequent class-enumeration sweep produced a 30-item punch list across the 6 BLOCK classes (instances + inverses + adjacents). R2 deltas:

  - **BLOCK-1 fix (dual-root plumbing).** R1 conflated one `compat_root` path for five distinct consumer roles (overlay tracking, discovery, fixture conversion, baseline sidecar, baseline cwd / `Cargo.lock` discovery). For workspace-member entry these roles split between two roots: the WORKSPACE root (baseline cwd, `Cargo.lock` discovery) and the MEMBER root (overlay, discovery, fixture conversion). R2 adds **§3.1.bis "Root vocabulary"** defining `workspace_root`, `member_root`, `workspace_root_manifest`, `member_manifest`, `overlay_root`, and a per-consumer routing table (PUNCH-1, PUNCH-2, PUNCH-3, PUNCH-4). All downstream sections (§4.2 driver wire-up, §4.5 carries-through, §5.4 context-struct shape) updated to consume the dual-root contract.
  - **BLOCK-2 fix (root-relative path absolutization).** R1's "carried verbatim" wording for workspace-root tables is wrong for path-bearing keys: `[workspace.dependencies.foo].path = "crates/foo"` carried verbatim into an overlay rooted at `<member>/target/lihaaf-overlay/Cargo.toml` would re-anchor relative paths against the overlay dir, not the workspace root — broken. R2 adds **§3.2.bis "Workspace-table path policy"** with a 4-column classification (workspace-root path / member-root path / URL or non-path string / non-copied) for every table key the plan carries down (PUNCH-5 through PUNCH-11). Path-bearing keys are absolutized against `workspace_root` before write into the overlay; non-path keys carried verbatim.
  - **BLOCK-3 fix (Option H composition order).** R1 left ambiguous what happens when the workspace ROOT carries `[patch.crates-io]` and Option H Rule 1 (INJECT into member's self-patch slot) runs first — a workspace-root self-patch entry covering the member could be hidden by Rule 1's INJECT. R2 adds **§5.3.bis "Option H composition order — root-first, member-second"** specifying: (a) Cargo only reads `[patch.crates-io]` from the workspace ROOT (member-local `[patch.crates-io]` is a cargo error — we match by erroring); (b) the effective patch table is computed from the workspace root's `[patch.crates-io]` with path entries absolutized against `workspace_root` FIRST, then Option H's 4 rules run against the MERGED effective table (PUNCH-12, PUNCH-13). §5.3.bis also documents `[patch.<registry>]` scope-out (PUNCH-14), `[replace]` handling (PUNCH-15), and `[profile.*]` precedence (PUNCH-16).
  - **BLOCK-4 fix (resolver glob expansion).** R1's resolver supported only top-level directory-name globs (`axum-*`), missing the common axum-style `members = ["crates/*"]` layout. R2 expands **§4.3 step 5 (Glob expansion details)** to support: single-segment-with-slash globs (`crates/*`), explicit nested literal paths (`crates/foo`, `tools/bar`), absolute-path rejection, trailing-slash normalization, deep-glob rejection (`**/*` returns directed error) (PUNCH-17, PUNCH-18, PUNCH-19, PUNCH-20).
  - **BLOCK-5 fix (`workspace.exclude` + `default-members` + nested workspaces + duplicates).** R1's resolver matched against `[workspace.members]` only, missing `[workspace.exclude]` subtraction, `[workspace.default-members]` semantics, nested-workspace traversal boundaries, and ambiguity behavior on duplicate package names after expansion. R2 adds **§4.3 step 3.5 "Exclude subtraction"** + **§4.3 step 4.5 "Nested-workspace traversal boundary"** + **§4.3 step 7.5 "Default-members non-interaction"** + **§4.3 step 8 "Duplicate package names after expansion"** (PUNCH-21, PUNCH-22, PUNCH-23, PUNCH-24).
  - **BLOCK-6 fix (test coverage for new classes).** R1's §7 tests (23 unit + 1 integration) missed coverage for the 6 BLOCK classes' instances + inverses. R2 adds 18 new tests to **§7.2** (now 41 unit tests + 1 integration test + corpus addition), grouped by the 6 BLOCK classes' instances + inverses + adjacents: dual-root routing (T-24), workspace-root path absolutization (T-25, T-26), Option H composition with root self-patch + member-local `[patch]` rejection (T-27, T-28), glob/path resolver coverage (T-29 through T-35), exclude / default-members / nested / duplicate coverage (T-36 through T-40), and the package+workspace root rejection per §1 scope (T-41) (PUNCH-25 through PUNCH-30). §7.7 + §10b mirror table extended (rows 47-70).
  - **FIX_BEFORE_IMPL #1 (stale "#40+#47 not landed" branch).** PR #56 (Option H + staged-mirror) is merged on `main` at `cb5d5bf`. R1 §11.9 had a conditional branch "if #40+#47 has NOT landed, the carry-down for `[patch.crates-io]` is simpler". R2 removes the conditional; #40+#47 is the new floor.
  - **FIX_BEFORE_IMPL #2 (spec amendments accuracy).** R2 §8.1 / §8.2 / §8.3 spec-amendment text rewritten to reference the dual-root contract (§3.1.bis), path-policy table (§3.2.bis), Option H composition order (§5.3.bis), and the v0.1.0 virtual-workspace-only scope (§1).
  - **FIX_BEFORE_IMPL #3 (status references).** Updated beta.6 / `586cc68` → beta.8 / `cb5d5bf` throughout (top of plan, §11.9, dispatch shape).
  - **FIX_BEFORE_IMPL #4 (virtual-vs-package+workspace scope).** R1 was silent on workspace-root-shape scope. R2 **scopes v0.1.0 to VIRTUAL workspaces only** (workspace root declares `[workspace]` without `[package]`; this is the axum / tokio / clap shape and the only case Round-2 needs). Package+workspace shape (root has both `[package]` and `[workspace]`, e.g. some smaller multi-crate repos like `serde_with`) is documented as a v0.2 / v1.0 follow-up with explicit rationale in §1 ("Out of scope") and §11.11. Verified against axum: tokio-rs/axum's root `Cargo.toml` is a virtual workspace (https://github.com/tokio-rs/axum/blob/main/Cargo.toml — `[workspace] members = ["axum", "axum-*"]`, no `[package]`).
  - **DEFERRABLE_NIT (§10b row 32 stale `R5` marker).** R1 §10b row 32's grep target string `Workspace-member entry via `--package` (R5 / issue #53)` carried a stale `R5 /` prefix from a copy-paste residue of the #40+#47 R8 plan structure. Removed; this plan is at R2 and the module-level rustdoc text now reads `Workspace-member entry via `--package` (issue #53)` (matching §8.5).
  - **INTERNAL-CONSISTENCY surface (corpus shape correction).** While addressing BLOCK-6, R2 discovered R1 §7.4 invented a TOML schema (`[[entry]]` tables in `compat/baseline.toml`) that does not match the actual code. The byte-determinism corpus lives at `tests/compat/overlay_corpus/<name>.input.toml` + `<name>.expected.toml` (file-pair fixtures); the test (`byte_identical_across_two_lihaaf_binaries_on_corpus` at `tests/compat/overlay_determinism.rs:432-499`) iterates a hardcoded `names` array of 6 fixtures and asserts `checked == 6`. R2 §7.4 rewritten to add a `workspace_member_with_package` fixture pair to that directory and bump the hardcoded names array + count assertion from 6 to 7. `compat/baseline.toml` is the §5 pilot-gate baseline table (per-pilot `n_max` ceilings), unrelated to the corpus. R1's §7.4 description was structurally wrong; R2 corrects to match repo reality. This is in-scope for BLOCK-6 (test coverage) and is not a scope expansion.

  Per [[sweep-after-review]] discipline: BLOCK-class findings + 30-item punch list were sourced from a single Codex xhigh adversarial pass; R2 addresses ALL of them (no carry-forward to R3 unless Codex R2 surfaces a new class).

---

## 1. Target + scope

**Implementer tier:** `careful-coder` (Opus, max effort).

**Closes:** lihaaf #53.

**v0.1.0 blocker:** yes (per [[lihaaf-v01-ga-gate]] 2026-05-18 milestone correction). #53 is the final entry-point gap blocking Round-2 enrollment of axum-macros; without it the Round-2 coverage matrix is incomplete and v0.1.0 GA cannot cut.

**Out of scope for this PR (explicit, surfaced to head off Codex misclassification):**

- Per-package overlay shape changes — the resolver maps `-p <pkg>` to a member manifest path; downstream overlay materialization (workspace-inheritance preservation, `[patch.crates-io]` self-patch policy) is unchanged for the member case and inherits the #40+#47 fixes (Option H 4-rule policy + staged package-root mirror) now landed on `main` at commit `cb5d5bf` (PR #56).
- Multiple-package selection — `-p` accepts a single package; `-p axum-macros -p axum-core` is rejected at CLI parse time. Cargo's own `-p` is also single-valued in `cargo rustc`; we match.
- Non-compat-mode `-p` support — the flag is compat-mode-only. The v0.1 surface outside compat mode does not need a package selector (lihaaf's non-compat mode already takes `--manifest-path` directly to the consumer crate). Adding `-p` to non-compat mode would be a v0.2 conversation.
- Workspace globs in `-p` — `-p axum-*` is rejected. Cargo's `-p` does not accept globs either; single literal package name.
- `Cargo.lock` consultation — the resolver reads `Cargo.toml` files only. The lockfile is the dylib build's concern, not the entry-point's. (Note: `Cargo.lock` discovery is the WORKSPACE root's role per §3.1.bis routing table, not the member root's.)
- Cross-workspace `-p` — `-p` is resolved within the SINGLE workspace rooted at `compat_root` or the workspace found by walking up from `compat_root` (one or the other; §4 picks the rule). Workspaces containing other workspaces (nested) are addressed in §4.3 step 4.5 + §6.1.
- **Package+workspace root shape (R2 scope decision).** v0.1.0 `--package` is scoped to **VIRTUAL workspaces only** — the workspace root manifest declares `[workspace]` and does NOT declare `[package]`. The package+workspace shape (root carries both `[package]` and `[workspace]`, treating the root as both a workspace coordinator AND a publishable package) is left to v0.2 / v1.0. **Rationale:** (a) the only Round-2 pilot needing `--package` is axum-macros, and `tokio-rs/axum` is a virtual workspace (`members = ["axum", "axum-*"]`, no root `[package]`; verified against https://github.com/tokio-rs/axum/blob/main/Cargo.toml as of 2026-05-18); (b) virtual workspaces are the canonical "multi-crate library" shape in the cargo ecosystem (clap, tokio, tonic, axum, hyper all use this); (c) package+workspace adds a meaningful complication — the root `[package]` is itself a candidate match for `--package <root-pkg-name>`, AND its `[lib]`/`[dependencies]`/`{ workspace = true }` keys interact with the same workspace tables the member would inherit, AND the question of whether `--package <root>` should target the root's `[package]` or REJECT (since the root is the workspace coordinator, not a member subdirectory) needs a clear policy. Punting to v0.2 lets us ship Round-2 GA without designing that policy now. Per §11.11 the v0.2 follow-up issue MUST be filed when this plan lands. The resolver MUST surface a directed diagnostic when `--compat-root` points at a package+workspace root (per §4.3 step 2.5 — new in R2).

**Acceptance criteria (verbatim from issue #53 with §-id refinements):**

1. `cargo lihaaf --compat -p <package>` resolves to the correct workspace-member manifest. Implementation: §3 CLI surface + §4 resolver.
2. axum-macros enrolls cleanly at stage 2 on the chosen lihaaf version. Integration test: §7 integration test (`cargo_lihaaf_resolves_axum_macros_shape_workspace_member`).
3. Test coverage: workspace-member entry shape added to `compat/baseline.toml` corpus. §7 corpus expansion.
4. Spec §X documents the new entry-point semantics. §8 spec amendment to `docs/spec/lihaaf-v0.1.md` §8.2 + `docs/compatibility-plan.md` §3.1 + §3.2.3.
5. CHANGELOG entry. §9 verifier list + §10b mirror table entry.

---

## 2. Failure analysis (current code path)

The over-broad REJECT fires inside `override_workspace_inheritance` at `src/compat/overlay.rs:750-865`. The full path from CLI to REJECT is:

### 2.1 CLI → resolve_upstream_manifest → materialize_overlay

1. **CLI parse** (`src/cli.rs:190-225` — `parse_from`). The adopter invokes `cargo lihaaf --compat --compat-root /path/to/tokio-rs/axum/axum-macros --compat-report /tmp/r.json`. clap parses; `validate_mode_consistency` (`src/cli.rs:262-313`) confirms compat-mode + required flags. No `-p` flag exists yet.
2. **CompatArgs projection** (`src/compat/cli.rs:133-179` — `CompatArgs::from_cli`). `compat_root` is absolutized; `compat_manifest` is `None` so the default path discovery applies.
3. **Driver entry** (`src/compat/mod.rs:80-86` — `compat::run`). The driver computes the upstream manifest via `resolve_upstream_manifest` (`src/compat/mod.rs:329-334`):
   ```rust
   fn resolve_upstream_manifest(args: &cli::CompatArgs) -> Result<PathBuf, Error> {
       if let Some(m) = &args.compat_manifest {
           return Ok(m.clone());
       }
       Ok(args.compat_root.join("Cargo.toml"))
   }
   ```
   In the axum-macros invocation, this returns `/path/to/tokio-rs/axum/axum-macros/Cargo.toml` — the workspace-MEMBER Cargo.toml.
4. **Overlay materializer** (`src/compat/overlay.rs:418-446` — `materialize_overlay_with_synthetic_metadata_builder`). Reads + parses the member manifest. The member has no `[workspace]` table (members never do — only the root does), so `is_workspace_root_manifest` (`src/compat/overlay.rs:1528-1535`) returns `false` and that REJECT branch does NOT fire here.
5. **`override_workspace_inheritance` is invoked** at `src/compat/overlay.rs:564` (`override_workspace_inheritance(top, upstream_manifest_path)?;`).

### 2.2 The REJECT branch that fires today

`override_workspace_inheritance` has FIVE branches (cited from `src/compat/overlay.rs:754-889`):

- **Branch 1 (explicit member):** `[package].workspace = "<path>"` present → REJECT (`overlay.rs:761-781`). axum-macros' manifest does NOT use this explicit form (it uses workspace inheritance keys like `rust-version = { workspace = true }` but not the explicit `package.workspace` pointer). Does not fire.
- **Branch 2 (implicit member via ancestor `Cargo.toml`):** No local `[workspace]` AND `detect_implicit_ancestor_workspace(upstream_manifest_path)?` returns `Some(ancestor_manifest)` → REJECT (`overlay.rs:800-825`). **THIS IS THE OVER-BROAD BRANCH.** axum-macros has no local `[workspace]`; the ancestor walk-up from `axum-macros/Cargo.toml` reaches `axum/Cargo.toml` which carries `[workspace] members = ["axum", "axum-*"]`. Branch 2 fires.
- **Branch 3 (implicit member via inheritance refs only):** No local `[workspace]` AND `manifest_has_inheritance_reference(top)` is true (catches the residual case where ancestor walk-up finds nothing parseable) → REJECT (`overlay.rs:844-865`). Branch 2 fires first in the axum-macros case; Branch 3 never runs.
- **Branch 4 (build overlay workspace):** Builds the overlay's `[workspace]` table by cloning the upstream's (if any) and stripping membership keys (`overlay.rs:867-889`). Not reached.
- **Branch 5 (idempotency belt-and-braces):** Idempotency strip (`overlay.rs:884-886`). Not reached.

### 2.3 Why Branch 2 is over-broad for the workspace-MEMBER entry case

The REJECT's rationale (cited verbatim from `overlay.rs:785-799`):

> "The ancestor workspace may carry `[patch.crates-io]`, `[replace]`, `[profile]`, `resolver`, or `[workspace.dependencies]` tables that affect baseline cargo's dependency resolution; the lihaaf overlay terminates cargo's walk-up at the staged manifest and skips the ancestor's state entirely, producing a divergent dependency graph between baseline and overlay and therefore false compat verdicts. The check is CONSERVATIVE: any ancestor workspace triggers rejection regardless of whether its `members` array explicitly names this manifest."

This reasoning is correct for invocations from a workspace-member subdirectory where the adopter did NOT explicitly opt in — those invocations may indeed be accidental (the adopter cd'd one level too deep). But when the adopter explicitly says "I am targeting this package within this workspace" (via a `-p <pkg>` flag), the entry shape is intentional and the REJECT's "accidental shadowing" hypothesis no longer applies.

The R4 review comment in `overlay.rs:716-723` itself acknowledges the gap:

> "None of the four Round-1 pilots (cxx, serde-json, anyhow, thiserror) invokes lihaaf from a workspace member — they all invoke from upstream ROOT (which carries both `[package]` and `[workspace]`, the workspace-root case) — so none of the rejections affect any currently-enrolled pilot. The R3 + R4 rejections are defense-in-depth for any future invocation from a workspace sub-crate. **The follow-up to enable workspace-member overlays (copying ancestor inheritance tables down) will land separately.**"

This plan IS that follow-up. The R4 design correctly identified the future-work scope; #53 is the trigger to land it.

### 2.4 Concrete axum-macros walk-up trace

Given `compat_root = /path/to/tokio-rs/axum/axum-macros`:

1. `resolve_upstream_manifest` returns `<compat_root>/Cargo.toml` = `/path/to/tokio-rs/axum/axum-macros/Cargo.toml`.
2. `detect_implicit_ancestor_workspace(/path/to/tokio-rs/axum/axum-macros/Cargo.toml)` (`overlay.rs:933-986`):
   - `manifest_dir` = `/path/to/tokio-rs/axum/axum-macros`
   - `current` = `manifest_dir.parent()` = `/path/to/tokio-rs/axum`
   - Iteration 1: `candidate` = `/path/to/tokio-rs/axum/Cargo.toml`. `read_to_string` succeeds; `toml::from_str` parses; `value.get("workspace").is_some_and(|v| v.is_table())` is `true`. Returns `Ok(Some(/path/to/tokio-rs/axum/Cargo.toml))`.
3. Back in `override_workspace_inheritance`: `has_local_workspace` is `false` (axum-macros has no `[workspace]` of its own), `detect_implicit_ancestor_workspace` returned `Some(...)`, Branch 2 fires.
4. Adopter sees the directed diagnostic at `overlay.rs:803-824`:
   > "error: `--compat-root` `<...>/axum-macros/Cargo.toml` is an implicit workspace member: it has no local `[workspace]` table but an ancestor manifest at `<...>/axum/Cargo.toml` carries `[workspace]`. … Either invoke `cargo lihaaf --compat` from the workspace ROOT (`<...>/axum/Cargo.toml` or its containing directory), or restructure the fork so the crate-under-test has no ancestor workspace."

The diagnostic does point the adopter at the workspace root — but the workspace root is ITSELF a workspace-root manifest (declares `[workspace]` without `[package]`), so invoking from there hits `is_workspace_root_manifest` REJECT (`overlay.rs:488-498`):

> "error: `--compat-root` must point to a single-crate Cargo.toml; `<...>/axum/Cargo.toml` is a workspace root (declares `[workspace]` without `[package]`). Pass a member crate's Cargo.toml as `--compat-root` instead."

Round-and-round: the member shape rejects to the root, the root shape rejects to a member. **There is no path through today's code for the axum-macros entry shape.** That is the bug.

### 2.5 The fix shape (Option A — `-p <package>` flag)

Per the issue body's recommendation: add `-p <package>` / `--package <package>` to the compat-mode CLI grammar. When supplied:

- The adopter invokes from the workspace ROOT (`--compat-root /path/to/tokio-rs/axum`).
- The resolver walks the workspace's `[workspace.members]` array (resolving globs against the workspace-root directory), reads each member's `Cargo.toml`, and picks the one whose `[package].name == <pkg>`.
- The resolved member manifest becomes the upstream manifest for the rest of compat mode (overlay materialization, fixture discovery, etc.).
- Branch 2 of `override_workspace_inheritance` is BYPASSED (the implicit-member REJECT is no longer over-broad — the adopter explicitly named the member).
- Branches 1, 3, 4, 5 still run normally on the resolved member's manifest.

The implementation surface is small (~150-220 LOC of new code + ~80-120 LOC of tests + ~30-50 LOC of doc updates). The risk surface is the REJECT-bypass interaction (§5) — a careless bypass could regress PR #37 R4's anti-shadowing defense for accidental workspace-member entries.

---

## 3. CLI surface (`-p <package>` / `--package <package>`)

### 3.1 New field on `crate::cli::Cli`

The new flag is added to the existing clap-derive struct `crate::cli::Cli` in `src/cli.rs:40-160`. Field placement: between `compat_root` (line 76) and `compat_trybuild_macro` (line 81), keeping the `compat_*` cluster alphabetical (`compat_*` field group ends at `compat_trybuild_macro`).

Exact field:

```rust
/// Compat-mode workspace-member package selector. When set, the
/// upstream manifest is resolved from the workspace rooted at
/// `--compat-root` by matching `<package>` against each member's
/// `[package].name`. Required when `--compat-root` points at a
/// workspace ROOT (declares `[workspace]` without `[package]`);
/// rejected otherwise (see `validate_mode_consistency`).
///
/// The short form `-p` mirrors cargo's `-p` convention. Multi-valued
/// is rejected at parse time (single package per invocation).
#[arg(short = 'p', long = "package", value_name = "PACKAGE")]
pub compat_package: Option<String>,
```

**Why `Option<String>`:** absence is the not-supplied marker; presence is the explicit-target marker. clap's `value_name = "PACKAGE"` makes `--help` print `-p <PACKAGE>` (matching cargo's display). Multi-match is NOT supported (no `Vec<String>` — clap will error on `-p A -p B` because the field is `Option<String>` not `Vec<String>`; clap's default behavior on a second occurrence is to overwrite, but the multi-valued shape would change the field type; we want the single-valued rejection at parse time, achieved by `Option<String>` + no `action = "append"`).

**Short-name collision check:** `src/cli.rs:91-95` defines `-j` for `--jobs` (positive-integer parser). `-q` is `--quiet` (`:128`), `-v` is `--verbose` (`:133`). No existing `-p`. `-h` and `-V` are clap built-ins. `-p` is free.

**Help-text wording (long-form `--help` output):**

> Compat-mode workspace-member package selector. Required when `--compat-root` points at a workspace root that declares `[workspace]` without `[package]`. The named package must appear in the workspace's `[workspace.members]` array (literal or glob-expanded match) and its manifest's `[package].name` must equal `<package>`. Conflicts with `--compat-manifest` (which supplies an explicit manifest path, bypassing the member-resolver). Mirrors cargo's `-p` convention.

### 3.1.bis Root vocabulary (R2 — BLOCK-1 fix, PUNCH-1 through PUNCH-4)

Pre-R2, the plan used the single term `compat_root` for five consumer roles. For workspace-member entry these roles split. R2 defines the vocabulary FIRST and binds every downstream section to it.

**Terms (used uniformly in the rest of the plan):**

| Term | Definition | Example (axum-macros via `-p`) |
|---|---|---|
| `workspace_root` | The directory the adopter passes via `--compat-root`. For `--package` invocations, this is the workspace ROOT directory (declares `[workspace]` without `[package]` per the v0.1.0 virtual-workspace-only scope, §1). For non-`--package` invocations, this is whatever directory the adopter named (member subdirectory, single-crate, etc.). | `/path/to/tokio-rs/axum/` |
| `workspace_root_manifest` | `<workspace_root>/Cargo.toml`. The TOML file the resolver parses to find `[workspace.members]`, `[workspace.exclude]`, `[workspace.dependencies]`, `[patch.crates-io]`, etc. | `/path/to/tokio-rs/axum/Cargo.toml` |
| `member_root` | The directory containing the resolved member's `Cargo.toml`. Computed as `member_manifest.parent().unwrap()`. For `--package axum-macros`, this is the `axum-macros/` subdirectory inside the workspace. For non-`--package` invocations (single-crate, etc.), `member_root == workspace_root`. | `/path/to/tokio-rs/axum/axum-macros/` |
| `member_manifest` | The resolved member's `Cargo.toml` path. The output of `resolve_workspace_member_manifest`. For non-`--package` invocations, `member_manifest == workspace_root_manifest`. | `/path/to/tokio-rs/axum/axum-macros/Cargo.toml` |
| `overlay_root` | The directory where the staged overlay's `Cargo.toml` is written. Always `<member_root>/target/lihaaf-overlay/`. The overlay's sibling `Cargo.toml` lives here. (Per #40+#47 R8 staged-mirror plan, the `target/lihaaf-overlay/` dir also carries the symlinked package-root mirror; that infrastructure is unchanged for #53.) | `/path/to/tokio-rs/axum/axum-macros/target/lihaaf-overlay/` |

**Invariant.** For non-`--package` invocations the three roots collapse: `workspace_root == member_root` and `workspace_root_manifest == member_manifest`. The plan's R1 single-root contract was correct for this case. R2's dual-root contract is the proper superset.

**Per-consumer routing table (PUNCH-2).** Every consumer that previously read `compat_root` now reads the explicit role:

| Consumer | Reads | Why | Code site (post-implementer) |
|---|---|---|---|
| Overlay materialization (`materialize_overlay_inner`) | `member_root` (parent dir) + `member_manifest` (parse target) | The overlay is staged AT the member, with all `[package]` / `[lib]` / `[dependencies]` keys coming from the member's manifest. | `src/compat/overlay.rs:418-611` (existing surface; takes `member_manifest` as input, derives `member_root` via `.parent()`) |
| Fixture discovery (`compat::discovery`) | `member_root` | `tests/*.rs` lives under the member, not the workspace root. The workspace root has no `tests/` directory of its own in the virtual-workspace shape. | `src/compat/discovery.rs` (takes `compat_root`-equivalent; rewires to `member_root`) |
| Corpus / fixture conversion (whichever stage rewrites `__UPSTREAM_DIR__`) | `member_root` | The byte-determinism corpus uses `member_root` as the substitution value (see §7.4 R2 correction — the corpus test lives at `tests/compat/overlay_determinism.rs:432-499` and substitutes `__UPSTREAM_DIR__` with the upstream dir, which for `-p` means the member dir). | `tests/compat/overlay_determinism.rs:478-483` (substitution call) |
| Baseline cargo cwd | `workspace_root` | `cargo test -p <pkg>` MUST run from the workspace root (cargo discovers `Cargo.lock` and the workspace state from the cwd; running from the member would either fail to locate the workspace OR pick up the member's local lockfile if any). | `src/compat/baseline.rs` (or whatever module spawns the baseline `cargo test`); takes a cwd, currently passes `compat_root` — rewires to `workspace_root` when `--package` is set, else `compat_root` per the collapse invariant. |
| `Cargo.lock` discovery (cargo internal) | `workspace_root` (implicit via baseline cwd) | Cargo writes `Cargo.lock` at the workspace root; both the baseline `cargo test` invocation and the dylib build (via overlay) consume this lockfile. The dylib build discovers it by walking up from the overlay manifest, which walks past `<member>/target/lihaaf-overlay/` → `<member>/target/` → `<member>/` → `<workspace_root>/` and finds `Cargo.lock` there. | Cargo's own behavior; no lihaaf code site. |
| Baseline sidecar (envelope sidecar files written next to `--compat-report`) | `--compat-report.parent()` — unchanged by #53 | The sidecar is anchored to the report path, not to any compat root. R2 surfaces this for completeness; no change. | `src/compat/envelope.rs` (path computed from `--compat-report`) |
| Diagnostics (REJECT messages, no-match errors) | Both `workspace_root_manifest` AND `member_manifest` (when applicable) | Error messages must cite the resolved manifest path AND the workspace-root path so the adopter can audit both. | `src/compat/overlay.rs` (resolver + override_workspace_inheritance diagnostics; §4.4 R2 cases 1–6 cite both paths). |
| Active-toolchain capture (envelope §3.4) | `workspace_root` | The `rust-toolchain.toml` discovery walks up from the cargo invocation cwd, which is the workspace root for baseline cargo. Both baseline and overlay must capture the same toolchain. | `src/compat/envelope.rs` (existing surface; takes a cwd) |

**Non-`--package` single-crate path (PUNCH-4).** For invocations WITHOUT `--package`:

- `workspace_root` = the dir passed via `--compat-root`.
- `member_root = workspace_root` (collapse).
- `workspace_root_manifest = member_manifest = <workspace_root>/Cargo.toml`.
- All consumers read the single value; behavior is byte-identical to pre-#53 main.

The exact cases this collapse applies to:

1. Single-crate repos (e.g. cxx, serde_json, anyhow, thiserror — all Round-1 pilots). `Cargo.toml` is a `[package]`-only manifest; no `[workspace]`. The collapse is trivial.
2. Workspace-member subdirectory entry WITHOUT `--package` (the buggy shape #53 fixes). Today this REJECTs at Branch 2; the collapse still applies during the REJECT path because the resolver is not consulted. (R1's Branch 2 diagnostic is augmented in §6.9 to point the adopter at the workspace-root + `--package` shape.)
3. Adopter passes `--compat-manifest` (explicit manifest path, bypassing both `--compat-root`/`Cargo.toml` discovery and the resolver). `workspace_root = --compat-manifest.parent()`, `member_root = workspace_root`. The resolver is not consulted.

**Dual-root path (PUNCH-3).** For invocations WITH `--package`:

- `workspace_root` = the dir passed via `--compat-root` (MUST be a virtual workspace root per §1 scope).
- `workspace_root_manifest = <workspace_root>/Cargo.toml`.
- The resolver reads `workspace_root_manifest`, expands `[workspace.members]` against `workspace_root`, matches by `[package].name`, and returns `(member_manifest, workspace_root_manifest, workspace_root_value)` — the parsed TOML value is carried through (per §5.4 `WorkspaceMemberContext`) so the materializer doesn't re-parse.
- `member_root = member_manifest.parent().unwrap()`.
- All consumers route per the table above.

**Data-flow checkpoint.** The implementer's primary surface change is in `src/compat/mod.rs::run` and `src/compat/cli.rs::CompatArgs::from_cli`:

1. CLI parse → `Cli` carries `compat_package: Option<String>` (§3.1).
2. `CompatArgs::from_cli` projects → `CompatArgs` carries `compat_package: Option<String>` (§3.2).
3. `compat::run` calls a NEW helper `resolve_dual_root(&args)` that returns a `DualRoot` struct (the resolver's structured output; replaces R1's `UpstreamManifest` enum — R2 simplification, single shape covers both single-crate and dual-root via the collapse invariant):

   ```rust
   pub(crate) struct DualRoot {
       pub(crate) workspace_root: PathBuf,
       pub(crate) workspace_root_manifest: PathBuf,
       pub(crate) member_root: PathBuf,
       pub(crate) member_manifest: PathBuf,
       pub(crate) workspace_member_context: Option<WorkspaceMemberContext>,
   }
   ```

   When `args.compat_package.is_none()`, `workspace_member_context = None` and the four paths collapse (per the invariant). When `args.compat_package.is_some()`, the resolver populates `workspace_member_context = Some(WorkspaceMemberContext { workspace_root_manifest, workspace_root_value })`.

4. Every downstream consumer takes the explicit role from `DualRoot` (not the legacy `compat_root`).

This is the single most important structural change R2 introduces. R1 carried the dual-root semantics implicitly across §4 / §5 / §6 / §7 without a unified vocabulary; R2 anchors them all in this section.

---

### 3.2 New field on `CompatArgs`

`crate::compat::cli::CompatArgs` in `src/compat/cli.rs:87-114` projects validated `Cli` into a typed bundle. New field:

```rust
/// Workspace-member package selector forwarded from `--package`.
/// Resolved to a member-manifest path inside the compat driver.
pub(crate) compat_package: Option<String>,
```

Field placement: between `compat_filter` and `compat_trybuild_macro` (alphabetical within the `compat_*` block). `from_cli` (`src/compat/cli.rs:133-179`) clones `cli.compat_package` into the projection (one new line, mirrors the existing `compat_filter = cli.compat_filter.clone();` pattern). No new validation logic in `from_cli` — the mode-error matrix in `Cli::validate_mode_consistency` (`src/cli.rs:262-313`) handles validation.

### 3.2.bis Workspace-table path policy (R2 — BLOCK-2 fix, PUNCH-5 through PUNCH-11)

R1 used the phrase "carried verbatim" for all workspace-root-level tables flowing into the overlay. This is wrong for any TOML key holding a filesystem path: `path = "crates/foo"` carried verbatim to an overlay rooted at `<member>/target/lihaaf-overlay/Cargo.toml` would resolve against the overlay dir (giving `<member>/target/lihaaf-overlay/crates/foo` — broken), not against the workspace root.

R2 defines a 4-column classification policy for every workspace-root-level key the plan carries down. The implementer MUST apply the classification before writing the overlay.

**Classification semantics.**

- **Workspace-root path** — a relative path that cargo would resolve against the workspace-root directory. MUST be absolutized against `workspace_root` before carry-down.
- **Member-root path** — a relative path that cargo would resolve against the member-root directory (a `[lib].path`, `[[bin]].path`, etc., living inside the member's own `[package]`). MUST be absolutized against `member_root` before carry-down. NOTE: these keys live in the MEMBER's manifest, not the workspace root's; this category appears in §3.2.bis for completeness (the implementer also handles member-level path resolution; details below).
- **URL or non-path string** — a git URL, registry URL, version string, identifier, etc. Carried verbatim. No path semantics; no absolutization.
- **Non-copied** — a key the overlay does NOT carry down (membership keys like `members`, deprecated keys like `[workspace.metadata.cargo.rerun-if-changed]` adopters may set on the workspace but not used by lihaaf, etc.).

**Policy table for workspace-root-level keys (PUNCH-5).**

| Table / key path | Category | Absolutization rule | Notes |
|---|---|---|---|
| `[workspace.dependencies.<name>].path` | Workspace-root path | Absolutize against `workspace_root` | PUNCH-6. Common shape: `serde = { path = "crates/serde-internal" }` for workspace-internal deps. R1's "carried verbatim" wording is WRONG here. |
| `[workspace.dependencies.<name>].git` | URL | Verbatim | PUNCH-7. URL string, no path. |
| `[workspace.dependencies.<name>].branch` | Identifier | Verbatim | PUNCH-7. Git ref, no path. |
| `[workspace.dependencies.<name>].tag` | Identifier | Verbatim | PUNCH-7. Git ref, no path. |
| `[workspace.dependencies.<name>].rev` | Identifier | Verbatim | PUNCH-7. Git ref, no path. |
| `[workspace.dependencies.<name>].version` | Version string | Verbatim | Semver spec, no path. |
| `[workspace.dependencies.<name>].features` | Array of identifiers | Verbatim | Feature names. |
| `[workspace.dependencies.<name>].default-features` | Bool | Verbatim | — |
| `[workspace.dependencies.<name>].optional` | Bool | Verbatim | — |
| `[workspace.dependencies.<name>].package` | Identifier | Verbatim | Package-rename target name. |
| `[workspace.package.readme]` | Workspace-root path | Absolutize against `workspace_root` | PUNCH-8. Members inheriting `readme = { workspace = true }` resolve to a path; cargo would resolve relative to the WORKSPACE root, not the inheriting member. |
| `[workspace.package.license-file]` | Workspace-root path | Absolutize against `workspace_root` | PUNCH-8. Same shape as `readme`. |
| `[workspace.package.repository]` | URL | Verbatim | PUNCH-8. URL string. |
| `[workspace.package.homepage]` | URL | Verbatim | URL string. |
| `[workspace.package.documentation]` | URL | Verbatim | URL string. |
| `[workspace.package.description]` | String | Verbatim | Free-form text. |
| `[workspace.package.keywords]` | Array | Verbatim | Identifiers. |
| `[workspace.package.categories]` | Array | Verbatim | Identifiers. |
| `[workspace.package.license]` | String | Verbatim | SPDX identifier. |
| `[workspace.package.edition]` | String | Verbatim | Year-string. |
| `[workspace.package.rust-version]` | String | Verbatim | MSRV version. |
| `[workspace.package.version]` | String | Verbatim | Semver. |
| `[workspace.package.authors]` | Array | Verbatim | Free-form. |
| `[workspace.package.publish]` | Bool / array | Verbatim | Registry name list. |
| `[workspace.lints.*]` (entire table) | Identifier tree | Verbatim | Lint names; no paths. |
| `[workspace.metadata.*]` (entire table) | Opaque | Verbatim — PUNCH-9 caveat | PUNCH-9: `[workspace.metadata]` is adopter-defined; lihaaf does NOT perform deep metadata rewriting. If an adopter encodes a path inside `[workspace.metadata.X]` (e.g. `[workspace.metadata.docs.rs] features = ["full"]` or a custom `[workspace.metadata.deploy.target-dir] = "deploy/"`), lihaaf carries the byte sequence verbatim — adopters with path-bearing metadata MUST use absolute paths or accept the overlay's anchoring. This is a documented limitation. |
| `[workspace.resolver]` | Integer / string | Verbatim | "1", "2", "3". |
| `[workspace.members]` | Array of paths/globs | NON-COPIED (stripped) | PUNCH-10. Membership keys are stripped from the overlay's `[workspace]` table (existing Branch 4 behavior; preserved in R2). |
| `[workspace.exclude]` | Array of paths/globs | NON-COPIED (stripped) | PUNCH-10. Membership exclusion key; stripped. The resolver USES this table (per §4.3 step 3.5) but the overlay does not carry it. |
| `[workspace.default-members]` | Array of paths | NON-COPIED (stripped) | PUNCH-10. Used by cargo CLI default selection; not consulted by `--package`-resolved overlay. Stripped. |
| `[patch.crates-io.<name>].path` | Workspace-root path | Absolutize against `workspace_root` BEFORE Option H | PUNCH-6 + cargo behavior — cargo's own `[patch]` path resolution is relative to the DECLARING manifest, which is the workspace root. Must absolutize before Option H Rules 1-4 run on the merged table. See §5.3.bis. |
| `[patch.crates-io.<name>].git`, `branch`, `tag`, `rev`, `version` | URL / identifier / version | Verbatim | Same shape as `[workspace.dependencies]`. |
| `[replace.<spec>]` | Workspace-root path (when `path = "..."` set) | Absolutize against `workspace_root` | PUNCH-15. Deprecated; rare; modern crates use `[patch]`. axum does not use it. |
| `[profile.*]` (entire table) | Profile config (no paths) | Verbatim | PUNCH-16. Profile keys are LTO settings, opt-level, codegen-units, etc.; no paths. |

**Policy table for member-root-level keys (PUNCH-11).** These keys live in the MEMBER's manifest (not the workspace root's), but the overlay materialization touches them and must apply the right resolution:

| Table / key path | Category | Absolutization rule | Notes |
|---|---|---|---|
| `[package.build]` | Member-root path | Absolutize against `member_root` | Build-script path; relative to the member's own dir. The existing `materialize_overlay_inner` already handles this for the single-root case; R2 confirms it stays member-root-anchored when `member_root != workspace_root`. |
| `[lib].path` | Member-root path | Absolutize against `member_root` | Existing `materialize_overlay_inner` behavior (it currently does `member_root.join(lib_path)`); unchanged. |
| `[[bin]].path` | Member-root path | Absolutize against `member_root` | Same as `[lib].path`. |
| `[[test]].path` | Member-root path | Absolutize against `member_root` | Same shape. |
| `[[bench]].path` | Member-root path | Absolutize against `member_root` | Same shape. |
| `[[example]].path` | Member-root path | Absolutize against `member_root` | Same shape. |
| `[package.readme]`, `[package.license-file]` | Member-root path (when not inheriting) | Absolutize against `member_root`; when inheriting from `[workspace.package.readme]`, resolve against `workspace_root` per the inheritance policy. | The inherit-vs-local distinction is handled by Option A1 carry-down (§4.6): the workspace-root values flow down absolutized; if the member also declares a local `[package.readme]`, the local form takes precedence and resolves member-root. |
| `[dependencies.<name>].path` | Member-root path | Absolutize against `member_root` | Member-local path-deps. Existing behavior. |
| `[dev-dependencies.<name>].path` | Member-root path | Absolutize against `member_root` | Existing. |
| `[build-dependencies.<name>].path` | Member-root path | Absolutize against `member_root` | Existing. |

**Implementation contract for the carry-down function.**

The `apply_workspace_member_inheritance` function (§5.3 R1 surface; R2 finalizes the contract) takes `&WorkspaceMemberContext` (carrying `workspace_root_manifest` + parsed `workspace_root_value`) and the overlay's `top` TOML map, and:

1. **For workspace-root-path keys** in the policy table above, reads the relative path from `workspace_root_value`, computes `absolutized = workspace_root.join(relative_path)` (using `member_manifest.parent().unwrap()`-equivalent for `workspace_root`, derivable as `workspace_root_manifest.parent().unwrap()`), normalizes to forward-slash form (per existing `to_forward_slash` convention used elsewhere in `overlay.rs`), and writes the absolutized form into the overlay's matching key under `[workspace.*]` or top-level `[patch.crates-io]`.

2. **For URL / verbatim keys**, copies the TOML value byte-for-byte (using `toml::Value::clone()`).

3. **For non-copied keys**, drops them (does not write into the overlay).

4. **For member-root-path keys**, NO action by `apply_workspace_member_inheritance` — those keys live in the MEMBER's manifest and are handled by the existing `materialize_overlay_inner` path-rewriting machinery, which already takes `member_root` as its base.

**Why this matters in practice.** axum's `Cargo.toml` declares `[workspace.dependencies.serde] = { version = "1.0", features = [...] }` (verbatim — URL/version/identifier only, no path; this carry-down is straightforward). But adopter forks of cargo workspaces sometimes use workspace-relative `path = "..."` for sibling-crate development; if R2 did not catch BLOCK-2, those forks would produce an overlay with broken path-resolution and the compat verdict would be a false-negative.

**Test surface (cross-reference §7.2 R2).** T-30 (`workspace_root_path_absolutization_for_dependencies_path`) covers the dependencies.path case; T-31 (`workspace_root_path_absolutization_for_package_readme_license_file`) covers the inherited package path-fields case. Both new in R2.

---

### 3.3 Validator extensions in `Cli::validate_mode_consistency`

Two new rules added to the existing matrix (`src/cli.rs:262-313`):

**Rule A (non-compat-mode rejection — symmetric with existing rules):** Outside compat mode, `--package` (`compat_package.is_some()`) is a mode error. Diagnostic uses the existing `non_compat_mode_error` helper (`src/cli.rs:338-346`). Placement: in the existing else-branch alphabetical block (between `compat_manifest` at `:299-301` and `compat_report` at `:302-304`).

**Rule B (compat-mode interaction with `--compat-manifest`):** When `--compat` is set, `compat_package.is_some() && compat_manifest.is_some()` is a mode error. The two flags are mutually exclusive: `--compat-manifest` supplies an explicit manifest path, bypassing the member-resolver; `--package` invokes the resolver. Combining them is incoherent. Diagnostic (new helper `cli_mutual_exclusion_error` or inline `Error::Cli`):

> "error: `--package` and `--compat-manifest` cannot be combined: `--compat-manifest` supplies an explicit manifest path directly to compat mode, while `--package` invokes the workspace-member resolver. Use one or the other."

Placement: in the if-compat branch, after the existing required-flag checks (`:280-285`), before the new section ends.

**No additional validator coupling.** The "member-resolver is required iff compat_root points at a workspace root" check is NOT done in the validator — it requires filesystem I/O (parsing `<compat_root>/Cargo.toml` to detect the workspace-root shape). That belongs in `compat::run` after the manifest is parsed (§4.4 — the directed diagnostic when `-p` is required but absent).

### 3.4 Interaction with other flags

- `--compat-root` is still required when `--compat` is set. With `--package`, `--compat-root` MUST point at the workspace ROOT, not at a member subdirectory. The resolver enforces this (§4).
- `--compat-report` is still required.
- `--compat-cargo-test-argv`, `--compat-commit`, `--compat-filter`, `--compat-trybuild-macro`: unchanged interactions. The baseline cargo test runs at the workspace root (workspace cargo treats `cargo test -p axum-macros` as the right invocation; the baseline argv must include the equivalent — see §6.X edge case).
- Pass-through v0.1 flags (`--bless`, `--no-cache`, `--list`, `--quiet`, `--verbose`, `--use-symlink`, `--keep-output`, `-j`): unchanged.
- `--suite`: unchanged.

### 3.5 Help-text update

`src/cli.rs:32-39` (the `long_about` attribute on the `Cli` struct) does not enumerate flags; no change needed there. Per-flag help is the doc-comment on the `compat_package` field (§3.1). clap auto-includes the new flag in `--help` output.

---

## 4. Resolver

### 4.1 Where the resolver runs

A new function `resolve_workspace_member_manifest` lives in `src/compat/overlay.rs` (collocated with the existing `detect_implicit_ancestor_workspace` and the workspace-root-rejection logic; the function deals with workspace-shape navigation). R2 signature returns the parsed workspace-root TOML value alongside the manifest path so the materializer doesn't re-parse:

```rust
/// Resolve `<workspace_root>/Cargo.toml` + `<package_name>` to the
/// member's manifest path. Reads the workspace root's `[workspace.members]`
/// array, expands globs against the workspace-root directory, reads each
/// candidate member's `Cargo.toml`, and returns the path of the manifest
/// whose `[package].name == package_name` together with the parsed
/// workspace-root TOML value (consumed by `apply_workspace_member_inheritance`
/// for the carry-down per §5.3 + §5.3.bis).
///
/// **Returns** `Ok((member_manifest_path, workspace_root_value))` on a single
/// unambiguous match, `Err(Error::Cli)` on no-match / multiple-match /
/// unparseable-workspace-root / unparseable-member-manifest /
/// workspace-root-not-a-workspace-root (per §4.3 step 2 and step 2.5 for the
/// v0.1.0 virtual-workspace-only scope).
pub(crate) fn resolve_workspace_member_manifest(
    workspace_root_manifest: &Path,
    package_name: &str,
) -> Result<(PathBuf, toml::Value), Error>
```

### 4.2 Driver wire-up (R2 — `DualRoot` shape)

The driver (`src/compat/mod.rs`) calls a new helper `resolve_dual_root` that returns a `DualRoot` struct (per §3.1.bis) covering both the dual-root (`--package`-supplied) and collapsed single-root (non-`--package`) cases. This REPLACES R1's `resolve_upstream_manifest` returning `Result<PathBuf, Error>`.

Pseudocode (final form belongs to the implementer, but the decision tree is pre-committed):

```rust
fn resolve_dual_root(args: &cli::CompatArgs) -> Result<DualRoot, Error> {
    let workspace_root = args.compat_root.clone();

    // 1. Explicit `--compat-manifest` always wins (mutual-exclusion with
    //    `--package` is enforced by validate_mode_consistency; if we
    //    reach here with both set, that's a validator bug).
    if let Some(m) = &args.compat_manifest {
        // Single-root collapse — `--compat-manifest` overrides the default
        // `<compat_root>/Cargo.toml`. Both roots collapse to the manifest's
        // parent dir.
        let member_root = m.parent().expect("manifest has parent").to_owned();
        return Ok(DualRoot {
            workspace_root: member_root.clone(),
            workspace_root_manifest: m.clone(),
            member_root,
            member_manifest: m.clone(),
            workspace_member_context: None,
        });
    }

    // 2. Conventional default: <compat_root>/Cargo.toml.
    let default_manifest = workspace_root.join("Cargo.toml");

    // 3. If `--package` was not supplied, return the default in collapsed
    //    single-root form. The overlay materializer will REJECT a workspace-
    //    root manifest here with a directed diagnostic (existing
    //    `is_workspace_root_manifest` branch at overlay.rs:488-498),
    //    augmented in §4.4 below to suggest `--package`.
    let Some(pkg) = &args.compat_package else {
        return Ok(DualRoot {
            workspace_root: workspace_root.clone(),
            workspace_root_manifest: default_manifest.clone(),
            member_root: workspace_root,
            member_manifest: default_manifest,
            workspace_member_context: None,
        });
    };

    // 4. `--package` is supplied. The default manifest MUST be a virtual
    //    workspace-root manifest (declares `[workspace]` without `[package]`);
    //    the resolver verifies and rejects otherwise (§4.3 step 2 + step 2.5
    //    for the v0.1.0 virtual-workspace-only scope).
    let (member_manifest, workspace_root_value) =
        overlay::resolve_workspace_member_manifest(&default_manifest, pkg)?;
    let member_root = member_manifest
        .parent()
        .expect("member manifest has parent")
        .to_owned();
    Ok(DualRoot {
        workspace_root,
        workspace_root_manifest: default_manifest.clone(),
        member_root,
        member_manifest,
        workspace_member_context: Some(WorkspaceMemberContext {
            workspace_root_manifest: default_manifest,
            workspace_root_value,
        }),
    })
}
```

The driver call site at `src/compat/mod.rs:86` (`let upstream_manifest = resolve_upstream_manifest(&args)?;`) is REPLACED with `let dual_root = resolve_dual_root(&args)?;`. Every downstream consumer of `compat_root` or `upstream_manifest` is updated to take the explicit `DualRoot` field per the §3.1.bis routing table.

### 4.3 Resolver algorithm

Step-by-step (this is the contract the implementer follows):

1. **Read + parse workspace-root manifest.** `std::fs::read_to_string(workspace_root_manifest)` → `toml::from_str::<toml::Value>(...)`. On I/O failure or TOML parse failure, return `Error::Io` or `Error::TomlParse` (mirror the existing `materialize_overlay_inner` error shapes at `overlay.rs:455-476`).

2. **Verify workspace-root shape.** Use the existing `is_workspace_root_manifest` predicate (`overlay.rs:1528-1535`). If the manifest is NOT a workspace-root manifest, return `Error::Cli` with a directed diagnostic:

   > "error: `--package <pkg>` requires `--compat-root` to point at a workspace root (a `Cargo.toml` declaring `[workspace]` without `[package]`); `<compat_root>/Cargo.toml` does not match this shape. Either drop `--package` and point `--compat-root` directly at the member's `Cargo.toml`, or fix `--compat-root` to the workspace-root directory."

   This is the §6 edge case "`-p` supplied AND invocation is NOT from a workspace" — surfaced inside the resolver, not the validator (filesystem read required).

   **2.5 (R2 — package+workspace root rejection).** `is_workspace_root_manifest` returns `true` ONLY for virtual workspaces (declares `[workspace]` WITHOUT `[package]`); see `overlay.rs:1528-1535` predicate body. If a package+workspace shape slipped in (root declares both `[package]` AND `[workspace]`), the predicate returns `false` and step 2's diagnostic fires. R2 §1 scopes v0.1.0 to virtual-workspace-only, so this REJECT is correct. The diagnostic text in step 2 specifically names "without `[package]`" so adopters of package+workspace roots get a precise reason. v0.2 / v1.0 may relax this; v0.1.0 does not. See §11.11 for the follow-up issue requirement.

3. **Read `[workspace.members]` array.** `value.get("workspace").and_then(|w| w.get("members")).and_then(|m| m.as_array())`. If absent or non-array, return `Error::Cli`:

   > "error: `--package <pkg>` resolver: `<workspace_root_manifest>` has `[workspace]` but no `[workspace.members]` array; cannot resolve `<pkg>`. Add the package to `[workspace.members]` or pass the member's manifest path directly via `--compat-manifest`."

   **3.5 (R2 — `[workspace.exclude]` subtraction, BLOCK-5 PUNCH-21).** After reading `members`, ALSO read `[workspace.exclude]`: `value.get("workspace").and_then(|w| w.get("exclude")).and_then(|e| e.as_array())`. If present, parse each entry by the same string-or-glob rules as step 5 below, and build a `HashSet<PathBuf>` of excluded directories (each resolved against `workspace_root` per step 5). The resolver then SKIPS any candidate directory whose canonicalized path is in the exclude set BEFORE applying any package-name match. If `workspace.exclude` is absent, the exclude set is empty. Rationale: cargo's own semantics — `members - exclude` is the effective workspace member set. Without this subtraction, the resolver could pick a directory the adopter intended to exclude.

4. **Iterate workspace-root entries.** For each entry in the `members` array (after exclude subtraction per step 3.5):
   - The entry must be a string (TOML schema; non-string entries return `Error::TomlParse` with a directed diagnostic).
   - Determine glob-or-literal-or-nested-literal:
     - Contains `*`, `?`, or `[` → glob (handled in step 5).
     - Otherwise → literal entry (no glob metachars). The literal can be a simple directory name (`"axum"`) OR an explicit nested path (`"crates/foo"`, `"tools/bar"` — slash separators, PUNCH-18).
   - Resolve against the workspace-root directory:
     - Workspace root dir = `workspace_root_manifest.parent()` (panics-not-possible: `workspace_root_manifest` always has a parent since it's a file path with a parent dir).
     - Literal entry: `<workspace_root>/<entry>` is the candidate directory; `<workspace_root>/<entry>/Cargo.toml` is the candidate manifest. Slash separators within `<entry>` are honored as path traversal (so `crates/foo` resolves to `<workspace_root>/crates/foo/Cargo.toml`). If the candidate manifest does NOT exist, skip it (cargo behavior: missing members are noted but not enumerated unless explicitly requested). The skip is silent in the resolver — the no-match diagnostic below will surface if no candidates match.
     - Glob entry: see step 5.

   **4.5 (R2 — nested-workspace traversal boundary, BLOCK-5 PUNCH-23).** When the resolver descends into a candidate directory, if that directory's own `Cargo.toml` declares `[workspace]`, the candidate is a nested-workspace root rather than a normal member. The resolver does NOT recurse into the nested workspace's `[workspace.members]`; the candidate is treated as a single match candidate by its own `[package].name` (if any). Rationale: cargo's own behavior — each `[workspace]` declaration is a workspace boundary; the outer workspace's `members` array points AT the nested root, not THROUGH it. If the adopter wants to target a member of the nested workspace, they pass the nested workspace's `Cargo.toml` as `--compat-root` and use `--package <nested-member>`. The outer-workspace resolver does NOT cross the boundary.

5. **Glob expansion details (R2 — PUNCH-17, PUNCH-18, PUNCH-19, PUNCH-20).** Per the v0.1 "no `glob` crate dependency" rule (consistent with `src/discovery.rs:117-131` for fixture glob expansion), the resolver uses stdlib `std::fs::read_dir` + pattern matching. The pattern matcher is a small helper inline in the resolver, NOT a re-export from discovery — the discovery glob matcher applies to file paths (literal/`*`/`?`/`[abc]`); the workspace-members pattern is more permissive (supports a single slash-separated parent segment).

   **Supported glob shapes (the resolver MUST accept):**

   | Shape | Example | Resolution | Notes |
   |---|---|---|---|
   | Bare wildcard | `axum-*` | Enumerates `<workspace_root>/*/Cargo.toml` matching pattern `axum-*` against the child dir name. | Most common shape (axum, tokio, tonic use this). |
   | Single-segment-with-slash | `crates/*` | Enumerates `<workspace_root>/crates/*/Cargo.toml` matching pattern `*` against grandchild dir names under `<workspace_root>/crates/`. | PUNCH-17. Common alternative shape (rust-lang/cargo, rust-lang/rust, many adopter forks). The parent segment (`crates/`) is a literal directory name; the glob applies only to the LAST segment. |
   | Explicit nested literal | `crates/foo` | Resolves to `<workspace_root>/crates/foo/Cargo.toml`. | PUNCH-18. No glob; literal nested path. Handled by step 4 (literal branch). |
   | Character class | `axum-[mc]ore` | Single-segment glob with character class; behaves as `axum-core` OR `axum-more`. | Rare. Supported per `src/discovery.rs:117-131` matcher. |
   | Single-char wildcard | `axum-?` | Single-segment glob; `?` matches exactly one char. | Rare. Supported. |

   **REJECTED glob shapes (the resolver MUST error with directed diagnostic):**

   | Shape | Example | Diagnostic |
   |---|---|---|
   | Deep glob (`**`) | `**/*`, `crates/**`, `**/sub/*` | PUNCH-19. Cargo's `[workspace.members]` does NOT support `**` (verified against cargo reference — `**` is a fileglob convention from gitignore, not a cargo workspace-member pattern). Return `Error::Cli` with: "error: `--package <pkg>` resolver: workspace member entry `<entry>` uses `**` (deep glob); cargo does not support `**` in `[workspace.members]`. Use `*` (single-segment glob) or an explicit literal path instead." |
   | Multiple slashes with glob in non-last segment | `crates/*/foo`, `*/foo/*` | PUNCH-19 corollary. Reject with: "error: `--package <pkg>` resolver: workspace member entry `<entry>` uses a glob in a non-final path segment; only the LAST segment may contain glob metachars (`*`, `?`, `[...]`). Use a literal parent path or split into multiple entries." |
   | Absolute path | `/usr/local/foo`, `C:\workspace\foo` (Windows) | PUNCH-20. Reject with: "error: `--package <pkg>` resolver: workspace member entry `<entry>` is absolute; `[workspace.members]` entries are workspace-relative paths only. Use a relative path." Cargo's own behavior matches: absolute paths in `[workspace.members]` are an error. |
   | Parent traversal | `../sibling`, `../../uncle` | PUNCH-20 corollary. Reject with: "error: `--package <pkg>` resolver: workspace member entry `<entry>` uses `..` (parent traversal); workspace members must be descendants of the workspace root. Use a relative path within the workspace." Cargo's own behavior: parent-traversing members are an error. |

   **Trailing-slash normalization (R2 — PUNCH-20).** Entries like `crates/foo/` or `axum-macros/` are normalized by trimming the trailing slash before resolution. This is a forgiveness rule; cargo accepts both forms. The resolver normalizes to no-trailing-slash internally so the exclude-set membership check (step 3.5) is consistent.

   **Glob matcher implementation.** Reference `src/discovery.rs:117-131` for the existing helper; if that helper supports the `axum-*` single-segment shape, REUSE it (with a thin wrapper for the `crates/*` two-segment case that pre-splits on `/`). Otherwise, inline a new helper. The implementer's choice; §10b row 4 / row 33 verify behavior correctness via test outputs.

6. **Read each candidate's `Cargo.toml`.** `std::fs::read_to_string(<candidate>/Cargo.toml)` → `toml::from_str::<toml::Value>(...)` → `value.get("package").and_then(|p| p.get("name")).and_then(|n| n.as_str())`. **Workspace-inheritance note:** the `[package].name` field is NOT inheritable in cargo (verify against cargo source — see §11 risks). A package may inherit `version`, `authors`, `description`, `edition`, `rust-version`, `repository`, `license`, etc. from `[workspace.package]`, but NOT `name`. So the resolver can trust the literal string at `package.name` without recursing into the workspace inheritance tables.

   Verification: cargo's reference (https://doc.rust-lang.org/cargo/reference/workspaces.html#the-package-table) lists inheritable keys; `name` is not among them. The package name is the member's own contract.

   If a candidate's manifest fails to parse, log a non-fatal warning (mirror the `detect_implicit_ancestor_workspace` skipping behavior at `overlay.rs:953-966`) and continue. The candidate is not a match.

   **6.5 (R2 — nested-workspace candidate skip).** If a candidate's manifest declares `[workspace]` (a nested-workspace root), AND it ALSO has `[package]` with a matching name, the candidate is a valid match (per step 4.5 — the outer resolver treats nested-workspace roots as match candidates by their own `[package].name`, without descending into the nested workspace). If the candidate has `[workspace]` but no `[package]`, it is a pure-virtual nested workspace — skipped as a non-match (the outer resolver does not descend; if the adopter wants to target a nested-workspace member, see §6.1).

7. **Match `<package_name>`.** Collect candidates whose `package.name == package_name`. Possible outcomes:
   - Zero matches → `Error::Cli` with no-match diagnostic (§4.4 case 1).
   - One match → return `Ok(candidate_manifest_path)`.
   - Multiple matches → `Error::Cli` with multiple-match diagnostic (§4.4 case 2). In normal cargo workspaces this shape is impossible (cargo enforces unique package names within a workspace), but we surface it as a directed error in case the workspace shape is corrupted.

   **7.5 (R2 — `workspace.default-members` non-interaction, BLOCK-5 PUNCH-22).** `[workspace.default-members]` declares which packages cargo's CLI selects by default for unqualified `cargo build` / `cargo test`. It is a SUBSET of `[workspace.members]` and does NOT change membership. The resolver does NOT consult `[workspace.default-members]`. The adopter's `--package <pkg>` selects ANY member by name regardless of whether it appears in `default-members`. (For the baseline cargo argv, the adopter chooses whether to add `-p <pkg>` themselves — per §6.13; the compat driver does not inject it. If the adopter wants the baseline to ALSO consult `default-members` semantics, they invoke baseline cargo without `-p`. This is documented in §8.3.)

8. **Duplicate-package handling after expansion (R2 — BLOCK-5 PUNCH-24).** Multiple member entries (literal + glob, or two globs with overlapping match sets) may resolve to the SAME directory. The resolver de-duplicates candidates by canonicalized directory path before applying the package-name match. Distinct directories that BOTH declare the same `[package].name` (which cargo itself would error on at load time) reach step 7's "multiple matches" branch and surface §4.4 case 2 ("multiple workspace members claim …") with both manifest paths in the error message. The de-duplication is important: without it, an adopter using both `members = ["axum-*"]` and `members = ["axum-macros"]` (overlapping) would receive a spurious multiple-match error for axum-macros.

### 4.4 Directed diagnostics

**Case 1 (no match):**

> "error: `--package <pkg>` resolver: no member of workspace `<workspace_root_manifest>` has `[package].name = \"<pkg>\"`. Members scanned: [<list>]. Confirm `<pkg>` exists in `[workspace.members]` and its `Cargo.toml` declares the expected package name."

**Case 2 (multiple matches — shouldn't happen but surface gracefully):**

> "error: `--package <pkg>` resolver: multiple workspace members claim `[package].name = \"<pkg>\"`: [<list of manifest paths>]. Workspace package names must be unique. Inspect each manifest and resolve the duplicate."

**Case 3 (workspace root with no `[workspace.members]`):** see §4.3 step 3 above.

**Case 4 (workspace-root not a workspace root):** see §4.3 step 2 above.

**Case 5 (cargo lihaaf without `-p` but compat_root IS a workspace root):** existing rejection at `overlay.rs:488-498` is AUGMENTED to suggest `--package`. New text:

> "error: `--compat-root` `<...>/Cargo.toml` is a workspace root (declares `[workspace]` without `[package]`); pass `--package <pkg>` to target a specific workspace member, or set `--compat-root` to a single-crate Cargo.toml."

### 4.5 Carries-through to subsequent compat-mode stages (R2 — dual-root routing)

Once `resolve_dual_root` returns a `DualRoot`, the rest of compat mode operates per the §3.1.bis routing table:

- `materialize_overlay_with_synthetic_metadata_builder` (`src/compat/overlay.rs:418-446`) reads `dual_root.member_manifest`. The overlay is staged at `<dual_root.member_root>/target/lihaaf-overlay/Cargo.toml` (using `dual_root.member_root` as the crate dir; the existing code already derives this via `member_manifest.parent()` at `overlay.rs:578-584` — unchanged).
- `[lib] crate-type` canonicalization runs on the member's `[lib]` table (mirroring the existing behavior). Path keys (`[lib].path`, `[[bin]].path`, etc.) are absolutized against `dual_root.member_root` per §3.2.bis policy table (member-root path category) — existing behavior, unchanged.
- `override_workspace_inheritance` runs on the MEMBER's TOML, passing `dual_root.workspace_member_context.as_ref()`. When `Some(ctx)`, Branches 2 + 3 are SUPPRESSED and `apply_workspace_member_inheritance(ctx)` runs to carry down workspace-root tables per §5 Option A1 + §5.3.bis composition order. Branch 1 (explicit `[package].workspace`) still fires. When `None` (the collapsed single-root case), all five branches behave as today — zero behavior change for existing pilots.
- Baseline cargo test runs at `dual_root.workspace_root` (NOT `dual_root.member_root`) and the argv must include `-p <pkg>` so cargo only runs the target package's tests. **THIS IS A BASELINE-RUNNER INTERACTION POINT** — see §6.13 edge case for the adopter-explicit argv-injection decision.
- Fixture discovery (`src/compat/discovery.rs`) reads the member's `tests/*.rs` from `dual_root.member_root`. The existing discovery takes a base dir as input; the driver passes `dual_root.member_root` (which equals `dual_root.workspace_root` in the collapsed case, so existing non-`--package` pilots see no change).
- Corpus / byte-determinism substitution (`tests/compat/overlay_determinism.rs:478-483` `__UPSTREAM_DIR__` placeholder substitution) uses `dual_root.member_root` per §3.1.bis routing table — the corpus test's tempdir IS the member dir for the new `workspace_member_with_package` fixture (§7.4 R2).
- Cargo.lock discovery (cargo internal) walks up from `dual_root.member_root` / `target/lihaaf-overlay/Cargo.toml` and finds `Cargo.lock` at `dual_root.workspace_root` — exactly the behavior we want; no code change.
- Active-toolchain capture (envelope §3.4) uses `dual_root.workspace_root` per §3.1.bis routing table.

### 4.6 Workspace-inheritance materialization (§5 Option A required)

The member manifest may carry `{ workspace = true }` inheritance refs in `[package]` (e.g. axum-macros' `rust-version = { workspace = true }`), `[dependencies]`, `[dev-dependencies]`, `[lints]` (axum-macros' `[lints] workspace = true`), or `[build-dependencies]`. Cargo resolves these by reading the workspace root's `[workspace.package]`, `[workspace.dependencies]`, `[workspace.lints]`, `[workspace.metadata]` tables.

The overlay's `override_workspace_inheritance` Branch 3 (`overlay.rs:844-865`) currently REJECTS this shape with the diagnostic "no local `[workspace]` table but uses workspace inheritance (one or more `{ workspace = true }` references…)". The §5 design must ensure Branch 3 does NOT fire on the `-p` case.

Two implementation options:

- **Option A1 (carry-down):** When `-p` is supplied, the resolver reads the workspace root's `[workspace.dependencies]`, `[workspace.package]`, `[workspace.lints]`, `[workspace.metadata]`, `[workspace.resolver]`, and any other `[workspace.X]` tables, and the overlay materializer copies them into the staged overlay's `[workspace]` table (suppressing only the membership keys per the existing pattern). Adopters' `{ workspace = true }` references resolve against the carried-down tables.
- **Option A2 (rewrite-references-down):** Read the workspace tables, then walk the member's TOML and REPLACE every `{ workspace = true }` reference with the resolved value. This produces a self-contained member manifest with no `{ workspace = true }` references.

**§5 picks Option A1 (carry-down).** Rationale: simpler, preserves byte-shape of the member's manifest (so the §3.2.3 byte-determinism rule applies cleanly), and the existing Branch 4 of `override_workspace_inheritance` already handles "clone upstream `[workspace]`, strip membership keys" — the only delta is that the upstream `[workspace]` came from the WORKSPACE ROOT manifest, not the member manifest.

Implementation: the resolver returns a `(member_manifest_path, workspace_root_manifest_path, workspace_root_value)` triple instead of a single path. The overlay materializer takes the workspace root context as a NEW parameter and consults it when building the overlay's `[workspace]` table. The driver wire-up passes both.

**R2 refinement.** The R1 plan introduced `ResolvedMember` and `UpstreamManifest` (enum) as separate types. R2 §3.1.bis consolidates this into a single `DualRoot` struct that covers BOTH the dual-root case (when `args.compat_package.is_some()`) and the collapsed single-root case (when `args.compat_package.is_none()`), using the explicit `workspace_member_context: Option<WorkspaceMemberContext>` field as the discriminator:

```rust
// R2 — unified shape (replaces R1's UpstreamManifest enum + ResolvedMember struct)
pub(crate) struct DualRoot {
    pub(crate) workspace_root: PathBuf,
    pub(crate) workspace_root_manifest: PathBuf,
    pub(crate) member_root: PathBuf,
    pub(crate) member_manifest: PathBuf,
    pub(crate) workspace_member_context: Option<WorkspaceMemberContext>,
}

pub(crate) struct WorkspaceMemberContext {
    pub(crate) workspace_root_manifest: PathBuf,
    pub(crate) workspace_root_value: toml::Value,
}

pub(crate) fn resolve_workspace_member_manifest(
    workspace_root_manifest: &Path,
    package_name: &str,
) -> Result<(PathBuf, toml::Value), Error>;
// Returns (member_manifest_path, workspace_root_value parsed once for downstream use).
// The driver wraps this into a DualRoot together with the workspace_root_manifest
// it already has from CLI parsing.
```

**Why a single struct, not an enum.** The five consumer routes (per §3.1.bis routing table) all consume the same set of fields; the enum form forced every consumer to match on `Direct` / `WorkspaceMember` and re-derive the four paths from the matched variant. The struct form unifies the API surface; the collapse invariant (`workspace_root == member_root` when `workspace_member_context.is_none()`) handles the non-`--package` case without a dispatch.

`compat::run` calls `resolve_dual_root(&args) -> Result<DualRoot, Error>` (the new helper replacing R1's `resolve_upstream_manifest`), then passes `&dual_root` through to every consumer that previously took `compat_root` or `upstream_manifest`. The materializer takes `dual_root.member_manifest` as input and `dual_root.workspace_member_context.as_ref()` as optional context; when `Some`, it runs `apply_workspace_member_inheritance` per §5.3 / §5.3.bis composition order.

`materialize_overlay_inner` (`src/compat/overlay.rs:448-611`) gains a new optional parameter `workspace_root_manifest: Option<&Path>`. When `Some`, after the `override_workspace_inheritance` call (which currently REJECTs Branches 2 + 3), a new pre-pass `apply_workspace_member_inheritance` runs FIRST, reads the workspace root's `[workspace.*]` tables, and merges them into the staged overlay's `[workspace]` table (Branch 4 of `override_workspace_inheritance` builds the overlay's `[workspace]` from the upstream's `[workspace]`; we want it built from the WORKSPACE ROOT's `[workspace]` instead when `-p` is set).

This is more invasive than the issue body's "fix space" sketch implied (which suggested just a `-p` flag + resolver). But it's necessary: without Option A1 carry-down, the resolved member manifest will hit Branch 3 REJECT on its `{ workspace = true }` references, and the user-facing fix does not work end-to-end.

### 4.7 Resolver tests (placement only — exact list in §7)

Unit tests in `src/compat/overlay.rs::tests` for:

- Workspace-root parse failure → `Error::TomlParse`.
- Workspace-root is NOT a workspace root (has `[package]`) → `Error::Cli` directed.
- Workspace-root has no `[workspace.members]` → `Error::Cli` directed.
- `members = ["axum", "axum-*"]` + `package_name = "axum-macros"` → resolves to `<root>/axum-macros/Cargo.toml`.
- `members = ["axum", "axum-*"]` + `package_name = "axum-core"` → resolves to `<root>/axum-core/Cargo.toml` (glob match).
- `members = ["axum"]` + `package_name = "axum-macros"` → no match → `Error::Cli` no-match.
- Workspace inheritance tables (`[workspace.dependencies]`, etc.) carried into `ResolvedMember`.
- `[workspace.members]` entry pointing at non-existent directory → silently skipped.
- `[workspace.members]` entry whose `Cargo.toml` fails to parse → warning, skipped.

Integration test (cargo-build-gated):

- `cargo_lihaaf_resolves_axum_macros_shape_workspace_member` — synthesizes the axum-macros workspace shape on disk, invokes `cargo lihaaf --compat -p axum-macros --compat-root <ws-root> --compat-report <r>`, asserts the overlay is staged at `<ws-root>/axum-macros/target/lihaaf-overlay/Cargo.toml`, the resolved upstream is `<ws-root>/axum-macros/Cargo.toml`, and the inner session reaches the dylib-build stage without REJECT. Gate behind `LIHAAF_RUN_CARGO_BUILD_TESTS=1` per [[lihaaf-no-local-binary-builds]].

---

## 5. Interaction with the implicit-ancestor REJECT (Branch 2)

### 5.1 Two options considered

**Option A (BYPASS-ON-EXPLICIT-TARGET):** When `-p` is supplied, the resolver hands the overlay materializer the WORKSPACE ROOT path AS WELL as the resolved member manifest path (per §4.6). The overlay materializer uses the workspace root for two purposes:

1. Carry-down of `[workspace.*]` tables into the staged overlay (Option A1 in §4.6).
2. Suppress Branch 2 REJECT by skipping `detect_implicit_ancestor_workspace` ENTIRELY when `workspace_root_manifest: Some(_)` is passed in. The semantics: when the adopter EXPLICITLY named the member (via `-p`), the implicit-ancestor REJECT is no longer the right guard — the adopter already opted in, and the workspace state is being merged into the overlay by Option A1's carry-down. The REJECT's "divergent dependency graph" hypothesis is closed by the carry-down: the overlay now carries the same `[workspace.dependencies]` / `[workspace.package]` / `[workspace.lints]` tables that baseline cargo would apply when resolving the member at the workspace root, so the dependency graph CONVERGES rather than diverges.

   Branches 1, 3, 4, 5 of `override_workspace_inheritance` still run. Branch 1 (explicit `[package].workspace = "<path>"`) is still a REJECT — an explicit-member declaration is incompatible with `-p` (the member declares membership in some path that isn't this workspace) and we surface that. Branch 3 (implicit member via inheritance refs) is SUPPRESSED when carry-down is active — the workspace tables are carried down, so the inheritance refs resolve cleanly.

**Option B (RELAX-WHEN-SELF-CONTAINED):** Detect when the member's dep graph is self-contained (no `{ workspace = true }` refs that reference ancestor `[workspace.dependencies]` entries; no ancestor `[patch.crates-io]` covering any of the member's transitive deps; no ancestor `[replace]`) and relax the REJECT only in that case. The escape hatch for non-self-contained workspaces is `--compat-manifest` (direct path).

### 5.2 Decision: Option A

**Pick Option A.** Rationale:

1. **Option B has a coverage cliff.** The "self-contained" detection is non-trivial: walking ancestor `[workspace.dependencies]` to check for `{ workspace = true }` indirection, walking ancestor `[patch.crates-io]` against the member's transitive dep graph (which requires either cargo-metadata or transitive parsing — both heavy for a guard). Any miss in the detection produces a false-clean overlay (silent divergence). Option A's carry-down is unconditional and doesn't have a detection-cliff failure mode.

2. **Option A is what cargo's own `cargo test -p <pkg>` already does.** When you invoke `cargo test -p axum-macros` from the workspace root, cargo applies the workspace's `[patch.crates-io]`, `[replace]`, `[workspace.dependencies]`, and `[workspace.package]` tables to the package being built. The overlay's correct semantic is to MATCH cargo's behavior, and that's exactly what Option A1 carry-down achieves. Option B's "relax for self-contained" diverges from cargo's behavior in the corner where it falsely classifies a graph as self-contained.

3. **Option A is more invasive but the scope is bounded.** The new function `apply_workspace_member_inheritance` (or equivalent) reads four to five tables from the workspace root and merges them into the overlay's `[workspace]`. The existing Branch 4 already handles "clone + strip membership keys" — Option A is "clone the workspace ROOT's `[workspace]` instead of the upstream's `[workspace]` (which is the member, which has none)". Plumbing change, not algorithmic change.

4. **The over-broad REJECT removal is precise:** Branch 2 fires today on EVERY workspace-member entry (because `detect_implicit_ancestor_workspace` finds the workspace root for every member). Option A keeps Branch 2 firing on workspace-member entries that DO NOT supply `-p` — those are still the "accidental member entry" case the REJECT was designed to catch. The relaxation is gated on the explicit opt-in signal (`-p`).

### 5.3 The REJECT's "divergent dependency graph" risk closure

The R4 review comment that introduced Branch 2 cited (overlay.rs:786-796):

> "The ancestor workspace may carry `[patch.crates-io]`, `[replace]`, `[profile]`, `resolver`, or `[workspace.dependencies]` tables that affect baseline cargo's dependency resolution; the lihaaf overlay terminates cargo's walk-up at the staged manifest and skips the ancestor's state entirely, producing a divergent dependency graph between baseline and overlay and therefore false compat verdicts."

Option A's carry-down closes this RISK for the following table classes:

- `[workspace.dependencies]` — carried down verbatim. Resolves `{ workspace = true }` references in `[dependencies]`, `[dev-dependencies]`, `[build-dependencies]`.
- `[workspace.package]` — carried down verbatim. Resolves `{ workspace = true }` references in `[package]` (e.g. `rust-version = { workspace = true }`, `edition = { workspace = true }`).
- `[workspace.lints]` — carried down verbatim. Resolves `[lints] workspace = true`.
- `[workspace.metadata]` — carried down verbatim (it's adopter-defined and may be consumed by build scripts).
- `[workspace.resolver]` — carried down verbatim (governs the dep resolver edition).

But the workspace ROOT also has potentially:

- `[patch.crates-io]` — this is the #40+#47 territory. The Option H 4-rule policy (in the #40+#47 plan) handles `[patch.crates-io]` resolution; the ROOT's `[patch.crates-io]` table should be carried down to the overlay following the same rules. **§5 PRE-COMMITTED CONSTRAINT:** the implementer must merge the workspace ROOT's `[patch.crates-io]` table into the overlay using the same Option H 4-rule policy that #40+#47 use. The merge happens AFTER the Option H rules run on the member manifest's own `[patch.crates-io]` (if any), with the workspace ROOT's entries layered on top for any key not present in the member's. (Cargo's own behavior: workspace `[patch.crates-io]` is the only `[patch.crates-io]` cargo reads — members do not contribute. Verify against cargo reference: https://doc.rust-lang.org/cargo/reference/overriding-dependencies.html#the-patch-section. Cargo source: `crates/cargo/src/cargo/util/toml/mod.rs` for the `[patch]` discovery — members' `[patch]` tables are an error in cargo.) So in fact the implementer's job is simpler: ONLY the workspace ROOT's `[patch.crates-io]` is read; the member's is invariably absent.

- `[replace]` — same treatment as `[patch]`; carried down. (Deprecated; rare in modern crates; axum doesn't use it.)

- `[profile.*]` — release / dev / test profiles. Carried down. Cargo's profile resolution reads ONLY the workspace root's profiles when building a workspace member, so this is mandatory for behavior parity.

- `resolver = "2"` — workspace edition selector. Carried down. The overlay's `[workspace]` table must declare the same resolver version as the workspace root, or cargo errors out.

**Concrete pre-commitment:** the implementer's `apply_workspace_member_inheritance` function (or whatever name) MUST carry the following keys from the workspace root's `[workspace.*]` and top-level into the overlay's `[workspace.*]`:

- `[workspace.dependencies]` (entire table)
- `[workspace.package]` (entire table)
- `[workspace.lints]` (entire table)
- `[workspace.metadata]` (entire table)
- `[workspace.resolver]` (entire table — actually a top-level key in `[workspace]`, not a sub-table)
- `[patch.crates-io]` (entire table; layered via #40+#47 Option H rules)
- `[replace]` (entire table; rare)
- `[profile.*]` (entire table)

**Concrete pre-commitment on STRIPPED keys:** the implementer's function MUST strip the following from the overlay's `[workspace]` (it's the overlay's own scope, not the workspace root's):

- `members`, `exclude`, `default-members` (membership keys; existing Branch 4 already handles).

### 5.3.bis Option H composition order — root-first, member-second (R2 — BLOCK-3 fix, PUNCH-12 through PUNCH-16)

R1 §6.2 stated that "the workspace root's `[patch.crates-io]` is layered into the overlay using the same Option H 4-rule policy that #40+#47 use", but left ambiguous what order things happen in. Specifically: if the WORKSPACE root carries `[patch.crates-io.<self>] = { path = "..." }` (an upstream self-patch on the member), and Option H Rule 1 INJECTs a synthetic self-patch into the MEMBER's slot first, the workspace-root self-patch could be hidden — the member would compare the merged table against the WRONG baseline.

R2 fixes this by specifying the composition order explicitly. The implementer follows this order verbatim.

**Composition order (PUNCH-12).** When `--package` is supplied AND the workspace root's `[patch.crates-io]` is non-empty:

1. **Step 1 — Compute the effective `[patch.crates-io]` table from the workspace root FIRST.**
   - Read `[patch.crates-io]` from `workspace_root_value`.
   - For each entry, classify the path-bearing keys per §3.2.bis policy table:
     - `[patch.crates-io.<name>].path` (workspace-root path) → absolutize against `workspace_root` (NOT against `member_root`, NOT against `overlay_root`). Per cargo's own behavior — `[patch]` paths are anchored to the declaring manifest, which is the workspace root.
     - `[patch.crates-io.<name>].git/branch/tag/rev/version` (URL/identifier) → verbatim.
   - The output is the "effective workspace-root patch table".

2. **Step 2 — Read the MEMBER's `[patch.crates-io]` table.** Per cargo's own behavior — `[patch.crates-io]` in a member manifest is a hard ERROR (cargo source: `crates/cargo/src/cargo/util/toml/mod.rs` rejects member-level `[patch]`). The resolver MUST match by rejecting member-level `[patch.crates-io]`:
   - If the member's `Cargo.toml` declares `[patch.crates-io]`, return `Error::Cli` with: "error: `--package <pkg>` resolver: workspace member `<member_manifest>` declares `[patch.crates-io]`; cargo does not permit `[patch]` in workspace members (only the workspace root). Move the patch entries to the workspace root's `[patch.crates-io]` or remove them."
   - In practice this is unreachable for valid workspaces (cargo would itself error on baseline `cargo test`). The check is defense-in-depth + clear diagnostic if an adopter is mid-conversion.

3. **Step 3 — Run Option H's 4 rules against the effective workspace-root patch table (with the member as the "self" target).**
   - Rule 1 (INJECT) — if no `[patch.crates-io.<self>]` exists in the effective table AND the member's `[package].name` does not appear, inject the synthetic self-patch `{ path = "<overlay_root>" }` per #40+#47 Option H Rule 1.
   - Rule 2 (REMAP) — if an existing `[patch.crates-io.<self>] = { path = "..." }` is present in the effective table, re-anchor its `path` to the overlay-root form per Rule 2.
   - Rules 3, 4 — apply per the existing #40+#47 R8 plan.
   - The 4-rule logic is OWNED by the existing `apply_self_patch_policy` function in `src/compat/overlay.rs` (per #40+#47 R8 plan §4.X — function exists on `main` at `cb5d5bf`). R2 EXTENDS this function with an optional `workspace_root_patch_table: Option<&toml::Value>` parameter; when `Some`, the function operates on the merged effective table rather than the member's own (invariably-empty) `[patch.crates-io]`.

4. **Step 4 — Write the resolved `[patch.crates-io]` into the overlay's top-level `[patch.crates-io]` table.**
   - Path entries are stored absolutized per step 1 (already absolutized from workspace_root) or per Rule 1/2's overlay-root injection.
   - The overlay's `[workspace]` table does NOT carry `[patch.crates-io]` (cargo reads `[patch]` from top-level only, not from `[workspace]`).

**Why root-first.** If the implementer ran Option H Rule 1 against the MEMBER's empty `[patch.crates-io]` FIRST (injecting a synthetic self-patch into an empty table), then merged the workspace-root entries SECOND, the synthetic self-patch would either be silently overwritten by a workspace-root entry (false-negative on the upstream's own self-patch) OR shadow the workspace-root entry (false-positive). The correct merge is: compute the effective workspace-root table first (carrying the upstream's authoritative `[patch.crates-io]` state), then let Option H's 4 rules adjust the SELF entry within that effective table.

**Cargo-behavior cross-check.** Cargo's runtime reads `[patch.crates-io]` ONLY from the workspace root (https://doc.rust-lang.org/cargo/reference/overriding-dependencies.html#the-patch-section). For workspace-member entry, the workspace root's `[patch.crates-io]` IS the upstream's `[patch.crates-io]`; treating it as the input to Option H is what makes the lihaaf overlay's `[patch]` resolution match cargo's behavior.

**Member-manifest `[patch.crates-io]` rejection (PUNCH-13).** Per step 2 above: if the resolved member manifest declares `[patch.crates-io]`, the resolver REJECTS with the cargo-compat diagnostic. Reasoning: matches cargo (which would itself error); avoids ambiguity about which table the implementer should respect.

**Non-`crates-io` patch tables (PUNCH-14).** R2 SCOPES `[patch.<registry>]` carry-down to `[patch.crates-io]` only. The Round-1 pilots and axum's workspace do not use alternate-registry patches. Adopter forks using `[patch.https-some-private-registry-url]` MUST currently rebase onto a workspace shape using `[patch.crates-io]` until v0.2 expands the scope. R2 explicitly DOES NOT carry `[patch.<other-registry>]` tables down to the overlay; if a workspace declares one, it is silently dropped (no diagnostic in this PR; v0.2 may surface a warning). The §5.3 bullet-list "(entire table)" wording is REFINED to be `[patch.crates-io]` only.

**`[replace]` handling (PUNCH-15).** Cargo's `[replace]` is deprecated in favor of `[patch]`. R2's carry-down policy: read `[replace]` from `workspace_root_value`, absolutize `path = "..."` entries per §3.2.bis, write into the overlay's top-level `[replace]` table verbatim otherwise. Path absolutization uses `workspace_root` (same rule as `[patch.crates-io]`). axum does not use `[replace]`; thiserror does not; serde_json does not. Most Round-2 pilots will not exercise this code path. Implementer adds the carry-down for completeness; tests T-15 (R1) cover it indirectly via the `apply_workspace_member_inheritance_carries_workspace_package_lints_metadata`-shaped test family.

**`[profile.*]` precedence (PUNCH-16).** Cargo's profile resolution reads `[profile.*]` ONLY from the workspace root when building a workspace member (https://doc.rust-lang.org/cargo/reference/profiles.html#profile-settings — "Profiles can be specified at the workspace level"). A member-local `[profile.*]` table is IGNORED by cargo. R2's policy: carry the workspace root's `[profile.*]` into the overlay's top-level `[profile.*]` table verbatim (no path absolutization — profiles have no path keys, only LTO settings / opt-levels / codegen-units / etc.). The member-local `[profile.*]` (if any) is also carried (it's part of the member manifest the overlay clones), but cargo will ignore it at build time. R1 §11 listed `[profile.*]` precedence as an open item; R2 closes it: workspace-root wins, per cargo. The implementer does NOT need to detect or warn about member-local `[profile.*]`.

**Test surface (cross-reference §7.2 R2).** T-32 (`apply_workspace_member_inheritance_carries_workspace_root_patch_crates_io_via_option_h`) covers the workspace-root `[patch.crates-io]` carry-down with Option H Rule 1 INJECT and Rule 2 REMAP cases. T-32 also covers the member-`[patch.crates-io]`-rejection diagnostic (the inverse).

---

### 5.4 The over-broad REJECT is RELAXED, not removed

**Branch 2 of `override_workspace_inheritance` remains in code.** It still fires when `dual_root.workspace_member_context.is_none()` AND the upstream manifest is a workspace member (no local `[workspace]` + ancestor `Cargo.toml` carries `[workspace]`).

The relaxation is: when `dual_root.workspace_member_context.is_some()` (i.e. the resolver succeeded for a `--package` invocation per §3.1.bis), the overlay materializer takes a new optional parameter `workspace_member_context: Option<&WorkspaceMemberContext>` (analogous to `synthetic_metadata` builder) and gets it from the `DualRoot` struct (per §3.1.bis routing table). When `Some(ctx)`, the materializer:

1. Skips Branch 2 of `override_workspace_inheritance` (passes a flag through, or `override_workspace_inheritance` consults `ctx.is_some()` directly).
2. Runs `apply_workspace_member_inheritance(ctx.workspace_root_manifest)` to carry down the workspace tables.
3. Branch 3 (inheritance-refs REJECT) is ALSO suppressed when `Some(ctx)` (the carry-down provides the tables the refs resolve against).
4. Branch 1 (explicit `[package].workspace = "<path>"`) STILL fires — even with `-p`, an explicit member declaration pointing OUTSIDE the workspace root is incoherent.

The function signature evolution is:

```rust
// Old (current main):
fn override_workspace_inheritance(
    top: &mut toml::map::Map<String, toml::Value>,
    upstream_manifest_path: &Path,
) -> Result<(), Error>

// New:
fn override_workspace_inheritance(
    top: &mut toml::map::Map<String, toml::Value>,
    upstream_manifest_path: &Path,
    workspace_member_context: Option<&WorkspaceMemberContext>,
) -> Result<(), Error>

struct WorkspaceMemberContext {
    workspace_root_manifest: PathBuf,
    workspace_root_value: toml::Value, // parsed once at the resolver, threaded through
}
```

When `workspace_member_context.is_some()`:

- Branch 2 (`detect_implicit_ancestor_workspace` path) is SKIPPED.
- Branch 3 (inheritance-refs detection) is SKIPPED.
- Branch 4 (build overlay workspace from upstream's `[workspace]`) is REPLACED by `apply_workspace_member_inheritance` which builds from the workspace root's `[workspace]`.

When `workspace_member_context.is_none()` (the existing non-`-p` path):

- All five branches run as today. Zero behavior change for existing pilots.

---

## 6. Edge cases

### 6.1 Workspace-in-workspace (nested workspaces)

Cargo permits — and the workspace nesting rules say each workspace stops at the next `[workspace]` declaration walking up. If `tokio-rs/axum` contained a sub-workspace at `axum/sub-workspace/Cargo.toml` declaring its own `[workspace]`, the cargo resolver would terminate at that sub-workspace when building any member underneath it.

**Behavior:** The resolver's `detect_implicit_ancestor_workspace` walk-up (used internally when computing the workspace root from a member manifest) terminates at the FIRST ancestor with `[workspace]`. If `-p` is supplied and `--compat-root` is the OUTER workspace root, but the named package belongs to the INNER (sub-)workspace, the resolver will not find it in the OUTER's `[workspace.members]` (because the OUTER's members do not include the sub-workspace's members) → no-match → diagnostic.

**Pre-committed behavior:** the resolver consults ONLY the workspace root that `compat_root` POINTS AT. It does NOT recursively walk into sub-workspaces. If the adopter wants to target a sub-workspace member, they set `--compat-root` to the sub-workspace root.

**Documented diagnostic:** the no-match diagnostic (§4.4 case 1) lists the scanned members from the OUTER workspace, which makes the omission visible. No special-case handling for nested workspaces in this PR.

### 6.2 `[patch.crates-io]` on the workspace root

This is the #40+#47 territory. Pre-committed by §5.3: the workspace root's `[patch.crates-io]` table is carried down to the overlay using the same Option H 4-rule policy that #40+#47 use. The implementer does NOT need to re-design the Option H policy — they extend the existing `apply_self_patch_policy` (the function the #40+#47 plan introduces; expected name) to take a workspace-root `[patch.crates-io]` table as an additional input. The merge is layered: member-local `[patch.crates-io]` first (invariably absent for non-root cargo workspaces — verify §4.6), then workspace-root entries.

The order matters: cargo treats workspace `[patch]` as the only `[patch]` it reads (members' `[patch]` is an error). So in practice the workspace root's table is the only source.

### 6.3 No-match: `-p <pkg>` doesn't match any member

Surfaced by §4.4 case 1. The error message includes the scanned member list so the adopter can diagnose typos / glob mismatches.

### 6.4 Multiple-match (shouldn't happen if cargo accepts; verify)

Cargo enforces unique package names within a workspace at cargo's load time (`cargo build` would itself error). The resolver surfaces this as §4.4 case 2 in case the workspace shape is corrupted (e.g. an adopter manually edited the `[package].name` field after a copy-paste). Surface it as an error rather than silently picking the first match.

### 6.5 Glob mismatch: `[workspace.members]` has `axum-*` but pkg is `axum-macros`

The resolver expands `axum-*` against the workspace-root directory. If `axum-macros/` is present as a directory AND `axum-macros/Cargo.toml` exists AND it declares `[package].name = "axum-macros"`, the resolver matches.

The glob match is on DIRECTORY NAME, not package name. So `members = ["axum-*"]` matches a directory named `axum-macros`, and then the resolver reads `axum-macros/Cargo.toml` to confirm `[package].name == "axum-macros"`. If the directory is named `axum-macros` but the manifest declares `[package].name = "something-else"`, the resolver does NOT match on `-p axum-macros` — package name is the truth, not directory name.

**Test:** add a unit test where the member directory name differs from the package name (cargo permits this; adopters occasionally use it). The resolver MUST match by `[package].name`, not by directory name.

### 6.6 `-p` supplied AND invocation is NOT from a workspace

Two sub-cases:

**6.6a — `--compat-root` points at a single-crate Cargo.toml (no `[workspace]` table).** `is_workspace_root_manifest` returns `false` → resolver §4.3 step 2 rejects with directed diagnostic:

> "error: `--package <pkg>` requires `--compat-root` to point at a workspace root (a `Cargo.toml` declaring `[workspace]` without `[package]`); `<compat_root>/Cargo.toml` does not match this shape. Either drop `--package` and point `--compat-root` directly at the member's `Cargo.toml`, or fix `--compat-root` to the workspace-root directory."

**6.6b — `--compat-root` points at a member subdirectory (no local `[workspace]`, ancestor has `[workspace]`).** `is_workspace_root_manifest` returns `false` (the member has `[package]` and no `[workspace]`) → same diagnostic as 6.6a.

The error message handles both sub-cases uniformly. The adopter's mental model is: "`-p` means: I'm starting from a workspace root." Anything else is an invocation-shape error.

### 6.7 Invocation IS from a workspace root and `-p` IS supplied (legit)

Resolver runs §4.3 → returns the member manifest path + workspace root context. Materializer runs Option A carry-down + suppresses Branches 2 + 3 of `override_workspace_inheritance`. End-to-end success.

### 6.8 Invocation IS from a workspace MEMBER subdir and `-p` IS supplied

Sub-case of 6.6b above. The resolver rejects: `-p` requires a workspace ROOT. The diagnostic suggests one of two fixes:

1. Drop `-p`, point `--compat-root` directly at the member (which Branch 2 will then still REJECT, because the member is implicitly in a workspace — round-tripping back to the bug). This is the path before #53; the diagnostic notes it's "not recommended for workspace members; use the workspace-root + `--package` shape."
2. Re-aim `--compat-root` at the workspace root and re-invoke.

**Pre-committed decision:** option 2 is the canonical fix. The diagnostic recommends it.

### 6.9 Invocation IS from a workspace MEMBER subdir and `-p` is NOT supplied

This is the current bug shape. Today: Branch 2 REJECTs with "implicit workspace member; pass workspace root as compat-root" — which then hits `is_workspace_root_manifest` REJECT. After #53: Branch 2 REJECTs with EXTENDED text (§4.4 case 5 + §6 extension):

> "error: `--compat-root` `<member-manifest>` is an implicit workspace member: it has no local `[workspace]` table but an ancestor manifest at `<ancestor>` carries `[workspace]`. Pass the workspace ROOT (`<ancestor>` or its containing directory) as `--compat-root` AND target this specific member with `--package <pkg-name>`, where `<pkg-name>` is the value of the member's `[package].name`."

The "either invoke from workspace ROOT, or restructure" wording in the current diagnostic (`overlay.rs:817-819`) is replaced with the directed `--compat-root <ws-root> --package <pkg>` suggestion. Adopters get an actionable fix instead of a dead-end.

### 6.10 Member has its own `[workspace]` table (declares itself a nested workspace)

Cargo permits this (members can be workspace roots in their own right). If `axum-macros/Cargo.toml` declared `[workspace] members = ["..."]` in addition to `[package]`, it would be a workspace root AT the member level.

**Pre-committed behavior:** the resolver matches the member by `[package].name`. The member's own `[workspace]` table (if any) is preserved by Option A's carry-down — Branch 4 of `override_workspace_inheritance` clones the existing `[workspace]` and strips membership keys. Crucially, the OUTER workspace's `[workspace.*]` tables are also carried down by `apply_workspace_member_inheritance`. The merge order: outer first, member's own next (overrides).

In practice this nested shape is rare. axum-macros does not have it. No special test case in §7; documented in §11 risks.

### 6.11 Pilots without `[workspace]` at all (single-crate repos)

If `--compat-root` points at a single-crate repo (no `[workspace]`) and `-p` is supplied, §4.3 step 2 rejects (6.6a). No additional work.

### 6.12 `Cargo.lock` interaction

Cargo writes `Cargo.lock` at the workspace root, not at the member level. The lockfile is consumed by both the baseline `cargo test -p axum-macros` invocation (which expects `Cargo.lock` at the workspace root) and the staged-overlay dylib build (`cargo rustc --manifest-path <member-dir>/target/lihaaf-overlay/Cargo.toml`).

**Pre-committed scope-out:** the lockfile is NOT touched by this PR. The dylib build uses the upstream's `Cargo.lock` (cargo discovers it by walking up from the overlay manifest), or generates a new one if absent. The compat-mode flow has always relied on cargo's lockfile discovery — no change.

A risk: if the overlay manifest's resolved dep graph differs from the workspace's resolved dep graph in a way that requires a different `Cargo.lock`, cargo will regenerate it (without `--frozen`). This is the existing behavior for the Round-1 pilots. axum-macros is not expected to surface a new failure mode here. **Risk: low.** Surfaced in §11.

### 6.13 Baseline runner interaction

The baseline `cargo test` invocation runs at the workspace root (where the lockfile lives) and must include `-p <pkg>` to only run the target member's tests. The default baseline argv is `["cargo", "test"]` which would run ALL workspace members' tests. The adopter must override with `--compat-cargo-test-argv '["cargo","test","-p","axum-macros"]'`.

**Pre-committed:** this is an adopter-facing knob, NOT an automatic override. The compat driver does NOT inject `-p <pkg>` into the baseline argv. Rationale: adopters may want a workspace-wide baseline (compare the OVERALL workspace pass/fail rate) or a member-specific one — the choice is theirs. Documented in §8 spec amendment.

**Audit:** the existing test corpus (`compat/baseline.toml`) assumes single-crate pilots. The new corpus entry for axum-macros must include the `-p`-bearing baseline argv shape so the test can pin the convention.

### 6.14 Glob expansion bug class (Codex check-target)

The glob matcher in §4.3 step 5 is a small inline helper (not the `glob` crate per v0.1 dependency policy). Possible bugs the implementer must avoid:

- `axum-*` matching `axum/` (the bare `axum` directory) — `*` requires at least ONE char after the prefix. The matcher must enforce this.
- `axum-*` matching `axum-macros/example/` recursively — directory-name glob only, no recursion.
- `[abc]` character class — confirm the matcher handles the single-bracket character class consistently with discovery's pattern matcher.

**Test:** unit test for `axum-*` matching `axum-macros` but NOT `axum` (codified in §7).

### 6.15 Tilde / shell-meta in `-p <pkg>`

`-p ~/bad-package` should be passed through as the literal string `~/bad-package` and fail to match in the workspace. No shell expansion. Cargo's `-p` has the same behavior.

### 6.16 Empty package name

`-p ""` is rejected at the validator layer? Or at the resolver? **Pre-committed:** clap's `value_parser` for `Option<String>` accepts empty strings; we add a parse-time validator (similar to `parse_jobs` at `src/cli.rs:167-178`) that rejects empty:

```rust
fn parse_compat_package(s: &str) -> Result<String, String> {
    if s.is_empty() {
        return Err("`--package` requires a non-empty package name".to_string());
    }
    // No further validation — cargo's package-name validation lives at
    // resolver time.
    Ok(s.to_string())
}
```

Field annotation:

```rust
#[arg(short = 'p', long = "package", value_name = "PACKAGE", value_parser = parse_compat_package)]
pub compat_package: Option<String>,
```

### 6.17 Cargo-package-name validation

Cargo enforces `[a-zA-Z0-9_-]+` for package names. The resolver does NOT re-validate this — the workspace root's `[workspace.members]` is the truth, and cargo would have already failed to load if the names were invalid. If an adopter passes `-p "ax%um"` (invalid char), the resolver will simply not find a match and surface §4.4 case 1.

### 6.18 Case sensitivity

Cargo package names are case-sensitive. `-p Axum-macros` does NOT match `[package].name = "axum-macros"`. Pre-committed: case-sensitive exact match.

---

## 7. Test corpus expansion

### 7.1 Spec §-id placement

The new entry-point shape is documented at:

- `docs/spec/lihaaf-v0.1.md` §8.2 — new flag entry `--package <package>` (alphabetical placement after `--no-cache`, before `--quiet`).
- `docs/compatibility-plan.md` §3.1 — new bullet under "Optional options when `--compat` is set" + amended invocation example.
- `docs/compatibility-plan.md` §3.2.3 — new sub-section "Workspace-member entry via `--package`" after the existing overlay paragraphs.

Section IDs the implementer must update — see §8 spec amendment.

### 7.2 Unit tests in `src/compat/overlay.rs::tests`

**Resolver tests** (the implementer writes all of these):

1. `resolve_workspace_member_manifest_succeeds_on_literal_member` — `members = ["axum"]` + pkg `axum` → resolves to `<root>/axum/Cargo.toml`.
2. `resolve_workspace_member_manifest_succeeds_on_glob_match` — `members = ["axum-*"]` + pkg `axum-macros` → resolves to `<root>/axum-macros/Cargo.toml`. (axum-macros shape pin.)
3. `resolve_workspace_member_manifest_matches_by_package_name_not_dir_name` — directory `foo/`, manifest `foo/Cargo.toml` with `[package].name = "bar"`, `members = ["foo"]` + pkg `bar` → resolves to `<root>/foo/Cargo.toml`.
4. `resolve_workspace_member_manifest_glob_does_not_match_bare_prefix` — `members = ["axum-*"]` + pkg `axum` (bare prefix, no suffix char) → no-match → `Error::Cli`.
5. `resolve_workspace_member_manifest_rejects_when_root_not_workspace_root` — workspace-root manifest is actually a single-crate `[package]` Cargo.toml → §6.6a diagnostic.
6. `resolve_workspace_member_manifest_rejects_when_no_members_array` — workspace root has `[workspace]` but no `[workspace.members]` → directed diagnostic.
7. `resolve_workspace_member_manifest_no_match_lists_scanned_members` — `members = ["a","b"]` + pkg `c` → error includes "scanned: [a, b]".
8. `resolve_workspace_member_manifest_skips_unparseable_member_manifest` — `members = ["a"]`, `a/Cargo.toml` is malformed TOML, pkg `a` → resolver skips with non-fatal warning, treats as no-match.
9. `resolve_workspace_member_manifest_skips_missing_member_directory` — `members = ["a"]`, no `a/` directory, pkg `a` → silently skipped, no-match.
10. `resolve_workspace_member_manifest_workspace_inheritance_captured` — verifies `ResolvedMember.workspace_root_manifest` is populated for downstream use.

**`override_workspace_inheritance` interaction tests:**

11. `override_workspace_skips_branch_2_with_workspace_member_context` — pre-conditions: manifest with no local `[workspace]`, ancestor with `[workspace]`, `workspace_member_context: Some(_)`. Expected: function succeeds (no REJECT). Inverse: with `None`, same input REJECTs.
12. `override_workspace_skips_branch_3_with_workspace_member_context` — pre-conditions: manifest with `{ workspace = true }` inheritance reference, `workspace_member_context: Some(_)`. Expected: function succeeds.
13. `override_workspace_still_rejects_branch_1_explicit_member_with_context` — pre-conditions: `[package].workspace = "<path>"`, `workspace_member_context: Some(_)`. Expected: REJECT still fires (the explicit declaration is incompatible with `-p`).
14. `apply_workspace_member_inheritance_carries_workspace_dependencies` — verifies `[workspace.dependencies]` from workspace root flows into overlay's `[workspace.dependencies]`.
15. `apply_workspace_member_inheritance_carries_workspace_package_lints_metadata` — same as 14 for the other tables.
16. `apply_workspace_member_inheritance_strips_membership_keys` — overlay's `[workspace]` MUST NOT contain `members`, `exclude`, or `default-members` after the carry-down.
17. `apply_workspace_member_inheritance_carries_workspace_root_patch_crates_io` — verifies workspace-root `[patch.crates-io]` is layered into the overlay via Option H (depends on #40+#47 being landed first).

**CLI / args projection tests:**

18. `cli_parses_short_p_flag` — `cargo lihaaf --compat -p axum-macros ...` parses; `cli.compat_package == Some("axum-macros")`.
19. `cli_parses_long_package_flag` — `cargo lihaaf --compat --package axum-macros ...` parses; same field.
20. `cli_rejects_empty_package_name` — `cargo lihaaf --compat -p ""` → clap-layer error.
21. `cli_rejects_package_outside_compat_mode` — `cargo lihaaf -p foo` (no `--compat`) → mode error.
22. `cli_rejects_package_with_compat_manifest` — `cargo lihaaf --compat --compat-root /x --compat-manifest /y -p foo --compat-report /z` → mutual-exclusion error.
23. `compat_args_from_cli_carries_compat_package` — `CompatArgs::from_cli(cli)` where `cli.compat_package == Some("axum-macros")` → `args.compat_package == Some("axum-macros")`.

**R2 NEW — Dual-root routing tests (BLOCK-1 / PUNCH-25):**

24. `dual_root_routing_baseline_cwd_is_workspace_root_member_consumers_use_member_root` — synthesizes a 2-member workspace, resolves `-p member-a`, and verifies that the resulting `DualRoot` struct routes: `workspace_root` to baseline cargo cwd, `member_root` to overlay materialization sibling-manifest computation, `member_root` to fixture discovery base, and that diagnostics emitted during the run (forced via an injected error in a fixture) cite BOTH `workspace_root_manifest` AND `member_manifest` in the error message. Inverse: in the non-`--package` collapse case, all routes resolve to the same path (asserted by reading `DualRoot.workspace_root == DualRoot.member_root`). Test class: unit (synthesized DualRoot struct + assertion on routing destination strings). Bites pre-fix: R1's single-`compat_root` plumbing would route `member_root` to baseline cargo cwd (incorrect — baseline must run at workspace root to find `Cargo.lock`).

**R2 NEW — Workspace-root path absolutization tests (BLOCK-2 / PUNCH-26):**

25. `workspace_root_path_absolutization_for_dependencies_path` — synthesizes a workspace with `[workspace.dependencies.foo] = { path = "crates/foo" }` (relative path), resolves `-p some-member`, and asserts the overlay's `[workspace.dependencies.foo].path` is absolutized to `<workspace_root>/crates/foo` (NOT to a path under `<overlay_root>` or `<member_root>`). Inverse: a `git = "..."` entry on the same workspace dep is preserved verbatim (no path absolutization on URL keys). Test class: unit. Bites pre-fix: R1's "carried verbatim" wording would leave the relative path as-is; cargo at overlay build time would resolve it against the overlay root, finding nothing and erroring.

**R2 NEW — Inherited workspace-package path-fields tests (BLOCK-2 / PUNCH-27):**

26. `workspace_root_path_absolutization_for_package_readme_license_file` — synthesizes a workspace with `[workspace.package] readme = "../../README.md" license-file = "LICENSE-MIT"` (both relative paths). The member uses `readme = { workspace = true }` and `license-file = { workspace = true }`. Resolves `-p the-member` and asserts the overlay's resolved `[package].readme` (after Option A1 carry-down through `[workspace.package]`) is absolutized to `<workspace_root>/../../README.md` (canonicalized form) and `[package].license-file` is absolutized to `<workspace_root>/LICENSE-MIT`. Test class: unit. Bites pre-fix: R1 §3.2.bis was missing; the implementer following R1 would carry these verbatim, breaking the overlay's readme/license-file resolution. (Notably, cargo's own behavior anchors inherited paths to the workspace root per https://doc.rust-lang.org/cargo/reference/workspaces.html#the-package-table — verified.)

**R2 NEW — Option H composition with workspace-root self-patch (BLOCK-3 / PUNCH-28):**

27. `option_h_root_first_member_second_with_workspace_root_self_patch_entry` — synthesizes a workspace where the WORKSPACE root declares `[patch.crates-io.pkg-name] = { path = "../local-fork" }` (an upstream self-patch on a member named `pkg-name`). Resolves `-p pkg-name` and asserts:
    - The overlay's top-level `[patch.crates-io.pkg-name].path` exists and is absolutized to `<workspace_root>/../local-fork` (NOT to overlay-root).
    - Option H Rule 1 (INJECT) does NOT inject a synthetic self-patch (the workspace-root entry already covers the member; the merged table is non-empty for `pkg-name`).
    - Option H Rule 2 (REMAP) is applied if the workspace-root entry's path needed re-anchoring (verify on a test variant). Test class: unit. Bites pre-fix: R1's underspecified composition order would either silently overwrite the workspace-root entry (Rule 1 INJECT-after-merge case) or shadow it (Rule 1 INJECT-before-merge case); R2's root-first order produces the correct merged table.

28. `option_h_rejects_member_local_patch_crates_io` — synthesizes a workspace where the MEMBER declares `[patch.crates-io]` (which cargo itself would error on). Resolves `-p the-member` and asserts the resolver returns `Error::Cli` with the directed diagnostic naming the member manifest path. Test class: unit. Bites pre-fix: silent merge of member-local `[patch.crates-io]` with workspace-root `[patch.crates-io]` would produce divergent overlay state vs baseline cargo (which errors). R2's PUNCH-13 surfaces a clear error.

**R2 NEW — Resolver glob / path / exclude / nested tests (BLOCK-4 / PUNCH-29):**

29. `resolver_glob_crates_star_finds_nested_member` — `members = ["crates/*"]`, directory `<root>/crates/foo/` with `Cargo.toml` declaring `[package].name = "foo"`. Resolves `-p foo` → `<root>/crates/foo/Cargo.toml`. Class: unit.
30. `resolver_glob_crates_explicit_nested_literal_finds_member` — `members = ["crates/foo", "tools/bar"]`, resolves `-p foo` to `<root>/crates/foo/Cargo.toml` AND a sibling test resolves `-p bar` to `<root>/tools/bar/Cargo.toml`. Class: unit.
31. `resolver_glob_rejects_deep_glob` — `members = ["**/*"]` or `members = ["crates/**"]`. Resolves `-p anything` → `Error::Cli` directed diagnostic ("cargo does not support `**`"). Class: unit.
32. `resolver_glob_rejects_glob_in_non_final_segment` — `members = ["*/foo"]`. Resolves `-p foo` → `Error::Cli` directed diagnostic ("only the LAST segment may contain glob metachars"). Class: unit.
33. `resolver_glob_normalizes_trailing_slash` — `members = ["axum-macros/"]` (trailing slash) → matches the same directory as `members = ["axum-macros"]`. Resolves `-p axum-macros` succeeds. Class: unit.
34. `resolver_glob_rejects_absolute_path_member` — `members = ["/usr/local/foo"]` → `Error::Cli` directed diagnostic ("entries are workspace-relative paths only"). Class: unit. On Windows, `members = ["C:\\\\workspace\\\\foo"]` produces the same error.
35. `resolver_glob_rejects_parent_traversal_member` — `members = ["../sibling"]` → `Error::Cli` directed diagnostic ("members must be descendants of the workspace root"). Class: unit.

**R2 NEW — Exclude / default-members / nested / duplicate tests (BLOCK-5 / PUNCH-30):**

36. `resolver_subtracts_workspace_exclude_set` — `members = ["pkg-*"]`, `exclude = ["pkg-private"]`. Directory `pkg-private/` exists with `[package].name = "pkg-private"`. Resolves `-p pkg-private` → no match → `Error::Cli` (the exclude subtraction removed the candidate before match). The diagnostic's "scanned" list does NOT include `pkg-private` (it was excluded before the scan). Class: unit. Bites pre-fix: R1 resolver would match `pkg-private` despite the exclude, producing a divergent overlay vs baseline cargo (which respects exclude).
37. `resolver_default_members_does_not_filter_package_resolution` — `members = ["a", "b"]`, `default-members = ["a"]`. Resolves `-p b` → succeeds (the resolver consults `members`, NOT `default-members`). Inverse: `-p a` also succeeds (default-members membership does not bias the resolver). Class: unit. Bites pre-fix: R1 §1 was silent on `default-members` semantics; a naive implementer might filter by `default-members` first, which would break `-p` for non-default members.
38. `resolver_excluded_package_diagnostic_lists_excluded_name` — `members = ["a", "b"]`, `exclude = ["b"]`. Resolves `-p b` → `Error::Cli` with the no-match diagnostic explicitly noting that `b` IS in `members` but ALSO in `exclude`, so it was subtracted before scanning. (Implementer judgment on exact wording; the test asserts that the diagnostic mentions both the package name and that it was excluded.) Class: unit.
39. `resolver_does_not_descend_into_nested_workspace` — `members = ["outer-pkg", "nested-ws"]`. `<root>/outer-pkg/Cargo.toml` is a normal `[package]`. `<root>/nested-ws/Cargo.toml` declares `[workspace] members = ["inner-pkg"]` (a nested virtual workspace). `<root>/nested-ws/inner-pkg/Cargo.toml` declares `[package].name = "inner-pkg"`. Resolves `-p inner-pkg` from the OUTER workspace → no-match → `Error::Cli`. The diagnostic's scanned list includes `outer-pkg` and `nested-ws` (the outer's members), but NOT `inner-pkg` (the resolver does not descend). Sibling assertion: resolving `-p nested-ws` from the outer workspace ALSO no-matches (nested-ws has no `[package]` — it's a pure-virtual nested workspace). Class: unit. Bites pre-fix: R1 §6.1 was vague on traversal boundary; an over-eager implementer might recurse into nested workspaces, which would cross a cargo workspace boundary and produce confusing carry-down state.
40. `resolver_duplicate_package_after_glob_expansion_returns_multiple_match_error` — `members = ["pkg-a"]` + `members = ["pkg-*"]` (overlapping). Both expand to include `pkg-a/`. The de-duplication step canonicalizes the path; if `pkg-a/Cargo.toml` is matched once (correct), `-p pkg-a` resolves cleanly. If a separate manifest at `pkg-a-clone/Cargo.toml` ALSO declares `[package].name = "pkg-a"` (cargo would error on baseline; the test synthesizes the corrupted-workspace shape to exercise the resolver's diagnostic), `-p pkg-a` returns `Error::Cli` case 2 listing both manifest paths. Class: unit. Bites pre-fix: R1's resolver had no de-duplication step; an adopter using overlapping `members` entries would receive spurious multiple-match errors.

**R2 NEW — Package+workspace root rejection test:**

41. `resolver_rejects_package_plus_workspace_root_per_v01_scope` — synthesizes a root manifest declaring BOTH `[package].name = "the-root-pkg"` AND `[workspace] members = ["the-member"]`. Resolves `-p the-member` → `Error::Cli` directed diagnostic per §4.3 step 2.5 (`is_workspace_root_manifest` returns `false` because `[package]` is present; the diagnostic says v0.1.0 supports virtual-workspace-only and points the adopter at the v0.2 follow-up issue once filed). Class: unit. R2 NEW per §1 scope decision.

### 7.3 Integration test (cargo-build-gated)

Test name: `cargo_lihaaf_resolves_axum_macros_shape_workspace_member`. Placement: `tests/compat/overlay_determinism.rs` (the existing integration-test home for compat-mode cargo-build tests).

Setup (synthesized on disk in a tempdir):

```
<tmp>/ws/
  Cargo.toml                  # [workspace] members=["pkg-a","pkg-*"]
                              # [workspace.package] edition="2021"
                              # [workspace.dependencies] serde="1.0"
  pkg-a/
    Cargo.toml                # [package] name="pkg-a" rust-version={workspace=true}
    src/lib.rs
    tests/ui_test.rs          # trybuild::TestCases::new().pass(...)
    tests/ui/
      pass_basic.rs
  pkg-macros/
    Cargo.toml                # [package] name="pkg-macros" rust-version={workspace=true}
                              # [lints] workspace=true
                              # [dependencies] serde={workspace=true}
    src/lib.rs
    tests/ui_test.rs
    tests/ui/
      pass_basic.rs
```

Invocation:

```bash
cargo lihaaf --compat \
  --compat-root <tmp>/ws \
  --compat-report <tmp>/report.json \
  --package pkg-macros
```

Assertions:

1. Exit code 0 (compat run succeeds; the synthesized fixture is `pass_basic.rs` so the inner lihaaf run passes).
2. Overlay manifest staged at `<tmp>/ws/pkg-macros/target/lihaaf-overlay/Cargo.toml`.
3. Overlay manifest has:
   - `[lib] crate-type = ["dylib","rlib"]`
   - `[workspace.dependencies.serde] = "1.0"` (carried down)
   - `[workspace.package.edition] = "2021"` (carried down)
   - NO `[workspace.members]` key (stripped).
4. Report envelope at `<tmp>/report.json` has `mode = "compat"`, `crate_name = "pkg-macros"`, `mismatch_count = 0`.

Gate: `LIHAAF_RUN_CARGO_BUILD_TESTS=1` (per [[lihaaf-no-local-binary-builds]]).

### 7.4 Byte-determinism corpus addition (R2 — CORRECTED, see revision history "INTERNAL-CONSISTENCY surface")

**R1 correction.** R1 invented a TOML schema (`[[entry]]` tables embedded in `compat/baseline.toml`) that does not match the actual code. The real byte-determinism corpus lives at `tests/compat/overlay_corpus/<name>.input.toml` + `<name>.expected.toml` (file-pair fixtures); the test (`byte_identical_across_two_lihaaf_binaries_on_corpus` at `tests/compat/overlay_determinism.rs:432-499`) iterates a hardcoded `names = ["bare_package", "with_rlib_only", "with_cdylib", "with_patch_section", "with_comments", "with_replace_section"]` array (6 names) and asserts `checked == 6`. `compat/baseline.toml` is the unrelated §5 pilot-gate baseline table (`<crate-name>.n_max` ceilings; see `src/compat/gate.rs::parse_baseline`).

**R2 corpus addition.** Add ONE new fixture pair to `tests/compat/overlay_corpus/`:

- `workspace_member_with_package.input.toml` — the **MEMBER's** input `Cargo.toml`. The corpus shape stores ONE `Cargo.toml` per fixture pair (the overlay materialization runs on the member's manifest as input, parameterized by the workspace-root context). The existing fixtures (`bare_package`, etc.) are all member-shape inputs; the new fixture follows the same convention.

  ```toml
  # tests/compat/overlay_corpus/workspace_member_with_package.input.toml
  [package]
  name = "pkg-macros"
  version = "0.1.0"
  edition = { workspace = true }

  [lib]
  proc-macro = true

  [dependencies]
  serde = { workspace = true }

  [lints]
  workspace = true
  ```

- `workspace_member_with_package.expected.toml` — the expected overlay output after `materialize_overlay` + `apply_workspace_member_inheritance` runs against the workspace-root context. The `__UPSTREAM_DIR__` placeholder is substituted at test time with the member's tempdir (per the existing test's substitution convention at `overlay_determinism.rs:478-483`). Sketch (exact bytes determined at implementation time by running the materializer once, capturing output, and pinning it):

  ```toml
  # tests/compat/overlay_corpus/workspace_member_with_package.expected.toml
  [workspace]

  [workspace.package]
  edition = "2021"
  rust-version = "1.65"

  [workspace.dependencies]
  serde = "1.0"

  [workspace.lints.rust]
  unsafe_code = "forbid"

  [package]
  name = "pkg-macros"
  version = "0.1.0"
  edition = "2021"

  [lib]
  proc-macro = true
  crate-type = ["dylib", "rlib"]
  path = "__UPSTREAM_DIR__/src/lib.rs"

  [dependencies]
  serde = { version = "1.0" }

  [lints]
  workspace = true
  ```

  Notes for the implementer:

  - The corpus test uses a SYNTHESIZED workspace-root context: the test setup creates a tempdir containing the workspace root's `Cargo.toml` (with the carry-down tables: `[workspace.package]`, `[workspace.dependencies]`, `[workspace.lints.rust]`) and the member's manifest at `__UPSTREAM_DIR__/Cargo.toml`. The materializer is called with `WorkspaceMemberContext { workspace_root_manifest, workspace_root_value }` populated from the synthesized workspace root.
  - The expected output captures Option A1 carry-down: the workspace-root `[workspace.package]`, `[workspace.dependencies]`, and `[workspace.lints]` tables flow into the overlay's matching top-level `[workspace.*]` tables.
  - The `{ workspace = true }` references on the MEMBER side are NOT re-written to literal values by R2's policy (Option A1 carry-down, not Option A2 reference-rewrite). The overlay carries the inheritance refs verbatim; cargo at build time resolves them against the overlay's own `[workspace.*]` tables.

**Implementer responsibility.** Generate `workspace_member_with_package.expected.toml` by running the implementation once with the synthesized workspace-root + member manifests, capturing the output, and pinning. The expected file's exact bytes are NOT pre-committed in this plan (that would lock the implementer into a specific TOML serializer ordering); the test's contract is "two binaries produce identical output", and the pinned bytes become the contract after the first successful run. If a `toml` crate patch bump produces drift, the corpus test fires and the careful-coder handling the bump regenerates the fixture.

**Count assertion bump.** `overlay_determinism.rs:495-498` asserts `checked == 6`; R2 bumps this to `checked == 7` and extends the hardcoded names array:

```rust
let names = [
    "bare_package",
    "with_rlib_only",
    "with_cdylib",
    "with_patch_section",
    "with_comments",
    "with_replace_section",
    "workspace_member_with_package",  // R2 NEW
];
// ...
assert_eq!(
    checked, 7,
    "corpus must include all 7 representative fixtures"
);
```

The `tests/compat/overlay_determinism.rs:26-28` module-level docstring (`"The cross-binary determinism corpus (byte_identical_across_two_lihaaf_binaries_on_corpus — five [actually six] fixtures synthesized in tempdirs from tests/compat/overlay_corpus/, each checked against a ..."`) ALSO updates: the prose count is bumped from "five" / "six" to "seven" (R2 leaves the implementer judgment on the exact wording; the §10b mirror table verifies the count assertion at line 497 — see §10b row 26 R2 update).

**Compat-test argv shape (separate concern).** `compat/baseline.toml` (the §5 pilot-gate baseline table) is unrelated to the overlay corpus. axum-macros' pilot enrollment is a SEPARATE PR (per §7.6 — outside #53's scope); when that PR lands, the implementer adds an `[axum-macros]` table with `n_max = ...` to `compat/baseline.toml`. R2 does NOT pre-commit that addition; #53 lands the CAPABILITY, the follow-up pilot-enrollment PR uses it.

### 7.5 Backward-compat re-verification

The existing Round-1 pilots (cxx, serde-json, anyhow, thiserror) all enter from the workspace ROOT or from a single-crate root. **None of them sets `-p`.** The new `compat_package: Option<String>` field defaults to `None`, and the existing `resolve_upstream_manifest` path (§4.2 pseudocode step 3 — early return) preserves the existing behavior byte-for-byte.

The implementer must run the existing `byte_identical_across_two_lihaaf_binaries_on_corpus` test (or its #40+#47-updated successor) and verify the 8 existing corpus entries produce unchanged bytes.

### 7.6 Per-pilot Round-2 effect

axum-macros (pilot #4 in Round-2) is the immediate beneficiary. Once #53 lands, axum-macros enrollment in `compat/pilots/` becomes:

- Fork-shape: `tokio-rs/axum` at a pinned SHA.
- Compat invocation: `cargo lihaaf --compat --compat-root . --package axum-macros --compat-report <p>` from the workspace root.
- Baseline argv: `["cargo", "test", "-p", "axum-macros"]`.

The pilot enrollment PR is a separate follow-up (not part of #53's PR). #53 lands the CAPABILITY; the pilot-enrollment PR USES the capability. Per [[lihaaf-round2-fork-shape-analysis]] this is the entry-point precondition Round 2 needed.

### 7.7 Test-name list for §10b mirror table

The §10b mirror table (§10b) MUST reference every test by name. Test names, in order of plan introduction (R2 extends the R1 list with tests 24-41 covering the BLOCK-1 through BLOCK-5 classes from Codex R1):

| § | Test name |
|---|---|
| 7.2 #1 | `resolve_workspace_member_manifest_succeeds_on_literal_member` |
| 7.2 #2 | `resolve_workspace_member_manifest_succeeds_on_glob_match` |
| 7.2 #3 | `resolve_workspace_member_manifest_matches_by_package_name_not_dir_name` |
| 7.2 #4 | `resolve_workspace_member_manifest_glob_does_not_match_bare_prefix` |
| 7.2 #5 | `resolve_workspace_member_manifest_rejects_when_root_not_workspace_root` |
| 7.2 #6 | `resolve_workspace_member_manifest_rejects_when_no_members_array` |
| 7.2 #7 | `resolve_workspace_member_manifest_no_match_lists_scanned_members` |
| 7.2 #8 | `resolve_workspace_member_manifest_skips_unparseable_member_manifest` |
| 7.2 #9 | `resolve_workspace_member_manifest_skips_missing_member_directory` |
| 7.2 #10 | `resolve_workspace_member_manifest_workspace_inheritance_captured` |
| 7.2 #11 | `override_workspace_skips_branch_2_with_workspace_member_context` |
| 7.2 #12 | `override_workspace_skips_branch_3_with_workspace_member_context` |
| 7.2 #13 | `override_workspace_still_rejects_branch_1_explicit_member_with_context` |
| 7.2 #14 | `apply_workspace_member_inheritance_carries_workspace_dependencies` |
| 7.2 #15 | `apply_workspace_member_inheritance_carries_workspace_package_lints_metadata` |
| 7.2 #16 | `apply_workspace_member_inheritance_strips_membership_keys` |
| 7.2 #17 | `apply_workspace_member_inheritance_carries_workspace_root_patch_crates_io` (R2: name retained but the implementation now goes through `apply_self_patch_policy(workspace_root_patch_table: Some(...))` per §5.3.bis composition order; the test asserts the merged-table outcome, not the intermediate inject step) |
| 7.2 #18 | `cli_parses_short_p_flag` |
| 7.2 #19 | `cli_parses_long_package_flag` |
| 7.2 #20 | `cli_rejects_empty_package_name` |
| 7.2 #21 | `cli_rejects_package_outside_compat_mode` |
| 7.2 #22 | `cli_rejects_package_with_compat_manifest` |
| 7.2 #23 | `compat_args_from_cli_carries_compat_package` |
| 7.2 #24 | **(R2 NEW — BLOCK-1)** `dual_root_routing_baseline_cwd_is_workspace_root_member_consumers_use_member_root` |
| 7.2 #25 | **(R2 NEW — BLOCK-2)** `workspace_root_path_absolutization_for_dependencies_path` |
| 7.2 #26 | **(R2 NEW — BLOCK-2)** `workspace_root_path_absolutization_for_package_readme_license_file` |
| 7.2 #27 | **(R2 NEW — BLOCK-3)** `option_h_root_first_member_second_with_workspace_root_self_patch_entry` |
| 7.2 #28 | **(R2 NEW — BLOCK-3)** `option_h_rejects_member_local_patch_crates_io` |
| 7.2 #29 | **(R2 NEW — BLOCK-4)** `resolver_glob_crates_star_finds_nested_member` |
| 7.2 #30 | **(R2 NEW — BLOCK-4)** `resolver_glob_crates_explicit_nested_literal_finds_member` |
| 7.2 #31 | **(R2 NEW — BLOCK-4)** `resolver_glob_rejects_deep_glob` |
| 7.2 #32 | **(R2 NEW — BLOCK-4)** `resolver_glob_rejects_glob_in_non_final_segment` |
| 7.2 #33 | **(R2 NEW — BLOCK-4)** `resolver_glob_normalizes_trailing_slash` |
| 7.2 #34 | **(R2 NEW — BLOCK-4)** `resolver_glob_rejects_absolute_path_member` |
| 7.2 #35 | **(R2 NEW — BLOCK-4)** `resolver_glob_rejects_parent_traversal_member` |
| 7.2 #36 | **(R2 NEW — BLOCK-5)** `resolver_subtracts_workspace_exclude_set` |
| 7.2 #37 | **(R2 NEW — BLOCK-5)** `resolver_default_members_does_not_filter_package_resolution` |
| 7.2 #38 | **(R2 NEW — BLOCK-5)** `resolver_excluded_package_diagnostic_lists_excluded_name` |
| 7.2 #39 | **(R2 NEW — BLOCK-5)** `resolver_does_not_descend_into_nested_workspace` |
| 7.2 #40 | **(R2 NEW — BLOCK-5)** `resolver_duplicate_package_after_glob_expansion_returns_multiple_match_error` |
| 7.2 #41 | **(R2 NEW — scope decision)** `resolver_rejects_package_plus_workspace_root_per_v01_scope` |
| 7.3 | `cargo_lihaaf_resolves_axum_macros_shape_workspace_member` |
| 7.4 | `byte_identical_across_two_lihaaf_binaries_on_corpus` (R2: count assertion bumped from 6 to 7; new fixture pair `workspace_member_with_package.input.toml` + `workspace_member_with_package.expected.toml` added to `tests/compat/overlay_corpus/`) |

---

## 8. Spec amendment

### 8.1 `docs/spec/lihaaf-v0.1.md` §8.2 — new flag entry

Insert a new sub-section between `#### --no-cache` (current line ~1255) and `#### --manifest-path` (current line ~1260):

```markdown
#### `-p <package>`, `--package <package>` (compat mode only)

Workspace-member package selector. When `--compat` is set and
`--compat-root` points at a workspace ROOT manifest (declares
`[workspace]` without `[package]`), `--package <pkg>` resolves the
upstream manifest to the workspace member whose `[package].name`
equals `<pkg>`. The member is located by expanding the
`[workspace.members]` array against the workspace-root directory and
matching candidate manifests by their declared package name (cargo's
`[package].name` is not workspace-inheritable, so the match is on the
member's own field).

The workspace's `[workspace.dependencies]`, `[workspace.package]`,
`[workspace.lints]`, `[workspace.metadata]`, `[workspace.resolver]`,
`[patch.crates-io]`, `[replace]`, and `[profile.*]` tables are carried
down into the staged overlay so the member's
`{ workspace = true }` references and patch resolution match baseline
cargo's behavior at the workspace-root level.

Required when `--compat-root` is a workspace root; rejected otherwise.
Mutually exclusive with `--compat-manifest` (which supplies an explicit
manifest path, bypassing the resolver). Mirrors cargo's `-p` convention.

Example invocation for axum-macros:

```bash
cargo lihaaf --compat \
  --compat-root /path/to/tokio-rs/axum \
  --package axum-macros \
  --compat-report /tmp/report.json
```

The baseline runner argv must also include `-p <pkg>` so cargo runs
only the target member's tests; pass it via `--compat-cargo-test-argv
'["cargo","test","-p","axum-macros"]'`.
```

### 8.2 `docs/compatibility-plan.md` §3.1 — invocation shape + optional-flag bullet

Insert into the existing "Optional options when `--compat` is set" list (currently at lines 71-77), alphabetically positioned between `--compat-manifest` and `--compat-report`:

```markdown
- `--package <pkg>` / `-p <pkg>` — workspace-member package selector. Required when `--compat-root` points at a workspace root (a `Cargo.toml` declaring `[workspace]` without `[package]`); see §3.2.3 for the resolver semantics and §3.1 for the mutual-exclusion with `--compat-manifest`.
```

Add a new bullet to the "Flag-shadowing rule" / "Compat-mode interactions with other v0.1 flags" table (lines 86-100):

```markdown
| `-p <pkg>` / `--package <pkg>` | compat-mode-only, required when `--compat-root` is a workspace root | resolves the member manifest; see §3.2.3 |
```

### 8.3 `docs/compatibility-plan.md` §3.2.3 — new sub-section

Append a new sub-section after the existing §3.2.3 paragraphs (after the symlink-based overlay docs but before §3.3):

```markdown
**Workspace-member entry via `--package`.** When the adopter's target crate is a workspace MEMBER (e.g. axum-macros inside the tokio-rs/axum workspace), compat mode resolves the entry shape via `--compat-root <workspace-root>` + `--package <member-pkg>`. The resolver reads the workspace root's `[workspace.members]` array, expands globs against the workspace-root directory, and matches a member by its declared `[package].name`.

The staged overlay is built at `<workspace-root>/<member-dir>/target/lihaaf-overlay/Cargo.toml`. The workspace root's `[workspace.dependencies]`, `[workspace.package]`, `[workspace.lints]`, `[workspace.metadata]`, `[workspace.resolver]`, `[patch.crates-io]`, `[replace]`, and `[profile.*]` tables are carried down into the staged overlay's `[workspace]` and top-level tables so `{ workspace = true }` inheritance references in the member's manifest resolve, and the dependency-graph + patch resolution match baseline cargo's behavior at the workspace-root level.

The over-broad implicit-ancestor REJECT (PR #37 R4) is suppressed when `--package` is supplied: the adopter has explicitly named the target, so the "accidental member entry" hypothesis no longer applies and the carry-down closes the divergent-dependency-graph risk the REJECT was designed to catch. Without `--package`, the REJECT continues to fire for workspace-member subdirectory entries — that case remains the "accidental entry" guard.

Cargo's own `[package].name` field is NOT workspace-inheritable (see https://doc.rust-lang.org/cargo/reference/workspaces.html#the-package-table for the inheritable-keys list), so the resolver can trust the literal string at the member's `package.name`. The match is case-sensitive.

The baseline `cargo test` invocation runs at the workspace ROOT, not the member dir; adopters who want to compare only the target member's pass/fail must override the baseline argv: `--compat-cargo-test-argv '["cargo","test","-p","<pkg>"]'`.
```

### 8.4 CHANGELOG entry

Add a `### Added` entry to the next release section in `CHANGELOG.md`:

```markdown
### Added

- Compat-mode `--package <pkg>` / `-p <pkg>` flag for workspace-member entry. When `--compat-root` points at a workspace root, `--package` resolves the upstream manifest to the named workspace member. The workspace's `[workspace.*]`, `[patch.crates-io]`, `[replace]`, and `[profile.*]` tables are carried down into the staged overlay so the member's `{ workspace = true }` references and patch resolution match baseline cargo's behavior. Closes #53 — unblocks Round-2 enrollment of axum-macros and similar workspace-member-shape pilots.

  See `docs/compatibility-plan.md` §3.2.3 ("Workspace-member entry via `--package`") and `docs/spec/lihaaf-v0.1.md` §8.2 for the adopter-facing surface.
```

### 8.5 Module-level rustdoc on `src/compat/overlay.rs`

Extend the existing module-level docs (`overlay.rs:1-238`) with a new sub-section under the "Workspace-member cases" paragraph (currently at `overlay.rs:703-723`). The text:

> "**Workspace-member entry via `--package` (issue #53).** When the adopter supplies `--package <pkg>` and `--compat-root` is a virtual workspace root (per the v0.1.0 scope; see compat plan §3.2.3), the resolver (`resolve_workspace_member_manifest`) maps `<pkg>` to the member's manifest path. The materializer takes a `WorkspaceMemberContext` parameter and:
>
> - Skips Branch 2 (implicit-ancestor REJECT) of `override_workspace_inheritance`.
> - Skips Branch 3 (inheritance-refs REJECT) of `override_workspace_inheritance`.
> - Replaces Branch 4's per-upstream `[workspace]` clone with `apply_workspace_member_inheritance(workspace_root_manifest)` — clones the WORKSPACE ROOT's `[workspace.*]` tables and merges the workspace root's `[patch.crates-io]` / `[replace]` / `[profile.*]` into the overlay.
>
> Branch 1 (explicit `[package].workspace = "<path>"`) still REJECTs even with `--package` — the explicit declaration is incompatible with the resolver-determined workspace.
>
> The carry-down ensures the overlay's dependency graph CONVERGES with baseline cargo's: cargo applies the workspace root's `[workspace.*]` and `[patch]` tables when building any member, and the overlay now does the same."

### 8.6 Function-level rustdoc on `resolve_workspace_member_manifest`

Per §4.1 the function carries a 30-60 line docstring covering:

- Purpose (one paragraph).
- Algorithm overview (the §4.3 7 steps).
- Error variants and their diagnostics (§4.4 cases 1-5).
- Workspace-inheritance interaction (`[package].name` is not inheritable; what is inheritable).
- Glob expansion semantics (directory-name only; no `glob` crate dependency).
- Pre-conditions (workspace_root_manifest is the workspace-root path, not a member path; the resolver verifies and rejects otherwise).

### 8.7 Inline source comment alongside the resolver call-site

When wiring `resolve_workspace_member_manifest` into the driver's `resolve_upstream_manifest` (§4.2), add a 5-10 line comment block immediately above the call:

```rust
// Workspace-member entry via `--package` (issue #53). The adopter
// invokes from the workspace root + names a member explicitly; the
// resolver maps the package name to the member manifest path. The
// overlay materializer takes a WorkspaceMemberContext that carries the
// workspace root through, so it can:
//   1. Skip the over-broad implicit-ancestor REJECT (Branch 2 of
//      override_workspace_inheritance) — the adopter opted in.
//   2. Carry the workspace root's [workspace.*] / [patch] / [profile]
//      tables down into the overlay (apply_workspace_member_inheritance)
//      so the dependency graph + patch resolution match baseline cargo.
// See docs/compatibility-plan.md §3.2.3 (workspace-member entry) and
// plan docs/plans/issue-53-workspace-member-entry.md R1.
```

---

## 9. Verifier invocations

### 9.1 Implementer pre-PR mandatory commands (the four hard gates)

Per [[lihaaf-review-verify-cmds]], every implementer dispatch and reviewer dispatch MUST run these commands locally and report pass/fail:

| # | Command | Purpose | Expected outcome |
|---|---|---|---|
| 9a | `cargo fmt --all -- --check` | formatter discipline | exit 0, no diff |
| 9b | `cargo clippy --all-targets -- -D warnings` | lint discipline (warnings as errors) | exit 0, no warnings |
| 9c | `cargo test --lib` | unit-test suite | exit 0, all tests pass |
| 9d | `RUSTDOCFLAGS=-D warnings cargo doc --no-deps` | rustdoc discipline | exit 0, no doc warnings |

**§9 PRE-COMMITMENT:** the implementer's PR description MUST report pass/fail for each of 9a-9d explicitly. A test-suite addition that breaks 9b (e.g. unused-import in a test module) is a BLOCK-class regression.

### 9.2 Cargo-build-gated integration tests (CI-only)

The new integration test `cargo_lihaaf_resolves_axum_macros_shape_workspace_member` is gated behind `LIHAAF_RUN_CARGO_BUILD_TESTS=1` (per [[lihaaf-no-local-binary-builds]] WSL2 OOM avoidance). The implementer:

- Adds the env-var skip-guard at the top of the test (mirroring existing cargo-build-gated tests in `tests/compat/overlay_determinism.rs`).
- Verifies `.github/workflows/ci.yml` already sets `LIHAAF_RUN_CARGO_BUILD_TESTS: "1"` for the test job (per #40+#47 plan §11; the gate should be at line ~56).
- The local pre-PR `cargo test --lib` (9c) will SKIP this test; CI will RUN it. The implementer's PR description must explicitly call this out.

### 9.3 Corpus byte-determinism re-verification

`byte_identical_across_two_lihaaf_binaries_on_corpus` (existing test, updated by #40+#47 to handle 8 entries; #53 bumps to 9):

- Runs in CI without env gate.
- Tests that two `lihaaf` binaries built from clean state produce byte-identical envelopes for every corpus entry, including the new `workspace-member-with-package` entry from §7.4.
- A non-determinism regression here is BLOCK-class.

### 9.4 Pilot-gate smoke (CI workflow)

The §5 pilot-gate smoke (`cargo lihaaf --compat` for each enrolled pilot) does NOT add axum-macros in this PR — that's the follow-up pilot-enrollment PR. The smoke for the existing pilots (anyhow, thiserror, derive_more if landed by Round-2 progress) must continue to pass. CI workflow re-run on this PR's branch confirms.

### 9.5 Final commit-time checklist

The implementer's PR commit message must include:

- Reference to issue #53.
- Reference to this plan's path (e.g. "Plan: docs/plans/issue-53-workspace-member-entry.md R<n> at <commit>").
- 9a-9d pass/fail.
- `LIHAAF_RUN_CARGO_BUILD_TESTS` env-gate verification.
- Test-name list verification (per §10b).

---

## 10. §10b mirror table

Every test the implementer writes / extends has a corresponding grep command the reviewer can run to verify the test exists with the named signature. Each row: test name (per §7.7), verbatim grep command (file-anchored), expected match count, what it verifies.

### 10.1 §10b verification process

The reviewer (Codex / Gemini / strict-swe Opus) runs each grep command from the repo root after the implementer's PR is in the working tree. Each expected match count must be exact (no over-grep, no under-grep). A 0-match row is BLOCK (test missing). A multi-match row is BLOCK (duplicate test names or stale residue).

The §10b table is the contract for reviewer-side completion verification. If a grep target moves at implementation time (e.g. the implementer chose a different test location), the §10b table must be updated in the implementer's PR (in the temporary plan file) before the reviewer pass.

### 10.2 §10b mirror table

| # | Test name | Verbatim grep command | Expected | What it verifies |
|---|---|---|---|---|
| 1 | `resolve_workspace_member_manifest_succeeds_on_literal_member` | `grep -F 'fn resolve_workspace_member_manifest_succeeds_on_literal_member' src/compat/overlay.rs` | 1 | §7.2 #1 — literal member match |
| 2 | `resolve_workspace_member_manifest_succeeds_on_glob_match` | `grep -F 'fn resolve_workspace_member_manifest_succeeds_on_glob_match' src/compat/overlay.rs` | 1 | §7.2 #2 — glob match (axum-* shape pin) |
| 3 | `resolve_workspace_member_manifest_matches_by_package_name_not_dir_name` | `grep -F 'fn resolve_workspace_member_manifest_matches_by_package_name_not_dir_name' src/compat/overlay.rs` | 1 | §7.2 #3 — name-not-dir invariant |
| 4 | `resolve_workspace_member_manifest_glob_does_not_match_bare_prefix` | `grep -F 'fn resolve_workspace_member_manifest_glob_does_not_match_bare_prefix' src/compat/overlay.rs` | 1 | §7.2 #4 — glob matcher correctness (§6.14) |
| 5 | `resolve_workspace_member_manifest_rejects_when_root_not_workspace_root` | `grep -F 'fn resolve_workspace_member_manifest_rejects_when_root_not_workspace_root' src/compat/overlay.rs` | 1 | §7.2 #5 — §6.6a diagnostic |
| 6 | `resolve_workspace_member_manifest_rejects_when_no_members_array` | `grep -F 'fn resolve_workspace_member_manifest_rejects_when_no_members_array' src/compat/overlay.rs` | 1 | §7.2 #6 — no members array diagnostic |
| 7 | `resolve_workspace_member_manifest_no_match_lists_scanned_members` | `grep -F 'fn resolve_workspace_member_manifest_no_match_lists_scanned_members' src/compat/overlay.rs` | 1 | §7.2 #7 — diagnostic content (lists scanned) |
| 8 | `resolve_workspace_member_manifest_skips_unparseable_member_manifest` | `grep -F 'fn resolve_workspace_member_manifest_skips_unparseable_member_manifest' src/compat/overlay.rs` | 1 | §7.2 #8 — defensive non-fatal skip |
| 9 | `resolve_workspace_member_manifest_skips_missing_member_directory` | `grep -F 'fn resolve_workspace_member_manifest_skips_missing_member_directory' src/compat/overlay.rs` | 1 | §7.2 #9 — defensive non-fatal skip |
| 10 | `resolve_workspace_member_manifest_workspace_inheritance_captured` | `grep -F 'fn resolve_workspace_member_manifest_workspace_inheritance_captured' src/compat/overlay.rs` | 1 | §7.2 #10 — ResolvedMember.workspace_root_manifest populated |
| 11 | `override_workspace_skips_branch_2_with_workspace_member_context` | `grep -F 'fn override_workspace_skips_branch_2_with_workspace_member_context' src/compat/overlay.rs` | 1 | §7.2 #11 — Branch 2 suppressed |
| 12 | `override_workspace_skips_branch_3_with_workspace_member_context` | `grep -F 'fn override_workspace_skips_branch_3_with_workspace_member_context' src/compat/overlay.rs` | 1 | §7.2 #12 — Branch 3 suppressed |
| 13 | `override_workspace_still_rejects_branch_1_explicit_member_with_context` | `grep -F 'fn override_workspace_still_rejects_branch_1_explicit_member_with_context' src/compat/overlay.rs` | 1 | §7.2 #13 — Branch 1 NOT suppressed |
| 14 | `apply_workspace_member_inheritance_carries_workspace_dependencies` | `grep -F 'fn apply_workspace_member_inheritance_carries_workspace_dependencies' src/compat/overlay.rs` | 1 | §7.2 #14 — carry-down §5.3 |
| 15 | `apply_workspace_member_inheritance_carries_workspace_package_lints_metadata` | `grep -F 'fn apply_workspace_member_inheritance_carries_workspace_package_lints_metadata' src/compat/overlay.rs` | 1 | §7.2 #15 — carry-down §5.3 |
| 16 | `apply_workspace_member_inheritance_strips_membership_keys` | `grep -F 'fn apply_workspace_member_inheritance_strips_membership_keys' src/compat/overlay.rs` | 1 | §7.2 #16 — membership keys stripped per existing pattern |
| 17 | `apply_workspace_member_inheritance_carries_workspace_root_patch_crates_io` | `grep -F 'fn apply_workspace_member_inheritance_carries_workspace_root_patch_crates_io' src/compat/overlay.rs` | 1 | §7.2 #17 — #40+#47 dependency check |
| 18 | `cli_parses_short_p_flag` | `grep -F 'fn cli_parses_short_p_flag' src/cli.rs` | 1 | §7.2 #18 — `-p` short form parses |
| 19 | `cli_parses_long_package_flag` | `grep -F 'fn cli_parses_long_package_flag' src/cli.rs` | 1 | §7.2 #19 — `--package` long form parses |
| 20 | `cli_rejects_empty_package_name` | `grep -F 'fn cli_rejects_empty_package_name' src/cli.rs` | 1 | §7.2 #20 — empty-name validator |
| 21 | `cli_rejects_package_outside_compat_mode` | `grep -F 'fn cli_rejects_package_outside_compat_mode' src/cli.rs` | 1 | §7.2 #21 — non-compat-mode error |
| 22 | `cli_rejects_package_with_compat_manifest` | `grep -F 'fn cli_rejects_package_with_compat_manifest' src/cli.rs` | 1 | §7.2 #22 — mutual-exclusion error |
| 23 | `compat_args_from_cli_carries_compat_package` | `grep -F 'fn compat_args_from_cli_carries_compat_package' src/compat/cli.rs` | 1 | §7.2 #23 — projection plumbing |
| 24 | `cargo_lihaaf_resolves_axum_macros_shape_workspace_member` | `grep -F 'fn cargo_lihaaf_resolves_axum_macros_shape_workspace_member' tests/compat/overlay_determinism.rs` | 1 | §7.3 — integration test (cargo-build-gated) |
| 25 | corpus fixture input present (R2 corrected) | `test -f tests/compat/overlay_corpus/workspace_member_with_package.input.toml && echo found \|\| echo missing` | output `found` | §7.4 — new corpus fixture input (R2 corrected target — file at `tests/compat/overlay_corpus/`, NOT in `compat/baseline.toml` as R1 misclaimed) |
| 25b | corpus fixture expected present (R2 corrected) | `test -f tests/compat/overlay_corpus/workspace_member_with_package.expected.toml && echo found \|\| echo missing` | output `found` | §7.4 — new corpus fixture expected output |
| 26 | corpus names array extended (R2 corrected) | `grep -F '"workspace_member_with_package",' tests/compat/overlay_determinism.rs` | 1 | §7.4 — count assertion bump from 6 to 7; new entry in `let names = [...]` array at `overlay_determinism.rs:453-460` |
| 26b | corpus count assertion bumped to 7 (R2 corrected) | `grep -F 'checked, 7' tests/compat/overlay_determinism.rs` | 1 | §7.4 — `assert_eq!(checked, 7, ...)` at `overlay_determinism.rs:495-498` |
| 27 | spec §8.2 flag entry | `grep -F '#### \`-p <package>\`, \`--package <package>\` (compat mode only)' docs/spec/lihaaf-v0.1.md` | 1 | §8.1 — spec flag entry |
| 28 | spec §8.2 example block | `grep -F 'cargo lihaaf --compat \\' docs/spec/lihaaf-v0.1.md` | 1 | §8.1 — invocation example present |
| 29 | compat plan §3.1 optional-flag bullet | `grep -F '- \`--package <pkg>\` / \`-p <pkg>\` — workspace-member' docs/compatibility-plan.md` | 1 | §8.2 — compat plan bullet |
| 30 | compat plan §3.2.3 workspace-member sub-section | `grep -F 'Workspace-member entry via `--package`' docs/compatibility-plan.md` | 1 | §8.3 — compat plan sub-section header |
| 31 | CHANGELOG `Added` entry | `grep -F '- Compat-mode `--package <pkg>` / `-p <pkg>` flag for workspace-member entry' CHANGELOG.md` | 1 | §8.4 — CHANGELOG entry |
| 32 | module-level rustdoc extension (R2 — stale `R5 /` marker removed) | `grep -F 'Workspace-member entry via `--package` (issue #53)' src/compat/overlay.rs` | 1 | §8.5 — module-level docs extended (R1's `R5 / issue #53` was copy-paste residue from the #40+#47 R8 plan; R2 strips `R5 /` since this plan is at R2). The §8.5 rustdoc text in this plan is also updated to drop `R5 /`. |
| 33 | inline call-site comment | `grep -F 'Workspace-member entry via `--package` (issue #53)' src/compat/mod.rs` | 1 | §8.7 — driver call-site comment |
| 34 | new field on `Cli` struct | `grep -F 'pub compat_package: Option<String>,' src/cli.rs` | 1 | §3.1 — field present |
| 35 | new field on `CompatArgs` struct | `grep -F 'pub(crate) compat_package: Option<String>,' src/compat/cli.rs` | 1 | §3.2 — projection field present |
| 36 | `resolve_workspace_member_manifest` function exists | `grep -F 'pub(crate) fn resolve_workspace_member_manifest(' src/compat/overlay.rs` | 1 | §4.1 — resolver function |
| 37 | `apply_workspace_member_inheritance` function exists | `grep -F 'fn apply_workspace_member_inheritance(' src/compat/overlay.rs` | 1 | §5.3 — carry-down function (visibility per implementer's choice; existence is the contract) |
| 38 | `WorkspaceMemberContext` struct exists | `grep -F 'struct WorkspaceMemberContext' src/compat/overlay.rs` | 1 | §5.4 — context struct |
| 39 | `parse_compat_package` validator | `grep -F 'fn parse_compat_package' src/cli.rs` | 1 | §6.16 — empty-name validator |
| 40 | empty-name diagnostic text | `grep -F '`--package` requires a non-empty package name' src/cli.rs` | 1 | §6.16 — validator diagnostic |
| 41 | `compat_root` augmented diagnostic | `grep -F 'pass `--package <pkg>` to target a specific workspace member' src/compat/overlay.rs` | 1 | §4.4 case 5 — workspace-root REJECT diagnostic suggests `--package` |
| 42 | Branch 2 augmented diagnostic | `grep -F 'AND target this specific member with `--package' src/compat/overlay.rs` | 1 | §6.9 — Branch 2 diagnostic suggests `--package` |
| 43 | resolver-rejection no-workspace-root diagnostic | `grep -F '`--package <pkg>` requires `--compat-root` to point at a workspace root' src/compat/overlay.rs` | 1 | §4.4 case 4 / §6.6 |
| 44 | resolver-rejection no-members diagnostic | `grep -F 'has `[workspace]` but no `[workspace.members]` array' src/compat/overlay.rs` | 1 | §4.4 case 3 |
| 45 | resolver-rejection no-match diagnostic | `grep -F 'no member of workspace' src/compat/overlay.rs` | 1 | §4.4 case 1 |
| 46 | resolver-rejection multiple-match diagnostic | `grep -F 'multiple workspace members claim' src/compat/overlay.rs` | 1 | §4.4 case 2 |
| 47 | **(R2 NEW — BLOCK-1)** dual-root routing test | `grep -F 'fn dual_root_routing_baseline_cwd_is_workspace_root_member_consumers_use_member_root' src/compat/overlay.rs` | 1 | §7.2 #24 — PUNCH-25 |
| 48 | **(R2 NEW — BLOCK-1)** `DualRoot` struct exists | `grep -F 'pub(crate) struct DualRoot' src/compat/overlay.rs` | 1 | §3.1.bis — dual-root struct |
| 49 | **(R2 NEW — BLOCK-2)** workspace-root path absolutization test (dependencies.path) | `grep -F 'fn workspace_root_path_absolutization_for_dependencies_path' src/compat/overlay.rs` | 1 | §7.2 #25 — PUNCH-26 |
| 50 | **(R2 NEW — BLOCK-2)** workspace-root path absolutization test (readme/license-file) | `grep -F 'fn workspace_root_path_absolutization_for_package_readme_license_file' src/compat/overlay.rs` | 1 | §7.2 #26 — PUNCH-27 |
| 51 | **(R2 NEW — BLOCK-3)** Option H root-first composition test | `grep -F 'fn option_h_root_first_member_second_with_workspace_root_self_patch_entry' src/compat/overlay.rs` | 1 | §7.2 #27 — PUNCH-28 |
| 52 | **(R2 NEW — BLOCK-3)** member-local `[patch.crates-io]` rejection test | `grep -F 'fn option_h_rejects_member_local_patch_crates_io' src/compat/overlay.rs` | 1 | §7.2 #28 — PUNCH-28 inverse / PUNCH-13 |
| 53 | **(R2 NEW — BLOCK-3)** member-local `[patch.crates-io]` rejection diagnostic | `grep -F 'cargo does not permit `[patch]` in workspace members' src/compat/overlay.rs` | 1 | §5.3.bis PUNCH-13 — directed diagnostic text |
| 54 | **(R2 NEW — BLOCK-4)** glob `crates/*` test | `grep -F 'fn resolver_glob_crates_star_finds_nested_member' src/compat/overlay.rs` | 1 | §7.2 #29 — PUNCH-29 |
| 55 | **(R2 NEW — BLOCK-4)** explicit nested literal test | `grep -F 'fn resolver_glob_crates_explicit_nested_literal_finds_member' src/compat/overlay.rs` | 1 | §7.2 #30 — PUNCH-29 |
| 56 | **(R2 NEW — BLOCK-4)** deep-glob rejection test | `grep -F 'fn resolver_glob_rejects_deep_glob' src/compat/overlay.rs` | 1 | §7.2 #31 — PUNCH-29 |
| 57 | **(R2 NEW — BLOCK-4)** deep-glob rejection diagnostic | `grep -F 'cargo does not support `**` in `[workspace.members]`' src/compat/overlay.rs` | 1 | §4.3 step 5 — directed diagnostic text |
| 58 | **(R2 NEW — BLOCK-4)** non-final-segment glob rejection test | `grep -F 'fn resolver_glob_rejects_glob_in_non_final_segment' src/compat/overlay.rs` | 1 | §7.2 #32 — PUNCH-29 |
| 59 | **(R2 NEW — BLOCK-4)** non-final-segment glob diagnostic | `grep -F 'only the LAST segment may contain glob metachars' src/compat/overlay.rs` | 1 | §4.3 step 5 — directed diagnostic text |
| 60 | **(R2 NEW — BLOCK-4)** trailing-slash normalization test | `grep -F 'fn resolver_glob_normalizes_trailing_slash' src/compat/overlay.rs` | 1 | §7.2 #33 — PUNCH-29 |
| 61 | **(R2 NEW — BLOCK-4)** absolute-path rejection test | `grep -F 'fn resolver_glob_rejects_absolute_path_member' src/compat/overlay.rs` | 1 | §7.2 #34 — PUNCH-29 |
| 62 | **(R2 NEW — BLOCK-4)** absolute-path rejection diagnostic | `grep -F '`[workspace.members]` entries are workspace-relative paths only' src/compat/overlay.rs` | 1 | §4.3 step 5 — directed diagnostic text |
| 63 | **(R2 NEW — BLOCK-4)** parent-traversal rejection test | `grep -F 'fn resolver_glob_rejects_parent_traversal_member' src/compat/overlay.rs` | 1 | §7.2 #35 — PUNCH-29 |
| 64 | **(R2 NEW — BLOCK-4)** parent-traversal rejection diagnostic | `grep -F 'members must be descendants of the workspace root' src/compat/overlay.rs` | 1 | §4.3 step 5 — directed diagnostic text |
| 65 | **(R2 NEW — BLOCK-5)** `workspace.exclude` subtraction test | `grep -F 'fn resolver_subtracts_workspace_exclude_set' src/compat/overlay.rs` | 1 | §7.2 #36 — PUNCH-30 |
| 66 | **(R2 NEW — BLOCK-5)** `default-members` non-interaction test | `grep -F 'fn resolver_default_members_does_not_filter_package_resolution' src/compat/overlay.rs` | 1 | §7.2 #37 — PUNCH-30 |
| 67 | **(R2 NEW — BLOCK-5)** excluded-package diagnostic test | `grep -F 'fn resolver_excluded_package_diagnostic_lists_excluded_name' src/compat/overlay.rs` | 1 | §7.2 #38 — PUNCH-30 |
| 68 | **(R2 NEW — BLOCK-5)** nested-workspace traversal-boundary test | `grep -F 'fn resolver_does_not_descend_into_nested_workspace' src/compat/overlay.rs` | 1 | §7.2 #39 — PUNCH-30 |
| 69 | **(R2 NEW — BLOCK-5)** duplicate-after-expansion test | `grep -F 'fn resolver_duplicate_package_after_glob_expansion_returns_multiple_match_error' src/compat/overlay.rs` | 1 | §7.2 #40 — PUNCH-30 |
| 70 | **(R2 NEW — scope decision)** package+workspace root rejection test | `grep -F 'fn resolver_rejects_package_plus_workspace_root_per_v01_scope' src/compat/overlay.rs` | 1 | §7.2 #41 — §1 scope decision |
| 47 | validator rule A: non-compat rejection | `grep -F 'return Err(non_compat_mode_error("--package"));' src/cli.rs` | 1 | §3.3 rule A |
| 48 | validator rule B: mutual exclusion | `grep -F 'cannot be combined: `--compat-manifest` supplies an explicit' src/cli.rs` | 1 | §3.3 rule B |

### 10.3 §10b implementer pre-PR checklist

The implementer must run every grep in §10.2 before opening PR and report any 0-match or multi-match rows. If a grep target moves (e.g. function name changed), update the §10b row in the temporary plan file in the same commit and the change is BOTH the test name AND the grep — keep them in sync.

---

## 11. Counter-signal probe — adjacent scope traps

The implementer is expected to stay within the §1 scope. Adjacent classes the implementer might be tempted to over-touch (with the pre-committed decision):

### 11.1 Non-compat-mode `--package` (OUT of scope)

The v0.1 surface outside compat mode does NOT need a package selector. lihaaf's non-compat path takes `--manifest-path` directly. **Pre-committed:** do NOT add `-p` to non-compat mode. The clap field annotation is shared (`#[arg(...)]` is on the single `compat_package` field), but the validator (`validate_mode_consistency`) makes `--package` a mode error outside compat mode. Adding non-compat `-p` is a v0.2 conversation.

### 11.2 `Cargo.lock` synthesis or rewriting (OUT of scope)

The dylib build relies on cargo's own lockfile discovery from the staged overlay manifest. **Pre-committed:** do NOT add lockfile generation, copying, or rewriting in this PR. Cargo's existing discovery from `<staged-overlay-parent>/../../Cargo.lock` (the workspace root's lockfile) works. The §6.12 edge case acknowledges the small regeneration risk and accepts it.

### 11.3 Multi-package selection (OUT of scope)

`-p A -p B` is rejected at clap parse time (single `Option<String>` field). Multi-package compat runs would multiply the report-envelope complexity (per-package mismatch counts, separate fixture sets, etc.) — out of scope for v0.1.0. **Pre-committed:** if an adopter wants two packages in one workspace covered, they invoke compat mode twice with two `--compat-report` paths.

### 11.4 Glob expansion in `-p` (OUT of scope)

`-p axum-*` is rejected — `-p` accepts a literal package name only. Cargo's own `-p` accepts globs but compat mode does not (would require multi-package selection per §11.3 above). **Pre-committed:** literal-only. Surfaced in §6.5.

### 11.5 Reusing the discovery glob matcher (DECISION DEFERRED to implementer)

The fixture glob matcher in `src/discovery.rs:117-131` exists. The resolver's `[workspace.members]` glob matcher is similar but operates on directory names. The implementer's choice:

- Reuse the discovery helper if its scope allows directory-name use.
- Inline a minimal alternative if the discovery helper introduces unwanted coupling.

§10b row 36 verifies the resolver function exists; the internal pattern matcher's implementation is the implementer's call. **Pre-committed:** either choice is acceptable; reviewer panel verifies the matcher's correctness via §10b row 4 (`glob_does_not_match_bare_prefix`) and §10b row 2 (`succeeds_on_glob_match`).

### 11.6 Workspace inheritance for `[package].name` (CLOSED by §4.3 step 6)

Cargo does not permit workspace inheritance of `[package].name`. The resolver trusts the literal field. **Pre-committed:** do NOT add recursive `{ workspace = true }` resolution for `[package].name`. The cargo reference cite in §4.3 step 6 is the contract.

### 11.7 Auto-injection of `-p` into baseline argv (OUT of scope)

The baseline runner's `cargo test` invocation receives the argv verbatim from `--compat-cargo-test-argv`. **Pre-committed:** do NOT inject `-p <pkg>` automatically when `--package` is set. Rationale: adopters may want a workspace-wide baseline (compare overall workspace pass/fail) or a member-specific one. The choice is theirs.

### 11.8 Restructure pilot fork to standalone repo (REJECTED — issue body)

The issue body explicitly says: "Restructure pilot fork — forks lihaaf's pilot infra from upstream's actual workspace layout — bad fidelity. **Don't pick this.**" **Pre-committed:** do NOT restructure the axum-macros fork; ship the `-p` resolver.

### 11.9 Touching the #40+#47 `apply_self_patch_policy` function (R2 UPDATE — #40+#47 IS LANDED)

**R2 update.** PR #56 (#40+#47 Option H + staged-mirror) is merged on `main` at commit `cb5d5bf` as of 2026-05-18 (before this plan starts implementer dispatch). The R1 conditional branch ("if #40+#47 has NOT landed…") is REMOVED.

The resolver's `apply_workspace_member_inheritance` MUST call into `apply_self_patch_policy` (or the function's actual final name on `main`; verify the exact symbol via `grep -F 'fn apply_self_patch_policy' src/compat/overlay.rs` before implementer dispatch) for the workspace root's `[patch.crates-io]` carry-down per §5.3.bis composition order (root-first, member-second). The implementer:

- Extends the existing function with an optional `workspace_root_patch_table: Option<&toml::Value>` parameter. When `Some`, the function:
  - Takes the supplied workspace-root patch table as the input (with path entries absolutized against `workspace_root` per §3.2.bis policy table).
  - Runs Rules 1-4 against this effective table with the MEMBER as the "self" target.
  - Returns the merged output for write into the overlay's top-level `[patch.crates-io]`.
- When `None` (the non-`--package` path), the function behaves exactly as today.
- Does NOT rewrite the function's core 4-rule policy.

The §5.3.bis composition order constraint is the contract; reviewer verifies via §10b row 17 (apply_workspace_member_inheritance test) + row 51 (Option H root-first test).

### 11.10 Cross-platform symlink concerns (OUT of scope for #53)

The staged-overlay path uses `target/lihaaf-overlay/Cargo.toml` (relative to the member dir). The package-root mirror (per #40+#47 plan §4.5 — now landed on `main` at `cb5d5bf` via PR #56) is the only cross-platform symlink concern, and it's in #40+#47's scope, not #53's. **Pre-committed:** do NOT add new cross-platform symlink logic in #53.

### 11.11 Package+workspace root shape (R2 — scope decision, v0.2 / v1.0 follow-up)

v0.1.0 `--package` is scoped to **virtual workspaces only** (workspace root declares `[workspace]` without `[package]`). See §1 Out of scope for the rationale.

**Implementer responsibility for the v0.2 follow-up.** When this plan lands, the implementer's PR description MUST file a follow-up GitHub issue titled `compat: --package support for package+workspace root shape (v0.2)` with the following acceptance-criteria sketch (the v0.2 / v1.0 design conversation owns the final shape):

- Decide whether `--package <root-pkg-name>` (matching the root's own `[package].name`) should target the root's `[package]` (which is a publishable crate AT the workspace root) OR REJECT (since the workspace root is the coordinator, not a member subdirectory).
- Decide how the root's `[lib]` / `[dependencies]` keys interact with carry-down: are the root's `[dependencies]` ALSO carried into the overlay when targeting a member, OR are they ignored (since the member has its own `[dependencies]`)?
- Decide whether the root being publishable changes any `[package].publish` / `[package].license` handling in carry-down.
- Add resolver tests for the package+workspace root shape; the resolver's step 2.5 REJECT diagnostic is replaced with the v0.2 policy.

R2 §4.3 step 2.5 ensures the v0.1.0 resolver REJECTs the package+workspace shape with a clear diagnostic, so adopters of that shape get a direct pointer to the v0.2 follow-up rather than a silent or confusing failure.

**Why v0.2 not v0.1.0.** The only Round-2 pilot needing `--package` is axum-macros, which lives in a virtual workspace. No v0.1.0 GA pilot is blocked by the package+workspace scope deferral. Adopters of package+workspace shape are a smaller cohort (most modern multi-crate libraries adopt the virtual-workspace shape); v0.2 is the right timing to design the policy properly rather than rush a decision for v0.1.0 GA.

---

## 12. Revision history

- **R1 (2026-05-18, initial)** — drafted by strict-swe Opus planner. All 12 sections written in one pass. Pre-adversarial-review (Codex xhigh expected to flag at least one round; per [[lihaaf-plan-adversarial-cycle]]).
- **R2 (2026-05-18, post-Codex-R1-BLOCK + sweep-after-review)** — see top-of-plan "Revision history" section for the full R1 → R2 delta map. Summary: 6 HARD_BLOCKs addressed (dual-root plumbing, root-relative path absolutization, Option H composition order, glob expansion, exclude/default/nested/duplicate, test coverage); 4 FIX_BEFORE_IMPL items addressed (stale #40+#47 conditional, spec amendments accuracy, status references, virtual-vs-package+workspace scope); 1 DEFERRABLE_NIT addressed (§10b row 32 stale R5 marker); 1 INTERNAL-CONSISTENCY surface corrected (R1 §7.4 invented a TOML schema that did not match the actual code; R2 §7.4 rewritten to target `tests/compat/overlay_corpus/` fixture pairs). All 30 punch items from Codex's class-enumeration sweep are addressed across the new §3.1.bis, §3.2.bis, §5.3.bis, expanded §4.3 steps 2.5/3.5/4.5/6.5/7.5/8, and expanded §7.2 tests 24-41 + §10b rows 47-70.

---

## Open items the implementer must NOT decide alone

- Whether `apply_workspace_member_inheritance` (or equivalent) is a NEW function or an extension of the existing Branch 4 of `override_workspace_inheritance`. R1 specifies new function for clarity; implementer may inline into Branch 4 if reviewer panel accepts. Either choice keeps the §10b grep row 37 satisfied (function-name match; if inlined, row 37 is replaced by "inlined; verify Branch 4 takes workspace_member_context: Option<&WorkspaceMemberContext> parameter" — implementer updates §10b in the temporary plan file accordingly).
- The exact integration-test fixture surface (synthesized in tempdir vs new pilot-style fixture). R1 specifies synthesized-in-tempdir; if the implementer wants to use a checked-in fixture under `tests/fixtures/`, the cleanup pattern from the existing tests applies.
- The `DualRoot` struct's exact visibility — `pub(crate)` (R2 default, allows the driver in `src/compat/mod.rs` to construct + consume) vs `pub(super)` (tighter scoping, requires the resolver and driver to share a module). R2 §3.1.bis specifies `pub(crate)`; implementer may flatten if the reviewer panel prefers, as long as §10b row 48 grep still succeeds (`pub(crate) struct DualRoot` may become `pub(super) struct DualRoot` or similar — update §10b row 48 in the temporary plan file).
- Whether the baseline-runner argv auto-injection (§6.13) should be reconsidered as a v0.2 follow-up. R1 keeps it adopter-explicit; if a Round-2 pilot UX session shows the manual `--compat-cargo-test-argv` override is consistently painful, file a v0.2 follow-up. Out of scope for #53.
- The exact wording of the v0.2 follow-up issue filed per §11.11 (package+workspace root shape support). R2 specifies the title `compat: --package support for package+workspace root shape (v0.2)` and a sketch acceptance-criteria list; the implementer files the issue when this plan lands and the URL goes into the §11.11 reference.

R2 closes the following R1 open items (which are now pre-committed in R2):

- `[profile.*]` carry-down policy → §5.3.bis PUNCH-16 closes this: workspace-root `[profile.*]` is carried verbatim; member-local `[profile.*]` is ignored by cargo at build time anyway, so the implementer does NOT need to handle member-local profile precedence (it's a no-op).
- `ResolvedMember` struct vs tuple → §3.1.bis closes this by consolidating into the `DualRoot` struct; the resolver returns `(PathBuf, toml::Value)` (a minimal tuple — just enough for the driver to construct a `DualRoot`), and `DualRoot` is the public API.
- `UpstreamManifest` enum vs flat struct → §3.1.bis closes this by replacing the enum with the `DualRoot` struct (single shape, collapse via the `workspace_member_context: Option<_>` field).

All other decisions in this plan are pre-committed; the implementer follows them as-written unless adversarial review of THIS plan (R2 → R3) flags them.
