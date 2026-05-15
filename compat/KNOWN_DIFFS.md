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

_None yet. v0.1.0-beta.4 ships this section empty; pilot PRs add rows._

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
