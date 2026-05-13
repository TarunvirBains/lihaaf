//! `target/lihaaf/manifest.json` schema + atomic refresh.
//!
//! The manifest is written atomically (`.tmp` + rename), then re-read on each
//! fixture dispatch so stale cache state is detected before linking continues.
//!
//! ## Why JSON, not TOML
//!
//! TOML is harder to script in CI environments that lean on `jq` and similar
//! tooling. During `MANIFEST_CORRUPT` debugging, `cat target/lihaaf/manifest.json`
//! into `jq .dylib_sha256` is usually the shortest path.
//!
//! ## Schema versioning
//!
//! `lihaaf_version` is the harness's own version marker. Adding fields is
//! non-breaking because older readers get `serde(default)` values. Removing or
//! renaming fields is a breaking change.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::Error;
use crate::util;

/// `target/lihaaf/manifest.json` shape.
///
/// One manifest is written per suite: the default suite uses
/// `target/lihaaf/manifest.json` (backward-compatible name), and each
/// named suite writes to `target/lihaaf/manifest-<suite>.json`. The
/// [`Self::suite_name`] field carries the suite name explicitly so
/// out-of-band tooling reading the manifest does not need to derive the
/// name from the file path.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Manifest {
    /// The lihaaf release that wrote this manifest. Pinned to
    /// [`crate::VERSION`] on write; reads of older versions are
    /// tolerated via `serde(default)` on additive fields.
    pub lihaaf_version: String,

    /// Suite name. `"default"` for the implicit suite built from the
    /// top-level `[package.metadata.lihaaf]` table; otherwise the named
    /// `[[package.metadata.lihaaf.suite]].name`. Defaulted to the
    /// reserved `"default"` so manifests written by lihaaf <0.1.0-alpha.3
    /// (which had no suite concept) keep deserializing.
    #[serde(default = "default_suite_name_field")]
    pub suite_name: String,

    /// Verbatim first line of `rustc --version --verbose` (drift-detection key).
    pub rustc_release: String,

    /// `commit-hash:` value if rustc reported one; empty string for
    /// custom builds.
    pub rustc_commit_hash: String,

    /// `host:` value (e.g., `x86_64-unknown-linux-gnu`).
    pub host_triple: String,

    /// Output of `rustc --print sysroot`.
    pub sysroot: PathBuf,

    /// `dylib_crate` from `[package.metadata.lihaaf]`.
    pub dylib_crate: String,

    /// The cargo-emitted dylib path
    /// (`target/<triple>/<profile>/deps/lib<crate>-<hash>.so`). Cargo
    /// may freely replace this between fixture dispatches.
    pub cargo_dylib_path: PathBuf,

    /// The lihaaf-managed copy path
    /// (`target/lihaaf/lib<crate>-current-<hash>.so`). Fixture workers
    /// link against this, never the cargo-managed original.
    pub managed_dylib_path: PathBuf,

    /// SHA-256 of the managed dylib at the moment of copy. The freshness
    /// check re-hashes on every dispatch and a mismatch triggers rebuild.
    pub dylib_sha256: String,

    /// The managed dylib's mtime in Unix seconds. A backward jump is
    /// the freshness-check trigger for clock skew or external mutation.
    pub dylib_mtime_unix_secs: i64,

    /// True when `--use-symlink` was active and `managed_dylib_path` is
    /// a symlink rather than a copy.
    pub use_symlink: bool,

    /// Cargo features enabled for the dylib build (verbatim copy from
    /// `[package.metadata.lihaaf].features`).
    #[serde(default)]
    pub features: Vec<String>,

    /// Verbatim copy of `extern_crates` from the metadata.
    pub extern_crates: Vec<String>,

    /// Verbatim copy of `edition` from the metadata.
    pub edition: String,

    /// Verbatim copy of the entire `[package.metadata.lihaaf]` table.
    /// Stored as JSON value for cross-tool readability.
    pub metadata_snapshot: serde_json::Value,
}

/// Default for the [`Manifest::suite_name`] field on legacy manifests.
/// `serde` uses this when deserializing a manifest from a lihaaf
/// release that predated the suite concept. Kept in sync with
/// [`crate::config::DEFAULT_SUITE_NAME`].
fn default_suite_name_field() -> String {
    crate::config::DEFAULT_SUITE_NAME.to_string()
}

/// Compute the on-disk path for a per-suite manifest under
/// `<workspace_target>/lihaaf/`.
///
/// The default suite uses the legacy unsuffixed name (`manifest.json`)
/// to keep manifest paths stable across the suite-introducing release;
/// named suites get a `manifest-<name>.json` filename. The suite name
/// is validated by [`crate::config::parse`] to contain only ASCII
/// alphanumerics, hyphens, and underscores, so this string substitution
/// is safe to use as a filename component on every supported platform.
pub fn manifest_path_for_suite(workspace_target: &Path, suite_name: &str) -> PathBuf {
    let lihaaf_dir = workspace_target.join("lihaaf");
    if suite_name == crate::config::DEFAULT_SUITE_NAME {
        lihaaf_dir.join("manifest.json")
    } else {
        lihaaf_dir.join(format!("manifest-{suite_name}.json"))
    }
}

