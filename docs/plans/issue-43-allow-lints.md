# ⚠️ TEMPORARY ARTIFACT — DELETE AFTER POST-IMPLEMENTATION REVIEW-ALLOW

This is a **pre-implementer-dispatch design plan**, NOT durable repository documentation.

- Lives on branch `docs/v01-plan-artifacts` for the duration of the implementer-dispatch + adversarial-review cycle for lihaaf #43.
- The implementer's PR (target: `careful-coder-sonnet`) MUST `rm docs/plans/issue-43-allow-lints.md` as part of its diff so this file does not land on `main`.
- If the implementer's PR is reviewed and ALLOWed but this file is still present, the post-merge cleanup must remove it before the next release branch cuts.

---

# lihaaf #43 — Plan: `allow_lints` config key for suppressing rustc lints in fixtures

**Revision: R10 (2026-05-18, §10b verbatim-command fix)**

**Issue:** [TarunvirBains/lihaaf#43](https://github.com/TarunvirBains/lihaaf/issues/43)
**Upstream mirror:** [dtolnay/trybuild#302](https://github.com/dtolnay/trybuild/issues/302)
**Target implementer:** `careful-coder-sonnet` (single-area, mechanical addition)
**Target branch:** `feat/issue-43-allow-lints` (cut from `main`)
**Working dir:** `/home/tarunvir/projects/lihaaf` (currently on `feat/compat-mode-beta-4` — implementer cuts a fresh branch from `main`)

---

## Revision history

- **R1 (initial)** — drafted by strict-swe Opus planner.
- **R2-R4 (collapsed for brevity).** Four BLOCK rounds drove the plan from
  initial draft to converged shape. R2 redesigned the integration test to
  `CompileFail` and added compat-mode default injection + a documentation-
  updates section. R3 then fixed three concrete claim-vs-reality bugs: the
  R2 test used `unexpected_cfgs` which is `--check-cfg`-gated and lihaaf
  does not pass that flag (worker.rs:916-919, 929-972), so R3 switched the
  integration test to `unused_imports` (a default-on lint that fires under
  bare rustc, name-resolution-pass — survives alongside type errors); R3
  also dropped a false claim that the §3.3 envelope carries synthetic-
  metadata `allow_lints` (`OverlayMetadata` at report.rs:104-139, 286-301
  carries only `generated`, `dropped_comments`,
  `upstream_already_has_dylib`); R3 added §4d.3 to pin the replacement
  invariant at overlay.rs:1051-1058. R4 reframed all plan language around
  the compat default `["unexpected_cfgs"]` being **forward-only insurance**
  (a no-op under v0.1.0 because lihaaf does not pass `--check-cfg`), not a
  Round-2 Day-1 fix; R4 also rewrote §10 as an honest CI / PR-paste /
  manual-post-merge matrix (§10a/§10b/§10c) instead of overclaiming "every
  net runs in CI"; R4 fixed a broken awk range for the CHANGELOG grep.
- **R5** — post-Codex-R4-BLOCK. Three mechanical contradiction sweeps after
  R4 left sibling sections inconsistent with R4's own framing: (BLOCK-1)
  aligned §9d header / body / summary with §10's "PR-paste, not CI-
  enforceable" wording for doc greps; (BLOCK-2) aligned §9d's grep target
  with §9a's actual README example (`["unused_imports", "dead_code"]`,
  not `["unexpected_cfgs"]`); (BLOCK-3) rewrote the §3d planned rustdoc on
  `SyntheticMetadata.allow_lints` to qualify the noise claim with "once
  `--check-cfg` is active" — under v0.1.0 today the default is a no-op.
- **R6 (2026-05-18)** — holistic + block-pattern-aware sweep, pre-
  Codex-R5. Pass-1 holistic: corrected a stale §7 claim that no existing
  test covers `dev_deps`-precedent inheritance (the test
  `named_suite_inherits_unspecified_keys_from_default` at config.rs:760-
  792 explicitly asserts `spatial.dev_deps == ["serde"]` at line 784);
  cross-referenced §1 / §2's forward-only-insurance restatement back to
  §3d to reduce repetition without losing the reader's first-pass
  framing; verified every cited file:line against the current repo at
  commit `362bec0`. Pass-2 block-pattern: added an OOM-gating note in
  §4c.3 covering the `LIHAAF_RUN_CARGO_BUILD_TESTS` env-var gate that
  every dylib-building integration test currently uses (memory: the user's
  WSL2 box crashes on local builds, so an unguarded test would degrade
  the dev loop without affecting CI); added a one-line acknowledgment
  under §9b that the spec §3.2 example intentionally uses
  `["unexpected_cfgs", "dead_code"]` (compat-default + v0.1-active lint
  for canonical illustration) while §9a's README example uses
  `["unused_imports", "dead_code"]` (the v0.1-active pair Round-2 forks
  actually configure) — both forms are correct; the difference is
  illustrative.
- **R7 (2026-05-18)** — comprehensive class-sweep fix, 4 root-cause
  classes, 18 instances. Class A (2): split two broken `grep | grep`
  pipelines in §9d into independent single-term greps; also fixed the
  same-line mismatch in the README inheritance grep. Class B (7): expanded
  §3b from 1 construction site to all 7, including the load-bearing
  compat-mode wiring at `src/compat/mod.rs:151`; added a dedicated §3b.7
  callout that this site is what makes the compat-mode default take effect.
  Class C (7): added spec and rustdoc touchpoints for named-suite
  inheritance list (C.1), validation bullets (C.2), canonical argv block
  (C.3), compat-default grep (C.4), `config.rs` module rustdoc
  qualification re manifest.rs guidance (C.5), `config.rs:15` field
  enumeration (C.6), and `WorkerContext::new` rustdoc (C.7); each has a
  corresponding §9d grep. Class D (2): split §4c.3 guards into distinct
  "skip-guard" and "panic-guard" with unambiguous trigger conditions;
  changed §9d and Summary `cargo doc` invocations from the incorrect
  `-D warnings` argv form to the CI-matching `RUSTDOCFLAGS=-D warnings`
  env-var form. No new scope introduced beyond addressing these 18 items.
- **R7.1 (2026-05-18, post-simplify-sweep)** — 3 verification-command
  cleanups in §9d: added C.5 + C.6 joint comment (was mislabeled C.6
  only); removed vacuous bare-`inherit` README grep (`inherit` already
  present in README for unrelated reasons — the `allow_lints` grep is the
  load-bearing check); consolidated 4 identical `allow_lints`
  `docs/spec/lihaaf-v0.1.md` greps into 1 with a unified comment naming
  all four touchpoints (§3.2 / §3.6 / C.1 / C.2), with an explicit note
  that section discrimination relies on the surrounding section-specific
  terms. No new scope.
- **R8 (2026-05-18, post-Codex-R6-BLOCK)** — vacuous-grep class fix — 8
  §9d/§10b greps replaced with discriminating fixed-string greps
  containing neighboring anchor text. Class signal: verification commands
  passing vacuously when targeted strings appear elsewhere in the same file
  or in pre-existing content. Instances addressed: (1) README §Multi-suite
  bare `allow_lints` → anchor on `per_fixture_memory_mb, allow_lints`)
  inherit text; (2) spec consolidated single `allow_lints` grep → 5
  section-specific greps (§3.2 example value, §3.6 DO-inherit list append,
  C.1 comment append, C.2 starts-with-dash bullet, C.2
  whitespace/quote/backslash bullet); (3) spec bare `empty string` → C.2
  anchor `An entry in \`allow_lints\` is an empty string.`; (4) spec bare
  `unexpected_cfgs` → compat-mode callout anchor; (5) `src/config.rs` bare
  `allow_lints` → 2 anchors for C.5 qualification clause and C.6 field
  enumeration; (6) `src/worker.rs` bare `allow_lints` → C.7 WorkerContext
  rustdoc anchor; (7) CHANGELOG single awk+grep → 2 awk+greps, one per
  planned bullet; (ADJACENT 1) spec bare `DO inherit` → §3.6 full
  inheritance-list append anchor. §10b table mirrors updated consistently.
  Per sweep-after-review discipline.
- **R9 (2026-05-18, post-Codex-R7-BLOCK)** — §10b mirror parity gap fix —
  2 missing/divergent rows for §9a README explanatory comment + §9b §3.2
  example value; 3 stale-counter fixes (revision-history stamp, §10f grep
  count, §9d item 2 title). Per sweep-after-review applied to R7 BLOCK
  verification-mirror-drift class.
- **R10 (2026-05-18, post-Codex-R8-BLOCK)** — §10b mechanism-column verbatim
  command fix — all 13 truncated rows replaced with verbatim §9d commands
  including file paths and full pipelines; rows that had "from §9d" paraphrase
  stubs now carry the complete `grep -F '...' <file>` invocations or full
  `awk ... | grep -F ...` CHANGELOG pipelines. COUNTER FIX: revision-header
  consistency checklist added as process note in §10 intro. Per sweep-after-
  review applied to R8 BLOCK verification-mirror-drift-via-command-target-
  elision class.

---

## 1. Problem statement

lihaaf invokes `rustc` per fixture as a bare process (see `src/worker.rs:929` —
`Command::new("rustc")`). Bare `rustc` produces unsuppressable warning noise
that leaks into normalized snapshots under `tests/lihaaf/*.stderr` and forces
adopters to either hand-sprinkle `#![allow(...)]` at the head of every
fixture file or accept permanent snapshot noise.

The lints adopters legitimately want suppressed on the per-fixture rustc
invocation include:

- **`unused_imports`, `unused_variables`, `dead_code`** — common when fixtures
  share scaffolding modules with the main crate that don't use every import.
  These fire under bare rustc TODAY (default-on, not `--check-cfg`-gated).
- **`clippy::needless_collect` and similar `clippy::*`** — when fixtures
  contain idiomatic-but-not-perfect code that adopters intentionally leave
  un-fixed (the fixture is the test).
