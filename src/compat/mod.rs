//! Compat mode driver. Activated by `cargo lihaaf --compat`.
//!
//! Implements `docs/compatibility-plan.md` §3 end-to-end. The driver
//! wires the supporting modules (`overlay`, `baseline`, `discovery`,
//! `fixture_convert`, `cleanup`, `report`, `rustup`, `gate`) into a
//! single end-to-end run: read upstream `Cargo.toml`, synthesize a
//! staged overlay at `<compat_root>/target/lihaaf-overlay/Cargo.toml`
//! with an in-memory `[package.metadata.lihaaf]` block, run the
//! argv-only baseline (§3.4), discover trybuild fixtures via syn AST
//! walk (§3.2.1), convert each fixture to the lihaaf-compatible
//! directory tree, invoke `lihaaf::run` in-process for the inner
//! session, capture the active toolchain (§3.4), and write the §3.3
//! envelope. The cleanup guard catches panic / early-return paths and
//! removes registered transient paths.
//!
//! Adopters opt in via `cargo lihaaf --compat --compat-root <DIR>
//! --compat-report <PATH>`. The Rust API is not part of the v0.1
//! stability contract; treat `pub fn run` as private.

pub(crate) mod baseline;
pub(crate) mod cleanup;
pub(crate) mod cli;
pub(crate) mod discovery;
pub(crate) mod fixture_convert;
pub(crate) mod gate;
pub(crate) mod overlay;
pub(crate) mod report;
pub(crate) mod rustup;

use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::error::Error;