impl Manifest {
    /// Read an existing manifest from disk. A failure to read or parse
    /// returns `None`; callers treat that as a stale-cache event and rebuild.
    pub fn try_read(path: &Path) -> Option<Self> {
        let text = std::fs::read_to_string(path).ok()?;
        serde_json::from_str(&text).ok()
    }

    /// Write the manifest atomically.
    pub fn write(&self, path: &Path) -> Result<(), Error> {
        let text = serde_json::to_vec_pretty(self).map_err(|e| Error::JsonParse {
            context: "serializing manifest".into(),
            message: e.to_string(),
        })?;
        let mut text = text;
        // Final newline so `cat` output stays readable.
        text.push(b'\n');
        util::write_file_atomic(path, &text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::tempdir;

    fn sample() -> Manifest {
        Manifest {
            lihaaf_version: "0.1.0".into(),
            suite_name: crate::config::DEFAULT_SUITE_NAME.into(),
            rustc_release: "rustc 1.95.0 (abc 2026-01-01)".into(),
            rustc_commit_hash: "abc".into(),
            host_triple: "x86_64-unknown-linux-gnu".into(),
            sysroot: PathBuf::from("/home/u/.rustup/toolchains/stable-x86_64-unknown-linux-gnu"),
            dylib_crate: "consumer".into(),
            cargo_dylib_path: PathBuf::from("/p/target/release/deps/libconsumer-abc.so"),
            managed_dylib_path: PathBuf::from("/p/target/lihaaf/libconsumer-current-abc.so"),
            dylib_sha256: "deadbeef".into(),
            dylib_mtime_unix_secs: 1746883200,
            use_symlink: false,
            features: vec!["testing".into()],
            extern_crates: vec!["consumer".into(), "consumer-macros".into()],
            edition: "2021".into(),
            metadata_snapshot: json!({"dylib_crate": "consumer"}),
        }
    }

    #[test]
    fn round_trips_through_disk() {
        let tmp = tempdir().unwrap();
        let path = tmp.path().join("manifest.json");
        let m = sample();
        m.write(&path).unwrap();
        let read = Manifest::try_read(&path).unwrap();
        assert_eq!(read, m);
    }

    #[test]
    fn try_read_returns_none_on_corrupt_manifest() {
        let tmp = tempdir().unwrap();
        let path = tmp.path().join("manifest.json");
        std::fs::write(&path, "not json").unwrap();
        assert!(Manifest::try_read(&path).is_none());
    }

    #[test]
    fn try_read_returns_none_on_missing_manifest() {
        let tmp = tempdir().unwrap();
        let path = tmp.path().join("nope.json");
        assert!(Manifest::try_read(&path).is_none());
    }

    #[test]
    fn legacy_manifest_without_suite_name_round_trips_with_default() {
        // Manifests written by lihaaf <0.1.0-alpha.3 (pre-suite) had no
        // `suite_name` field. The `serde(default)` annotation must
        // backfill the reserved "default" name so older on-disk state
        // continues to deserialize without manual migration.
        let legacy_json = r#"{
            "lihaaf_version": "0.1.0-alpha.2",
            "rustc_release": "rustc 1.95.0 (abc 2026-01-01)",
            "rustc_commit_hash": "abc",
            "host_triple": "x86_64-unknown-linux-gnu",
            "sysroot": "/r",
            "dylib_crate": "consumer",
            "cargo_dylib_path": "/p/target/release/deps/libconsumer-abc.so",
            "managed_dylib_path": "/p/target/lihaaf/libconsumer-current-abc.so",
            "dylib_sha256": "deadbeef",
            "dylib_mtime_unix_secs": 1746883200,
            "use_symlink": false,
            "features": [],
            "extern_crates": ["consumer"],
            "edition": "2021",
            "metadata_snapshot": {"dylib_crate": "consumer"}
        }"#;
        let m: Manifest = serde_json::from_str(legacy_json).expect("legacy manifest must parse");
        assert_eq!(m.suite_name, crate::config::DEFAULT_SUITE_NAME);
    }

    #[test]
    fn manifest_path_for_default_suite_uses_legacy_name() {
        let p = manifest_path_for_suite(Path::new("/p/target"), crate::config::DEFAULT_SUITE_NAME);
        assert_eq!(p, PathBuf::from("/p/target/lihaaf/manifest.json"));
    }

    #[test]
    fn manifest_path_for_named_suite_includes_name() {
        let p = manifest_path_for_suite(Path::new("/p/target"), "spatial");
        assert_eq!(p, PathBuf::from("/p/target/lihaaf/manifest-spatial.json"));
    }
}