- **`unexpected_cfgs`** — fires only when rustc is invoked with
  `--check-cfg`. lihaaf's current `spawn_and_monitor` does NOT pass that
  flag (verified at `src/worker.rs:916-919, 929-972`), so this lint is
  dead under v0.1 lihaaf today. See §3d for the **forward-only insurance**
  rationale for the compat-mode default `["unexpected_cfgs"]` — the
  one-line summary is: the suppression matters only once `--check-cfg`
  becomes active (e.g. via #42 / RUSTFLAGS plumbing or a future rustc
  default), so under v0.1.0 today the default is a no-op.

The Round-2 v0.1.0 enrollees (`derive_more`, `axum-macros`) hit the
v0.1-active default-on lints on enrollment (NOT `unexpected_cfgs` — see
above). Round-2 forks add the relevant entries to their own
`[package.metadata.lihaaf].allow_lints` via the v0.1 TOML path (§2) — the
existing config surface this plan introduces; no extra work beyond writing
the fork TOML key.

---

## 2. Target behavior

A new optional key `allow_lints: Vec<String>` is accepted in
`[package.metadata.lihaaf]` and in each `[[package.metadata.lihaaf.suite]]`
entry. Each string is forwarded verbatim as `-A <lint>` to the per-fixture
`rustc` invocation, appended after `--edition` and before the per-fixture
source-file argument.

Example consumer config:

```toml
[package.metadata.lihaaf]
dylib_crate = "consumer"
extern_crates = ["consumer"]
allow_lints = ["unexpected_cfgs", "dead_code"]
```

Concrete effect: each `rustc` invocation gains `-A unexpected_cfgs -A dead_code`
in argv. The lints stop firing; fixture `.stderr` snapshots regain signal.

