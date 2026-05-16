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
`TarunvirBains/<upstream-crate>-lihaaf-pilot`. The orchestrator at
`.github/workflows/refresh-pilots.yml` declares one explicit job per
fork (matrix expressions are forbidden in `jobs.<job_id>.uses`).

## Pilot ladder (compatibility-plan.md §4)

1. **Stage 1** — fork exists. One-time
   `gh repo fork upstream/<crate> --fork-name <crate>-lihaaf-pilot --clone=false`.
   The fork lands in the authenticated user's namespace
   (`TarunvirBains/...`). Do NOT pass `--org TarunvirBains`; `--org`
   takes an actual GitHub organization, and `TarunvirBains` is a
   personal account.
2. **Stage 2** — fork's CI produces a §3.3 envelope per run. This is
   what `compat/templates/pilot-stage2.yml` enables.
3. **Stage 3** — PR to this repo adds `[<crate-name>] n_max = <count>`
   to `compat/baseline.toml`. v0.1.0 GA cuts after EVERY enrolled
   pilot fork has a merged stage-3 row.

## Execution-context essentials (read before debugging)

A cross-repo reusable workflow runs in the CALLER's run context, not
the called repo's. For this project that means, for every refresh
invocation:

- `github.repository` evaluates to `TarunvirBains/lihaaf`, not the
  pilot fork. `actions/checkout` in `pilot-stage2.yml` therefore
  receives an explicit `repository:` input (the orchestrator passes
  `fork_repo`).
- Workflow logs and uploaded artifacts appear in **lihaaf's Actions
  tab**, not each fork's Actions tab. `gh run download` examples
  below pass `--repo TarunvirBains/lihaaf`.
- The fork's resolved HEAD SHA is recorded via the native `commit`
  output of `actions/checkout` (no separate `git rev-parse` step) and
  threaded into the §3.3 envelope via `cargo lihaaf --compat-commit
  "$FORK_SHA"`. Stage-3 PR reviewers reproduce from envelope alone.

### Public-only forks (v0.1)

v0.1 pilot forks MUST be public repositories. The orchestrator's
GITHUB_TOKEN is scoped to `TarunvirBains/lihaaf` only; cross-repo
`actions/checkout` against a private fork returns 403 because that
token carries zero scope on the fork repo. The four v0.1 forks
(`cxx-lihaaf-pilot`, `serde-json-lihaaf-pilot`, `anyhow-lihaaf-pilot`,
`thiserror-lihaaf-pilot`) are forks of public upstream crates, so they
default to public. Verify each fork's **Settings → General → Danger
Zone** shows "Public repository" before enrolling.

Private-fork support is **v0.2 work** and requires a PAT-based
dispatch design (the orchestrator would need a PAT scoped across each
fork, with the corresponding rotation procedure — out of scope for
v0.1 GA).

## One-time fork setup

For each new pilot fork:

1. **Fork the upstream crate** (one-time):

   ```bash
   gh repo fork upstream/<crate> \
       --fork-name <crate>-lihaaf-pilot \
       --clone=false
   ```

   The fork lands at
   `https://github.com/TarunvirBains/<crate>-lihaaf-pilot`.

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

3. **Enable cross-repo `workflow_call` access** from
   `TarunvirBains/lihaaf`. This is required for BOTH public and
   private forks — the "Allow access" gate is independent of fork
   visibility. In the fork repo:

   `Settings -> Actions -> General -> Access` -> verify that
   `workflow_call` invocations from `TarunvirBains/lihaaf` are
   permitted. The simplest setting is
   "Accessible from repositories owned by the user 'TarunvirBains'",
   or specifically permit `TarunvirBains/lihaaf`.

4. **Pin the cross-repo `uses:` SHA** in the orchestrator (one-time
   per fork) — see the "SHA-rotation workflow" section below.

5. **Add the fork to the orchestrator** by declaring a new job in
   `.github/workflows/refresh-pilots.yml` in this repo (PR-reviewed
   change — pilot enrollment is intentionally not dynamic). The job
   block mirrors the four existing jobs; the `fork_repo`,
   `pilot_name`, and `uses:` paths change per fork. Matrix
   expressions are forbidden in `jobs.<job_id>.uses`, so each fork is
   its own job.

## SHA-rotation workflow

