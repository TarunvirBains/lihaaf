# lihaaf release playbook

General publish steps and per-release checklists.

## General publish steps

These steps apply to every release and should be followed in order.

1. **Verify all milestone issues are closed.** Check the GitHub milestone
   for the target version; every issue must be either closed or explicitly
   moved to a future milestone with user authorization.

2. **Verify CI is fully green on the release branch.** All check suites
   must pass — do not publish with a failing CI lane.

3. **Bump `Cargo.toml` version.** Change `version = "0.1.0-beta.N"` to the
   target release version (e.g. `version = "0.1.0"`). Commit with message
   `chore(release): bump version to X.Y.Z`.

4. **Update `CHANGELOG.md`.** Move the `## [Unreleased]` section contents
   into a new dated section (e.g. `## [0.1.0] — YYYY-MM-DD`). Leave an
   empty `## [Unreleased]` section at the top for post-release commits.

5. **Merge the release branch to `main` via PR.** Require all reviewers and
   CI to pass before merging. Do not push directly to `main`.

6. **Tag the release commit.** After merge: `git tag v0.1.0 <sha>` and
   `git push origin v0.1.0`. The tag triggers the crates.io publish workflow
   if one is wired; otherwise continue to step 7.

7. **`cargo publish --dry-run`.** Run from the repo root against the tagged
   commit. All warnings are treated as errors at publish time; the dry-run
   catches them locally first.

8. **`cargo publish`.** With an active crates.io login (`cargo login`).
   Publishes are irreversible (yank ≠ delete) — verify the dry-run output
   before this step.

9. **Verify docs.rs build.** After crates.io indexes the crate (usually
   within a few minutes), confirm the docs.rs build succeeds and the crate
   page renders the README and API docs correctly.

10. **Close the GitHub milestone.** Mark the milestone closed after the
    crate is live on crates.io.

---

## v0.1.0 release checklist

Current milestone issues: #40, #43, #47, #53 — verify all are closed before
proceeding to publish.

- [ ] All v0.1.0-milestone issues closed (#40, #43, #47, #53 — verify
      current status on GitHub before publishing; this list may be stale)
- [ ] CI fully green on the release branch (no failing lanes)
- [ ] `Cargo.toml` `version` bumped from `0.1.0-beta.N` to `0.1.0`
- [ ] `CHANGELOG.md` `[Unreleased]` section converted to
      `[0.1.0] — YYYY-MM-DD`
- [ ] `cargo publish --dry-run` clean (run from repo root, zero warnings)
- [ ] crates.io login active (`cargo login`)
- [ ] docs.rs build clean (`RUSTDOCFLAGS="-D warnings" cargo +stable doc --no-deps --all-features`)
- [ ] README renders correctly on crates.io (no broken intra-doc links,
      code blocks display as expected)
- [ ] Git tag `v0.1.0` pushed after merge to `main`
- [ ] docs.rs page live and showing the correct crate summary and README
- [ ] GitHub milestone `v0.1.0` closed

### Pre-publish verification commands

```bash
# Format
cargo fmt --all -- --check

# Lint (jobs=2 to stay within WSL2 RAM budget)
cargo clippy --all-features --jobs 2 -- -D warnings

# Library unit tests (safe on WSL2 — does NOT run OOM-prone integration tests)
cargo test --lib --jobs 2

# Docs — must build clean; broken intra-doc links fail docs.rs at publish
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --jobs 2

# Publish dry-run
cargo publish --dry-run
```

> **WSL2 OOM warning:** Do NOT run `cargo test --all-features`,
> `cargo test --test cli_mode_errors`, `cargo test --test cleanup_dirty_worktree`,
> `cargo test --test baseline_conservative`, or `cargo lihaaf --compat ...`
> locally. These spawn subprocesses or link large binaries and will crash
> WSL2. CI handles them.
