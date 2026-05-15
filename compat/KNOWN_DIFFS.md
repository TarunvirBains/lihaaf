# Compat-mode KNOWN_DIFFS

Tracker for documented compat-mode differences between trybuild and
lihaaf — the catalogue that informs which pilot fixtures land in
`excluded_fixtures` vs which qualify for the §5 mismatch ceiling.

Maintained alongside `compat/baseline.toml`: a baseline ceiling entry
without a corresponding KNOWN_DIFFS row is a red flag in review.

## Workflow

1. A pilot run produces a `compat-report.json` envelope with
   `results.mismatch_count > 0`.
2. The pilot owner reviews each entry in `mismatch_examples` and
   classifies it:
   - **Tracked here (KNOWN_DIFFS)** — the divergence is understood and
     either acceptable as-is or queued for a lihaaf fix.
   - **Excluded** — the fixture exercises a trybuild surface lihaaf
     does not yet implement; added to `excluded_fixtures` in the
     envelope and listed in the per-crate excluded section below.
3. The PR adjusts `compat/baseline.toml` to the post-classification
   mismatch count (only shrinking; growth requires explicit review per
   §5).

## Tracked divergences

### Wrapper-vs-per-fixture totals correlation (v0.2 work)

**Symptom:** `results.baseline.pass + results.baseline.fail` reports
per-libtest-test counts (one wrapper-function per `#[test] fn ui()`),
while `results.lihaaf.pass + results.lihaaf.fail` reports per-fixture
counts (one per `.rs` file under `tests/ui/`). The §5 totals rule's
"equal unless `excluded_fixtures` accounts for the delta" doesn't hold
for the typical trybuild usage pattern where ONE wrapper test invokes
N internal fixtures.

**Affects:** Any pilot crate using the trybuild wrapper pattern
(`t.pass(...)` / `t.compile_fail(...)` inside a `#[test]` fn). Most
existing trybuild adopters.

**Workaround for v0.1.0-beta.4:** Pilot enrollment in
`compat/baseline.toml` is operationally on hold for crates with the
wrapper pattern. Crates whose trybuild tests expose one libtest test
per fixture (uncommon) work today.

**Resolution path (v0.2):** The conservative parser needs
wrapper-aware semantics — recognize the libtest wrapper line as a
wrapper (not a per-fixture verdict), and either skip it or correlate
it to the union of internal fixtures. Spec revision may be required at
`docs/compatibility-plan.md:239-244` to clarify the per-side totals
contract.

Tracked since `1fced5c` (round-2 fixups; this issue surfaced in
round-2 adversarial review).

Each row uses the shape:

> **`<area>`** — `<short summary>`. Tracked since `<commit/PR>`.
> Plan: `<fix in lihaaf vNN | accept | excluded>`.

## Per-crate excluded fixtures

_None yet._

Each crate gets a subsection listing the fixtures it ships in
`excluded_fixtures` and why.

## Schema notes

- This is a hand-curated document — there is no parser. The §5 gate
  reads `compat/baseline.toml` (machine-readable) and the
  `excluded_fixtures` field in the envelope; this file is the human
  audit trail tying them together.
- `docs/compatibility-plan.md` §6 is the upstream spec for this
  workflow; this file is the in-tree instance.