Cross-repo reusable workflows should be SHA-pinned (per
https://docs.github.com/en/actions/security-guides/security-hardening-for-github-actions).
v0.1 ships with `@main` placeholders because the four forks do not
yet have `pilot-stage2.yml` committed — we cannot pin a SHA that
does not exist.

When a fork lands its first copy of `pilot-stage2.yml` (or when the
lihaaf template updates and the fork copies the new version), the
SHA needs to be rotated in the orchestrator:

1. User copies `compat/templates/pilot-stage2.yml` (latest from
   lihaaf main) into the fork's `.github/workflows/pilot-stage2.yml`.
2. User commits and pushes to the fork's `main`.
3. User reads the commit's SHA:

   ```bash
   cd <fork-clone>
   git rev-parse HEAD
   ```

4. User opens a PR to `TarunvirBains/lihaaf` updating
   `refresh-pilots.yml`'s `uses:` line for that fork from
   `@main` to `@<SHA>`. The TODO comment immediately above each
   `uses:` line marks the locations.
5. When the lihaaf template itself updates (e.g., a future v0.2
   hardening pass), each fork copies the new template, commits, and
   the user opens a follow-up PR to lihaaf updating the SHA.

Why `@main` is acceptable for v0.1:

- The user owns all four forks. The threat model is "user's GitHub
  credentials are compromised", which would compromise BOTH the
  orchestrator and any pinned SHA strategy.
- The follow-up "pin SHAs after fork setup" task is naturally
  bounded and one-time per fork.

## Running a refresh

Manual only in v0.1 (workflow_dispatch).

1. Navigate to <https://github.com/TarunvirBains/lihaaf/actions> ->
   "refresh pilots (stage-2 cross-repo)".
2. Click "Run workflow". Optionally override `lihaaf_version`
   (default pins to the version released alongside this PILOTS.md
   revision).
3. The orchestrator dispatches four parallel jobs — one per pilot
   fork. Each job invokes the fork's `pilot-stage2.yml` via
   cross-repo `workflow_call`. The called workflow runs in the
   orchestrator's run context but checks out the fork via
   `repository: ${{ inputs.fork_repo }}`.

Independent jobs (no `needs:`): one fork's failure does not cancel
the other three. Inspect each pilot job in the orchestrator's
Actions run.

## Inspecting results

All four pilots' logs and artifacts live in lihaaf's Actions tab
under one orchestrator run.

1. Open <https://github.com/TarunvirBains/lihaaf/actions> -> the
   most recent "refresh pilots (stage-2 cross-repo)" run.
2. Each of the four jobs (`refresh-cxx`, `refresh-serde-json`,
   `refresh-anyhow`, `refresh-thiserror`) shows:
   - the step summary table (fork SHA, mismatch_count, exit codes);
   - the envelope artifact
     (`compat-envelope-<pilot-name>-<run-id>`), 30-day retention.
3. To pull an envelope locally for review:

   ```bash
   gh run download <run-id> \
       --repo TarunvirBains/lihaaf \
       --name compat-envelope-<pilot-name>-<run-id> \
       --dir /tmp/envelope-<pilot-name>
   ```

   `<pilot-name>` is one of `cxx`, `serde-json`, `anyhow`,
   `thiserror`. Artifact names include `<pilot-name>` because all
   four shards share the orchestrator's `github.run_id` and
   `actions/upload-artifact@v4.6.2` (the SHA pinned in
   `pilot-stage2.yml`) rejects duplicate names within a run.

## Opening a stage-3 PR

After a fork produces a clean envelope:

1. **Review the envelope** at
   `/tmp/envelope-<pilot-name>/compat-report.json`.
   - `results.errors` must be empty.
   - `results.baseline.pass + .fail` should equal
     `results.lihaaf.pass + .fail` (or the delta accounted for by
     `excluded_fixtures` — see `compat/KNOWN_DIFFS.md`).
   - `commit` should be the fork's HEAD SHA at the time of the run
     (passed to lihaaf via `--compat-commit`). This is the SHA
     reviewers reproduce against.
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
   reviewers can reproduce. Include the `lihaaf_version` and `commit`
   fields from the envelope.

## Operational notes

### Trigger surface (v0.1)

The orchestrator at `.github/workflows/refresh-pilots.yml` triggers
on **`workflow_dispatch` only**. There is NO `pull_request` /
`pull_request_target` trigger. Reasoning:

- A PR-triggered cross-repo `workflow_call` would expose
  orchestrator inputs (and the orchestrator's GITHUB_TOKEN) to PR
  authors. Manual `workflow_dispatch` requires `repo` write scope,
  which PR authors don't have — so the dispatch surface stays gated
  to maintainers.
- Drift detection is manual in v0.1 (Mondays-after-release cadence,
  or ad-hoc on lihaaf releases). v0.2 may add a `schedule:` trigger
  once the dispatch surface has been re-audited under load.

### Per-pilot vs aggregated summary

The orchestrator does not aggregate per-pilot output. Matrix-job
outputs in GitHub Actions retain only the LAST matrix entry's
values (silent overwrite); separate jobs sidestep this but a clean
cross-pilot summary still needs either an artifact-collection step
or a downstream JS aggregation, both of which are fragile enough
that v0.1 ships without them.

Operators inspect each job's step summary in the orchestrator's
Actions run. Each pilot's step summary includes a ready-to-paste
TOML row for the stage-3 PR (suppressed when the parse step failed,
so a missing envelope does not produce a misleading TOML row).

### Auth

Each cross-repo `workflow_call` shard runs in the ORCHESTRATOR's run
context (`TarunvirBains/lihaaf`). The GITHUB_TOKEN available to the
called workflow is the orchestrator's, scoped by the orchestrator's
`permissions: contents: read` block. No PAT required for any of the
four pilots; all four forks are owned by `TarunvirBains`, so
cross-org governance does not apply.

### v0.2 work

- **Windows runner**: the KNOWN_DIFFS Windows-cleanup divergence is
  v0.2 work. v0.1 stage-2 runs `ubuntu-24.04` only.
- **Scheduled refresh**: `schedule: - cron: '0 6 * * 1'` is the
  intended v0.2 cadence (Mondays 06:00 UTC), gated on the
  dispatch-surface audit above.
- **Aggregated summary**: v0.2 may add a downstream summarize job
  that re-downloads each pilot's envelope artifact and emits a
  single-page summary. Designed-around-Actions-matrix-output limits.
- **Asymmetric `expected_exit_code`**: per-side baseline/lihaaf exit
  code expectation lives in v0.2 (see `KNOWN_DIFFS.md`
  schema-version section).
