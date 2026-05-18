//! Fixture discovery.
//!
//! Walk `fixture_dirs` non-recursively, collect `*.rs` files, classify
//! each as compile_pass or compile_fail by directory marker, and sort
//! results lexicographically for deterministic output.
//!
//! ## Why non-recursive
//!
//! The flat walk keeps fixture intent obvious and avoids ambiguity in
//! marker handling. If a project wants deep fixture trees, each sub-dir
//! is listed explicitly in `fixture_dirs`.
//!
//! ## Filtering
//!
//! `--filter <substr>` is applied here. Multiple filters
//! are OR'd. Substring match against the relative path from the
//! crate root (forward-slash form for cross-OS determinism).

use std::path::{Path, PathBuf};

use crate::config::Suite;
use crate::error::{Error, Outcome};
use crate::util;

/// One discovered fixture.
#[derive(Debug, Clone)]
pub struct Fixture {
    /// Absolute path to the `.rs` file.
    pub path: PathBuf,
    /// Relative path from the crate root, forward-slash form. Used for
    /// `--filter`, the report, and the snapshot's relative-path
    /// rendering.
    pub relative_path: String,
    /// Stem (filename without extension), for naming the per-fixture
    /// output binary.
    pub stem: String,
    /// Whether the fixture is compile_pass or compile_fail (per the
    /// directory-name marker).
    pub kind: FixtureKind,
}

/// Compile-pass vs compile-fail classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FixtureKind {
    /// Adopter expects rustc to exit 0.
    CompilePass,
    /// Adopter expects rustc to exit non-zero AND the normalized stderr
    /// to match the sibling `.stderr` snapshot.
    CompileFail,
}

/// Collect all fixtures under `suite.fixture_dirs`, sort
/// lexicographically, apply `--filter` if non-empty.
///
/// `crate_root` is the directory containing the consumer crate's
/// `Cargo.toml`. Relative `fixture_dirs` resolve against it.
///
/// Each suite's discovery runs independently — fixtures from different
/// suites never appear in the same returned vector. The session orchestrator
/// loops over suites and aggregates per-suite results into a final report.
pub fn collect(
    suite: &Suite,
    crate_root: &Path,
    filters: &[String],
) -> Result<Vec<Fixture>, Error> {
    let mut existing_dirs: Vec<PathBuf> = Vec::new();
    for dir in &suite.fixture_dirs {
        let resolved = if dir.is_absolute() {
            dir.clone()
        } else {
            crate_root.join(dir)
        };
        if resolved.is_dir() {
            existing_dirs.push(resolved);
        }
    }

    if existing_dirs.is_empty() {
        return Err(Error::Session(Outcome::ConfigInvalid {
            message: format!(
                "suite \"{}\".fixture_dirs resolves to zero existing directories under {}.\nWhy this matters: nothing to test.\n  Configured: {:?}",
                suite.name,
                crate_root.display(),
                suite.fixture_dirs
            ),
        }));
    }

    let marker = suite.compile_fail_marker.as_str();
    let mut fixtures: Vec<Fixture> = Vec::new();

    for dir in &existing_dirs {
        let kind = classify_dir(dir, marker);
        let entries = std::fs::read_dir(dir)
            .map_err(|e| Error::io(e, "reading fixture_dirs", Some(dir.clone())))?;
        for entry in entries {
            let entry =
                entry.map_err(|e| Error::io(e, "iterating fixture_dirs", Some(dir.clone())))?;
            let p = entry.path();
            if !p.is_file() {
                continue;
            }
            if p.extension().and_then(|e| e.to_str()) != Some("rs") {
                continue;
            }
            let stem = p
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_string();
            let rel =
                util::relative_to(&p, crate_root).unwrap_or_else(|err| err.non_absolute_path());
            fixtures.push(Fixture {
                path: p,
                relative_path: rel,
                stem,
                kind,
            });
        }
    }

    // Sort lexicographically by relative path for deterministic
    // emission (the policy).
    fixtures.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));

    if !filters.is_empty() {
        fixtures.retain(|f| filters.iter().any(|sub| f.relative_path.contains(sub)));
    }

    Ok(fixtures)
}

