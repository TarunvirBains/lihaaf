# v0.1 Compatibility Plan: Trybuild-derived Real-World Validation

Goal: reach v0.1 with confidence before release by proving lihaaf in the same fixture ecosystems that already validate against trybuild.

This plan starts with deterministic, repeatable checks and grows into a CI matrix.

## 1) Design principle

Treat the compatibility runner as a **feature of lihaaf itself**: one consistent command path that produces one deterministic report.

The compat driver runs both halves:

- `cargo lihaaf` executes fixtures and emits the §3.3 envelope directly.
- `cargo test` runs the baseline; the driver captures libtest output to a `baseline_capture.json` sidecar and aggregates the relevant fields into the §3.3 envelope under `results.baseline`.

The two formats stay distinct on disk (libtest output is structurally different from lihaaf's verdict catalog), and the aggregation step is the only translator. That keeps the design honest: lihaaf owns its own report, and the baseline comparison is bounded to the fields §3.3 names.

Baseline extraction is intentionally conservative. Compat mode records the original `cargo test` command result as the coarse baseline. Fixture-level baseline status may only be reported when it is derived from explicitly recognized trybuild invocations and stable path matches; otherwise the fixture baseline is `unknown` and the report must say why. The v0.1 §5 pilot gate enforces the §3.3 envelope's mismatch ceiling and per-side exit-code rule; it does NOT enforce `unknown_count == 0` (the libtest wrapper line alone produces `unknown_count >= 1` on every adopter run). `results.baseline.unknown_count` remains a diagnostic field operators can inspect; the implementation must not infer fixture-level truth from arbitrary libtest output.

## 2) What we want to validate

For each target crate, validate all of the following:

1. Baseline check
   - Run the crate’s existing trybuild tests (`cargo test` with fixture path filter if needed).
   - Record: pass/fail and failure messages.

2. Generated-lihaaf fixture check
   - Convert the same trybuild fixture set into lihaaf fixtures.
   - Run `cargo lihaaf` with equivalent filter constraints.

3. Result parity
   - Compare fixture counts and verdict outcomes in a stable form.
   - Record normalized diagnostics diffs only where lihaaf output differs from baseline.

4. Determinism
   - Re-run same command twice in clean environment and require stable report bytes (ignoring duration fields).

5. Performance envelope
   - Capture wall-clock and worker utilization.
   - Keep a before/after note so v0.1 can claim real speedups.

6. Upstream PR check
   - Prepare changes in a fork of the target crate:
     - fixture conversion shim
     - minimal `cargo lihaaf` harness/config adjustment
     - deterministic manifest overlay support (no manual one-off steps)
   - Open or update a PR back to the upstream project.
   - PR is considered “completed” only if it can be reviewed without local, non-committable steps.

## 3) Deterministic “compat mode” inside lihaaf

Implement compat as an existing-tool feature: a `--compat` mode that takes explicit, reproducible inputs and emits the §3.3 envelope.

### 3.1 Command shape

```bash
cargo lihaaf --compat \
  --compat-root /path/to/target-crate \
  --compat-cargo-test-argv '["cargo", "test"]' \
  --compat-report /tmp/compat-run.json
```

Required options when `--compat` is set:

- `--compat-root <dir>` — target crate checkout path
- `--compat-report <path>` — §3.3 envelope output path

Optional options when `--compat` is set:

- `--compat-cargo-test-argv <json-array>` — baseline command (default: `cargo test`)

  The argv value is a JSON array of command arguments, for example `["cargo", "test", "--test", "ui"]`. Compat mode must not execute this value through a shell. This keeps fork CI reproducible and avoids shell quoting or injection differences across platforms.
- `--compat-manifest <path>` — override the target crate's `Cargo.toml` (see §3.2.3)
- `--compat-commit <sha>` — recorded in envelope for traceability
- `--compat-filter <substr>` — substring filter on fixture paths (compat-mode equivalent of `--filter`)
- `--compat-trybuild-macro <path>` — additional discovery patterns for crates that wrap Trybuild in custom macros (see §3.2.1)

**Flag-shadowing rule.** When `--compat` is set, the compat-prefixed flags shadow the standard v0.1 flags:

- `--compat-filter` shadows `--filter` (in compat mode, the standard `--filter` is a mode error)
- `--compat-manifest` shadows `--manifest-path` (same shadowing rule)

When `--compat` is unset, every `--compat-*` flag is rejected as a mode error. This preserves the v0.1 stable contract (`docs/spec/lihaaf-v0.1.md` §8.5): existing scripts that use `--filter` and `--manifest-path` continue to work, and new compat-mode users learn the compat-prefixed surface without polluting it from non-compat flags.

**Compat-mode interactions with other v0.1 flags.** The remaining v0.1 stable flags (`docs/spec/lihaaf-v0.1.md` §8.2) carry the same semantics in compat mode unless explicitly shadowed above. The table below records the policy for every v0.1 stable flag so adopters do not have to guess:

| v0.1 flag | Compat-mode policy | Rationale |
|---|---|---|
| `--filter <substr>` | shadowed by `--compat-filter` (parse error if used) | filter target is fixture path; compat run owns the surface |
| `--manifest-path <path>` | shadowed by `--compat-manifest` (parse error if used) | user-facing manifest surface is compat-prefixed |
| `--bless` | pass-through, meaningful | compat mode runs the same comparison; re-blessing a Trybuild fixture is the natural way to record an accepted divergence; resulting bless writes still land in the §3.3 envelope as `blessed` outcomes |
| `--no-cache` | pass-through | stage-3 dylib build runs the same way; useful when pilot crate changes between runs |
| `--list` | pass-through | useful for sharding pilot runs across CI workers |
| `--quiet` / `-q` | pass-through | formatting only |
| `--verbose` / `-v` | pass-through | formatting only |
| `--use-symlink` | pass-through | runner-internal dylib placement; orthogonal to compat |
| `--keep-output` | pass-through | runner-internal residue control; orthogonal to compat |
| `-j <n>` / `--jobs <n>` | pass-through | worker parallelism; orthogonal to compat |
| `--help` / `-h`, `--version` / `-V` | pre-mode (no compat interaction) | clap exits before compat-mode argument parsing runs; behavior is the v0.1 default |

No compat-prefixed flag in §3.1 collides with any pass-through flag above; the only shadowings are the two named in the flag-shadowing rule.

### 3.2 Deterministic fixture conversion rules

For each crate in compat mode, do one deterministic transform pass:

- Discover Trybuild fixture invocations in crate test code (§3.2.1).
- Map Trybuild fixture categories:
  - `pass` files → `tests/lihaaf/compile_pass/`
  - `compile_fail` files → `tests/lihaaf/compile_fail/`
- Copy fixture `.rs` files and matching `.stderr` snapshots. Snapshot vocabulary alignment is handled by a compat-mode normalizer flag (§3.2.2); no separate translation pass is needed.
- Ensure the target crate can emit a dylib via the manifest overlay mechanism (§3.2.3).
- Preserve one-to-one fixture identity in the report via stable IDs: fixture path canonicalized to repo-relative forward-slash form, ASCII byte order.
- Do **not** mutate unrelated crate source in ways that could affect behavior.

### 3.2.1 Fixture invocation discovery

Discovery uses static AST analysis via `syn`. Implementing this as a string-scan would produce non-deterministic matches across crate styles; AST analysis bounds the set of recognized shapes and produces the same result on every run from clean state.

**Recognized invocation patterns (v0.1).** The discovery pass walks every `tests/*.rs` file in the target crate and recognizes:

1. **Direct `TestCases` calls** — `trybuild::TestCases::new()` followed by `.pass(<lit>)` or `.compile_fail(<lit>)` chained terminators. The literal argument is the fixture path (absolute or repo-relative).
2. **Glob arguments** — when the literal contains an asterisk, question mark, or a square-bracket character class (e.g. `[abc]`), the discovery pass resolves the glob from the test file's directory using stdlib `std::fs` traversal (no `glob` crate dependency).
3. **`#[test]`-wrapped invocations** — `#[test] fn <name>() { let t = trybuild::TestCases::new(); t.compile_fail(...); ... }`. The discovery pass descends into `#[test]` bodies and applies pattern 1 inside.

**Unrecognized shapes.** Anything that does not match the patterns above (dynamically-built fixture lists, macro-expansion-time generation, custom wrapper macros without `--compat-trybuild-macro` registration) produces an `errors` entry of type `discovery_unrecognized` in the §3.3 envelope, naming the file and line of the unrecognized AST node. Fixture-set discovery continues; unrecognized shapes do not abort.

**Custom wrapper escape hatch.** `--compat-trybuild-macro <path>` accepts a fully-qualified macro path (e.g. `mycrate::ui_tests`) that the AST walk treats as an alias for `trybuild::TestCases::new()`. The argument is a path, not a regex or substring. Multiple `--compat-trybuild-macro` flags are allowed and OR'd.

**Determinism guarantee.** For the enumerated patterns, two runs from clean state produce the same fixture-set ordering (sorted by repo-relative path, forward-slash, ASCII byte order). For unrecognized shapes, the determinism guarantee covers the `discovery_unrecognized` entries only — the patterns themselves stay out of scope until they enter the recognized set in a later version.

### 3.2.2 Snapshot vocabulary alignment

Trybuild's checked-in `.stderr` snapshots use the same four placeholder names lihaaf does: `$DIR`, `$WORKSPACE`, `$RUST`, `$CARGO`. The first three rewrite to the same forms in both normalizers. The fourth diverges:

- Trybuild emits `$CARGO/<crate>-<ver>/<rest>` — registry host directory and 16-byte index hash stripped (see `trybuild/src/normalize.rs:304-317`). Trybuild recognizes both `/registry/src/github.com-<hash>/` (cargo ≤ 1.69 layouts and `[source]` overrides) and `/registry/src/index.crates.io-<hash>/` (cargo ≥ 1.70 sparse registry) via a two-branch `find().or_else()` (`trybuild/src/normalize.rs:305-306`).
- Lihaaf emits `$CARGO/registry/src/index.crates.io-<hash>/<crate>-<ver>/<rest>` — full registry layout preserved via a literal-prefix substitution (see `lihaaf/src/normalize.rs:83`, which does `push_path(reg, "$CARGO/registry")`).

A direct first-run comparison would diff on any fixture that references a dep's source path.

**Resolution: compat-mode normalization convergence.** Lihaaf's normalizer gains a compat-mode flag (`NormalizationContext::compat_short_cargo`) that emits Trybuild's shorter `$CARGO/<crate>-<ver>/<rest>` form. The compat driver sets the flag; non-compat lihaaf is unchanged and keeps the v0.1 stable contract for snapshot byte format. With the flag set, all four placeholders converge on the same **shape** between the two normalizers, and the comparison is direct — no translation table, no runtime lookup of the registry index hash, no compat-only translator module.

**Scope-asymmetry caveat.** Trybuild gates all four placeholder rewrites to lines whose prefix is `--> ` or `::: ` (the `if prefix.is_some()` block at `trybuild/src/normalize.rs:190`). Lihaaf's substitution loop applies on every line (`lihaaf/src/normalize.rs:100,117-124`). On real rustc output the gap is small — rustc almost always emits absolute paths on span lines only — but a path embedded inside a `note:` body, a panic message, or other inline diagnostic text will be rewritten by lihaaf and left verbatim by Trybuild. That divergence surfaces in the §3.3 envelope as a `snapshot_mismatch` entry of subtype `non_span_path_rewrite` and counts toward `mismatch_count` against the §5 gate. The compat driver does not narrow lihaaf's substitution scope to match Trybuild — narrowing would silently change non-compat lihaaf's contract on the same line shapes and is a v0.2 conversation; instead, the asymmetry is surfaced and accepted as part of `N_<crate>`.

**Placeholder source-of-truth (cross-checked against trybuild source).**

| Placeholder | Trybuild source | lihaaf source | Status |
|---|---|---|---|
| `$DIR` | `trybuild/src/normalize.rs:185, 247, 460` | `lihaaf/src/normalize.rs:79` | identity |
| `$WORKSPACE` | `trybuild/src/normalize.rs:261, 464` | `lihaaf/src/normalize.rs:80` | identity |
| `$RUST` | `trybuild/src/normalize.rs:284, 289, 299` | `lihaaf/src/normalize.rs:81` | identity (Trybuild has three branches for different rustup layouts; all converge on `$RUST`) |
| `$CARGO` | `trybuild/src/normalize.rs:317` | `lihaaf/src/normalize.rs:83` | resolved via the compat-mode normalization flag above |

**Implementation surface.** Lihaaf's existing normalizer (`src/normalize.rs`) substitutes `$CARGO/registry` via a literal-prefix `push_path` call (line 83) executed inside the `replace_advancing` loop (line 116). That substitution is correct for the v0.1 stable contract and stays unchanged for non-compat callers. Compat-mode adds:

1. A new `compat_short_cargo: bool` field on `NormalizationContext`, default `false`. Non-compat callers leave it `false` and observe byte-identical v0.1 normalizer output.
2. A new compat-mode helper alongside `push_path` (call it `push_cargo_short` or similar) that does **structural** hash-shape recognition rather than a literal-prefix replace. The helper walks the line bytes looking for `/registry/src/github.com-` or `/registry/src/index.crates.io-`; on a match, it checks that the next 16 bytes are ASCII lowercase hex (using stdlib `u8::is_ascii_hexdigit` filtered to lowercase, plus explicit byte-range comparison — no regex per §6.1) and the byte after the hash is `/`. When all three conditions hold, the helper strips `/registry/src/<host>-<hash>` and emits `$CARGO/<crate>-<ver>/<rest>`. The byte-walker style mirrors Trybuild's own logic at `trybuild/src/normalize.rs:304-317` and stays inside lihaaf's "no regex" rule.
3. Selection logic: when `compat_short_cargo` is `true`, the compat helper runs in place of the literal-prefix `$CARGO/registry` substitution. When `false`, the existing `push_path("$CARGO/registry")` substitution runs as before.

**Scope and tests.** Roughly ~30–50 lines of Rust (the helper + the bool field + the selection branch + unit tests). Tests under `src/normalize.rs` assert: (a) compat-mode output for an `index.crates.io-<hash>` input matches Trybuild's `$CARGO/<crate>-<ver>/<rest>` form; (b) the older `github.com-<hash>` form is handled identically; (c) a path with no registry segment is left unchanged; (d) with the flag `false`, output is byte-identical to today's v0.1 normalizer. No new top-level module; no regex; v0.1 stable-contract byte format outside compat mode is unchanged.

### 3.2.3 Manifest overlay mechanism

The target crate's `Cargo.toml` may or may not declare `[lib] crate-type = ["dylib"]`. The compat driver handles both cases internally without exposing a `--manifest-path` flag to the user — the user-facing surface is `--compat-manifest <path>` (defaulting to the upstream `Cargo.toml` discovered from `--compat-root`).

When the upstream manifest already declares dylib crate-type, the driver still materializes a staged overlay (uniform §3.3 envelope classification — the overlay always exists, the `overlay.upstream_already_has_dylib` flag records whether the dylib declaration was a real change or a redundant one). When the upstream manifest does not declare dylib crate-type, the same staged overlay carries the `[lib] crate-type = ["dylib", "rlib", ...]` canonicalization. In both cases the dylib build's internal `cargo rustc` invocation is pointed at the staged overlay, not the upstream `Cargo.toml`. The staged form keeps the upstream manifest byte-identical and isolates the overlay to a file the user does not edit. Cargo's own `--manifest-path` flag is the driver's internal stage-3 build flag, not the user's lihaaf CLI flag (which is shadowed in compat mode per §3.1).

**Why the staged overlay lives at `<upstream>/target/lihaaf-overlay/Cargo.toml`.** Cargo validates the `--manifest-path` filename at startup and rejects any path whose last component is not literally `Cargo.toml` (exit code 1, "the manifest-path must be a path to a Cargo.toml file"). Staging the overlay under `<upstream>/target/lihaaf-overlay/Cargo.toml` satisfies that constraint while keeping the file isolated from the upstream `Cargo.toml`. The `<target_root>/target/` subtree is treated as implicitly ignored by the cleanup classifier (see `src/compat/cleanup.rs` for the `<target_root>/target/` short-circuit), so the overlay never pollutes the fork's worktree regardless of whether the fork's `.gitignore` covers it.

**Why a staged-target overlay (vs. sibling, rewrite-then-restore, or `[patch]`).**

- **Sibling-manifest overlay** (`<upstream>/Cargo.lihaaf.toml` next to the upstream `Cargo.toml`) was the original v0.1.0-beta.4 shape. Cargo's `--manifest-path` filename check rejects any path whose last component is not literally `Cargo.toml` — every stage-2 pilot run failed with `lihaaf_session_failed` / detail "the manifest-path must be a path to a Cargo.toml file" until the PR #34 redesign moved the overlay under `target/lihaaf-overlay/`.
- **Rewrite-then-restore** touches the upstream `Cargo.toml` mid-run; a concurrent file watcher, IDE, or another `cargo` process observing the worktree sees the modified state, and a crash before restore leaves the worktree dirty.
- **`[patch]` overlays** target dependency resolution, not crate-type declaration; they cannot add `crate-type = ["dylib"]`. The overlay passes through `[patch.<registry>.X]` `git`/`branch`/`tag`/`rev` keys verbatim. The `path` sub-key IS absolutized with the same semantics as `[dependencies.X] path` — a fork that carries `cxx = { path = "." }` would otherwise resolve against the staged manifest dir after the overlay is placed two dirs deeper.
- **Staged-target overlay** keeps the upstream manifest byte-identical, satisfies cargo's filename check, isolates the overlay to a file the user never edits, lives under the cargo-ignored `target/` subtree (so no `.gitignore` entry is needed in the fork), and crash-recovers cleanly (the staged file is just leftover state under `target/`).

**Overlay file contents.** The staged manifest is produced by reading the upstream `Cargo.toml` once and re-serializing with:

1. `[lib] crate-type` set to `["dylib", "rlib"]` if absent, or extended in-place if present (the `rlib` is retained so the `cargo test` baseline still works).
2. Every path-bearing key absolutized against the upstream crate dir. Cargo resolves path-bearing keys against the manifest's parent directory; the staged overlay lives two directories deeper than the upstream `Cargo.toml`, so relative paths must be rewritten to absolute (forward-slash) form before serialization. The covered key set is: `[lib] path`, `[[bin]] path`, `[[example]] path`, `[[test]] path`, `[[bench]] path`, `[dependencies.X] path`, `[dev-dependencies.X] path`, `[build-dependencies.X] path`, `[target.*.<deps>] path`, `[workspace] members`, `[workspace] exclude`, `[workspace] default-members`, `[workspace.dependencies.X] path`, `[package].workspace`, `[package] build`, `[patch.<registry>.X] path` (see note below on `[patch]` handling), and `[replace."<source-id>"] path` (the older soft-deprecated replacement form; same absolutization semantics as `[patch]`). The `[lib] path` is unconditionally injected to point at the upstream `src/lib.rs` so cargo's auto-discovery does not search the (empty) staged dir. `[package] build` is injected when the upstream carries a `build.rs` so the build script still compiles correctly under the overlay. Auto-discovery for `[[bin]]`, `[[example]]`, `[[test]]`, `[[bench]]` is explicitly disabled (`autobins = false`, etc.) because the staged dir has no `src/bin/` / `tests/` / `examples/` / `benches/` to discover from.
3. All other dependency semantics are preserved. The generated overlay is intentionally canonicalized and is not a byte-for-byte copy of the upstream manifest.
4. Table key order sorted in cargo's canonical order (`package`, `lib`, `bin`, `dependencies`, `dev-dependencies`, `build-dependencies`, `features`, `workspace`, then alphabetical for the long tail).
5. No comments — upstream `Cargo.toml` comments are dropped in the overlay. If a comment carries load-bearing meaning (e.g. a `[patch]` rationale), the overlay records the comment text in the §3.3 envelope under `overlay.dropped_comments` rather than preserving it in the TOML. No trailing whitespace, line endings `\n`.

**Serializer choice.** The compat driver serializes the overlay through the existing `toml` crate dependency (`Cargo.toml:39`, currently `toml = "1"`), pinned to the same major version lihaaf itself depends on. Adopting `toml_edit` or any other serializer is out of scope for v0.1.0 GA — a change here would break the byte-determinism guarantee below across lihaaf versions and is a v0.2 conversation. Two lihaaf binaries built against the same `toml` 1.x patch level produce byte-identical overlays; two binaries built against different `toml` 1.x patch levels are expected to match in practice but are only guaranteed by the test below.

**Determinism guarantee.** Two compat-mode runs from clean state against the same upstream `Cargo.toml`, run by the same lihaaf binary, produce a byte-identical `target/lihaaf-overlay/Cargo.toml`. A test asserting this lives at `tests/compat/overlay_determinism.rs`. Cross-binary determinism (across `toml` 1.x patch upgrades) is asserted by a second test against a small fixture corpus checked into `tests/compat/overlay_corpus/`; the corpus is regenerated whenever the `toml` dependency is bumped, and divergence triggers an `overlay_serializer_drift` error in the §3.3 envelope rather than silently changing the overlay bytes.

**Determinism caveat: absolutized paths bake in the upstream crate dir.** Because the path-absolutization step writes the upstream crate's absolute path into the overlay TOML, two runs from clean state against the **same** crate dir produce byte-identical overlays; two runs from clean state against **different** crate dirs (e.g. CI runners with different checkout paths) produce overlays that differ in the absolutized-path fields but match in every other byte. The §3.3 envelope's determinism contract (`mismatch_count`, `errors`, `crate_name`, etc.) is unaffected because the envelope never embeds the overlay's path fields verbatim — `commands.lihaaf` contains the overlay manifest path only as a shell-friendly hint, not as a determinism input.

**Dirty-worktree rule.** Generated overlays, copied fixture trees, and generated Lihaaf snapshots must either live under an ignored compat output directory or be deliberately included in the PR payload. A compat run must not leave ambiguous untracked files in the fork. The report must list every generated path and classify it as `committed`, `ignored`, or `cleaned`. The staged overlay at `target/lihaaf-overlay/Cargo.toml` always classifies as `ignored` via the cleanup classifier's `<target_root>/target/` short-circuit, regardless of the fork's `.gitignore` state.

**Cleanup.** The staged overlay lives under `target/lihaaf-overlay/`, which the cleanup classifier's `<target_root>/target/` short-circuit treats as implicitly ignored — the file is preserved across runs by design (cargo owns the `target/` subtree). Cleanup runs on normal exit and on panic via `std::panic::set_hook` — the panic hook fires during stack unwinding and `Drop` covers panic / early-return paths, so the cleanup itself is guaranteed across the normal-exit, error-return, and panic axes for the OTHER generated artifacts (converted fixtures, sidecar files). **SIGINT/SIGTERM cleanup is OUT OF SCOPE for v0.1** — installing a signal handler would either pull in a new crate (`ctrlc` / `signal-hook`) or hand-roll cross-platform FFI; both expand the dependency surface for marginal gain. If the driver is interrupted by signal mid-run, generated paths may remain in the worktree; `--keep-output` is the supported recovery surface for inspecting that residue, and re-running `cargo lihaaf --compat` (without `--keep-output`) cleans up next time. The single exception is `--keep-output`, which preserves all generated paths for local debugging (§8.2 of the v0.1 spec); the report records the residue paths and classifies them `kept` instead of `cleaned`. A panic during overlay generation itself (i.e. before the exit hook is registered) is treated as a defect and the panic message names the partial path so the operator can remove it manually.

### 3.3 Report format

Write one deterministic JSON envelope:

```json
{
  "schema_version": 1,
  "mode": "compat",
  "crate": "serde-json",
  "commit": "abcdef123",
  "commands": {
    "baseline": "cargo test --test trybuild_tests",
    "lihaaf": "cargo lihaaf --compat --compat-root ."
  },
  "results": {
    "baseline": {"pass": 120, "fail": 5, "exit_code": 0, "dur_ms": 42000},
    "lihaaf": {"pass": 120, "fail": 5, "exit_code": 0, "dur_ms": 9000, "toolchain": "stable-x86_64-unknown-linux-gnu"},
    "mismatch_count": 0
  },
  "mismatch_examples": [
    {
      "fixture": "tests/trybuild/compile_fail/foo.rs",
      "type": "baseline_only_fail|lihaaf_only_fail|verdict_mismatch|snapshot_mismatch|infra_error",
      "notes": "..."
    }
  ],
  "errors": [
    {
      "type": "discovery_unrecognized|toolchain_drift|manifest_overlay_failed|...",
      "fixture": "tests/...",
      "file": "tests/foo.rs",
      "line": 42,
      "detail": "..."
    }
  ],
  "excluded_fixtures": [
    {"fixture": "tests/...", "reason": "..."}
  ]
}
```

**Stability rules.**

- `schema_version` is the integer envelope schema version (currently `1`). A breaking change to the envelope shape increments this; additive fields do not.
- `mismatch_examples` is sorted by `fixture` (repo-relative, forward-slash, ASCII byte order). `errors` is sorted by `file` then `line`. `excluded_fixtures` is sorted by `fixture`.
- All `fixture` and `file` paths are repo-relative, forward-slash, never absolute. The target crate's repo root is the relative root. Structured path conversion uses a fallible prefix strip; an out-of-root absolute trybuild literal is rendered with an explicit non-absolute `outside-base/...` diagnostic prefix rather than serializing the runner's absolute path.
- `errors[].detail` free-text strings are normalized: the absolute `compat_root` prefix is stripped from any embedded path before serialization. Infrastructure errors (e.g. `DylibBuildFailed`) embed the cargo invocation, which uses absolute `--manifest-path` and `--target-dir` values. The normalization is applied at the envelope write boundary via the same prefix-stripping mechanism as `commands.lihaaf` (R3 FIX class III / R5 FIX class V). Local terminal output from `cargo lihaaf --compat` still shows absolute paths; only the §3.3 envelope artifact is normalized.
- `dur_ms` fields are non-deterministic and explicitly excluded from determinism checks (per §2 item 4, Determinism). Every other field is part of the determinism guarantee.

**Fields the §5 gate reads.**

- `errors` length (gate: `errors == []`)
- `results.mismatch_count` (gate: `<= N_<crate>`, see §5)
- `results.baseline.pass + results.baseline.fail` and the matching `results.lihaaf.*` totals (gate: equal unless `excluded_fixtures` accounts for the delta)
- `results.baseline.exit_code` and `results.lihaaf.exit_code` (gate: both `0`, or both equal and documented as expected-fail in the crate's matrix entry)

### 3.4 Toolchain resolution in compat mode

Compat mode interacts with two toolchain-drift mechanisms:

1. **Pilot pinning** (§4 — "one pinned toolchain") — every pilot fork runs against a toolchain recorded in the fork's `rust-toolchain.toml`.
2. **Lihaaf freshness check** (`docs/spec/lihaaf-v0.1.md` §4.5, §4.6) — every fixture dispatch re-validates `rustc --version --verbose` against the dylib's build-time rustc and hard-fails with exit code 67 on drift.

These two mechanisms are compatible if and only if the dylib build and every fixture dispatch resolve to the same `rust-toolchain.toml`. Compat mode enforces this:

- The compat driver invokes `rustup show active-toolchain` before stage 3 of the lihaaf session and records the resolved toolchain string in the §3.3 envelope under `results.lihaaf.toolchain`.
- The dylib is built inside the same `rust-toolchain.toml` resolution as the fixture dispatch loop. The freshness check still runs per-dispatch and still hard-fails on drift; in compat mode, drift is impossible by construction unless the toolchain mutates mid-session.
- The pilot fork's `rust-toolchain.toml` is the source of truth for the resolution. The compat driver does not override it.

If the fork has no `rust-toolchain.toml`, the compat driver records the resolved toolchain in the §3.3 envelope and proceeds (no toolchain file is materialized; the resolution comes from `rustup`'s default, usually stable). For pilot reproducibility, the fork commits a `rust-toolchain.toml` once the pilot's baseline run is recorded; subsequent runs against the committed toolchain are byte-deterministic.

**Compat-mode narrowed correctness claim.** The v0.1 spec's full §4.5 invariant set (file existence, mtime, SHA-256, rustc version) and §4.6 hard-fail behavior are preserved unchanged. The claim compat mode adds: pilot reproducibility depends on a committed `rust-toolchain.toml` in the fork, not on lihaaf's rustc-version drift detection.

**Precondition: no concurrent cargo activity during a compat run.** The §4.5 invariants are checked per fixture dispatch. A concurrent `cargo build` in the same worktree (IDE save, another shell, file watcher) can replace the dylib mid-session and trip the SHA-256 or mtime invariant, hard-failing the run with exit 67. Compat-mode pilots run in CI by default (§4.4), where this is not a concern. For local pilot runs, close other cargo activity in the worktree before invoking `cargo lihaaf --compat`.

## 4) Pilot crate plan (fork-driven)

Fork workflow:

- Create/refresh a short list of target forks.
- Run compat checks from each fork locally using the same pinned commit.
- If changes are needed to run lihaaf, push deterministic compatibility adjustments to the fork and update a PR.
- Track PR outcomes (`open`, `merged`, `needs changes`, `rejected`) in the compatibility report metadata.

Initial fork shortlist (from `trybuild` reverse-dependency scan + fixture count pass):

| Candidate crate | Downloads (crates.io sample entry) | Max version | Fixture files (`tests/*.rs + *.stderr`) | Repo | Upstream PR expectation | Why |
| --- | ---: | --- | ---: | --- | --- |
| `derive_more` | 31,380,040 | `2.1.1` | 173 | https://github.com/JelteF/derive_more *(fork: `derive_more-lihaaf`)* | Prefer PR (if maintainer-friendly) | heavy compile-fail coverage, pure macro ecosystem |
| `cxx` | 1,752,744 | `1.0.194` | 171 | https://github.com/dtolnay/cxx *(fork: `cxx-lihaaf`)* | Fork-only run; no PR dependency | large parser/FFI macro tests, macro-heavy |
| `axum-macros` | 1,510,918 | `0.5.1` | 158 | https://github.com/tokio-rs/axum *(fork: `axum-macros-lihaaf`)* | Prefer PR (if maintainer-friendly) | public API-heavy macro crate with many fixtures |
| `bon` | 2,859,535 | `3.9.1` | 115 | https://github.com/elastio/bon *(fork: `bon-lihaaf`)* | Prefer PR (if maintainer-friendly) | derive-heavy and macro-heavy compile-fail surface |
| `rocket_codegen` | 4,529,987 | `0.5.1` | 116 | https://github.com/rwf2/Rocket *(fork: `rocket_codegen-lihaaf`)* | Prefer PR (if maintainer-friendly) | web-framework macro patterns + migration-relevant fixtures |

**Coverage-matrix requirement.** The pilot set must span the macro-shape matrix below, not merely meet the `>=140` fixture threshold:

| Shape | Pilot coverage required | Why |
|---|---|---|
| derive macros | yes (`derive_more`, `bon`) | most common adopter shape |
| function-like proc-macros | yes (`cxx`) | parser/codegen-heavy diagnostics |
| attribute proc-macros | yes (`axum-macros`, `rocket_codegen`) | wraps user functions; different diagnostic shapes |
| glob-style fixture lookup | yes (`derive_more`) | exercises the §3.2.1 glob path |
| `#[test]`-wrapped Trybuild | yes (`axum-macros`) | exercises the §3.2.1 descend-into-test path |
| custom wrapper macro | optional | exercises `--compat-trybuild-macro`; deferrable if no recognized v0.1 pilot uses one |

A pilot set that meets the `>=140` threshold but misses a row above is not the v0.1 gate set. Add a smaller crate to fill the row before declaring §7's "pilot set is stable" criterion satisfied.

Backlog / stretch candidates if the first pass is healthy:

- `modular-bitfield-msb` (171 fixtures, lower download share)
- `binrw` (161 fixtures)
- `doku` (143 fixtures)
- Stretch fork naming rule: `<crate>-lihaaf` for all planned target forks.

Observed result from this search pass:

- No reverse-dependency crate met `>=200` fixture files in the sampled set.
- `derive_more` and `cxx` are the best high-signal options to start (both >170 fixtures).
- Keep the shortlist editable as counts change; re-run this scan before each milestone.
- `cxx` qualifies as a high-fidelity local validation target because it is closely tied to a first-party trybuild workflow; treat it as fork-only while preserving its data in the fork+CI evidence path.
- `cxx` is fork-only with no in-tree corpus carve-out. An earlier draft considered "moved inside lihaaf" as a fallback for the case where upstream PRs are blocked; that option is closed because an in-tree pinned corpus would drift from upstream and the compat signal would degrade into a self-consistency check — the opposite of §8's deterministic-against-the-ecosystem promise. If `cxx`'s maintainers reject the PR, the pilot result records that and `cxx` remains a fork-only data point.

Selection gate for v0.1 pilots:

1. Priority-1: keep candidates with fixture count >=140 and high downloads.
2. Priority-2: keep candidates below 140 only if they expose a different ecosystem pattern (macro style, proc-macro constraints, platform constraints).
3. Any candidate requiring local manifest edits must be routed through temporary deterministic manifest overlay only.

Phased rollout:

1. Pilot (3–5 crates): identify blockers and normalize adapter.
2. Ramp (10–15 crates): collect stable pass-rate stats.
3. Gate set (20+ crates): add to CI matrix once mismatch rate is consistently low.

For each new crate run:

- one pinned toolchain (e.g. stable)
- one pinned dependency revision
- full log and JSON artifact retained for 30 days

### 4.4 Distribution into fork CI

The compat workflow assumes `cargo lihaaf` is callable inside the fork's CI environment. Two paths are admissible for v0.1:

1. **`cargo install` from crates.io** (recommended for pilots):
   ```yaml
   - name: Install lihaaf
     run: cargo install lihaaf --version 0.1.0-alpha.2 --locked
   ```
   Trade-off: 30–60s install cost per CI run, paid every run unless cached. Cache key includes the lihaaf version and the rustc version. No upstream-manifest modification.
2. **Pinned binary from a release asset** (deferred to v0.1.0 GA): the lihaaf release workflow attaches a static-linked `cargo-lihaaf` binary to GitHub releases; fork CI downloads it directly. Trade-off: faster install, requires lihaaf to maintain the release-asset pipeline.

The compat plan does **not** support adding lihaaf as a dev-dependency in the fork's upstream `Cargo.toml`. That would touch the upstream manifest, which §3.2.3 forbids. Forks that need a deeper integration than `cargo install` carry that integration in the fork's own `Cargo.toml` only when the fork itself authors the change deliberately (and accepts that the staged overlay will canonicalize that dependency through its own re-serialization); the v0.1 compat driver never injects dependencies into the staged overlay on behalf of the fork.

**Sample fork CI workflow.**

```yaml
# .github/workflows/lihaaf-compat.yml in the fork
name: lihaaf compat
on: [push, pull_request]
jobs:
  compat:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - name: Show active toolchain
        run: rustup show active-toolchain
      - name: Install lihaaf
        run: cargo install lihaaf --version 0.1.0-alpha.2 --locked
      - name: Run compat
        run: cargo lihaaf --compat --compat-root . --compat-report /tmp/compat-report.json
      - uses: actions/upload-artifact@v4
        with:
          name: compat-report
          path: /tmp/compat-report.json
```

## 5) CI integration plan

Add a dedicated job (not merged into default quick checks):

- Matrix over `[{crate, commit, fixture_count}]`
- Steps:
  1. checkout target crate at pinned commit
  2. run compatibility mode
  3. parse JSON report
  4. enforce gating thresholds

Initial gating thresholds (per pilot crate, all expressed as concrete fields in the §3.3 envelope):

- `errors == []` — no infra or command failures
- `results.baseline.exit_code == 0 AND results.lihaaf.exit_code == 0` — or both equal and documented as expected-fail in the crate's matrix entry
- `results.mismatch_count <= N_<crate>` — `N_<crate>` is a concrete integer per pilot crate, frozen at pilot-merge time at the value the first stable run produced
- `results.baseline.pass + results.baseline.fail == results.lihaaf.pass + results.lihaaf.fail` — or the delta is accounted for by entries in `excluded_fixtures`

**`N_<crate>` shrinking process.** Each `N_<crate>` lives in a tracked file (`compat/baseline.toml`) keyed by crate name. The CI gate reads the value from that file. Decrementing `N_<crate>` is a PR that touches `compat/baseline.toml` — the gate continues to pass at the new lower N because the bug that caused the prior mismatch was fixed. CI does not enforce a trend; it enforces the value committed at the time the run executes. The shrinking trajectory is enforced by review (PRs only ever decrement `N_<crate>`, never increment).

Add a dashboard-style artifact upload (`compat-report.json`) so regressions are diagnosable quickly.

## 6) Issue triage process for mismatches

When lihaaf diverges from trybuild baseline:

1. Capture exact fixture and fixture output.
2. Classify mismatch:
   - baseline gap (adopter crate issue or flake)
   - normalization mismatch
   - harness policy mismatch
   - environment dependency
3. If harness bug, fix in lihaaf first.
4. If fixture-format mismatch, add adapter rule and document exception.
5. Treat every incompatibility as a GitHub issue with exact fixture evidence; non-blocking items still count as value expansion opportunities.
6. Keep a `compat/KNOWN_DIFFS.md` with rationale + status.

## 7) v0.1.0 target definition

v0.1.0 is ready when:

- compat mode is merged and documented (§3 implemented end-to-end, including §3.2.1–§3.2.3 and §3.4)
- the pilot set is stable: no hardening regressions, and the §4 coverage matrix has at least one pilot per row
- at least one external crate suite is fully reproduced **with a fork CI workflow that uploads the §3.3 envelope as a GitHub Actions artifact** (the gate is not satisfied by a local-run JSON pasted into a PR comment — the gate is the artifact attached to a green CI run, per §4.4)
- CI gate is in place per §5 with per-crate `N_<crate>` values committed to `compat/baseline.toml`
- release notes explicitly list any permanent incompatibilities and link to the `compat/KNOWN_DIFFS.md` rationale (§6)

## 8) Why this is the right path

This gives us real confidence with low ceremony:

- deterministic inputs,
- deterministic comparisons,
- deterministic reporting.

No hidden behavior. No “works on one project” claim. Just clear evidence before release.