/// Top-level compat-mode entry. Called from `cargo-lihaaf.rs` when
/// `cli.compat` is true.
///
/// End-to-end §3 sequence:
///
/// 1. Install the panic hook + construct the cleanup guard.
/// 2. Resolve the upstream `Cargo.toml` path (`--compat-manifest`
///    overrides `--compat-root/Cargo.toml`).
/// 3. Materialize the staged overlay at
///    `<compat_root>/target/lihaaf-overlay/Cargo.toml` with a synthetic
///    `[package.metadata.lihaaf]` table; the builder closure reads
///    `[package].name` from the parsed manifest in a single pass.
///    Track the overlay path with the cleanup classifier.
/// 4. Run the syn AST discovery walk over the upstream `tests/*.rs`
///    files (§3.2.1). Discovery runs BEFORE baseline so the
///    recognized-fixture set is available to feed the conservative
///    libtest parser in step 5.
/// 5. Run the argv-only baseline `cargo test` invocation; the
///    conservative parser is invoked once with the recognized
///    fixture list so the v2 sidecar's `pass` / `fail` /
///    `unknown_count` fields are populated.
/// 6. Convert each recognized fixture to the lihaaf-compatible
///    directory tree under `<compat_root>/target/lihaaf-compat-
///    converted/{compile_pass,compile_fail}/`; the conversion tracks
///    every output path with the cleanup guard and clears the prior
///    tree on entry so stale snapshots cannot leak across runs.
/// 7. Invoke `lihaaf::run` in-process with the overlay manifest path
///    so the inner session reads the synthetic metadata block. The
///    inner Cli sets `inner_compat_normalize: true` so the §3.2.2
///    short-`$CARGO` rewrite fires for compat snapshots.
/// 8. Capture the active toolchain via `rustup show active-toolchain`
///    (§3.4) with the rustc release-line fallback. Failures
///    here are recorded in the envelope's `errors[]` rather than
///    short-circuiting the write — §3.3 requires the envelope to be
///    the single CI artifact.
/// 9. Run the explicit cleanup finalize; the guard's Drop is the
///    safety net for panic / early-return paths.
/// 10. Build the §3.3 envelope from every component above and write
///     it atomically via [`report::write_envelope`].
///
/// This is `pub` so the crate's binary (`src/bin/cargo-lihaaf.rs`) and
/// out-of-tree integration tests can reach it through the re-export at
/// the crate root. It is `#[doc(hidden)]` at the re-export site —
/// adopters should drive compat mode through `cargo lihaaf --compat`,
/// not through the Rust API.
pub fn run(args: cli::CompatArgs) -> Result<(), Error> {
    let started = Instant::now();
    cleanup::install_panic_hook();

    let compat_report = args.compat_report.clone();
    // Issue #53 — `resolve_dual_root` returns the dual-root vocabulary
    // (workspace_root / member_root) and the optional
    // `WorkspaceMemberContext`. Every downstream consumer reads the
    // explicit role from `DualRoot` instead of the legacy single
    // `compat_root` field. The collapse invariant (workspace_root ==
    // member_root when workspace_member_context is None) means
    // non-`--package` pilots see the same paths as pre-#53.
    //
    // Workspace-member entry via `--package` (issue #53): the adopter
    // invokes from the workspace root + names a member explicitly; the
    // resolver maps the package name to the member manifest path. The
    // overlay materializer takes a WorkspaceMemberContext that carries
    // the workspace root through, so it can:
    //   1. Skip the over-broad implicit-ancestor REJECT (Branch 2 of
    //      override_workspace_inheritance) — the adopter opted in.
    //   2. Carry the workspace root's [workspace.*] / [patch] / [profile]
    //      tables down into the overlay (apply_workspace_member_inheritance)
    //      so the dependency graph + patch resolution match baseline cargo.
    // See docs/compatibility-plan.md §3.2.3 (workspace-member entry).
    let dual_root = resolve_dual_root(&args)?;
    // `compat_root` (legacy name preserved for diff minimality) now
    // points at `workspace_root` per the §3.1.bis routing table.
    // Consumers that need member-anchored paths route through
    // `member_root` explicitly below. In the non-`--package` collapse
    // case (`workspace_member_context.is_none()`) both paths are
    // identical and behavior is unchanged from pre-#53.
    let compat_root = dual_root.workspace_root.clone();
    let member_root = dual_root.member_root.clone();
    let upstream_manifest = dual_root.member_manifest.clone();

    let guard = cleanup::CleanupGuard::new(args.inner_cli.keep_output);

    // The synthetic `[package.metadata.lihaaf]` block embedded in the
    // overlay needs the crate name BEFORE the overlay serializer runs.
    // We hand `materialize_overlay_with_synthetic_metadata_builder` a
    // closure that constructs the metadata once the overlay code has
    // parsed Cargo.toml and read `[package].name` — that way the file
    // is opened once and the synthetic block carries the right name on
    // the same write.
    //
    // `fixture_dirs` points at the two CHILD directories where the
    // §3.2.1 conversion writes converted fixtures, NOT the parent
    // `<target>/lihaaf-compat-converted/`. Reason: lihaaf's discovery
    // (`src/discovery.rs`) is non-recursive — it lists immediate
    // `is_file()` children only. If `fixture_dirs` pointed at the
    // parent, discovery would skip the `.rs` files that sit under
    // `compile_pass/` / `compile_fail/` and the inner session would see
    // zero fixtures.
    //
    // **Why ABSOLUTE paths here (approach A from the PR #34 redesign).**
    // The overlay now lives at `<compat_root>/target/lihaaf-overlay/Cargo.toml`,
    // two dirs deeper than the upstream `Cargo.toml`. lihaaf's
    // [`crate::discovery::collect`] resolves relative `fixture_dirs`
    // against the manifest's parent dir (the `[lib]` crate's root). A
    // repo-relative `./target/lihaaf-compat-converted/...` string would
    // therefore resolve to
    // `<compat_root>/target/lihaaf-overlay/target/lihaaf-compat-converted/...`
    // — a double-`target/` path that does not exist (fixture-conversion
    // writes to `<compat_root>/target/lihaaf-compat-converted/...`).
    //
    // Approach A absolutizes the two paths against `compat_root` here,
    // so the inner session sees the real on-disk locations regardless
    // of where the overlay manifest is staged. The cross-platform
    // §3.2.3 byte-determinism requirement (no platform-dependent
    // absolute paths in the §3.3 envelope) is preserved because these
    // paths flow into the OVERLAY MANIFEST, not the envelope —
    // `render_inner_command` shows the overlay manifest path itself but
    // never the synthesized `fixture_dirs`, and the envelope's
    // `crate_name` / `commit` / `commands.lihaaf` fields contain no
    // absolute paths. Approach B (carry upstream root through the
    // inner-session struct) would be semantically cleaner but
    // significantly more invasive — every call site of
    // `discovery::collect` would need a second path argument, and the
    // existing v0.1 surface only exposes one root. Approach A keeps
    // the §3.2.3 invariants while landing the GA fix in the smallest
    // possible change.
    // Issue #53 — fixture conversion + discovery anchor on `member_root`
    // (per §3.1.bis routing table). The overlay is staged at
    // `<member_root>/target/lihaaf-overlay/`; converted fixtures live
    // at `<member_root>/target/lihaaf-compat-converted/` so the inner
    // session's `fixture_dirs` reference points at the correct on-disk
    // location after the cargo walk-up from the overlay manifest. In
    // the non-`--package` collapse case (`member_root == workspace_root`),
    // both points are equivalent — non-`--package` pilots see no
    // behavioral drift.
    let converted_fixtures_root = member_root.join("target").join("lihaaf-compat-converted");
    let abs_compile_pass = crate::util::to_forward_slash(
        &converted_fixtures_root
            .join("compile_pass")
            .to_string_lossy(),
    );
    let abs_compile_fail = crate::util::to_forward_slash(
        &converted_fixtures_root
            .join("compile_fail")
            .to_string_lossy(),
    );
    let overlay_plan = overlay::materialize_overlay_with_workspace_member_context(
        &upstream_manifest,
        |upstream_name| {
            let name = upstream_name
                .map(str::to_string)
                .unwrap_or_else(|| basename_fallback(&member_root));
            overlay::compat_default_synthetic_metadata(
                &name,
                vec![abs_compile_pass.clone(), abs_compile_fail.clone()],
            )
        },
        dual_root.workspace_member_context.as_ref(),
    )?;
    let crate_name = overlay_plan
        .upstream_crate_name
        .clone()
        .unwrap_or_else(|| basename_fallback(&member_root));
    // `guard.track` relativizes the tracked path against the supplied
    // root for the §3.3 envelope's `generated_paths` field. We use
    // `compat_root` (= workspace_root) here so the envelope reports
    // paths relative to the dir the adopter passed. In the workspace-
    // member case, the overlay sibling at
    // `<member_root>/target/lihaaf-overlay/Cargo.toml` is rendered as
    // `<member-subdir>/target/lihaaf-overlay/Cargo.toml` relative to
    // the workspace root, which is the natural form for the envelope.
    guard.track(overlay_plan.sibling_manifest.clone(), &compat_root);

    // Issue #53 — discovery walks `<member_root>/tests/*.rs`. The
    // workspace root has no `tests/` of its own in the virtual-
    // workspace shape, so anchoring on `compat_root` (= workspace_root)
    // would find zero fixtures. Anchor on `member_root` per §3.1.bis
    // routing table.
    let discovery_output = discovery::discover(&member_root, &args.compat_trybuild_macro)?;

    // Baseline sidecar + baseline cargo cwd anchor on `workspace_root`
    // (per §3.1.bis routing table). Cargo writes `Cargo.lock` at the
    // workspace root; the baseline `cargo test` invocation must run
    // from the workspace root for cargo to discover the lockfile +
    // workspace state. `compat_root` aliases `workspace_root` here.
    let baseline_sidecar = compat_root
        .join("target")
        .join("lihaaf-compat-baseline.json");
    let recognized: Vec<baseline::FixtureId> = discovery_output
        .fixtures
        .iter()
        .map(|f| baseline::FixtureId {
            repo_relative_path: PathBuf::from(f.relative_path.clone()),
        })
        .collect();
    let baseline_result = baseline::run_baseline_with_recognized_fixtures(
        &args.compat_cargo_test_argv,
        &compat_root,
        &baseline_sidecar,
        &recognized,
    )?;
    guard.track(baseline_sidecar.clone(), &compat_root);

    // Fixture conversion writes to `<member_root>/target/lihaaf-compat-converted/`
    // per the routing table; the overlay's `[package.metadata.lihaaf].fixture_dirs`
    // synthetic block already points there.
    let converted =
        fixture_convert::convert_fixtures(&member_root, &discovery_output.fixtures, &guard)?;

    let lihaaf_started = Instant::now();
    let inner_cli = build_inner_cli(&args, &overlay_plan.sibling_manifest);
    let inner_result = crate::session::run(inner_cli);
    let lihaaf_dur_ms = u64::try_from(lihaaf_started.elapsed().as_millis()).unwrap_or(u64::MAX);
    let (lihaaf_pass, lihaaf_fail, lihaaf_exit_code, inner_session_error) = match inner_result {
        Ok(report) => {
            let pass = u32::try_from(
                report
                    .results
                    .iter()
                    .filter(|r| matches!(r.verdict, crate::verdict::Verdict::Ok))
                    .count(),
            )
            .unwrap_or(u32::MAX);
            let fail = u32::try_from(
                report
                    .results
                    .iter()
                    .filter(|r| !matches!(r.verdict, crate::verdict::Verdict::Ok))
                    .count(),
            )
            .unwrap_or(u32::MAX);
            let exit_code = if fail == 0 { 0 } else { 1 };
            (pass, fail, exit_code, None)
        }
        Err(e) => {
            let exit_code = inner_error_exit_code(&e);
            (0u32, 0u32, exit_code, Some(format!("{e}")))
        }
    };

    // Active-toolchain capture (§3.4). A failure here must NOT
    // short-circuit the envelope write — §3.3 requires the envelope to
    // be the single CI artifact, so an IO/spawn failure that hides every
    // other captured signal violates the contract. Capture into an
    // `Option<String>`; on Err, record a `toolchain_capture_failed`
    // entry in `envelope_errors` and proceed with an empty
    // `toolchain` field. The fallback release-line path inside
    // `capture_active_toolchain` already absorbs ordinary rustup-absent
    // / non-zero-exit cases — this Err branch only fires when BOTH
    // rustup AND rustc subprocesses fail, which is a degenerate
    // environment but not a reason to lose the rest of the run's signal.
    let (toolchain, toolchain_capture_error) = match rustup::capture_active_toolchain(&compat_root)
    {
        Ok(s) => (s, None),
        Err(e) => (String::new(), Some(format!("{e}"))),
    };

    let mismatch_examples = build_mismatch_examples(&baseline_result, &converted);
    let mismatch_count = u32::try_from(mismatch_examples.len()).unwrap_or(u32::MAX);

    let mut envelope_errors = assemble_diagnostic_errors(
        toolchain_capture_error,
        &discovery_output.unrecognized,
        inner_session_error,
        &compat_root,
    );

    let generated_paths_envelope = match guard.finalize() {
        Ok(entries) => entries
            .iter()
            .map(|e| report::generated_path_from_cleanup(e, &compat_root))
            .collect::<Vec<_>>(),
        Err(e) => {
            // Cleanup failure is recorded in the envelope's `errors`
            // list rather than aborting the run — the §3.3 contract is
            // that the envelope is the single CI artifact; surfacing
            // the cleanup failure in `errors` keeps the operator
            // notified without losing the rest of the captured signal.
            envelope_errors.push(report::EnvelopeError {
                error_type: "cleanup_failed".into(),
                fixture: None,
                file: String::new(),
                line: 0,
                detail: format!("{e}"),
            });
            Vec::new()
        }
    };

    let baseline_pass = baseline_result.pass.unwrap_or(0);
    let baseline_fail = baseline_result.fail.unwrap_or(0);
    let baseline_dur_ms = baseline_result.dur_ms;

    let mut envelope = report::CompatEnvelope {
        schema_version: 1,
        mode: "compat".into(),
        crate_name,
        commit: args.compat_commit.clone().unwrap_or_default(),
        commands: report::Commands {
            baseline: render_argv(&baseline_result.argv),
            lihaaf: render_inner_command(&args, &overlay_plan.sibling_manifest, &compat_root),
        },
        results: report::Results {
            baseline: report::BaselineCounts {
                pass: baseline_pass,
                fail: baseline_fail,
                unknown_count: baseline_result.unknown_count,
                exit_code: baseline_result.exit_code,
                dur_ms: baseline_dur_ms,
            },
            lihaaf: report::LihaafCounts {
                pass: lihaaf_pass,
                fail: lihaaf_fail,
                exit_code: lihaaf_exit_code,
                dur_ms: lihaaf_dur_ms,
                toolchain: toolchain.clone(),
            },
            mismatch_count,
        },
        mismatch_examples,
        errors: envelope_errors,
        excluded_fixtures: Vec::new(),
        generated_paths: generated_paths_envelope,
        overlay: report::OverlayMetadata {
            generated: true,
            dropped_comments: overlay_plan.dropped_comments,
            upstream_already_has_dylib: overlay_plan.upstream_already_has_dylib,
        },
        toolchain,
    };

    // Normalize absolute `compat_root` prefixes in `errors[].detail`
    // before writing the envelope. Infrastructure errors (e.g.
    // `DylibBuildFailed`) embed the cargo invocation, which contains
    // runner-specific absolute paths. Without this step, two CI runners
    // at different checkout roots produce non-identical envelope bytes
    // on failure, violating the §3.3 determinism rule. The `Display`
    // impl is intentionally left unchanged — absolute paths remain
    // useful for local terminal output; this is the single
    // normalization boundary.
    report::normalize_error_detail_paths(&mut envelope, &compat_root);

    report::write_envelope(&mut envelope, &compat_report)?;
    let _ = started;
    Ok(())
}

