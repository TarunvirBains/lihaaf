# `extra_substitutions` design + implementation plan — v0.1.0-beta.10

Status: FINAL DRAFT (round 6 — post Codex round-5 ALLOW + DOC cleanup). Round-6 addresses the two Codex round-5 DOC_BEFORE_BETA findings (Finding A field-level test matrix gap propagating Class 9 + F'' through the wiring layer, Finding B §5/§6.6 silent on full-string anchor rule for bare placeholder patterns). No predicate-behavior changes; surgical Edit-tool amendments only. Previous round status was AMENDED DRAFT round 5 — post Codex round-4 ALLOW + DOC cleanup, which addressed the five Codex round-4 DOC_BEFORE_BETA findings (Finding 1 §12 stale "unreachable" claim, Finding 2 OQ-B interior-dollar ambiguity, Finding 3 rule 4(c) anchor ambiguity, Finding 4 stale CI-banner wording, Finding 5 byte/char count corrections).
Author: strict-swe PLANNER (Opus, 2026-05-19, round 6 surgical amendment).
Implementation target: v0.1.0-beta.10.
Implementer: `careful-coder` (Opus, max effort) after user OK on this final draft.

## 0.0 User-locked decisions (2026-05-19)

After the planner produced this round-3 draft, the user walked the
substantive decision points and locked the following, OVERRIDING the
planner's earlier choices where they conflict:

- **Decision #1 — banner-strip scope.** Planner deferred banner-shape
  stripping to v0.2 with a future `[[strip_banner]]` config key. **User
  override: banner-strip is IN SCOPE for v0.1.0-beta.10.**
- **Decision #2 — strip surface.** `strip_lines` and
  `strip_line_prefixes` ARE the banner-strip surface (NO separate
  `[[strip_banner]]` key). They accept a separate banner-anchored
  allowlist `is_banner_shape`, distinct from the `is_path_like`
  allowlist used by `extra_substitutions.from`. The strip validator is
  the disjunction: `is_path_like(s) || is_banner_shape(s)`.
- **Decision #3 — locked answers to planner round-3 OQs.**
  - **OQ-A (single-segment paths / pure tokens / Windows drive-letter-
    only rejected) — LOCKED defensible.** Rejection IS the safety
    property; no legitimate adopter use case blocked.
  - **OQ-B (`$X` uppercase-only) — LOCKED uppercase-only, with explicit
    user-facing documentation emphasis** in §5 / §6.6 spec prose.
  - **OQ-C (banner deferral real?)** — collapsed by Decision #1.
  - **OQ-D (`to` unconstrained beyond no-newline) — LOCKED.** Round-2
    BLOCK class was `from`-side; `to` writes into adopter's own
    snapshot.
  - **OQ-E (strip allowlist symmetry with `from`)** — collapsed by
    Decision #2.

§13 carries forward the locked decisions as documentation and surfaces
ONE new design open question (the `is_banner_shape` predicate itself,
designed in §3.3) for Codex round-3 to critique per
[[lihaaf-plan-adversarial-cycle]].

**Round-4 amendments preserve all user-locked decisions.** The
FIX_BEFORE_BETA-1 fix REINFORCES OQ-B by closing a gap in the
round-3 predicate that admitted `$lowercase/path` shapes; no other
locked decision is relaxed. See §0 round-4 headline and §14 round-4
table.

## 0. What changed since round 2

Round 2 returned `VERDICT: BLOCK` (Codex 5.5 xhigh): 2 BLOCKs, 2 NITs.
The round-2 design used a closed 8-marker substring blocklist on every
`extra_substitutions.from` and strip pattern. Codex round-2 showed the
blocklist could not be closed:

- **BLOCK-1** — strip patterns matching `  |` (rustc span context: two
  spaces + pipe, no markers) or `For more information about this error`
  (rustc explain footer: no markers) pass the blocklist and still drop
  protected diagnostic content.
- **BLOCK-2** — `from = "error"`, `from = "E0277"`, `from = ":"` contain
  none of the eight markers but rewrite protected diagnostic substrings
  inside `error[E0277]` / `= note:` / trailer lines.

Round-3 headline shifts (post user-lock):

- **BLOCK-1 + BLOCK-2 fix (§3.3)** — eight-marker substring blocklist
  replaced with positive **allowlists**. `extra_substitutions.from` is
  gated by `is_path_like`. Strip keys are gated by the disjunction
  `is_path_like || is_banner_shape`, where `is_banner_shape` accepts
  known rustc trailers + CI/environment banners while rejecting span
  context, diagnostic-message bodies, and bare diagnostic keywords.
- **Banner-strip is in scope (§1.2, §3.3, §3.4, §5/§6.6, §7.3, §12)** —
  reversal of the planner's pre-lock-in deferral. The disjoint
  allowlist makes banner-strip safe by construction: the predicate
  itself is the safety property, not a downstream marker scan.
- **NIT-1 fix (§7.3)** — predicate-level tests are table-driven over
  acceptance + rejection matrices for BOTH `is_path_like` AND
  `is_banner_shape`. Field-level wiring tests exercise both predicates
  via the strip-key disjunction. Round-2 BLOCK regression rows pinned
  explicitly.
- **NIT-2 fix (§7.1)** — compat-mode test rows annotated as normalizer
  composition tests, not user-facing compat support. §5 / §6.6 prose
  states the three keys are *unsupported in compat mode* for
  v0.1.0-beta.10.
- **OQ-2 re-pressure-tested (§3.4)** — two-key rationale holds under
  the disjoint allowlist; both strip keys share the same disjunction;
  validation is symmetric per key.

Round-4 headline shifts (post Codex round-3 review):

- **BLOCK-1 fix (§3.3.2 (C.1) + §7.3.2)** — the round-3 design listed
  a short `"linker: "` banner prefix that contradicted the (A.1)
  20-byte length floor: `linker: lld-15.0.7` is 18 bytes. The short
  prefix is DROPPED; the length floor is preserved at 20 (kept
  separation from `mismatched types`-class diagnostic bodies); the
  longer `"linker version: "` form is retained. §7.3.2 banner-rejection
  matrix gains an explicit regression row for `linker: lld-15.0.7`.
- **FIX_BEFORE_BETA-1 fix (§3.3.1) — TREATED AS BLOCK-EQUIVALENT.**
  The round-3 `is_path_like` admitted `$nix/path` via the `/` branch,
  silently violating the user-locked OQ-B uppercase-only $-placeholder
  contract. A new rule 3 (leading-`$` guard) fires BEFORE the
  disjunction: any string starting with `$` must have an ASCII
  uppercase letter as its second byte, regardless of path-separator
  presence. §7.3.1 rejection-class F' (`$lowercase + /`) added; test
  14a `is_path_like_rejects_lowercase_dollar_with_separator` added;
  wiring tests 43 and 53 extended.
- **DOC_BEFORE_BETA-1 fix (§1.2, §11 Risk 5, §5/§6.6)** — the round-3
  prose claimed both predicates reject diagnostic message bodies. That
  claim was over-stated: path-bearing diagnostic lines (e.g.,
  `error: couldn't read /build/generated.rs`) pass `is_path_like` via
  the embedded `/`. The claim is now tightened to "diagnostic bodies
  WITHOUT a path-shaped substring." Path-bearing diagnostic stripping
  is documented as adopter-authorized, not framework-guaranteed. New
  §11 Risk 5 captures the full surface and mitigations.
- **DOC_BEFORE_BETA-2 fix (§3.3.2 (C.2), §7.3.2, §5/§6.6)** — the
  (C.2) disjunction alternative was labeled "CI-BANNER STRUCTURAL" but
  admits any uppercase-leading, 40+ byte, deprecation-marker-bearing
  line — including non-CI structural banners (proc-macro / code-gen
  deprecation banners, build-system migration banners). Renamed to
  "STRUCTURAL BANNER SHAPE." Design rationale expanded; non-CI example
  added to worked-acceptance table; test 34 renamed and required to
  cover both CI and non-CI subfamilies.

**No locked decision relaxed in round 4.** OQ-A / OQ-B / OQ-D remain
locked. The FIX_BEFORE_BETA-1 fix REINFORCES OQ-B (catches a
contract-violation the round-3 predicate admitted). All four round-3
findings are addressed surgically; no section restructured.

Round-5 headline shifts (post Codex round-4 ALLOW):

- **DOC Finding 1 fix (§12)** — stale "diagnostic-message bodies
  unreachable" claim tightened to "diagnostic bodies WITHOUT a
  path-shaped substring unreachable"; path-bearing diagnostic
  bodies cross-referenced to §11 Risk 5 as adopter-authorized
  opt-in noise removal.
- **DOC Finding 2 fix (§3.3.1, §5/§6.6, §7.3.1)** — interior
  `$lowercase` within paths (e.g., `/path/$nix/sub`) explicitly
  documented as path text accepted via rule 4(a). Rule 3
  leading-`$` guard only fires on `s.as_bytes()[0]`. New acceptance
  class 9 + test 8a pin the boundary. OQ-B clarified to govern
  recognized-placeholder-naming convention for LEADING tokens, not
  arbitrary `$` occurrences in path text.
- **DOC Finding 3 fix (§3.3.1, §7.3.1)** — rule (4c) restated as
  full-string-anchored (`^\$[A-Z][A-Za-z0-9_]*$`). New rejection
  class F'' + test 14b pin that `$DIR-`, `$A!`, `$RUST.` etc. do
  NOT pass via (4c) alone. Companion assertion that `$DIR/x`
  still passes overall (via 4a) but a unit-level (4c)-isolation
  helper rejects it.
- **DOC Finding 4 fix (§3.3.2, §3.3.3, §11 Risk 2 + Risk 3, §13
  OQ-NEW-1)** — "CI-banner" / "CI-structural" / "CI markers"
  framing replaced with "structural-banner" / "deprecation-marker"
  / "tool-emitted banner" framing in design-rationale and risk
  prose. CI vendor examples (GitHub Actions, GitLab CI) preserved
  where they appear as legitimate worked examples.
