//! Phase 10 of compat mode — trybuild → lihaaf fixture conversion.
//!
//! Walks the [`crate::compat::discovery::DiscoveryOutput`] list produced
//! in Phase 6 and copies each fixture (`.rs` file plus matching
//! `.stderr` snapshot if present) into the converted-fixtures tree under
//! `<compat_root>/target/lihaaf-compat-converted/{compile_pass,
//! compile_fail}/` per Q3 of the Phase 10 design.
//!
//! ## Why `target/`?
//!
//! `<compat_root>/target/` is the cargo-owned directory cargo itself
//! treats as transient. Placing the converted-fixtures tree there means:
//!
//! 1. The dirty-worktree rule (§3.2.3) never fires — cargo's own
//!    `.gitignore` semantics cover the directory.
//! 2. The Phase 5 cleanup classifier ([`crate::compat::cleanup::classify`])
//!    reports the path as `Ignored` without invoking git, so the §3.3
//!    envelope's `generated_paths` entry is uniform across fork-CI
//!    environments that may or may not have git available.
//! 3. The directory is removed by the usual `cargo clean` workflow,
//!    matching adopter expectations for build-artifact cleanup.
//!
//! ## What gets copied
//!
//! For each `DiscoveredFixture`:
//!
//! - `<compat_root>/target/lihaaf-compat-converted/<subdir>/<basename>.rs` —
//!   the fixture's `.rs` source. `<subdir>` is `compile_pass` or
//!   `compile_fail` depending on the fixture's
//!   [`crate::compat::discovery::FixtureKind`].
//! - `<compat_root>/target/lihaaf-compat-converted/<subdir>/<basename>.stderr` —
//!   only present if the source fixture had a matching `.stderr`
//!   sibling. Snapshots are needed for `compile_fail` fixtures; the
//!   conversion still copies a `.stderr` for `compile_pass` if the
//!   adopter happened to ship one.
//!
//! ## Determinism
//!
//! The output list is sorted by `dest_path` ASCII byte order before
//! being returned. Two runs from clean state produce the same vector.
//!
//! ## Errors
//!
//! Returns `Error::Io` on the first filesystem failure (mkdir / copy).
//! Partial state may remain on disk — the Phase 5 cleanup guard's Drop
//! safety net removes whatever was tracked before the failure, so the
//! adopter's `target/lihaaf-compat-converted/` tree stays consistent
//! with the on-exit envelope.

use std::path::{Path, PathBuf};

use crate::compat::cleanup::CleanupGuard;
use crate::compat::discovery::{DiscoveredFixture, FixtureKind};
use crate::error::Error;

/// One converted fixture entry. The pair `(src_path, dest_path)` is
/// preserved so the §3.3 envelope's `fixture` field can render the
/// adopter-facing source path while lihaaf's per-fixture worker reads
/// from `dest_path`.
#[derive(Debug, Clone)]
// Fields are read by the Phase 10 driver and by integration tests via
// the `#[doc(hidden)]` re-export at the crate root; the lint sees only
// the in-tree `compat::run` call path which currently consumes
// `dest_path` directly. Kept on the struct so adopters' downstream
// envelope-consumers and the v0.2 "join per-fixture verdicts" surface
// have the data they need without a re-walk.
#[allow(dead_code)]
pub struct ConvertedFixture {
    /// Absolute path to the original trybuild fixture (the
    /// [`DiscoveredFixture::fixture_path`] verbatim).
    pub src_path: PathBuf,
    /// Absolute path to the copied fixture under
    /// `<compat_root>/target/lihaaf-compat-converted/<subdir>/`.
    pub dest_path: PathBuf,
    /// `Some(absolute path)` when a matching `.stderr` snapshot was
    /// also copied; `None` otherwise. Compile_fail fixtures typically
    /// have one; compile_pass typically do not.
    pub dest_stderr: Option<PathBuf>,
    /// Pass / compile_fail classification, preserved from the discovery
    /// output so the v0.1 [`crate::discovery`] walker can re-classify
    /// the copied fixture against its parent directory's name
    /// (`compile_pass/` vs `compile_fail/`) without re-running the syn
    /// visitor.
    pub kind: FixtureKind,
}