/// Resolve the dual-root vocabulary from `CompatArgs` (issue #53 — see
/// plan §3.1.bis routing table + §4.2 driver wire-up).
///
/// Decision tree:
///
/// 1. **`--compat-manifest` wins.** When the adopter passes
///    `--compat-manifest`, both roots collapse to the manifest's
///    parent dir. This is the v0.1 escape hatch — bypasses workspace
///    resolution entirely. (`--package` and `--compat-manifest` are
///    mutually exclusive per `Cli::validate_mode_consistency`.)
///
/// 2. **No `--package`: single-root collapse.** When `--package` is
///    absent, the four paths collapse: `workspace_root == member_root`
///    and `workspace_root_manifest == member_manifest`. The overlay
///    materializer's existing single-root logic runs unchanged. Round-1
///    pilots (cxx, serde_json, anyhow, thiserror) all hit this branch.
///
/// 3. **`--package` supplied: dual-root resolver.** The default
///    manifest at `<compat_root>/Cargo.toml` MUST be a virtual
///    workspace root (declares `[workspace]` without `[package]` per
///    v0.1.0 scope; see §1 / §11.11). The resolver
///    [`overlay::resolve_workspace_member_manifest`] verifies the
///    shape and walks `[workspace.members]` to find the member whose
///    `[package].name == args.compat_package`. The resolver returns
///    `(member_manifest, workspace_root_value)`; this function wraps
///    them into a `DualRoot` with `workspace_member_context: Some(_)`.
///
/// # Returns
///
/// `Ok(DualRoot)` on a successful resolution (single-root or dual-
/// root). `Err(Error::Cli)` on resolver failures (no-match,
/// multiple-match, package+workspace shape, etc.); `Err(Error::Io)`
/// or `Err(Error::TomlParse)` propagating from the resolver.
fn resolve_dual_root(args: &cli::CompatArgs) -> Result<overlay::DualRoot, Error> {
    let workspace_root = args.compat_root.clone();

    // 1. `--compat-manifest` overrides everything else. Collapse both
    //    roots to the manifest's parent dir.
    if let Some(m) = &args.compat_manifest {
        let member_root = m
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));
        return Ok(overlay::DualRoot {
            workspace_root: member_root.clone(),
            workspace_root_manifest: m.clone(),
            member_root,
            member_manifest: m.clone(),
            workspace_member_context: None,
        });
    }

    // 2. Default manifest. `<compat_root>/Cargo.toml`.
    let default_manifest = workspace_root.join("Cargo.toml");

    // 3. `--package` absent → single-root collapse. The overlay
    //    materializer's existing workspace-root-rejection branch at
    //    `is_workspace_root_manifest` REJECT (§4.4 case 5 augmented)
    //    fires here if `compat_root` is a workspace root — the
    //    augmented diagnostic suggests `--package`.
    let Some(pkg) = &args.compat_package else {
        return Ok(overlay::DualRoot {
            workspace_root: workspace_root.clone(),
            workspace_root_manifest: default_manifest.clone(),
            member_root: workspace_root,
            member_manifest: default_manifest,
            workspace_member_context: None,
        });
    };

    // 4. `--package` supplied → dual-root resolver. The resolver
    //    verifies the workspace-root shape (rejects package+workspace
    //    + non-workspace-root manifests via §4.3 step 2 / step 2.5)
    //    and returns `(member_manifest, workspace_root_value)`.
    let (member_manifest, workspace_root_value) =
        overlay::resolve_workspace_member_manifest(&default_manifest, pkg)?;
    let member_root = member_manifest
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    Ok(overlay::DualRoot {
        workspace_root,
        workspace_root_manifest: default_manifest.clone(),
        member_root,
        member_manifest,
        workspace_member_context: Some(overlay::WorkspaceMemberContext {
            workspace_root_manifest: default_manifest,
            workspace_root_value,
        }),
    })
}

