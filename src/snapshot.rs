//! `.stderr` snapshot file I/O + `--bless` semantics (spec §7.3, §7.4).
//!
//! ## Byte-determinism
//!
//! Spec §7.4: snapshots are written with LF line endings on every
//! platform and a final newline. They are rewritten in full (no append;
//! no in-place edit). [`write`] enforces both invariants.
//!
//! ## `--bless` is destructive
//!
//! Spec §7.3 + KR-4: bless overwrites checked-in `.stderr` files. The
//! harness assumes adopters have version control and review diffs
//! before committing. There is no sidecar mode in v0.1.

use std::path::{Path, PathBuf};

use crate::error::Error;
use crate::util;

/// The `.stderr` sibling for a fixture.
pub fn snapshot_path(fixture_path: &Path) -> PathBuf {
    fixture_path.with_extension("stderr")
}

/// Read the snapshot for `fixture_path` if present. Returns `Ok(None)`
/// when the file does not exist; that is the `SNAPSHOT_MISSING` case
/// the caller distinguishes.
pub fn try_read(fixture_path: &Path) -> Result<Option<String>, Error> {
    let p = snapshot_path(fixture_path);
    match std::fs::read_to_string(&p) {
        Ok(s) => Ok(Some(normalize_for_compare(&s))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(Error::io(e, "reading snapshot", Some(p))),
    }
}

/// Write `normalized_stderr` to `fixture_path`'s sibling `.stderr` per
/// spec §7.4 (LF, final newline, full rewrite).
pub fn write(fixture_path: &Path, normalized_stderr: &str) -> Result<PathBuf, Error> {
    let p = snapshot_path(fixture_path);
    let mut bytes: Vec<u8> = normalized_stderr.bytes().collect();
    // Ensure exactly one trailing LF.
    while bytes.last().copied() == Some(b'\n') {
        bytes.pop();
    }
    bytes.push(b'\n');
    util::write_file_atomic(&p, &bytes)?;
    Ok(p)
}

/// Snapshot-side normalization for COMPARISON only: unify line endings
/// to LF and strip a single trailing LF if present. Mirrors the
/// `normalize::normalize` final shape so adopters with mixed line
/// endings on disk see clean diffs.
fn normalize_for_compare(s: &str) -> String {
    let mut s = s.replace("\r\n", "\n").replace('\r', "\n");
    while s.ends_with('\n') {
        s.pop();
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn snapshot_path_swaps_extension() {
        assert_eq!(
            snapshot_path(Path::new("foo/bar.rs")),
            PathBuf::from("foo/bar.stderr")
        );
    }

    #[test]
    fn write_then_read_round_trips() {
        let tmp = tempdir().unwrap();
        let fixture = tmp.path().join("fixture.rs");
        let p = write(&fixture, "alpha\nbeta").unwrap();
        assert_eq!(p, tmp.path().join("fixture.stderr"));
        let bytes = std::fs::read(&p).unwrap();
        assert_eq!(bytes, b"alpha\nbeta\n".to_vec());
        let read = try_read(&fixture).unwrap().unwrap();
        assert_eq!(read, "alpha\nbeta");
    }

    #[test]
    fn write_strips_then_appends_single_trailing_newline() {
        let tmp = tempdir().unwrap();
        let fixture = tmp.path().join("x.rs");
        write(&fixture, "alpha\n\n\n").unwrap();
        let bytes = std::fs::read(tmp.path().join("x.stderr")).unwrap();
        assert_eq!(bytes, b"alpha\n".to_vec());
    }

    #[test]
    fn try_read_returns_none_when_missing() {
        let tmp = tempdir().unwrap();
        assert!(try_read(&tmp.path().join("absent.rs")).unwrap().is_none());
    }

    #[test]
    fn try_read_normalizes_crlf() {
        let tmp = tempdir().unwrap();
        let fixture = tmp.path().join("crlf.rs");
        let snap = tmp.path().join("crlf.stderr");
        std::fs::write(&snap, b"a\r\nb\r\n").unwrap();
        let read = try_read(&fixture).unwrap().unwrap();
        assert_eq!(read, "a\nb");
    }
}