What does NOT change:
- Default value: `[]` (omitted is equivalent to today's behavior) for the v0.1
  TOML-driven path (`[package.metadata.lihaaf]` in adopter forks).
- No new env-var fallback (the [TOML-only configuration principle](src/config.rs:7-11) is preserved).
- Manifest snapshot (`raw_metadata` round-trip via `Config::raw_metadata`) keeps the new key verbatim — no special-case stripping needed.

**What DOES change** (compat-mode-only, full rationale in §3d): synthetic
metadata injected by `cargo lihaaf --compat` defaults
`allow_lints = ["unexpected_cfgs"]` instead of `[]`. Forward-only insurance,
a no-op under v0.1.0 today; the default does NOT address v0.1-active
default-on lints (`unused_imports`, `dead_code`, etc.) — those go on the
per-pilot-fork TOML path described above.

---

## 3. File-level changes

### 3a. `src/config.rs`

**Add to `Suite` struct (after line 147, before the closing `}` at line 148):**

```rust
/// rustc lints to forward as `-A <lint>` on every per-fixture
/// invocation. Empty by default; inherits from the default suite if
/// omitted on a named suite (same precedent as `dev_deps`).
pub allow_lints: Vec<String>,
```

**Add to `RawMetadata` (after line 177, before `suite:`):**

```rust
allow_lints: Option<Vec<String>>,
```

**Add to `RawSuite` (after line 196, before `dylib_crate:`):**

```rust
allow_lints: Option<Vec<String>>,
```

**`build_default_suite` (line 298-372):**
Set `allow_lints: raw.allow_lints.clone().unwrap_or_default()` in the
`Ok(Suite { ... })` block. Validate via a new `validate_allow_lints(suite_label, &lints)?` call before the struct literal.

**`finalize_named_suite` (line 374-470):**
Set
```rust
allow_lints: raw.allow_lints.unwrap_or_else(|| default_suite.allow_lints.clone()),
```
Mirror the `dev_deps` inheritance precedent at config.rs:461-463. Run the same
`validate_allow_lints(&name, ...)` on the resolved value.

**Add `validate_allow_lints` helper (near `validate_edition` at line 500):**

```rust
fn validate_allow_lints(suite_label: &str, lints: &[String]) -> Result<(), Error> {
    for lint in lints {
        if lint.is_empty() {
            return Err(/* ConfigInvalid: empty string */);
        }
        if lint.starts_with('-') {
            return Err(/* ConfigInvalid: caller must not include `-A` prefix */);
        }
        // Whitespace and shell-meta chars rejected — these would
        // either break argv quoting or smuggle extra flags past
        // rustc's argument parser.
        if lint.chars().any(|c| c.is_whitespace() || c == '"' || c == '\'' || c == '\\') {
            return Err(/* ConfigInvalid: whitespace/quotes/backslash not permitted */);
        }
    }
    Ok(())
}
```

Validation is **structural only** — we do NOT check the lint name against rustc's
known-lint list. rustc itself emits `warning: unknown lint: X` for unrecognized
names and continues; surface that as-is (rationale in §5).

### 3b. `src/worker.rs` (+ struct-field impact sweep)

**Add to `WorkerContext` (after `features` at line 65, before `edition`):**

```rust
/// `allow_lints` from the suite. Each becomes a `-A <lint>` flag.
pub allow_lints: Vec<String>,
```

**Construction-site impact sweep.** Adding `pub allow_lints: Vec<String>` to
`Suite` and `WorkerContext` requires updating EVERY direct struct literal that
constructs those types. The implementer must update all 7 sites below — missing
any one will cause a compile error (Rust struct literals require all fields to
be named). The sites are enumerated here so reviewers can verify completeness:

| # | File:line | Context | Required addition |
|---|---|---|---|
| 1 | `src/config.rs:358` | `Ok(Suite { ... })` — top-level default construction in `build_default_suite` | `allow_lints: raw.allow_lints.clone().unwrap_or_default()` |
| 2 | `src/config.rs:451` | `Ok(Suite { ... })` — named-suite construction in `finalize_named_suite` | `allow_lints: raw.allow_lints.unwrap_or_else(\|\| default_suite.allow_lints.clone())` |
| 3 | `src/session.rs:828` | direct `Suite { ... }` literal in `fn suite(...)` test helper | `allow_lints: vec![]` |
| 4 | `src/discovery.rs:159` | direct `Suite { ... }` literal in `fn suite(...)` test helper | `allow_lints: vec![]` |
| 5 | `src/worker.rs:131` | `Self { ... }` struct literal in `WorkerContext::new` | `allow_lints: suite.allow_lints.clone()` |
| 6 | `src/worker.rs:1487` | `WorkerContext { ... }` literal in `unit_test_ctx` test fixture | `allow_lints: vec![]` |
| 7 | `src/compat/mod.rs:151` | `overlay::SyntheticMetadata { ... }` construction in compat driver | `allow_lints: vec!["unexpected_cfgs".to_string()]` — see §3b.7 |

Sites 3 and 4 are in test helpers that build minimal `Suite` values for unit tests
that do not exercise the lints pathway — `vec![]` is correct. Sites 1 and 2 wire
the TOML-parsed value. Site 5 propagates the validated suite into the worker
context. Site 6 provides a zero-lints test fixture. Site 7 is the load-bearing
compat-mode wiring — see §3b.7.

**Note on `RawMetadata.allow_lints` and `RawSuite.allow_lints`:** these are
`Option<Vec<String>>` fields added to `#[derive(Deserialize)]` structs; serde's
derive macro handles the `None` default for omitted TOML keys automatically. No
direct literal construction sites exist for these raw types in `src/` or
`tests/` — no additional impact sites beyond the 7 above.

#### 3b.7. Load-bearing compat wiring — `src/compat/mod.rs:151`

Site 7 above is the wiring that makes the compat-mode default actually take
effect. Without this site updated, the `SyntheticMetadata` struct gains the
field but the compat driver never sets it — the compat-mode default becomes
dead code. The `allow_lints: vec!["unexpected_cfgs".to_string()]` literal at
`src/compat/mod.rs:151` is the single injection point where compat-mode
synthesis picks up the forward-only insurance default documented in §3d. The
full rationale for WHY this value is `["unexpected_cfgs"]` (not empty, not
broader) is in §3d.

**Update `WorkerContext::new` (line 111-151):**
Add `allow_lints: suite.allow_lints.clone(),` to the struct literal at line 131
(site #5 from the table above).

**Update `unit_test_ctx` helper (line 1486-1523):**
Add `allow_lints: vec![],` to the literal (site #6 from the table above).

**Add helper near `apply_feature_cfgs` (line 916-920):**

```rust
fn apply_allow_lints(cmd: &mut Command, lints: &[String]) {
    for lint in lints {
        cmd.arg("-A").arg(lint);
    }
}
```

**Wire into `spawn_and_monitor` (line 970, right after the existing `apply_feature_cfgs` call):**

```rust
apply_feature_cfgs(&mut cmd, &ctx.features);
apply_allow_lints(&mut cmd, &ctx.allow_lints);  // NEW
```

Placement is deliberate: between feature-cfgs and the source-file argument
(`cmd.arg(&fx.path)` at line 972), so `-A` flags come after `--cfg` and before
the input file — order matches the existing pattern of "build up all flags,
then the source file last."

### 3c. Manifest snapshot — no change needed

`Config::raw_metadata` is stored verbatim and serialized via
`toml_value_to_json` (`src/session.rs:354`); a new TOML key flows through
without code change. Verify by re-running the existing
`raw_metadata_is_preserved_verbatim` test (`src/config.rs:736`).

### 3c.2. `src/config.rs` module-level rustdoc — qualification required

`src/config.rs:4-5` currently reads: "If you add a new key, add it here and in
`manifest.rs` so snapshot behavior stays aligned." This guidance was written for
top-level keys (such as `dylib_crate`, `extern_crates`, `features`) that are
also surfaced in `manifest.rs` for snapshot purposes. The `allow_lints` key is a
**per-suite** key processed inside `Suite` / `RawSuite`; it flows through
`raw_metadata` verbatim via the existing `toml_value_to_json` path and does NOT
require a `manifest.rs` change (verified in §3c above — `Config::raw_metadata`
is stored as-is from the raw TOML).

The implementer must update the `src/config.rs:4-5` rustdoc comment to qualify
this guidance. Suggested change:

```rust
//! This module is the single point where raw TOML becomes the typed [`Config`]
//! used by the rest of the harness. If you add a new TOP-LEVEL key (one that
//! lives directly in `[package.metadata.lihaaf]`, such as `dylib_crate`,
//! `extern_crates`, or `features`), also add it in `manifest.rs` so snapshot
//! behavior stays aligned. Per-suite keys (those in `Suite` / `RawSuite`,
//! such as `dev_deps`, `edition`, and `allow_lints`) are preserved verbatim
//! via the `raw_metadata` round-trip and do NOT require a `manifest.rs` change.
```

This qualification prevents the next contributor adding a per-suite key from
making an unnecessary `manifest.rs` edit or, worse, a top-level key from
missing a required `manifest.rs` edit because the guidance seemed too broad.
A corresponding §9d grep verifies this update landed.

### 3d. `src/compat/overlay.rs` + `src/compat/mod.rs` — compat-mode injection

**Why this section exists (verified evidence).** The compat driver synthesizes
its own `[package.metadata.lihaaf]` block at `src/compat/mod.rs:145-156`. The
on-disk overlay synthesis at `src/compat/overlay.rs:1080-1106` inserts ONLY
`dylib_crate`, `extern_crates`, `fixture_dirs` — and per the comment at
`src/compat/overlay.rs:1051-1058`, replaces any pre-existing
`[package.metadata.lihaaf]` table in full (the "compat owns inner config"
invariant at `src/compat/overlay.rs:347-353`). Without explicit handling,
compat-mode pilots get `allow_lints = []` regardless of what (if anything)
the upstream codebase's TOML declares. This section adds a default
(`["unexpected_cfgs"]`) so the day lihaaf or rustc starts passing
`--check-cfg`, compat-mode pilots already have the right suppression
baked in. Under v0.1.0 today the default is a no-op (verified §1).

**Add field to `SyntheticMetadata`** (`src/compat/overlay.rs` after line 314,
before the closing `}` at line 315):

```rust
/// `allow_lints` — rustc lints forwarded as `-A <lint>` on every
/// per-fixture invocation. Defaults to `["unexpected_cfgs"]` in
/// compat mode as **forward-only insurance**. Today, with rustc
/// 1.95 not passing `--check-cfg` automatically and lihaaf not
/// setting it (verified `src/worker.rs:916-919, 929-972`), this
/// default is a no-op — the `unexpected_cfgs` lint is
/// `--check-cfg`-gated and does not fire. Once `--check-cfg` is
/// active in rustc (either by default or by lihaaf passing it
/// explicitly in a future release), compat pilots would otherwise
/// produce unavoidable `unexpected_cfgs` noise from their
/// proc-macro-emitted `#[cfg(feature = "...")]` annotations. This
/// default suppresses that noise preemptively so the toolchain
/// shift is uneventful.
///
/// This default does NOT address the v0.1-active default-on lints
/// (`unused_imports`, `dead_code`, etc.) that fire under bare
/// rustc today. Round-2 compat pilots that hit those add the
/// relevant entries to their own fork's
/// `[package.metadata.lihaaf].allow_lints` via the v0.1 TOML path.
///
/// To override (e.g. add more lints, or empty for diagnostic
/// debugging), the compat-driver caller passes a custom list when
/// constructing `SyntheticMetadata`.
pub allow_lints: Vec<String>,
```

**Update the `SyntheticMetadata` literal at `src/compat/mod.rs:151-155`:**

```rust
overlay::SyntheticMetadata {
    dylib_crate: name.clone(),
    extern_crates: vec![name],
    fixture_dirs: vec![abs_compile_pass.clone(), abs_compile_fail.clone()],
    allow_lints: vec!["unexpected_cfgs".to_string()],  // NEW
}
```

**Update `inject_synthetic_metadata` at `src/compat/overlay.rs:1063-1107`** to
write the new key into the TOML table:

```rust
// Insert after the fixture_dirs block at line ~1104.
lihaaf_table.insert(
    "allow_lints".to_string(),
    toml::Value::Array(
        meta.allow_lints
            .iter()
            .cloned()
            .map(toml::Value::String)
            .collect(),
    ),
);
```

**Why we don't make this the v0.1 default too.** Non-compat adopters opt
into `[package.metadata.lihaaf]` deliberately and may legitimately want
`unexpected_cfgs` to fire (e.g. testing diagnostic output of macros that
emit `#[cfg]`). The v0.1 TOML-driven path keeps the unsurprising "empty
default, adopter opts in" rule. The compat-mode default is an explicit
exception because compat mode's whole purpose is to wrap an upstream
codebase that is not lihaaf-aware: there is no place for the upstream to
declare a `[package.metadata.lihaaf]` block that would survive synthesis,
and pre-shipping a forward-only insurance default avoids requiring a
future post-toolchain-shift change. **Scope reminder:** this default does
NOT address Round-2 pilots' Day-1 v0.1.0 noise for `unused_imports`,
`dead_code`, and the other default-on lints — those are addressed via the
v0.1 TOML path (§2) where each Round-2 fork adds the relevant entries to
its own `[package.metadata.lihaaf].allow_lints`. Per-pilot widening of the
compat default via the override seam (below) is available if a specific
pilot warrants it; §7 risks discusses the trade-off rationale for keeping
the compat default narrow rather than bundling v0.1-active lints into it.

**Override surface.** Adopters / orchestrator-side compat-driver callers who
want a different list can construct `SyntheticMetadata` with a custom
`allow_lints` value. This is gated behind code change — there is intentionally
no CLI flag for it because compat mode is not a stable v0.1 API
(`src/compat/mod.rs:17-18` documents this). Future compat-mode work may surface
an env-var or flag; that is out of scope for #43.

**Backward compat of compat envelope (§3.3) — NO ENVELOPE CHANGE.** Verified
against live source: `CompatEnvelope.overlay: OverlayMetadata` at
`src/compat/report.rs:104-139, 286-301` carries only `generated`,
`dropped_comments`, `upstream_already_has_dylib`. The driver populates only
those fields; the synthetic metadata block is written to the on-disk overlay
TOML at `<upstream>/target/lihaaf-overlay/Cargo.toml`, NOT to the §3.3
envelope. Adding `allow_lints` to `SyntheticMetadata` therefore has zero
effect on envelope serialization and zero effect on `baseline.toml`
signature. The change is invisible to envelope consumers and to the §5 gate.

If a future maintainer wants envelope visibility of `synthetic_metadata.allow_lints`
for debugging, that is a separate work item that adds an `allow_lints` field
to `OverlayMetadata` and threads it through the driver — explicitly OUT OF
SCOPE for #43.

---

## 4. Test plan

### 4a. `src/config.rs::tests` — additions

Required cases (each ~15-25 lines, mirror the existing `parse_str` /
`assert_parse_rejects_with` patterns):

1. **`allow_lints_default_is_empty`** — parse without the key; assert
   `cfg.suites[0].allow_lints.is_empty()`.
2. **`allow_lints_accepts_simple_lint_names`** — parse with
   `allow_lints = ["unexpected_cfgs", "dead_code"]`; assert the resolved suite
   has both entries in order.
3. **`allow_lints_accepts_clippy_namespaced_lints`** — parse with
   `allow_lints = ["clippy::needless_collect"]`; assert it round-trips.
   (Confirms `::` is not rejected by structural validation.)
4. **`allow_lints_rejects_empty_string`** — `allow_lints = [""]` →
   `ConfigInvalid` containing `"allow_lints"`.
5. **`allow_lints_rejects_leading_dash`** — `allow_lints = ["-A unexpected_cfgs"]`
   → `ConfigInvalid` mentioning the `-A` prefix being caller-supplied is wrong.
6. **`allow_lints_rejects_whitespace`** — `allow_lints = ["dead code"]` →
   `ConfigInvalid`.
7. **`allow_lints_rejects_quote_and_backslash`** — three parametric assertions
   for `"a\"b"`, `"a'b"`, `"a\\b"`; all reject.
8. **`allow_lints_named_suite_inherits_from_default`** — top-level
   `allow_lints = ["dead_code"]`, named suite omits the key, assert the named
   suite's resolved `allow_lints == ["dead_code"]`. Mirror the existing
   `named_suite_inherits_unspecified_keys_from_default` test at
   `src/config.rs:760-792`, which already asserts `dev_deps` /
   `extern_crates` / `edition` / `compile_fail_marker` /
   `fixture_timeout_secs` / `per_fixture_memory_mb` inheritance; preferred
   shape is to extend that test with an `allow_lints` assertion (one extra
   line near config.rs:784) rather than write a parallel sibling.
9. **`allow_lints_named_suite_overrides_default`** — top-level
   `allow_lints = ["dead_code"]`, named suite sets `allow_lints = ["unused"]`;
   assert the named suite resolves to `["unused"]` (replacement, not merge).
10. **`allow_lints_named_suite_empty_array_overrides_to_empty`** — top-level
    has lints, named suite explicitly sets `allow_lints = []`; assert resolved
    value is `[]` — important so adopters can opt-out per-suite.
11. **`raw_metadata_preserves_allow_lints`** — extend the existing
    `raw_metadata_is_preserved_verbatim` test (config.rs:736) or add a sibling
    that confirms `cfg.raw_metadata` carries the new key for the manifest
    snapshot.

### 4b. `src/worker.rs::tests` — additions

Mirror the existing `apply_feature_cfgs_*` tests (lines 1725-1748):

1. **`apply_allow_lints_emits_dash_a_per_lint`** — feed
   `["unexpected_cfgs", "dead_code"]`; assert `cmd.get_args()` equals
   `["-A", "unexpected_cfgs", "-A", "dead_code"]`.
2. **`apply_allow_lints_is_noop_for_empty_slice`** — feed `&[]`; assert
   `cmd.get_args().count() == 0`.
3. **`apply_allow_lints_handles_namespaced_lint`** — feed
   `["clippy::needless_collect"]`; assert args are `["-A", "clippy::needless_collect"]`.

These are pure-function tests; no rustc spawn. `unit_test_ctx` only needs to be
updated to initialize the new field (no test should construct contexts that
exercise spawn for this feature).

### 4c. Integration test — `tests/lihaaf_allow_lints.rs`

**Design rationale (R3).** Prior designs were vacuous:
- **R1** used a `CompilePass` fixture, but `Verdict::Ok` for `(CompilePass, true)`
  at `src/worker.rs:707` does NOT consume stderr — the test would pass even
  with `apply_allow_lints` absent.
- **R2** used a `CompileFail` fixture with `#[cfg(feature = "phantom_feature")]`
  to trigger `unexpected_cfgs`, but `unexpected_cfgs` is `--check-cfg`-gated
  and lihaaf's bare-rustc invocation at `src/worker.rs:916-919, 929-972` does
  NOT pass `--check-cfg`. Verified directly against `rustc 1.95.0` — the
  warning never fires under lihaaf's actual command line, so the R2 test
  would still pass with the implementation absent.

**R3 chooses `unused_imports`** — a default-on lint that:
1. fires under bare rustc with no `--check-cfg`;
2. emits during name resolution, BEFORE the type-check pass that aborts;
3. survives alongside a `E0308` type error in the same compilation;
4. has a stable rendered string across rustc 1.95 patch versions
   (default-on since pre-1.0; format `warning: unused import: \`<path>\``).

#### 4c.0. Load-bearing verification (DO NOT REMOVE)

The implementer must re-run this on their toolchain. Command shape mirrors
`spawn_and_monitor` (no `--check-cfg`, `--edition 2021`, `--crate-type=bin`):

```text
# probe.rs:
#   use std::collections::HashMap;
#   fn main() { let _x: u8 = "not a number"; }
#
# rustc --edition 2021 --crate-type=bin --error-format=json -o out probe.rs
#
# Observed (rustc 1.95.0): JSON stderr contains TWO diagnostics with
# extractable `rendered`:
#   1. warning: unused import: `std::collections::HashMap`
#      (code.code = "unused_imports", level = "warning")
#   2. error[E0308]: mismatched types
# Adding `-A unused_imports` suppresses (1); only (2) remains.
```

**Why not `dead_code` / `unused_variables`:** verified empirically — both fire
DURING or AFTER the type-check pass that aborts on `E0308`, so neither
appears in stderr when the fixture also has a type error. Only
`unused_imports` (name-resolution pass) survives.

#### 4c.1. Test structure

In-process lihaaf invocation (no subprocess), using `tempfile::tempdir()`
(`tempfile` in scope per `Cargo.toml:117`):

1. Synthetic adopter crate: `Cargo.toml` with `[package.metadata.lihaaf]`
   containing `dylib_crate`, `extern_crates`, `allow_lints = ["unused_imports"]`.
2. Minimal `src/lib.rs` (dylib target).
3. Fixture `tests/lihaaf/compile_fail/unused_import_and_type_error.rs`
   matching `probe.rs` from §4c.0 exactly.
4. Pre-blessed snapshot `unused_import_and_type_error.stderr` containing ONLY
   the `E0308` rendered diagnostic.

Reference pattern: the existing `tests/lihaaf/compile_fail/type_mismatch.rs`
+ `.stderr` pair (260 + 333 bytes). The new test file
`tests/lihaaf_allow_lints.rs` invokes the lihaaf library entry directly
(consult `src/lib.rs`; compat driver at `src/compat/mod.rs:11` uses the same
entry).

**Assertion path A (passes when wired):** `(CompileFail, false)` branch at
`src/worker.rs:712-742` runs `diff::unified_diff(&expected, &normalized)`,
empty diff with the lint suppressed → `Verdict::Ok` → zero verdict failures.

**Assertion path B (fails when broken):** without `apply_allow_lints` wired,
normalized stderr contains the `unused_imports` warning; diff is non-empty
→ `Verdict::Diff { .. }` → test fails.

#### 4c.2. Counter-design verification (PR-time check)

The implementer includes in the PR description a recorded check showing that
commenting out the `apply_allow_lints` call in `spawn_and_monitor` causes
`cargo test --test lihaaf_allow_lints` to fail with a diff containing the
literal substring `unused import: \`std::collections::HashMap\``. One-shot
manual; the steady-state regression net is CI green.

#### 4c.3. Test gating (toolchain + OOM)

Two guards in the test body with distinct semantics. CI exercises both paths
green because `.github/workflows/ci.yml:48-57` sets
`LIHAAF_RUN_CARGO_BUILD_TESTS=1` and ships a rustc.

1. **Skip-guard — `LIHAAF_RUN_CARGO_BUILD_TESTS` opt-in.** This test drives
   `lihaaf::run` end-to-end, which builds the synthetic adopter's dylib via
   a real `cargo rustc` invocation against a tempdir target — the same
   OOM-prone shape as `cargo_accepts_staged_overlay_for_dylib_build`
   (`tests/compat/overlay_determinism.rs:689` — verified, gate logic at
   lines 694-701). Local RAM-limited boxes (e.g. 4 GB WSL2) OOM when this
   runs alongside `cargo test --all-features`. When `LIHAAF_RUN_CARGO_BUILD_TESTS`
   is **not set**, the test emits `eprintln!("skipped: ...")` and returns
   early (NOT panic — this is the intentional local skip path). The `Test`
   step in `ci.yml:48-57` opts in by setting the env var, so the regression
   net stays authoritative in CI. Mirror the literal skip-message format from
   `tests/compat/overlay_determinism.rs:695-700` so the gate reads identically
   across the test suite.

2. **Panic-guard — toolchain presence.** This guard fires ONLY when
   `LIHAAF_RUN_CARGO_BUILD_TESTS` IS set (i.e. the skip-guard was passed).
   If `LIHAAF_RUN_CARGO_BUILD_TESTS` is set AND `rustc` is not on `$PATH`,
   the test `panic!`s — this signals a misconfigured CI environment rather
   than an expected local skip. lihaaf's existing test suite assumes a
   present rustc once opted in; a silent no-op under a missing toolchain
   would produce a false green in CI. This panic is NOT a skip path.

The guard ordering in the test body: check skip-guard first (env var absent →
return early), THEN check panic-guard (env var present but rustc missing →
panic). This ensures the panic only fires in opted-in contexts where a missing
toolchain is a real environment failure.

#### 4c.4. Alternatives rejected

- **Option B (new public stderr accessor):** expands public API for one test.
- **Option C (subprocess `cargo lihaaf`):** heavier, brittle, no in-tree precedent.
- **Option D (drop integration test):** unit tests 4b prove argv shape but
  not invocation; no guard against a future refactor dropping the call site.
- **Option E (add `--check-cfg` + use `unexpected_cfgs`):** separate feature,
  forces snapshot re-blessing across pilots, broadens scope.
- **Option F (custom infrastructure bypassing snapshot path):** unnecessary
  complexity; snapshot path is already the right vehicle.

### 4d. Compat-mode unit tests — `src/compat/overlay.rs::tests`

Three test cases. Each calls `inject_synthetic_metadata` directly with a
constructed input `top: toml::map::Map` and asserts on the post-call shape.

**4d.1 `synthetic_metadata_injects_allow_lints`** — empty input table.
Construct a `SyntheticMetadata` with
`allow_lints = vec!["unexpected_cfgs".to_string()]`, call
`inject_synthetic_metadata` on an empty `top`, parse the resulting TOML, and
assert `top["package"]["metadata"]["lihaaf"]["allow_lints"]` is the array
`["unexpected_cfgs"]`. No existing test in `src/compat/overlay.rs::tests`
covers `inject_synthetic_metadata` directly (R6-verified — the module's
existing tests focus on `canonicalize_*`, `absolutize_*`, comment
scanning); this test establishes the precedent. Test 4d.3 below extends
the same shape for the REPLACES invariant.

**4d.2 `synthetic_metadata_default_in_compat_driver`** — pin the
compat-driver's literal. Call the compat-driver construction site at
`src/compat/mod.rs:151-155` (extract into a `pub(crate)` helper if needed so
the test can exercise it without a full compat run), and assert the
constructed `SyntheticMetadata.allow_lints == ["unexpected_cfgs"]`. This pins
the default in case a future refactor accidentally drops it.

**4d.3 `synthetic_metadata_replaces_upstream_allow_lints` (R3-new — FIX_BEFORE_MERGE-2).**
Pin the replacement invariant at `src/compat/overlay.rs:1051-1058` (which
documents that the synthetic metadata REPLACES any pre-existing
`[package.metadata.lihaaf]` block in full). Construct an input `top` whose
`[package.metadata.lihaaf]` table already contains
`allow_lints = ["some_other_lint"]` (use a value that is structurally valid
under the v0.1 config schema). Call `inject_synthetic_metadata` with a
synthetic metadata whose `allow_lints = vec!["unexpected_cfgs".to_string()]`.
Assert that the post-call `top["package"]["metadata"]["lihaaf"]["allow_lints"]`
is `["unexpected_cfgs"]` — i.e. the upstream value is REPLACED, not merged or
preserved.

Why this is load-bearing: a future regression that switches the
implementation to partial-merge semantics (preserving upstream
`allow_lints = ["some_other_lint"]` and ignoring the synthetic
`["unexpected_cfgs"]`) would silently undo the "compat owns inner config"
invariant in a way no in-source test currently catches. Under v0.1.0 the
observable effect is muted (the `unexpected_cfgs` lint is dead anyway), but
the day `--check-cfg` becomes active OR the day the compat default is
broadened to include v0.1-active lints (a tractable post-merge follow-up
per §7), a partial-merge regression would deliver the wrong suppression to
every compat-mode pilot. The existing replacement comment at lines
1051-1058 is documentation only; this test makes the invariant executable.

Construct the upstream table programmatically (not via TOML parse) for
clarity:

```rust
let mut upstream_lihaaf = toml::map::Map::new();
upstream_lihaaf.insert(
    "allow_lints".to_string(),
    toml::Value::Array(vec![toml::Value::String("some_other_lint".to_string())]),
);
// Then nest: top["package"]["metadata"]["lihaaf"] = upstream_lihaaf
// (matches the shape `inject_synthetic_metadata` walks).
```

### 4e. Minimum coverage

The three unit clusters above (config validation, suite inheritance, worker
arg-shape) plus three compat-synthesis unit tests plus the one integration test
are the minimum lock-in. Skipping any of:

- inheritance test (4a #8) → silent precedent drift if a future contributor
  changes `dev_deps` to merge instead of replace and forgets `allow_lints`;
- empty-array override (4a #10) → adopters lose the per-suite opt-out;
- worker arg-shape (4b #1) → no regression guard against future refactor that
  drops the `-A` flag spelling;
- compat-synthesis injection test (4d.1) → no guard against
  `inject_synthetic_metadata` dropping the field from the synthesized TOML;
- compat-synthesis default test (4d.2) → silent regression if a future
  compat-mode refactor drops the `allow_lints` field from the
  `SyntheticMetadata` literal at `src/compat/mod.rs:151-155`. Observable
  effect is currently nil (forward-only insurance, §3d) but the regression
  would only surface on the day `--check-cfg` becomes active under lihaaf;
  the test pins the default so that future shift is uneventful;
- compat-synthesis replacement test (4d.3, R3-new) → silent regression if a
  future refactor switches to partial-merge semantics, allowing upstream
  conflicting `allow_lints` values to mask the synthetic default;
- integration test (4c) → no regression net for the `spawn_and_monitor`
  wiring of `apply_allow_lints` to per-fixture rustc.

### 4f. Edge cases pinned by tests

| Case | Test that pins it |
|---|---|
| empty array (default, v0.1 path) | 4a #1 |
| empty string element | 4a #4 |
| caller includes `-A` | 4a #5 |
| whitespace | 4a #6 |
| quote / backslash | 4a #7 |
| `clippy::` namespace | 4a #3, 4b #3 |
| named-suite inheritance | 4a #8 |
| named-suite override | 4a #9 |
| named-suite empty override | 4a #10 |
| manifest round-trip | 4a #11 |
| end-to-end suppression on compile_fail path | 4c |
| compat synthesis injects key | 4d.1 |
| compat synthesis default value | 4d.2 |
| compat synthesis REPLACES upstream conflict | 4d.3 (R3-new) |

---

## 5. Edge cases identified (non-test concerns)

- **Unknown lint name** (e.g., `allow_lints = ["does_not_exist"]`): rustc
  emits `warning: unknown lint: \`does_not_exist\`` and continues. This warning
  WILL appear in fixture stderr. **Decision: do not pre-validate.** Rationale:
  (a) lihaaf can't enumerate every rustc lint without binding to a toolchain
  version; (b) the warning is self-documenting and points the adopter at their
  own typo; (c) Round-2 adopters will see the warning surface clearly via the
  existing snapshot diff. Document this in the new key's rustdoc.

- **Namespaced lints (`clippy::*`, `rustdoc::*`)**: pass through verbatim;
  rustc accepts these when the relevant tool is registered. Tests 4a #3 and
  4b #3 pin this.

- **Group lints (`-A warnings`, `-A unused`)**: pass through verbatim; rustc
  accepts these. Not pinned by a dedicated test but covered by the generic
  arg-shape test (4b #1) since lihaaf does no semantic interpretation.

- **Ordering / duplicates**: `-A` flags are idempotent (rustc accepts duplicates
  and the last wins for same-level overrides). No de-duplication in
  `apply_allow_lints` — preserves the "config → rustc verbatim" mental model.
  If duplicates cause real problems, that's a follow-up.

- **Argv quoting**: structural validation rejects whitespace + quote chars
  (4a #6, 4a #7), so each lint is one literal argv token. No shell expansion
  ever happens because lihaaf uses `std::process::Command::arg` (not a shell
  string).

- **Interaction with `-D warnings`-style adopter flags**: out of scope here —
  lihaaf doesn't synthesize `-D` flags today, and #42 (RUSTFLAGS) is the
  workstream that touches that surface.

- **Compat-mode default and adopter expectations**: the compat-default
  `["unexpected_cfgs"]` is automatic only in compat mode. A pilot fork that
  WANTS to surface `unexpected_cfgs` (rare — diagnostic-of-cfg-emitting
  macros) must drop out of compat mode and use the v0.1 TOML-driven path
  with `allow_lints = []`. Document this trade-off in the spec §3.6.

---

## 6. Alternatives considered + why rejected

### v0.1 TOML-driven path defaults

- **Always inject `-A unexpected_cfgs` automatically (v0.1 path).** Rejected:
  silently suppresses a lint that exists for a reason. An adopter who
  *intentionally* uses cfg-feature gates in fixtures (to test diagnostic
  output of macros that emit `#[cfg]`) would lose signal. Opt-in is the safer
  default for adopters who deliberately wrote `[package.metadata.lihaaf]`.

- **Per-fixture annotation only (`#![allow(...)]` at file head).** Rejected:
  this is the status quo and is exactly what the issue calls "permanent
  snapshot noise / hand-annotation tax." Round-2 enrollment of `derive_more`
  has ~20 fixtures; that's 20 hand-edits today.

- **`LIHAAF_ALLOW_LINTS` env var.** Rejected: violates the "TOML-only,
  no env-var fallback" principle stated in `src/config.rs:7-11`. Configuration
  is committed alongside fixtures so reproduction is deterministic across
  contributors.

- **At-most-once enforcement** (reject duplicates in `allow_lints`). Rejected:
  rustc tolerates duplicates and lihaaf adds no value by being stricter than
  the underlying tool. Keeps validation surface small.

- **Suite-level inheritance via append/merge** (top-level + suite-level concatenated).
  Rejected: the `dev_deps` precedent at config.rs:461-463 is replacement, not
  merge. Mirroring that precedent keeps the mental model consistent — "named
  suite either inherits the whole list, or fully replaces it."

### Compat-mode defaults (R2-new)

- **(b) Preserve upstream `[package.metadata.lihaaf].allow_lints` via partial
  merge.** Rejected: contradicts the existing "compat owns inner config"
  invariant at `src/compat/overlay.rs:347-353` and `1051-1058`. The synthesis
  intentionally REPLACES the upstream block in full because the upstream
  block's `extern_crates` / `fixture_dirs` would not match the
  compat-driver's converted-fixtures layout. Adding a partial-merge for
  exactly one key creates a "mostly-replace" semantic that is harder to
  reason about than "always replace, opinionated defaults."

- **(c) Env-var / CLI flag (`LIHAAF_COMPAT_ALLOW_LINTS=...`).** Rejected: nobody
  would discover it; the point of in-source injection is that the default is
  present without runtime configuration. If a future expert-user wants
  override, they can construct `SyntheticMetadata` directly (the override
  seam in §3d). Compat mode is not a stable v0.1 API surface
  (`src/compat/mod.rs:17-18`), so we don't owe a flag/env API yet.

- **No compat-mode change (rescope).** Rejected: forward-only insurance is
  cheap to add now (one field on `SyntheticMetadata`, one literal in the
  compat driver, three unit tests). Skipping it means the day lihaaf or
  rustc starts passing `--check-cfg`, every compat-mode pilot under that
  toolchain immediately gets noise in normalized snapshots, requiring
  either a same-PR fix that touches every Round-1+ pilot's baseline or
  the same compat-driver field added later under release pressure.
  Adding the field now in #43 — where the field already touches every
  surface that would need to change for the forward-shift — is the
  cheaper sequencing. **NOTE:** this rationale does not depend on
  Round-2 enrollment timing; #43's value to Round-2 is on the v0.1
  TOML path (§2), not the compat-mode default.

---

## 7. Risks / unknowns

- **Inheritance-precedent coverage is already in place** (R6 correction —
  R5 and earlier mistakenly said no such test existed). The test
  `named_suite_inherits_unspecified_keys_from_default` at
  `src/config.rs:760-792` explicitly asserts `spatial.dev_deps ==
  vec!["serde".to_string()]` at line 784, plus `extern_crates`, `edition`,
  `compile_fail_marker`, `fixture_timeout_secs`, `per_fixture_memory_mb`.
  Test 4a #8 mirrors this exact pattern with `allow_lints` added; the
  implementer should extend the existing test (preferred) or add a sibling
  alongside it.

- **Integration test runtime cost.** The new test in §4c spawns rustc
  end-to-end (via lihaaf internals on a tempdir). It is one of the slowest
  tests in `tests/` after `cleanup_dirty_worktree.rs`. Adding one more
  in-tree is acceptable; adding three would warrant a feature-gate to keep CI
  fast. Sticking to one is the plan.

- **Manifest schema versioning (v0.1 TOML path).** Adding a key does not bump
  the manifest schema version (existing precedent: every key addition in
  `Suite` shipped the same way). The §3.3 compat envelope downstream cares
  about schema version separately; the envelope already accommodates additive
  keys, so this should land without an envelope schema bump.

- **Lint-name shadowing across rustc versions.** A lint that exists today may
  be renamed in a future toolchain; the adopter's `allow_lints` entry would
  surface as `unknown lint`. This is expected behavior and self-healing once
  the adopter updates their TOML. Documented in the new key's rustdoc.

- **Compat-default future evolution.** If the compat-mode default
  `["unexpected_cfgs"]` proves wrong for some Round-3+ pilot (e.g. a pilot
  that legitimately wants to surface `unexpected_cfgs` and can't drop out of
  compat mode), the fix is to expose a CLI/env override surface — but that's
  a v0.2 conversation, not a #43 conversation.

- **Compat-default narrowness vs Round-2 pilot reality.** The compat-mode
  default is `["unexpected_cfgs"]` only, but `unused_imports` / `dead_code`
  / similar default-on lints will produce snapshot noise TODAY under v0.1
  lihaaf for Round-2 pilots (`derive_more`, `axum-macros`). This plan does
  NOT solve Round-2 Day-1 noise via the compat default — the compat
  default is forward-only insurance for the `--check-cfg` future (§1, §3d).
  Round-2 Day-1 noise is solved via the **v0.1 TOML path** (§2): pilot
  forks add `unused_imports` / `dead_code` / etc. to their own
  `[package.metadata.lihaaf].allow_lints`. The plan intentionally keeps
  the compat default narrow because (a) the v0.1-path is the right channel
  for v0.1-active lints (the adopter who knows what their fixtures need
  declares it explicitly), (b) an opinionated wider compat-default could
  mask intentional lint surfacing, and (c) the override seam in
  `SyntheticMetadata` lets the compat-driver caller widen the list
  per-pilot when needed. Decision: ship narrow compat default
  (forward-only); rely on v0.1-path adopter config for v0.1-active lints.

---

## 8. Backward compatibility

### 8a. v0.1 TOML path

`allow_lints` defaults to `[]` (empty) when omitted from TOML on the v0.1
adopter path. `[]` produces zero `-A` flags in argv (verified by 4b #2). All
existing tests, fixtures, and snapshots run with `[]` and are unaffected.

The manifest snapshot (`raw_metadata`) will gain the key only when the adopter
sets it — `toml::Value` does not synthesize missing keys.

### 8b. Compat-mode change (on-disk overlay TOML only — NOT the §3.3 envelope)

Compat-mode synthesis writes `allow_lints = ["unexpected_cfgs"]` into the
synthesized `[package.metadata.lihaaf]` table at
`<upstream>/target/lihaaf-overlay/Cargo.toml`.

**Envelope and baseline.toml: unaffected.** Verified against
`src/compat/report.rs:104-139, 286-301`: `CompatEnvelope.overlay` is
`OverlayMetadata` which contains only `generated`, `dropped_comments`,
`upstream_already_has_dylib`. The synthetic metadata block is never written
into the envelope, so adding `allow_lints` to `SyntheticMetadata` does not
change envelope bytes or baseline.toml signature. Round-1 pilots
(`anyhow`, `cxx`, `serde_json`, `thiserror`) require no re-blessing.

### 8c. Out-of-scope follow-up

Envelope visibility of `synthetic_metadata.allow_lints` for debugging would
require adding the field to `OverlayMetadata` and threading it through driver
population — a separate work item, explicitly OUT OF SCOPE for #43.

---

## 9. Documentation updates (R2-new — FIX_BEFORE_MERGE-3)

All doc changes land in the SAME implementation PR.

### 9a. `README.md` updates

**§Quick start (around lines 41-48).** The example `[package.metadata.lihaaf]`
config block currently shows `dylib_crate`, `extern_crates`, `features`,
`dev_deps`, `edition`. Add a one-line `# allow_lints` example AND a brief
inline comment explaining what it does — keep the §Quick start example
concise (it's the marketing surface). Suggested addition:

```toml
[package.metadata.lihaaf]
dylib_crate = "consumer"
extern_crates = ["consumer", "consumer-macros"]
features = ["testing"]
dev_deps = ["serde", "serde_json"]
edition = "2021"
# Suppress rustc lints on each per-fixture invocation (forwarded as `-A <lint>`).
# Common entries under v0.1: unused_imports and dead_code, which fire under
# lihaaf's bare-rustc invocation when fixtures share scaffolding with the
# main crate. Other entries (e.g. unexpected_cfgs) become active when
# rustc is invoked with --check-cfg; today lihaaf does not pass --check-cfg.
allow_lints = ["unused_imports", "dead_code"]
```

**§Multi-suite (around lines 132-135).** The inheritance bullet list at lines
132-135 enumerates which keys inherit from the top-level table. Append
`allow_lints` to that list:

```
- Other keys (`extern_crates`, `dev_deps`, `edition`,
  `compile_fail_marker`, `fixture_timeout_secs`,
  `per_fixture_memory_mb`, `allow_lints`) inherit from the top-level
  table when omitted on a named suite.
```

### 9b. `docs/spec/lihaaf-v0.1.md` updates

**§3.2 Schema (lines 303-347).** Add the new key to the schema block with
DEFAULT and behavior description:

```toml
# DEFAULT: []. rustc lints to forward as `-A <lint>` on every per-
# fixture invocation. Each entry becomes one `-A` argv flag.
# Validation rejects empty strings, leading-dash entries (caller
# should not supply `-A` prefix), and entries containing whitespace,
# quotes, or backslashes. Unknown lint names are NOT pre-validated —
# rustc surfaces `warning: unknown lint: X` on the per-fixture stderr.
allow_lints = ["unexpected_cfgs", "dead_code"]
```

**Note on example divergence (R6):** the spec example above uses
`["unexpected_cfgs", "dead_code"]` (canonical illustration: one compat-
default entry plus one v0.1-active lint), while §9a's README example uses
`["unused_imports", "dead_code"]` (the v0.1-active pair Round-2 forks
actually configure). Both forms are valid; the difference is intentional —
the spec illustrates the schema with a representative example, the README
illustrates what a Round-2 fork looks like. Reviewers tempted to demand
one-or-the-other should see this note before flagging the difference.

**§3.6 Inheritance rules (lines 476-478).** Append `allow_lints` to the list
of inheriting keys:

```
- `extern_crates`, `dev_deps`, `edition`, `compile_fail_marker`,
  `fixture_timeout_secs`, `per_fixture_memory_mb`, and `allow_lints`
  DO inherit from the default suite when omitted.
```

**§3.6 (or new §3.7) — Compat-mode default callout.** Add a short subsection
beginning verbatim with the phrase `` Compat-mode default `allow_lints = ["unexpected_cfgs"]` ``
(this exact wording is mandatory so the §9d C.4 anchor-text grep verifies it landed —
do not paraphrase). The callout documents why this default exists
(forward-only insurance for the day lihaaf or rustc passes `--check-cfg` —
today the default is a no-op because the lint is check-cfg-gated and lihaaf
does not pass that flag, verified at `src/worker.rs:916-919, 929-972`), and
how to override (drop out of compat mode and use v0.1 TOML-driven path).
Place this near the existing compat-mode references in the spec, or under
a new "Compat-mode interactions" subsection if no natural anchor exists.

**C.1 — Named-suite inheritance list in spec example (line 446).** The
named-suite config example at spec line 446 has an inline comment listing
which keys inherit from the top-level table:

```
# extern_crates, dev_deps, edition, compile_fail_marker,
# fixture_timeout_secs, per_fixture_memory_mb all inherit from the
# top-level table when omitted on a named suite.
```

Add `allow_lints` to this comment so the named-suite example stays consistent
with the §3.6 inheritance rules text. Suggested update:

```
# extern_crates, dev_deps, edition, compile_fail_marker,
# fixture_timeout_secs, per_fixture_memory_mb, allow_lints all
# inherit from the top-level table when omitted on a named suite.
```

**C.2 — Validation bullets (spec line 393).** The config-validation bullet
list at spec line 388-400 enumerates all conditions that cause a hard error at
startup. Add `allow_lints` rejection cases:

```
- An entry in `allow_lints` is an empty string.
- An entry in `allow_lints` starts with `-` (caller must not supply the
  `-A` prefix; lihaaf supplies it).
- An entry in `allow_lints` contains whitespace, double quotes, single
  quotes, or backslashes (would break argv tokenization).
```

Place these bullets after the `per_fixture_memory_mb` bullet (currently the
last entry in the list). Document clearly that unknown lint names are NOT
pre-validated — rustc surfaces `warning: unknown lint: X` itself.

**C.3 — Canonical per-fixture rustc argv (spec line 785).** The rustc
invocation block at spec lines 785-800 enumerates all required flags in order.
The `[--cfg feature="<feat>"]` rows are the last optional rows before
`<fixture.rs>`. Add the new optional `-A` rows in the same position group,
AFTER feature cfgs and BEFORE the source file:

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
    [-A <lint1>]
    [-A <lint2>]
    <fixture.rs>
```

Add a corresponding "Where:" bullet: "`-A <lint>`: one flag pair per entry in
the suite's `allow_lints`; omitted when the list is empty; forwarded verbatim
to rustc with no shell expansion (lihaaf uses `Command::arg`, not a shell
string)."

**C.6 — `src/config.rs:15` module rustdoc suite-field enumeration.** The
module-level rustdoc at `src/config.rs:13-18` (the `## Suites` section)
enumerates the per-suite fields:

```
A *suite* is a named bundle of (features, fixture_dirs, edition,
dev_deps, extern_crates, compile_fail_marker, fixture_timeout_secs,
per_fixture_memory_mb).
```

The implementer must add `allow_lints` to this enumeration:

```
A *suite* is a named bundle of (features, fixture_dirs, edition,
dev_deps, extern_crates, compile_fail_marker, fixture_timeout_secs,
per_fixture_memory_mb, allow_lints).
```

**C.7 — `src/worker.rs:105` `WorkerContext::new` rustdoc.** The rustdoc for
`WorkerContext::new` at `src/worker.rs:100-110` enumerates the per-suite fields
read from `suite`:

```
/// Everything else (`features`, `extern_crates`, `edition`, `dev_deps`,
/// timeout, memory ceiling) is per-suite and read from `suite`.
```

The implementer must append `allow_lints` to this enumeration:

```
/// Everything else (`features`, `extern_crates`, `edition`, `dev_deps`,
/// timeout, memory ceiling, `allow_lints`) is per-suite and read from `suite`.
```

### 9c. `CHANGELOG.md` entry under `[Unreleased]`

**Confirmed live state (lines 1-9):** `CHANGELOG.md:7` reads `## [Unreleased]`
and is followed by a blank line at `:8` and the `## [0.1.0-beta.6]` heading
at `:9`. The new entry slots between lines 7 and 9.

Suggested entry (the implementer can adjust wording):

```markdown
## [Unreleased]

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
```

### 9d. Doc-change verification (mixed CI / PR-paste reviewer-rerun, R5-aligned with §10)

One verification is CI-enforced; the rest are PR-paste reviewer-reproducible
per §10's honest framing. The implementer's PR exercises:

1. **`RUSTDOCFLAGS="-D warnings" cargo doc --no-deps`** (already in
   `.github/workflows/ci.yml` at the "cargo doc (warnings as errors)" step —
   verified at `ci.yml:163-166`) — the new rustdoc on `Suite.allow_lints` and
   `SyntheticMetadata.allow_lints` must not warn. Pre-existing CI guard; no new
   workflow change required. Note: the CI step uses the `RUSTDOCFLAGS` env var,
   NOT the `cargo doc -D warnings` argv form — the env-var form is what actually
   propagates to rustdoc. Use `RUSTDOCFLAGS="-D warnings" cargo doc --no-deps`
   for local pre-push verification to match CI exactly. This item IS automated
   by CI (§10a).

2. **Documentation and rustdoc content checks.** The PR description includes the exact
   `grep -F` invocations that confirm the documentation landed. These are
   suitable for a reviewer to run locally OR for the implementer to paste
   the command + output into the PR description:

   ```bash
   # README §Quick start: allow_lints example value present
   grep -F 'allow_lints = ["unused_imports", "dead_code"]' README.md
   # README §Quick start: explanatory comment present
   grep -F '# Suppress rustc lints on each per-fixture invocation' README.md

   # README §Multi-suite: inheritance list appends allow_lints to the key
   # enumeration. Discriminating anchor includes the neighboring key and the
   # sentence fragment — vacuous bare `grep -F 'allow_lints'` replaced.
   grep -F '`per_fixture_memory_mb`, `allow_lints`) inherit from the top-level' README.md

   # Spec §3.2 schema: description keyword present (A.1 fix — two independent
   # greps, not piped: "allow_lints" and "rustc lints" are on different lines in
   # the planned spec block, so a piped grep would silently fail; the §3.2
   # example value is verified by the discriminating grep in the block below).
   grep -F 'rustc lints' docs/spec/lihaaf-v0.1.md

   # Spec §3.6 inheritance rules: allow_lints appended to the DO-inherit list.
   # The bare `grep -F 'DO inherit'` was vacuous — the line already exists in
   # the spec without allow_lints. Anchor now includes the planned append text.
   grep -F '`fixture_timeout_secs`, `per_fixture_memory_mb`, and `allow_lints`' docs/spec/lihaaf-v0.1.md

   # §3.2 schema: example value present (discriminates from any other allow_lints
   # occurrence — spec example uses the compat-default + v0.1-active pair).
   grep -F 'allow_lints = ["unexpected_cfgs", "dead_code"]' docs/spec/lihaaf-v0.1.md
   # §3.6 inheritance rules: allow_lints appended to the DO-inherit key list
   # (same anchor as the A.2 fix above; repeated here for section traceability).
   grep -F '`fixture_timeout_secs`, `per_fixture_memory_mb`, and `allow_lints`' docs/spec/lihaaf-v0.1.md
   # C.1 named-suite comment: allow_lints appended to per-suite inheritance note.
   grep -F '# fixture_timeout_secs, per_fixture_memory_mb, allow_lints all' docs/spec/lihaaf-v0.1.md
   # C.2 validation bullet: starts-with-dash rejection case (discriminating anchor).
   grep -F 'An entry in `allow_lints` starts with `-`' docs/spec/lihaaf-v0.1.md
   # C.2 validation bullet: whitespace/quote/backslash rejection case.
   grep -F 'An entry in `allow_lints` contains whitespace, double quotes, single' docs/spec/lihaaf-v0.1.md
   # C.2 validation bullet: empty-string rejection case (discriminating anchor
   # replaces vacuous `grep -F 'empty string'` which matched pre-existing text).
   grep -F 'An entry in `allow_lints` is an empty string.' docs/spec/lihaaf-v0.1.md

   # C.3: Canonical argv block includes -A flag rows (spec ~line 785)
   grep -F '[-A <lint' docs/spec/lihaaf-v0.1.md

   # C.4: Compat-mode default callout section in spec (discriminating anchor
   # — bare `grep -F 'unexpected_cfgs'` was vacuous because §3.2 schema
   # example also contains that string; this anchor requires the callout text).
   grep -F 'Compat-mode default `allow_lints = ["unexpected_cfgs"]`' docs/spec/lihaaf-v0.1.md

   # C.5: config.rs:4-5 module rustdoc qualification re manifest.rs guidance
   # (discriminating anchor includes the qualifying clause; vacuous bare
   # `grep -F 'allow_lints'` replaced — many other sites would satisfy it).
   grep -F 'such as `dev_deps`, `edition`, and `allow_lints`) are preserved verbatim' src/config.rs
   # C.6: config.rs:13-18 suite-field enumeration appends allow_lints.
   grep -F 'per_fixture_memory_mb, allow_lints).' src/config.rs

   # C.7: WorkerContext::new rustdoc appends allow_lints to the per-suite field
   # enumeration (discriminating anchor; vacuous bare grep replaced).
   grep -F 'timeout, memory ceiling, `allow_lints`) is per-suite and read from `suite`' src/worker.rs

   # CHANGELOG [Unreleased] — both planned bullets are present.
   # Single `grep -F 'allow_lints'` was vacuous: either bullet alone satisfies
   # it; expanded into two discriminating greps, one per planned entry.
   #
   # NOTE: the comma-form awk range `/^## \[Unreleased\]/,/^## \[/`
   # terminates on the same line as the start pattern (both regexes
   # match `## [Unreleased]`), so it prints only the header. Use the
   # flag-based form below to start AFTER the header and stop at the
   # next heading.
   awk '/^## \[Unreleased\]/{flag=1; next} /^## \[/{flag=0} flag' CHANGELOG.md \
     | grep -F '**`allow_lints` config key**'
   awk '/^## \[Unreleased\]/{flag=1; next} /^## \[/{flag=0} flag' CHANGELOG.md \
     | grep -F '**Compat-mode default `allow_lints = ["unexpected_cfgs"]`**'
   ```

   Each grep MUST return at least one matching line. The implementer pastes
   the actual `grep` output into the PR description (no screenshots, no
   rendered HTML). Reviewer runs the same greps locally for verification.

3. **CHANGELOG `[Unreleased]` non-empty.** Enforced by the last grep above.
   R6-verified: no CHANGELOG-discipline test exists in the repo
   (`grep -rn "CHANGELOG\|Unreleased" src/ tests/` returned no matches at
   commit `362bec0`); the grep IS the gate.

---

## 10. CI-enforcement audit (R4-revised — explicit honesty about what is and is NOT in CI)

This section was rewritten in R4. R3 framed every regression net as "CI
without developer-attention." Codex R3 BLOCKed on two items where that claim
is false: doc-content greps (§9d) require an implementer/reviewer to actually
run them, and compat-mode behavior in pilots runs under
`refresh-pilots.yml` which is `workflow_dispatch` only against a PUBLISHED
crates.io version — not an automatic check on this PR's branch.

R4 takes the **downgrade path**: explicitly state per category what is
CI-enforced on the PR vs what requires manual / PR-paste verification. No
new CI infrastructure is added for v0.1.0 (each addition is its own scope
risk). The honest documentation is sufficient given that the load-bearing
correctness is captured by unit + integration tests that DO run in CI.

**Process note (R10):** Before each R-N review dispatch, verify that plan line 3 (`**Revision: ...**`) matches the latest revision-history bullet AND the review prompt's target revision. Header-history-prompt skew is a recurring source of false BLOCK findings.

### 10a. Fully CI-enforced on this PR (no developer attention)

These run automatically on every push and PR via `.github/workflows/ci.yml`
on `branches: [main]` for push and PR-to-main. The diff in this PR triggers
all of them.

| Section | Mechanism | CI step in ci.yml |
|---|---|---|
| §3a config-key + 4a validation/inheritance tests | `cargo test` | `cargo test` (line 57) |
| §3b worker + 4b arg-shape tests | `cargo test` | `cargo test` (line 57) |
| §3d compat injection + 4d.1/4d.2/4d.3 unit tests | `cargo test` | `cargo test` (line 57) |
| §4c integration test wiring (`tests/lihaaf_allow_lints.rs`) [^lihaaf-build-tests-gate] | `cargo test` | `cargo test` (line 57) with `LIHAAF_RUN_CARGO_BUILD_TESTS=1` (line 56) |
| Self-test corpus must stay green with new flags wired | `cargo run --bin cargo-lihaaf -- lihaaf` | `Self-test corpus end-to-end` step (line 59-84) |
| §9 rustdoc on `Suite.allow_lints` / `SyntheticMetadata.allow_lints` | `RUSTDOCFLAGS=-D warnings cargo doc --no-deps` | `cargo doc (warnings as errors)` step (line 163-166) |
| `cargo fmt --all -- --check` and `cargo clippy --all-targets -- -D warnings` apply to the new code | formatter / lint enforcement | `Check formatting` (line 39-40), `Clippy` (line 42-43) |

A failure in any of these BLOCKS the PR via the standard CI gate.

[^lihaaf-build-tests-gate]: The integration test under §4c builds a synthetic
adopter dylib, which is the same OOM-prone shape as
`cargo_accepts_staged_overlay_for_dylib_build`. Per §4c.3 it is gated by
`LIHAAF_RUN_CARGO_BUILD_TESTS=1` so local RAM-limited boxes can skip it
without the CI lane losing the regression net (CI sets the env var at
`ci.yml:56`). A no-rustc local box still runs `cargo test`; the gate
`eprintln!`s + `return`s — green locally, authoritative in CI.

### 10b. PR-paste reviewer-reproducible (NOT automated in CI)

These are NOT automated by `ci.yml`. The implementer runs them locally and
pastes the literal command + output into the PR description; the reviewer
re-runs the same commands to verify.

| Section | Mechanism | Where it runs |
|---|---|---|
| §9a README §Quick start has `allow_lints` example | `grep -F 'allow_lints = ["unused_imports", "dead_code"]' README.md` from §9d | implementer paste + reviewer rerun |
| §9a README §Quick start explanatory comment present | `grep -F '# Suppress rustc lints on each per-fixture invocation' README.md` from §9d | implementer paste + reviewer rerun |
| §9a README §Multi-suite inheritance list appends `allow_lints` | `grep -F '`per_fixture_memory_mb`, `allow_lints`) inherit from the top-level' README.md` | implementer paste + reviewer rerun |
| §9b spec §3.2 schema description keyword present | `grep -F 'rustc lints' docs/spec/lihaaf-v0.1.md` from §9d | implementer paste + reviewer rerun |
| §9b spec §3.2 schema example value present | `grep -F 'allow_lints = ["unexpected_cfgs", "dead_code"]' docs/spec/lihaaf-v0.1.md` from §9d | implementer paste + reviewer rerun |
| §9b spec §3.6 inheritance list appends `allow_lints` (C.A2 fix) | `grep -F '`fixture_timeout_secs`, `per_fixture_memory_mb`, and `allow_lints`' docs/spec/lihaaf-v0.1.md` | implementer paste + reviewer rerun |
| §9b C.1 named-suite comment appends `allow_lints` | `grep -F '# fixture_timeout_secs, per_fixture_memory_mb, allow_lints all' docs/spec/lihaaf-v0.1.md` | implementer paste + reviewer rerun |
| §9b C.2 spec validation bullets — starts-with-dash rejection case | `grep -F 'An entry in `allow_lints` starts with `-`' docs/spec/lihaaf-v0.1.md` | implementer paste + reviewer rerun |
| §9b C.2 spec validation bullets — whitespace/quote/backslash rejection case | `grep -F 'An entry in `allow_lints` contains whitespace, double quotes, single' docs/spec/lihaaf-v0.1.md` | implementer paste + reviewer rerun |
| §9b C.2 spec validation bullets — empty-string rejection case | `grep -F 'An entry in `allow_lints` is an empty string.' docs/spec/lihaaf-v0.1.md` | implementer paste + reviewer rerun |
| §9b C.3 canonical argv block includes `-A <lint>` row | `grep -F '[-A <lint' docs/spec/lihaaf-v0.1.md` | implementer paste + reviewer rerun |
| §9b C.4 compat-mode default callout section present in spec | `grep -F 'Compat-mode default `allow_lints = ["unexpected_cfgs"]`' docs/spec/lihaaf-v0.1.md` | implementer paste + reviewer rerun |
| §3c.2 C.5 `src/config.rs` module rustdoc qualified re manifest.rs guidance | `grep -F 'such as `dev_deps`, `edition`, and `allow_lints`) are preserved verbatim' src/config.rs` | implementer paste + reviewer rerun |
| §3c.2 C.6 `src/config.rs:15` suite-field enumeration appends `allow_lints` | `grep -F 'per_fixture_memory_mb, allow_lints).' src/config.rs` | implementer paste + reviewer rerun |
| §3b C.7 `WorkerContext::new` rustdoc appends `allow_lints` | `grep -F 'timeout, memory ceiling, `allow_lints`) is per-suite and read from `suite`' src/worker.rs` | implementer paste + reviewer rerun |
| §9c CHANGELOG `[Unreleased]` config-key bullet present | `awk '/^## \[Unreleased\]/{flag=1; next} /^## \[/{flag=0} flag' CHANGELOG.md \| grep -F '**`allow_lints` config key**'` | implementer paste + reviewer rerun |
| §9c CHANGELOG `[Unreleased]` compat-mode default bullet present | `awk '/^## \[Unreleased\]/{flag=1; next} /^## \[/{flag=0} flag' CHANGELOG.md \| grep -F '**Compat-mode default `allow_lints = ["unexpected_cfgs"]`**'` | implementer paste + reviewer rerun |

**Why not automate now:** adding a grep-based doc-content CI step is
straightforward, but every CI addition is independent scope risk for v0.1.0.
The PR-paste pattern is the established lihaaf precedent for doc-content gates
and is sufficient given that all greps are mechanical and the reviewer pass is
the same gating event. **Future work:** convert §10b items to a CI step
post-v0.1.0 if the PR-paste pattern proves friction-heavy. NOT a v0.1.0 gate.

### 10c. Manual post-merge (NOT exercised by any CI on this PR)

Compat-mode behavior end-to-end (synthetic metadata writing
`allow_lints = ["unexpected_cfgs"]` into the on-disk overlay TOML at
`<upstream>/target/lihaaf-overlay/Cargo.toml`) is only exercised by
`.github/workflows/refresh-pilots.yml`. That workflow:

- Triggers via `workflow_dispatch` only (line 51) — manual maintainer
  invocation, NOT an automatic PR check.
- Takes a `lihaaf_version` input (line 53-57) that points at a PUBLISHED
  crates.io version (`cargo install lihaaf --version <input>` inside the
  pilot job). The workflow does NOT install lihaaf from this branch's
  source.

This means: compat-mode behavior changes introduced by this PR are NOT
verified by `refresh-pilots.yml` on the PR. They are verified manually
**after** the PR merges, lihaaf cuts a release, the new version is
published to crates.io, and a maintainer manually dispatches
`refresh-pilots.yml` with the new version as input.

**What this means for §4d (compat unit tests):** the 4d.1 / 4d.2 / 4d.3
unit tests (compat synthesis injects key, default value, REPLACES
upstream) are the CI-side gate for compat-mode correctness. The
post-merge `refresh-pilots.yml` run is a second confirmation against
real pilot codebases, not the primary gate.

**What this means for the §3d implementation:** the implementer cannot
get end-to-end confidence from CI alone. The §4d unit tests must be
strong enough to stand alone as the compat-mode regression net.
Specifically, 4d.3 (REPLACES upstream) is the load-bearing test that
pins the invariant the post-merge pilot run depends on. If 4d.3 is
missing or weak, a compat-mode regression could ship and only be
caught by the next maintainer who manually fires `refresh-pilots.yml`
against a future lihaaf release.

### 10d. One-shot manual checks per PR

Beyond §10b reviewer-reruns:

- **§4c.2 counter-design verification.** The implementer commenting out the
  `apply_allow_lints` call in `spawn_and_monitor` and confirming
  `cargo test --test lihaaf_allow_lints` fails with the literal substring
  `unused import: \`std::collections::HashMap\``. One-shot evidence in the
  PR description that the integration test actually FAILS when the
  implementation is absent. Reviewer reproduces if doubting.

### 10e. Developer-attention items converted to CI in earlier revisions

- R2 §9d "screenshot rendered README" → R3 §9d "grep -F output paste"
  (reviewer re-runs). Still PR-paste (§10b), not automated CI.
- R2 §8 "implementer checks baseline.toml signature" → R3 §8b "verified in
  plan: envelope carries no synthetic-metadata fields, no implementer check
  needed." Verification is structural (plan §3d, §8b) not a runtime step.

### 10f. Why no new CI infrastructure for v0.1.0

Two candidate workflow changes were considered and rejected for v0.1.0:

1. **Adding a `doc-content-greps` step to `ci.yml`** — would convert §10b
   items to fully automated. Rejected because: (a) the greps are 18 mechanical
   `grep -F` invocations in §9d, including two CHANGELOG awk+grep checks,
   (b) each is reviewer-reproducible in seconds, (c) every
   CI addition is independent scope risk before v0.1.0 ship. Tractable
   post-v0.1.0.
2. **Adding a branch-artifact pilot workflow that builds lihaaf from this
   PR's source and runs the compat-mode flow** — would convert §10c
   to fully automated. Rejected because: (a) requires substantial workflow
   plumbing (the existing `refresh-pilots.yml` deliberately reuses the
   `workflow_call` cross-repo pattern that needs a published version), (b)
   the §4d unit tests provide the primary regression net, (c) the
   post-merge `refresh-pilots.yml` dispatch already covers this lane for
   each release. Tractable post-v0.1.0.

Both are noted as POST_V010 follow-up candidates.

---

## Summary

- **Problem:** Bare `rustc` invocation has no cargo feature context → noisy
  lints leak into every fixture snapshot. For `unexpected_cfgs` specifically,
  the lint is `--check-cfg`-gated and lihaaf does not currently pass that
  flag, so the on-the-ground noise today comes from other default-on lints
  (`unused_imports`, etc.); compat-mode synthesis still defaults
  `allow_lints = ["unexpected_cfgs"]` for forward-compat when adopters or a
  future feature do enable check-cfg-driven diagnostics.
- **Approach:** New optional `allow_lints: Vec<String>` config key (top-level
  and per-suite, mirroring `dev_deps` inheritance precedent); forwarded as
  `-A <lint>` per entry on the rustc invocation. **Compat mode synthesizes
  `["unexpected_cfgs"]` by default** as forward-only insurance for the day
  lihaaf or rustc starts passing `--check-cfg`; under v0.1.0 today the
  default is a no-op. Round-2 pilots that hit `unused_imports` /
  `dead_code` noise under v0.1 use the v0.1 TOML path in their fork.
- **Test plan:** ~11 unit tests in `src/config.rs::tests` (validation +
  inheritance), ~3 unit tests in `src/worker.rs::tests` (arg-shape), 3 unit
  tests in `src/compat/overlay.rs::tests` (compat synthesis: injects key,
  default value, REPLACES upstream — R3-new test 4d.3), 1 integration test
  (`tests/lihaaf_allow_lints.rs`) using `unused_imports` as the
  load-bearing lint (R3-redesigned from R2's `unexpected_cfgs` choice which
  required `--check-cfg` that lihaaf doesn't pass).
- **Documentation:** README (§Quick start + §Multi-suite),
  `docs/spec/lihaaf-v0.1.md` (§3.2 + §3.6 + compat-mode callout),
  `CHANGELOG.md` `[Unreleased]` entry — all land in the same PR, verified by
  `grep -F` invocations in §9d (R3 introduced grep-paste over screenshots;
  R4 §10 catalogued these as PR-paste reviewer-reproducible, NOT automated
  in CI; rustdoc on the new fields IS automated via `RUSTDOCFLAGS="-D warnings" cargo doc --no-deps`).
- **Envelope / `baseline.toml` impact:** none. `OverlayMetadata` carries no
  synthetic-metadata fields, so adding `allow_lints` to `SyntheticMetadata`
  does not change envelope bytes or baseline.toml signature (R3 — corrects
  R2's incorrect envelope-serialization claim).
- **CI enforcement (R4 honest framing — see §10):**
  - Fully CI-enforced on this PR via `ci.yml`: all §4a/4b/4d unit tests, the
    §4c integration test (`cargo test` with `LIHAAF_RUN_CARGO_BUILD_TESTS=1`
    set at `ci.yml:56`; R7 §4c.3 gating note), rustdoc on the new fields
    (`RUSTDOCFLAGS="-D warnings" cargo doc --no-deps`), fmt and clippy on new code, and the
    self-test corpus end-to-end.
  - PR-paste reviewer-reproducible (NOT automated): the §9a-c doc-content
    `grep -F` invocations and the corrected awk for CHANGELOG.
  - Manual post-merge (NOT on this PR): compat-mode end-to-end via
    `refresh-pilots.yml`, which is `workflow_dispatch`-only against a
    PUBLISHED crates.io version. The §4d unit tests (especially 4d.3
    REPLACES) are the primary CI-side gate for compat-mode correctness.