fn basename_fallback(compat_root: &Path) -> String {
    compat_root
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "unknown".to_string())
}

/// Build the [`crate::cli::Cli`] passed into the in-process
/// `lihaaf::run` invocation. The manifest path is overridden to the
/// sibling overlay so `config::load` reads the synthetic
/// `[package.metadata.lihaaf]` block; pass-through flags
/// (`--bless`, `--no-cache`, `--jobs`, `--verbose`, `--use-symlink`,
/// `--keep-output`, `--quiet`) forward verbatim.
///
/// `--filter` and `--manifest-path` are NOT carried from the outer Cli
/// (compat mode rejects them at parse time); `--compat-filter` is
/// translated into `--filter` here so the inner session sees the
/// adopter's fixture-substring choice.
///
/// `inner_compat_normalize` is set to `true` so the inner session's
/// [`crate::normalize::NormalizationContext`] picks up the §3.2.2
/// short-form `$CARGO/<crate>-<ver>/...` rewrite. Without this, compat
/// snapshots expecting the trybuild short form would mismatch as
/// non-compat `$CARGO/registry/...` strings. The field is hidden from
/// the public CLI surface (see `Cli::inner_compat_normalize`).
fn build_inner_cli(args: &cli::CompatArgs, overlay_manifest: &Path) -> crate::cli::Cli {
    crate::cli::Cli {
        bless: args.inner_cli.bless,
        compat: false,
        compat_cargo_test_argv: None,
        compat_commit: None,
        compat_filter: Vec::new(),
        compat_manifest: None,
        // `--package` is a compat-mode-only outer flag (per #53); the
        // inner session sees a v0.1 surface and the field is always
        // `None` here. The compat driver has already used the field to
        // resolve the member manifest; the inner session does not
        // re-consume it.
        compat_package: None,
        compat_report: None,
        compat_root: None,
        compat_trybuild_macro: Vec::new(),
        filter: args.compat_filter.clone(),
        jobs: args.inner_cli.jobs,
        suite: args.inner_cli.suite.clone(),
        no_cache: args.inner_cli.no_cache,
        manifest_path: Some(overlay_manifest.to_path_buf()),
        list: args.inner_cli.list,
        quiet: args.inner_cli.quiet,
        verbose: args.inner_cli.verbose,
        use_symlink: args.inner_cli.use_symlink,
        keep_output: args.inner_cli.keep_output,
        inner_compat_normalize: true,
    }
}