- **DOC Finding 5 fix (§3.3.2, §7.3.2)** — wrong byte/char count
  annotations corrected: `error: aborting due to 1 previous error`
  41 → 39 bytes; `linker version: GNU ld (GNU Binutils) 2.40`
  41 → 42 bytes; `linker version: rust-lld 15.0.7` 29 → 31 bytes;
  `error: cannot find type \`Foo\`` 36 → 38 bytes (with "in
  scope" added for full-string consistency). All other length
  annotations re-verified.

**No locked decision relaxed in round 5.** OQ-A / OQ-B / OQ-D remain
locked. DOC Finding 2 SHARPENS the OQ-B contract (clarifies it
governs leading placeholder shape, not arbitrary `$` occurrences in
path text); DOC Finding 3 SHARPENS rule (4c) (clarifies
full-string-anchored, not prefix). Neither relaxes the
implementer-facing constraint.

Round-6 headline shifts (post Codex round-5 ALLOW):

- **DOC Finding A fix (§7.3.3)** — round-5 added predicate-level
  acceptance Class 9 (interior `$lowercase` within paths) and
  rejection Class F'' (placeholder with trailing junk under rule
  (4c) full-string anchoring) but the field-level wiring tests
  still listed "Classes 1-8" / Class F-only enumerations. Updated:
  test 43 (`validate_extra_substitutions_rejects_non_path_from`)
  rejection-class list extended A/B/C/E/F/F'/H/I/J → A/B/C/E/F/F'
  /F''/H/I/J; test 44 (`validate_extra_substitutions_accepts_path
  _from`) acceptance list extended Classes 1-8 → Classes 1-9; test
  47 (`validate_strip_patterns_accepts_path`, strip-key mirror of
  test 44) extended Classes 1-8 → Classes 1-9; test 53
  (`validate_strip_patterns_rejects_short_and_dollar`, strip-key
  mirror of test 43 for the `$` family) rejection-class list
  extended D/E/F/F'/G/H → D/E/F/F'/F''/G/H. No new tests added;
  test bodies already cover the disjunction via predicate calls.
  Description-only propagation.
- **DOC Finding B fix (§5/§6.6 spec amendment prose)** — round-5
  documented the rule (4c) full-string anchor in implementer prose
  at §3.3.1 lines 421-426 but the §5/§6.6 adopter-doc surface only
  covered leading-`$` uppercase requirement and interior
  `$lowercase` acceptance. New `bare-placeholder full-string-anchor
  prose` paragraph inserted between the uppercase-only prose and
  the structural-banner-shape prose. Explains that bare patterns
  (e.g., `$DIR`) match the full string under
  `^\$[A-Z][A-Za-z0-9_]*$`; trailing non-placeholder characters
  (`$DIR-`, `$DIR.`, `$A!`, `$RUST extra`, `$WORKSPACE,`) are
  REJECTED; placeholder + path content (`$DIR/x`,
  `$RUST/lib/rustlib`) is accepted through the path-separator
  branch, not the bare-placeholder branch. Adopters now have
  user-facing documentation for the round-5 sharpening.

**No locked decision relaxed in round 6.** OQ-A / OQ-B / OQ-D
remain locked. DOC Finding A is a test-description propagation;
DOC Finding B is an adopter-doc surface-up of an implementer-prose
constraint already documented at §3.3.1. Neither touches predicate
behavior or contract.

Per-finding section mapping: §14 round-3 + round-4 + round-5 +
round-6 table.

## 1. Motivation

### 1.1 §2 pilot evidence

§2 pilots (anyhow, serde-json, derive_more, axum-macros) surfaced
divergences between upstream blessed snapshots and lihaaf's normalizer.
Issues #65/#66/#67 (framed as parity bugs) were closed 2026-05-19 as
misframings per [[lihaaf-is-its-own-product-not-trybuild-clone]]: lihaaf
has its own normalizer opinions; adopter divergence is solved by giving
adopters configuration, not by mutating defaults. The remaining real
adopter friction has two distinct shapes:

- **Environment-specific paths** — NixOS store paths, Bazel sandboxes,
  vendored toolchains, custom CI cache mounts, in-house rustc builds
  with non-standard sysroot layouts.
- **Environment-specific banner lines** — rustc-emitted trailers (the
  `For more information about this error, try ...` footer, the
  `note: this error originates from the macro ...` macro-origin
  trailer, the `error: aborting due to N previous errors` summary)
  that adopters in some environments want stripped to keep snapshots
  toolchain-agnostic; plus non-rustc banners (CI deprecation warnings,
  vendored-toolchain version banners).

### 1.2 Issue #45 expansion (2026-05-19, post user-lock)

Issue #45 is a v0.1.0-beta.10 deliverable with three sub-scopes:

- **`$RUST` inner-path variants** — sysroot layouts differ across rustup,
  Nix-store sysroots, vendored toolchains. Adopters supply path-shaped
  substitutions like `{ from = "/nix/store/abc123-rust-1.95.0/lib/rustlib", to = "$RUST/lib/rustlib" }`.

- **Path-noise stripping AND banner stripping** — two classes of lines
  adopters want dropped:

  1. **Path-noise** — absolute paths leaked by a vendored toolchain
     prologue (`/build/sandbox/internal/wrappers/cc-wrapper`), sysroot
     path fragments that survive built-in normalization in unusual
     NixOS layouts. Strip patterns in this class pass the `is_path_like`
     allowlist (path separator OR `$X` placeholder).

  2. **Banners** — rustc-emitted trailers (`For more information about
     this error, try ...`, `note: this error originates from the macro
     ...`, `error: aborting due to N previous errors`); CI deprecation
     banners (`Node.js 16 actions are deprecated. Please update ...`);
     non-rustc tool version banners (`linker version: GNU ld (GNU
     Binutils) 2.40`); vendored-toolchain version banners (`info:
     using rustc from /opt/vendored/rust-1.95.0/bin/rustc`). Strip
     patterns in this class pass `is_banner_shape`. Per Decision #2
     they share the `strip_lines` / `strip_line_prefixes` surface with
     path-noise patterns; the strip validator is the disjunction
     `is_path_like(s) || is_banner_shape(s)`.

  **What remains OUT OF SCOPE for strip patterns** (and structurally
  unreachable by the disjoint allowlist):

  - **Rustc span context** (`  |`, `expected due to this`, `^^^^^`,
    `--> path:line:col`). This is diagnostic content. `is_banner_shape`
    rejects it (§3.3). The `-->` case passes `is_path_like` via the `/`,
    but adopters who explicitly write a `-->`-shaped strip pattern have
    opted into stripping their own location pointers — an action the
    framework does not need to second-guess once the pattern is
    path-shaped.
  - **Diagnostic message bodies** WITHOUT a path-shaped substring
    (`the trait bound \`X: Y\` is not satisfied`, `cannot find
    function \`foo\` in scope`, `mismatched types`). Rejected by
    both `is_path_like` AND `is_banner_shape`. Built-in normalizer
    test `does_not_touch_diagnostic_text` (`normalize.rs:562-567`)
    continues to pin this for built-in behavior; the strip surface
    predicates pin it for adopter opt-in.

    **Round-4 amendment (DOC_BEFORE_BETA-1 resolution):** Diagnostic
    message bodies that DO include a path-shaped substring (e.g.,
    `error: couldn't read /build/generated.rs`) pass `is_path_like`
    via the path component. The framework does NOT detect such
    cases structurally. The design rationale: an adopter who writes
    a strip pattern equal to `error: couldn't read /build/generated.rs`
    has explicitly opted into dropping the named line. The framework
    enforces shape contracts (path-shaped OR banner-shaped), not
    intent over the path-shaped class. See §11 Risk 5 for the full
    risk surface and §6.6 adopter-doc prose for the user-facing
    caveat.
  - **Diagnostic-keyword bare patterns** (`error`, `warning`, `note`,
    `help`, `error[E0277]`, `= note`, `error:`). Rejected by both
    predicates.

- **Per-fixture-suite substitution overrides** — workspaces with
  heterogeneous fixture environments need different substitution sets
  per suite.

### 1.3 Design philosophy and default invariant

Lihaaf is its own product. `extra_substitutions` and the strip keys
are the adopter-facing extension mechanism for **path-shaped
substitution + path-or-banner-shaped stripping**; not a switch to make
lihaaf output match any other tool, not a mechanism for rewriting or
deleting arbitrary text. The disjoint allowlists (§3.3) enforce shape
contracts structurally. Lihaaf's hardcoded defaults (`$DIR`,
`$WORKSPACE`, `$RUST`, `$CARGO/registry`, `$TYPEID`, `$LONGTYPE_FILE`)
do not change in beta.10. The built-in normalizer's
preserve-byte-for-byte commitments to rustc explain footers / aborting
summaries / macro-origin trailers (`normalize.rs:569-606`) are
**unchanged**: these lines pass through the built-in pass byte-
identical, and only the new opt-in strip keys can drop them.

**Default invariant.** An adopter who does not set any of the three new
keys observes byte-identical output vs beta.9. Feature is additive and
opt-in. The 13 existing normalizer unit tests
(`src/normalize.rs:493-840`) and every in-tree snapshot must pass
unchanged.

## 2. Design philosophy and non-goals

The framework is lihaaf-leads. Adopters configure; lihaaf executes.

**Non-goals.**

- NOT a switch to match any other harness's output shape.
- NOT a way to silence rustc diagnostic CONTENT for path-FREE
  diagnostic message bodies — the disjoint allowlists in §3.3 reject
  path-free diagnostic-message-body patterns (`the trait bound ...`,
  `mismatched types`, `cannot find ...`) on both `from` and strip
  surfaces. **Path-bearing diagnostic lines** (e.g., `error: couldn't
  read /build/generated.rs`) pass `is_path_like` via the path
  substring; an adopter who writes such a strip pattern has
  explicitly opted into dropping that line family. See §1.2,
  §11 Risk 5, and §6.6 for the full surface (round-4
  DOC_BEFORE_BETA-1 resolution).
- NOT regex (§6.1 of the v0.1 spec).
- NOT a way to strip rustc span context (`  |`, `^^^^^`, `expected due
  to this`). `is_banner_shape` rejects span context by structural
  precondition + anti-prefix list (§3.3).
- NOT a way to rewrite diagnostic-keyword bare patterns (`error`,
  `warning`, `note`). Rejected by both `is_path_like` and
  `is_banner_shape`.

**Order of precedence.** Built-ins first. Adopter extras after. §4.

## 3. Config schema

### 3.1 Top-level keys under `[package.metadata.lihaaf]`

All three keys default to `[]`. `extra_substitutions` applies AFTER
built-ins on every normalized line. `strip_lines` matches by full-line
equality (after `trim_end_matches([' ', '\t'])`, before blank-line
collapse). `strip_line_prefixes` matches by prefix (same trimming).
Validation: §3.3.

```toml
[package.metadata.lihaaf]
extra_substitutions = [
    { from = "/nix/store/abc123-rust-1.95.0/lib/rustlib", to = "$RUST/lib/rustlib" },
    { from = "/build/vendored-cargo", to = "$CARGO/registry" },
    { from = "$RUST/lib/rust-1.95.0", to = "$RUST" },
]
strip_lines = [
    "/build/sandbox/internal/wrappers/cc-wrapper-1.0",
    "error: aborting due to 1 previous error",
]
strip_line_prefixes = [
    "$WORKSPACE/.cargo-cache/",
    "For more information about this error",
    "note: this error originates from ",
]
```

Every `extra_substitutions.from` example is path-shaped. Each strip
example is either path-shaped or banner-shaped; §3.3 validators reject
non-path-non-banner patterns at parse time.

### 3.2 Per-suite override under `[[package.metadata.lihaaf.suite]]`

```toml
[[package.metadata.lihaaf.suite]]
name = "vendored"
fixture_dirs = ["tests/lihaaf/vendored"]
extra_substitutions = [
    { from = "/build/sandbox", to = "$WORKSPACE" },
]
strip_lines = []
strip_line_prefixes = []
```

**Per-suite semantics: REPLACE, not MERGE.** Matches the `features`
inheritance precedent (`src/config.rs:485-491`; v0.1 spec §3.6). REPLACE
is defensible only because it is enforced consistently with `features`.
The empty-on-omission rule is pinned in three places: v0.1 spec §3.4
amendment, v0.1 spec §6.6 amendment, and regression test §7.2
`extra_substitutions_omitted_on_named_suite_is_empty`.

### 3.3 Validation rules (disjoint allowlists — round-3 + user-locks)

Validation runs at config parse time (`src/config.rs:557-637`, alongside
`validate_features` / `validate_allow_lints`).

Round 2's eight-marker substring blocklist could not be closed. Round 3
inverts the question to a positive allowlist. User Decision #2 then
splits the allowlist into TWO disjoint predicates with different
purposes:

- **`is_path_like`** — gates `extra_substitutions.from`. Purpose:
  ensure substitutions target environment-specific paths and
  placeholders only.
- **`is_banner_shape`** — additional gate for `strip_lines` /
  `strip_line_prefixes` AS AN ALTERNATIVE to `is_path_like`
  (disjunction). Purpose: allow strip of rustc trailers and
  environment banners while rejecting span context, diagnostic
  content, and diagnostic-keyword bare patterns.

Strip validator is `is_path_like(s) || is_banner_shape(s)`.
Substitution validator on `from` is `is_path_like(s)` only.
`to` remains gated only by "no newlines."

#### 3.3.1 `is_path_like` predicate

Define `is_path_like(s: &str) -> bool` returning true iff all of the
following hold:

