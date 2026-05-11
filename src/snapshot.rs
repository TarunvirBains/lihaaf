//! `.stderr` snapshot file I/O + `--bless` semantics (spec §7.3, §7.4).
//!
//! ## Byte-determinism
//!
//! Spec §7.4: snapshots are written with LF line endings on every
//! platform and a final newline. They are rewritten in full (no append;
//! no in-place edit). [`write()`] enforces both invariants.
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

/// Outcome of reading a snapshot file, distinguishing the three cases
/// that drive different verdict paths.
#[derive(Debug)]
pub enum ReadOutcome {
    /// File exists, was UTF-8 valid, and was normalized for comparison.
    Found(String),
    /// File does not exist on disk. Caller emits `SNAPSHOT_MISSING`.
    Missing,
    /// File exists but failed UTF-8 validation. Caller emits
    /// `MALFORMED_DIAGNOSTIC` with the carried byte offset (the index
    /// of the first invalid byte, equal to `Utf8Error::valid_up_to()`).
    Malformed {
        /// Byte offset of the first invalid byte. `0` means the file
        /// began with an invalid sequence.
        byte_offset: usize,
        /// Total byte length of the file (for the diagnostic line).
        total_bytes: usize,
    },
}

/// Read the snapshot for `fixture_path` and classify the outcome.
///
/// Spec §7.2 ("Non-UTF-8 / binary-content handling") is the authority
/// for the malformed branch — the snapshot file failing UTF-8 is a
/// `MALFORMED_DIAGNOSTIC` verdict, not a soft fallback. Earlier drafts
/// of this module returned a generic IO error for malformed files,
/// which collapsed the byte offset to a useless `0`; the typed
/// outcome here preserves the offset that `Utf8Error::valid_up_to()`
/// provides.
pub fn try_read(fixture_path: &Path) -> Result<ReadOutcome, Error> {
    let p = snapshot_path(fixture_path);
    match std::fs::read(&p) {
        Ok(bytes) => match std::str::from_utf8(&bytes) {
            Ok(s) => Ok(ReadOutcome::Found(normalize_for_compare(s))),
            Err(e) => Ok(ReadOutcome::Malformed {
                byte_offset: e.valid_up_to(),
                total_bytes: bytes.len(),
            }),
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(ReadOutcome::Missing),
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
        match try_read(&fixture).unwrap() {
            ReadOutcome::Found(s) => assert_eq!(s, "alpha\nbeta"),
            other => panic!("expected Found, got {other:?}"),
        }
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
    fn try_read_returns_missing_when_absent() {
        let tmp = tempdir().unwrap();
        match try_read(&tmp.path().join("absent.rs")).unwrap() {
            ReadOutcome::Missing => {}
            other => panic!("expected Missing, got {other:?}"),
        }
    }

    #[test]
    fn try_read_normalizes_crlf() {
        let tmp = tempdir().unwrap();
        let fixture = tmp.path().join("crlf.rs");
        let snap = tmp.path().join("crlf.stderr");
        std::fs::write(&snap, b"a\r\nb\r\n").unwrap();
        match try_read(&fixture).unwrap() {
            ReadOutcome::Found(s) => assert_eq!(s, "a\nb"),
            other => panic!("expected Found, got {other:?}"),
        }
    }

    #[test]
    fn try_read_returns_malformed_with_correct_offset() {
        // Spec §7.2: snapshot UTF-8 failure surfaces as
        // MALFORMED_DIAGNOSTIC with the precise byte offset of the
        // first invalid byte. `0xFE` is invalid as a UTF-8 leading
        // byte; placing it after 5 valid bytes gives a deterministic
        // expected offset of 5.
        let tmp = tempdir().unwrap();
        let fixture = tmp.path().join("badbytes.rs");
        let snap = tmp.path().join("badbytes.stderr");
        let mut payload: Vec<u8> = b"hello".to_vec();
        payload.push(0xFE);
        payload.extend_from_slice(b"world");
        std::fs::write(&snap, &payload).unwrap();
        match try_read(&fixture).unwrap() {
            ReadOutcome::Malformed {
                byte_offset,
                total_bytes,
            } => {
                assert_eq!(byte_offset, 5, "first invalid byte is at offset 5");
                assert_eq!(total_bytes, payload.len());
            }
            other => panic!("expected Malformed, got {other:?}"),
        }
    }

    #[test]
    fn try_read_malformed_at_offset_zero_when_first_byte_invalid() {
        // Edge case: `0xFE` at position 0 — `valid_up_to()` returns 0.
        let tmp = tempdir().unwrap();
        let fixture = tmp.path().join("badfirst.rs");
        let snap = tmp.path().join("badfirst.stderr");
        std::fs::write(&snap, [0xFE]).unwrap();
        match try_read(&fixture).unwrap() {
            ReadOutcome::Malformed {
                byte_offset,
                total_bytes,
            } => {
                assert_eq!(byte_offset, 0);
                assert_eq!(total_bytes, 1);
            }
            other => panic!("expected Malformed, got {other:?}"),
        }
    }
}