/// Render the inner `cargo lihaaf` invocation as a copy/paste-friendly
/// shell-style string for the §3.3 envelope's `commands.lihaaf` field.
/// Per §3.1 this is for human inspection only — the spec forbids
/// constructing a shell command line in compat mode itself.
///
/// `overlay_manifest` is always under `compat_root` (it is staged at
/// `<compat_root>/target/lihaaf-overlay/Cargo.toml`); the manifest
/// path is relativized against `compat_root` before serialization so
/// the envelope's `commands.lihaaf` field never contains runner-specific
/// absolute paths. This satisfies the §3.3 determinism rule
/// (`docs/compatibility-plan.md §3.3`): different CI runners produce
/// byte-identical envelopes from the same crate checkout.
///
/// # Panics
///
/// Panics if `overlay_manifest` cannot be made relative to `compat_root`.
/// This is a driver invariant: the overlay is always staged under
/// `compat_root`; if it is not, the compat driver has a bug.
fn render_inner_command(
    args: &cli::CompatArgs,
    overlay_manifest: &Path,
    compat_root: &Path,
) -> String {
    // Strip the compat_root prefix and convert to forward-slash form so the
    // envelope field is the same on every runner regardless of its absolute
    // checkout path (e.g. `/home/runner/work/...` vs `/home/tarunvir/...`).
    let rel_manifest = overlay_manifest
        .strip_prefix(compat_root)
        .unwrap_or_else(|_| {
            panic!(
                "render_inner_command: overlay_manifest `{}` is not under compat_root `{}`; \
                 this is a compat-driver bug — the overlay must always be staged under \
                 compat_root",
                overlay_manifest.display(),
                compat_root.display()
            )
        });
    let rel_manifest_str = crate::util::to_forward_slash(&rel_manifest.to_string_lossy());

    let mut parts: Vec<String> = vec![
        "cargo".into(),
        "lihaaf".into(),
        "--manifest-path".into(),
        rel_manifest_str,
    ];
    if args.inner_cli.bless {
        parts.push("--bless".into());
    }
    if args.inner_cli.no_cache {
        parts.push("--no-cache".into());
    }
    if args.inner_cli.list {
        parts.push("--list".into());
    }
    if args.inner_cli.quiet {
        parts.push("--quiet".into());
    }
    if args.inner_cli.verbose {
        parts.push("--verbose".into());
    }
    if args.inner_cli.use_symlink {
        parts.push("--use-symlink".into());
    }
    if args.inner_cli.keep_output {
        parts.push("--keep-output".into());
    }
    if let Some(j) = args.inner_cli.jobs {
        parts.push("--jobs".into());
        parts.push(j.to_string());
    }
    for s in &args.inner_cli.suite {
        parts.push("--suite".into());
        parts.push(s.clone());
    }
    for f in &args.compat_filter {
        parts.push("--filter".into());
        parts.push(f.clone());
    }
    parts.join(" ")
}

