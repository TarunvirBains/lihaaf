//! `target/lihaaf/manifest.json` schema + atomic refresh.
//!
//! Spec §4.4 dictates the exact field set. The manifest is written
//! atomically (write `.tmp`, rename) per spec §4.1 stage 5, and the
//! freshness check (§4.5) re-reads it on every fixture dispatch.
//!
//! ## Why JSON, not TOML
//!
//! TOML is harder to script in CI environments that lean on `jq` and
//! similar tools. Adopters debugging `MANIFEST_CORRUPT` (§10.2) reach
//! for `cat target/lihaaf/manifest.json | jq .dylib_sha256` and that
//! is a much shorter path with JSON.
//!
//! ## Schema versioning
//!
//! `lihaaf_version` is the harness's own version. Adding fields to the
//! manifest is non-breaking (adopters reading older manifests see
//! defaults via `serde(default)`). Removing or renaming fields is a
//! v1.0 break.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::Error;
use crate::util;

/// `target/lihaaf/manifest.json` shape (spec §4.4).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Manifest {
    /// The lihaaf release that wrote this manifest. Pinned to
    /// [`crate::VERSION`] on write; reads of older versions are
    /// tolerated via `serde(default)` on additive fields.
    pub lihaaf_version: String,

    /// Verbatim first line of `rustc --version --verbose` (the
    /// drift-detection key — spec §4.6).
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

    /// SHA-256 of the managed dylib at the moment of copy. The
    /// freshness check (§4.5) re-hashes on every dispatch and a
    /// mismatch triggers the rebuild path.
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

    /// Verbatim copy of the entire `[package.metadata.lihaaf]` table
    /// — spec §4.4. Stored as JSON value for cross-tool readability.
    pub metadata_snapshot: serde_json::Value,
}

impl Manifest {
    /// Read an existing manifest from disk. A failure to read or parse
    /// returns `None` rather than an error — spec §10.2 treats unreadable
    /// manifests as stale-cache events that simply trigger a rebuild.
    pub fn try_read(path: &Path) -> Option<Self> {
        let text = std::fs::read_to_string(path).ok()?;
        serde_json::from_str(&text).ok()
    }

    /// Write the manifest atomically per spec §4.1 stage 5.
    pub fn write(&self, path: &Path) -> Result<(), Error> {
        let text = serde_json::to_vec_pretty(self).map_err(|e| Error::JsonParse {
            context: "serializing manifest".into(),
            message: e.to_string(),
        })?;
        let mut text = text;
        // Final newline so adopters running `cat` get a clean prompt.
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
}
