//! Small cross-module helpers: atomic file writes, SHA-256, and path
//! normalization helpers used by snapshot comparisons.

use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::error::Error;

/// SHA-256 of a file's bytes, formatted as a 64-character lowercase
/// hex string. The representation matches `sha256sum`'s default for
/// grep-friendliness.
pub fn sha256_file(path: &Path) -> Result<String, Error> {
    let bytes = std::fs::read(path)
        .map_err(|e| Error::io(e, "reading file for sha256", Some(path.to_path_buf())))?;
    Ok(sha256_bytes(&bytes))
}

/// SHA-256 of an in-memory byte slice. Same hex format as
/// [`sha256_file`].
pub fn sha256_bytes(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let digest = hasher.finalize();
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        // Hand-rolled hex so we don't need the `hex` dep. Stable lower
        // hex; matches sha256sum byte-for-byte.
        out.push(nibble_to_hex((byte >> 4) & 0xF));
        out.push(nibble_to_hex(byte & 0xF));
    }
    out
}

#[inline]
fn nibble_to_hex(n: u8) -> char {
    match n {
        0..=9 => (b'0' + n) as char,
        10..=15 => (b'a' + n - 10) as char,
        _ => unreachable!(),
    }
}

/// Atomic write helper: write to `path.tmp` then rename into place.
/// `rename` is atomic on POSIX; on Windows we still surface read/readiness
/// issues through normal session checks.
///
/// The `.tmp` suffix is per-call (not per-process) to avoid collisions
/// when the same path is rewritten in quick succession.
pub fn write_file_atomic(path: &Path, contents: &[u8]) -> Result<(), Error> {
    let parent = path.parent().ok_or_else(|| {
        Error::io(
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "path has no parent directory",
            ),
            "atomic write",
            Some(path.to_path_buf()),
        )
    })?;
    std::fs::create_dir_all(parent).map_err(|e| {
        Error::io(
            e,
            "creating parent directory for atomic write",
            Some(parent.to_path_buf()),
        )
    })?;

    let mut tmp_path: PathBuf = path.to_path_buf();
    let mut tmp_name = tmp_path
        .file_name()
        .map(|n| n.to_os_string())
        .unwrap_or_default();
    tmp_name.push(".lihaaf.tmp");
    tmp_path.set_file_name(tmp_name);

    {
        let mut f = File::create(&tmp_path)
            .map_err(|e| Error::io(e, "creating tmp file", Some(tmp_path.clone())))?;
        f.write_all(contents)
            .map_err(|e| Error::io(e, "writing tmp file", Some(tmp_path.clone())))?;
        f.sync_all()
            .map_err(|e| Error::io(e, "syncing tmp file", Some(tmp_path.clone())))?;
    }

    std::fs::rename(&tmp_path, path)
        .map_err(|e| Error::io(e, "renaming tmp file into place", Some(path.to_path_buf())))?;

    Ok(())
}

/// Convert a path's separators to forward-slash form. Used by the
/// stderr normalizer and by relative-path reporting.
pub fn to_forward_slash(s: &str) -> String {
    if !s.contains('\\') {
        return s.to_string();
    }
    s.chars().map(|c| if c == '\\' { '/' } else { c }).collect()
}

/// Compute a path relative to `base`, returning the path verbatim if
/// it cannot be made relative. Used for fixture display paths.
///
/// The result is always forward-slash form for stable report output.
pub fn relative_to(path: &Path, base: &Path) -> String {
    match path.strip_prefix(base) {
        Ok(rel) => to_forward_slash(&rel.to_string_lossy()),
        Err(_) => to_forward_slash(&path.to_string_lossy()),
    }
}

/// Read total system RAM in MB.
///
/// Linux: `/proc/meminfo`'s `MemTotal:` line in KiB.
/// macOS: `sysctl hw.memsize` (bytes) — invoked via subprocess to keep
///   the dep tree libc-free.
/// Windows: returns `None` until a platform path is added.
///
/// A read failure returns `None`; the caller falls back to CPU
/// limit alone.
pub fn total_ram_mb() -> Option<u64> {
    #[cfg(target_os = "linux")]
    {
        let text = std::fs::read_to_string("/proc/meminfo").ok()?;
        for line in text.lines() {
            if let Some(rest) = line.strip_prefix("MemTotal:") {
                let kib: u64 = rest
                    .trim()
                    .strip_suffix("kB")
                    .or_else(|| Some(rest.trim()))?
                    .trim()
                    .parse()
                    .ok()?;
                return Some(kib / 1024);
            }
        }
        None
    }

    #[cfg(target_os = "macos")]
    {
        let out = std::process::Command::new("sysctl")
            .args(["-n", "hw.memsize"])
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        let bytes: u64 = String::from_utf8_lossy(&out.stdout).trim().parse().ok()?;
        Some(bytes / (1024 * 1024))
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn sha256_bytes_matches_known_vector() {
        // Empty input — well-known vector.
        assert_eq!(
            sha256_bytes(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        // "abc" — well-known vector.
        assert_eq!(
            sha256_bytes(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn write_file_atomic_creates_parents_and_writes() {
        let tmp = tempdir().unwrap();
        let target = tmp.path().join("nested/dirs/out.txt");
        write_file_atomic(&target, b"hello world").unwrap();
        let read = std::fs::read(&target).unwrap();
        assert_eq!(read, b"hello world");
        // The .tmp sidecar must have been renamed away.
        let mut tmp_name = target.file_name().unwrap().to_os_string();
        tmp_name.push(".lihaaf.tmp");
        let tmp_sidecar = target.with_file_name(tmp_name);
        assert!(!tmp_sidecar.exists());
    }

    #[test]
    fn forward_slash_passes_through_when_clean() {
        assert_eq!(to_forward_slash("foo/bar/baz.rs"), "foo/bar/baz.rs");
    }

    #[test]
    fn forward_slash_rewrites_backslashes() {
        assert_eq!(
            to_forward_slash(r"C:\Users\me\code\fixture.rs"),
            "C:/Users/me/code/fixture.rs"
        );
    }

    #[test]
    fn relative_to_strips_common_prefix() {
        let base = std::path::PathBuf::from("/a/b");
        let p = std::path::PathBuf::from("/a/b/c/d.rs");
        assert_eq!(relative_to(&p, &base), "c/d.rs");
    }

    #[test]
    fn relative_to_returns_input_when_no_common_prefix() {
        let base = std::path::PathBuf::from("/x");
        let p = std::path::PathBuf::from("/y/z.rs");
        // Forward-slash form preserved.
        assert_eq!(relative_to(&p, &base), "/y/z.rs");
    }
}