/// Render an argv vector as a copy/paste-friendly shell-style string
/// for the §3.3 envelope. Joins on single spaces; no quoting (per
/// §3.1's "no shell command line" rule, the string is for human
/// inspection only). Per the spec, callers wanting to re-execute the
/// baseline must consume the structured argv vector recorded in the
/// baseline sidecar, not parse this string.
fn render_argv(argv: &[String]) -> String {
    argv.join(" ")
}

/// Build the §3.3 envelope's `mismatch_examples` list from the
/// baseline-side mismatch entries.
///
/// For v0.1.0-beta.4 the comparison surface is conservative: each
/// baseline-side `BaselineMismatch` becomes one `MismatchExample` with
/// `mismatch_type = "baseline_only_fail"` / `"baseline_only_pass"`
/// derived from the baseline verdict alone. The lihaaf-side outcome is
/// not joined yet — that is the v0.2 "compare per-fixture verdicts"
/// surface. The §5 gate reads only `results.mismatch_count` for v0.1.
fn build_mismatch_examples(
    baseline_result: &baseline::BaselineResult,
    _converted: &[fixture_convert::ConvertedFixture],
) -> Vec<report::MismatchExample> {
    baseline_result
        .mismatch_entries
        .iter()
        .map(|m| {
            let mismatch_type = match m.baseline_verdict {
                baseline::BaselineVerdict::Pass => "baseline_only_pass",
                baseline::BaselineVerdict::Fail => "baseline_only_fail",
            };
            report::MismatchExample {
                fixture: m.fixture.clone(),
                mismatch_type: mismatch_type.into(),
                notes: String::new(),
            }
        })
        .collect()
}

/// Map an inner [`crate::session::run`] error to the §3.3 envelope's
/// `results.lihaaf.exit_code` integer.
///
/// `Error::Session(outcome)` maps through [`crate::error::Outcome::exit_code`]
/// (the same mapping the `cargo-lihaaf` binary uses for non-compat
/// runs). Any other variant maps to `CONFIG_INVALID` (`64`); the
/// compat driver records the full `Display` body in the envelope's
/// `errors[]` for diagnosis.
fn inner_error_exit_code(e: &Error) -> i32 {
    match e {
        Error::Session(outcome) => i32::from(outcome.exit_code() as u8),
        _ => i32::from(crate::exit::ExitCode::ConfigInvalid as u8),
    }
}

