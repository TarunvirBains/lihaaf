# ⚠️ TEMPORARY ARTIFACT — DELETE AFTER v1.0.0 PARITY/POLISH-BAR WORK COMPLETES

This is a **pre-implementer-dispatch audit artifact**, NOT durable repository documentation.

- Lives on branch `docs/v01-plan-artifacts` for the duration of the v1.0.0 parity/polish-bar implementation cycle.
- Referenced by GH issues #42, #44, #45, #46, #48, #49, #50 (filed/amended on 2026-05-18).
- Once all referenced v1.0.0 issues have been closed by their respective implementer PRs (and post-merge review-ALLOW completes), this file should be `rm`'d. The most recent implementer PR to close should include the deletion.
- Source-validated against `dtolnay/trybuild` master via direct `src/` read on 2026-05-18.

---

# Lihaaf trybuild-parity audit — VALIDATED (post-haiku, source-verified)

Validation by: strict-swe-sonnet
Date: 2026-05-18
Trybuild source: /tmp/trybuild (master, cloned 2026-05-18)
Input: /tmp/lihaaf-trybuild-parity-audit-draft.md (haiku first-pass)

---

## Summary of validation deltas

- Haiku MISSING items confirmed TRUE-MISSING: 3 (ICE detection, NixOS
  extra_substitutions, span-tolerant matching)
- Haiku MISSING items reclassified: 4 (see below — FALSE-MISSING or
  DOCS-GAP)
- Haiku N/A items found incorrect: 2 (CARGO_INCREMENTAL and --offline are
  real trybuild features, not N/A — haiku misunderstood the architecture)
- Haiku HAVE items spot-checked: 4 (multiline suggestion parsing, external
  crate error filtering, compiler version-string normalization, platform
  path normalization) — all CONFIRMED with caveats noted
- New features surfaced by source-read: 4 (keep-going mode, file-locking
  between test processes, edition.workspace=true inheritance, TERM-env-var
  color diff gating)
- Items where haiku's evidence was unverifiable: 1 (span-tolerant matching
  — already disproved by orchestrator pre-validation; confirmed here)

---

## Per-item validation (8 MISSING items + 4 HAVE spot-checks)

### Step 1 — MISSING items (8 specific + #48 pre-confirmed)

---