/// Convert every fixture in `fixtures` into the lihaaf-compatible
/// `<compat_root>/target/lihaaf-compat-converted/{compile_pass,
/// compile_fail}/` tree.
///
/// `cleanup` tracks every produced path so the Phase 5 dirty-worktree
/// guard cleans up the tree on exit (the path lives under `target/` so
/// classification is `Ignored`; the tracking still produces the
/// `generated_paths` envelope entry the operator may inspect).
///
/// The root directory `<compat_root>/target/lihaaf-compat-converted/`
/// is created if it does not exist; pre-existing files inside it are
/// OVERWRITTEN on conflict (the fixture source has shifted; the
/// converted copy must follow).
///
/// **Errors.** Returns `Error::Io` on the first directory-creation or
/// file-copy failure; the partial tree is left behind for the Phase 5
/// cleanup guard's Drop safety net to remove on exit.
pub fn convert_fixtures(
    compat_root: &Path,
    fixtures: &[DiscoveredFixture],
    cleanup: &CleanupGuard,
) -> Result<Vec<ConvertedFixture>, Error> {
    let converted_root = compat_root.join("target").join("lihaaf-compat-converted");
    let compile_pass_dir = converted_root.join("compile_pass");
    let compile_fail_dir = converted_root.join("compile_fail");

    std::fs::create_dir_all(&compile_pass_dir).map_err(|e| {
        Error::io(
            e,
            "creating compat-converted compile_pass directory",
            Some(compile_pass_dir.clone()),
        )
    })?;
    cleanup.track(compile_pass_dir.clone(), compat_root);
    std::fs::create_dir_all(&compile_fail_dir).map_err(|e| {
        Error::io(
            e,
            "creating compat-converted compile_fail directory",
            Some(compile_fail_dir.clone()),
        )
    })?;
    cleanup.track(compile_fail_dir.clone(), compat_root);
    cleanup.track(converted_root.clone(), compat_root);

    let mut out: Vec<ConvertedFixture> = Vec::with_capacity(fixtures.len());
    for fixture in fixtures {
        let subdir = match fixture.kind {
            FixtureKind::Pass => &compile_pass_dir,
            FixtureKind::CompileFail => &compile_fail_dir,
        };
        let basename = fixture.fixture_path.file_name().ok_or_else(|| {
            Error::io(
                std::io::Error::other("fixture path has no file name"),
                "deriving fixture basename for compat conversion",
                Some(fixture.fixture_path.clone()),
            )
        })?;

        let dest_path = subdir.join(basename);
        std::fs::copy(&fixture.fixture_path, &dest_path).map_err(|e| {
            Error::io(
                e,
                "copying trybuild fixture to compat-converted tree",
                Some(fixture.fixture_path.clone()),
            )
        })?;
        cleanup.track(dest_path.clone(), compat_root);

        let mut dest_stderr: Option<PathBuf> = None;
        let src_stderr = fixture.fixture_path.with_extension("stderr");
        if src_stderr.exists() {
            let dest_stderr_path = dest_path.with_extension("stderr");
            std::fs::copy(&src_stderr, &dest_stderr_path).map_err(|e| {
                Error::io(
                    e,
                    "copying trybuild .stderr snapshot to compat-converted tree",
                    Some(src_stderr.clone()),
                )
            })?;
            cleanup.track(dest_stderr_path.clone(), compat_root);
            dest_stderr = Some(dest_stderr_path);
        }

        out.push(ConvertedFixture {
            src_path: fixture.fixture_path.clone(),
            dest_path,
            dest_stderr,
            kind: fixture.kind,
        });
    }

    out.sort_by(|a, b| a.dest_path.cmp(&b.dest_path));
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compat::discovery::CallSite;

    fn dummy_call_site(file: PathBuf) -> CallSite {
        CallSite {
            file,
            line: 1,
            enclosing_test_fn: None,
        }
    }

    #[test]
    fn convert_copies_pass_fixture_into_compile_pass_subdir() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let compat_root = tmp.path();
        let src_dir = compat_root.join("tests").join("ui");
        std::fs::create_dir_all(&src_dir).expect("create src dir");
        let src = src_dir.join("good.rs");
        std::fs::write(&src, b"fn main() {}").expect("write fixture");

        let cleanup = CleanupGuard::new(false);
        let fixtures = vec![DiscoveredFixture {
            fixture_path: src.clone(),
            relative_path: "tests/ui/good.rs".into(),
            kind: FixtureKind::Pass,
            call_site: dummy_call_site(src_dir.join("trybuild.rs")),
        }];

        let out = convert_fixtures(compat_root, &fixtures, &cleanup).expect("convert");
        assert_eq!(out.len(), 1);
        let dest = compat_root
            .join("target")
            .join("lihaaf-compat-converted")
            .join("compile_pass")
            .join("good.rs");
        assert!(
            dest.exists(),
            "compile_pass fixture must be copied to {dest:?}"
        );
        assert_eq!(out[0].dest_path, dest);
        assert_eq!(out[0].dest_stderr, None);
    }

    #[test]
    fn convert_copies_compile_fail_with_stderr_snapshot() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let compat_root = tmp.path();
        let src_dir = compat_root.join("tests").join("ui");
        std::fs::create_dir_all(&src_dir).expect("create src dir");
        let src_rs = src_dir.join("bad.rs");
        let src_stderr = src_dir.join("bad.stderr");
        std::fs::write(&src_rs, b"compile error here").expect("write rs");
        std::fs::write(&src_stderr, b"expected: error[E0...]").expect("write stderr");

        let cleanup = CleanupGuard::new(false);
        let fixtures = vec![DiscoveredFixture {
            fixture_path: src_rs.clone(),
            relative_path: "tests/ui/bad.rs".into(),
            kind: FixtureKind::CompileFail,
            call_site: dummy_call_site(src_dir.join("trybuild.rs")),
        }];

        let out = convert_fixtures(compat_root, &fixtures, &cleanup).expect("convert");
        assert_eq!(out.len(), 1);
        let dest_rs = compat_root
            .join("target")
            .join("lihaaf-compat-converted")
            .join("compile_fail")
            .join("bad.rs");
        let dest_stderr = dest_rs.with_extension("stderr");
        assert!(
            dest_rs.exists(),
            "compile_fail fixture must be copied to {dest_rs:?}"
        );
        assert!(
            dest_stderr.exists(),
            "stderr snapshot must be copied to {dest_stderr:?}"
        );
        assert_eq!(out[0].dest_path, dest_rs);
        assert_eq!(out[0].dest_stderr.as_ref(), Some(&dest_stderr));
    }

    #[test]
    fn convert_sorts_output_by_dest_path() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let compat_root = tmp.path();
        let src_dir = compat_root.join("tests").join("ui");
        std::fs::create_dir_all(&src_dir).expect("create src dir");
        let src_z = src_dir.join("z.rs");
        let src_a = src_dir.join("a.rs");
        std::fs::write(&src_z, b"").expect("write z");
        std::fs::write(&src_a, b"").expect("write a");

        let cleanup = CleanupGuard::new(false);
        let fixtures = vec![
            DiscoveredFixture {
                fixture_path: src_z.clone(),
                relative_path: "tests/ui/z.rs".into(),
                kind: FixtureKind::Pass,
                call_site: dummy_call_site(src_dir.join("trybuild.rs")),
            },
            DiscoveredFixture {
                fixture_path: src_a.clone(),
                relative_path: "tests/ui/a.rs".into(),
                kind: FixtureKind::Pass,
                call_site: dummy_call_site(src_dir.join("trybuild.rs")),
            },
        ];

        let out = convert_fixtures(compat_root, &fixtures, &cleanup).expect("convert");
        assert_eq!(out.len(), 2);
        assert!(
            out[0].dest_path < out[1].dest_path,
            "output must be sorted; got {:?} then {:?}",
            out[0].dest_path,
            out[1].dest_path,
        );
    }
}