/// Assemble the “diagnostic side-channel” entries for the §3.3 envelope's
/// `errors[]` field — toolchain-capture failure, discovery-unrecognized
/// items, and the inner-session error (if any).
///
/// Deliberately does NOT take `baseline_result.unknown_count`. The
/// baseline parser's `unknown_count` is a diagnostic counter that lives
/// in `results.baseline.unknown_count` for the operator to inspect; it
/// is NOT an envelope-level error condition. Pushing a `baseline_unknown`
/// entry into `errors[]` from a positive `unknown_count` is what
/// recreates the §5 gate coupling that was explicitly removed in
/// `89fec16` (round-3): every trybuild adopter run produces
/// `unknown_count >= 1` (the libtest wrapper line) so making the gate
/// key off `errors.is_empty()` would always block. Encoding the
/// invariant in the function signature ensures that future edits to
/// the envelope-construction path cannot reintroduce the coupling.
///
/// Cleanup-finalize errors are appended by the caller — `finalize`
/// consumes the [`cleanup::CleanupGuard`] and its result is bound up
/// with the [`report::generated_paths`] vector, which is built in the
/// same expression.
fn assemble_diagnostic_errors(
    toolchain_capture_error: Option<String>,
    discovery_unrecognized: &[discovery::DiscoveryUnrecognized],
    inner_session_error: Option<String>,
    compat_root: &Path,
) -> Vec<report::EnvelopeError> {
    let mut errors: Vec<report::EnvelopeError> = Vec::new();
    if let Some(detail) = toolchain_capture_error {
        errors.push(report::EnvelopeError {
            error_type: "toolchain_capture_failed".into(),
            fixture: None,
            file: String::new(),
            line: 0,
            detail,
        });
    }
    for unrecog in discovery_unrecognized {
        errors.push(report::EnvelopeError {
            error_type: "discovery_unrecognized".into(),
            fixture: None,
            file: crate::util::relative_to(&unrecog.file, compat_root)
                .unwrap_or_else(|err| err.non_absolute_path()),
            line: u32::try_from(unrecog.line).unwrap_or(u32::MAX),
            detail: unrecog.detail.clone(),
        });
    }
    if let Some(detail) = inner_session_error {
        errors.push(report::EnvelopeError {
            error_type: "lihaaf_session_failed".into(),
            fixture: None,
            file: String::new(),
            line: 0,
            detail,
        });
    }
    errors
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::Cli;

    /// Build a minimal `CompatArgs` with the inner Cli `compat` flag set
    /// so the contract precondition for `build_inner_cli` is satisfied.
    /// Every field on `CompatArgs` and the inner `Cli` is set to the
    /// posture an empty `cargo lihaaf --compat --compat-root /tmp --compat-report /tmp/r.json`
    /// invocation would produce.
    fn neutral_compat_args() -> cli::CompatArgs {
        cli::CompatArgs {
            compat_root: PathBuf::from("/tmp/lihaaf-build-inner-cli-test-root"),
            compat_report: PathBuf::from("/tmp/lihaaf-build-inner-cli-test-report.json"),
            compat_cargo_test_argv: vec!["cargo".to_string(), "test".to_string()],
            compat_manifest: None,
            compat_package: None,
            compat_commit: None,
            compat_filter: Vec::new(),
            compat_trybuild_macro: Vec::new(),
            inner_cli: Cli {
                bless: false,
                compat: true,
                compat_cargo_test_argv: None,
                compat_commit: None,
                compat_filter: Vec::new(),
                compat_manifest: None,
                compat_package: None,
                compat_report: Some(PathBuf::from(
                    "/tmp/lihaaf-build-inner-cli-test-report.json",
                )),
                compat_root: Some(PathBuf::from("/tmp/lihaaf-build-inner-cli-test-root")),
                compat_trybuild_macro: Vec::new(),
                filter: Vec::new(),
                jobs: None,
                suite: Vec::new(),
                no_cache: false,
                manifest_path: None,
                list: false,
                quiet: false,
                verbose: false,
                use_symlink: false,
                keep_output: false,
                inner_compat_normalize: false,
            },
        }
    }

    #[test]
    fn build_inner_cli_sets_inner_compat_normalize_true() {
        // The compat driver's inner session needs the §3.2.2 short-$CARGO
        // rewrite. `build_inner_cli` is the single seam responsible for
        // setting the hidden `inner_compat_normalize` flag on the inner
        // Cli; `session::run` then reads that flag and constructs the
        // `NormalizationContext` with `compat_short_cargo = true`. If
        // this assertion regresses, compat snapshots expecting
        // `$CARGO/<crate>-<ver>/...` will mismatch as
        // `$CARGO/registry/...` strings without any other diagnostic.
        let args = neutral_compat_args();
        let overlay_manifest =
            PathBuf::from("/tmp/lihaaf-build-inner-cli-test/target/lihaaf-overlay/Cargo.toml");
        let inner = build_inner_cli(&args, &overlay_manifest);
        assert!(
            inner.inner_compat_normalize,
            "compat driver must set inner_compat_normalize=true so the inner session's \
             NormalizationContext.compat_short_cargo is true",
        );
        // And the outer compat flag must be cleared on the inner Cli —
        // the inner session is a regular v0.1 run that happens to have
        // the compat normalizer flag set.
        assert!(
            !inner.compat,
            "inner Cli must have `compat: false` so the inner session does NOT recurse into \
             the compat driver",
        );
    }

    #[test]
    fn normalization_context_reads_inner_compat_normalize() {
        // The session-construction path is
        // `NormalizationContext::new(...).with_compat_short_cargo(cli.inner_compat_normalize)`.
        // This test asserts the builder produces the expected flag
        // value for both inputs so the plumbing seam in `session::run`
        // cannot regress silently. A renamed flag, a typo'd `true`/`false`,
        // or an inverted builder argument bites here.
        //
        // Issue #45 / v0.1.0-beta.10 note: the three adopter-defined
        // normalizer override keys (`extra_substitutions`, `strip_lines`,
        // `strip_line_prefixes`) are intentionally NOT chained here. The
        // compat driver synthesizes `[package.metadata.lihaaf]` entirely
        // and does not surface adopter TOML into the inner session, so
        // there is no Suite-level value to forward. Compat-mode adopter
        // extras are unsupported in v0.1.0-beta.10 (OQ-4 deferral) —
        // see docs/spec/lihaaf-v0.1.md §6.6.
        let ctx_compat =
            crate::normalize::NormalizationContext::new(PathBuf::from("/p"), PathBuf::from("/r"))
                .with_compat_short_cargo(true);
        assert!(
            ctx_compat.compat_short_cargo,
            "with_compat_short_cargo(true) must set the flag",
        );
        let ctx_non_compat =
            crate::normalize::NormalizationContext::new(PathBuf::from("/p"), PathBuf::from("/r"))
                .with_compat_short_cargo(false);
        assert!(
            !ctx_non_compat.compat_short_cargo,
            "with_compat_short_cargo(false) must clear the flag (mirrors the default)",
        );
    }

    /// **Diagnostic-errors do NOT couple to `baseline.unknown_count`.**
    /// Round-3 commit `89fec16` removed the direct `unknown_count != 0`
    /// rule from the §5 gate, but a positive `unknown_count` was still
    /// pushing a `baseline_unknown` entry into `envelope.errors[]`. Every
    /// trybuild adopter run produces `unknown_count >= 1` (the libtest
    /// wrapper line), so any gate that keys off `errors.is_empty()`
    /// would always block. This test pins the invariant in the function
    /// signature: `assemble_diagnostic_errors` does not take a
    /// `unknown_count` input, so the only way to regress this is a
    /// deliberate refactor that re-introduces the parameter. The
    /// `unknown_count` value still flows into the envelope at
    /// `results.baseline.unknown_count` for operator visibility.
    #[test]
    fn assemble_diagnostic_errors_ignores_baseline_unknown_count() {
        let errors = assemble_diagnostic_errors(
            None,                          // toolchain capture clean
            &[],                           // no discovery_unrecognized
            None,                          // inner session clean
            Path::new("/tmp/compat-root"), // path used only for forward-slash mapping
        );
        assert!(
            errors.is_empty(),
            "with no toolchain / discovery / session errors, errors[] must be \
             empty regardless of how many baseline `unknown_count` lines the \
             libtest parser saw; got {errors:?}",
        );
        // Belt-and-braces: no entry of type `baseline_unknown` may ever
        // appear in the assembled vec. A future refactor that wires
        // unknown_count back into the assembler would fail this even if
        // the call above somehow stayed empty.
        assert!(
            !errors.iter().any(|e| e.error_type == "baseline_unknown"),
            "no `baseline_unknown` entry may appear in errors[]; the field \
             lives in results.baseline.unknown_count as a diagnostic counter"
        );
    }

    /// **`render_inner_command` produces a repo-relative `--manifest-path`.**
    ///
    /// This pins the §3.3 determinism rule: the envelope's `commands.lihaaf`
    /// field must not contain runner-specific absolute paths.  Two calls with
    /// the same repo layout but different `compat_root` absolute prefixes must
    /// produce byte-identical `--manifest-path` values.
    ///
    /// Without the relativization fix, `overlay_manifest.to_string_lossy()` is
    /// used directly — producing `/home/runner/work/.../target/lihaaf-overlay/Cargo.toml`
    /// on one runner and `/home/tarunvir/.../target/lihaaf-overlay/Cargo.toml` on
    /// another. This test would fail before the fix.
    #[test]
    fn render_inner_command_manifest_path_is_repo_relative() {
        // Two hypothetical compat_root values with the same repo layout but
        // different absolute prefixes — simulates two different CI runners
        // that checked out at different absolute paths.
        let compat_root_a = PathBuf::from("/home/runner/work/my-crate");
        let compat_root_b = PathBuf::from("/home/tarunvir/projects/my-crate");

        // The overlay is staged at `<compat_root>/target/lihaaf-overlay/Cargo.toml`
        // on both runners.
        let overlay_a = compat_root_a
            .join("target")
            .join("lihaaf-overlay")
            .join("Cargo.toml");
        let overlay_b = compat_root_b
            .join("target")
            .join("lihaaf-overlay")
            .join("Cargo.toml");

        let args = neutral_compat_args();

        let cmd_a = render_inner_command(&args, &overlay_a, &compat_root_a);
        let cmd_b = render_inner_command(&args, &overlay_b, &compat_root_b);

        // The two strings must be identical (byte-for-byte) — no runner-specific
        // prefix leaks into the envelope.
        assert_eq!(
            cmd_a, cmd_b,
            "commands.lihaaf must be runner-independent; \
             runner A: `{cmd_a}`\nrunner B: `{cmd_b}`"
        );

        // The `--manifest-path` value must not be an absolute path.
        assert!(
            !cmd_a.contains("/home/"),
            "commands.lihaaf must not contain an absolute path; got `{cmd_a}`"
        );

        // The `--manifest-path` value must be the canonical repo-relative form.
        assert!(
            cmd_a.contains("target/lihaaf-overlay/Cargo.toml"),
            "commands.lihaaf must contain the repo-relative manifest path; got `{cmd_a}`"
        );
    }

    /// **`assemble_diagnostic_errors` preserves the historical ordering**
    /// across the three live entry kinds: `toolchain_capture_failed`,
    /// `discovery_unrecognized`, `lihaaf_session_failed`. Cleanup-failed
    /// is appended by the caller after `guard.finalize()` and lives
    /// outside this helper. The §3.3 schema sorts `errors[]` by
    /// `(file, line, error_type)` at the writer, so this ordering only
    /// matters for in-memory traversal — but pinning it makes the
    /// helper's contract obvious to future readers.
    #[test]
    fn assemble_diagnostic_errors_orders_kinds_by_source_seam() {
        let unrec = discovery::DiscoveryUnrecognized {
            file: PathBuf::from("/tmp/compat-root/tests/ui.rs"),
            line: 7,
            detail: "non-literal arg".into(),
        };
        let errors = assemble_diagnostic_errors(
            Some("rustup spawn failed".into()),
            std::slice::from_ref(&unrec),
            Some("inner panicked".into()),
            Path::new("/tmp/compat-root"),
        );
        let types: Vec<&str> = errors.iter().map(|e| e.error_type.as_str()).collect();
        assert_eq!(
            types,
            vec![
                "toolchain_capture_failed",
                "discovery_unrecognized",
                "lihaaf_session_failed",
            ],
            "in-memory order is toolchain → discovery → session; got {types:?}"
        );
    }
}