1. `s.len() >= 2` (bytes).
2. `s` contains no `\n` byte.
3. **Leading-`$` guard (post round-4 amendment, FIX_BEFORE_BETA-1
   resolution).** If `s.as_bytes()[0] == b'$'`, then `s.as_bytes()[1]`
   MUST be in `b'A'..=b'Z'`. (Rule 1 already guarantees `s.len() >= 2`,
   so accessing `s.as_bytes()[1]` is safe.) This rule fires BEFORE
   the disjunction (4) below and is unconditional: a leading-`$`
   pattern whose second byte is not ASCII uppercase is rejected even
   if the string also contains `/` or `\`. This closes the
   `is_path_like("$nix/path") == true` gap that violated OQ-B's
   uppercase-only contract via the `/` branch.

   **Round-5 clarification (DOC_BEFORE_BETA Finding 2 resolution).**
   Rule 3 only fires on the LEADING `$` byte (`s.as_bytes()[0]`).
   Interior `$lowercase` substrings — for example, `/path/$nix/sub`,
   `/some/$cache/dir`, `$WORKSPACE/$tmp/run` — are accepted as path
   text via rule 4(a) (contains `/`); they are NOT treated as
   placeholder references. OQ-B governs the recognized placeholder
   shape (lihaaf's `$[A-Z][A-Za-z0-9_]*` placeholder-naming
   convention), not arbitrary `$` occurrences anywhere in the
   string. The safety property of OQ-B is naming-convention clarity
   for the LEADING placeholder token, not a blanket prohibition on
   `$` appearing inside path text. Interior `$lowercase` is common
   in real adopter paths (NixOS env-var-style names, Bazel
   sandboxes, custom CI cache mounts) and rejecting it would
   over-constrain `is_path_like` without any safety gain.
4. At least one of:
   - (a) `s.contains('/')`, OR
   - (b) `s.contains('\\')`, OR
   - (c) **`s` is a complete placeholder token (round-5 DOC Finding 3
     clarification):** `s` starts with `$`, followed by an ASCII
     uppercase letter `[A-Z]`, followed by zero or more
     `[A-Za-z0-9_]` characters, AND there are NO additional
     characters after the placeholder tail. The implementer-facing
     constraint is full-string-anchored: the regex equivalent is
     `^\$[A-Z][A-Za-z0-9_]*$`. This means (4c) admits only bare
     standalone placeholders (`$DIR`, `$WORKSPACE`, `$NIX_STORE`,
     `$RUST`, `$LONGTYPE_FILE`). Strings like `$DIR-`, `$A!`, or
     `$DIR/x` do NOT pass via (4c). (`$DIR/x` still passes via
     (4a) because it contains `/`; the test below pins that (4c)
     alone does not admit it.)

Rule (4c) accepts every built-in lihaaf placeholder (`$DIR`,
`$WORKSPACE`, `$RUST`, `$CARGO`, `$TYPEID`, `$LONGTYPE_FILE`) and
adopter-introduced placeholder names (`$NIX_STORE`, `$SANDBOX`).
**Uppercase-only is locked per Decision #3 / OQ-B**, with explicit
user-doc emphasis in §5 / §6.6. Rule (3) reinforces OQ-B by ensuring
the uppercase requirement holds even on `$lowercase/path` /
`$lowercase\path` shapes that would otherwise admit themselves via
the path-separator branch.

The minimum-length rule (1) rejects single-character patterns like
`from = "/"` (would rewrite every slash, breaking rustc's
`--> path:line:col` shape) or `from = "\\"`.

**Worked rejection examples that rule (3) catches** (the new $-guard
class — these would have passed under round-3 (4a) via the `/`
branch):

- `$nix/path` — leading `$`, second byte `n` (lowercase), rejected.
- `$lowercase/anything` — leading `$`, second byte `l`, rejected.
- `$_path/x` — leading `$`, second byte `_`, rejected.
- `$1/path` — leading `$`, second byte `1` (digit), rejected.
- `$ /space-then-slash` — leading `$`, second byte space, rejected.
- `$nix\path` — leading `$`, second byte `n`, rejected via `\` branch.

Note that strings NOT starting with `$` are unaffected by rule (3) —
`/nix/store/foo` and `path/with/slash` still pass via (4a) as before.

**OQ-A locked defensible.** Single-segment paths without separators
(`tmp`, `target`, `serde`), pure tokens (`HOME`, `error`), and Windows
drive letters without a separator (`C:` alone — but `C:\` passes via
rule 4b) are REJECTED. This IS the safety property: it's what blocks
the round-2 BLOCK class. No legitimate adopter use case is blocked by
it (verified by inspection — adopters that need to substitute paths
include either a separator or a `$X` placeholder; pure-token CI
identifiers must include surrounding path context, which is how they
appear in rustc output anyway).

#### 3.3.2 `is_banner_shape` predicate (planner design, Codex round-3 critiques)

Per [[lihaaf-plan-adversarial-cycle]], the planner designs the
predicate and Codex round-3 critiques it. The orchestrator may not
defer this design to the reviewer phase.

Define `is_banner_shape(s: &str) -> bool` returning true iff:

**(A) Shared preconditions — ALL must hold.**

1. `s.len() >= 20` (bytes). Banner lines are intrinsically multi-word;
   anything shorter is more plausibly a bare diagnostic keyword. This
   floor is comfortably above the longest currently-known banner-prefix
   anchor and below the shortest known full banner (`error: aborting
   due to 1 previous error` is 39 bytes; round-5 DOC Finding 5
   correction).
2. `!s.contains('\n')`. Single-line only.
3. `s` does NOT start with whitespace (`' '` or `'\t'`). Rejects rustc
   span context indentation (`  |`, `   ^^^^^`, `      = note`).
4. `s` does NOT start with `'^'`, `'='`, or `'|'`. Defense-in-depth
   against span context shapes whose first byte is not whitespace
   (unusual but possible in adversarial input).

**(B) Anti-prefix list — REJECT if `s` starts with any of these
case-sensitive strings:**

```
"expected "
"found "
"the trait "
"the type "
"cannot find "
"mismatched types"
"consider "
"help: "
"warning: "
"error["
"  "     // two spaces — span context (also caught by (A.3); defense-in-depth)
```

These anti-prefixes block the diagnostic-message-body shape family
that rustc commonly emits. `error[` blocks `error[E0277]`-style code
lines without rejecting `error: aborting due to ...` (which uses a
colon-space separator).

**(C) Disjunction — at least ONE alternative must hold:**

- **(C.1) ENUMERATED BANNER PREFIX.** `s` starts with one of (case-
  sensitive):

  ```
  "For more information about this error"
  "error: aborting due to "
  "note: this error originates from "
  "info: "
  "linker version: "
  ```

  These cover the rustc-emitted banner trailers (#1-3 in the MUST-accept
  list) and the non-rustc tool-version banner shape (#5). The list is
  conservative; a v0.2 follow-up may add an adopter-extensible prefix
  list (§13 OQ-NEW-2 below). The vendored-toolchain banner
  `info: using rustc from /opt/...` matches via `"info: "` AND passes
  `is_path_like` — the disjunction handles dual-match cleanly (true is
  true).

  **Round-4 amendment (BLOCK-1 resolution):** the round-3 prefix list
  included a shorter `"linker: "` prefix. That prefix is dropped: it
  admitted strings like `linker: lld-15.0.7` (18 bytes) that violate
  the (A.1) 20-byte length floor — the predicate self-contradicted.
  The (A.1) floor is preserved at 20 bytes (chosen to keep separation
  from `mismatched types` at 16, `cannot find` at 11, and other
  diagnostic-body shapes that must remain rejected). Adopters who
  need to strip the short `linker:` form should either upgrade the
  toolchain to emit the longer `linker version:` form (already
  supported here) or use a path-shaped strip pattern naming the
  linker binary path. The longer `linker version:` form remains
  supported via this list; structurally-banner-shaped linker lines
  may also fall through to (C.2) if uppercase-leading and ≥40 bytes.

- **(C.2) STRUCTURAL BANNER SHAPE.** ALL must hold:
  - `s.len() >= 40`.
  - `s.as_bytes()[0].is_ascii_uppercase()`.
  - `s.contains(' ')`.
  - `s` contains at least one of (case-sensitive):
    `"deprecated"`, `"deprecation"`, `"Please update"`,
    `"actions to use"`, `"EOL"`, `"end-of-life"`.

  **Round-4 amendment (DOC_BEFORE_BETA-2 resolution):** this
  alternative was previously named "CI-BANNER STRUCTURAL." That name
  was misleading: structurally, (C.2) catches any uppercase-leading,
  40+ byte, deprecation-marker-bearing line. CI-runner deprecation
  banners (`Node.js 16 actions are deprecated. Please update ...`,
  `GitHub Actions deprecation: please migrate by end-of-life ...`)
  are one example family. But (C.2) ALSO admits other tool-emitted
  structural banners that share the same shape: proc-macro generator
  deprecation banners (`Deprecated generator output: Please update
  the generated API before release`), code-gen tool deprecation
  banners (e.g., a `cbindgen`-style `Deprecated cbindgen flag: ...`
  line), build-system migration banners, and similar tooling-emitted
  noise lines.

  **This is intentional.** The strip surface exists for
  adopter-opt-in noise removal; the framework's job is to enforce
  shape contracts (`is_banner_shape`) at config-parse time, not to
  second-guess which specific tool emitted a structurally-banner-
  shaped line that the adopter explicitly wrote a strip pattern for.
  The uppercase-first-byte requirement is a deliberate carve-out
  from rustc's lowercase-first-byte convention
  (`error`/`warning`/`note`/`help`) — that is the boundary, not
  "produced by a CI runner."

  The uppercase-first-byte requirement still serves its safety
  purpose. The deprecation-marker substring set remains narrow:
  rustc emits `warning: use of deprecated function` for
  `#[deprecated]` items — but that line starts with `warning: `
  (lowercase), which fails the uppercase-first-byte check AND trips
  the anti-prefix list (B), so it cannot match (C.2). Verified by
  inspection: no rustc lint or diagnostic emits a top-line message
  starting with an uppercase letter that also contains
  "deprecated" or its siblings.

If none of (C.1) / (C.2) holds, return false.

**Adopter-pattern overmatch is not a framework concern.** Once a strip
pattern passes both predicates, the adopter has explicitly opted into
dropping every line that matches the pattern. The
`error: aborting due to ` prefix in (C.1) accepts both the rustc summary
trailer (`error: aborting due to 1 previous error`) AND the unrelated
diagnostic line `error: aborting due to user request` (which the
built-in normalizer preserves byte-for-byte per
`preserves_unrelated_aborting_text`, `normalize.rs:591-595`). The
framework's job is to enforce shape contracts (path or known banner
family), not to second-guess the adopter's intent within the family
they explicitly chose. §6.6 prose calls this out so adopters use exact-
match `strip_lines` (not `strip_line_prefixes`) when they want to
target a single banner instance rather than a family.

**Design rationale.** The hybrid (anchored-prefix enumeration +
structural-floor with anti-prefix carve-outs) was chosen over
alternatives:

- *Anchored prefix only* — tight rejection guarantee, but brittle
  against environment banners that lihaaf cannot enumerate (#4-6 in
  MUST-accept). Forces every adopter banner-strip to extend lihaaf's
  prefix list via a config key, multiplying the design surface.
- *Structural minimums only* — catches structural-banner-shaped
  lines generically, but cannot distinguish `error: aborting due to
  1 previous error` (length 39, starts with `error: `) from
  `error: cannot find type \`Foo\` in scope` (length 38, starts
  with `error: `) without case-by-case marker analysis. The
  anti-prefix list would still need to enumerate diagnostic-body
  shapes.
- *Hybrid* — anchored prefixes for known rustc + non-rustc trailers
  (closed list, conservative); structural-banner alternative for
  uppercase-leading deprecation-marker-bearing lines (covers CI
  deprecation banners, proc-macro / code-gen deprecation banners,
  build-system migration banners) with case-sensitive
  uppercase-first-byte carve-out from rustc's lowercase
  convention. Anti-prefix list enforces the must-reject
  diagnostic-body shapes. Each layer is independently auditable.

**Worked acceptance examples** (must return true):