/// Classify a fixture directory by name. the policy: a fixture is
/// compile_fail if its enclosing directory name (relative to crate
/// root) contains the `compile_fail_marker` substring; otherwise
/// compile_pass.
///
/// Classification is by the directory's filename (the leaf segment), not the
/// full path — the marker ought to be discriminating at the leaf, and
/// fixture authors shouldn't accidentally have a parent path
/// containing the marker (e.g., a `tests/compile_fail/sub/` adopter
/// who wants the `sub/` to be compile_pass).
fn classify_dir(dir: &Path, marker: &str) -> FixtureKind {
    let leaf = dir.file_name().and_then(|s| s.to_str()).unwrap_or("");
    if leaf.contains(marker) {
        FixtureKind::CompileFail
    } else {
        FixtureKind::CompilePass
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    fn suite(name: &str, dirs: Vec<&str>, marker: &str) -> Suite {
        Suite {
            name: name.to_string(),
            extern_crates: vec!["consumer".into()],
            fixture_dirs: dirs.into_iter().map(PathBuf::from).collect(),
            features: vec![],
            allow_lints: vec![],
            edition: "2021".into(),
            dev_deps: vec![],
            compile_fail_marker: marker.to_string(),
            fixture_timeout_secs: 90,
            per_fixture_memory_mb: 1024,
        }
    }

    #[test]
    fn classification_matches_directory_marker() {
        let tmp = tempdir().unwrap();
        let cf = tmp.path().join("tests/lihaaf/compile_fail");
        let cp = tmp.path().join("tests/lihaaf/compile_pass");
        fs::create_dir_all(&cf).unwrap();
        fs::create_dir_all(&cp).unwrap();
        fs::write(cf.join("a.rs"), "").unwrap();
        fs::write(cp.join("b.rs"), "").unwrap();

        let s = suite(
            "default",
            vec!["tests/lihaaf/compile_fail", "tests/lihaaf/compile_pass"],
            "compile_fail",
        );
        let fixtures = collect(&s, tmp.path(), &[]).unwrap();
        assert_eq!(fixtures.len(), 2);
        let a = fixtures.iter().find(|f| f.stem == "a").unwrap();
        let b = fixtures.iter().find(|f| f.stem == "b").unwrap();
        assert_eq!(a.kind, FixtureKind::CompileFail);
        assert_eq!(b.kind, FixtureKind::CompilePass);
    }

    #[test]
    fn sort_is_lexicographic() {
        let tmp = tempdir().unwrap();
        let cf = tmp.path().join("tests/lihaaf/compile_fail");
        fs::create_dir_all(&cf).unwrap();
        for name in ["zeta.rs", "alpha.rs", "mu.rs"] {
            fs::write(cf.join(name), "").unwrap();
        }
        let s = suite("default", vec!["tests/lihaaf/compile_fail"], "compile_fail");
        let fixtures = collect(&s, tmp.path(), &[]).unwrap();
        let stems: Vec<_> = fixtures.iter().map(|f| f.stem.clone()).collect();
        assert_eq!(stems, vec!["alpha".to_string(), "mu".into(), "zeta".into()]);
    }

    #[test]
    fn filter_or_match_against_relative_path() {
        let tmp = tempdir().unwrap();
        let cf = tmp.path().join("tests/lihaaf/compile_fail");
        fs::create_dir_all(&cf).unwrap();
        fs::write(cf.join("phase7_aaa.rs"), "").unwrap();
        fs::write(cf.join("phase8_bbb.rs"), "").unwrap();
        fs::write(cf.join("unrelated.rs"), "").unwrap();
        let s = suite("default", vec!["tests/lihaaf/compile_fail"], "compile_fail");
        let fixtures = collect(
            &s,
            tmp.path(),
            &["phase7".to_string(), "phase8".to_string()],
        )
        .unwrap();
        assert_eq!(fixtures.len(), 2);
    }

    #[test]
    fn empty_fixture_dirs_is_session_outcome() {
        let tmp = tempdir().unwrap();
        let s = suite(
            "default",
            vec!["nope/never/exists", "also/missing"],
            "compile_fail",
        );
        let err = collect(&s, tmp.path(), &[]).unwrap_err();
        match err {
            Error::Session(Outcome::ConfigInvalid { message }) => {
                assert!(message.contains("zero existing directories"));
                // The diagnostic names the offending suite so multi-suite
                // adopters can locate the typo without grepping.
                assert!(message.contains("\"default\""));
            }
            other => panic!("expected ConfigInvalid, got {other:?}"),
        }
    }

    #[test]
    fn empty_fixture_dirs_diagnostic_includes_named_suite() {
        // Multi-suite diagnostic surface: when a NAMED suite's
        // fixture_dirs resolve to zero existing directories, the failure
        // message must include that suite's name so adopters know where
        // to look.
        let tmp = tempdir().unwrap();
        let s = suite("spatial", vec!["tests/spatial/missing"], "compile_fail");
        let err = collect(&s, tmp.path(), &[]).unwrap_err();
        match err {
            Error::Session(Outcome::ConfigInvalid { message }) => {
                assert!(message.contains("\"spatial\""));
            }
            other => panic!("expected ConfigInvalid, got {other:?}"),
        }
    }

    #[test]
    fn non_recursive_within_each_dir() {
        let tmp = tempdir().unwrap();
        let cf = tmp.path().join("tests/lihaaf/compile_fail");
        let nested = cf.join("subdir");
        fs::create_dir_all(&nested).unwrap();
        fs::write(cf.join("flat.rs"), "").unwrap();
        fs::write(nested.join("deep.rs"), "").unwrap();
        let s = suite("default", vec!["tests/lihaaf/compile_fail"], "compile_fail");
        let fixtures = collect(&s, tmp.path(), &[]).unwrap();
        let stems: Vec<_> = fixtures.iter().map(|f| f.stem.clone()).collect();
        assert_eq!(stems, vec!["flat".to_string()]);
    }
}
