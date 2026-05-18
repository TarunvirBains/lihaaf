# ⚠️ TEMPORARY ARTIFACT — DELETE AFTER POST-IMPLEMENTATION REVIEW-ALLOW

This is a **pre-implementer-dispatch design plan**, NOT durable repository documentation.

- Lives on branch `docs/v01-plan-artifacts` for the duration of the implementer-dispatch + adversarial-review cycle for lihaaf #40 + #47.
- The implementer's PR (target: `careful-coder` Opus) MUST `rm docs/plans/issue-40-47-overlay-vs-registry.md` as part of its diff so this file does not land on `main`.
- If the implementer's PR is reviewed and ALLOWed but this file is still present, the post-merge cleanup must remove it before the next release branch cuts.
- Codex R7 ALLOWed this plan on 2026-05-18 (see `docs/audits/trybuild-parity-2026-05-18.md` for the related v1.0.0 parity context).

---

# Plan: lihaaf #40 + #47 — overlay self-`[patch.crates-io]` injection

Revision: R8 (2026-05-18, post-Codex-post-implementation-diagnosis)

Issue: https://github.com/TarunvirBains/lihaaf/issues/40 (compat: serde_json "ambiguous specification" resolution-time failure)
Issue: https://github.com/TarunvirBains/lihaaf/issues/47 (compat: cxx pilot fails with "links = cxxbridge1" collision in beta.6 overlay)

