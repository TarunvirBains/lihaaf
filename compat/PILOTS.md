# Compat-mode pilots (v0.1)

Operational reference for the lihaaf compat-mode pilot ladder. See
`docs/compatibility-plan.md` §4 for the spec and §5 for the gate
contract.

## Enrolled pilots (v0.1)

Four crates are on the stage-2 ladder. None are stage-3 enrolled in
`compat/baseline.toml` yet — that table ships empty in v0.1.0-beta.4.

| upstream crate | pilot fork |
| --- | --- |
| `cxx` | <https://github.com/TarunvirBains/cxx-lihaaf-pilot> |
| `serde_json` | <https://github.com/TarunvirBains/serde-json-lihaaf-pilot> |
| `anyhow` | <https://github.com/TarunvirBains/anyhow-lihaaf-pilot> |
| `thiserror` | <https://github.com/TarunvirBains/thiserror-lihaaf-pilot> |

All four forks are owned by `TarunvirBains`. Naming convention:
`TarunvirBains/<upstream-crate>-lihaaf-pilot` (matrix in
`.github/workflows/refresh-pilots.yml`).

## Pilot ladder (compatibility-plan.md §4)

1. **Stage 1** — fork exists. One-time `gh repo fork upstream/<crate>
   --org TarunvirBains --fork-name <crate>-lihaaf-pilot --clone=false`.
2. **Stage 2** — fork's CI produces a §3.3 envelope per run. This is
   what `compat/templates/pilot-stage2.yml` enables.
3. **Stage 3** — PR to this repo adds `[<crate-name>] n_max = <count>`
   to `compat/baseline.toml`. v0.1.0 GA cuts after EVERY enrolled
   pilot fork has a merged stage-3 row.

## One-time fork setup

For each new pilot fork:

1. **Fork the upstream crate** (one-time):

   ```bash
   gh repo fork upstream/<crate> \
       --org TarunvirBains \
       --fork-name <crate>-lihaaf-pilot \
       --clone=false
   ```

2. **Copy the stage-2 template** into the fork (one-time):

   ```bash
   cd /tmp && gh repo clone TarunvirBains/<crate>-lihaaf-pilot
   cd <crate>-lihaaf-pilot
   mkdir -p .github/workflows
   cp /path/to/lihaaf/compat/templates/pilot-stage2.yml \
       .github/workflows/pilot-stage2.yml
   git add .github/workflows/pilot-stage2.yml
   git commit -m "ci: add lihaaf compat stage-2 reusable workflow"
   git push origin main
   ```

3. **(Private forks only)** enable `workflow_call` access from the
   lihaaf repo: Actions -> General -> "Access" -> "Accessible from
   repositories owned by the user 'TarunvirBains'", or specifically
   permit `TarunvirBains/lihaaf`. Public forks need no extra config.

4. **Add the fork to the orchestrator matrix** at
   `.github/workflows/refresh-pilots.yml` in this repo (PR-reviewed
   change — pilot enrollment is intentionally not dynamic).

## Running a refresh

Manual only in v0.1 (workflow_dispatch).

1. Navigate to <https://github.com/TarunvirBains/lihaaf/actions> ->
   "refresh pilots (stage-2 cross-repo)".
2. Click "Run workflow". Optionally override `lihaaf_version` (default
   pins to the version released alongside this PILOTS.md revision).
3. The orchestrator dispatches one matrix shard per pilot. Each shard
   invokes the fork's `pilot-stage2.yml` via cross-repo
   `workflow_call`, running on the FORK's own runner with the FORK's
   own GITHUB_TOKEN.

`fail-fast: false`: one fork's failure does not cancel the other
three. Inspect each pilot independently per the workflow below.

## Inspecting results

Each pilot fork's Actions tab has the complete record for that pilot
— the orchestrator does NOT aggregate per-pilot output (see "v0.2
work" below for the rationale).

For each pilot:

1. Open the fork at `https://github.com/TarunvirBains/<pilot>/actions`.
2. The most recent "lihaaf compat stage-2 (pilot)" run shows:
   - the step summary table (mismatch_count + exit codes);
   - the envelope artifact (`compat-envelope-<run-id>`), 30-day
     retention.
3. To pull the envelope locally for review:

   ```bash
   gh run download <run-id> \
       --repo TarunvirBains/<pilot> \
       --name compat-envelope-<run-id> \
       --dir /tmp/envelope-<pilot>
   ```

## Opening a stage-3 PR

After a fork produces a clean envelope:

1. **Review the envelope** at `/tmp/envelope-<pilot>/compat-report.json`.
   - `results.errors` must be empty.
   - `results.baseline.pass + .fail` should equal
     `results.lihaaf.pass + .fail` (or the delta accounted for by
     `excluded_fixtures` — see `compat/KNOWN_DIFFS.md`).
2. **Pick `n_max`** >= `results.mismatch_count`. Picking exactly
   `mismatch_count` is the strictest gate; picking a small headroom
   (e.g. `mismatch_count + 2`) is acceptable per §5 review.
3. **Open a PR** to `TarunvirBains/lihaaf` adding the row:

   ```toml
   [<crate-name>]
   n_max = <chosen-count>
   ```

   Use the upstream crate's `[package].name` exactly as it appears in
   the envelope's `crate_name` field — that is the §5 gate's
   baseline.toml key.

4. **Reference the envelope artifact URL** in the PR body so
   reviewers can reproduce. Include the `lihaaf_version` from the
   envelope toolchain field.

## Operational notes

### Trigger surface (v0.1)

The orchestrator at `.github/workflows/refresh-pilots.yml` triggers on
**`workflow_dispatch` only**. There is NO `pull_request` /
`pull_request_target` trigger. Reasoning:

- Cross-repo `workflow_call` from a PR-triggered workflow exposes
  the calling repo's GITHUB_TOKEN context to the called workflow's
  environment under some configurations. Manual-only invocation
  eliminates that surface.
- Drift detection is manual in v0.1 (Mondays-after-release cadence,
  or ad-hoc on lihaaf releases). v0.2 may add a `schedule:` trigger
  once the cross-repo auth surface has been audited.

### Per-pilot vs aggregated summary

The orchestrator does not aggregate per-pilot output. Matrix-job
outputs in GitHub Actions only retain the LAST matrix entry's values
(silent overwrite); a clean cross-pilot summary needs either an
artifact-collection step or a downstream JS aggregation, both of
which are fragile enough that v0.1 ships without them.

Operators inspect each fork's Actions tab directly. Each pilot's
step summary includes a ready-to-paste TOML row for the stage-3 PR.

### Auth

Each cross-repo `workflow_call` shard runs with the CALLED
workflow's own GITHUB_TOKEN (issued by GitHub at run time, scoped to
the FORK repo). The orchestrator's own GITHUB_TOKEN is NOT
forwarded. No PAT required for any of the four pilots.

### v0.2 work

- **Windows runner**: the KNOWN_DIFFS Windows-cleanup divergence is
  v0.2 work. v0.1 stage-2 runs `ubuntu-24.04` only.
- **Scheduled refresh**: `schedule: - cron: '0 6 * * 1'` is the
  intended v0.2 cadence (Mondays 06:00 UTC), gated on the auth-surface
  audit above.
- **Aggregated summary**: v0.2 may add a downstream summarize job
  that re-downloads each pilot's envelope artifact and emits a
  single-page summary. Designed-around-Actions-matrix-output limits.
- **Asymmetric `expected_exit_code`**: per-side baseline/lihaaf exit
  code expectation lives in v0.2 (see `KNOWN_DIFFS.md` schema-version
  section).