| Input | Path through |
|---|---|
| `For more information about this error, try \`rustc --explain E0277\`.` | (A) ok, (B) ok, (C.1) prefix #1 |
| `For more information about this error, try \`rustc --explain E0001\`.` | (A) ok, (B) ok, (C.1) prefix #1 |
| `note: this error originates from the macro \`m\` in the crate \`c\` (in Nightly builds, ...)` | (A) ok, (B) ok, (C.1) prefix #3 |
| `error: aborting due to 1 previous error` | (A) ok, (B) ok (`error[` does not match `error: `), (C.1) prefix #2 |
| `error: aborting due to 42 previous errors` | (A) ok, (B) ok, (C.1) prefix #2 |
| `info: using rustc from /opt/vendored/rust-1.95.0/bin/rustc` | (A) ok, (B) ok, (C.1) prefix #4. Also passes `is_path_like`; disjunction handles dual-match fine. |
| `linker version: GNU ld (GNU Binutils) 2.40` | (A) ok, (B) ok, (C.1) prefix #5 |
| `Node.js 16 actions are deprecated. Please update the following actions to use Node.js 20: ...` | (A) ok, (B) ok, (C.2) all conditions (CI-runner deprecation banner — one structural-banner family) |
| `GitHub Actions deprecation: please migrate to Node.js 20 by end-of-life date` | (A) ok, (B) ok, (C.2) all conditions (CI-runner deprecation banner — one structural-banner family) |
| `Deprecated generator output: Please update the generated API before release` | (A) ok, (B) ok, (C.2) all conditions (non-CI structural banner — round-4 DOC_BEFORE_BETA-2 example: tool-emitted code-gen deprecation banner, demonstrates that (C.2) admits more than CI-runner banners) |

**Worked rejection examples** (must return false):

| Input | Rejected by |
|---|---|
| `  \|` | (A.1) length 3 < 20; also (A.3) starts with space |
| `  ^^^^^^` | (A.3) starts with space |
| `^^^^^^` | (A.4) starts with `^` |
| `= note: foo` | (A.4) starts with `=` |
| `\| trait bound` | (A.4) starts with `\|` |
| `expected due to this` | (A.1) length 20 — actually 20 chars, passes (A); (B) starts with `"expected "`. REJECT via (B). |
| `the trait bound \`X: Y\` is not satisfied` | (B) starts with `"the trait "` |
| `cannot find function \`foo\` in scope` | (B) starts with `"cannot find "` |
| `mismatched types` | (A.1) length 16 < 20; also (B) prefix match |
| `error` | (A.1) length 5 < 20 |
| `warning` | (A.1) length 7 < 20 |
| `note` | (A.1) length 4 < 20 |
| `error[E0277]` | (A.1) length 12 < 20; also (B) prefix `error[` |
| `error[E0277]: cannot find` | length 25 ≥ 20; passes (A); (B) starts with `"error["`. REJECT via (B). |
| `error: cannot find type \`Foo\` in scope` | length 38 ≥ 20; passes (A); does NOT match (B) (no anti-prefix is `"error: cannot"`); does NOT match (C.1) (no banner prefix is `"error: cannot"`); does NOT match (C.2) (lowercase first byte). REJECT by no alternative matching. |
| `: ` | (A.1) length 2 < 20 |
| `warning: use of deprecated function \`f\`` | passes (A); (B) starts with `"warning: "`. REJECT via (B). |
| `help: consider importing this trait` | passes (A); (B) starts with `"help: "`. REJECT via (B). |

The `error: cannot find type \`Foo\` in scope` case is the most
subtle: it passes (A) and (B) (no matching anti-prefix), and is then
rejected by failing to match any disjunction alternative. This is the
defense-in-depth that makes the hybrid robust: enumeration in (C.1)
is conservative, not a marker scan; the structural-banner (C.2)
requires specific deprecation markers AND uppercase first byte;
together they admit nothing that resembles a rustc diagnostic
message body.

#### 3.3.3 Validator wiring

**`validate_extra_substitutions`** rejects entries where:

1. `!is_path_like(from)`.

   *Error:* `extra_substitutions[N].from = "<from>" is not path-like
   (must contain '/', '\\\\', or start with a $X placeholder token,
   where X is an ASCII uppercase letter). Patterns starting with '$'
   must have an ASCII uppercase letter immediately after, regardless
   of path separators — '$lowercase/path' is rejected.
   extra_substitutions is for path-shaped substitution only, not
   arbitrary text rewriting. See docs/spec/lihaaf-v0.1.md §6.6.`

2. `to.contains('\n')`.

   *Error:* `extra_substitutions[N].to contains a newline character;
   replacements must be single-line.`