Status entering: beta.6 (workspace-identity fix from PR #37 already landed, anyhow + thiserror clean, cxx + serde-json still red).
Status target: v0.1.0 (per [[lihaaf-v01-ga-gate]] 2026-05-18 correction — #40 + #47 are v0.1.0 blockers; cxx + serde-json clear is required before v0.1.0 GA).

R1 → R2 deltas (summary; details in §2 / §4 / §5 / §6 / §7 / §10):

- BLOCK-1: patch target reaimed at the STAGED OVERLAY DIR (`<upstream>/target/lihaaf-overlay/`), not the upstream dir. The R1 target was a self-loop (same source-id as upstream).
- BLOCK-2: conflict-comparison now uses lexical path normalization (`Path::components()`), so `<upstream>/.` and `<upstream>` compare equal. Idempotency policy explicit.
- BLOCK-3: repro tests now add a root `[dependencies] test-suite = { path = "test-suite" }` edge so the workspace member enters the overlay's resolved graph (since `members` is stripped per overlay.rs:812-829).
- FIX-4: cxx-shape test now includes a minimal `build.rs` alongside `links`.
- FIX-5: corpus-list `names` array bumped to 7 entries; expected-count assertion bumped to 7.
- FIX-6: emission preserves absolutized form (continues §3.2.3 contract); comparison uses lexical normalization. Documented explicitly.
- FIX-7: pilot-citation language is now context-not-correctness; synthetic repros are authoritative.
- FIX-8: corpus-test reference corrected to `byte_identical_across_two_lihaaf_binaries_on_corpus`.
- FIX-9: vendored/forked-upstream escape hatch documented as v0.2/v1.1 follow-up; current REJECT is conservative.
- Docs: new §10 mirrors plan #43 R2 §9 (README + docs/compatibility-plan.md §3.2.3 + CHANGELOG + spec). User-guide docs land in the SAME PR per the user's hard requirement.

R2 → R3 deltas (summary; details in §2 / §4 / §5 / §6 / §7 / §10 / §11):

- **SEC-5 (HARD BLOCK)**: REJECT-on-conflict policy contradicted the very cxx pilot the plan claims to fix. cxx upstream carries `[patch.crates-io.cxx] = { path = "." }`, and the existing `cargo_accepts_rich_overlay_for_dylib_build` test (`overlay_determinism.rs:1696-1815`) exercises exactly this `rich-demo = { path = "." }` shape and EXPECTS overlay-then-cargo success. R3 replaces REJECT with **DETECT-AND-PRESERVE-PER-KEY (Option A + D hybrid)**: if upstream's `[patch.crates-io.<self>]` already exists, R3 evaluates whether the upstream's existing entry already accomplishes the intent (per the §6.1 decision table); if so, R3 SKIPS injection and preserves the upstream's entry verbatim. Other-crate `[patch.crates-io.<X>]` entries are always preserved verbatim (orthogonal scope). The `--compat-allow-patch-override` escape hatch is no longer required for v0.1.0 / v1.0.0 and is struck from §7.1.
- **SEC-6 (HARD BLOCK)**: §5.2 synthetic repros' "foo → test-suite → foo" dependency graph is NOT a cycle in cargo's resolver model — after the patch, the two `foo` references resolve to the SAME source (the staged-overlay dir), and cargo collapses them to a single package (self-reference, not cycle). R3 documents this in new §5.2.0 with a citation to cargo source behavior and confirms via the existing `cargo_accepts_rich_overlay_for_dylib_build` test which uses the same shape (`rich-demo` root + `rich-demo-impl` path-dep workspace member dep-by-name on `rich-demo` is NOT in the existing test, but the same self-patch path-via-`"."` IS, demonstrating cargo accepts the topology with self-patching). R3 explicitly verifies via the cargo-build-gated test that the patched topology resolves.
- **TER-4 (HARD BLOCK)**: with SEC-5 resolved via DETECT-AND-PRESERVE, the `--compat-allow-patch-override` flag is no longer needed for v0.1.0 cxx resolution. The "future flag" deferral in §7.1 is struck. v0.2/v1.1 may still add an explicit override flag for the rare vendored-fork-with-incompatible-patch case, but it is not a v0.1.0 blocker and not coupled to issue #47.
- **BLOCK-2 (PARTIAL — closed)**: §4.1.1 normalizer test cases extended to cover repeated separators (`//`) — `Path::components()` collapses these naturally on Unix — and to document the symlink-equivalence boundary explicitly (lexical normalization does NOT resolve symlinks; symlinked-equivalent paths compare unequal at the lexical layer; known limitation surfaced in §6.X).
- **BLOCK-3 (PARTIAL — closed)**: §5.2.4 test names the cxx-shape's real `links = "cxxbridge1"` value (synthetic still uses `foo-native` to avoid name collisions in test fixtures, but the test description references the real string); §5.2.5 names the serde-json-shape's real `specification serde_json is ambiguous` error string.
- **SEC-3 (wording)**: §3 line about "survives in the upstream" reworded — the prior-injection state is reproduced deterministically by recomputation, not "survives in upstream."
- Tests: §5.1 gains tests `lexical_path_normalize_handles_repeated_separators` and `lexical_path_normalize_does_not_resolve_symlinks`. §5.1 gains tests for the new policy: `materialize_preserves_upstream_self_patch_when_cxx_shape`, `materialize_injects_when_upstream_clean_anyhow_shape`, `materialize_noop_when_upstream_has_valid_path_override_for_target_crate`. §5.2 gains a new cargo-build-gated test `cargo_accepts_overlay_with_preserved_upstream_self_patch_cxx_shape`.
- Docs: §10 updated to describe DETECT-AND-PRESERVE policy in adopter-facing terms. §11 adds CI-first audit: every §5 test runs in CI; the LIHAAF_RUN_CARGO_BUILD_TESTS gate is confirmed flipped on in `.github/workflows/ci.yml:56`.

R3 → R4 deltas (summary; details in §2.6 / §3 / §4 / §5 / §6 / §7 / §10 / §11):

- **SEC-7 (HARD BLOCK)**: R3's PRESERVE-PATH branch claimed to preserve the upstream's `[patch.crates-io.cxx] = { path = "." }` verbatim in the overlay. Codex R3 surfaced that cargo anchors `[patch.crates-io.X].path` relative to **the manifest declaring the patch**, not relative to the manifest the path came from. So when lihaaf copies cxx's upstream `path = "."` verbatim into the staged overlay manifest, cargo re-anchors `.` against the staged-overlay dir (NOT the upstream dir as R3 reasoned). This actually works out FOR CXX — cargo resolves the preserved-verbatim `.` as the staged-overlay dir, which is the same source-id as the package being built, so cargo correctly treats it as a self-reference rather than a competing source. But R3's REASONING was wrong (it claimed `.` would absolutize to `<upstream>/.` in the overlay output, which is what happens during the `absolutize_patch_paths` pass but then cargo would still re-anchor at READ time if the path were relative — and the absolutized form `<upstream>/.` is in fact wrong for the general case). R4 corrects the source-id reasoning in §2.6 and replaces the per-key PRESERVE policy with **Option H (intent-aware self-patch REMAP)**: when the upstream's existing `[patch.crates-io.<self>]` path resolves to the upstream root crate (in upstream context), R4 emits a path that resolves to the staged-overlay root crate (in overlay context — i.e. `path = "."` literally, OR the absolutized staged-overlay-dir for clarity). The cxx case is now handled by Rule 2 (REMAP), not Rule 2's accidentally-correct PRESERVE-AS-IS that R3 relied on.
- **SEC-8 (HARD BLOCK)**: R3 §5.2.0 cited the existing `cargo_accepts_rich_overlay_for_dylib_build` test as proof cargo accepts the cxx-shape patched topology. Codex R3 correctly identified that the cited test exercises `rich-demo = { path = "." }` at a DIFFERENT topology than `foo → test-suite → foo`. R4 §5 adds a load-bearing cargo-build-gated test `cargo_accepts_root_to_test_suite_to_root_topology` that actually proves cargo accepts the patched root→member→root graph (SEC-6 / SEC-8 closure) and stops relying on the misleading existing test as the proof.
- **TER-5 (open / scope-out)**: Codex R3 flagged the absence of git-dependency coverage in §5. R4 adds `cargo_accepts_git_dependency_branch_in_patched_graph` (test 6) OR scopes out git deps explicitly with rationale (no real cxx-shape pilot in the corpus needs git deps; defer behind v0.2/v1.1 follow-up). R4 documents both options and chooses scope-out with explicit rationale in §6.13.
- **§2.6 NEW**: cargo `[patch.crates-io].path` anchoring analysis with citations. Pinpoints that the `.path` is relative to the manifest declaring the patch (= staged overlay manifest, NOT upstream manifest). Explains why R3's PRESERVE-AS-IS-of-`path=.` works for cxx specifically (the relative `.` follows the manifest move) but FAILS for the general case where upstream's path is non-`.` (e.g. `path = "../my-fork"` would re-anchor at the staged overlay dir, NOT the upstream dir → wrong source-id). Cites cargo book § "The [patch] section" + cargo source `cargo/core/manifest/mod.rs` (PathSource resolution).
- **§3 REWRITE**: Replace R3's PRESERVE-PER-KEY narrative with **Option H 4-rule decision tree** (INJECT / REMAP / continue-absolutize / REJECT). Each rule has detection condition, action, worked example.
- **§4 UPDATE**: Materializer algorithm parses upstream `[patch.crates-io.<root>]` (if any), determines whether its resolved target IS the upstream root crate via lexical normalization (exact match to upstream dir name OR canonical equivalent), and applies Rule 1 / 2 / 3 / 4. Algorithm placement is AFTER `absolutize_path_bearing_keys` and BEFORE `override_workspace_inheritance` (unchanged from R3).
- **§5 EXPANDED**: 10-test list per Option H (Rule 1 INJECT, Rule 2 REMAP, Rule 3 continue-absolutize, Rule 4 REJECT) plus SEC-8 cargo-graph proof, lexical-normalize corner cases, and orthogonal-key preservation. Per-test cargo gate columns updated.
- **§6.1 REWRITE**: Decision-table rewritten as the 4-rule Option H tree (was R3's 4-branch DETECT-AND-PRESERVE). Adopter-facing version in §10 mirrors this.
- **§7.1 UPDATE**: Escape hatch (`--compat-allow-patch-override`) still v0.2/v1.1, BUT scoped only to Rule 4 (vendored fork / non-root patch target). cxx is handled by Rule 2 (REMAP), so the escape hatch is no longer needed for v0.1.0 cxx resolution (unchanged from R3 conclusion; only the rule-routing changes).
- **§11 UPDATE**: CI-first audit table updated to match Option H's 10-test surface. `LIHAAF_RUN_CARGO_BUILD_TESTS` gate verified set at `.github/workflows/ci.yml:56`.

R5 → R6 deltas (summary; details in §4.5.2 / §4.5.6 / §5.1.4 / §5.1.14 / §5.1.15 / §11 / revision history):

- **ID.1 (Codex R5 BLOCK — mirror lifecycle / idempotency gap):** R5's §4.5.2 pseudocode said only "create a symlink at `<staged-overlay>/E → <upstream>/E`" with no behavior specified for rerun when entries already exist. A 22-item sweep (15 rerun-state cases + 7 contract decisions) identified the class. R6 adopts **Option B (Idempotent skip + reconcile-by-replacement)** as the full idempotency contract. The mirror step now skips only when the current staged symlink is already canonical (CASE 2 — analogue of `overlay.rs:527-531` bytes-match skip); for all other 14 cases it reconciles by replacing stale state, running a stale-cleanup pass, and asserting the CASE 15 post-condition.
- **ID.2 (§4.5.2 pseudocode updated):** Bare "create a symlink" loop replaced with the full per-case decision tree (CASEs 1–9 forward pass, stale-cleanup Group C pass, CASE 15 post-condition assertion). Copy-fallback exact-sync semantics (no merge; MUST remove destination-only files) specified inline.
- **ID.3 (new §4.5.6 "Idempotency / rerun-state reconciliation"):** New subsection with Option B chosen-strategy statement, 15-case rerun-state table (Groups A and B), and 7 idempotency-contract decisions. Existing §4.5.6 (apply_self_patch_policy interaction) renumbered §4.5.7; existing §4.5.7 (known limitations) renumbered §4.5.8. §4.3 doc reference updated to §4.5.8.
- **ID.4 (§5.1.4 extended):** `apply_self_patch_idempotent_second_materialize` test extended with four Option B assertions: second call returns `Ok(_)` (no AlreadyExists / OverlayMirrorFailed), staged state identical after second call, generated `Cargo.toml` remains a regular file, CASE 2 skip preserves symlink inode identity for already-canonical entries.
- **ID.5 (new §5.1.14):** `mirror_upstream_rerun_reconciles_stale_entries` unit test covering CASE 3 (wrong-target symlink), CASE 5 (real file ↔ upstream file: replace with canonical symlink), CASE 6 (real directory ↔ upstream directory: replace with canonical symlink in symlink mode), CASE 7 (type mismatch file ↔ dir), and CASE 12 (mixed partial state with one correct and one stale entry in the same run).
- **ID.6 (new §5.1.15):** `mirror_copy_fallback_exact_sync_removes_destination_only_files` unit test (CASE 6) verifying that copy-fallback exact-sync removes destination-only files after upstream deletion between runs (decision 5 of the idempotency contract).
- **ID.7 (§11 dispatch-required list extended):** Items 17–19 added to the dispatch-required test list: §5.1.4 extended assertions + §5.1.14 + §5.1.15.

R4 → R5 deltas (summary; details in §2.6 / §3.2 / §4.1 / §5.2 / §4.5 / revision history):

- **M.1 (Codex R4 BLOCK-1 — materialization gap — staged package-root mirror strategy):** Codex R4 surfaced that the staged-overlay dir is currently EMPTY except for the generated `Cargo.toml`. Build scripts (`build.rs`) in cxx, anyhow, and thiserror access package-root-relative files (`src/cxx.cc`, `include/cxx.h`, `src/nightly.rs`, `build/probe.rs`) via `CARGO_MANIFEST_DIR` / cwd which Cargo sets to the package manifest dir — i.e. the staged-overlay dir when the overlay manifest is built. An empty overlay dir causes these file reads to fail (hard-error for cxx; silent-false for anyhow's and thiserror's probe files). R5 documents the **staged package-root mirror strategy** in new **§4.5 (Staged Package-Root Mirror)**: for each top-level entry in `<upstream>/` EXCEPT `target/`, `.git/`, and the generated `Cargo.toml`, create a symlink in the staged overlay dir pointing back to the upstream entry. Copy fallback on Windows / permission-denied / symlink-unavailable scenarios. The staged `Cargo.toml` remains the only WRITTEN file in the overlay; everything else is a symlink (or copy) of upstream. Build scripts see the symlinked tree as if it were the upstream package root: `manifest_dir.join("src/cxx.cc")`, `Path::new("src").join("nightly.rs")`, `Path::new("build").join("probe.rs")` all resolve to the real upstream files.
- **M.2-M.3 (cxx `build.rs` package-root access — §3.2 closure):** `cxx build.rs:143-148` reads `src/cxx.cc` and `build.rs:154-159` references `include/cxx.h` via `CARGO_MANIFEST_DIR`. §3.2 cxx entry updated to enumerate these as covered access patterns under the staged-mirror strategy.
- **M.4 (§5.2.6 test upgrade — cxx cargo-build test now exercises real build.rs file reads):** The R4 `build.rs: fn main() {}` stub in §5.2.6 does not exercise the cxx-shape file-read pattern. R5 upgrades §5.2.6 to a cargo-build-gated test whose `build.rs` reads `CARGO_MANIFEST_DIR`, accesses a `src/cxx.cc`-shape native file and an `include/cxx.h`-shape header, and FAILs against the manifest-only overlay (empty dir) and PASSes when the staged-mirror strategy is implemented.
- **M.5 (anyhow `build.rs` silent-false probe pattern — §3.2 + §5.2 new test):** `anyhow build.rs:255-257` and `:323-367` compile `Path::new("src").join("nightly.rs")` from cwd to probe for nightly features. The probe does NOT error on missing file — it returns false, silently disabling nightly cfg. §3.2 anyhow entry updated with this DANGER note. New §5.2 test `cargo_build_anyhow_shape_probe_file_resolves_via_mirror` verifies the probe succeeds (returns correct boolean, not silent-false) when the staged-mirror strategy provides `src/nightly.rs` via symlink.
- **M.6 (thiserror `build.rs` silent-false probe pattern — §3.2 + §5.2 test):** `thiserror build.rs:261-263` and `:328-371` compile `Path::new("build").join("probe.rs")` from cwd. Same silent-false pattern as M.5. §3.2 thiserror entry updated. §5.2 test for thiserror-shape probe (or notes M.5 test covers the class; see §5.2).
- **Verified non-drivers (serde_json / derive_more / axum-macros) documented in §3.2:** serde_json has a build.rs but it is env-only (no package-root file read); derive_more root has a build.rs but no package-root file read; axum-macros declares `build = false`. These are now explicitly called out in §3.2 as verified non-drivers, not silently omitted.
- **AC.1 (v0.1.0 framing throughout):** Plan previously said `v1.0.0` target and "v0.1.0 GA does NOT require all-4-clean". Per the 2026-05-18 user milestone correction, #40 + #47 ARE v0.1.0 blockers. All "v1.0.0 work" framing updated to v0.1.0. CHANGELOG target section updated.
- **AC.2 (pilot inventory precision — 4 build-script classes):** §3.2 now distinguishes: (1) has `build.rs` AND reads package-root files — hard-error (cxx) or silent-false (anyhow, thiserror); (2) has `build.rs` but env-only / no package-root file read (serde_json); (3) has `build.rs` root but no package-root file read (derive_more); (4) declares `build = false` (axum-macros). Prior conflation of "pure Rust" with "no build.rs" removed.
- **C.1 (§2.6 cargo anchoring citation fix):** The quoted sentence "Relative paths are resolved relative to the manifest in which they appear." does not appear verbatim on the cited Cargo Book `[patch]` page. R5 replaces it with the verifiable Cargo Book `[patch]` dependency-like citation + path-dep anchoring citation + cargo source citation using `manifest_ctx.file.parent().join(path)`.
- **C.2 (overlay.rs line citation fix):** Three occurrences of `:2431-2439` in the plan cite a test function, not the production `[patch] "."` preservation check. R5 replaces these with the correct production citation: `absolutize_patch_paths` at `overlay.rs:1393` (`.is_absolute()` check) and `:1402` (absolutized form emission).
- **C.3 ("step 6" + stale §5.2.X numbering):** "per step 6" was a call-site ordering note inadvertently written as an internal step label; replaced with the exact ordering phrase. `§5.2.X` stale placeholder references replaced with `§5.2.9`.

---

## 1. Problem statement

Beta.6 refresh-pilots run [26012006199](https://github.com/TarunvirBains/lihaaf/actions/runs/26012006199) confirms two distinct downstream cargo failures with a shared root cause.

**cxx (#47):**

```
error: failed to select a version for `cxx`.
    ... required by package `cxx-test-suite v0.0.0 (...)`
    ... which satisfies path dependency `cxx-test-suite` of package `cxx v1.0.194 (.../target/lihaaf-overlay)`

package `cxx` links to the native library `cxxbridge1`, but it conflicts with a previous package
which links to `cxxbridge1` as well:
package `cxx v1.0.194 (.../target/lihaaf-overlay)`
Only one package in the dependency graph may specify the same links value.
```

**serde-json (#40):**

```
error: specification `serde_json` is ambiguous
help: re-run this command with one of the following specifications
  path+file:///home/runner/work/lihaaf/lihaaf/target/lihaaf-overlay#serde_json@1.0.149
  registry+https://github.com/rust-lang/crates.io-index#serde_json@1.0.149
```

**Shared root cause.** When the staged overlay at `<upstream>/target/lihaaf-overlay/Cargo.toml` re-declares the upstream as a path-source package, cargo sees TWO distinct sources for the same crate-name+version pair:

1. The overlay itself — `path+file://.../target/lihaaf-overlay` carrying `[package].name = "cxx"` (or `serde_json`).
2. The registry — `registry+https://github.com/rust-lang/crates.io-index` resolved transitively through any path-dep / workspace-member that names the package by registry-id.

Cargo treats these as different packages — there is no canonical "the path version IS the registry version" assertion in the manifest. Downstream symptoms diverge by what other crates in the resolved graph reference the package-under-test BY NAME:

- cxx's `cxx-test-suite` member declares `[dependencies] cxx = "1.0"` AND `cxx` declares `links = "cxxbridge1"` → both copies of cxx in the resolved graph claim the same native library → `links` collision.
- serde-json's `serde_json-test-suite` member declares `[dependencies] serde_json = "1.0"` → cargo cannot decide which `serde_json` the test-suite means → `ambiguous specification` at resolution time.

The workspace-identity fix in PR #37 (R1–R4, beta.5 → beta.6) stopped cargo from claiming the overlay's package as a workspace member of the upstream `[workspace]`. It did NOT teach cargo "the path version IS the registry version" — that is exactly what `[patch.crates-io]` is designed for.

**Family-completeness note.** anyhow + thiserror pass on beta.6 because no path-dep / workspace member in their graphs references them by registry-name. (anyhow has no workspace members at all; thiserror's `thiserror-impl` depends on its sibling by path, not by name.) Both cxx and serde-json have at least one in-graph entity that depends on the package-under-test BY NAME — that's the trigger.

**Pilot-citation language note.** The cxx / serde-json / anyhow / thiserror pilot-manifest shapes above are context that motivates the strategy choice. They are NOT the proof of correctness for this plan. The synthetic-repro cargo-build-gated tests in §5.2 are the authoritative correctness signal: they exactly mirror the failure shape (root-package + path-dep edge + version requirements + `links` + `build.rs`) and must FAIL pre-fix / PASS post-fix.

---

## 2. Strategy choice + rationale

**Choose Strategy 1 (REVISED): self-`[patch.crates-io]` injection pointing at the STAGED OVERLAY DIR.**

For every overlay produced, inject (or merge into the upstream's existing) `[patch.crates-io.<overlay-crate-name>] = { path = "<absolutized staged-overlay-dir>" }` where:

- `<overlay-crate-name>` is the upstream's `[package].name` (already captured into `OverlayPlan.upstream_crate_name` per overlay.rs:289 / read by `read_upstream_crate_name` overlay.rs:560-567).
- `<absolutized staged-overlay-dir>` is `<upstream>/target/lihaaf-overlay/` (computed exactly the way `sibling_path` is computed at overlay.rs:519-525). This is the dir cargo writes the overlay manifest into.

### 2.1 Why staged-overlay-dir, not upstream-dir (R1 → R2 BLOCK-1 fix)

The R1 plan injected `[patch.crates-io.<X>] = { path = "<upstream>" }`. Codex R1 BLOCK-1 correctly identified this is a no-op:

- Cargo resolves source identity by the absolutized path. The upstream's `[package].name = "cxx"` lives at `path+file://<upstream>`. The registry-side `cxx` aliases to crates.io (`registry+https://github.com/rust-lang/crates.io-index`). `[patch.crates-io.cxx] = { path = "<upstream>" }` tells cargo "wherever you'd resolve cxx from crates.io, use the path at `<upstream>` instead" — but that path is *already* what cargo considers the canonical upstream source. The patch is a self-loop pointing back at the same source-id.
- Worse, depending on cargo's exact version, this can produce a confusing "patch points at the source it's patching" diagnostic, OR silently no-op (cargo's `[patch]` machinery validates the patched-source ≠ patch-target invariant in some versions).

The CORRECT target is the staged overlay dir. The overlay manifest lives at `<upstream>/target/lihaaf-overlay/Cargo.toml` (overlay.rs:519-525) and declares `[package].name = "<X>"` with the upstream's version. From cargo's POV, the staged-overlay package is at source-id `path+file://<upstream>/target/lihaaf-overlay` — DIFFERENT from upstream's `path+file://<upstream>`. The patch redirects "registry cxx → staged-overlay cxx" which IS a real redirect, not a self-loop.

The staged dir does NOT exist on disk at the moment `inject_self_patch_crates_io` runs (the overlay is serialized first, then written by `write_file_atomic` at overlay.rs:542-543). This is fine: `[patch.crates-io.<X>] = { path = "..." }` does not require filesystem existence at serialize time; cargo resolves the patch only when `cargo rustc --manifest-path <staged>` runs, and by then `write_file_atomic` has created the parent dir as part of staging the manifest itself.

### 2.2 Why not Strategy 2 (workspace-member stripping)

Removes the symptom but breaks the upstream-declared graph in ways the user did not consent to. Pilot forks may have workspace members the overlay is not the test target of — those members' own dependencies and dev-dependencies form the cargo-test baseline the compat report compares against. Stripping them silently shrinks the baseline and produces a false-clean compat verdict. Also requires re-parsing every member's manifest, expanding the I/O surface.

### 2.3 Why not Strategy 3 (`-p <crate>` + filtered manifest synthesis)

Equivalent to strategy 2 in invasiveness PLUS defeats PR #37 R2's inheritance preservation: a manifest synthesized to exclude workspace members cannot resolve `{ workspace = true }` references. Two-step regression.

### 2.4 Why not Strategy 4 (`--frozen` + lockfile pre-population)

Does not address the `links` collision (which fires at the resolved-package level, independent of the lockfile's version pin). And `--frozen` would require us to materialize a Cargo.lock — adding a second on-disk artifact beyond the overlay, with its own determinism contract.

### 2.5 Why not Strategy 5 (1 + 2 combined) — re-evaluation under corrected Strategy 1

R1 rejected Strategy 5 on the premise that Strategy 1 alone works. Since R1's Strategy 1 was broken (BLOCK-1 above), that rejection rested on a false premise.

Under the corrected R4 Strategy 1 (Option H 4-rule policy: Rule 1 INJECT for clean upstreams, Rule 2 REMAP for upstream-root self-patches, Rule 3 CONTINUE-ABSOLUTIZE for sibling patches, Rule 4 REJECT for vendored / git / non-root targets), does Strategy 1 alone still resolve BOTH failure shapes?

- **cxx (links collision) — Rule 2 REMAP branch.** cxx's upstream Cargo.toml already carries `[patch.crates-io.cxx] = { path = "." }`. R4 detects: `upstream_dir.join(".")` lexical-normalizes to `upstream_dir` = upstream root → Rule 2 fires. R4 emits `[patch.crates-io.cxx] = { path = "<absolutized staged-overlay-dir>" }` in the overlay. Cargo's resolver: `cxx-test-suite`'s `cxx = "1.0"` registry-name reference is redirected via `[patch.crates-io.cxx]` to `path+file://<staged-overlay-dir>` source-id; the root `[package].name = "cxx"` (overlay's own package) is also at `path+file://<staged-overlay-dir>` source-id. The two references resolve to the SAME source-id → cargo collapses them to one Package in the resolved graph → `links = "cxxbridge1"` collision cannot fire. ✓ Proven by the new §5.2.6 cargo-build-gated test (replaces R3's misleading citation of `cargo_accepts_rich_overlay_for_dylib_build`).
- **serde-json (ambiguous specification) — Rule 1 INJECT branch.** serde-json's upstream has no pre-existing `[patch.crates-io.serde_json]`. R4 Rule 1 INJECTS `[patch.crates-io.serde_json] = { path = "<staged-overlay-dir>" }`. The patch redirects "registry serde_json" → "staged-overlay serde_json". The transitive reference from `serde_json-test-suite` to `serde_json = "1.0"` is now redirected to the staged overlay. The resolved graph contains exactly ONE source for serde_json after patch application. The "two candidate sources" ambiguity is gone. ✓ Proven by the new §5.2.9 dedicated cargo-build-gated SEC-8 closure test (`cargo_accepts_root_to_test_suite_to_root_topology`).

Strategy 1 (R4 Option H 4-rule policy) is sufficient for both failure shapes — neither shape requires Strategy 2 / 3 / 4. Strategy 5 remains rejected because Strategy 2's workspace-member stripping adds invasiveness without solving anything Strategy 1 doesn't already solve, AND defeats PR #37 R2's inheritance preservation. The §5.2 cargo-build-gated synthetic repros (including the new §5.2.6 Rule 2 REMAP proof, §5.2.7 Rule 3 CONTINUE-ABSOLUTIZE proof, §5.2.8 Rule 4 REJECT proof, and §5.2.9 dedicated cargo-graph SEC-8 proof) are the authoritative verification of "Strategy 1 (R4 Option H) suffices."

**Strategy 1 (R4 Option H, intent-aware self-patch handling) is sufficient and minimal.** §2.6 covers the cargo anchoring detail that motivated the R3 → R4 shift; §3 documents the new 4-rule decision tree.

### 2.6 Cargo `[patch.crates-io].path` anchoring (R4 — Codex R3 SEC-7 closure)

The R4 shift from PRESERVE-PER-KEY to Option H (intent-aware REMAP) rests on a single cargo behavior the R3 plan got subtly wrong:

> **`[patch.crates-io.<X>].path` is resolved relative to the manifest THAT DECLARES THE PATCH, NOT relative to the manifest the path came from.**

Citations:
- Cargo book § "The [patch] section" (https://doc.rust-lang.org/cargo/reference/overriding-dependencies.html) — `[patch]` entries accept the same fields as `[dependencies]`, and dependency-like `path` fields are resolved relative to the manifest containing the declaration, exactly as `[dependencies].path` values are (see Cargo book § "Specifying path dependencies": path deps are "resolved relative to the manifest that contains the `path`").
- Cargo book § "Specifying path dependencies" (https://doc.rust-lang.org/cargo/reference/specifying-dependencies.html#specifying-path-dependencies) — path-dep anchoring rule: "Cargo will look for a `Cargo.toml` file in the directory at `<path>`, where `<path>` is relative to the manifest containing the `path` key."
- Cargo source: `cargo/util/toml/mod.rs` patch-table normalization uses `manifest_ctx.file.parent().join(path)` to anchor the path value against the directory of the manifest being read. When lihaaf writes the overlay manifest at `<staged-overlay-dir>/Cargo.toml`, cargo anchors any relative `[patch]` path value against `<staged-overlay-dir>/`, NOT against `<upstream-dir>/`.

**Concrete walk-through (cxx case).**

Upstream `<cxx-upstream>/Cargo.toml` declares:
```toml
[patch.crates-io]
cxx = { path = "." }
```

When cargo reads the upstream manifest directly, `.` is anchored at `<cxx-upstream>/` (the dir containing the manifest declaring the patch) → resolves to `<cxx-upstream>/.` = `<cxx-upstream>`. This is the upstream root crate, which IS the package cargo is resolving — cargo treats this as a self-patch where the patch redirect points at the same path-source as the root `[package]`. Cargo resolves both references (registry `cxx = "1.0"` and root `[package].name = "cxx"`) to the same source-id and emits one `cxx` package in the resolved graph. ✓ This is why cxx builds in isolation.

When lihaaf materializes the overlay, R3's PRESERVE-AS-IS branch copies the upstream's `[patch.crates-io.cxx] = { path = "." }` verbatim into `<cxx-upstream>/target/lihaaf-overlay/Cargo.toml`. Cargo now reads `.` from the STAGED OVERLAY manifest. The `.` is re-anchored at the staged overlay dir → resolves to `<cxx-upstream>/target/lihaaf-overlay/.` = `<cxx-upstream>/target/lihaaf-overlay`. This is the SAME source-id as the staged-overlay `[package]` itself (which is also at `<cxx-upstream>/target/lihaaf-overlay`). Cargo treats this as a self-patch from the overlay's perspective. The resolved graph contains exactly one `cxx` source — the staged-overlay path-source. ✓ The `links = "cxxbridge1"` collision cannot fire.

**R3's reasoning bug.** R3 §3 / §6.1 / §5.1.10 claimed the upstream's `path = "."` gets absolutized to `<upstream>/.` by `absolutize_patch_paths` (overlay.rs:1383-1410). That absolutization IS what `absolutize_patch_paths` does for path-bearing keys — it rewrites relative-path values to absolute form so the path doesn't move when the manifest moves. Under this scheme, the overlay's `[patch.crates-io.cxx].path` would become `<upstream>/.` (absolute), pointing at the upstream root crate. But this gives the WRONG source-id for the overlay's self-patch: the overlay's `[package]` is at `<staged-overlay-dir>`, and the patch would point at `<upstream-dir>`. The patch becomes a redirect from "registry cxx" → "upstream path cxx", which collides with the overlay's `[package]` source-id (= staged-overlay path cxx). The two distinct path-sources reintroduce the `links` collision that beta.6 exhibited.

**R3 partially escaped this by NOT actually running `absolutize_patch_paths` on the cxx self-patch entry — instead R3's PRESERVE-AS-IS branch copied the upstream's `path = "."` literally. Cargo then re-anchored `.` at the staged-overlay dir at READ time, giving the correct staged-overlay-path source-id. The accident works for `path = "."` specifically because:**

1. `.` is a relative path that re-anchors when the manifest moves.
2. The staged-overlay dir IS the package cargo is building (via `--manifest-path <staged>`), so re-anchoring `.` to the staged-overlay dir gives the same source-id as the overlay's `[package]`.
3. Cargo collapses self-referential patches to a single source in the resolved graph.

**The accident breaks for the general case.** Consider a hypothetical adopter whose upstream `Cargo.toml` carries:
```toml
[patch.crates-io]
my-crate = { path = "../my-fork" }
```

The upstream's intent is "redirect crates.io my-crate to a local fork at `<upstream>/../my-fork`". When cargo reads the upstream manifest directly, `..` is anchored at `<upstream>/` → resolves to `<upstream>/..`. That's the upstream's parent dir, where `my-fork` lives. ✓

When R3's PRESERVE-AS-IS branch copies `path = "../my-fork"` verbatim into the staged overlay manifest, cargo re-anchors `..` at the staged-overlay dir → resolves to `<upstream>/target/lihaaf-overlay/../my-fork` = `<upstream>/target/my-fork`. That dir almost certainly does NOT exist, and even if it did it's NOT the dir the adopter intended. The patch fails to resolve correctly. ✗

**Option H's intent-aware REMAP fixes this.** Rule 2 detects "upstream's existing patch resolves to the upstream root crate" and emits a path that resolves to the staged-overlay root crate (the equivalent intent). For cxx (`path = "."` resolves to upstream root): emit `path = "."` literally (relies on cargo's re-anchoring) OR emit `path = "<absolutized staged-overlay-dir>"` for clarity. For the hypothetical adopter (`path = "../my-fork"` resolves to a sibling dir, NOT the upstream root): Rule 2 does NOT fire; Rule 4 fires (REJECT) because the upstream's patch intent does not match "redirect to the staged overlay" semantics. The escape hatch (`--compat-allow-patch-override`) covers Rule 4.

**Why R4 chooses REMAP over PRESERVE-AS-IS even when PRESERVE-AS-IS would work.** Two reasons:

1. **Future-proofing.** A future cargo behavior change in path re-anchoring (or in `absolutize_patch_paths`'s handling of `.` — the production skip-if-already-absolute check at overlay.rs:1393 together with the absolutized-form emission at overlay.rs:1402 deliberately preserve relative-path semantics only for non-absolute entries, but that policy could change) would silently break PRESERVE-AS-IS without breaking REMAP. The REMAP form (`path = "<absolutized staged-overlay-dir>"`) is robust to any cargo / absolutization policy change.
2. **Determinism clarity.** PRESERVE-AS-IS produces overlay bytes that read `path = "."`, which depends on cargo's manifest-relative re-anchoring to mean anything. REMAP produces overlay bytes that read `path = "/abs/path/to/staged-overlay"`, which is unambiguous. The corpus golden tests are more readable with REMAP.

**Implementer choice for REMAP emission form:** literal `.` (relies on cargo re-anchoring; matches upstream byte shape) OR absolutized staged-overlay-dir (unambiguous, robust). R4 §3 specifies the latter for clarity; if implementer review surfaces a strong reason to prefer the former, escalate during plan review.

---

## 3. Target behavior

After the fix lands, the overlay materializer applies **Option H: intent-aware self-patch handling** to `[patch.crates-io.<self>]` (where `<self>` is the upstream's `[package].name`). The four rules below cover every observed and reasonably-expected case in the compat-mode pilot corpus.

### 3.1 The four-rule decision tree (Option H)

The rules are mutually exclusive and exhaustive. R4 evaluates them in order; the first matching rule fires.

**Rule 1 (INJECT) — no upstream root-crate patch exists.**

*Detection.* `top["patch"]["crates-io"][<self>]` is ABSENT (the upstream's `Cargo.toml` does not contain `[patch.crates-io.<self>]`).

*Action.* INJECT `[patch.crates-io.<self>] = { path = "<absolutized staged-overlay-dir>" }`. The path is the forward-slash absolute form of `<upstream>/target/lihaaf-overlay/`.

*Rationale.* The upstream has not declared any self-patch override. lihaaf adds a fresh self-patch redirecting the registry-name reference to the staged-overlay-dir source. The patch is a no-op when no transitive registry-version path exists (anyhow / thiserror) and load-bearing when one does (serde-json).

*Pilots in this rule:* anyhow, thiserror, serde-json, clean Round-2 candidates (likely most of derive_more / axum-macros until proven otherwise).

**Rule 2 (REMAP) — upstream's patch resolves to upstream root crate.**

*Detection.* `top["patch"]["crates-io"][<self>]` is PRESENT with a `path` key whose resolved target (anchored at the upstream manifest dir) IS the upstream root crate.

The "resolved target IS the upstream root crate" check uses lexical normalization (§4.1.1): join the upstream's manifest dir with the `.path` value, lexically normalize, compare to the lexically-normalized upstream manifest dir. Match → Rule 2 fires. The cxx case `path = "."` joined with `<cxx-upstream>` gives `<cxx-upstream>/.` which lexically-normalizes to `<cxx-upstream>` = upstream manifest dir. ✓

*Action.* REMAP the path. Emit `[patch.crates-io.<self>] = { path = "<absolutized staged-overlay-dir>" }` (overwriting the upstream's entry in the overlay output, NOT removing it). The emitted form is the absolutized staged-overlay-dir for byte-clarity (per §2.6 implementer-choice resolution). The semantic intent is preserved: the upstream's "patch crates-io.<self> with the upstream root crate" becomes "patch crates-io.<self> with the staged-overlay root crate" — same intent, applied to the overlay's manifest context.

*Rationale.* The upstream's intent is to self-patch the crate to a path-source equal to its own root. Translated to the staged-overlay context, the equivalent intent is to self-patch to a path-source equal to the staged-overlay root. Both forms produce a one-source resolved graph (no `links` collision, no `ambiguous specification` ambiguity); R4 chooses the staged-overlay-rooted form for determinism and future-proofing per §2.6.

*Pilots in this rule:* cxx, hypothetical Round-2 candidates with the same `[patch.crates-io.<self>] = { path = "." }` shape.

**Rule 3 (CONTINUE-ABSOLUTIZE) — upstream has non-root path-bearing patch entries.**

*Detection.* `top["patch"]["crates-io"][<X>]` is PRESENT for SOME `<X>`, AND the existing `absolutize_patch_paths` pass (overlay.rs:1383-1410) was going to apply to that entry anyway (because the entry has a `.path` key). Rule 3 covers entries that are NOT the upstream root-crate self-patch — e.g., `[patch.crates-io.cxx-build] = { path = "gen/build" }` (a sibling-crate patch in the cxx upstream's `Cargo.toml`).

*Action.* CONTINUE-ABSOLUTIZE — let the existing `absolutize_patch_paths` pass handle the entry, absolutizing the relative path against the upstream dir. R4 does NOT special-case these entries; they are orthogonal to the root-crate self-patch and preserve their existing behavior.

*Rationale.* These are not self-patches against the upstream root; they are adjacent patches against sibling crates. The R3 absolutization scheme (anchor against upstream, emit absolute form) is correct for these — they're not the "redirect registry-name to overlay path-source" intent that Rule 2 handles. R4 is intentionally narrow: it only changes behavior for the `<self>` key.

*Pilots in this rule:* cxx (which has `[patch.crates-io.cxx-build] = { path = "gen/build" }` in addition to the root `cxx = { path = "." }`; R4 Rule 2 handles `cxx`, Rule 3 handles `cxx-build`).

**Rule 4 (REJECT) — upstream's patch targets an external source (vendored fork / git / non-root path).**

*Detection.* `top["patch"]["crates-io"][<self>]` is PRESENT, AND either:
- (a) the entry has a `.path` key but its resolved target is NOT the upstream root crate (e.g., `path = "../my-fork"` resolves to a sibling dir); OR
- (b) the entry has `git`/`branch`/`tag`/`rev` keys but no `.path` (e.g., the adopter pulls a fork from git as a registry-name override); OR
- (c) the entry has BOTH `.path` AND `git`/etc (rare pathology).

*Action.* REJECT with a clear error. The compat-mode materializer returns `Err(_)` and surfaces a structured envelope error containing:
- The crate name (`<self>`).
- The upstream's existing entry (what was rejected).
- The expected resolution (lihaaf would inject `[patch.crates-io.<self>] = { path = "<staged-overlay-dir>" }` but cannot, because the upstream's entry already declares a non-compatible intent).
- A pointer to the v0.2/v1.1 follow-up issue (filed at implementation time): "to opt into overwriting the upstream's existing patch, use `--compat-allow-patch-override` (v0.2/v1.1, not yet available)."

*Rationale.* The adopter has explicitly overridden the registry-name with a non-root, non-staged-overlay source. lihaaf v0.1.0 / v1.0.0 must NOT silently overwrite this — it would mask the adopter's intent. The conservative path is REJECT-with-clear-error. The escape hatch is deferred to v0.2/v1.1 per §7.1.

*Pilots in this rule:* none known in the current corpus. This rule is defensive coverage for unanticipated adopter manifests. Future Round-2 / Round-3 pilots may surface a real Rule 4 case; the file-an-issue path is documented.

### 3.2 Per-pilot mapping under Option H

R5 distinguishes four build-script classes relevant to the staged-mirror strategy (§4.5):

| Class | Description | Build-script access pattern | Mirror impact |
|---|---|---|---|
| **Class A** | `build.rs` present AND reads package-root files | Hard-error (cxx) or silent-false (anyhow, thiserror) without mirror | REQUIRED: mirror provides the accessed files via symlinks |
| **Class B** | `build.rs` present, env-only, no package-root file read | serde_json build probe uses env vars only | Mirror is benign; no file access to redirect |
| **Class C** | `build.rs` present at workspace root but no package-root file read | derive_more root build.rs | Mirror is benign; no file access to redirect |
| **Class D** | `build = false` declared | axum-macros | Mirror is benign; no build.rs runs |

Per-pilot detail:

- **cxx (#47) — Class A, hard-error variant.** cxx's upstream carries `[patch.crates-io.cxx] = { path = "." }`. Rule 2 (REMAP) fires. R5 emits `[patch.crates-io.cxx] = { path = "<absolutized staged-overlay-dir>" }` in the overlay. cargo resolves both `cxx-test-suite`'s `cxx = "1.0"` registry-name reference (via the patch) and the root `cxx`'s `[package]` to the same staged-overlay source-id. The resolved graph contains exactly one `cxx`; `links = "cxxbridge1"` collision cannot fire. ✓ (Adjacent `[patch.crates-io.cxx-build] = { path = "gen/build" }` falls under Rule 3 and is absolutized as before.)

  **Build-script file access (M.2-M.3 closure).** `cxx build.rs:143-148` reads `src/cxx.cc` via `manifest_dir.join("src/cxx.cc")` (where `manifest_dir` = `CARGO_MANIFEST_DIR` = staged-overlay-dir). `cxx build.rs:154-159` references `include/cxx.h`. Without the staged-mirror strategy, the staged overlay dir is empty and both file reads FAIL (hard error during `cargo build`). With the staged-mirror strategy (§4.5), `src/` and `include/` are symlinked into the staged overlay dir → both accesses resolve to the real upstream files. ✓ Covered by §5.2.6 cargo-build test (upgraded in R5 to exercise this access pattern explicitly — M.4 closure).

- **serde-json (#40) — Class B (env-only build.rs).** serde-json's upstream has no `[patch.crates-io.serde_json]`. Rule 1 (INJECT) fires. R5 emits `[patch.crates-io.serde_json] = { path = "<absolutized staged-overlay-dir>" }`. The patch redirects registry-name `serde_json` to the staged-overlay source-id. The resolved graph contains exactly one `serde_json`; `ambiguous specification` cannot fire. ✓

  **Build-script access (verified non-driver).** serde_json has a `build.rs` that probes features via env vars (e.g. `CARGO_CFG_TARGET_*`). It does NOT read any package-root-relative file. The staged-mirror strategy is benign for serde_json — the overlay dir need not have any serde_json-specific files for the build script to succeed. Class B confirmed.

- **anyhow — Class A, silent-false variant (M.5 closure).** Rule 1 (INJECT). No transitive registry-version path → patch is benign / no-op. ✓

  **DANGER — silent-false probe pattern.** `anyhow build.rs:255-257` and `:323-367` compile `Path::new("src").join("nightly.rs")` from the current working directory (= `CARGO_MANIFEST_DIR` = staged-overlay-dir) to probe for nightly-only features. Without the staged-mirror strategy, `src/nightly.rs` does NOT exist in the staged overlay dir. The probe does NOT error — it returns `false` (compilation fails silently), which DISABLES nightly cfg flags. This is a silent misconfiguration: the overlay's compat report may use wrong cfg flags relative to the upstream build, producing a false-clean verdict. With the staged-mirror strategy, `src/` is symlinked into the staged overlay dir → `src/nightly.rs` is accessible → probe returns the correct result. Covered by new §5.2 test `cargo_build_anyhow_shape_probe_file_resolves_via_mirror` (M.5 closure).

- **thiserror — Class A, silent-false variant (M.6 closure).** Rule 1 (INJECT). `thiserror-impl` member depends on `thiserror` only by path, not registry-name → patch is benign / no-op. ✓

  **DANGER — silent-false probe pattern.** `thiserror build.rs:261-263` and `:328-371` compile `Path::new("build").join("probe.rs")` from cwd (= staged-overlay-dir) to probe for nightly and tool-attributes features. Without the staged-mirror strategy, `build/probe.rs` does NOT exist in the staged overlay dir. Same silent-false failure mode as anyhow: the probe returns `false`, disabling cfg flags that should be enabled. With the staged-mirror strategy, `build/` is symlinked into the staged overlay dir → `build/probe.rs` is accessible → probe returns the correct result. §5.2 test coverage: shares the silent-false class with M.5; the `cargo_build_anyhow_shape_probe_file_resolves_via_mirror` test demonstrates the class. A separate thiserror-shape test (`cargo_build_thiserror_shape_probe_file_resolves_via_mirror`) is listed in §5.2 to pin the `build/probe.rs` path form specifically (distinct from anyhow's `src/nightly.rs` path form). If implementer deems redundant, they may collapse the two into one parameterized test — surfaced as an open item in §13.

- **derive_more (Round-2 pilot) — Class C (build.rs no package-root read).** Evaluated per Option H decision tree. Most likely Rule 1 (clean upstream); if upstream self-patches its own crate, Rule 2 (REMAP) fires. Round-2 enrollment will surface the actual rule routing.

  **Build-script access (verified non-driver).** derive_more root has a `build.rs` (confirmed by Round-2 fork shape analysis per [[lihaaf-round2-fork-shape-analysis]]). The build.rs does NOT read any package-root-relative file (env-var or codegen only). The staged-mirror strategy is benign — no file access to redirect. Class C confirmed.

- **axum-macros (Round-2 pilot) — Class D (build = false).** Evaluated per Option H decision tree. Most likely Rule 1 (clean upstream). Round-2 enrollment will surface the actual rule routing.

  **Build-script access (verified non-driver).** axum-macros declares `build = false` in its manifest — no build script runs. The staged-mirror strategy is benign. Class D confirmed.

- **Non-workspace single-crate pilots:** Rule 1 (INJECT). Benign patch when no transitive registry-version path exists. Mirror is still applied for completeness (any future pilot with a build.rs that reads package-root files will benefit without per-pilot special-casing).

- **Adopter with vendored fork (hypothetical Rule 4):** REJECTED with structured error pointing to v0.2/v1.1 escape hatch follow-up.

**Negative criterion (explicit, R5 update):** no compat behavior change for any pilot that is passing on beta.6 (anyhow, thiserror). The existing `cargo_accepts_rich_overlay_for_dylib_build` test (which exercises `rich-demo = { path = "." }`) is NOT load-bearing for the SEC-8 cycle-acceptance proof under R5 (R3 incorrectly relied on it; R4/R5 replace it with the new `cargo_accepts_root_to_test_suite_to_root_topology` test at §5.2.9). The existing test is expected to PASS under R5 — but R5's Rule 2 REMAP changes the `rich-demo`-shape overlay's `[patch.crates-io.rich-demo].path` value from the R3 preserved `.` to the R4/R5 REMAPPED absolutized staged-overlay-dir. The test assertion (cargo build success) is preserved, but the corpus expected file for the `with_patch_section` fixture must be updated to reflect the REMAP — captured in §5.2 fixture updates.

**Toolchain criterion (explicit, unchanged from R3):** the §3.2.3 byte-determinism guarantee holds across the fix. The 4-rule policy is fully deterministic: the same upstream manifest produces the same overlay bytes on every run.

---

## 4. File-level changes

**Primary change site: `src/compat/overlay.rs`.**

### 4.1 New function: `apply_self_patch_policy` (R4 — renamed from R3's `inject_self_patch_crates_io`)

Location: insert immediately after `absolutize_patch_paths` (current line ~1383), before `absolutize_replace_paths` (current line ~1429). The new function and `absolutize_patch_paths` form a natural pair.

Signature (R4):

```rust
fn apply_self_patch_policy(
    top: &mut toml::map::Map<String, toml::Value>,
    upstream_crate_name: Option<&str>,
    upstream_dir: &Path,
    staged_overlay_dir: &Path,
) -> Result<(), Error>;
```

Caller passes `staged_overlay_dir = <upstream>/target/lihaaf-overlay/` (computed the same way as `sibling_path` at overlay.rs:519-525) AND `upstream_dir = <upstream>` (for Rule 2 resolved-target detection). The function returns `Result` because Rule 4 rejects on incompatible upstream patches.

R4 renames from R3's `inject_self_patch_crates_io` to `apply_self_patch_policy` to reflect that the function does more than inject — it also remaps (Rule 2) and rejects (Rule 4). Implementer may keep R3's name if review prefers; the rename is descriptive-only. R4 also adds the `upstream_dir` parameter (R3 took only `staged_overlay_dir`); Rule 2 detection needs the upstream root path to join against the patch's `.path` value.

Behavior (R4 — Option H 4-rule decision tree):

1. If `upstream_crate_name` is `None`, return `Ok(())` immediately. The workspace-root manifest case is already rejected by `is_workspace_root_manifest` at overlay.rs:429; this is defense-in-depth for partial/malformed manifests.
2. Compute the absolutized staged-overlay path string: `let staged_overlay_abs = crate::util::to_forward_slash(&staged_overlay_dir.to_string_lossy())`. This matches the absolutization shape used by every other path-bearing key (overlay.rs:1402-1403).
3. Ensure `top["patch"]` exists as a table. If absent, create empty.
4. Ensure `top["patch"]["crates-io"]` exists as a table. If absent, create empty.
5. **Option H 4-rule policy** on `top["patch"]["crates-io"][<upstream_crate_name>]`:

   **Rule 1 (INJECT) — entry ABSENT.**

   If `top["patch"]["crates-io"][<upstream_crate_name>]` does not exist, insert `{ path = staged_overlay_abs }`. Return `Ok(())`. Anyhow / thiserror / serde-json / clean Round-2 pilots take this path.

   **Rule 2 (REMAP) — entry PRESENT with `.path`, resolved target IS upstream root.**

   If the entry has a `path` key AND no `git`/`branch`/`tag`/`rev` keys, evaluate whether the path resolves to the upstream root crate:

   ```rust
   let entry_path_raw: &str = entry.get("path")?.as_str()?;
   let joined = upstream_dir.join(entry_path_raw);
   let joined_normalized = lexical_path_normalize_path(&joined);
   let upstream_normalized = lexical_path_normalize_path(upstream_dir);
   if joined_normalized == upstream_normalized {
       // Rule 2: REMAP to staged-overlay path.
       // Per §6.1 line 1111 ("Overwrite the entry: emit `{ path = ... }`"), REPLACE the
       // entire entry with a clean `{ path = ... }`, not just insert/upsert the path key.
       // This is defensive: Rule 2's entry condition guarantees no git/branch/tag/rev keys
       // today, but if a future cargo version adds a new patch key, we want clean overlay
       // output that matches the §6.1 normative spec (path-only, no leftover keys).
       entry.clear();
       entry.insert("path".to_string(), toml::Value::String(staged_overlay_abs.clone()));
       return Ok(());
   }
   // Falls through to Rule 4 because path resolves elsewhere (vendored fork case).
   ```

   The `lexical_path_normalize_path` helper (§4.1.1) joins, removes `Component::CurDir`, and preserves all other components. NB: this is a LEXICAL check; it does not call `canonicalize()` (symlink limitation per §6.11).

   cxx case: `path = "."`, `upstream_dir.join(".") = <upstream>/.`, lexical-normalize = `[<upstream-components>]` = `upstream_dir` normalized. Match → Rule 2 fires → emit `path = staged_overlay_abs`. The cxx upstream's `[patch.crates-io.cxx-build] = { path = "gen/build" }` entry is NOT keyed against `<upstream_crate_name>` (key is `cxx-build`, name is `cxx`), so Rule 2 evaluation does not touch it; it falls under Rule 3.

   **Rule 3 (CONTINUE-ABSOLUTIZE) — non-root path-bearing entries.**

   Rule 3 is NOT enforced by this function explicitly. It is the no-op fallthrough: any entry in `[patch.crates-io.<X>]` where `<X> ≠ <upstream_crate_name>` is left untouched by this function. The pre-existing `absolutize_patch_paths` pass (overlay.rs:1383-1410) already runs BEFORE this function (run `absolutize_patch_paths` BEFORE `apply_self_patch_policy` so Rule 3 entries are already normalized); that pass absolutizes the entry's path against `upstream_dir` as before. No additional work in `apply_self_patch_policy`.

   The reason this rule exists explicitly in the plan (even though it's a no-op in the function) is so the test surface §5 covers the "non-root patch entry is preserved-and-absolutized" assertion against possible future refactors that move absolutization logic inside `apply_self_patch_policy`.

   **Rule 4 (REJECT) — entry PRESENT but target is external.**

   If the entry has `git`/`branch`/`tag`/`rev` keys (regardless of whether `.path` is also present), OR if the entry has only `.path` but the path resolves to a directory OTHER than the upstream root crate (e.g., `path = "../my-fork"`), reject:

   ```rust
   return Err(Error::CompatPatchOverrideConflict {
       crate_name: upstream_crate_name.to_string(),
       upstream_entry: format!("{:?}", entry),
       expected_resolution: "lihaaf would inject [patch.crates-io.<self>] = \
           { path = \"<staged-overlay-dir>\" } but cannot, because the upstream's \
           entry declares a non-compatible intent. To opt into overwriting, use \
           --compat-allow-patch-override (v0.2/v1.1, not yet available; see issue #X)."
           .to_string(),
   });
   ```

   The error is structured (per the envelope contract); the implementer files a v0.2/v1.1 follow-up issue and references it in the error text at implementation time. `Error::CompatPatchOverrideConflict` is a new variant in the compat module's error type (additive); the implementer adds it.

6. Run AFTER `absolutize_patch_paths` so the upstream's pre-existing entries (Rule 3 entries — non-root patches) have been absolutized before this function runs. Our Rule 1 INJECT and Rule 2 REMAP both emit an already-absolute path, so a second pass through `absolutize_patch_paths` would be a no-op. **Implementer note:** the ordering "absolutize first, then policy" means Rule 2's resolved-target check runs against an upstream entry whose `.path` may or may not have been already absolutized by the `absolutize_patch_paths` pass immediately prior. R5 specifies: Rule 2's detection re-joins the entry's `.path` value against `upstream_dir` regardless of whether the value is absolute (`upstream_dir.join("/abs/path")` returns `/abs/path` on Unix, ignoring the prefix). The lexical normalization handles both pre-absolutized and post-absolutized forms.

   **Alternative ordering — run BEFORE `absolutize_patch_paths`.** A simpler ordering would be to run Rule 2's detection on the raw upstream value before absolutization. The implementer may prefer this; either ordering is correct as long as the test surface §5 pins the behavior. R4 documents the AFTER-absolutize ordering as primary because that's what R3 used; reordering is implementer's call during PR-1 review.

#### 4.1.1 Lexical path normalization (R2 BLOCK-2 / R3 BLOCK-2 finish; R4 — Rule 2 detection)

Lexical normalization is used by Rule 2 (REMAP) to determine whether the upstream's `[patch.crates-io.<self>].path` resolves to the upstream root crate. Implementation:

```rust
fn lexical_path_normalize_path(p: &Path) -> Vec<std::path::Component<'_>> {
    p.components()
        .filter(|c| !matches!(c, std::path::Component::CurDir))
        .collect()
    // ParentDir components are PRESERVED (not collapsed) — `..` semantics depend on
    // filesystem state and cannot be collapsed without canonicalize(). For our
    // purposes any `..` in the patch target indicates the user is hand-routing,
    // and we want Rule 2 to NOT match those (they fall to Rule 4 REJECT instead).
}

fn paths_lexically_equal(a: &Path, b: &Path) -> bool {
    lexical_path_normalize_path(a) == lexical_path_normalize_path(b)
}
```

Rule: drop all `.` (`Component::CurDir`) components; preserve all other components (`Normal`, `ParentDir`, `RootDir`, `Prefix`). Compare resulting `Vec<Component>` for equality.

Test cases for the normalizer (pinned in §5):

- `Path::new("/work/cxx")` == `Path::new("/work/cxx/.")` — `.` filtered. Rule 2 detection: `<upstream>/.` joined and normalized equals `<upstream>` normalized → Rule 2 fires.
- `Path::new("/work/cxx/")` == `Path::new("/work/cxx")` — trailing slash handled by `Path::components()` parsing.
- `Path::new("/work/cxx/target/lihaaf-overlay")` == `Path::new("/work/cxx/target/lihaaf-overlay")` — basic identity.
- `Path::new("/work/cxx")` != `Path::new("/work/cxx/target/lihaaf-overlay")` — different normal components. Rule 2 detection: staged-overlay-dir is NOT the upstream root, but this is comparing the wrong values for Rule 2; the actual Rule 2 detection uses `upstream_dir.join(entry_path_raw)` not staged-overlay-dir.
- `Path::new("/work/cxx/..")` != `Path::new("/work/cxx")` — `..` preserved (defensive). Rule 2 detection: vendored-fork shape `path = ".."` joined with `<upstream>` gives `<upstream>/..` which lexical-normalizes to `[<upstream-components>, ParentDir]` ≠ `[<upstream-components>]` → Rule 2 does NOT fire → falls to Rule 4 REJECT. ✓
- `Path::new("/work//cxx")` == `Path::new("/work/cxx")` — **repeated separators (R3 BLOCK-2 finish)**: `Path::components()` collapses `//` to a single separator on Unix; cargo treats `path = "/foo//bar"` as equivalent to `/foo/bar` (cargo's path-source resolution canonicalizes via `Path::new(s).components()`). Test `lexical_path_normalize_handles_repeated_separators` (§5.1.12) pins this.
- `Path::new("/work/cxx")` and `Path::new("/work/symlink-to-cxx")` are **NOT** lexically equal even if the filesystem resolves the symlink to the same canonical path. R4 lexical normalization does NOT call `canonicalize()` / `read_link()` — only `.` and trailing-slash are normalized. **Known limitation (R3 BLOCK-2 finish, unchanged in R4)**: adopters with symlinked upstream paths trigger an edge case in Rule 2 detection — if the upstream's `[patch.crates-io.<self>] = { path = "/symlink/to/upstream" }` and the operator passed `--compat-root /real/upstream`, Rule 2's joined-and-normalized comparison sees them as different lexical paths and Rule 2 does NOT fire → falls to Rule 4 REJECT. In practice this is rare (operator-controlled `--compat-root` and adopter-controlled upstream patch usually use the same form). Test `lexical_path_normalize_does_not_resolve_symlinks` (§5.1.13) pins this as a known limitation. §6.11 documents the adopter workaround.

This handles the cxx case under Rule 2: upstream's `[patch.crates-io.cxx].path = "."`, `upstream_dir.join(".") = <upstream>/.`, lexical-normalize = `[<upstream-components>]`. `upstream_dir` lexical-normalize = `[<upstream-components>]`. Lexically equal → Rule 2 fires → REMAP to staged-overlay-dir. ✓

This rejects the vendored-fork case under Rule 4: upstream's `[patch.crates-io.<self>].path = "../my-fork"`, `upstream_dir.join("../my-fork") = <upstream>/../my-fork`, lexical-normalize = `[<upstream-components>, ParentDir, Normal("my-fork")]`. `upstream_dir` lexical-normalize = `[<upstream-components>]`. NOT lexically equal → Rule 2 does NOT fire → Rule 4 REJECT. ✓

#### 4.1.2 Emission policy (R2 FIX-6, unchanged in R4)

Emission (the bytes written into the overlay) PRESERVES the absolutized form, identical to the existing absolutization scheme (production `absolutize_patch_paths` at overlay.rs:1393 + :1402). This is the §3.2.3 byte-stable guarantee: the overlay's `[patch.crates-io.<self>].path` is exactly `<staged-overlay-dir>` in `to_forward_slash` form, for both Rule 1 (INJECT) and Rule 2 (REMAP) — both emit the same byte shape.

R4 has no separate "comparison policy vs emission policy" distinction the way R3 did (R3 used lexical normalization to compare for idempotency / conflict-detect; R4 uses lexical normalization only inside Rule 2's detection logic, and the emission is deterministically absolutized).

Corpus determinism tests pin EXACT bytes for both Rule 1 and Rule 2 outputs — including the updated `with_patch_section.expected.toml` for the `rich-demo = { path = "." }` shape, which now reflects Rule 2 REMAP to the staged-overlay-dir absolutized form.

### 4.2 Call-site wiring in `materialize_overlay_inner`

Currently in overlay.rs:454-506, the path-bearing absolutization (`absolutize_path_bearing_keys`, which calls `absolutize_patch_paths` and `absolutize_replace_paths`) runs at line 479, followed by `inject_synthetic_metadata` at 482 and `override_workspace_inheritance` at 505.

The wire-up requires the staged-overlay-dir path. Currently `sibling_path` is computed AFTER the `[workspace]` rewrite (overlay.rs:519-525), too late to feed `inject_self_patch_crates_io` if we want to call it inside the `if let toml::Value::Table(top) = &mut value` block.

**Two acceptable shapes — implementer picks one in code review:**

Shape A (compute `staged_overlay_dir` early, reuse for `sibling_path`):

```rust
// Inside materialize_overlay_inner, before the `if let toml::Value::Table(top) = &mut value` block.
let staged_overlay_dir = upstream_dir.join("target").join("lihaaf-overlay");

// Inside the table-mutation block, after `absolutize_path_bearing_keys`:
apply_self_patch_policy(top, upstream_crate_name.as_deref(), &upstream_dir, &staged_overlay_dir)?;

// Later (currently overlay.rs:519-525), reuse:
let sibling_path = staged_overlay_dir.join("Cargo.toml");
```

Shape B (compute the staged-overlay-dir inline in `apply_self_patch_policy` from `upstream_dir`):

```rust
// Inside materialize_overlay_inner, after `absolutize_path_bearing_keys`:
apply_self_patch_policy(top, upstream_crate_name.as_deref(), &upstream_dir)?;

// apply_self_patch_policy internally computes:
let staged_overlay_dir = upstream_dir.join("target").join("lihaaf-overlay");
```

Either is acceptable; Shape A is preferred because it shares the construction with `sibling_path` (DRY, harder to drift). Shape A also makes the function signature explicit about its two path arguments, which is helpful for Rule 2 detection clarity. The new call goes BEFORE `inject_synthetic_metadata` (line 482), AFTER `absolutize_path_bearing_keys` (line 479). Reasons:

- After absolutization: the upstream's pre-existing `[patch.crates-io.<X>].path` entries have been rewritten to absolute form. (Rule 2 detection re-joins via `upstream_dir.join(entry_path_raw)` regardless of whether the value is pre-absolutized, so ordering here is: Run `apply_self_patch_policy` AFTER `absolutize_patch_paths` so Rule 3 entries are already normalized before the policy function evaluates.)
- Before `inject_synthetic_metadata`: the synthetic metadata injection touches `[package].metadata.lihaaf` only — orthogonal to `[patch]` — but maintaining a single "all overlay rewrites" block makes the function shape easier to reason about.
- Before `override_workspace_inheritance`: the workspace-inheritance override does not touch `[patch]`, so order is independent here.

The call-site returns `Result<(), Error>` and propagates via `?`. `materialize_overlay_inner` already returns `Result<OverlayPlan, Error>`. Rule 4 REJECT cases surface as the new `Error::CompatPatchOverrideConflict` variant; the error is structured (compat envelope contract) and the materializer fails fast before any further overlay work.

### 4.3 Doc updates

1. **Module-level docs (overlay.rs:1-234).** Extend "What the overlay does and does NOT touch in `[patch]`" (currently lines 60-72) to add a third bullet about self-patch policy and a fourth bullet about the staged-mirror strategy. Sample text (R5):

   > **NEW (issue #40/#47):** When the overlay carries a `[package].name`, the materializer applies a 4-rule **Option H intent-aware self-patch policy** to the upstream's `[patch.crates-io.<overlay-package-name>]`: (1) if absent, INJECT `{ path = "<absolutized staged-overlay-dir>" }` (anyhow / thiserror / serde-json / clean Round-2 pilots); (2) if present with `.path` whose resolved target IS the upstream root crate, REMAP to the staged-overlay-dir (cxx case, `path = "."`); (3) non-target-crate `[patch.crates-io.<X>]` entries are left untouched (the existing `absolutize_patch_paths` pass handles them); (4) if present but the target is external (vendored fork / git source / non-root path), REJECT with a structured error pointing to the v0.2/v1.1 `--compat-allow-patch-override` escape hatch. The CLEAN-case Rule 1 patch target choice (staged-overlay-dir, NOT upstream-dir) avoids the self-loop bug R1 had. The REMAP-case Rule 2 emission is deterministic and robust to future cargo / absolutization-policy changes (see §2.6 for the cargo-anchoring analysis that drove the R3 → R4 shift from PRESERVE-AS-IS to REMAP).

2. **Function-level docs on `apply_self_patch_policy`.** Explain the failure modes it resolves with concrete cargo error quotes (`links = "cxxbridge1"` collision; `specification serde_json is ambiguous`), the staged-overlay-dir-not-upstream design choice for Rule 1 (self-loop avoidance per §2.1), the Option H 4-rule policy (Rule 1 INJECT / Rule 2 REMAP / Rule 3 CONTINUE-ABSOLUTIZE / Rule 4 REJECT, per §3.1 + §6.1 decision-table), the §2.6 cargo-anchoring reasoning that drove the R3 → R4 shift, and the §3.2.3 byte-determinism contract preservation.

2a. **Function-level docs on `mirror_upstream_into_overlay`.** Explain the problem it solves (build scripts reading package-root files via `CARGO_MANIFEST_DIR` / cwd, which cargo sets to the staged-overlay dir; §4.5.1), the exclusion list (§4.5.4: `target/`, `.git/`, `Cargo.toml`, `Cargo.lock`), the symlink-first / copy-fallback strategy (§4.5.3), the idempotency / rerun-state reconciliation contract (§4.5.6: Option B skip-on-canonical / reconcile-by-replacement for all other states), and the known limitations (§4.5.8: one-shot mirror, `.cargo/` intentionally included). Cite the Class A pilot file-access patterns by name: `cxx build.rs:143-148` (`src/cxx.cc`), `cxx build.rs:154-159` (`include/cxx.h`), `anyhow build.rs:255-257,323-367` (`src/nightly.rs`), `thiserror build.rs:261-263,328-371` (`build/probe.rs`).

3. **§3.2.3 doc update.** `docs/compatibility-plan.md` §3.2.3 currently lists `[patch.<registry>.X].path` as one of the absolutized keys (line 175). Add a new bullet immediately after about the Option H self-patch policy AND a second new bullet about the staged-mirror strategy. Sample text (R5):

   > In addition, the overlay materializer applies an **Option H intent-aware self-patch policy** to the upstream's `[patch.crates-io.<overlay-package-name>]`: (1) if absent, an entry `{ path = "<absolutized staged-overlay-dir>" }` is injected (resolving the resolution-time ambiguity on clean upstreams like anyhow / thiserror / serde-json); (2) if present with `.path` whose resolved target IS the upstream root crate (cxx-style self-patch `path = "."`), the entry is REMAPPED to the staged-overlay-dir (preserving the upstream's "patch crates-io to root" intent in the overlay's context, where the equivalent root is the staged-overlay-dir); (3) non-target-crate `[patch.crates-io.<X>]` entries are absolutized as before (orthogonal scope, unchanged); (4) if present but the target is external (vendored fork / git source / non-root path), the materializer REJECTS with a structured error and recommends `--compat-allow-patch-override` (v0.2/v1.1, not yet available). The Rule 2 REMAP-over-PRESERVE-AS-IS choice is driven by cargo's manifest-relative anchoring of `[patch.crates-io.X].path` values — see `docs/compatibility-plan.md` appendix on cargo patch-path anchoring (or §2.6 of the R4 implementation plan) for the full reasoning.

---

## 4.5 Staged Package-Root Mirror (§4.5 — R5 new, M.1 closure)

### 4.5.1 Problem: empty staged-overlay dir breaks build-script file access

The staged overlay at `<upstream>/target/lihaaf-overlay/` is currently created by `write_file_atomic` writing only the generated `Cargo.toml`. The directory exists; its only content is that manifest file.

When cargo builds the overlay package (via `cargo rustc --manifest-path <staged-overlay>/Cargo.toml`), it sets `CARGO_MANIFEST_DIR` to the staged-overlay dir for every `build.rs` that runs. Build scripts in Class A pilots (§3.2) read files relative to `CARGO_MANIFEST_DIR` or the process cwd (same dir for build scripts):

- `cxx build.rs:143-148`: `manifest_dir.join("src/cxx.cc")` → reads `<staged-overlay>/src/cxx.cc` — **file does not exist → hard error**
- `cxx build.rs:154-159`: `manifest_dir.join("include/cxx.h")` → reads `<staged-overlay>/include/cxx.h` — **file does not exist → hard error**
- `anyhow build.rs:255-257,323-367`: `Path::new("src").join("nightly.rs")` (cwd-relative) → reads `<staged-overlay>/src/nightly.rs` — **file does not exist → silent-false, wrong cfg flags**
- `thiserror build.rs:261-263,328-371`: `Path::new("build").join("probe.rs")` (cwd-relative) → reads `<staged-overlay>/build/probe.rs` — **file does not exist → silent-false, wrong cfg flags**

Cargo has no manifest key to override `CARGO_MANIFEST_DIR`; the build script receives whatever dir the manifest lives in. The overlay manifest MUST live in `<staged-overlay>/`, so the only fix is to make the staged-overlay dir look like the upstream package root.

### 4.5.2 Chosen strategy: staged package-root mirror with symlinks and copy fallback

**Codex R4 recommendation (adopted in R5 without alternatives).** After the overlay `Cargo.toml` is written (by `write_file_atomic`), a second pass creates a mirror of the upstream package root in the staged-overlay dir. On rerun (when the staged-overlay dir already exists), the mirror step applies **Option B (Idempotent skip + reconcile-by-replacement)** per §4.5.6: skip only when the current state is the canonical symlink to the correct `<upstream>/E`; for all other cases, reconcile by replacing stale state with the canonical mirror.

```
# ── Per-entry forward pass ─────────────────────────────────────────────────
for each top-level entry E in <upstream>/:
    if E is "target" | ".git" | "Cargo.toml" | "Cargo.lock" → skip (§4.5.4)

    let staged   = <staged-overlay>/E
    let upstream = <upstream>/E

    if staged is absent:
        # CASE 1: create canonical mirror (first-run)
        create symlink staged → upstream   # or copy under fallback

    elif staged is symlink AND symlink_target(staged) == upstream:
        # CASE 2: idempotent skip — already the canonical state
        # Analogue of overlay.rs:527-531 bytes-match skip: skip the write
        # when the content already matches the desired state.
        continue

    elif staged is broken symlink:
        # CASE 4: upstream entry removed/renamed since last run
        unlink staged
        if upstream exists:
            create symlink staged → upstream
        # else: upstream entry is gone too; staged symlink already unlinked above — no further action needed

    elif staged is symlink with wrong target:
        # CASE 3: upstream moved, manual edit, or stale overlay
        unlink staged
        create symlink staged → upstream

    elif staged is real file AND upstream is file:
        # CASE 5
        # Symlink mode: replace with canonical symlink
        # Copy mode: byte-check (skip if match, replace if mismatch — mirrors
        #            the overlay.rs:530-531 bytes-match skip for files)
        remove staged
        create symlink staged → upstream   # or byte-check-copy under fallback

    elif staged is real dir AND upstream is dir:
        # CASE 6
        # Symlink mode: replace with canonical symlink
        # Copy mode: exact-sync (MUST remove destination-only files — no merge;
        #            removed-upstream files must not persist in staged)
        remove_tree staged
        create symlink staged → upstream   # or exact-sync-copy under fallback

    elif type(staged) != type(upstream):
        # CASE 7: type mismatch (file↔dir swap)
        remove staged or remove_tree staged (as appropriate)
        create canonical mirror (symlink or copy per fallback)
        # structured error if removal fails (e.g. permission-denied)

    else:
        # CASE 8: entry present but was never produced by the mirror
        # (manual placement in <staged-overlay>/) — replace with canonical
        # mirror; Lihaaf-managed dir has NO preservation semantics
        remove staged or remove_tree staged
        create canonical mirror
        # structured Error::OverlayMirrorFailed if unsafe to remove

# ── Stale cleanup pass (CASE 9 + CASE 14b) ──────────────────────────────────
for each entry F in <staged-overlay>/ (excluding the exclusion set + Cargo.toml):
    if F has no corresponding entry in <upstream>/:
        remove F   # stale entry; upstream no longer has it

# CASE 14b: must-be-absent-or-removed entries
# .git/ and Cargo.lock must never be present in the staged overlay.
# If a prior buggy mirror run or manual placement left them here, remove them.
for each path P in [<staged-overlay>/.git, <staged-overlay>/Cargo.lock]:
    if P exists (as any file/dir type):
        remove P (file: remove; dir: remove_tree)
        # return Err(Error::OverlayMirrorFailed { .. }) if removal fails

# ── Post-condition assertion (CASE 15) ──────────────────────────────────────
# ASSERT <staged-overlay>/Cargo.toml is a regular file, not a symlink.
# This is a type-only structural check (Option B-15a): it guards against a
# mirror bug that would replace the generated overlay manifest with a symlink
# to the upstream manifest. Manifest-content correctness is write_file_atomic's
# own contract (overlay.rs:527-543: bytes-match skip); stale-content state
# cannot arise from the mirror step itself.
# Fail with structured error if the assertion does not hold.
assert is_regular_file(<staged-overlay>/Cargo.toml)
    && !is_symlink(<staged-overlay>/Cargo.toml)
```

The result: `<staged-overlay>/src/`, `<staged-overlay>/include/`, `<staged-overlay>/build/`, `<staged-overlay>/gen/`, etc. all exist as symlinks pointing to the real upstream subdirectories (or copies under fallback). A build script running with `CARGO_MANIFEST_DIR = <staged-overlay>/` reads `src/cxx.cc` → follows the symlink → finds the real upstream `src/cxx.cc`. ✓ On rerun, the idempotent-skip guard (CASE 2) prevents re-creating symlinks that are already correct, while the reconcile-by-replacement branches (CASEs 3–9) ensure stale state is replaced rather than left silently in place.

**The staged `Cargo.toml` remains the only WRITTEN file in the overlay.** Everything else is a symlink (or copy under fallback). The overlay manifest is the authoritative content; the mirror is structural scaffolding for build-script access.

### 4.5.3 Copy fallback

On platforms or configurations where symlinks are unavailable, the mirror falls back to a recursive copy:

- **Triggers:** Windows without Developer Mode / symlink privilege enabled; filesystems mounted `nosymlink`; `std::os::unix::fs::symlink` returns `PermissionDenied` or `Unsupported`.
- **Behavior:** for each entry that would have been symlinked, recursively copy the upstream subtree into `<staged-overlay>/`. Copies are one-time; they are NOT re-synchronized if the upstream changes between overlay creation and cargo build. This is acceptable because the compat run is a single atomic invocation.
- **Cost:** copy fallback adds I/O overhead proportional to the upstream package size. For large packages (e.g. cxx with `gen/` containing generated C++ files), this may be measurable. Documented in the module-level rustdoc as a known cost of the fallback path.

**Implementation note.** The implementer should prefer `std::os::unix::fs::symlink` on Unix (always available) and `std::os::windows::fs::symlink_dir` / `symlink_file` on Windows (may fail with `ERROR_PRIVILEGE_NOT_HELD`). On failure, fall through to `std::fs::copy` + `std::fs::create_dir_all` recursive copy. The implementer wraps this in a helper `mirror_upstream_into_overlay(upstream_dir, staged_overlay_dir) -> Result<(), Error>`.

### 4.5.4 Exclusion list

The following top-level entries are EXCLUDED from the mirror (not symlinked or copied). Exclusions fall into two categories:

- **Disposable** — not mirrored; if present in `<staged-overlay>/` the mirror step does NOT touch them (they are owned by other Lihaaf subsystems or by cargo itself).
- **Must-be-absent-or-removed** — not mirrored; if present in `<staged-overlay>/` at the end of the per-entry forward pass, the stale-cleanup pass removes them (CASE 14b).

| Entry | Category | Reason |
|---|---|---|
| `target/` | Disposable | Build artifacts; re-ingesting them into the overlay would create circular artifact paths and dramatically increase I/O on large projects. The mirror step does not touch `target/` in either direction. |
| `.git/` | Must-be-absent-or-removed | Git metadata is irrelevant to build-script execution and could cause confusion if cargo or git tooling inspects the overlay dir. If present (e.g., from a prior buggy mirror or manual placement), the stale-cleanup pass removes it. |
| `Cargo.toml` | Must-be-absent-as-symlink (managed by `write_file_atomic`) | The overlay's generated manifest is already in place at this path; the upstream's original `Cargo.toml` must NOT overwrite it. The CASE 15 post-condition assertion guards this invariant. |
| `Cargo.lock` | Must-be-absent-or-removed | The overlay does not carry a lockfile; a symlinked or copied lockfile could interfere with cargo's fresh-resolve semantics for the overlay. If present, the stale-cleanup pass removes it. |

All other top-level entries (`src/`, `include/`, `build/`, `gen/`, `tests/`, `benches/`, `examples/`, custom dirs, non-manifest files) are symlinked (or copied under fallback). Subdirectory traversal is not needed at the top level — a single symlink per top-level entry is sufficient because the symlink target resolves the full subtree.

### 4.5.5 Call-site wiring

The mirror step is called from `write_overlay` (or the equivalent write path in overlay.rs) AFTER `write_file_atomic` has written the generated `Cargo.toml`. Ordering matters:

1. `write_file_atomic` writes `<staged-overlay>/Cargo.toml` (creates the dir via `create_dir_all`).
2. `mirror_upstream_into_overlay(<upstream>, <staged-overlay>)` — creates symlinks for each non-excluded top-level upstream entry.

The function signature:

```rust
fn mirror_upstream_into_overlay(
    upstream_dir: &Path,
    staged_overlay_dir: &Path,
) -> Result<(), Error>;
```

`Error::OverlayMirrorFailed` is a new error variant (additive). It carries the upstream entry path, the overlay target path, the I/O error, and whether the failure was in the symlink step or the copy fallback step. Structured error per the envelope contract.

### 4.5.6 Idempotency / rerun-state reconciliation (R6 — Option B)

The R5 pseudocode said only "create a symlink at `<staged-overlay>/E → <upstream>/E`". Codex R5 surfaced that the mirror step has no specified behavior on rerun — when `<staged-overlay>/E` already exists from a previous invocation, the step must define what to do for every reachable state. Fifteen rerun-state cases are enumerated below and covered by the per-entry pseudocode in §4.5.2.

#### Chosen strategy: Option B — Idempotent skip + reconcile-by-replacement

Skip the entry **only** when the current state is the canonical symlink to the correct `<upstream>/E`. For **all other cases**, reconcile by replacing the stale state with the canonical mirror. This is the analogue of `overlay.rs:527-543`'s bytes-match skip for the `Cargo.toml` write: skip when the current content already matches; replace/write when it does not.

Excluded entries (`target/`, `.git/`, `Cargo.toml`, `Cargo.lock`) are **never mirrored**. They fall into two categories: **disposable** (`target/`) — not touched by the mirror step in either direction; and **must-be-absent-or-removed** (`.git/`, `Cargo.lock`) — if present in the staged overlay at end of the forward pass, the CASE 14b stale-cleanup step removes them (see §4.5.2 and §4.5.4).

Copy fallback contract under Option B: byte-check for file entries (skip if bytes match, replace if not); exact-sync for directory entries (replace the destination tree, **removing destination-only files** — no merge). Merge semantics would allow stale upstream-removed files to persist in the staged overlay; exact-sync prevents this.

Manual content in `<staged-overlay>/` has **no preservation semantics**. The staged-overlay dir is Lihaaf-managed; any user-placed content that occupies a mirror-eligible path is replaced by the canonical mirror.

If reconciliation fails (e.g., `PermissionDenied` on a stale file that must be replaced), the function returns `Err(Error::OverlayMirrorFailed { .. })` with structured context.

#### 15-case rerun-state table

**Group A — Mirror state aligned with current upstream (skip or no-op):**

| Case | Staged-overlay state | Required behavior under Option B |
|---|---|---|
| **CASE 1** | Entry absent; upstream entry eligible | Create canonical symlink (or copy under fallback) — first-run path. |
| **CASE 2** | Symlink exists, target == `<upstream>/E` | **Skip** — idempotent guard. The analogue of `overlay.rs:530–531` bytes-match skip. |
| **CASE 10** | New upstream entry; no staged counterpart | Add canonical mirror (same as CASE 1; surfaced separately because it occurs during upstream-tree growth rather than first run). |
| **CASE 11** | Same entry; upstream file contents changed since last mirror | Symlink mode: skip (symlink is transparent; file reads through it see new upstream bytes automatically). Copy mode: byte-check → skip if match, replace if not. |

**Group B — Stale or wrong state (reconcile-by-replacement):**

| Case | Staged-overlay state | Required behavior under Option B |
|---|---|---|
| **CASE 3** | Symlink exists, target ≠ `<upstream>/E` (upstream moved, manual edit, or stale from different upstream) | Unlink staged entry; create canonical symlink → `<upstream>/E`. |
| **CASE 4** | Broken symlink (upstream entry was removed or renamed since last mirror) | Unlink staged entry. If `<upstream>/E` still exists: create canonical symlink. Else: upstream entry is gone too; staged symlink already unlinked above — no further action needed. |
| **CASE 5** | Real file at staged path; upstream entry is a file | Symlink mode: remove staged file; create canonical symlink. Copy mode: byte-check → skip if match, replace if not. |
| **CASE 6** | Real directory at staged path; upstream entry is a directory | Symlink mode: remove staged tree; create canonical symlink. Copy mode: exact-sync (**MUST remove destination-only files** — no merge; removed-upstream files must not persist in staged). |
| **CASE 7** | Staged entry type ≠ upstream entry type (file↔dir swap) | Remove stale staged entry (file: `remove`; dir: `remove_tree`); create canonical mirror with current type. Return `Err(Error::OverlayMirrorFailed { .. })` if removal fails. |
| **CASE 8** | File or dir at a mirror-eligible staged path that was never produced by the mirror (manual placement) | Replace with canonical mirror. Lihaaf-managed dir has NO preservation semantics. Return structured error if unsafe to remove. |
| **CASE 9** | Stale staged entry; upstream no longer has a corresponding entry | Remove the stale entry. (This is the stale-cleanup forward-pass, run after the per-entry forward pass as shown in §4.5.2.) |
| **CASE 12** | Mixed partial state (interrupted prior run; some entries correct, some stale/absent) | Apply per-entry reconciliation: CASE 2 skip for correct entries; appropriate CASE 3–9 branch for each stale or missing entry. Fail only on specific unrecoverable entry, not the entire mirror. |
| **CASE 13** | Entire staged overlay is stale from a different upstream (e.g., compat-root changed between runs) | Reconcile **every** entry against current upstream — do NOT trust the `Cargo.toml` bytes-match skip alone (bytes can coincidentally match across upstreams). Mirror idempotency is per-entry, not per-manifest. |
| **CASE 14a** | Excluded-disposable entry (`target/`) present in staged overlay | **Never mirror** upstream `target/`. `target/` is Lihaaf-managed build-artifact state; it is not owned by the mirror step. Leave it alone — do not touch, do not remove. |
| **CASE 14b** | Excluded-must-be-absent entry (`.git/` or `Cargo.lock`) present in staged overlay | **Never mirror** upstream versions of these. If `.git/` or `Cargo.lock` are present in `<staged-overlay>/` at the end of the per-entry forward pass, the stale-cleanup pass **must remove them** (see §4.5.2 stale-cleanup extension). If removal fails, return `Err(Error::OverlayMirrorFailed { .. })`. |
| **CASE 15** | `<staged-overlay>/Cargo.toml` is a symlink after mirror completes (mirror bug replaced the generated overlay manifest with a symlink to the upstream manifest) | After mirror reconciliation completes, **assert** `<staged-overlay>/Cargo.toml` is a regular file and not a symlink. Return structured error if the assertion fails. **Scope note (Option B-15a):** this post-condition checks file type only, not content. Manifest-content correctness is `write_file_atomic`'s contract (`overlay.rs:527–543`): it skips the write only when bytes match, so a stale-content state cannot arise from the mirror step itself. The mirror step's responsibility is structural (file type, symlink integrity), not content. |

#### 7-item idempotency-contract decisions

The following decisions govern the `mirror_upstream_into_overlay` idempotency contract and must be respected throughout the implementation:

1. **Idempotent-skip guard required.** The mirror step skips an entry **only** when the current staged symlink already points to the correct `<upstream>/E`. This is the per-entry analogue of `overlay.rs:527–531`'s bytes-match skip for the `Cargo.toml` write.

2. **Reconcile, don't merely create-if-missing.** A re-entrant mirror call must replace stale symlinks, stale copies, and wrong-type entries — not merely fill gaps. A mirror that only calls `create_symlink` when the path is absent will silently leave wrong-target symlinks (CASE 3) and real files (CASE 5) in place.

3. **Desired root state.** After the mirror step completes (first-run or rerun), the staged-overlay directory's root must contain exactly: the generated `Cargo.toml` (written by `write_file_atomic`) **plus** one canonical mirror entry per non-excluded top-level upstream entry. Excluded entries (`target/`, `.git/`, `Cargo.toml`, `Cargo.lock`) are **not** upstream mirrors. The "exactly" claim for `.git/` and `Cargo.lock` is backed by the CASE 14b stale-cleanup pass (§4.5.2), which removes them if present. `target/` is Lihaaf-managed build state (disposable category, CASE 14a) and is not asserted absent.

4. **Discrepancies = replace-or-error.** Leaving mismatched content silently in place (wrong-target symlink, stale real file) is the unsafe path. A symlink pointing to the wrong upstream leads to `cargo build` reading files from a different crate version; a stale real file may contain outdated source. Every mismatch must be replaced or produce a structured error.

5. **Copy-fallback exact-sync.** When in copy mode, directory entries must be synchronized by **removing destination-only files** (files that exist in `<staged-overlay>/E/` but not in `<upstream>/E/`) as part of exact-sync. A merge (additive copy) would leave upstream-removed files as ghost entries that can silently affect build-script behavior or cargo's incremental compilation.

6. **No preservation for manual collisions.** `target/lihaaf-overlay` is Lihaaf-managed. Any user-placed content that occupies a mirror-eligible path is replaced by the canonical mirror without warning. The staged-overlay dir is not user-editable space.

7. **Failure modes documented.** A non-idempotent mirror produces `AlreadyExists` on the second `materialize_overlay` call (the second call tries to create a symlink at a path that the first call already created, without the idempotent-skip guard). Worse: a mirror that skips reconciliation leaves stale content that causes `cargo build` hard errors (cxx `src/cxx.cc` not found if upstream path changed) or silent false probe results (anyhow `src/nightly.rs` from a different upstream). Test §5.1.4 (extended in R6) and the new §5.1.14 / §5.1.15 tests are the load-bearing verification of this contract.

### 4.5.7 Interaction with the `apply_self_patch_policy` function

The staged-mirror step (§4.5.5) is independent of the `apply_self_patch_policy` function (§4.1). They address different problems:

- `apply_self_patch_policy` rewrites the overlay manifest's `[patch.crates-io.<self>]` to redirect cargo's resolver to the staged-overlay source-id.
- `mirror_upstream_into_overlay` populates the staged-overlay dir with symlinks so build scripts can access package-root files.

Both are required for Class A pilots (cxx, anyhow, thiserror). They run independently; neither depends on the other's output. The mirror runs after the manifest is written; the policy runs during manifest construction.

### 4.5.8 Known limitations and non-goals

- **Symlink cycles.** If the upstream dir itself contains symlinks that point back into `target/`, the mirror's top-level symlink to those entries would not create a cycle (we only create one level of top-level symlinks, and `target/` is excluded). Deep recursive symlink cycles inside upstream source dirs are the upstream's problem and are out of scope.
- **Overlay update on upstream change.** The mirror is created once per overlay write. If the upstream tree changes between overlay creation and cargo build, the symlinked files are the new files (symlinks are transparent). For compat runs (single invocation), this is not a concern.
- **Files at `<staged-overlay>/Cargo.toml`.** The exclusion of `Cargo.toml` in §4.5.4 prevents the upstream's manifest from overwriting the overlay's generated manifest. The implementer must check for this explicitly: after the loop, assert that `<staged-overlay>/Cargo.toml` is a regular file and not a symlink (type-only structural check; see §4.5.2 CASE 15 post-condition — manifest-content correctness is `write_file_atomic`'s contract, not the mirror step's).
- **`.cargo/config.toml` and workspace-level configs.** If the upstream tree has a `.cargo/` directory at the top level, it will be symlinked into the overlay dir. This is intentional — cargo reads `.cargo/config.toml` relative to the manifest dir and up to the filesystem root; having the upstream's `.cargo/` available in the overlay dir means any upstream-level cargo config is respected during the overlay build. This is correct behavior.

---

## 5. Test plan

### 5.1 Unit tests in `src/compat/overlay.rs::tests`

Each test must FAIL without the fix and PASS with the fix (defense-in-depth criterion from dispatch §5). The tests cover all four Option H rules (Rule 1 INJECT / Rule 2 REMAP / Rule 3 CONTINUE-ABSOLUTIZE / Rule 4 REJECT) plus the lexical normalizer's BLOCK-2 corner cases plus the orthogonal-key preservation contract.

1. **`apply_self_patch_writes_entry_for_named_package_rule1_inject`** — **Rule 1 (INJECT) happy path.** Input manifest has `[package].name = "demo"` and NO `[patch]` table at all. After `materialize_overlay`, parsed overlay contains `[patch.crates-io.demo].path` and that value equals the absolutized STAGED OVERLAY DIR (`<upstream-dir>/target/lihaaf-overlay`, forward-slash form). NO upstream patch existed → INJECT fires.

2. **`apply_self_patch_no_entry_when_package_name_absent`** — defense-in-depth. Input manifest where `[package]` exists but `name` is missing. `materialize_overlay` returns `Ok(_)`. NO `[patch.crates-io]` entry is injected, but the rest of the overlay materializes correctly.

3. **`apply_self_patch_path_form_is_staged_overlay_dir_not_upstream_rule1`** — **Rule 1 + BLOCK-1 self-loop avoidance pin.** Verify the emitted `path` value is the absolutized staged-overlay-dir (i.e. ends with `/target/lihaaf-overlay`), NOT the upstream dir. A regression that re-aims at the upstream dir would re-introduce the self-loop bug R1 had.

4. **`apply_self_patch_idempotent_second_materialize`** — **R6 extended for Option B mirror idempotency.** Call `materialize_overlay` twice on the same upstream. Verify:
   (a) Both manifest bytes match (existing assertion — policy idempotency).
   (b) The second call returns `Ok(_)` — no `AlreadyExists` error, no `OverlayMirrorFailed` error, even though the staged-overlay dir and all mirror entries already exist from the first call.
   (c) After the second call, the staged-overlay state is identical to after the first call: every mirror entry is the canonical symlink (or copy) to `<upstream>/E`; the generated `Cargo.toml` is a regular file, not a symlink.
   (d) The second call did NOT re-create any symlink that was already correct (CASE 2 idempotent-skip fired for all already-canonical entries).
   Assertion (d) MUST be verified by recording each canonical symlink's inode (`std::os::unix::fs::MetadataExt::ino()`) before the second call and asserting the inode is unchanged afterward — a re-created symlink gets a NEW inode on most filesystems even within the same second, whereas a skipped (idempotent CASE 2) symlink retains its original inode. **Do NOT use mtime alone**: ext4's default 1-second mtime granularity can mask a broken implementation that re-creates symlinks within the same second window, making the assertion vacuously pass on fast hardware. The inode-identity check is the per-entry analogue of `overlay.rs:527–531`'s bytes-match skip for the manifest write.

5. **`apply_self_patch_remap_when_upstream_self_patch_cxx_shape_rule2`** — **Rule 2 (REMAP) cxx-shape pin.** Input has `[patch.crates-io.demo] = { path = "." }` (the EXACT shape cxx's upstream Cargo.toml carries, per `tests/compat/overlay_determinism.rs:938-940`, `:1753-1754`). `materialize_overlay` must return `Ok(_)`. The overlay's `[patch.crates-io.demo].path` must equal `<staged-overlay-dir>` (the REMAPPED form, NOT the upstream-dir-absolute form R3 specified). NO competing PRESERVE-AS-IS entry, NO competing INJECT entry. This is the explicit pin that R4 Rule 2 REMAP fires when the upstream's path resolves to the upstream root crate.

6. **`apply_self_patch_remap_path_dot_slash_form_rule2`** — **Rule 2 variant pin.** Input has `[patch.crates-io.demo] = { path = "./" }` (variant with trailing slash). Rule 2 must fire because the joined path lexical-normalizes to upstream root. Output: REMAP to staged-overlay-dir. Verifies the lexical normalizer correctly handles trailing slash and `./` form.

7. **`apply_self_patch_rejects_when_upstream_path_targets_external_source_rule4_path`** — **Rule 4 (REJECT) path-to-fork pin.** Input has `[patch.crates-io.demo] = { path = "../forked-demo" }` (a vendored-fork-style override; resolves to a sibling dir, NOT upstream root). `materialize_overlay` must return `Err(_)` with the structured `Error::CompatPatchOverrideConflict` variant. The error message must reference: (a) the crate name `demo`; (b) the upstream's existing entry; (c) the v0.2/v1.1 escape hatch (`--compat-allow-patch-override`).

8. **`apply_self_patch_rejects_when_upstream_git_form_rule4_git`** — **Rule 4 (REJECT) git-source pin.** Input has `[patch.crates-io.demo] = { git = "https://example.com/demo" }`. Rule 4 fires (no `.path`, but `git` is present). `materialize_overlay` returns `Err(Error::CompatPatchOverrideConflict { .. })` with the same error-shape contract as test 7.

9. **`apply_self_patch_rejects_when_upstream_mixed_rule4_mixed`** — **Rule 4 (REJECT) mixed-shape pin.** Input has `[patch.crates-io.demo] = { path = ".", git = "https://example.com/demo" }` (pathological, but TOML-valid). Rule 4 fires (both `.path` and `git` present). `materialize_overlay` returns `Err(Error::CompatPatchOverrideConflict { .. })`.

10. **`apply_self_patch_preserves_other_crate_patches_when_remap_or_inject`** — **Orthogonal-key preservation pin (Rule 3 + Rule 1/2).** Input has BOTH `[patch.crates-io.serde] = { git = "..." }` (an unrelated crate's patch — orthogonal to the target crate's self-patch) AND `[patch.crates-io.demo] = { path = "." }` (cxx-shape, triggers Rule 2). The overlay's `[patch.crates-io.serde]` must be preserved verbatim (Rule 3 no-op for non-target keys; the existing `absolutize_patch_paths` pass handles any path-bearing serde entry). The overlay's `[patch.crates-io.demo]` must be REMAPPED to staged-overlay-dir per Rule 2.

11. **`lexical_path_normalize_handles_dot_and_trailing_slash`** — unit test on the normalizer helper (§4.1.1): assert that `/work/cxx`, `/work/cxx/.`, and `/work/cxx/` all lexically-normalize to the same component vector; that `/work/cxx/..` does NOT lexically-equal `/work/cxx`; that `/work/cxx/target/lihaaf-overlay` does NOT equal `/work/cxx`.

12. **`lexical_path_normalize_handles_repeated_separators`** — **R3 BLOCK-2 finish, unchanged in R4.** Unit test asserting `lexical_path_normalize(Path::new("/work//cxx")) == lexical_path_normalize(Path::new("/work/cxx"))` (`Path::components()` collapses `//` on Unix). Also: `lexical_path_normalize(Path::new("/work///cxx")) == lexical_path_normalize(Path::new("/work/cxx"))` (multiple separators).

13. **`lexical_path_normalize_does_not_resolve_symlinks`** — **R3 BLOCK-2 finish, unchanged in R4.** Unit test creating a tmpdir with a real subdir and a symlink to it. Assert that `lexical_path_normalize(real_path) != lexical_path_normalize(symlink_path)` even though `canonicalize()` would equate them. Documents the known limitation in test form. Test must be Unix-gated (`#[cfg(unix)]`) since Windows symlink semantics differ.

14. **`mirror_upstream_rerun_reconciles_stale_entries`** — **R6 new. Option B reconcile-by-replacement for CASE 3 / CASE 5 / CASE 6 / CASE 7 / CASE 12 representative subset.** Constructs a staged overlay dir with pre-seeded stale state, calls `mirror_upstream_into_overlay` (or the equivalent through `materialize_overlay`), and asserts the stale state is replaced with the canonical mirror. Covers the following per-entry sub-cases (one logical assertion group each):

   - **CASE 3 (wrong-target symlink):** Pre-seed `<staged-overlay>/src` as a symlink pointing to an arbitrary tmpdir (not `<upstream>/src`). After the mirror step, `<staged-overlay>/src` must be a symlink pointing to `<upstream>/src`.
   - **CASE 5 (real file at mirror-eligible path; upstream has a regular file):** Pre-seed `<staged-overlay>/example.txt` as a regular file with dummy content (e.g., `"stale dummy bytes"`). `<upstream>/example.txt` is also a regular file with different content (e.g., `"real upstream bytes"`). After the mirror step (symlink mode), `<staged-overlay>/example.txt` must be a symlink pointing to `<upstream>/example.txt`; reading through it must yield the upstream content, not the dummy bytes.
   - **CASE 6 (real directory at mirror-eligible path; upstream has a directory):** Pre-seed `<staged-overlay>/build` as a real directory tree (e.g., from a prior copy-fallback run). `<upstream>/build` is also a directory. After the mirror step (symlink mode), `<staged-overlay>/build` must be a symlink pointing to `<upstream>/build`. The stale real directory must be gone.
   - **CASE 7 (type mismatch — staged regular file where upstream has a directory):** Pre-seed `<staged-overlay>/include` as a regular file with dummy content. `<upstream>/include` is a directory. After the mirror step (symlink mode), `<staged-overlay>/include` must be a symlink pointing to `<upstream>/include`. The dummy file must be gone.
   - **CASE 12 (mixed partial state — some entries correct, some stale):** Pre-seed the staged overlay with CASE 2 (canonical symlink, should be skipped) for `src/` AND CASE 3 (wrong-target symlink, should be reconciled) for `include/`. After the mirror step, `src/` symlink is unchanged (same inode preserved per the inode-identity check) and `include/` symlink is corrected.

   The test constructs a minimal upstream directory tree with `src/`, `include/`, and `build/` top-level subdirectories AND a top-level `example.txt` regular file (no `Cargo.toml` or excluded entries except the generated overlay manifest already in place from a prior `write_file_atomic` call). Each sub-case assertion is gated `#[cfg(unix)]` where symlink creation is unconditionally available. A copy-fallback sub-case is NOT included here — that is covered by §5.1.15 below.

15. **`mirror_copy_fallback_exact_sync_removes_destination_only_files`** — **R6 new. CASE 6 copy-fallback exact-sync: removed-upstream files are purged from staged.** This test specifically validates decision 5 of the idempotency contract (§4.5.6): copy-fallback exact-sync must remove destination-only files, not merge.

   - Construct a upstream directory `<upstream>/src/` with files `a.rs` and `b.rs`.
   - Run the mirror step (copy mode — use the copy-fallback path explicitly by forcing it in the test, e.g., by calling the copy-fallback helper directly or by invoking `mirror_upstream_into_overlay` with a test flag that disables symlink creation). After the first run, `<staged-overlay>/src/` contains copies of `a.rs` and `b.rs`.
   - Remove `<upstream>/src/b.rs` (simulate upstream file deletion between runs).
   - Run the mirror step again (copy mode, second rerun). After the second run, `<staged-overlay>/src/` must contain `a.rs` but NOT `b.rs` — exact-sync purged the destination-only file.
   - If the implementation does a merge instead (additive copy), `b.rs` would persist in `<staged-overlay>/src/` after the second run, and the test would FAIL. This turns the Option B contract decision 5 into a load-bearing regression test.

   The test must be designed so it can force copy-fallback mode even on platforms where symlinks are available (e.g., via a test-only constructor argument or a module-level cfg flag). If the implementation does not expose a testable copy-fallback override, the implementer must add one — the test surface coverage of CASE 6 in copy mode is required.

**Test count: 15** (R6 adds §5.1.14 and §5.1.15 for rerun-state reconciliation; §5.1.4 extended for Option B second-call semantics). If the implementer prefers to expand CASE 3/5/6/7/12 into separate test functions rather than a single composite test (§5.1.14), they may do so during PR-1 — the logic is equivalent and reviewers may prefer explicitness over composition.

### 5.2 Integration tests in `tests/compat/overlay_determinism.rs`

#### 5.2.0 Cargo acceptance of patched dependency graphs (R3 SEC-6 / R4 SEC-8 / R8 narrowing)

Codex R2 raised that the §5.2 synthetic-repro shape "foo → test-suite → foo" looks like a normal-dep cycle, which cargo's resolver rejects. Codex R3 SEC-8 further surfaced that R3's claim "cargo accepts the patched topology" was relying on a misleading cited test — the existing `cargo_accepts_rich_overlay_for_dylib_build` test exercises `rich-demo = { path = "." }` at a DIFFERENT topology than `foo → test-suite → foo`. R4 added a dedicated load-bearing test. **R8 (post-CI failure diagnosis, Codex rollout 019e3cc3) narrowed the claim further:**

**R8 narrowed claim: cargo's self-patch collapses registry-name references only when they do not create an active package self-dependency cycle.** It is valid for cxx's cargo-build path because cxx lacks the synthetic active root-to-test-suite dependency. Prior revisions stated that "cargo collapses these to a single Package in the resolved graph rather than treating them as a cycle" for ANY `root → member → root` topology. That is overbroad. Empirical verification (CI failure, PR #56, Codex rollout 019e3cc3) shows:

- cargo collapses sources (resolves two `path` references with the same absolute path to one source-id);
- cargo DOES still reject active dep cycles — if the root carries `[dependencies] test-suite = { path = "test-suite" }` AND test-suite carries `bar = "1.0"` (redirected to staged-overlay root via INJECT/REMAP), cargo fires `cyclic package dependency: package bar v1.0.0 (...) depends on itself`. The cycle check runs AFTER source-id resolution.

The correct characterization of when self-patching succeeds: **the root must NOT declare an active `[dependencies]` edge to the workspace member that carries the back-reference**. Real upstreams like cxx satisfy this — cxx's `Cargo.toml` carries `[workspace] members = ["test-suite"]` but no `[dependencies] test-suite = { path = "test-suite" }` at the root. The §5.2.6 and §5.2.9 synthetic fixtures are updated in R8 to match this faithful shape.

Cargo's resolver, when evaluating the faithful shape — root `foo` (source `path+file://<staged-overlay>`) with workspace member `test-suite` (workspace only, NOT a root dep) → `test-suite` carrying `foo = "1.0"` — under the influence of `[patch.crates-io.foo] = { path = "<staged-overlay-dir>" }` (Rule 1 INJECT or Rule 2 REMAP):

1. cargo resolves `test-suite`'s `foo = "1.0"` registry-name reference;
2. the `[patch.crates-io.foo]` mapping fires; the resolved source is `path+file://<staged-overlay-dir>`;
3. cargo's resolver sees the root and the member's `foo` dep resolve to the same source-id → no ambiguity, no active-dep cycle (root doesn't dep on member) → resolution clean.

This is the standard "self-patch" idiom cargo's `[patch.crates-io]` was designed for (cargo book § "The [patch] section"), valid specifically when the root lacks an active dep edge to the redirecting member.

**R8 load-bearing tests:**
- **`cargo_accepts_remap_when_upstream_self_patch_cxx_shape`** (§5.2.6): Rule 2 REMAP proof for the cxx failure shape. Fixture is faithful to cxx's actual `Cargo.toml` (workspace declaration only, no root dep on member). R8 removes the `[dependencies] test-suite = { path = "test-suite" }` that was causing the CI cycle.
- **`cargo_accepts_workspace_member_registry_dep_via_self_patch`** (§5.2.9): Rule 1 INJECT proof, rescoped from the empirically-false `root → member → root` active-dep topology to the faithful workspace-member-registry-dep-via-self-patch topology. R8 renames the test and removes the root dep on member.

**Real cargo error strings the synthetic repros must reproduce (R3 BLOCK-3 finish, unchanged in R4).** The synthetic test descriptions reference real cargo error strings even though the synthetic fixtures use synthetic crate names (`foo` / `bar` instead of `cxx` / `serde_json`) to avoid name-collision with real registry entries during CI:

- **cxx-shape (Rule 2 REMAP cargo-build test):** real cargo error string is `package \`cxx\` links to the native library \`cxxbridge1\`, but it conflicts with a previous package which links to \`cxxbridge1\` as well`. Synthetic test fixture uses `links = "foo-native"` to avoid registry name collision; the test assertion checks for the substring `links to the native library` and the specific `foo-native` value in the synthetic case.
- **serde-json-shape (Rule 1 INJECT cargo-build test):** real cargo error string is `error: specification \`serde_json\` is ambiguous`. Synthetic test fixture uses `bar` instead of `serde_json` to avoid registry name collision; the test assertion checks for the substring `specification \`bar\` is ambiguous` (cargo emits the actual referenced package name verbatim).

Both assertions verify pre-fix FAIL (with the synthetic error strings) and post-fix PASS.

**Test layout (R4 Option H mapping):** the dispatch's 10-test list (1 INJECT cargo accept / 2 REMAP cargo accept / 3 CONTINUE-ABSOLUTIZE / 4 REJECT / 5 SEC-8 cargo-graph proof / 6 git-dep coverage / 7 lex-norm separators / 8 lex-norm symlinks / 9 orthogonal key / 10 corpus determinism) is split between §5.1 (unit-level tests 7-9 and parts of 1-4) and §5.2 (integration / cargo-build-gated tests). The list below is the §5.2 integration test layout.

1. **Corpus addition.** New `with_self_patch_injected.input.toml` + `.expected.toml` pair under `tests/compat/overlay_corpus/`. Input is a bare single-crate manifest (Rule 1 INJECT). Expected output shows the injected `[patch.crates-io.<crate>] = { path = "__UPSTREAM_DIR__/target/lihaaf-overlay" }` line. Asserts byte-stable injection. Same `__UPSTREAM_DIR__` placeholder convention as the existing corpus.

2. **Corpus addition: `with_self_patch_remapped`** — **R4 NEW.** New fixture pair covering Rule 2 REMAP. Input has `[patch.crates-io.<crate>] = { path = "." }` (cxx-shape). Expected output shows the REMAPPED entry: `[patch.crates-io.<crate>] = { path = "__UPSTREAM_DIR__/target/lihaaf-overlay" }`. (The expected line is byte-identical to the Rule 1 INJECT case — both rules emit the same form. The fixture distinction is in the INPUT.) This is the corpus-level proof that Rule 2 fires deterministically and the emitted byte shape matches Rule 1's.

3. **Corpus update for existing fixtures.** Every existing corpus expected file currently lacks any self-patch entry. After the fix, every overlay carries a self-patch — so EVERY corpus `.expected.toml` must be updated to include the appropriate `[patch.crates-io.<crate>]` entry. Fixtures to update:
   - `bare_package`: Rule 1 INJECT → expected entry `[patch.crates-io.<crate>] = { path = "__UPSTREAM_DIR__/target/lihaaf-overlay" }`.
   - `with_rlib_only`: Rule 1 INJECT → same.
   - `with_cdylib`: Rule 1 INJECT → same.
   - `with_patch_section`: **R4-CRITICAL UPDATE.** Currently this fixture's input has `[patch.crates-io.<crate>] = { path = "." }` (the cxx-shape). Under R4 Rule 2 REMAP, the expected output is REMAPPED to `[patch.crates-io.<crate>] = { path = "__UPSTREAM_DIR__/target/lihaaf-overlay" }` — NOT R3's preserved `<upstream>/.` absolutized form. This is the corpus-level pin for the R3 → R4 behavioral shift documented in §3.
   - `with_comments`: depends on whether this fixture has a `[patch]` section. If it has a `<self>` entry, the appropriate rule fires (Rule 2 if path = "." resolves to upstream root; otherwise Rule 1). Implementer reads the fixture during PR-1 and updates accordingly.
   - `with_replace_section`: Rule 1 INJECT (no `[patch]` in this fixture) → entry added alongside the existing `[replace]` lines.

4. **Corpus-list test update.** The hardcoded `names` array at `tests/compat/overlay_determinism.rs:453-460` must be extended to include BOTH new entries: `"with_self_patch_injected"` (Rule 1) AND `"with_self_patch_remapped"` (Rule 2). The expected-count assertion at `tests/compat/overlay_determinism.rs:495-498` must be bumped from 6 to 8 (R4 — TWO new fixtures, not R3's one). The test name remains `byte_identical_across_two_lihaaf_binaries_on_corpus`.

5. **Cargo-build-gated test: `cargo_accepts_inject_when_clean_upstream_anyhow_shape`** — Rule 1 INJECT cargo-graph proof. Gated behind `LIHAAF_RUN_CARGO_BUILD_TESTS=1` per [[lihaaf-no-local-binary-builds]]. Constructs the minimal anyhow-shape repro:

   - Upstream Cargo.toml at `<tmp>/Cargo.toml`:
     ```toml
     [package]
     name = "anyhow-like"
     version = "1.0.0"
     edition = "2021"

     [lib]
     path = "src/lib.rs"
     ```
   - Minimal source at `<tmp>/src/lib.rs`: `pub fn _stub() {}`.
   - No workspace members → no transitive registry-version path → patch is benign.
   - Invoke `materialize_overlay(<tmp>/Cargo.toml)`, then `cargo rustc --manifest-path <staged>`.
   - **Assertion:** cargo succeeds with exit 0 (Rule 1 INJECT happy path; patch is benign for clean single-crate upstream).

6. **Cargo-build-gated test: `cargo_accepts_remap_when_upstream_self_patch_cxx_shape`** — Rule 2 REMAP cargo-graph proof AND staged-mirror strategy validation (M.4 closure). Constructs the cxx-shape repro with a real file-reading `build.rs`:

   - Upstream Cargo.toml at `<tmp>/Cargo.toml` (R8: faithful to cxx's actual `Cargo.toml` — workspace declaration only, NO root dep on member; verified by Codex rollout 019e3cc3):
     ```toml
     [package]
     name = "foo"
     version = "1.0.0"
     edition = "2021"
     links = "foo-native"
     build = "build.rs"

     [lib]
     path = "src/lib.rs"

     [workspace]
     members = ["test-suite"]

     # Mirrors cxx's upstream Cargo.toml: pre-existing self-patch to "." per
     # overlay.rs:1349-1351 + tests/compat/overlay_determinism.rs:938-940.
     # R8: cxx does NOT carry [dependencies] test-suite = { path = "test-suite" }
     # at the root. The prior fixture had this edge; it caused a `cyclic package
     # dependency: foo v1.0.0 depends on itself` CI failure (PR #56). Removed in
     # R8 to match the real cxx shape. cargo rustc runs `-p foo`; test-suite is
     # in the workspace but not a build dep of the root.
     [patch.crates-io]
     foo = { path = "." }
     ```
   - **Upgraded `build.rs` at `<tmp>/build.rs`** (R5 M.4 closure — no longer a stub):
     ```rust
     fn main() {
         // Mirrors cxx build.rs:143-148: read a package-root-relative C source.
         let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
         let src_path = std::path::Path::new(&manifest_dir).join("src").join("cxx_stub.cc");
         // Must be able to open and read the file — hard error if file not found.
         let _content = std::fs::read_to_string(&src_path)
             .expect("build.rs: failed to read src/cxx_stub.cc via CARGO_MANIFEST_DIR");
         // Mirrors cxx build.rs:154-159: reference a header file.
         let include_path = std::path::Path::new(&manifest_dir).join("include").join("stub.h");
         assert!(include_path.exists(),
             "build.rs: include/stub.h not found via CARGO_MANIFEST_DIR: {:?}", include_path);
         println!("cargo:rerun-if-changed=src/cxx_stub.cc");
         println!("cargo:rerun-if-changed=include/stub.h");
     }
     ```
   - Stub source at `<tmp>/src/cxx_stub.cc`: `// stub C++ file for build-script test`.
   - Stub header at `<tmp>/include/stub.h`: `// stub header for build-script test`.
   - Minimal Rust source at `<tmp>/src/lib.rs`: `pub fn _stub() {}`.
   - Workspace member at `<tmp>/test-suite/Cargo.toml`:
     ```toml
     [package]
     name = "test-suite"
     version = "0.0.0"
     edition = "2021"

     [dependencies]
     foo = "1.0"
     ```
   - Workspace member source at `<tmp>/test-suite/src/lib.rs`: `pub fn _stub() {}`.
   - Invoke `materialize_overlay(<tmp>/Cargo.toml)`. Verify the overlay's `[patch.crates-io.foo].path` equals `<staged-overlay-dir>` (the REMAPPED form). Verify `<staged-overlay>/src/cxx_stub.cc` and `<staged-overlay>/include/stub.h` are accessible (via symlink or copy) — this is the staged-mirror verification (§4.5).
   - Invoke `cargo rustc --manifest-path <staged>`. Assert exit 0. **This test now validates TWO things simultaneously:** (a) Rule 2 REMAP correctly redirects `foo = "1.0"` so `links = "foo-native"` collision cannot fire (policy correctness); (b) the staged-mirror strategy makes `src/cxx_stub.cc` and `include/stub.h` accessible from the build script's `CARGO_MANIFEST_DIR` perspective (mirror correctness). Pre-fix (without the mirror), the build.rs `read_to_string` call FAILS with `No such file or directory` — the test is BLOCKED pre-fix on BOTH the patch policy AND the mirror. Post-fix it PASSES because both are implemented.
   - **Pre-fix assertion (if implementer chooses to keep it):** without R5, the materialized overlay either has the wrong patch form OR the build.rs fails to read `src/cxx_stub.cc` from the empty staged overlay dir. Pre-fix is BLOCKED on what version of the materializer is running; the implementer's call on whether to assert the pre-fix failure explicitly.

7. **Cargo-build-gated test: `cargo_accepts_continue_absolutize_when_non_root_patch`** — Rule 3 CONTINUE-ABSOLUTIZE cargo-graph proof. Constructs a synthetic upstream with the SELF-patch AND a non-self sibling patch (mirrors cxx's `cxx-build = { path = "gen/build" }` entry alongside the `cxx` self-patch):

   - Upstream Cargo.toml at `<tmp>/Cargo.toml`:
     ```toml
     [package]
     name = "foo"
     version = "1.0.0"
     edition = "2021"

     [lib]
     path = "src/lib.rs"

     [patch.crates-io]
     foo = { path = "." }        # Rule 2 REMAP
     foo-helper = { path = "helper" }   # Rule 3 CONTINUE-ABSOLUTIZE (non-self key)
     ```
   - Minimal `helper/Cargo.toml`: `[package] name = "foo-helper" version = "0.0.0" edition = "2021" [lib] path = "src/lib.rs"`.
   - Minimal `helper/src/lib.rs`: `pub fn _stub() {}`.
   - Minimal `src/lib.rs`: `pub fn _stub() {}`.
   - Invoke `materialize_overlay`. Verify the overlay's `[patch.crates-io.foo].path` is REMAPPED to `<staged-overlay-dir>` (Rule 2); the overlay's `[patch.crates-io.foo-helper].path` is absolutized to `<upstream>/helper` (Rule 3 — the existing `absolutize_patch_paths` pass handles it).
   - Invoke `cargo rustc --manifest-path <staged>`. Assert exit 0. Rule 3 proof: the non-self patch entry survives R4's policy untouched and is correctly absolutized for cargo.

8. **Cargo-build-gated test: `materialize_rejects_when_upstream_patch_targets_external_source_rule4`** — Rule 4 REJECT proof. **Negative test:** materializer must REJECT before cargo even runs. Constructs:

   - Upstream Cargo.toml at `<tmp>/Cargo.toml`:
     ```toml
     [package]
     name = "foo"
     version = "1.0.0"
     edition = "2021"

     [lib]
     path = "src/lib.rs"

     [patch.crates-io]
     # Vendored-fork shape: path resolves to a sibling dir, NOT upstream root.
     foo = { path = "../my-fork" }
     ```
   - Minimal `<tmp>/../my-fork/Cargo.toml`: stub manifest (not actually used by the test; just makes the test reproducible).
   - Minimal `<tmp>/src/lib.rs`: `pub fn _stub() {}`.
   - Invoke `materialize_overlay(<tmp>/Cargo.toml)`. **Assertion:** returns `Err(_)` with the structured error variant. NO `cargo rustc` invocation — the test stops at the materializer's REJECT. The error message must reference the v0.2/v1.1 escape hatch.

9. **`cargo_accepts_workspace_member_registry_dep_via_self_patch`** — **R8 rescope of R4 SEC-8 closure test (LIHAAF_RUN_CARGO_BUILD_TESTS=1).** Rule 1 INJECT proof: workspace member's registry dep gets correctly remapped to the staged overlay, even when the root carries no direct dep on the member.

   **R8 rescope rationale (Codex rollout 019e3cc3):** The prior `cargo_accepts_root_to_test_suite_to_root_topology` fixture had `[dependencies] test-suite = { path = "test-suite" }` in the root, creating a `bar → test-suite → bar` active-dep cycle. Empirical CI failure (PR #56) confirmed cargo rejects that topology unconditionally — `cyclic package dependency: package bar v1.0.0 (...) depends on itself`. The "root → member → root active-dep topology proof" is therefore empirically impossible. The faithful shape (matching real upstreams like cxx/serde_json) is: workspace declaration only, no root dep on member. The rescoped test proves the same Rule 1 INJECT property without the unfaithful dep edge.

   Distinct from §5.2.6 (Rule 2 cxx-shape) because:

   - §5.2.6 proves Rule 2 REMAP + cargo accepts the resulting topology FOR THE CXX FAILURE SHAPE (with `links`, `build.rs`, and the pre-existing self-patch `path = "."`).
   - §5.2.9 (this test) proves Rule 1 INJECT + cargo accepts the workspace-member-registry-dep topology AS A GENERAL CARGO-RESOLVER PROPERTY, independent of `links` collision. Uses the simpler serde-json-shape (no `links`, no `build.rs`) so cargo failures surface at the resolver level (`specification \`bar\` is ambiguous`) not the build-script level.
   - Both tests now use the faithful shape: workspace declaration only, no root dep on member.

   - Upstream Cargo.toml at `<tmp>/Cargo.toml` (R8: workspace declaration only, no root dep on member):
     ```toml
     [package]
     name = "bar"
     version = "1.0.0"
     edition = "2021"

     [lib]
     path = "src/lib.rs"

     [workspace]
     members = ["test-suite"]
     ```
   - Workspace member at `<tmp>/test-suite/Cargo.toml`:
     ```toml
     [package]
     name = "test-suite"
     version = "0.0.0"
     edition = "2021"

     [dependencies]
     bar = "1.0"
     ```
   - Workspace member source at `<tmp>/test-suite/src/lib.rs`: `pub fn _stub() {}`.
   - Root source at `<tmp>/src/lib.rs`: `pub fn _stub() {}`.
   - Invoke `materialize_overlay(<tmp>/Cargo.toml)`. Rule 1 fires (no upstream patch). The overlay carries `[patch.crates-io.bar] = { path = "<staged-overlay-dir>" }`.
   - Invoke `cargo rustc -p bar --manifest-path <staged>`. Assert exit 0. Root doesn't dep on member so no active-dep cycle. The injected patch redirects `bar = "1.0"` in the member to the staged-overlay path → both `bar` references (root `[package]` and member registry dep) resolve to the same source-id → resolution clean.

10. **Cargo-build-gated test: `cargo_build_anyhow_shape_probe_file_resolves_via_mirror`** — **M.5 closure: staged-mirror fixes anyhow-shape silent-false probe.** Constructs the anyhow-shape build-script probe repro:

    - Upstream Cargo.toml at `<tmp>/Cargo.toml`:
      ```toml
      [package]
      name = "anyhow-like"
      version = "1.0.0"
      edition = "2021"
      build = "build.rs"

      [lib]
      path = "src/lib.rs"
      ```
    - `build.rs` at `<tmp>/build.rs` that mirrors `anyhow build.rs:255-257` + `:323-367` probe pattern:
      ```rust
      fn main() {
          // Mirrors anyhow probe: compile src/nightly.rs from cwd to detect nightly.
          // The probe compiles via proc_macro from cwd; we simulate the file-access
          // part here by checking the path is accessible.
          let probe = std::path::Path::new("src").join("nightly.rs");
          // Emit a custom cfg based on whether the probe file exists (the real anyhow
          // build.rs does a compile-probe; we test the file-resolution step).
          if probe.exists() {
              println!("cargo:rustc-cfg=probe_file_found");
          } else {
              // Silent-false failure mode: no error, but wrong cfg.
              println!("cargo:rustc-cfg=probe_file_missing");
          }
          println!("cargo:rerun-if-changed=src/nightly.rs");
      }
      ```
    - Probe file at `<tmp>/src/nightly.rs`: `// nightly probe stub`.
    - Minimal Rust source at `<tmp>/src/lib.rs`:
      ```rust
      #[cfg(probe_file_found)]
      pub fn probe_found() {}
      #[cfg(probe_file_missing)]
      compile_error!("probe_file_missing: staged-mirror did not provide src/nightly.rs");
      ```
    - Invoke `materialize_overlay(<tmp>/Cargo.toml)`. Verify `<staged-overlay>/src/nightly.rs` is accessible (via symlink or copy) — staged-mirror verification.
    - Invoke `cargo build --manifest-path <staged>`. Assert exit 0. The `compile_error!` macro in `src/lib.rs` fires if the cfg is `probe_file_missing` → the test FAILS pre-fix (without the staged-mirror strategy, the probe file is not accessible from `<staged-overlay>/`) and PASSES post-fix (the mirror provides `src/nightly.rs` via symlink → `probe.exists()` returns true → `probe_file_found` cfg is emitted).
    - **This test covers the M.5 class (anyhow-shape silent-false) as a HARD failure** by turning the silent-false into a compile error via the `cfg`-gated `compile_error!`. The absence of an error message in the build output is not a reliable signal; the `compile_error!` makes the failure loud.

11. **Cargo-build-gated test: `cargo_build_thiserror_shape_probe_file_resolves_via_mirror`** — **M.6 closure: staged-mirror fixes thiserror-shape silent-false probe.** Same pattern as §5.2.10 but for the `build/probe.rs` path form:

    - Upstream Cargo.toml at `<tmp>/Cargo.toml`:
      ```toml
      [package]
      name = "thiserror-like"
      version = "1.0.0"
      edition = "2021"
      build = "build.rs"

      [lib]
      path = "src/lib.rs"
      ```
    - `build.rs` at `<tmp>/build.rs` that mirrors `thiserror build.rs:261-263` + `:328-371` probe pattern:
      ```rust
      fn main() {
          let probe = std::path::Path::new("build").join("probe.rs");
          if probe.exists() {
              println!("cargo:rustc-cfg=probe_file_found");
          } else {
              println!("cargo:rustc-cfg=probe_file_missing");
          }
          println!("cargo:rerun-if-changed=build/probe.rs");
      }
      ```
    - Probe file at `<tmp>/build/probe.rs`: `// thiserror build probe stub`.
    - Minimal Rust source at `<tmp>/src/lib.rs`:
      ```rust
      #[cfg(probe_file_found)]
      pub fn probe_found() {}
      #[cfg(probe_file_missing)]
      compile_error!("probe_file_missing: staged-mirror did not provide build/probe.rs");
      ```
    - Invoke `materialize_overlay(<tmp>/Cargo.toml)`. Verify `<staged-overlay>/build/probe.rs` is accessible.
    - Invoke `cargo build --manifest-path <staged>`. Assert exit 0 (same pre-fix FAIL / post-fix PASS logic as §5.2.10). **This test pins the `build/` path form** (distinct from anyhow's `src/` path form) as a separate class verification. If the mirror implementation special-cases `src/` but forgets `build/`, this test surfaces the gap.

12. **`patch_crates_io_self_injection_absolute_path_only`** — unit-style integration test asserting the materialized overlay's `[patch.crates-io.<self>].path` value is absolute (starts with `/` on unix, drive letter on windows) AND ends with `/target/lihaaf-overlay` (the staged-overlay-dir tail). Applies to both Rule 1 (INJECT) and Rule 2 (REMAP) — both rules emit the same byte shape.

13. **Optional / scope-out: `cargo_accepts_git_dependency_branch_in_patched_graph` (TER-5 coverage).** R4 explicitly addresses Codex R3's TER-5 finding (no git-dep coverage in §5). Options:

   - **(a) Implement.** Add a cargo-build-gated test where the upstream's workspace member declares `[dependencies] some-other-crate = { git = "https://example.com/some-other-crate" }`, and the test verifies that the patched-self-patch graph correctly resolves cargo's git-dep resolution alongside the patched-registry-name resolution. This requires CI access to a known-stable git URL.
   - **(b) Scope-out.** R4 explicitly scopes this out for v0.1.0: no cxx-shape pilot in the current corpus uses git deps as a transitive registry-name reference; the patched-self-patch policy is orthogonal to git-dep resolution (cargo resolves git deps independently from registry-name deps under `[patch.crates-io]`). Defer behind v0.2/v1.1 follow-up issue if a real pilot surfaces a git-dep edge case.

   **R4 chooses option (b) scope-out.** Rationale: (1) no current pilot in the lihaaf corpus exhibits a real git-dep failure shape under the patched topology; (2) implementing option (a) requires CI-stable git URLs which add maintenance burden disproportionate to the v0.1.0 / v1.0.0 scope; (3) Codex R3 TER-5 explicitly allowed scope-out as an acceptable resolution provided the rationale is documented. R4 documents the rationale in §6.13 and §13.

### 5.3 Backward-compat re-verification tests

1. **`cargo_accepts_workspace_inheritance_reference_in_overlay`** (existing, line 2030 in `overlay_determinism.rs`) — must still pass post-fix. The self-patch policy runs alongside R2's inheritance preservation; neither should interfere.

2. **`cargo_accepts_workspace_style_overlay_for_dylib_build`** (existing, line 1846) — must still pass post-fix. Verifies anyhow/thiserror don't regress under Rule 1 INJECT.

3. **`cargo_accepts_rich_overlay_for_dylib_build`** (existing, line 1696) — must still pass post-fix. **R4 note:** R3 incorrectly relied on this test as the SEC-6 cycle-acceptance proof; R4 replaces that proof-role with §5.2.9 `cargo_accepts_root_to_test_suite_to_root_topology`. The existing test continues to pass under R4 Rule 2 REMAP — the `rich-demo = { path = "." }` shape's overlay output changes from R3's PRESERVED `<upstream>/.` to R4's REMAPPED `<staged-overlay-dir>`, but the cargo-build assertion is identical (build success). The corpus fixture `with_patch_section.expected.toml` is updated to reflect the REMAP per §5.2.3.

4. **`byte_identical_across_two_lihaaf_binaries_on_corpus`** (existing, line 432, the canonical corpus-determinism test) — must still pass after the corpus expected files are updated AND the `names` array is bumped to 8 entries (R4 — TWO new fixtures: `with_self_patch_injected` for Rule 1, `with_self_patch_remapped` for Rule 2) AND the expected-count assertion is bumped to 8.

### 5.4 Determinism check

`byte_identical_across_two_lihaaf_binaries_on_corpus` (per §5.3.4) — must still pass after the corpus expected files are updated. This is the byte-stable assertion: every overlay's `[patch.crates-io.<self>]` line is invariant across runs on the same upstream-dir, regardless of which Option H rule fired.

The R5 emission policy (§4.1.2) preserves the absolutized form, identical to the existing absolutization scheme (production `absolutize_patch_paths` at overlay.rs:1393 + :1402). Both Rule 1 INJECT and Rule 2 REMAP emit the same byte shape — the staged-overlay-dir absolutized path. Rule 3 CONTINUE-ABSOLUTIZE uses the existing absolutization scheme unchanged. Rule 4 REJECT does not emit; the test surface verifies the materializer fails fast with the structured error.

---

## 6. Edge cases

### 6.1 Upstream already has `[patch.crates-io.<self>]` — Option H decision table (R3 SEC-5 / R4 SEC-7 closure)

**Background.** R3's DETECT-AND-PRESERVE-PER-KEY policy had a subtle correctness bug surfaced by Codex R3 SEC-7: the PRESERVE-AS-IS branch claimed to copy the upstream's `[patch.crates-io.<self>] = { path = "." }` verbatim into the staged overlay, relying on the `absolutize_patch_paths` pass to rewrite the `.` to `<upstream>/.` before emission. Codex pointed out that cargo anchors `[patch.crates-io.X].path` relative to the manifest declaring the patch — so the `<upstream>/.` ABSOLUTE form in the overlay manifest is a competing source-id from the overlay's perspective, NOT the same source-id as the overlay's `[package]`. The R3 PRESERVE-AS-IS branch worked for cxx ONLY because cargo re-anchored `.` literally if `absolutize_patch_paths` happened to preserve `.` (R3's historical claim referenced overlay.rs:2431-2439, but the current `absolutize_patch_paths` is at overlay.rs:1393-1402 — the R3-cited line range does not exist in the current codebase; this paragraph documents R3's reasoning at the time, not current code), which is implementation-coincidence rather than designed correctness. The general case (e.g. `path = "../my-fork"`) would silently break under R3 PRESERVE-AS-IS.

R4 replaces R3's per-key PRESERVE with **Option H: intent-aware self-patch policy (4 rules)**. See §2.6 for the cargo-anchoring analysis that drove the shift; §3.1 for the rule definitions; this section for the adopter-facing decision table.

**Decision table (Option H 4-rule).** R4 evaluates the upstream's `top["patch"]["crates-io"][<self>]` entry (where `<self>` = `upstream_crate_name` from `[package].name`) and chooses one of four rules:

| Rule | Upstream state | R4 action | Rationale |
|---|---|---|---|
| **Rule 1 (INJECT)** | No `[patch.crates-io.<self>]` entry exists | Insert `{ path = "<absolutized staged-overlay-dir>" }` | Standard new-injection path. Resolves resolution-time ambiguity for clean upstreams (anyhow / thiserror / serde-json / clean Round-2 pilots). |
| **Rule 2 (REMAP)** | `[patch.crates-io.<self>] = { path = "..." }` exists AND no `git`/`branch`/`tag`/`rev` keys AND the joined-and-normalized path equals the upstream root crate | Overwrite the entry: emit `{ path = "<absolutized staged-overlay-dir>" }` | Upstream's intent is "self-patch to root". Translated to the overlay's context, the equivalent intent is "self-patch to the staged-overlay root". R4 emits the absolutized staged-overlay-dir form for determinism and future-proofing per §2.6. cxx's `path = "."` resolves to upstream root → Rule 2 fires. |
| **Rule 3 (CONTINUE-ABSOLUTIZE)** | `[patch.crates-io.<X>]` exists for SOME `<X>` ≠ `<self>` (sibling-crate patches) | No action by `apply_self_patch_policy` — the existing `absolutize_patch_paths` pass (overlay.rs:1383-1410) handles the entry as before | These are not self-patches against the upstream root; they are adjacent patches against sibling crates. The R3 absolutization scheme is correct for these. cxx's `[patch.crates-io.cxx-build] = { path = "gen/build" }` falls under Rule 3 alongside Rule 2's handling of the `cxx` self-patch. |
| **Rule 4 (REJECT)** | `[patch.crates-io.<self>]` exists but the target is external: (a) `path` resolves to a non-root dir (vendored fork), OR (b) has `git`/`branch`/`tag`/`rev` keys (git source / registry override), OR (c) both `path` and `git`/etc | Return `Err(Error::CompatPatchOverrideConflict { .. })` with structured error referencing the v0.2/v1.1 escape hatch | Adopter has explicitly overridden the registry-name with a non-root, non-staged-overlay source. lihaaf v0.1.0 / v1.0.0 must NOT silently overwrite this. Conservative path is REJECT; escape hatch (`--compat-allow-patch-override`) deferred to v0.2/v1.1. |

**Cases covered by tests:**
- Rule 1 (INJECT): tests 5.1.1, 5.1.3, 5.1.10 (orthogonal-key variant), 5.1.11-13, 5.2.1, 5.2.3-4, 5.2.5, 5.2.9 (R8 rescope: workspace-member-registry-dep-via-self-patch cargo-graph proof)
- Rule 2 (REMAP): tests 5.1.5, 5.1.6 (`path = "./"` variant), 5.2.2 (corpus fixture), 5.2.3 (`with_patch_section` fixture update), 5.2.6 (cxx-shape cargo-build proof)
- Rule 3 (CONTINUE-ABSOLUTIZE): test 5.2.7 (cxx-build-shape cargo-build proof)
- Rule 4 (REJECT): tests 5.1.7-9, 5.2.8 (cargo-build-gated REJECT proof)

**Why REMAP over PRESERVE-AS-IS (Option H Rule 2 design choice).** §2.6 covers this in full. Summary: PRESERVE-AS-IS works for cxx's `path = "."` because cargo re-anchors `.` relative to the staged overlay manifest, which happens to give the right source-id. The general case (any non-`.` upstream path) would break under PRESERVE-AS-IS. REMAP unifies the emission form across all path-bearing self-patches → deterministic, robust to future cargo / absolutization-policy changes.

**Specific subcase: upstream carries `[patch.crates-io.<self>] = { path = "." }` (the cxx case).** Rule 2 fires. `upstream_dir.join(".")` lexical-normalizes to upstream root → Rule 2 detection matches. R4 overwrites the entry: emits `{ path = "<absolutized staged-overlay-dir>" }` in the overlay. cargo accepts the topology (proven by the new §5.2.6 cargo-build test, NOT relying on the misleading R3 citation of `cargo_accepts_rich_overlay_for_dylib_build`). The `links = "cxxbridge1"` collision cannot fire because the resolved graph contains exactly one source for `cxx` (the staged-overlay path). ✓

**Compat-mode policy boundary (R4 reformulation).** The Option H policy honors adopter intent for v0.1.0 / v1.0.0 by REJECTING Rule 4 cases (vendored forks / git sources) rather than silently overwriting them. Rule 1 / Rule 2 are universally applied to clean upstreams and upstream-root self-patches respectively. Future `--compat-allow-patch-override` flag (v0.2/v1.1) is a CONSIDERATION for the rare Rule 4 case where the operator explicitly wants lihaaf to overwrite the adopter's intent — but it is no longer a v0.1.0 blocker for cxx resolution (cxx is handled by Rule 2 REMAP, not Rule 4). Documented in the module-level docs, `docs/compatibility-plan.md` §3.2.3, and the CHANGELOG entry.

### 6.2 `[package].name` differs from registry resolution name (renames)

The cargo book defines `[dependencies] foo = { package = "bar", version = "1" }` — the dependency is named `foo` in the dependent's source but resolves to crate `bar` on the registry. For our purposes, what matters is what `[package].name` declares in the upstream — that IS what cargo looks up on the registry and what `[patch.crates-io.<X>]` keys against. So `apply_self_patch_policy` keys off `upstream_crate_name` (`[package].name`) exactly, and this case is correctly handled: the self-patch resolves the registry-side name regardless of any rename pseudonyms downstream code uses.

### 6.3 Workspace with multiple sub-packages, each declaring `links`

Out of scope. The R3 ancestor-workspace-walk rejection (overlay.rs:741) prevents lihaaf from materializing an overlay whose path is not the workspace root. The case we are fixing is: workspace ROOT has `[package]` + `[workspace] members = [...]` + workspace member depends on the root-package by name. Multi-`links` is a hypothetical that requires the OVERLAY to be a workspace member, which is rejected upstream.

### 6.4 Standalone single-crate (no transitive registry-version path)

The injection still fires (unconditional on `[package].name` presence). With no workspace member referencing the upstream by name, the patch is benign — cargo evaluates `[patch.crates-io.<X>]` only when something depends on `<X>` from crates.io. anyhow falls in this bucket: the patch will be in the overlay but never triggered, and cargo doesn't warn about unused patches in the cargo version range lihaaf supports.

**Determinism implication:** the corpus tests must accept the injected line even for the bare/standalone case. See §5.2 corpus updates.

### 6.5 Path-only self-reference forms (R2 BLOCK-1 + FIX-6; R4 Rule 1 + Rule 2 emission form)

**Question:** `path = "."` vs `path = "./"` vs absolute path — what's the correct emission form for the Rule 1 (INJECT) and Rule 2 (REMAP) entries?

**Answer:** absolute path pointing at the STAGED OVERLAY DIR, computed from `<upstream-dir>/target/lihaaf-overlay/` at policy-apply time. Same form for both Rule 1 and Rule 2 — the emission is unified per §4.1.2. Three reasons:

1. **Self-loop avoidance (BLOCK-1).** Pointing at `<upstream-dir>` (or `"."` which absolutizes to it) IS the upstream source-id cargo already aliases to crates.io — the patch is a no-op self-loop. Pointing at `<upstream-dir>/target/lihaaf-overlay/` is a DIFFERENT source-id (the staged overlay's manifest dir), which is the actual redirect we need.

2. **R3 absolutization interaction + R4 cargo anchoring.** Per §2.6, cargo anchors `[patch.crates-io.X].path` relative to the manifest declaring the patch (= the staged overlay manifest). Emitting an absolute path bypasses cargo's anchoring entirely → robust against future cargo / `absolutize_patch_paths` policy changes. If we emitted `path = "."`, cargo would re-anchor to the staged overlay dir at READ time, which happens to give the correct source-id today but depends on coincidental implementation details. The absolute form is unambiguous.

3. **Determinism (FIX-6).** An absolute path baked into the overlay bytes preserves the existing corpus golden tests' byte-stability shape. The emitted form is `__UPSTREAM_DIR__/target/lihaaf-overlay` (forward-slash, no trailing `.` or `/`), consistent with the existing scheme. The lexical normalizer (§4.1.1) is used only inside Rule 2's detection logic, not for emission.

### 6.6 Standalone-single-crate without `[package]` (test fixture / partial manifest)

`upstream_crate_name` is `None` → `apply_self_patch_policy` returns `Ok(())` early. Verified by test 5.1.2. R4 unchanged from R3 on this case.

### 6.7 Pilots without crates.io presence

**Question:** What if the upstream's `[package].name` doesn't exist on crates.io yet (unpublished, alpha, or pre-release)?

**Answer:** `[patch.crates-io.<X>]` is still valid TOML; cargo accepts a patch for a crate that has no transitive reference. The patch is a no-op for unpublished crates and harmless. No special-case needed.

### 6.8 Git-source upstream (no version pin)

Out of scope. Compat-mode invokes against an unpacked source directory (`--compat-root <dir>`); it does not interact with git sources. If a fork pulled the upstream from git and unpacked it, the upstream's `[package]` table still declares a name and that's what we use.

### 6.9 `[patch]` table absent in upstream

Handled in §4.1 step 3-4: `top["patch"]` and `top["patch"]["crates-io"]` are created on demand. The canonical-key-order serializer (overlay.rs:1556-1574) places `patch` at slot 11 in the canonical order, so the injected table appears in the deterministic position.

### 6.10 Workspace-member path-dep edge missing (R2 BLOCK-3 context)

The `[workspace] members = [...]` array is stripped during materialization (overlay.rs:812-829, `WORKSPACE_MEMBERSHIP_KEYS = ["members", "exclude", "default-members"]`). After stripping, a workspace member is NOT in the overlay's resolved dependency graph UNLESS it's also referenced via path-dep from the root package (`[dependencies] <member> = { path = "<member-dir>" }` or `[dev-dependencies]` equivalent).

This means the actual cxx / serde-json failure shapes — and the synthetic repros in §5.2 — require BOTH the `members = [...]` declaration AND a root path-dep to bring the test-suite into the resolved graph. The cxx pilot has exactly this shape (`cxx-test-suite` is a member AND referenced from the root). Without the root path-dep, the synthetic repro would NOT reproduce the failure, AND would PASS even pre-fix (vacuous coverage).

The §5.2 cargo-build-gated tests 5 / 6 / 9 are constructed to include this edge explicitly (the root path-dep to `test-suite` in the Rule 1 INJECT, Rule 2 REMAP, and SEC-8 cargo-graph proof tests).

### 6.11 Symlinked upstream paths (R3 BLOCK-2 finish — known limitation; R4 — falls to Rule 4 REJECT)

The §4.1.1 lexical normalizer does NOT call `canonicalize()` / `read_link()`. Two paths that point to the same canonical filesystem location via symlinks compare UNEQUAL at the lexical layer.

**Practical impact for R4 Option H Rule 2 detection.** If the operator passes `--compat-root /real-path/to/upstream` and the upstream's pre-existing `[patch.crates-io.<self>] = { path = "/symlinked-path/to/upstream" }` resolves to the same filesystem location via symlink, Rule 2's detection (`upstream_dir.join("/symlinked-path") = /symlinked-path` lexical-normalize ≠ `/real-path/to/upstream` lexical-normalize) sees them as different paths → Rule 2 does NOT fire. The entry has `.path` but the joined-and-normalized path doesn't match upstream root → falls to Rule 4 REJECT.

**R4 known limitation.** Symlinked upstream paths in adopter manifests can trigger Rule 4 REJECT even when the adopter's intent matches Rule 2's "self-patch to root". The escape hatch (`--compat-allow-patch-override` v0.2/v1.1) is one resolution; the simpler workaround is for the adopter to resolve all paths through the same form.

**Workaround for adopters:** if your upstream's `[patch.crates-io.<self>]` uses a symlinked path that should match Rule 2 (self-patch to upstream root), use the SAME form in `--compat-root` and in the upstream patch entry. Either both real or both symlinked — consistent form makes Rule 2 detection match.

**Documented in:** `docs/compatibility-plan.md` §3.2.3 (adopter-facing); module-level rustdoc on `apply_self_patch_policy`; test `lexical_path_normalize_does_not_resolve_symlinks` (§5.1.13).

**Future v0.2/v1.1 mitigation.** A symlink-aware Rule 2 detector (calling `canonicalize()` on both sides) would close this case automatically, but adds I/O dependency to the policy-apply step. R4 chooses lexical normalization for v0.1.0 / v1.0.0; v0.2/v1.1 may revisit. The `--compat-allow-patch-override` flag is a separate concern (for vendored forks); a symlink-aware Rule 2 detector is the cleaner path to closing this specific limitation.

### 6.12 Repeated path separators (R3 BLOCK-2 finish, unchanged in R4)

`Path::components()` collapses repeated separators (`//` or more) on Unix; the lexical normalizer naturally handles this case. cargo's path-source resolution applies the same `Path::new(s).components()` traversal, so `path = "/foo//bar"` and `path = "/foo/bar"` resolve to the same source-id. R4 lexical normalization preserves this equivalence (used in Rule 2 detection). Test `lexical_path_normalize_handles_repeated_separators` (§5.1.12) pins this.

### 6.13 Git-dependency edge in patched graph (R4 TER-5 scope-out)

**Codex R3 TER-5 finding.** R3 did not include any test verifying that the patched-self-patch graph correctly interacts with git-dependency resolution. If a workspace member declares `[dependencies] some-other-crate = { git = "https://example.com/some-other-crate" }`, would the patched `[patch.crates-io.<self>]` interfere with cargo's git-dep resolution?

**R4 analysis.** Under Option H, the patched `[patch.crates-io.<self>]` is keyed against the upstream root crate's registry-name; it does NOT touch git-dep resolution. Cargo resolves git deps independently from registry-name deps under `[patch.crates-io]`. The two resolution paths are orthogonal at the resolver level.

**R4 scope-out decision (Codex R3 TER-5 closure).** R4 explicitly scopes out the git-dependency edge for v0.1.0 / v1.0.0:

1. **No real pilot in the lihaaf corpus exhibits a git-dep failure shape under the patched topology.** The four current pilots (cxx / serde-json / anyhow / thiserror) and the planned Round-2 pilots (derive_more / axum-macros) all use registry-name workspace deps; none surface git-dep edges that would interact with the self-patch.

2. **Implementing the test requires CI-stable git URLs.** Adding a synthetic git-dep test means either (a) depending on a public git URL (e.g. github.com/some-crate) which adds external-dependency fragility to CI, or (b) spinning up a local git server in the test harness, which adds significant maintenance burden disproportionate to the v0.1.0 / v1.0.0 scope.

3. **Codex R3 TER-5 explicitly allowed scope-out as an acceptable resolution.** The dispatch text says "if no real cxx-shape needs git deps in practice, scope it out explicitly in §6 with a 'not in v0.1.0, file follow-up issue if needed' note. Codex's TER-5 finding allowed scope-out as an acceptable resolution."

**Follow-up issue.** The implementer files a v0.2/v1.1 follow-up issue at implementation time: "Add cargo-build-gated test for git-dependency edge in patched-self-patch graph." Implementer references this issue # in the §6.13 note and the §13 open-items list.

**Test row in §5.2 / §11 audit.** The git-dep test (`cargo_accepts_git_dependency_branch_in_patched_graph`) is listed as OPTIONAL / SCOPED-OUT in §5.2 item 11 with explicit rationale and the follow-up issue # reference.

---

## 7. Alternatives considered + why rejected (R4 update)

Already covered in §2 / §3. Recap with explicit rejection reasons:

- **Strategy 1 (R1, upstream-dir target):** **BROKEN per Codex R1 BLOCK-1.** The R1 plan aimed the patch at the upstream dir, which is the same source-id cargo aliases to crates.io. The result is a self-loop, not a redirect. **R2 corrected this to staged-overlay-dir.**
- **Strategy 1 (R2, REJECT-on-conflict + staged-overlay-dir target):** **BROKEN per Codex R2 SEC-5.** The REJECT policy would have blocked the cxx pilot (upstream's `[patch.crates-io.cxx] = { path = "." }` would have been rejected, contradicting the plan's stated goal of resolving cxx). **R3 corrects this to DETECT-AND-PRESERVE-PER-KEY.**
- **Strategy 1 (R3, DETECT-AND-PRESERVE-PER-KEY):** **BROKEN per Codex R3 SEC-7.** The PRESERVE-AS-IS branch relied on cargo's coincidental anchoring behavior for `path = "."` to work correctly; the general case (non-`.` upstream paths) would silently misroute. **R4 corrects this to Option H 4-rule decision tree (Rule 2 REMAP replaces PRESERVE-AS-IS for the upstream-root case; Rule 4 REJECT covers vendored-fork / git cases).**
- **Strategy 1 (R4, Option H 4-rule intent-aware policy):** **THE CHOSEN STRATEGY.** Verified by §2.6 cargo anchoring analysis + §3.1 rule definitions + §5.2 cargo-build-gated synthetic repros (incl. new §5.2.6 Rule 2 REMAP cxx-shape proof, §5.2.8 Rule 4 REJECT proof, §5.2.9 SEC-8 cargo-graph-acceptance dedicated proof).
- **Strategy 2 (workspace-member stripping):** invasive, false-clean risk by silently shrinking the baseline graph, additional I/O surface, defeats PR #37 R2's inheritance preservation. Rejected.
- **Strategy 3 (`-p <crate>` + filtered manifest synthesis):** equivalent to strategy 2 plus regresses PR #37 R2. Rejected.
- **Strategy 4 (`--frozen` + lockfile pre-population):** doesn't address `links` collision (resolved-package-level fire, independent of lockfile pinning); adds a second on-disk artifact with its own determinism contract. Rejected.
- **Strategy 5 (1+2 combined):** under corrected R4 Strategy 1, Strategy 1 alone resolves both failure shapes per §2.5 + §3.2; adding strategy 2 adds invasiveness without solving anything strategy 1 doesn't already solve. Rejected (re-evaluated under R4 corrected Strategy 1).

### 7.1 Future flag: `--compat-allow-patch-override` (v0.2 / v1.1) — NOT a v0.1.0 blocker (R3 TER-4 / R4 update)

R2's plan deferred this flag as a v0.2/v1.1 follow-up while relying on REJECT to handle conflicts. Codex R2 TER-4 correctly identified that deferring the flag was UNACCEPTABLE if cxx resolution depended on it.

**R3 resolution (now superseded by R4):** R3's DETECT-AND-PRESERVE policy did NOT need an override flag to resolve cxx — cxx fell under PRESERVE-PATH, which silently accepted the upstream's `path = "."`. But R3 had a separate correctness bug (SEC-7) that R4 fixed; R3 PRESERVE-PATH is gone.

**R4 resolution.** R4's Option H 4-rule policy handles cxx via Rule 2 REMAP, NOT a PRESERVE-AS-IS or REJECT path. The escape hatch (`--compat-allow-patch-override`) is now scoped ONLY to Rule 4 (REJECT cases — vendored forks, git sources, non-root path targets). No current pilot in the lihaaf corpus surfaces a Rule 4 case; the escape hatch is a v0.2/v1.1 NICE-TO-HAVE for unanticipated adopter manifests.

**Future scope for v0.2/v1.1.** A `--compat-allow-patch-override` flag would let an operator opt into "OVERWRITE the upstream's existing `[patch.crates-io.<self>]` with lihaaf's preferred injection" for Rule 4 cases. This is a CONSIDERATION for the rare case where the operator owns the upstream and explicitly wants lihaaf's preferred staged-overlay-dir target, OR where the upstream's Rule 4 case prevents cargo resolution and the operator wants explicit override. NOT in scope for v0.1.0 / v1.0.0.

**Net effect of R3 TER-4 closure (R4 reaffirmation):** the only "future flag" R4 documents is a v0.2/v1.1 NICE-TO-HAVE scoped to Rule 4, not a v0.1.0 LOAD-BEARING DEFERRAL. The plan no longer has the structural contradiction R2 carried, and R3's SEC-7 reasoning bug is closed.

---

## 8. Risks / unknowns

### 8.1 `[patch.crates-io]` for unpublished / git-only / alpha crates

Mitigated by §6.7: a patch for a non-existent registry crate is a no-op. Verified by reading cargo source behavior — `[patch]` only applies if a dep tree references the patched package; an unreferenced patch produces a warning at most in newer cargos, not an error. Risk level: **low**.

### 8.2 Upstream `Cargo.lock` pre-resolves versions that conflict with the patch

`[patch.crates-io]` overrides `Cargo.lock` for the patched key on a fresh resolve. The compat driver does not pre-populate the lockfile (Strategy 4 was rejected); cargo handles the patch correctly via its standard resolution flow. Risk level: **low**.

### 8.3 Implementation risk: 5th-round touch of overlay.rs, now with new I/O surface

overlay.rs has been edited 4 times during PR #37 R1-R4 (per [[lihaaf-workspace-identity-bug]]). R5 adds two new functions (`apply_self_patch_policy` and `mirror_upstream_into_overlay`) plus their call-sites, increasing the regression surface. Mitigations:

- Plan-first with adversarial review (this plan → Codex R1 → R2 → R3 → R4 → R5 → Codex R5 review pass).
- Test surface is BIG (15 unit tests + 13 integration tests + 8 corpus updates; R5 adds §5.2.10-11 probe-file tests; R6 adds §5.1.4 extended + §5.1.14-15 reconciliation tests) so regression detection is high.
- Both new functions are additive — no existing function is rewritten; `absolutize_patch_paths` is untouched (Option H Rule 3 leaves it as-is); `write_file_atomic` is not changed (mirror runs after it).
- All existing tests (including the 5-branch decision tree from R1-R5) re-verify in the test plan §5.3.

Risk level: **medium**. The `mirror_upstream_into_overlay` function introduces new I/O logic (symlink + copy fallback) that adds cross-platform risk (Windows symlink privilege). Mitigated by the §4.5.3 copy-fallback spec and the dispatch-required tests (§5.2.10-11 probe-file tests catch mirror correctness at the package-root-file-access level).

### 8.4 Backward-compat with `derive_more` / `axum-macros` (Round-2 pilots not yet enrolled)

The R5 fix is universal (applies to every overlay) under the Option H 4-rule policy + staged-mirror strategy. derive_more is a workspace (Class C per §3.2 — build.rs no package-root read); axum-macros is a workspace (Class D — `build = false`). Both will go through the §6.1 decision-table: clean upstreams → Rule 1 INJECT; upstream-root self-patches → Rule 2 REMAP; non-target sibling patches → Rule 3 CONTINUE-ABSOLUTIZE; vendored-fork / git sources → Rule 4 REJECT. The fix is expected to be helpful or neutral for Round-2 pilots when they fall into Rules 1-3; if either pilot exhibits a Rule 4 case (vendored fork), the REJECT will surface clearly with the v0.2/v1.1 escape-hatch reference rather than silently misrouting. The staged-mirror strategy is benign for both Class C and Class D pilots — no file access to redirect. Risk: **low to neutral**.

### 8.5 Cargo version incompatibilities

The `[patch.crates-io]` mechanism is stable since cargo 1.21 (Aug 2017). lihaaf's MSRV is well above that. Risk: **none**.

### 8.6 Implicit dependency on path-form `[patch]` behavior

Cargo's `[patch.crates-io.<X>] = { path = "..." }` semantics: when cargo resolves a dependency on `<X>` from crates.io, it transparently swaps in the path source. The version that the path source declares (overlay's `[package].version`) must satisfy any version requirements expressed in dependents. For pilot crates where workspace members declare `[dependencies] <X> = "1.0"`, the upstream's `[package].version = "1.0.X"` will satisfy. Risk: **low** — pilot crates pin their workspace member dep versions to match the workspace package version, by convention.

### 8.7 Staged-overlay-dir nonexistence at serialize time

`apply_self_patch_policy` runs BEFORE `write_file_atomic` writes the staged manifest (which creates the parent dir via `create_dir_all`). At serialize time, the staged-overlay-dir does not yet exist. This is fine: `[patch.crates-io.<X>] = { path = "..." }` does NOT require the path to exist at TOML-serialize time; cargo resolves the patch only when `cargo rustc --manifest-path <staged>` runs, at which point `write_file_atomic` has already created the directory. Risk: **low** — the contract is "the path must exist when cargo resolves," not "the path must exist when lihaaf writes the manifest."

### 8.8 R4 lexical-normalization correctness in Rule 2 detection

The §4.1.1 normalizer drops `Component::CurDir` (`.`) and preserves everything else, including `..`. R4 Rule 2 detection joins the upstream's patch `.path` value against `upstream_dir`, lexically normalizes both, and compares for equality. The cxx case (`path = "."`) joins to `<upstream>/.` which lexical-normalizes to `<upstream>` = upstream root → match → Rule 2 fires. The vendored-fork case (`path = "../my-fork"`) joins to `<upstream>/../my-fork` which lexical-normalizes to `[<upstream-components>, ParentDir, Normal("my-fork")]` ≠ upstream root → Rule 2 does NOT fire → falls to Rule 4 REJECT. The unit-test pin in §5.1.11-13 covers the normalizer's behavior explicitly. Risk: **low** — the normalization is intentionally simple and the test surface is comprehensive (3 dedicated tests).

### 8.9 R4 — cargo `[patch.crates-io].path` anchoring behavior (Codex R3 SEC-7 root cause)

The R3 → R4 shift is driven by cargo's behavior: `[patch.crates-io.X].path` is anchored relative to the manifest declaring the patch (= staged overlay manifest), NOT relative to the manifest the path came from (= upstream manifest). See §2.6 for the full analysis. The R4 Rule 2 REMAP emission form (absolute staged-overlay-dir) bypasses cargo's anchoring entirely, making the policy robust to future cargo changes in path-anchoring behavior. Risk: **low** — the emission form is unambiguous; only the detection logic depends on lexical normalization of joined paths (covered by §8.8 risk).

---

## 9. Backward compatibility / regression risk

### 9.1 Does anyhow keep working?

Yes. anyhow's overlay receives `[patch.crates-io.anyhow] = { path = "<staged-overlay-dir>" }` under Rule 1 INJECT. No workspace member references `anyhow` by name → no transitive registry-version path → patch is benign. Verified by re-running `cargo_accepts_workspace_style_overlay_for_dylib_build` test (5.3.2).

**R5 addition (M.5 closure).** The staged-mirror strategy (§4.5) also ensures anyhow's `build.rs` silent-false probe hazard is eliminated: `src/nightly.rs` is accessible from the staged-overlay dir via symlink → the probe returns the correct result. Verified by the new §5.2.10 test.

### 9.2 Does thiserror keep working?

Yes. thiserror's overlay receives `[patch.crates-io.thiserror] = { path = "<staged-overlay-dir>" }` under Rule 1 INJECT. `thiserror-impl` member depends on thiserror only by path, not by registry name → patch is benign. Verified by re-running the existing pilot regression tests.

**R5 addition (M.6 closure).** The staged-mirror strategy (§4.5) ensures thiserror's `build.rs` silent-false probe hazard is eliminated: `build/probe.rs` is accessible from the staged-overlay dir via symlink → the probe returns the correct result. Verified by the new §5.2.11 test.

### 9.3 Do PR #37 R1-R5 test surfaces (the 5-branch decision tree + ancestor-walk + inheritance preservation) regress?

No. The self-patch policy runs AFTER `absolutize_path_bearing_keys` and BEFORE `override_workspace_inheritance`. The `mirror_upstream_into_overlay` function runs after `write_file_atomic` and does not touch the manifest content. The workspace-inheritance branch logic (R2-R5) does not read `[patch]`; the self-patch policy does not read `[workspace]`; the mirror function does not touch either. Orthogonal modules, verified by the unchanged tests 5.3.1 / 5.3.2 / 5.3.3.

### 9.4 Cross-binary determinism (toml 1.x patch upgrades)

The overlay corpus test (`byte_identical_across_two_lihaaf_binaries_on_corpus`) catches `toml` 1.x patch-upgrade drift via the corpus fixtures. Adding both Rule 1 and Rule 2 emission lines to the relevant fixtures preserves this contract — the test fails on drift exactly as it did before. Verified by §5.4.

### 9.5 Corpus-list test list and count (R5 — bumped to 8)

The corpus-list test at `tests/compat/overlay_determinism.rs:453-460` and `:495-498` hardcodes the `names` array AND the expected count. R5 adds TWO new fixtures:
- `with_self_patch_injected` (Rule 1 INJECT)
- `with_self_patch_remapped` (Rule 2 REMAP)

The expected count is bumped from 6 to 8. The `with_patch_section` fixture's expected file is ALSO updated to reflect Rule 2 REMAP (was R3 PRESERVE-AS-IS form `<upstream>/.`; now R5 REMAP form `<staged-overlay-dir>`). All three updates are mechanical and pin the fixtures to CI. Without these updates the new fixtures would be silently skipped. Verified by §5.2.4.

### 9.6 R3 → R5 corpus behavioral shift on `with_patch_section`

The existing `with_patch_section` fixture's input declares `[patch.crates-io.<crate>] = { path = "." }` (matching cxx-shape). Under R3 PRESERVE-PATH, the expected output would have been `<upstream>/.` (absolutized form). Under R5 Rule 2 REMAP (same as R4), the expected output is `<absolutized staged-overlay-dir>` (the unified Rule 1 / Rule 2 emission form). This is a corpus-level breaking change vs R3 expectations — the fixture's `.expected.toml` is updated in §5.2.3.

**Backward-compat impact for existing test surfaces.** R3's PR was never merged (the plan is on the `feat/compat-mode-beta-4` branch); the R3-shape fixture only exists in plan-text, not in committed corpus state. R5's REMAP form is the first form actually committed to the corpus. No backward-compat impact on shipped lihaaf binaries.

---

## 10. Documentation updates (R4 — Option H rewrite)

Per the user's hard requirement, user-guide docs land in the SAME PR as the implementation. The implementer's checklist:

### 10.1 `README.md`

If `README.md` documents compat-mode behavior or `[patch]` handling, add a short note (1-2 sentences) about the new self-patch policy. Reference `docs/compatibility-plan.md` §3.2.3 for the full rationale. If the README does not currently surface compat-mode internals, no README change required — verify during implementation.

### 10.2 `docs/compatibility-plan.md` §3.2.3

The PRIMARY doc update. Add a new bullet after the existing `[patch.<registry>.X].path` line (currently at line 175 per R1 plan reference). Text covers the Option H 4-rule policy in adopter-facing terms:

- **Rule 1 (INJECT):** "If your upstream Cargo.toml has no `[patch.crates-io]` entry for the crate-under-test, lihaaf injects one pointing at the staged overlay directory (`<upstream>/target/lihaaf-overlay/`)."
- **Rule 2 (REMAP):** "If your upstream has `[patch.crates-io.<crate>] = { path = \".\" }` (or any path-form that resolves to the upstream root crate), lihaaf rewrites the entry to point at the staged overlay directory. cargo's resolver re-anchors `.` relative to the staged overlay manifest, but lihaaf emits the absolute form for determinism and clarity. The semantic intent (`patch crates-io.<crate> with the equivalent-to-root path-source`) is preserved."
- **Rule 3 (no action by self-patch policy):** "If your upstream has `[patch.crates-io.<other>]` entries for crates OTHER than the crate-under-test (e.g., cxx's `[patch.crates-io.cxx-build] = { path = \"gen/build\" }`), lihaaf preserves them untouched (the existing path-absolutization scheme applies)."
- **Rule 4 (REJECT):** "If your upstream has `[patch.crates-io.<crate>]` pointing somewhere else — a vendored fork, a git source, a non-root path — lihaaf rejects with a clear error. To opt into overwriting your upstream's intent with lihaaf's preferred staged-overlay-dir target, use the `--compat-allow-patch-override` flag (v0.2/v1.1, not yet available; see issue #X for tracking)."

Additionally include:
- The §2.6 cargo-anchoring analysis as an appendix or footnote, explaining why Rule 2 REMAP (NOT PRESERVE-AS-IS) is the correct choice.
- The §3.2.3 byte-determinism guarantee preservation (emission preserves the absolutized form per §4.1.2).
- The known limitation that symlinked paths compare lexically-unequal (§6.11) — adopters with symlinked compat-roots should use the same form (real or symlinked) in their `--compat-root` argument and in their upstream's `[patch.crates-io]` entries; otherwise Rule 2 may fall to Rule 4 REJECT.

### 10.3 `docs/spec/lihaaf-v0.1.md` (if applicable)

If a spec exists that pins compat-mode behavior at the spec level (parallel to `docs/compatibility-plan.md` but more contract-oriented), update it to mention the Option H self-patch policy. The implementer should grep for `\[patch.crates-io\]` or `compat-mode` and update wherever the existing absolutization is documented. If no such doc exists, skip — `docs/compatibility-plan.md` is authoritative.

### 10.4 `CHANGELOG.md`

Add a `### Fixed` entry under the next beta / v0.1.0 release section:

```markdown
### Fixed

- Compat-mode now applies an intent-aware self-patch policy to `[patch.crates-io.<overlay-package-name>]` in the staged overlay (Option H, 4 rules):
  - Rule 1 (INJECT): if your upstream Cargo.toml does not self-patch the package-under-test, lihaaf injects `[patch.crates-io.<overlay-package-name>] = { path = "<staged-overlay-dir>" }`. Resolves the previously-failing serde-json case (`ambiguous specification`) and the family-completeness equivalents on anyhow-shape pilots.
  - Rule 2 (REMAP): if your upstream self-patches the package-under-test to a path that resolves to the upstream root crate (cxx-style `path = "."`), lihaaf rewrites the entry to point at the staged overlay directory. Resolves the previously-failing cxx case (`links = "cxxbridge1"` collision).
  - Rule 3: non-target `[patch.crates-io.<X>]` entries are preserved untouched.
  - Rule 4 (REJECT): if your upstream self-patches the package-under-test to a non-root path (vendored fork) or to a git source, lihaaf rejects with a clear error. The escape hatch (`--compat-allow-patch-override`) is deferred to v0.2/v1.1; if you hit this case, file an issue.

  See `docs/compatibility-plan.md` §3.2.3 for the adopter-facing rule table.

- Compat-mode now creates a staged package-root mirror in the overlay directory. After writing the overlay `Cargo.toml`, lihaaf creates symlinks (or copies on platforms where symlinks are unavailable) for each top-level entry in the upstream package directory into the staged overlay dir. This ensures that `build.rs` scripts which read package-root-relative files via `CARGO_MANIFEST_DIR` (cxx: `src/cxx.cc`, `include/cxx.h`) or via cwd probes (anyhow: `src/nightly.rs`; thiserror: `build/probe.rs`) find the correct files during the overlay build. Upstream entries excluded from the mirror: `target/`, `.git/`, `Cargo.toml` (overlay-generated), `Cargo.lock`. Without this fix, cxx builds fail with a hard I/O error; anyhow and thiserror builds silently use incorrect cfg flags (silent-false probe pattern).

  Issues #40 and #47.
```

### 10.5 Module-level rustdoc on `src/compat/overlay.rs`

Per §4.3 doc-update 1, extend the existing `[patch]` policy bullet list (overlay.rs:60-72) with a third bullet about the Option H 4-rule self-patch policy. The text should be 6-10 sentences and cite the source-id self-loop avoidance reasoning AND the §2.6 cargo-anchoring analysis. Function-level rustdoc on the new `apply_self_patch_policy` should cite cargo's `[patch]` semantics, the §4.1.1 lexical normalization, and the 4-rule decision tree explicitly.

### 10.6 Inline source comment alongside the `apply_self_patch_policy` call-site

When wiring the call into `materialize_overlay_inner` (§4.2), add a 5-10 line comment block immediately above the call. The comment should cite:

- The two failure shapes the policy fixes (cxx links collision; serde-json ambiguous specification).
- The Option H 4-rule policy (Rule 1 INJECT / Rule 2 REMAP / Rule 3 CONTINUE-ABSOLUTIZE / Rule 4 REJECT; cite §6.1).
- The Rule 1 / Rule 2 emission form (staged-overlay-dir, NOT upstream-dir; cite the self-loop avoidance reasoning from §2.1 and the cargo-anchoring analysis from §2.6).
- The Rule 4 REJECT escape hatch (deferred to v0.2/v1.1 `--compat-allow-patch-override`).
- Issue references: `// See issues #40 and #47, plan-rev /tmp/lihaaf-issue-40-47-plan.md R5`.

### 10.7 Inline source comment alongside the `mirror_upstream_into_overlay` call-site

When wiring the call into the overlay write path (§4.5.5 — after `write_file_atomic`), add a 5-10 line comment block immediately above the call. The comment should cite:

- The materialization gap it closes: build scripts receive `CARGO_MANIFEST_DIR = <staged-overlay-dir>` and read package-root files that do not exist in an empty staged-overlay dir.
- The Class A pilots that require this (cxx hard-error: `src/cxx.cc`, `include/cxx.h`; anyhow silent-false: `src/nightly.rs`; thiserror silent-false: `build/probe.rs`).
- The exclusion list (`target/`, `.git/`, `Cargo.toml`, `Cargo.lock`).
- The symlink-first / copy-fallback strategy and where to find the fallback trigger logic (§4.5.3).
- Issue references: `// See issues #40 and #47, plan-rev /tmp/lihaaf-issue-40-47-plan.md R5`.

---

## 11. CI-first audit (R4 update — Option H 10-test surface)

Every test in §5 must run in CI. R4 confirms the following CI surface (verified by reading `.github/workflows/ci.yml`):

| Test category | File | Runs in CI? | Gate |
|---|---|---|---|
| Unit tests (§5.1, all 15) | `src/compat/overlay.rs::tests` | ✅ Yes | `cargo test --lib` standard path; no env gate. Covers Rules 1-4 detection + lexical normalizer corner cases + R6 mirror idempotency reconciliation (§5.1.4 extended, §5.1.14, §5.1.15). |
| Corpus determinism (§5.2.1-4, §5.3.4) | `tests/compat/overlay_determinism.rs` | ✅ Yes | `cargo test --test compat::overlay_determinism::byte_identical_across_two_lihaaf_binaries_on_corpus` standard path; no env gate. R5 includes BOTH new fixtures: `with_self_patch_injected` (Rule 1) and `with_self_patch_remapped` (Rule 2). Count bumped from 6 to 8. |
| Cargo-build-gated rule proofs (§5.2.5 INJECT, §5.2.6 REMAP+mirror-cxx, §5.2.7 CONTINUE-ABSOLUTIZE, §5.2.8 REJECT, §5.2.9 R8 workspace-member-registry-dep Rule 1 proof) | `tests/compat/overlay_determinism.rs` | ✅ Yes | `LIHAAF_RUN_CARGO_BUILD_TESTS=1` set in `.github/workflows/ci.yml:56`. Local runs skip these per `[[lihaaf-no-local-binary-builds]]`. |
| Probe-file silent-false mirror tests (§5.2.10 anyhow-shape, §5.2.11 thiserror-shape) | `tests/compat/overlay_determinism.rs` | ✅ Yes | `LIHAAF_RUN_CARGO_BUILD_TESTS=1`. M.5 + M.6 closure. |
| Backward-compat re-verification (§5.3.1-4) | `tests/compat/overlay_determinism.rs` | ✅ Yes | `LIHAAF_RUN_CARGO_BUILD_TESTS=1` for the cargo-rustc tests; standard path for the structural tests. |
| Patch absolute-path pin (§5.2.12) | `tests/compat/overlay_determinism.rs` | ✅ Yes | No env gate. |
| TER-5 scope-out (§5.2.13) | (no test file — scoped out per §6.13) | N/A | Explicit scope-out documented in §6.13 + §13 with follow-up issue # placeholder. |

**Confirmed:** `.github/workflows/ci.yml:56` sets `LIHAAF_RUN_CARGO_BUILD_TESTS: "1"` for the test job, so all cargo-build-gated tests run in CI without local-run risk per [[lihaaf-no-local-binary-builds]].

**Verification step (R4 — implementer pre-PR check):** before opening PR, the implementer must `grep "LIHAAF_RUN_CARGO_BUILD_TESTS" .github/workflows/ci.yml` and verify the gate is set (current expectation: line 56). If the line has moved, update the §11 audit table reference and verify the gate value is still `"1"`. If the gate is NOT set, that is a BLOCK-class precondition violation — the test surface depends on it.

**Negative criterion (R3 explicit, R4 unchanged):** no test in §5 may rely on a gate that is NOT set in CI. The R4 review verifies the gate is flipped on; if a future PR removes the gate-set from CI, the cargo-build-gated tests must either be re-enabled by a different mechanism or the gate-removal must be flagged BLOCK.

**Implementer checklist (R3 addition, R4 unchanged):** during PR-1, the implementer must run a full local `cargo test --lib` AND `cargo build --release` AND `RUSTDOCFLAGS=-D warnings cargo doc --no-deps` per [[lihaaf-review-verify-cmds]]. The cargo-build-gated tests will be SKIPPED locally (per [[lihaaf-no-local-binary-builds]] WSL2 OOM avoidance) but will RUN in CI; the implementer's PR description must explicitly call out the cargo-build-gated tests as CI-only verification.

**Dispatch-required test list (R5 — implementer must include all of these in PR-1):**

1. `apply_self_patch_writes_entry_for_named_package_rule1_inject` (§5.1.1)
2. `apply_self_patch_remap_when_upstream_self_patch_cxx_shape_rule2` (§5.1.5)
3. `apply_self_patch_rejects_when_upstream_path_targets_external_source_rule4_path` (§5.1.7)
4. `apply_self_patch_rejects_when_upstream_git_form_rule4_git` (§5.1.8)
5. `apply_self_patch_preserves_other_crate_patches_when_remap_or_inject` (§5.1.10)
6. `lexical_path_normalize_handles_repeated_separators` (§5.1.12)
7. `lexical_path_normalize_does_not_resolve_symlinks` (§5.1.13)
8. `cargo_accepts_inject_when_clean_upstream_anyhow_shape` (§5.2.5)
9. `cargo_accepts_remap_when_upstream_self_patch_cxx_shape` (§5.2.6 — upgraded in R5 to exercise `src/cxx_stub.cc` and `include/stub.h` file reads via staged-mirror, M.4 closure)
10. `cargo_accepts_continue_absolutize_when_non_root_patch` (§5.2.7)
11. `materialize_rejects_when_upstream_patch_targets_external_source_rule4` (§5.2.8)
12. `cargo_accepts_workspace_member_registry_dep_via_self_patch` (§5.2.9 — R8 rescope: Rule 1 INJECT closure; workspace-member-registry-dep-via-self-patch; faithful upstream shape)
13. `cargo_build_anyhow_shape_probe_file_resolves_via_mirror` (§5.2.10 — M.5 closure, anyhow silent-false probe)
14. `cargo_build_thiserror_shape_probe_file_resolves_via_mirror` (§5.2.11 — M.6 closure, thiserror silent-false probe)
15. `byte_identical_across_two_lihaaf_binaries_on_corpus` (§5.3.4 — corpus count bumped to 8)
16. `patch_crates_io_self_injection_absolute_path_only` (§5.2.12 — integration-level pin on the absolute-path contract for `[patch.crates-io.<self>].path` emission; belt-and-suspenders alongside the unit-test invariant at §5.1.3)
17. `apply_self_patch_idempotent_second_materialize` extended for Option B (§5.1.4 — R6 assertion (b)/(c)/(d): second call Ok, identical state, no AlreadyExists, symlink inode identity preserved for CASE 2 skips)
18. `mirror_upstream_rerun_reconciles_stale_entries` (§5.1.14 — R6 new: CASE 3/5/6/7/12 representative stale-state reconciliation)
19. `mirror_copy_fallback_exact_sync_removes_destination_only_files` (§5.1.15 — R6 new: CASE 6 copy-fallback exact-sync removes destination-only files)

## 12. Implementation effort estimate (R5 update)

**Size: L (large).**

- ~200-280 LOC of new / modified code in `src/compat/overlay.rs`: `apply_self_patch_policy` function (incl. normalizer helper, Option H 4-rule policy); `mirror_upstream_into_overlay` function (symlink loop + copy fallback); call-sites for both functions; doc updates.
- ~600-780 LOC of new test code: 15 unit tests (R6 adds §5.1.14 stale-reconciliation test + §5.1.15 copy-fallback exact-sync test; §5.1.4 extended) + 13 integration tests (R5 additions unchanged).
- Corpus updates: 8 expected files revised (6 existing including the R3→R4→R5 behavioral shift on `with_patch_section` + 2 new for Rule 1 and Rule 2). Corpus-list test bumped to 8 entries.
- Docs: 5 update sites (README, compatibility-plan §3.2.3, spec, CHANGELOG, module-level / function-level rustdoc). R5 doc text covers the Option H 4-rule decision tree + §2.6 cargo-anchoring appendix + §4.5 staged-mirror strategy.
- Inline call-site comment blocks (~10-15 lines each for `apply_self_patch_policy` and `mirror_upstream_into_overlay`) per §10.6.
- **R5 new features:** `Error::CompatPatchOverrideConflict` variant (Rule 4 REJECT; ~15-25 LOC) + `Error::OverlayMirrorFailed` variant (mirror I/O failure; ~15-25 LOC). Both additive.

**Expected panel rounds: 2-3.**

- Round 1: implementer (careful-coder Opus) writes the fix + tests + docs. Triple-reviewer panel (Codex + Gemini + strict-swe Opus per [[lihaaf-three-reviewer-panel-calibration]]) reviews. Expected findings:
  - Rule 2 REMAP detection edge cases: implementer may discover that `Path::join` behavior on Windows differs from Unix; reviewers push on cross-platform pinning.
  - Mirror strategy symlink vs copy fallback correctness on Windows — reviewers may push on `symlink_dir` vs `symlink_file` dispatch, `ERROR_PRIVILEGE_NOT_HELD` handling, and the copy fallback cost.
  - Test 5.2.7 (Rule 3 CONTINUE-ABSOLUTIZE) — reviewers may flag it as redundant with the existing `cargo_accepts_rich_overlay_for_dylib_build` test; implementer's call to keep distinct.
  - Test 5.2.8 (Rule 4 REJECT) error message text — reviewers may push for specific wording / structured fields.
  - Normalizer correctness — reviewers may push on `..` preservation policy, repeated-separator edge cases, or symlink-equivalence boundary (covered by §5.1.12-13).
  - Corpus byte-shape on the `with_patch_section` fixture under R5 REMAP — reviewers may verify the updated expected file matches the R5 spec literally.
  - TER-5 scope-out justification (§6.13) — reviewers may push for an actual git-dep test or accept the scope-out.
  - Whether §5.2.10 and §5.2.11 should be collapsed into a single parameterized test vs kept separate — implementer's call per §13.
- Round 2: address Round-1 BLOCK findings. Re-run panel.
- Round 3 (probability ~25%): only if Round 2 introduces a surprising regression in cargo-build-gated tests (CI-only signal, may surface only after merge into a refresh-pilots run).

Confidence: higher than R4 given the staged-mirror strategy resolves the Class A pilot materialization gap comprehensively, and the two new probe-file tests (§5.2.10-11) turn silent-false failures into loud compile errors.

**Implementer agent recommendation:** `careful-coder` (Opus, max effort). The cross-cutting touch into overlay.rs + corpus updates + integration tests + docs + two new Error variants requires Opus-tier context handling; the mirror strategy introduces new I/O logic that benefits from careful review.

---

## 13. Open items the implementer must NOT decide alone

- The conflict policy in §6.1 / §4.1 step 5 (R5 chose **Option H 4-rule decision tree**; Rule 1 INJECT, Rule 2 REMAP, Rule 3 CONTINUE-ABSOLUTIZE, Rule 4 REJECT). If the implementer wants to change, surface during Codex R5 adversarial review of THIS plan first.
- The exact text of the Rule 4 `Error::CompatPatchOverrideConflict` error message — implementer drafts, reviewers refine in round 1. Must reference the v0.2/v1.1 escape-hatch follow-up issue (file at implementation time).
- The Rule 2 REMAP emission form choice: literal `path = "."` (relies on cargo re-anchoring; matches upstream byte shape) vs absolutized staged-overlay-dir (unambiguous, robust). R5 §3.1 + §2.6 specify the absolutized form; if implementer review surfaces a strong reason to prefer the literal `.` form, escalate during plan review (not during PR-1).
- Whether the `apply_self_patch_policy` algorithm runs BEFORE `absolutize_patch_paths` (Rule 2 detection on raw upstream values) or AFTER it (Rule 2 detection on absolutized values; R5 specifies AFTER). Either is correct as long as §5 tests pin the behavior; implementer chooses during PR-1.
- The Shape A vs Shape B call-site wiring choice (§4.2). Shape A (DRY with `sibling_path`) preferred; Shape B acceptable if Shape A introduces scope creep.
- The function rename `inject_self_patch_crates_io` → `apply_self_patch_policy` (R5 suggested name; reflects that it now does more than inject). Implementer may keep R3's name if reviewers prefer; descriptive-only choice.
- Whether §5.2.10 (`cargo_build_anyhow_shape_probe_file_resolves_via_mirror`) and §5.2.11 (`cargo_build_thiserror_shape_probe_file_resolves_via_mirror`) should be kept as two separate tests (each pinning a different path form: `src/nightly.rs` vs `build/probe.rs`) or collapsed into one parameterized test. R5 keeps them separate for explicitness; implementer may collapse if the reviewer panel accepts the parameterized form.
- Whether to ship test 5.2.13 (`cargo_accepts_git_dependency_branch_in_patched_graph` per TER-5) or scope out per §6.13. R5 scopes out; implementer may push for the test if a real pilot surfaces a git-dep edge case before PR-1 lands. If shipped, the implementer must vet CI git URL stability.
- The v0.2/v1.1 follow-up issue # for Rule 4 REJECT error message references. Implementer files the issue at implementation time; the issue number replaces the `#X` placeholder in the §3.1 Rule 4 error message + §10 doc text.
- Cross-platform pinning for Rule 2 detection (Windows vs Unix `Path::join` and lexical normalization). R5 specifies Unix-style normalization; implementer verifies Windows behavior during PR-1 and surfaces any divergence as a finding.
- Cross-platform symlink privilege handling for `mirror_upstream_into_overlay` (§4.5.3). On Windows, `symlink_dir` may require Developer Mode or elevated privilege. The implementer must test both the symlink path and the copy-fallback path on Windows CI if available, or document the gap explicitly if Windows CI is not in scope for v0.1.0.

All other decisions in this plan are pre-committed and the implementer follows them as-written unless adversarial review of THIS plan flags them.

---

## Revision history

- **R1 (2026-05-16):** Initial plan. INJECT strategy aimed at upstream-dir (broken self-loop). REJECT-on-conflict policy.
- **R2 (2026-05-17, post-Codex-R1-BLOCK):** INJECT target corrected to staged-overlay-dir (BLOCK-1 fix). REJECT-on-conflict retained with staged-overlay-dir target.
- **R3 (2026-05-17, post-Codex-R2-BLOCK):** REJECT-on-conflict replaced by DETECT-AND-PRESERVE-PER-KEY (SEC-5 fix: cxx's upstream self-patch preserved verbatim). Cargo-graph cycle closure (SEC-6). `--compat-allow-patch-override` escape hatch struck (TER-4 fix). BLOCK-2 normalizer corner cases.
- **R4 (2026-05-18, post-Codex-R3-BLOCK):** PRESERVE-AS-IS replaced by Option H 4-rule intent-aware policy (SEC-7 fix: cargo re-anchors `[patch].path` relative to declaring manifest; Rule 2 REMAP replaces PRESERVE). §2.6 cargo-anchoring analysis added. `cargo_accepts_root_to_test_suite_to_root_topology` SEC-8 closure test added. TER-5 (git-dep) scoped out explicitly.
- **R5 (2026-05-18, post-Codex-R4-BLOCK + sweep-after-review):** Staged package-root mirror with symlinks + copy fallback (§4.5 new section) — covers cxx M.2-M.4, anyhow M.5, thiserror M.6. §5.2.6 build.rs upgraded from stub to real file-read test. New §5.2.10-11 probe-file silent-false tests. Pilot inventory precision: 4 build-script classes (§3.2 AC.2). v0.1.0 framing throughout (AC.1). Three counter-signal fixes from R4 review: §2.6 citation (C.1), overlay.rs:1393+:1402 production citation (C.2), "step 6" ordering phrase + §5.2.9 numbering (C.3). Verified non-drivers (serde_json, derive_more, axum-macros) now explicit in §3.2. Per sweep-after-review discipline applied to R4 BLOCK-1.
- **R6 (2026-05-18, post-Codex-R5-BLOCK + sweep):** Mirror idempotency contract (Option B — Idempotent skip + reconcile-by-replacement). New §4.5.6 "Idempotency / rerun-state reconciliation" with 15-case rerun-state table (CASEs 1–15, grouped A/B) and 7-item idempotency-contract decisions (skip-on-canonical / reconcile-by-replacement / desired-root-state / discrepancies=replace-or-error / copy-exact-sync / no-preservation / failure-modes). §4.5.2 pseudocode replaced bare "create symlink" loop with the full per-case decision tree (CASEs 1–9 forward pass + stale-cleanup pass + CASE 15 post-condition assertion). Existing §4.5.6 (apply_self_patch_policy interaction) renumbered §4.5.7; existing §4.5.7 (known limitations) renumbered §4.5.8; §4.3 doc reference updated. §5.1.4 extended with Option B second-call assertions (Ok, identical state, no AlreadyExists, CASE 2 inode-identity preservation; superseding earlier mtime-based wording per R7 BLOCK-1 cleanup). New §5.1.14 `mirror_upstream_rerun_reconciles_stale_entries` (CASE 3/5/6/7/12 representative reconciliation sub-cases). New §5.1.15 `mirror_copy_fallback_exact_sync_removes_destination_only_files` (CASE 6 exact-sync removes destination-only files). §11 dispatch-required test list extended to 19 items. Per sweep-after-review applied to R5 BLOCK staged-mirror-lifecycle class.
- **R7 (2026-05-18, post-Codex-R6-BLOCK):** Idempotency propagation cleanup (BLOCK-1: mtime→inode-identity in §11 item 17 and ID.4 delta summary). CASE 14 split (BLOCK-2: §4.5.6 table CASE 14 split into CASE 14a disposable `target/` and CASE 14b must-be-absent-or-removed `.git/`+`Cargo.lock`; §4.5.2 stale-cleanup pass extended with explicit CASE 14b removal loop; §4.5.4 exclusion table extended with Disposable/Must-be-absent-or-removed category column; §4.5.6 decision 3 updated to note cleanup pass backs the "exactly" claim; §4.5.6 opening para updated). CASE 15 narrowed claim (BLOCK-3: Option B-15a — §4.5.6 table CASE 15 narrowed to type-only structural check; §4.5.2 post-condition comment updated with B-15a rationale; `write_file_atomic` cited as content-correctness owner). §11 item 18 + R6 revision history CASE 6 inclusion (FIX_BEFORE_MERGE: CASE 3/5/7/12 → CASE 3/5/6/7/12 in both sites). §4.5.6 CASE 4 table aligned with §4.5.2 pseudocode wording (MINOR: "no further action needed" replaces stale CASE-9 back-reference). §5.1.X placeholder concrete-numbering (MINOR: §5.1.X → §5.1.12 and §5.1.13 at two bullet sites).
- **R8 (2026-05-18, post-Codex-post-implementation-diagnosis):** Surgical fix per Codex rollout 019e3cc3 — CI cycle in §5.2.6/§5.2.9 cargo-build-gated fixtures; plan §5.2.0 claim narrowed. §5.2.0: prior "cargo collapses to a single Package rather than treating them as a cycle" was overbroad — cargo collapses sources but DOES reject active dep cycles (cycle check fires after source-id resolution). Narrowed claim: self-patch collapses registry-name references only when they do not create an active package self-dependency cycle; valid for cxx because cxx lacks a root dep on its test-suite member. §5.2.6 fixture: remove `[dependencies] test-suite = { path = "test-suite" }` from root — faithful to cxx's actual `Cargo.toml` (workspace declaration only; verified at github.com/dtolnay/cxx). §5.2.9: rescope from empirically-false `root → member → root` active-dep topology to `cargo_accepts_workspace_member_registry_dep_via_self_patch` — proves Rule 1 INJECT correctly remaps workspace member's registry dep via staged-overlay patch even when root carries no dep on member (the faithful upstream shape). §11 item 12 + Rule 1 coverage note + §12 table updated to match renamed test.