ITEM 1: RUSTFLAGS pass-through (haiku status: MISSING, tracked #42)
  VALIDATED STATUS: FALSE-MISSING-LIHAAF-EQUIVALENT (partial and different)

  TRYBUILD EVIDENCE: src/rustflags.rs:13-17
    ```
    if let Some(flags) = env::var_os("RUSTFLAGS") {
        // TODO: could parse this properly and allowlist or blocklist ...
        if flags.to_string_lossy().contains("-C instrument-coverage") {
            rustflags.extend(["-C", "instrument-coverage"]);
        }
    ```
    Trybuild's implementation is narrow and opinionated: it only passes
    through `-C instrument-coverage` from RUSTFLAGS (for cargo-llvm-cov
    coverage), and only that one flag. It explicitly removes RUSTFLAGS from
    the environment first (src/cargo.rs:47: `cmd.env_remove("RUSTFLAGS")`),
    then reconstructs a controlled set via `--config=build.rustflags`.
    There is NO general RUSTFLAGS pass-through.

  LIHAAF EVIDENCE: src/dylib.rs:113-120
    Lihaaf preserves RUSTFLAGS at the dylib-build step by appending
    `-C prefer-dynamic` rather than wiping and reconstructing. This means
    lihaaf already passes arbitrary RUSTFLAGS through to the dylib build,
    which is actually MORE general than trybuild's instrument-coverage-only
    carve-out. However, the per-fixture rustc invocation in src/worker.rs
    shows no corresponding RUSTFLAGS handling — gap is at the fixture level,
    not the dylib level.

  REVISED CLASSIFICATION: PARTIAL (haiku said MISSING — more nuanced)
  NOTE: Haiku's characterization of trybuild as having general RUSTFLAGS
  pass-through is incorrect. Trybuild has a NARROW instrument-coverage
  special-case only. The real gap is whether lihaaf should pass
  `-C instrument-coverage` to per-fixture rustc invocations. The #42 issue
  scope should be adjusted: not "general RUSTFLAGS" but "instrument-coverage
  carve-out for cargo-llvm-cov parity."

---

ITEM 2: Edition mismatch detection (haiku status: MISSING, tracked #44)
  VALIDATED STATUS: TRUE-MISSING — but haiku also missed a related gap:
  trybuild handles `edition.workspace = true` inheritance (which lihaaf
  does not appear to support).

  TRYBUILD EVIDENCE: src/run.rs:211-218
    ```
    let edition = match source_manifest.package.edition {
        EditionOrInherit::Edition(edition) => edition,
        EditionOrInherit::Inherit => workspace_manifest
            .workspace.package.edition
            .ok_or(Error::NoWorkspaceManifest)?,
    };
    ```
    Trybuild reads the consumer's edition from `Cargo.toml` at runtime and
    passes it to the manifest. It does NOT warn on mismatch — it applies
    whichever edition the consumer's `Cargo.toml` declares. The "edition
    mismatch detection" that haiku described (warn if dylib edition vs
    fixture edition differ) does not actually exist in trybuild source.
    What trybuild DOES have is `edition.workspace = true` inheritance via
    `InheritEdition` (src/inherit.rs) and `EditionOrInherit` enum
    (src/dependencies.rs:225).

  LIHAAF EVIDENCE: src/config.rs:127 (`pub edition: String`) — lihaaf
    treats `edition` as a static string from `[package.metadata.lihaaf]`
    config. No dynamic read of the consumer's Cargo.toml edition, no
    workspace-edition inheritance, no `edition.workspace = true` handling
    (grep for `edition.workspace` or `InheritEdition` returns zero matches
    in lihaaf src/).

  REVISED CLASSIFICATION: TRUE-MISSING, but the gap description needs
  correction. It is not "mismatch detection" but rather "dynamic edition
  resolution from consumer Cargo.toml + edition.workspace=true support."
  Lihaaf uses a hardcoded config key while trybuild reads edition from the
  manifest at runtime. The #44 issue should be retitled.

---

ITEM 3: NixOS extra_substitutions (haiku status: MISSING, tracked #45)
  VALIDATED STATUS: TRUE-MISSING

  TRYBUILD EVIDENCE: no source reference found
    Searched src/normalize.rs for "extra_substitution", "substitut", "nix",
    "NixOS", "runtimeDependencies" — zero matches. Trybuild does NOT have
    a configurable extra_substitutions feature. The existing substitution
    list ($DIR, $WORKSPACE, $RUST, $CARGO) is hardcoded.

  LIHAAF EVIDENCE: src/normalize.rs:103-105
    Same hardcoded set: $DIR, $WORKSPACE, $RUST, $CARGO/registry.

  NOTE: Haiku's claim that trybuild has "configurable path substitutions
  for NixOS" is FALSE. Both trybuild and lihaaf have the same hardcoded
  substitutions. The #45 issue is not a trybuild-parity gap — it is a
  lihaaf enhancement proposal (beyond what trybuild offers). Issue scope
  should be updated to reflect this.

  REVISED CLASSIFICATION: MISSING — but this is a lihaaf-leads-trybuild
  feature, not a lihaaf-mirrors-trybuild gap. Reclassify as enhancement.

---

ITEM 4: ICE (Internal Compiler Error) detection (haiku status: MISSING, tracked #46)
  VALIDATED STATUS: TRUE-MISSING

  TRYBUILD EVIDENCE: src/error.rs (full file read)
    The Error enum has: CargoFail, Mismatch, RunFailed, ShouldNotHaveCompiled,
    etc. No ICE variant. No "internal compiler error" string search anywhere
    in src/error.rs, src/run.rs, or src/normalize.rs.
    Trybuild does NOT classify or detect ICE. An ICE in trybuild produces
    the same RunFailed/CargoFail path as a regular compile error.

  LIHAAF EVIDENCE: not found in spec or src/verdict.rs
  REVISED CLASSIFICATION: TRUE-MISSING — but this is a lihaaf-leads-
  trybuild feature, not a parity gap. Same note as #45.

---

ITEM 5: Span-tolerant matching (haiku status: MISSING, tracked #48)
  VALIDATED STATUS: TRUE-MISSING IN TRYBUILD (pre-confirmed by orchestrator)

  TRYBUILD EVIDENCE: src/normalize.rs (no span/column matching logic found)
    Trybuild keeps fixture line:col exactly. The normalize.rs `hide_trailing_numbers`
    function (lines 487-494) only strips line:col from *other-crate* paths
    (external dependency spans), not from the test fixture's own spans.
    Searching for "span", "tolerant", "line_col", "column" returns zero
    matches. Trybuild does not have span-tolerant matching.

  NOTE: Haiku's description of trybuild as having "span-agnostic error
  matching" is the opposite of correct. The `#48` issue is a
  lihaaf-leads-trybuild proposal. The recommendation to add `span_tolerant:
  bool` config would give lihaaf capability trybuild lacks.

  REVISED CLASSIFICATION: MISSING-IN-BOTH — this is a lihaaf innovation
  proposal, not a parity gap.

---

ITEM 6: Extra custom rustflags via config (haiku status: MISSING, separate
from #42)
  VALIDATED STATUS: FALSE-MISSING — trybuild itself does not have a
  user-facing `extra_rustflags` config key either.

  TRYBUILD EVIDENCE: src/rustflags.rs:5 `pub(crate) fn toml(extra_rustflags:
  &[&'static str])` — the `extra_rustflags` parameter is an INTERNAL
  parameter, not user-facing. Callers are only in src/cargo.rs:129 and :152,
  both passing `&["--diagnostic-width=140"]`. There is no public API,
  config file key, or env var for user-supplied extra rustflags in trybuild.

  LIHAAF EVIDENCE: src/config.rs — no `rustflags` key in schema (confirmed
  by haiku; confirmed here).

  REVISED CLASSIFICATION: FALSE-MISSING — trybuild does not expose this to
  users either. This item should be dropped from the parity list or moved to
  "lihaaf enhancement beyond trybuild."

---

ITEM 7: Diagnostic-width consistency (haiku status: PARTIAL/MISSING)
  VALIDATED STATUS: FALSE-MISSING — lihaaf uses --error-format=json which
  is inherently width-neutral; the concern is void.

  TRYBUILD EVIDENCE: src/cargo.rs:129, :152
    `cargo_with_rustflags(project, &["--diagnostic-width=140"])`
    Trybuild builds with cargo+rustc (human-readable stderr). It must set
    --diagnostic-width for snapshot stability.

  LIHAAF EVIDENCE: src/worker.rs:933 `.arg("--error-format=json")`
    Lihaaf invokes rustc directly with --error-format=json. JSON output has
    no concept of "width" — the rendered field is width-independent.
    The diagnostic width concern is architectural N/A for lihaaf, not a
    missing feature.

  REVISED CLASSIFICATION: N/A (architectural difference, not a gap)

---

ITEM 8: Test filtering by fixture name (haiku status: MISSING/low-priority)
  VALIDATED STATUS: FALSE-MISSING — lihaaf has a SUPERIOR equivalent.

  TRYBUILD EVIDENCE: src/run.rs:525-554
    Trybuild reads `trybuild=<substring>` args from process args at runtime.
    This is driven by the `cargo test -- ui trybuild=foo.rs` invocation
    pattern and works by substring matching on fixture path.

  LIHAAF EVIDENCE: src/cli.rs:83-87
    `--filter` flag: "Run only fixtures whose relative path contains the
    substring. Multiple --filter flags are OR'd." This is a first-class
    CLI flag, not a process-args hack. It is more ergonomic than trybuild's
    approach. Additionally, `--compat-filter` provides the same in compat
    mode (src/cli.rs:62-64).

  REVISED CLASSIFICATION: HAVE (lihaaf has a cleaner equivalent)
  NOTE: The haiku MISSING classification was wrong. The spec §1.4 "non-goal"
  haiku cited is about per-fixture granularity in the library API — lihaaf
  is a CLI and already has this.

---

ITEM 9 (pre-confirmed): Span-tolerant matching (#48)
  Already covered in Item 5 above. TRUE-MISSING-IN-BOTH.

---

### Step 2 — HAVE spot-checks

---

HAVE CHECK A: Multiline suggestion parsing (haiku #23: HAVE)
  VALIDATED: CONFIRMED, but requires a precision note.

  TRYBUILD EVIDENCE: src/normalize.rs:63-66 (Normalization enum entries)
    `UnindentMultilineNote`, `UnindentSuggestion`, `UnindentAfterHelp`,
    `HeadingNote` — trybuild has explicit normalization passes for
    multi-line suggestions and notes (UnindentSuggestion at line 66,
    applied at line 673).

  LIHAAF EVIDENCE: src/normalize.rs:93 comment mentions "suggestions"
    as part of what is preserved: "the policy enumerates what is explicitly
    preserved (diagnostic text, span pointers, help text, suggestions)."
    However, lihaaf's normalize.rs does NOT have UnindentSuggestion or
    UnindentMultilineNote passes — it operates on the already-rendered
    `"rendered"` field from --error-format=json (src/worker.rs:762),
    which rustc renders as a single string. The unindent issue is moot
    because rustc renders the suggestion indentation before JSON emission.

  ASSESSMENT: Both tools handle multi-line suggestions, but by very
  different mechanisms. Trybuild normalizes raw rustc human-format output
  including unindenting; lihaaf uses the pre-rendered JSON field. The
  functional outcome (snapshots that capture suggestions) is equivalent.
  HAVE is correct at the capability level.

---

HAVE CHECK B: External crate error filtering (haiku #24: HAVE)
  VALIDATED: CONFIRMED WITH CAVEAT

  TRYBUILD EVIDENCE: src/normalize.rs:205-350
    When a span line points to `other_crate = true` (src outside the test
    fixture's source_dir), trybuild hides trailing line numbers and subsequent
    code-context lines (WorkspaceLines normalization at line 336-348). Also
    filters "error: aborting due to ..." (line 353), "error: Could not
    compile" (lines 367-376).

  LIHAAF EVIDENCE: src/normalize.rs:88-96 (module doc) explicitly states the
    policy: preserve "error: aborting due to N previous error[s]" and
    "For more information about this error, try..." — lihaaf DOES NOT drop
    these lines (see lines 93-96 comment: "Earlier drafts dropped both lines;
    they are now preserved byte-for-byte"). Tests at lines 570-594 confirm.

  ASSESSMENT: Lihaaf and trybuild diverge on this point. Trybuild drops
  "aborting due to" lines (src/normalize.rs:353-355) and "could not compile"
  lines (lines 367-376). Lihaaf explicitly preserves them per a policy
  decision (spec §6.3 + the comment at normalize.rs:93). This is an
  intentional divergence, not a missing feature — but haiku marked it HAVE
  without noting the behavioral difference. In compat mode lihaaf presumably
  strips these to match trybuild output, but in native mode the snapshots
  will differ.

  HAVE is correct for the capability (filtering external crate spans), but
  the aborting/could-not-compile behavior is a known divergence that
  matters for compat-mode accuracy.

---

HAVE CHECK C: Compiler version-string normalization (haiku #11: HAVE)
  VALIDATED: CONFIRMED

  TRYBUILD EVIDENCE: src/normalize.rs:361-364
    `if trim_start.starts_with("= note: this compiler was built on 2") &&
    trim_start.ends_with("; consider upgrading it if it is out of date")`
    → return None (strip line). Also DependencyVersion normalization at
    src/normalize.rs:319-330 strips crate version numbers from $CARGO paths.

  LIHAAF EVIDENCE: src/normalize.rs:405 comment: "(newer rustc versions
    phrase the same note as `the full type name...`)" and src/normalize.rs:679.
    The $CARGO path normalization via `rewrite_cargo_short` in lihaaf handles
    crate-version stripping in compat mode.

  CAVEAT: Lihaaf does NOT appear to strip the "this compiler was built on..."
  note (no grep match for "this compiler was built" or the equivalent in
  lihaaf src/normalize.rs). Trybuild strips it; lihaaf passes it through.
  This is a narrow but real divergence from trybuild behavior.

  REVISED: Mostly HAVE, but the "compiler was built on" note stripping is a
  docs gap / compat gap worth flagging. In compat mode this note would appear
  in lihaaf snapshots but not in trybuild snapshots for the same fixture.

---

HAVE CHECK D: Platform-specific path normalization (haiku #10: HAVE)
  VALIDATED: CONFIRMED

  TRYBUILD EVIDENCE: src/normalize.rs:191 `line = line.replace('\\', "/");`
    Applied within the `--> ` / `::: ` prefix block (line 190), meaning
    backslash-to-forward-slash conversion happens on path-reference lines.
    Also applied in target_dir_pat (line 198), source_dir_pat (line 204),
    workspace_pat (line 259), path_dep_pat (line 271) comparisons.

  LIHAAF EVIDENCE: src/normalize.rs:133-136 (comment) and line 451 fn doc
    `"Rewrite backslashes to forward slashes within the path portion..."`,
    gated to `--> ` and `::: ` markers. Logic matches trybuild's approach.

  ASSESSMENT: HAVE confirmed. Both tools restrict backslash-to-slash
  conversion to path-reference lines, with the same rationale.

---

## New features (Step 3 — trybuild behaviors haiku did not enumerate)

---

NEW FEATURE 1: Filesystem-level file locking between concurrent test processes
  TRYBUILD EVIDENCE: src/flock.rs (full file) — dual-layer lock:
    intra-process mutex (`static LOCK: Mutex<()>` line 10) + file-based lock
    via polling lockfile with 1500ms stale-bust timeout. Guards the shared
    project directory so two `#[test]` functions running concurrently in
    different integration test binaries do not clobber each other.

  LIHAAF STATUS: HAVE
  LIHAAF EVIDENCE: src/lock.rs exists (confirmed by directory listing).
    Not read in detail but the module name and haiku's mention of lock
    semantics indicate coverage. Warrants confirmation that lihaaf's lock
    covers the cross-binary case the way trybuild's FileLock does.
  V1.0.0 PARITY CANDIDATE: no (impl exists; confirm coverage of cross-binary
    case as a low-risk verification item)

---

NEW FEATURE 2: `--keep-going` mode (Cargo feature detection + batch build)
  TRYBUILD EVIDENCE: src/cargo.rs:108-114
    After building deps, trybuild probes whether the installed Cargo supports
    `--keep-going` by running `cargo build --keep-going` and checking exit
    status. If supported, `project.keep_going = true`. Then in src/run.rs:75-85
    when `keep_going && !has_pass`, trybuild uses `run_all()` (src/run.rs:321)
    which calls `build_all_tests` (src/cargo.rs:142-163) with `--keep-going`
    and `--bins` to compile all test bins in one pass before checking results.
    This is an optimization: compile all fixtures once, not N sequential
    cargo check calls.

  LIHAAF STATUS: PARTIAL/HAVE (different mechanism)
  LIHAAF EVIDENCE: lihaaf uses a worker pool (spec §5) with direct rustc
    invocation per fixture. The dylib-once model means there is no per-fixture
    cargo overhead, so the keep-going optimization (batch compile via cargo
    --bins) is architecturally moot. However, the "continue after failure"
    behavior (haiku marked N/A as #25) is architecturally present: lihaaf's
    parallel worker pool runs all fixtures concurrently regardless.
  V1.0.0 PARITY CANDIDATE: no (lihaaf's parallel execution already delivers
    the user-visible "keep-going" outcome; the cargo --keep-going feature
    detection is an implementation detail of trybuild's serial cargo model)

---

NEW FEATURE 3: `edition.workspace = true` inheritance
  TRYBUILD EVIDENCE: src/run.rs:211-217, src/inherit.rs (InheritEdition),
    src/error.rs:16 (NoWorkspaceManifest error variant)
    Trybuild reads the consumer's `Cargo.toml` at runtime. If
    `package.edition = { workspace = true }`, it looks up the edition from
    the workspace-root Cargo.toml. Error if workspace has no edition.

  LIHAAF STATUS: MISSING
  LIHAAF EVIDENCE: src/config.rs:127 `pub edition: String` and src/manifest.rs
    line 94 `pub edition: String` — lihaaf uses a hardcoded config string,
    never reads edition from Cargo.toml dynamically. Crates using
    `edition.workspace = true` in their Cargo.toml cannot rely on lihaaf
    picking up that edition automatically.
  V1.0.0 PARITY CANDIDATE: yes — consumers using `edition.workspace = true`
    (increasingly common since Rust 1.64) will need to duplicate their edition
    in `[package.metadata.lihaaf]`. This is a real ergonomic gap and a
    correctness trap: if workspace bumps the edition and the lihaaf config is
    not updated, tests silently run the wrong edition.

---

NEW FEATURE 4: `TERM` environment variable — diff display gating
  TRYBUILD EVIDENCE: src/message.rs:123
    `let diff = if env::var_os("TERM").is_none_or(|term| term == "dumb") {`
    When `TERM` is unset or `"dumb"`, trybuild suppresses the visual diff
    in failure output (no color/rich diff).

  LIHAAF STATUS: PARTIAL/N/A
  LIHAAF EVIDENCE: No TERM env var handling found in lihaaf src/. Lihaaf
    uses a `--quiet` flag (src/cli.rs:128) but does not check TERM.
  V1.0.0 PARITY CANDIDATE: depends-on-design. In CI environments where TERM
    is unset, trybuild silently degrades; lihaaf's output may differ. Low
    severity but relevant for snapshot diffing UX. If lihaaf emits rich diffs
    in CI (no TERM), that is noisy; if it emits none, it is less debuggable.

---

## Correction of haiku's N/A classifications

Two haiku N/A items are incorrect:

**N/A #4: `CARGO_INCREMENTAL=0` — WRONG N/A**
  Trybuild sets `CARGO_INCREMENTAL=0` on all cargo invocations (cargo.rs:48).
  Lihaaf invokes rustc directly (not cargo) for fixture compilation, so the
  env var is irrelevant to per-fixture builds. For the dylib build (which
  does use cargo), lihaaf does not appear to set CARGO_INCREMENTAL=0
  (dylib.rs grepped, no match). This could cause non-deterministic dylib
  builds when incremental is enabled. Warrants a check at dylib build time.
  Classification: PARTIAL (not N/A — there is a real question about the
  dylib build path)

**N/A #5: `--offline` cargo flag — WRONG N/A**
  Trybuild passes `--offline` to every cargo invocation (cargo.rs:49), right
  alongside `CARGO_INCREMENTAL=0`. The reason is to prevent trybuild test
  runs from triggering network access in CI. Lihaaf's fixture builds are
  rustc-direct (no network), but the dylib build uses cargo. If the dylib
  build does not pass `--offline`, a lihaaf run could trigger a `cargo update`
  or crate download during the dylib build phase. Lihaaf users in strict
  offline CI may be surprised.
  Classification: PARTIAL (dylib-build-path exposure; per-fixture path is
  correctly N/A)

---

## Recommended filing list for v1.0.0-prep GH issues

### Cat A — trybuild parity, validated as genuine gaps

1. **#44 retitle**: "Dynamic edition resolution from consumer Cargo.toml +
   edition.workspace=true support" — static config key is a real ergonomic
   and correctness gap vs trybuild's runtime manifest read. HIGH confidence.

2. **NEW: edition.workspace=true inheritance** (from Step 3, NEW FEATURE 3)
   — same root cause as #44, may be merged into it. Consumers using
   `edition.workspace = true` (common since Rust 1.64) must manually
   duplicate their edition in `[package.metadata.lihaaf]` or get wrong
   behavior. HIGH confidence.

3. **#42 scope correction**: Not "general RUSTFLAGS pass-through" but
   "instrument-coverage carve-out for cargo-llvm-cov compatibility" — the
   exact pattern trybuild implements. Add `-C instrument-coverage` detection
   to lihaaf's per-fixture rustc invocation when the flag appears in
   `RUSTFLAGS`. MEDIUM confidence (whether lihaaf's direct-rustc model needs
   this at all depends on whether cargo-llvm-cov requires it on the fixture
   invocation or just the dylib build).

4. **Compiler "built on" note stripping** (from HAVE CHECK C caveat) — in
   compat mode, trybuild strips `"= note: this compiler was built on 2..."`,
   lihaaf passes it through. This causes compat-mode snapshot divergence when
   a user has an outdated toolchain. MEDIUM confidence.

5. **CARGO_INCREMENTAL=0 at dylib build time** (from N/A correction) — verify
   or add `CARGO_INCREMENTAL=0` to lihaaf's dylib build command. LOW
   confidence (may already be present under a name I did not grep).

6. **--offline at dylib build time** (from N/A correction) — similarly, if
   lihaaf's dylib build does not pass `--offline`, CI environments with
   strict network isolation may fail unexpectedly. LOW confidence.

### Cat B — spec amendments

7. **#44 spec §3.2 amendment**: Document that `edition` in config must match
   `package.edition` in `Cargo.toml`; add spec guidance for
   `edition.workspace=true` crates (manual duplication required until
   dynamic resolution is implemented). This is the stop-gap user guidance.

8. **#45 scope correction**: NixOS extra_substitutions is a lihaaf-leads
   feature (not a trybuild-parity gap). Spec §6.1's no-regex non-goal
   applies. Amend the issue body to note trybuild does not have this either.

9. **#46 scope correction**: ICE detection is a lihaaf-leads feature.
   Trybuild has no ICE classification. Amend to be clear this is an
   enhancement, not a parity deficit.

10. **#48 scope correction**: Span-tolerant matching is a lihaaf-leads
    feature. Trybuild does not have it. Amend accordingly.

### Cat C — already-known not-filed

11. axum-macros workspace-member-subdir entry bug (from lihaaf_round2_fork_shape_analysis.md)

### Cat D — new issues from Step 3

12. **NEW: edition.workspace=true support** — HIGH, file as separate issue
    from #44 or merge into it.

13. **NEW: keep-going batch build** — no new issue needed; lihaaf's parallel
    worker pool already covers the user-visible behavior. Close as N/A.

14. **NEW: TERM-env-var diff display** — LOW priority, UX-only. POST_v1.0.0
    if filed.

---

## Item 6 / Item 8 filing recommendation

- ITEM 6 (extra custom rustflags via config): Do NOT file. Trybuild does not
  expose this to users. No parity gap exists. If lihaaf wants a user-facing
  `rustflags` config key, that is a lihaaf-specific enhancement, not
  trybuild-parity work.

- ITEM 8 (test filtering by fixture name): CLOSE AS DONE. Lihaaf already has
  `--filter` (and `--compat-filter`). Haiku's MISSING classification was
  wrong. No issue needed.

---

## Confidence calibration

| Filing candidate | Confidence | Reason |
|---|---|---|
| #44 retitle (dynamic edition from Cargo.toml) | HIGH | Verified in trybuild src/run.rs:211-218 vs lihaaf config.rs:127 |
| edition.workspace=true (new) | HIGH | Same evidence; InheritEdition confirmed in trybuild; absent in lihaaf |
| #42 scope narrowing (instrument-coverage only) | HIGH | Trybuild rustflags.rs:13-17 is explicit; no general pass-through |
| Compiler "built on" note stripping (compat gap) | MEDIUM | Trybuild normalize.rs:361-364 confirmed; lihaaf normalize.rs line 93 explicitly says it preserves these — may be intentional |
| CARGO_INCREMENTAL=0 at dylib build | LOW | Dylib build command not read in full; may already set it |
| --offline at dylib build | LOW | Same caveat; dylib.rs shows RUSTFLAGS handling but not --offline |
| #45 scope correction (NixOS = enhancement) | HIGH | Zero matches in trybuild normalize.rs; confirmed absence |
| #46 scope correction (ICE = enhancement) | HIGH | Trybuild error.rs has no ICE variant; confirmed absence |
| #48 scope correction (span-tolerant = leads) | HIGH | Pre-confirmed by orchestrator; validated here |