`to` is NOT subject to any allowlist (**OQ-D locked unconstrained
beyond no-newlines** per Decision #3). Adopters legitimately need
`to = ""` (strip-via-substitute), `to = "$RUST"`, and compound paths.
The risk surface for `to` is small: substitution only fires when
`from` matches, and `from` is already gated.

**`validate_strip_patterns`** (covers both `strip_lines` and
`strip_line_prefixes`) rejects entries where
`!is_path_like(entry) && !is_banner_shape(entry)`.

*Error:* `strip_lines[N] / strip_line_prefixes[N] = "<pat>" is neither
path-shaped nor banner-shaped (must contain '/', '\\\\', start with a
$X placeholder token where X is an ASCII uppercase letter, OR match
the banner allowlist — see docs/spec/lihaaf-v0.1.md §6.6). Patterns
starting with '$' must have an ASCII uppercase letter immediately
after, regardless of path separators — '$lowercase/path' is
rejected. Strip patterns target path-shaped environment noise OR
known banner shapes only.`

**Coverage of round-2 BLOCK surface (under disjoint allowlists).**
Every protected rustc span-context line (`  |`, `^^^^^^`, `expected
due to this`) and diagnostic-keyword bare pattern (`error`, `warning`,
`note`, `error[E0277]`, `: `) fails BOTH `is_path_like` (no separator,
no `$X`) AND `is_banner_shape` (rejected by (A) length/first-byte
preconditions or by (B) anti-prefix list, with the closed `error:` and
`warning:` carve-outs in (C.1) being the only paths through, and those
are anchored to specific tail strings). The round-2 BLOCK class is
foreclosed by construction.

**Coverage of round-4 FIX_BEFORE_BETA-1 surface (lowercase `$` +
path-separator).** The leading-`$` guard (§3.3.1 rule 3) catches
`$lowercase/path` shapes that would have admitted themselves through
the `/` or `\` branches of the round-3 disjunction. This closes the
OQ-B contract: the uppercase-only $-placeholder rule is now enforced
unconditionally for any leading-`$` pattern, regardless of whether
the pattern also contains a path separator.

### 3.4 Key-by-key decisions

**Why a table (`{ from, to }`) and not a delimited string?** Two-field
tables avoid sentinel-byte quoting; extension slot for a future `scope`
field.

**Why literal `from` and not regex?** Spec §6.1 prohibits regex.
Literal substring covers every motivating use case.

**Why disjoint allowlists instead of a single predicate (post user-lock)?**
The original round-3 design used `is_path_like` symmetrically on `from`
and strip keys. User Decision #2 split the allowlists because the two
surfaces serve different purposes: `from` is a substitution source
(always path-shaped because the targets are path-prefix layouts);
strip keys are line drops (path-shaped OR banner-shaped). A single
predicate would either (a) reject banner-strip use cases or (b) admit
banner shapes to `from` where they have no use and create attack
surface for diagnostic-text rewriting. Disjoint predicates fit each
surface's purpose.

**Why `to` is not allowlist-constrained?** The threat is inadvertent
diagnostic-text alteration via `from` matching unintended substrings.
Once `from` is gated to path-shape, an adversarial `to` could only
synthesize diagnostic-looking text where path-shape inputs appear —
extremely narrow surface. Locked per Decision #3 / OQ-D.

**Why two strip keys instead of one (OQ-2, re-pressure-tested)?**
`strip_lines` = exact equality (stable line, e.g.,
`/build/sandbox/internal/wrappers/cc-wrapper-1.0`,
`error: aborting due to 1 previous error`). `strip_line_prefixes` =
unstable suffix (`$WORKSPACE/.cargo-cache/<dirhash>-<sequence>` — drop
everything starting with cache prefix; `For more information about
this error` — drops the entire family of rustc explain footers
regardless of error code). Both take the SAME disjoint allowlist
(`is_path_like || is_banner_shape`); validation is symmetric per key.
Splitting keeps semantics clear (equality vs prefix) without a richer
matcher that would need its own validation story.

**Why per-suite REPLACE (OQ-1)?** Per the `features` precedent. Merge
is hidden coupling. Sharp edge documented in §3.2.

## 4. Composition order

Per-line ordering in `normalize::normalize`:

1. Unify line endings to `\n` (`normalize.rs:121`).
2. Per-line loop. Backslash→slash on path-marker lines (`:138-140`),
   `rewrite_long_type_note_path` (`:147`), built-in path substitutions
   length-sorted (`:148-155`), `rewrite_cargo_short` (compat-only).
3. **NEW step 7:** `extra_substitutions` applied left-to-right in
   declared order via `replace_advancing` (mirrors `:154`).
4. `rewrite_type_ids` → `$TYPEID` (`:169`); trim trailing whitespace.
5. **NEW step 10:** line-drop. If trimmed line equals any `strip_lines`
   entry OR starts with any `strip_line_prefixes` entry → drop;
   otherwise push.
6. Outside loop: blank-line collapse + trailing-blank trim.

**Why extras AFTER built-ins (OQ-3).** Built-ins resolve structural
placeholders first; adopter extras then operate on partially-normalized
lines (`{ from = "$RUST/lib/rust-1.95.0", to = "$RUST" }` can refer to
the placeholder).

**Built-in placeholders NOT reserved.** Adopters may write `from`
containing `$DIR`/`$WORKSPACE`/`$RUST`/`$CARGO/registry`/`$TYPEID` —
built-in pass runs first, adopter rules see the placeholders. §7.3.

**Why extras BEFORE TypeId.** TypeId is the final invariant; extras
must run earlier so any introduced `#<digits>` collapses.

**Why line-drop AFTER trim, BEFORE blank-line collapse.** Trim first →
strip patterns match logical content. Drop before collapse → stripped
line participates in collapse.

**Per-suite plumbing.** `NormalizationContext` gains three default-empty
Vec fields; `session.rs:421-422` + `worker.rs:1529-1534` populate from
the resolved `Suite`; compat-mode (`compat/mod.rs:810,817`) stays
empty-default (§5 deferral).

## 5. Spec amendments

Amend `docs/spec/lihaaf-v0.1.md`:

- **§3.2 schema (`:303-355`)** — append three keys with default `[]` +
  TOML examples mirroring §3.1.
- **§3.4 validation rules (`:394-416`)** — append both predicates with
  rationale; include the `is_banner_shape` carve-out for the rustc
  `error:` / `warning:` / `note:` colon-space prefix family.
- **§3.6 suites (`:481-498`)** — append three keys to "DOES NOT
  inherit" list; omission → `[]`.
- **§6.2 (`:1015-1038`)** — bullet noting adopter-defined extras.
- **NEW §6.6 adopter-extensible normalization** (120-180 lines, post
  round-4 expansion):
  composition order (§4); per-suite REPLACE sharp edge (§3.2); the
  TWO predicates' rationales + worked acceptance + rejection examples;
  non-reservation of built-in placeholders; **uppercase-only `$X`
  convention prose (Decision #3 / OQ-B):** an explicit user-facing
  paragraph stating "When an adopter pattern's FIRST byte is `$`,
  lihaaf expects the next byte to be an ASCII uppercase letter
  (lihaaf's recognized placeholder shape: `$[A-Z][A-Za-z0-9_]*`),
  e.g., `$NIX_STORE`, `$SANDBOX_ROOT`. Patterns with leading
  `$lowercase` (`$nix`, `$sandbox`) are rejected at config parse
  time. The leading-`$` uppercase requirement applies even when the
  pattern also contains a path separator: `$nix/path` is REJECTED
  (this is the round-4 FIX_BEFORE_BETA-1 fix — the `is_path_like`
  predicate's leading-`$` guard fires before the path-separator
  disjunction). Interior `$lowercase` substrings WITHIN paths
  (e.g., `/path/$nix/sub`, `/some/$cache/dir`) are path text, not
  placeholder references, and are ACCEPTED via the path-separator
  branch — OQ-B governs the recognized placeholder-naming
  convention for LEADING placeholder tokens, not arbitrary `$`
  occurrences inside path text. This convention matches lihaaf's
  built-in placeholders (`$DIR`, `$WORKSPACE`, `$RUST`, `$CARGO`,
  `$TYPEID`, `$LONGTYPE_FILE`) and keeps placeholder tokens
  visually distinct from environment variable expansions or shell
  variable references in surrounding documentation.";
  **bare-placeholder full-string-anchor prose (round-6 DOC
  Finding B):** an explicit user-facing paragraph stating "Bare
  placeholder patterns are full-string anchored. A pattern like
  `$DIR` is accepted because the entire string matches the
  recognized placeholder shape — `$` + ASCII uppercase letter +
  zero or more `[A-Za-z0-9_]` characters, with NO additional
  characters after the tail. Patterns with trailing non-placeholder
  characters — `$DIR-`, `$DIR.`, `$A!`, `$RUST extra`, `$WORKSPACE,`
  — are REJECTED by the `is_path_like` predicate even though they
  start with a valid placeholder prefix. Adopters who want a
  placeholder followed by additional path content (e.g., `$DIR/x`,
  `$RUST/lib/rustlib`) must use the path-separator branch: the
  pattern is accepted because it contains `/` (or `\` on Windows),
  not because it starts with a valid placeholder prefix. This is
  the round-5 DOC Finding 3 sharpening: rule (4c) — the
  `complete placeholder token` alternative — is full-string
  anchored (`^\$[A-Z][A-Za-z0-9_]*$`), not a prefix match. If you
  need to substitute a value that follows a placeholder with no
  path separator, write the full string in `from` (e.g., `from =
  \"$VENDOR-extra-suffix\"` is REJECTED at config parse time
  because no separator is present and (4c) is full-string
  anchored).";
  **structural-banner-shape
  use cases prose (round-4 DOC_BEFORE_BETA-2 rename):** an explicit
  paragraph documenting which banner shapes are accepted by
  `is_banner_shape` (the enumerated rustc trailers via (C.1); CI-runner
  deprecation banners AND other structural banners — proc-macro
  generator deprecation banners, code-gen tool deprecation banners,
  build-system migration banners — via (C.2)) with worked examples
  for both subfamilies and a forward-compatibility note that the
  enumerated banner-prefix set may be extended in future lihaaf
  versions; **path-bearing diagnostic stripping caveat (round-4
  DOC_BEFORE_BETA-1 resolution):** an explicit user-facing paragraph
  stating "Strip patterns are validated for SHAPE (path-shaped or
  banner-shaped), not for whether the line they will match contains
  rustc diagnostic content. A path-shaped strip pattern that overlaps
  with a path-bearing diagnostic line (e.g.,
  `strip_line_prefixes = [\"error: couldn't read /build/\"]` matching
  `error: couldn't read /build/generated.rs`) WILL drop that line.
  This is by design — the adopter has explicitly opted into dropping
  lines matching the pattern. Adopters who want to preserve all
  rustc-emitted error diagnostics should not write strip patterns
  whose path component appears in rustc's `error: couldn't read`,
  `error: linking with`, or similar path-bearing error families.
  See §11 Risk 5."; **compat-mode deferral (OQ-4):** user-facing
  note stating the three keys are *unsupported in compat mode for
  v0.1.0-beta.10* (not just unimplemented).
- **§6.5 determinism (`:1075-1089`)** — byte-deterministic tuple now
  includes the three new arrays.

Manifest schema (§4.4, `:666-695`) needs no explicit field — existing
`metadata_snapshot: serde_json::Value` round-trips the table.

## 6. Patch surface

- **`src/normalize.rs:30-47`** — extend `NormalizationContext` with
  three new Vec fields; add `pub struct Substitution { pub from:
  String, pub to: String }`; builder methods mirroring
  `with_compat_short_cargo`.
- **`src/normalize.rs:99-194`** — extend `normalize` body per §4: new
  step 7 applies extras, new step 10 applies line-drop.
- **`src/config.rs:76-94, 104-167, 186-222, 320-401, 403-508`** — three
  keys on `Suite` (per-suite REPLACE, not on `Config`); `RawSuite` gains
  `Option<Vec<...>>`; `finalize_named_suite` applies REPLACE via
  `unwrap_or_default()`.
- **`src/config.rs:557-637`** — add `validate_extra_substitutions` +
  `validate_strip_patterns` per §3.3. Both call shared
  `is_path_like(s: &str) -> bool` and `is_banner_shape(s: &str) -> bool`
  private predicates. Anti-prefix and banner-prefix tables are
  `&'static [&'static str]` constants colocated with `is_banner_shape`.
- **`src/worker.rs:114-156`** — `WorkerContext::new` forwards three new
  keys into `NormalizationContext` via builders.
- **`src/worker.rs:1529-1534`** — test-helper context literal: empty
  defaults.
- **`src/session.rs:421-422`** — production-path
  `NormalizationContext::new(...)` chain extended.
- **`src/compat/mod.rs:810,817`** — unchanged; add a `// compat-mode
  adopter extras unsupported per docs/spec/lihaaf-v0.1.md §6.6` comment.

## 7. Test plan

All new tests live in `src/normalize.rs` tests module
(`normalize.rs:467-841`) and `src/config.rs` tests module
(`config.rs:757-end`).

### 7.1 Normalizer behavior (`normalize.rs` tests)

- `extra_substitutions_apply_after_builtins`
- `extra_substitutions_apply_in_declared_order`
- `extra_substitutions_empty_default_byte_identical` — regression guard
  for §1.3 default invariant.
- `strip_lines_drops_full_line_match`
- `strip_lines_drops_banner_line_match` — exact-match strip of
  `error: aborting due to 1 previous error`.
- `strip_lines_no_partial_match`
- `strip_line_prefixes_matches_prefix_only`
- `strip_line_prefixes_drops_explain_footer_family` — prefix strip
  of `For more information about this error` family across multiple
  error codes.
- `strip_line_prefixes_drops_macro_origin_trailer_family` — prefix
  strip of `note: this error originates from ` across multiple macros.
- `strip_patterns_apply_after_trim_trailing_whitespace`
- `strip_patterns_do_not_affect_diagnostic_text` — negative-regression
  guard. **NOTE (NIT-2):** normalizer composition test, not user-facing
  compat support.
- `extra_substitutions_run_before_type_id_collapse`
- `compose_with_compat_short_cargo` — both compat-mode short-CARGO AND
  `extra_substitutions` fire; order: short-CARGO first, then extras.
  **NOTE (NIT-2):** exercises normalizer-internal composition. Adopter
  extras remain unsupported in compat mode per §5 / §6.6; the test pins
  the order in code so if compat-mode adopter extras land in beta.11+,
  composition is already correct.
- `extra_substitutions_no_newline_in_to` — debug assertion mirror of
  the validation rule.

### 7.2 Config parse and per-suite (`config.rs` tests)

- `extra_substitutions_per_suite_replace_not_merge`
- `extra_substitutions_omitted_on_named_suite_is_empty` — OQ-1 pin.
- `extra_substitutions_manifest_snapshot_round_trips`
- `strip_patterns_per_suite_replace_not_merge`
- `strip_patterns_omitted_on_named_suite_is_empty`

Predicate validation tests live in §7.3.

### 7.3 Predicate-level + field-level allowlist tests (NIT-1 resolution)

The two predicates are the BLOCK-1 + BLOCK-2 fix surface. Tests are
table-driven over both acceptance + rejection matrices.

#### 7.3.1 `is_path_like` predicate-level tests

**Acceptance classes (must pass `is_path_like`):**

| Class | Sample patterns |
|---|---|
| 1. Absolute Unix path | `/nix/store/abc123`, `/build/sandbox` |
| 2. Absolute Windows path | `C:\Users\runner\.cargo`, `D:\build\target` |
| 3. Relative path with separator | `target/release`, `src\compat` |
| 4. Path segment | `nix/store`, `vendor/cargo-cache` |
| 5. Built-in placeholder bare | `$DIR`, `$WORKSPACE`, `$RUST`, `$CARGO`, `$TYPEID`, `$LONGTYPE_FILE` |
| 6. Placeholder + path suffix | `$DIR/test.rs`, `$RUST/lib/rustlib`, `$CARGO/registry/src` |
| 7. Adopter-defined placeholder | `$NIX_STORE`, `$SANDBOX_ROOT`, `$VENDOR_CACHE_2026` |
| 8. Adopter placeholder + suffix | `$NIX_STORE/rust`, `$SANDBOX_ROOT/target/release` |
| 9. Interior `$lowercase` within paths (round-5 DOC Finding 2 acceptance row) | `/path/$nix/sub`, `/some/$cache/dir`, `$WORKSPACE/$tmp/run` (pass via (4a) — rule 3 only fires on leading `$`; interior `$lowercase` is path text, not a placeholder reference) |

**Rejection classes (must NOT pass `is_path_like`):**

| Class | Sample patterns | Cause |
|---|---|---|
| A. Diagnostic-text plain | `error`, `warning`, `help`, `note`, `error:`, `E0277`, `:` | No separator, no `$X` |
| B. Round-2 BLOCK-1 surface | `  \|`, `For more information about this error`, `expected due to this` | No separator, no `$X` |
| C. Round-2 BLOCK-2 surface | `error[`, `warning[`, `error: aborting due to` | No separator, no `$X` |
| D. Length-1 separator | `/`, `\` | Length < 2 |
| E. Length-1 placeholder-start | `$` | Length < 2 |
| F. Bare `$` patterns | `$ `, `$1`, `$lowercase`, `$ NAME`, `$_` | `$` not followed by `[A-Z]` |
| F'. Lowercase `$` + path separator (post round-4 amendment, FIX_BEFORE_BETA-1 regression guard) | `$nix/path`, `$lowercase/anything`, `$_path/x`, `$1/path`, `$ /space-then-slash`, `$nix\path` | Leading-`$` guard (rule 3): `$` must be followed by `[A-Z]`, regardless of path-separator presence. Without the rule-3 guard these would have passed via (4a)/(4b). |
| F''. Placeholder shape with trailing junk (round-5 DOC Finding 3 regression guard) | `$DIR-`, `$A!`, `$RUST.`, `$DIR ` (trailing space), `$WORKSPACE,` | Rule (4c) is full-string anchored (`^\$[A-Z][A-Za-z0-9_]*$`); trailing chars outside `[A-Za-z0-9_]` make (4c) fail. None of (4a)/(4b) match because no `/` or `\` present. |
| G. Empty | `""` | Length < 2 |
| H. Whitespace-only | `" "`, `"\t\t"` | No separator, no `$X` |
| I. Embedded marker no path | `errored`, `aborted` | No separator, no `$X` |
| J. Markers + colon | `error:`, `note:`, `help:` | No separator, no `$X` |
| K. Newline-bearing | `"a\nb"`, `"$DIR\n"` | Contains `\n` |

Tests 1-18 mirror the prior round-3 list:

1. `is_path_like_accepts_absolute_unix_path` (Class 1).
2. `is_path_like_accepts_absolute_windows_path` (Class 2).
3. `is_path_like_accepts_relative_path_with_separator` (Class 3).
4. `is_path_like_accepts_path_segment` (Class 4).
5. `is_path_like_accepts_builtin_placeholder_bare` (Class 5, each of 6).
6. `is_path_like_accepts_builtin_placeholder_with_suffix` (Class 6).
7. `is_path_like_accepts_adopter_placeholder` (Class 7).
8. `is_path_like_accepts_adopter_placeholder_with_suffix` (Class 8).
8a. `is_path_like_accepts_interior_lowercase_dollar_within_path` (Class 9).
    **Round-5 DOC Finding 2 acceptance guard:** ensures interior
    `$lowercase` substrings within paths (e.g., `/path/$nix/sub`,
    `/some/$cache/dir`) pass via rule 4(a). Rule 3 must only fire
    on the LEADING `$` byte. Companion to test 14a (which guards
    that LEADING `$lowercase` is rejected even with a path
    separator) — together they pin the boundary between
    placeholder-shape enforcement (leading) and path-text
    acceptance (interior).
9. `is_path_like_rejects_diagnostic_text_plain` (Class A).
10. `is_path_like_rejects_round2_block1_surface` (Class B). **BLOCK-1
    regression guard.**
11. `is_path_like_rejects_round2_block2_surface` (Class C). **BLOCK-2
    regression guard.**
12. `is_path_like_rejects_length_one_separator` (Class D).
13. `is_path_like_rejects_length_one_dollar` (Class E).
14. `is_path_like_rejects_bare_dollar_patterns` (Class F).
14a. `is_path_like_rejects_lowercase_dollar_with_separator` (Class F').
    **Round-4 FIX_BEFORE_BETA-1 regression guard:** ensures the
    leading-`$` guard (rule 3) catches `$lowercase/path` shapes that
    the round-3 design admitted via the `/` disjunction branch. Must
    cover each sample in Class F' (`$nix/path`, `$lowercase/anything`,
    `$_path/x`, `$1/path`, `$ /space-then-slash`, `$nix\path`).
14b. `is_path_like_rejects_placeholder_with_trailing_junk` (Class F'').
    **Round-5 DOC Finding 3 regression guard:** ensures rule (4c) is
    full-string-anchored (`^\$[A-Z][A-Za-z0-9_]*$`). Must cover each
    sample in Class F'' (`$DIR-`, `$A!`, `$RUST.`, `$DIR ` with
    trailing space, `$WORKSPACE,`) and assert that none pass via
    (4c) alone. Companion assertion: `$DIR/x` MUST still pass
    overall (via 4a) but a unit-level helper that isolates (4c)
    must reject it — pins the rationale that (4c) is the
    "complete placeholder token" alternative, not a prefix-of-path
    alternative.
15. `is_path_like_rejects_empty_and_whitespace` (Classes G, H).
16. `is_path_like_rejects_embedded_markers_no_path` (Class I).
17. `is_path_like_rejects_markers_with_trailing_colon` (Class J).
18. `is_path_like_rejects_newline_bearing` (Class K).

#### 7.3.2 `is_banner_shape` predicate-level tests

**Banner-acceptance classes (must pass `is_banner_shape`):**

| Class | Sample patterns |
|---|---|
| α. Explain footer | `For more information about this error, try \`rustc --explain E0277\`.`, same with E0001, with E9999, with `--explain`-less variant |
| β. Macro-origin trailer | `note: this error originates from the macro \`m\` in the crate \`c\` (in Nightly builds, run with -Z macro-backtrace for more info)`, shorter `note: this error originates from the attribute macro \`derive_more::Display\`` |
| γ. Error-count summary | `error: aborting due to 1 previous error`, `error: aborting due to 42 previous errors`, `error: aborting due to 1 previous error; 2 warnings emitted` |
| δ. Vendored-toolchain info | `info: using rustc from /opt/vendored/rust-1.95.0/bin/rustc`, `info: switching to nightly toolchain` |
| ε. Linker version | `linker version: GNU ld (GNU Binutils) 2.40` (42 bytes; round-5 DOC Finding 5 correction), `linker version: rust-lld 15.0.7` (31 bytes; round-5 DOC Finding 5 correction). NOTE: the short `linker: lld-15.0.7` form (18 bytes) is REJECTED in round-4 amendments — it fails (A.1) 20-byte length floor. Adopters who need to strip that form should use a path-shaped pattern naming the linker binary. |
| ζ. Structural banner shape (renamed from "CI deprecation" per round-4 DOC_BEFORE_BETA-2) | CI-runner deprecation: `Node.js 16 actions are deprecated. Please update the following actions to use Node.js 20: actions/checkout@v3`, `GitHub Actions deprecation: please migrate by end-of-life date 2026-06-01`. Non-CI structural banners (round-4 amendment, demonstrates broader (C.2) applicability): `Deprecated generator output: Please update the generated API before release` (proc-macro / code-gen deprecation), `Vendored toolchain deprecated: Please update to the supported version` (build-system migration banner) |

**Banner-rejection classes (must NOT pass `is_banner_shape`):**

| Class | Sample patterns | Cause |
|---|---|---|
| α'. Length floor | `error`, `note`, `: ` (≤19 chars), `linker: lld-15.0.7` (18 chars — post round-4 amendment, BLOCK-1 regression guard) | (A.1) |
| β'. Whitespace-leading | `  \|`, `   ^^^^^`, `\t= note: ...` | (A.3) |
| γ'. Span-context first byte | `^^^^^^`, `= note: foo`, `\| trait bound` | (A.4) |
| δ'. Diagnostic-body anti-prefix | `expected one of: cascade`, `found type \`u32\``, `the trait bound \`X\` is not satisfied`, `the type \`Foo\` cannot be ...`, `cannot find function \`foo\``, `mismatched types when expected ...`, `consider importing this trait`, `help: try adding \`as_ref\`` | (B) |
| ε'. Diagnostic-keyword + colon | `warning: use of deprecated function \`f\``, `error[E0277]: trait not implemented`, `error[E0277]` | (B) `warning:` / `error[` |
| ζ'. Lowercase-leading non-rustc | `gitlab ci deprecation banner here` | (A) ok, (B) ok, (C.1) no, (C.2) lowercase first byte |
| η'. Uppercase-leading short | `Node 16 deprecated` (length 18) | (A.1) length < 20; also (C.2) length < 40 |
| θ'. Diagnostic looking like banner | `error: cannot find type \`Foo\` in scope` | passes (A), passes (B) (no anti-prefix `error: cannot`), fails (C.1) (banner prefixes are anchored to specific tails), fails (C.2) (lowercase first byte) |

Tests:

29. `is_banner_shape_accepts_explain_footer` (Class α; multiple error
    codes; `--explain`-less variant).
30. `is_banner_shape_accepts_macro_origin_trailer` (Class β; full +
    shorter variants).
31. `is_banner_shape_accepts_error_count_summary` (Class γ; singular +
    plural; with-trailing-suffix variant).
32. `is_banner_shape_accepts_vendored_toolchain_info` (Class δ).
33. `is_banner_shape_accepts_linker_version` (Class ε).
34. `is_banner_shape_accepts_structural_banner_shape` (Class ζ;
    multiple structural-banner variants). **Round-4 DOC_BEFORE_BETA-2
    rename: test name changed from
    `is_banner_shape_accepts_ci_deprecation_banner` to reflect that
    (C.2) admits non-CI structural banners (proc-macro / code-gen
    deprecation, build-system migration banners) in addition to
    CI-runner deprecation banners.** Must cover both subfamilies:
    at least 2 CI-runner variants AND at least 1 non-CI
    structural-banner variant (e.g., `Deprecated generator output:
    Please update the generated API before release`).
35. `is_banner_shape_rejects_length_floor` (Class α'). **Round-2
    BLOCK-2 regression guard, banner-surface.**
36. `is_banner_shape_rejects_whitespace_leading` (Class β'). **Round-2
    BLOCK-1 regression guard, banner-surface.**
37. `is_banner_shape_rejects_span_context_first_byte` (Class γ').
    **Round-2 BLOCK-1 regression guard, banner-surface.**
38. `is_banner_shape_rejects_diagnostic_body_anti_prefix` (Class δ').
39. `is_banner_shape_rejects_diagnostic_keyword_with_colon` (Class ε').
    **Critical carve-out test: `warning: use of deprecated function`
    must be rejected even though it contains `deprecated`.**
40. `is_banner_shape_rejects_lowercase_leading_non_rustc` (Class ζ').
41. `is_banner_shape_rejects_uppercase_leading_short` (Class η').
42. `is_banner_shape_rejects_diagnostic_looking_like_banner` (Class θ').
    **Defense-in-depth test: `error: cannot find type` fails all
    disjunction alternatives despite passing (A) and (B).**

#### 7.3.3 Field-level wiring tests

**For `extra_substitutions.from` (gated by `is_path_like` only):**

43. `validate_extra_substitutions_rejects_non_path_from` — config-parse
    test across Classes A/B/C/E/F/F'/F''/H/I/J on `from`. Class F' is
    the round-4 FIX_BEFORE_BETA-1 field-level regression guard; Class
    F'' (round-5 DOC Finding 3 — `$DIR-`, `$A!`, `$RUST.`, `$DIR `
    with trailing space, `$WORKSPACE,`) is the round-5 field-level
    regression guard pinning that rule (4c) full-string-anchoring
    propagates through the wiring layer (not just the predicate-level
    helper).
44. `validate_extra_substitutions_accepts_path_from` — config-parse
    test across Classes 1-9 on `from`. Class 9 (round-5 DOC Finding 2
    — interior `$lowercase` within paths: `/path/$nix/sub`,
    `/some/$cache/dir`, `$WORKSPACE/$tmp/run`) is the round-5
    field-level acceptance guard pinning that rule 4(a) admits
    interior-`$lowercase` path text through the wiring layer.
45. `validate_extra_substitutions_rejects_newline_in_to`.
46. `validate_extra_substitutions_accepts_compound_to` — verifies `to =
    ""`, `to = "$RUST"`, `to = "$RUST/lib/rustlib"` all pass (no `to`
    allowlist; OQ-D locked).

**For `strip_lines` / `strip_line_prefixes` (gated by
`is_path_like || is_banner_shape`):**

47. `validate_strip_patterns_accepts_path` — Classes 1-9 on both strip
    keys (path-shaped acceptance through `is_path_like`). Class 9
    (round-5 DOC Finding 2 — interior `$lowercase` within paths)
    propagates the round-5 acceptance class through the strip-key
    wiring layer (mirror of test 44 for strip keys).
48. `validate_strip_patterns_accepts_banner` — Classes α-ζ on both
    strip keys (banner-shaped acceptance through `is_banner_shape`).
49. `validate_strip_patterns_rejects_span_context` — Classes β', γ' on
    both strip keys. **Round-2 BLOCK-1 regression guard, field-level.**
50. `validate_strip_patterns_rejects_diagnostic_keywords` — Classes A,
    C, ε' on both strip keys. **Round-2 BLOCK-2 regression guard,
    field-level.**
51. `validate_strip_patterns_rejects_diagnostic_body` — Class δ' on
    both strip keys.
52. `validate_strip_patterns_rejects_disguised_diagnostic` — Class θ'
    on both strip keys. **Defense-in-depth wiring test.**
53. `validate_strip_patterns_rejects_short_and_dollar` — Classes D, E,
    F, F', F'', G, H on both strip keys (length-1, bare `$`, lowercase
    `$x`, lowercase `$x` + path separator [round-4 FIX_BEFORE_BETA-1
    regression guard], placeholder with trailing junk [round-5 DOC
    Finding 3 regression guard — `$DIR-`, `$A!`, `$RUST.`, `$DIR `
    with trailing space, `$WORKSPACE,`; mirror of test 43 for strip
    keys], empty, whitespace-only).
54. `validate_strip_patterns_rejects_newline_bearing` — Class K on both
    strip keys.

#### 7.3.4 Composition + interaction tests

55. `extra_substitutions_collision_with_builtin_placeholder` — adopter
    `{ from = "$DIR", to = "$NOT_DIR" }`. Built-in `$DIR` fires first;
    adopter rule rewrites the resulting `$DIR` literal. Pins
    composition order + non-reservation (OQ-3).
56. `extra_substitutions_collision_with_typeid_marker` — `{ from =
    "/some/path/#0", to = "/some/path/#X" }` (path-prefixed to pass
    allowlist). Extras run BEFORE TypeId.
57. `extra_substitutions_collision_with_compat_short_cargo` — both
    rules active; order short-CARGO first then extras.
58. `extra_substitutions_per_suite_override_interaction` — default
    suite 2 entries, named suite 1 different entry; fixtures see only
    the 1 (REPLACE; OQ-1 guard).
59. `strip_patterns_per_suite_override_interaction` — same shape for
    strip keys, with one path-shaped + one banner-shaped strip per
    suite.

Items 1-28 (only 1-18 and 29-42 numbered, the gap reflects the splits
between path-predicate, banner-predicate, and wiring sections):
predicate unit coverage including BLOCK regression guards on both
predicates. 43-54: field-level wiring across both strip keys. 55-59:
composition + interaction.

Optional property-test extension (if `proptest`/`quickcheck` is in
dev-deps): for any input + any allowlist-passing strip pattern,
strip does not drop any line outside the structural class the pattern
matches. NOT required for v0.1.0-beta.10.

### 7.4 Determinism regression

Existing `determinism_same_inputs_produce_same_bytes` test
(`normalize.rs:609-616`) extended with a non-empty
`extra_substitutions` + `strip_lines` + `strip_line_prefixes` triple
that exercises BOTH allowlist predicates (one path-shaped + one
banner-shaped entry in each strip key).

### 7.5 Existing test invariant

The existing 13 normalizer unit tests (`normalize.rs:493-840`) and
every existing snapshot in `tests/lihaaf/compile_*/` MUST pass
unchanged. The built-in normalizer's byte-for-byte preservation of
rustc explain footers / aborting summaries / macro-origin trailers
(`normalize.rs:569-606`) is **unchanged**: those tests still pass
because the new strip keys are opt-in and default-empty. They simply
become opt-out-able once an adopter writes a matching banner pattern.

## 8. Compat / migration

- **Empty configs** — byte-identical to beta.9.
- **In-tree lihaaf tests** — no new keys used; pass unchanged.

**sassi / djogi compatibility** (round-1 NIT-1 evidence preserved).
Inventory 2026-05-19:

- sassi: 27 `.stderr` snapshots in
  `/home/tarunvir/projects/sassi/sassi-macros/tests/lihaaf/compile_fail/`.
- djogi: 172 `.stderr` snapshots in
  `/home/tarunvir/projects/djogi/djogi-macros/tests/compile_fail/`.

```
rtk grep -rn "extra_substitutions\|strip_lines\|strip_line_prefixes" \
  /home/tarunvir/projects/sassi /home/tarunvir/projects/djogi
# Result: zero matches.
```

Keys are unshipped; no adopter manifest uses them. §3.3 validation
fires only on opt-in configs. Absent configuration, the patch is byte-
identical to beta.9 per §1.3. Snapshot regression risk for sassi /
djogi at v0.1.0-beta.10 is **zero by construction**, not by assertion.

- **§2 pilot forks** — orthogonal. Re-bless is a separate workstream
  per [[lihaaf-pilot-scope-section-2-not-3]].
- **Compat mode** — per OQ-4, beta.10 documents the feature as
  **unsupported in compat mode**, not just unimplemented.

## 9. Verification gates

Per [[lihaaf-review-verify-cmds]]:

```
cargo fmt --all -- --check
cargo clippy --all-features --jobs 2 -- -D warnings
cargo test --lib --jobs 2
RUSTDOCFLAGS=-D warnings cargo doc --no-deps --jobs 2
```

All four MUST pass. `cargo test --lib` is the behavior gate; `cargo
doc` catches rustdoc-link breakage on the new struct + builders; clippy
`-D warnings` is the lint gate; fmt is style.

The three OOM-prone integration binaries are NOT run locally per
[[lihaaf-no-local-binary-builds]]; CI handles them.

## 10. Release sequencing

1. Branch `feat/extra-substitutions-45`. PR draft.
2. `careful-coder` (Opus max) implements after Codex round-3 ALLOW.
3. Review panel: Codex xhigh on diff + `strict-swe-sonnet` final gate
   with cargo verification. Triple-ALLOW merges.
4. Merge → cut v0.1.0-beta.10.
5. Publish via `careful-publisher-haiku` after user confirms.

## 11. Risk + rollback

Schema additions are additive; projects omitting keys get byte-
identical behavior. The normalizer pass adds two `String::contains` /
`starts_with` checks per line on strip and a `replace_advancing` loop
on substitutions — bounded by adopter config length, no-op when empty.

**Rollback.** Delete the keys. No deprecation cycle required.

**Risk 1: validation regression on existing manifests.** Mitigation:
lihaaf-specific key names; collision risk low.

**Risk 2: banner-prefix list staleness.** The enumerated banner-prefix
set in `is_banner_shape` (§3.3.2 (C.1)) is closed. A future rustc
release that adds a new trailer shape (or a new structural-banner
phrasing — e.g., a CI vendor, code-gen tool, or build system that
emits an unfamiliar deprecation line) will not match until lihaaf
is updated. Mitigation: a v0.2 follow-up (§13 OQ-NEW-2) introduces
an adopter-extensible banner-prefix list, validated against the
same anti-prefix and structural-floor rules. In the interim,
adopters can match new banner families via `strip_line_prefixes`
if the banner happens to start with one of the enumerated
prefixes, or wait for a lihaaf release that extends the list.

**Risk 3: structural-banner alternative false-positives.** Rule
(C.2) accepts any uppercase-leading 40+ char line containing one of
`{"deprecated", "deprecation", "Please update", "actions to use",
"EOL", "end-of-life"}`. This covers CI deprecation banners (GitHub
Actions, GitLab CI, etc.), proc-macro / code-gen deprecation
banners, and build-system migration banners — see §3.3.2 (C.2)
rationale and §7.3.2 acceptance class ζ for worked examples in
each subfamily. Mitigation: verified by inspection that rustc emits
no top-line uppercase-leading message containing these markers.
Anti-prefix list (B) catches the `warning: use of deprecated function`
case (lowercase `w`, anti-prefix `warning:`). Should a future rustc
release violate this convention, the anti-prefix list extends.

**Risk 4: allowlist false-positives on `from`.** An adopter writes a
path-shaped pattern that overlaps with rustc content (`from = "$DIR
error"`, allowlist passes via `$DIR` prefix). Mitigation: composition
with built-in normalization makes such collisions vanishingly
unlikely — `$DIR` only appears in rustc output after lihaaf's path
substitution, and those positions are always followed by path content,
not diagnostic keywords.

**Risk 5: path-bearing diagnostic stripping (round-4
DOC_BEFORE_BETA-1 surface).** A rustc-emitted diagnostic line that
contains a path-shaped substring (`error: couldn't read
/build/generated.rs`, `error: linking with \`cc\` failed:
/build/path/to/object.o`) PASSES `is_path_like` via the path
component and therefore PASSES the strip allowlist. An adopter who
writes `strip_line_prefixes = ["error: couldn't read /build/"]`
will drop that family of rustc error lines.

**This is adopter-authorized behavior, not a framework guarantee.**
The framework's contract is that strip patterns must be either
path-shaped (`is_path_like`) or banner-shaped (`is_banner_shape`);
within the path-shaped class, the framework does not second-guess
which path-bearing lines the adopter intends to drop. An adopter
who writes an exact-match `strip_lines = ["error: couldn't read
/build/generated.rs"]` has expressed intent to drop that specific
line; a prefix-match `strip_line_prefixes = ["error: couldn't
read "]` is rejected by `is_path_like` (no `/` or `\` in the
prefix, no `$X` start) but `strip_line_prefixes = ["error:
couldn't read /build/"]` passes via the embedded `/`.

Mitigation:
1. §6.6 adopter-doc prose calls this surface out explicitly so
   adopters are not misled into thinking the framework auto-protects
   path-bearing diagnostic lines.
2. The §1.2 prose (round-4 amended) tightens the "rejected by both
   predicates" claim to cover only path-FREE diagnostic bodies.
3. The default-empty invariant (§1.3) means absent adopter config,
   no strip pattern fires; built-in normalizer preservation is
   unchanged.
4. The two-key split (§3.4) gives adopters the precise tool: exact-
   match `strip_lines` for single-instance targeting,
   `strip_line_prefixes` for family targeting. Adopters bear the
   cost of choosing the right key for their intent.

## 12. Out of scope

- **Compat-mode adopter extras** (OQ-4): §6.6 documents as
  **unsupported in compat mode for v0.1.0-beta.10**. Beta.11+.
- **§2 pilot re-bless / benchmark numbers.** Separate workstreams.
- **Regex patterns.** §6.1 prohibits regex.
- **Auto-discovery** (`.lihaaf-substitutions` file). Out.
- **Envvar overrides** (`LIHAAF_EXTRA_PATH_PREFIXES`). Out.
- **Arbitrary text rewriting via `extra_substitutions`.** Allowlist
  rejects non-path `from` by construction.
- **Rustc diagnostic stripping** (round-2 BLOCK-1 + BLOCK-2 reframing).
  Disjoint allowlists reject diagnostic-text patterns on `from` and
  strip keys. Span context is unreachable. Diagnostic-message bodies
  WITHOUT a path-shaped substring are unreachable. Path-bearing
  diagnostic bodies (e.g., `error: couldn't read /build/generated.rs`)
  pass `is_path_like` via the `/` branch and are documented as
  adopter-authorized opt-in noise removal — not framework-guaranteed
  diagnostic preservation. See §11 Risk 5 for the full surface,
  mitigations, and the adopter-choice rationale.
- **Adopter-extensible banner-prefix list.** §3.3.2 (C.1) is closed
  in v0.1.0-beta.10. Adopter-extensible list deferred to v0.2 per §13
  OQ-NEW-2.
- **v1.0.0 polish-bar items not in #45's expanded scope.**

## 13. Open questions for round-3 adversarial review

Round-1 OQ-1..OQ-4 resolved in round 2. Round-2 BLOCK-1 + BLOCK-2 and
NITs resolved in round 3. Planner round-3 OQ-A..OQ-E resolved by
user-locked decisions (§0.0). The remaining questions for Codex
round-3 critique are about the planner's `is_banner_shape` design.

**Locked-decision documentation (no longer open):**

- **OQ-A — single-segment paths / pure tokens / Windows drive-letter-
  only rejected. LOCKED defensible.** Rationale §3.3.1.
- **OQ-B — `$X` uppercase-only. LOCKED.** §5 / §6.6 must spell out
  the convention in user-facing prose.
- **OQ-D — `to` unconstrained beyond no-newline. LOCKED.** Rationale
  §3.3.3 / §3.4.

**Collapsed (no longer apply):**

- **OQ-C — banner deferral.** Collapsed by Decision #1 (banner-strip
  in scope). Not deferred.
- **OQ-E — strip allowlist symmetry with `from`.** Collapsed by
  Decision #2 (disjoint allowlists). Not symmetric.

**Open for Codex round-3 critique:**

1. **OQ-NEW-1: `is_banner_shape` design soundness.** §3.3.2 specifies
   the predicate. Codex should attack the design across at least
   these axes:
   - **Anti-prefix list completeness.** Are there rustc-emitted
     diagnostic-message-body shapes that pass (A) and (B) and (C.1)
     or (C.2)? The `θ'` test class (`error: cannot find type \`Foo\` in
     scope`) is the canary; are there others?
   - **Deprecation-marker set coverage AND tightness.** Does the
     6-element `{deprecated, deprecation, Please update, actions to
     use, EOL, end-of-life}` set admit anything rustc could emit at
     top of line with uppercase first byte? Does it cover the
     subfamilies of structural banners adopters encounter (CI
     deprecation banners across major vendors, proc-macro /
     code-gen deprecation banners, build-system migration banners)?
   - **Length floors (20 / 40).** Are these the right numbers? Is
     there a real banner under 20 chars adopters need? A diagnostic
     message body over 40 chars with uppercase-leading that contains
     a deprecation marker?
   - **Case sensitivity.** All comparisons are case-sensitive. Is
     that the right call across rustc + tool-emitted banner
     variation?
   - **`info: ` and `linker version: ` enumeration risk.** (Post
     round-4: the short `linker: ` prefix was DROPPED per BLOCK-1
     resolution; only `linker version: ` remains.) Could an unrelated
     tool emit `info: <attack-shaped suffix>` that abuses adopter
     strip? Or is the predicate's purpose adequately bounded by the
     adopter writing patterns explicitly?
2. **OQ-NEW-2: adopter-extensible banner-prefix list (v0.2 follow-up).**
   Should v0.1.0-beta.10 leave a config-shape extension point for
   adopters to add their own banner prefixes? Argument for: the closed
   list in (C.1) WILL go stale as rustc / CI tooling evolves. Argument
   against: an extensible list re-introduces the validation surface
   round 2 spent fixing; adopters could write arbitrary banner-prefix
   strings that match diagnostic-message bodies. Compromise: defer to
   v0.2 with explicit forward-compatibility prose in §6.6 saying the
   list may grow but not via adopter config in v0.1.x.

## 14. Round-3 + round-4 + round-5 + round-6 changelog

| Source finding | Resolution location |
|---|---|
| Codex round-2 BLOCK-1 (strip allows marker-free diagnostic content) | §3.3 disjoint allowlists; `is_banner_shape` (A) preconditions + (B) anti-prefix; §7.3 items 10, 36, 37, 49; §12 |
| Codex round-2 BLOCK-2 (`from` allows marker-free partial needles) | §3.3.1 `is_path_like`; §7.3 items 9, 11, 17, 43; §12 |
| Codex round-2 NIT-1 (sample-based validation tests) | §7.3 expanded to ~59 tests across two predicate matrices + field-level wiring + composition |
| Codex round-2 NIT-2 (compat-mode wording muddles contract) | §7.1 test annotations; §5 / §6.6 user-facing prose |
| Codex round-2 Concern 1 (marker-list closure) | Obsolete under disjoint allowlists; replaced by §3.3 |
| Codex round-2 Concern 2 (substring vs prefix) | Obsolete under allowlist |
| Codex round-2 Concern 3 (REPLACE sharp edge) | Preserved |
| User-lock Decision #1 (banner-strip in scope) | §0.0, §1.2, §3.3.2, §3.4, §5/§6.6, §7.3, §12, §13 |
| User-lock Decision #2 (`strip_*` is banner surface; disjoint allowlist) | §0.0, §3.3, §3.3.3, §3.4, §7.3 |
| User-lock Decision #3 (OQ-A/B/D locked, OQ-C/E collapsed) | §0.0, §3.3.1, §3.3.3, §5/§6.6, §13 |
| Planner-owned `is_banner_shape` design | §3.3.2, §7.3.2, §13 OQ-NEW-1 |

| Round-3 finding (round-4 amendment) | Resolution location |
|---|---|
| Codex round-3 BLOCK-1 (`is_banner_shape` length floor 20 contradicts (C.1) `linker: lld-15.0.7` 18-byte test vector) | §3.3.2 (C.1) — short `"linker: "` prefix DROPPED, length floor preserved at 20; §7.3.2 banner-acceptance class ε replaces `linker: lld-15.0.7` with longer `linker version: ...` vectors; new banner-rejection class α' row for `linker: lld-15.0.7` as a regression guard. Round-4 amendment rationale + adopter migration guidance inline in §3.3.2 (C.1). |
| Codex round-3 FIX_BEFORE_BETA-1 (treated as BLOCK-equivalent: `is_path_like` admits `$nix/path` via `/` branch, violates OQ-B uppercase-only contract) | §3.3.1 — new rule 3 (leading-`$` guard) fires BEFORE the disjunction. §7.3.1 rejection-class F' (`$lowercase + /`) added; test 14a `is_path_like_rejects_lowercase_dollar_with_separator` added; tests 43 and 53 updated to include Class F'. §3.3.3 error messages updated to call out the rule. §5/§6.6 prose updated to make the unconditional uppercase requirement user-facing. §3.3 "Coverage of round-4 FIX_BEFORE_BETA-1 surface" paragraph documents the closure. |
| Codex round-3 DOC_BEFORE_BETA-1 (§1.2 prose over-claims that diagnostic-message bodies are rejected by both predicates; `error: couldn't read /build/generated.rs` passes `is_path_like` via `/`) | §1.2 prose tightened — "diagnostic bodies WITHOUT a path-shaped substring are rejected by both predicates"; path-bearing diagnostic case documented as adopter-authorized, not framework-guaranteed. §11 new Risk 5 captures the full surface, mitigations, and the adopter-choice rationale. §5/§6.6 user-facing caveat paragraph added. |
| Codex round-3 DOC_BEFORE_BETA-2 ((C.2) labeled "CI-banner" but admits any uppercase-leading deprecation-marker-bearing structural banner) | §3.3.2 (C.2) renamed from "CI-BANNER STRUCTURAL" to "STRUCTURAL BANNER SHAPE"; design rationale expanded to acknowledge non-CI structural banners (proc-macro / code-gen deprecation, build-system migration). §3.3.2 worked-acceptance table gains a non-CI example (`Deprecated generator output: Please update ...`). §7.3.2 banner-acceptance class ζ renamed and gains non-CI examples. Test 34 renamed from `is_banner_shape_accepts_ci_deprecation_banner` to `is_banner_shape_accepts_structural_banner_shape` and required to cover both subfamilies. §5/§6.6 adopter-doc prose uses "structural banner shape" framing. |

| Round-4 finding (round-5 amendment) | Resolution location |
|---|---|
| Codex round-4 DOC_BEFORE_BETA Finding 1 (§12 stale "unreachable" claim contradicts Risk 5) | §12 — "Rustc diagnostic stripping" bullet tightened: span context still unreachable; diagnostic-message bodies WITHOUT a path-shaped substring unreachable; path-bearing diagnostic bodies documented as adopter-authorized opt-in noise removal via `is_path_like` `/` branch, cross-referenced to §11 Risk 5. |
| Codex round-4 DOC_BEFORE_BETA Finding 2 (OQ-B interior-dollar ambiguity for `/path/$nix/sub`) | §3.3.1 — round-5 clarification paragraph after rule 3 explicitly states rule 3 only fires on LEADING `$`; interior `$lowercase` substrings within paths are path text and pass via rule 4(a). §5/§6.6 prose updated with the LEADING-`$` qualifier and explicit "interior `$lowercase` is accepted" sentence. §7.3.1 acceptance Class 9 added (`/path/$nix/sub`, `/some/$cache/dir`, `$WORKSPACE/$tmp/run`); test 8a `is_path_like_accepts_interior_lowercase_dollar_within_path` added as the acceptance guard companion to test 14a. |
| Codex round-4 DOC_BEFORE_BETA Finding 3 (rule 4(c) "starts with" anchor ambiguity admits `$DIR-`, `$A!`) | §3.3.1 rule (4c) restated as full-string-anchored: implementer-facing regex equivalent `^\$[A-Z][A-Za-z0-9_]*$`. §7.3.1 rejection Class F'' added (`$DIR-`, `$A!`, `$RUST.`, `$DIR ` with trailing space, `$WORKSPACE,`); test 14b `is_path_like_rejects_placeholder_with_trailing_junk` added. Test also asserts `$DIR/x` passes overall (via 4a) but a unit-level helper isolating (4c) rejects it — pins the "complete placeholder token" rationale. |
| Codex round-4 DOC_BEFORE_BETA Finding 4 (stale "CI" wording after (C.2) rename) | §3.3.2 design-rationale prose ("Structural minimums only" / "Hybrid" bullets) updated from "CI banners" framing to "structural-banner" framing. §3.3.3 coverage paragraph updated: "CI-structural (C.2)" → "structural-banner (C.2)" and "CI markers" → "deprecation markers". §11 Risk 2 + Risk 3 updated to use "structural-banner" framing while preserving CI vendor examples (GitHub Actions, GitLab CI) where they appear as legitimate worked examples. §13 OQ-NEW-1 deprecation-marker / structural-banner / tool-emitted-banner framing throughout. |
| Codex round-4 DOC_BEFORE_BETA Finding 5 (wrong byte/char counts in test vectors) | §3.3.2 (A.1) rationale: `error: aborting due to 1 previous error` corrected from "41 chars" to 39 bytes. §3.3.2 design-rationale bullet: `error: cannot find type` corrected from "length 36" to "length 38" (and disambiguated to include "in scope"). §7.3.2 acceptance class ε: `linker version: GNU ld (GNU Binutils) 2.40` corrected from "41 bytes" to 42 bytes; `linker version: rust-lld 15.0.7` corrected from "29 bytes" to 31 bytes. All other length annotations re-verified by inspection (`mismatched types` = 16, `cannot find` = 11, `linker: lld-15.0.7` = 18, `Node 16 deprecated` = 18, `expected due to this` = 20, `error[E0277]` = 12, `error[E0277]: cannot find` = 25, `error: cannot find type \`Foo\` in scope` = 38). |

| Round-5 finding (round-6 amendment) | Resolution location |
|---|---|
| Codex round-5 DOC_BEFORE_BETA Finding A (field-level test descriptions still reference "Classes 1-8" / Class F-only enumerations after round-5 added predicate-level Class 9 acceptance + Class F'' rejection) | §7.3.3 — test 43 (`validate_extra_substitutions_rejects_non_path_from`) rejection-class list extended `A/B/C/E/F/F'/H/I/J` → `A/B/C/E/F/F'/F''/H/I/J` with F'' annotated as round-5 DOC Finding 3 field-level regression guard pinning rule (4c) full-string-anchoring propagation through the wiring layer; test 44 (`validate_extra_substitutions_accepts_path_from`) acceptance list extended `Classes 1-8` → `Classes 1-9` with Class 9 annotated as round-5 DOC Finding 2 field-level acceptance guard pinning rule 4(a) interior-`$lowercase` path-text propagation; test 47 (`validate_strip_patterns_accepts_path`, strip-key mirror of test 44) similarly extended `Classes 1-8` → `Classes 1-9`; test 53 (`validate_strip_patterns_rejects_short_and_dollar`, strip-key mirror of test 43 for the `$` family) rejection-class list extended `D/E/F/F'/G/H` → `D/E/F/F'/F''/G/H`. No new field-level tests added — round-5 test bodies already cover the disjunction via predicate calls; description-only propagation. |
| Codex round-5 DOC_BEFORE_BETA Finding B (§5/§6.6 silent on rule (4c) full-string anchor; adopter reading the spec amendment cannot tell that `$DIR-`, `$RUST.`, `$WORKSPACE,` are rejected) | §5/§6.6 — new `bare-placeholder full-string-anchor prose` paragraph inserted between the existing uppercase-only Decision #3 / OQ-B paragraph (round-4) and the structural-banner-shape paragraph (round-4 DOC_BEFORE_BETA-2 rename). Surface-ups the implementer-prose constraint already documented at §3.3.1 lines 421-426 (`^\$[A-Z][A-Za-z0-9_]*$`). Walks adopters through (1) what "bare placeholder" means and why `$DIR` is accepted, (2) the trailing-junk rejection family (`$DIR-`, `$DIR.`, `$A!`, `$RUST extra`, `$WORKSPACE,`), (3) the path-separator branch as the route for placeholder + path content (`$DIR/x`, `$RUST/lib/rustlib`), and (4) the implication for adopters who need substring-like substitution without a separator (write the full string; (4c)'s full-string anchor means partial-token patterns are config-parse rejected). No predicate behavior change. |

| Round-1 finding (re-confirmed) | Resolution location |
|---|---|
| BLOCK-1 round-1 (trailer-stripping vs Cluster 10.3) | §1.2 carve-outs; §1.3 built-in preservation pin; §12 |
| NIT-1 round-1 (sassi/djogi evidence) | §8 grep + counts |
| OQ-1..OQ-4 round-1 | §3.2 / §3.3 / §4 / §5 / §6.6 / §12 |
