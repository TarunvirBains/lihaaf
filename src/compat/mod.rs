//! Compat mode driver. Activated by `cargo lihaaf --compat`.
//!
//! Implements `docs/compatibility-plan.md` §3 end-to-end. The driver
//! wires the supporting modules (`overlay`, `baseline`, `discovery`,
//! `fixture_convert`, `cleanup`, `report`, `rustup`, `gate`) into a
//! single end-to-end run: read upstream `Cargo.toml`, synthesize a
//! sibling `Cargo.lihaaf.toml` with an in-memory `[package.metadata.
//! lihaaf]` block, run the argv-only baseline (§3.4), discover
//! trybuild fixtures via syn AST walk (§3.2.1), convert each fixture
//! to the lihaaf-compatible directory tree, invoke `lihaaf::run`
//! in-process for the inner session, capture the active toolchain
//! (§3.4), and write the §3.3 envelope. The cleanup guard catches
//! panic / early-return paths and removes registered transient paths.
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
/// The 12-step sequence:
///
/// 1. Resolve the upstream `Cargo.toml` path (`--compat-manifest`
///    overrides `--compat-root/Cargo.toml`).
/// 2. Install the panic hook + construct the cleanup guard.
/// 3. Materialize the sibling overlay with a synthetic
///    `[package.metadata.lihaaf]` table; the builder closure reads
///    `[package].name` from the parsed manifest in a single pass.
/// 4. Track the overlay path for the cleanup classifier.
/// 5. Run the argv-only baseline `cargo test` invocation (the
///    recognized-fixture set is populated post-discovery; the
///    conservative parser is invoked once with the recognized list so
///    the v2 sidecar's `pass` / `fail` / `unknown_count` fields are
///    populated).
/// 6. Run the syn AST discovery walk over the upstream `tests/*.rs`
///    files (§3.2.1).
/// 7. Convert each recognized fixture to the lihaaf-compatible
///    directory tree under `<compat_root>/target/lihaaf-compat-
///    converted/{compile_pass,compile_fail}/`; the conversion tracks
///    every output path with the cleanup guard.
/// 8. Invoke `lihaaf::run` in-process with the overlay manifest path
///    so the inner session reads the synthetic metadata block.
/// 9. Capture the active toolchain via `rustup show active-toolchain`
///    (§3.4) with the rustc release-line fallback.
/// 10. Build the §3.3 envelope from every component above and write it
///     atomically via [`report::write_envelope`].
/// 11. Run the explicit cleanup finalize; the guard's Drop is the
///     safety net for panic / early-return paths.
///
/// This is `pub` so the crate's binary (`src/bin/cargo-lihaaf.rs`) and
/// out-of-tree integration tests can reach it through the re-export at
/// the crate root. It is `#[doc(hidden)]` at the re-export site —
/// adopters should drive compat mode through `cargo lihaaf --compat`,
/// not through the Rust API.
pub fn run(args: cli::CompatArgs) -> Result<(), Error> {
    let started = Instant::now();
    cleanup::install_panic_hook();

    let compat_root = args.compat_root.clone();
    let compat_report = args.compat_report.clone();
    let upstream_manifest = resolve_upstream_manifest(&args)?;

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
    // zero fixtures. Paths are repo-relative (resolved against the
    // overlay manifest dir, which is `<compat_root>`) with forward
    // slashes — §3.2.3's byte-determinism requirement bars absolute
    // platform-dependent paths from the envelope/manifest.
    let overlay_plan = overlay::materialize_overlay_with_synthetic_metadata_builder(
        &upstream_manifest,
        |upstream_name| {
            let name = upstream_name
                .map(str::to_string)
                .unwrap_or_else(|| basename_fallback(&compat_root));
            overlay::SyntheticMetadata {
                dylib_crate: name.clone(),
                extern_crates: vec![name],
                fixture_dirs: vec![
                    "./target/lihaaf-compat-converted/compile_pass".to_string(),
                    "./target/lihaaf-compat-converted/compile_fail".to_string(),
                ],
            }
        },
    )?;
    let crate_name = overlay_plan
        .upstream_crate_name
        .clone()
        .unwrap_or_else(|| basename_fallback(&compat_root));
    guard.track(overlay_plan.sibling_manifest.clone(), &compat_root);

    let discovery_output = discovery::discover(&compat_root, &args.compat_trybuild_macro)?;

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

    let converted =
        fixture_convert::convert_fixtures(&compat_root, &discovery_output.fixtures, &guard)?;

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

    let mut envelope_errors: Vec<report::EnvelopeError> = Vec::new();
    if let Some(detail) = toolchain_capture_error {
        envelope_errors.push(report::EnvelopeError {
            error_type: "toolchain_capture_failed".into(),
            fixture: None,
            file: String::new(),
            line: 0,
            detail,
        });
    }
    for unrecog in &discovery_output.unrecognized {
        envelope_errors.push(report::EnvelopeError {
            error_type: "discovery_unrecognized".into(),
            fixture: None,
            file: crate::util::relative_to(&unrecog.file, &compat_root),
            line: u32::try_from(unrecog.line).unwrap_or(u32::MAX),
            detail: unrecog.detail.clone(),
        });
    }
    if baseline_result.unknown_count > 0 {
        envelope_errors.push(report::EnvelopeError {
            error_type: "baseline_unknown".into(),
            fixture: None,
            file: String::new(),
            line: 0,
            detail: format!(
                "baseline parser produced {} unrecognized libtest verdict line(s)",
                baseline_result.unknown_count,
            ),
        });
    }
    if let Some(detail) = inner_session_error {
        envelope_errors.push(report::EnvelopeError {
            error_type: "lihaaf_session_failed".into(),
            fixture: None,
            file: String::new(),
            line: 0,
            detail,
        });
    }

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
            lihaaf: render_inner_command(&args, &overlay_plan.sibling_manifest),
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

    report::write_envelope(&mut envelope, &compat_report)?;
    let _ = started;
    Ok(())
}

/// Resolve the upstream `Cargo.toml` path the overlay reads.
///
/// `--compat-manifest` (when set) wins; otherwise the conventional
/// `<compat_root>/Cargo.toml` is used. The path must exist; a missing
/// manifest produces an [`Error::Io`] diagnostic at the overlay layer.
fn resolve_upstream_manifest(args: &cli::CompatArgs) -> Result<PathBuf, Error> {
    if let Some(m) = &args.compat_manifest {
        return Ok(m.clone());
    }
    Ok(args.compat_root.join("Cargo.toml"))
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
fn render_inner_command(args: &cli::CompatArgs, overlay_manifest: &Path) -> String {
    let mut parts: Vec<String> = vec![
        "cargo".into(),
        "lihaaf".into(),
        "--manifest-path".into(),
        overlay_manifest.to_string_lossy().into_owned(),
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
        let overlay_manifest = PathBuf::from("/tmp/lihaaf-build-inner-cli-test/Cargo.lihaaf.toml");
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
}
