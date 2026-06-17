//! Small cross-module helpers: atomic file writes, SHA-256, and path
//! normalization helpers used by snapshot comparisons.

use std::fmt;
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
        // Hand-rolled hex to avoid the `hex` dep. Stable lower
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
/// `rename` is atomic on POSIX; on Windows read/readiness issues
/// surface through normal session checks.
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

/// Error returned when a path cannot be rendered relative to a base.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelativePathError {
    path: PathBuf,
    base: PathBuf,
}

impl RelativePathError {
    /// Deterministic non-absolute rendering for callers that must keep
    /// reporting an out-of-base diagnostic path without leaking the
    /// runner-specific absolute prefix.
    pub fn non_absolute_path(&self) -> String {
        non_absolute_outside_base_path(&self.path)
    }
}

impl fmt::Display for RelativePathError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "path `{}` is not under base `{}`",
            self.path.display(),
            self.base.display()
        )
    }
}

/// Render an out-of-base path without an absolute prefix.
///
/// v0.1 compat envelopes are Linux-only, but this still routes through
/// [`to_forward_slash`] so the fallback is stable if a backslash-bearing
/// diagnostic path reaches this boundary.
pub fn non_absolute_outside_base_path(path: &Path) -> String {
    let rendered = to_forward_slash(&path.to_string_lossy());
    let trimmed = rendered.trim_start_matches('/');
    if trimmed.is_empty() {
        "outside-base".to_string()
    } else {
        format!("outside-base/{trimmed}")
    }
}

/// Compute a path relative to `base`.
///
/// The result is always forward-slash form for stable report output.
pub fn relative_to(path: &Path, base: &Path) -> Result<String, RelativePathError> {
    path.strip_prefix(base)
        .map(|rel| to_forward_slash(&rel.to_string_lossy()))
        .map_err(|_| RelativePathError {
            path: path.to_path_buf(),
            base: base.to_path_buf(),
        })
}

/// Canonicalize `candidate` and ensure it stays within `base`.
///
/// If `candidate` does not yet exist, canonicalize its parent directory and
/// rejoin the final path component. This keeps callers from having to special
/// case non-existent leaf paths while still rejecting symlink escapes.
pub(crate) fn canonicalize_within_base(base: &Path, candidate: &Path) -> std::io::Result<PathBuf> {
    let canonical_base = std::fs::canonicalize(base)?;

    let canonical_candidate = match std::fs::canonicalize(candidate) {
        Ok(path) => path,
        Err(_) => {
            let parent = candidate.parent().ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    format!("containment check: no parent for {}", candidate.display()),
                )
            })?;
            let canonical_parent = std::fs::canonicalize(parent)?;
            canonical_parent.join(candidate.file_name().ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    format!(
                        "containment check: no file_name for {}",
                        candidate.display()
                    ),
                )
            })?)
        }
    };

    if !canonical_candidate.starts_with(&canonical_base) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "path escapes allowed root",
        ));
    }

    Ok(canonical_candidate)
}

/// Remove `path` from the filesystem without relying on a prior
/// `symlink_metadata` stat. Best-effort: a non-existent path is treated
/// as already-cleaned (no error) so cleanup is idempotent across reruns.
///
/// ## Race-free cascade
///
/// The naïve shape is `symlink_metadata(path)` → branch on `file_type`
/// → dispatch to `remove_file` / `remove_dir` / `remove_dir_all`. That
/// is TOCTOU-vulnerable: between the stat and the removal call, the
/// path entry can be swapped to a different entry kind (e.g. a symlink
/// pointing outside the intended scope), and the wrong syscall fires.
///
/// This cascade eliminates the stat-then-dispatch. Each step operates
/// on the path's current state via a single syscall, and each step's
/// error space tells us which step to try next:
///
/// 1. **`remove_file`** — handles regular files AND file-symlinks.
///    - Unix: `unlink(2)` removes the directory entry for both regular
///      files and symlinks (regardless of target type) — never follows
///      the link.
///    - Windows: `DeleteFileW` handles regular files and file-symlinks;
///      it refuses directories and directory-symlinks with
///      `ERROR_ACCESS_DENIED` (surfaced as `PermissionDenied`, and in
///      some cases `IsADirectory`).
///    - On `IsADirectory` we proceed to step 2 (Unix EISDIR and the
///      stable mapping some Windows shapes surface).
///    - **`PermissionDenied` is platform-specific.** On Windows it is
///      the "this entry is a directory" signal, so we fall through.
///      On Unix it is a real EACCES (parent without write permission,
///      immutable attribute, missing search bit on an ancestor) —
///      surfacing that as "not a file → try step 2" would mask the
///      cause, so we return it directly.
/// 2. **`remove_dir`** — handles empty directories AND directory-symlinks.
///    - Unix: `rmdir(2)` succeeds on empty directories; fails
///      `ENOTEMPTY` (`DirectoryNotEmpty`) on non-empty ones.
///    - Windows: `RemoveDirectoryW` removes empty directories AND
///      directory-symlinks AND junctions — it removes the LINK, not the
///      target tree.
///    - On `DirectoryNotEmpty` we proceed to step 3.
///    - **Windows-only `PermissionDenied` fall-through.** A read-only
///      non-empty Windows directory returns `ERROR_ACCESS_DENIED`
///      from `RemoveDirectoryW` instead of `DirectoryNotEmpty`. Treat
///      it as "non-empty, try recursive" so step 3 can clear it.
/// 3. **`remove_dir_all`** — recursive removal of a non-empty directory.
///    - Rust 1.84+ `std::fs::remove_dir_all` is race-safe internally: it
///      refuses to follow symlinks during the recursive walk. MSRV is
///      1.95 (see `Cargo.toml`), so the race-safe behavior is
///      guaranteed.
///
/// Each step independently checks `NotFound` — a concurrent unlink
/// between steps is treated as already-cleaned (idempotent).
///
/// ## `context_prefix`
///
/// Callers supply a short noun phrase that describes the artifact
/// being removed. The three step-specific error contexts are built by
/// concatenating:
///
/// - `"removing {context_prefix} file/symlink"`
/// - `"removing {context_prefix} empty dir / dir-symlink"`
/// - `"recursively removing {context_prefix} directory"`
///
/// For compat-cleanup the prefix is `"compat-generated"`; for the
/// managed-dylib swap it is `"prior managed dylib"`. Keep the prefix
/// short and noun-shaped — the surrounding template names the syscall
/// stage.
pub(crate) fn remove_path_race_free(path: &Path, context_prefix: &str) -> Result<(), Error> {
    use std::io::ErrorKind;

    // Step 1: try to unlink as a file or file-symlink.
    match std::fs::remove_file(path) {
        Ok(()) => return Ok(()),
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(()),
        // IsADirectory is stable across platforms as the "not a file"
        // signal (Unix EISDIR; some Windows shapes surface here too).
        Err(e) if e.kind() == ErrorKind::IsADirectory => {}
        // Windows-only: DeleteFileW returns ERROR_ACCESS_DENIED for
        // directories and directory-symlinks (mapped to
        // PermissionDenied). Treat as "this is a directory" → step 2.
        // Unix PermissionDenied is a real EACCES; let it surface
        // via the catch-all so the diagnostic stays honest.
        #[cfg(windows)]
        Err(e) if e.kind() == ErrorKind::PermissionDenied => {}
        Err(e) => {
            return Err(Error::io(
                e,
                format!("removing {context_prefix} file/symlink"),
                Some(path.to_path_buf()),
            ));
        }
    }

    // Step 2: try to remove as an empty directory or a directory-symlink.
    match std::fs::remove_dir(path) {
        Ok(()) => return Ok(()),
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(()),
        Err(e) if e.kind() == ErrorKind::DirectoryNotEmpty => {}
        // Windows-only: RemoveDirectoryW returns ERROR_ACCESS_DENIED
        // (PermissionDenied) for a read-only non-empty directory
        // instead of DirectoryNotEmpty. Treat as "non-empty" so step
        // 3 clears it. Unix's `rmdir(2)` does not return EACCES for
        // a non-empty directory; the catch-all preserves the
        // diagnostic.
        #[cfg(windows)]
        Err(e) if e.kind() == ErrorKind::PermissionDenied => {}
        Err(e) => {
            return Err(Error::io(
                e,
                format!("removing {context_prefix} empty dir / dir-symlink"),
                Some(path.to_path_buf()),
            ));
        }
    }

    // Step 3: recursive removal of a non-empty directory.
    match std::fs::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(()),
        Err(e) => Err(Error::io(
            e,
            format!("recursively removing {context_prefix} directory"),
            Some(path.to_path_buf()),
        )),
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
        assert_eq!(relative_to(&p, &base).unwrap(), "c/d.rs");
    }

    #[test]
    fn relative_to_errors_when_no_common_prefix() {
        let base = std::path::PathBuf::from("/x");
        let p = std::path::PathBuf::from("/y/z.rs");
        let err = relative_to(&p, &base).unwrap_err();
        assert_eq!(err.to_string(), "path `/y/z.rs` is not under base `/x`");
    }

    #[test]
    fn relative_to_error_fallback_is_not_absolute() {
        let base = std::path::PathBuf::from("/x");
        let p = std::path::PathBuf::from("/y/z.rs");
        let err = relative_to(&p, &base).unwrap_err();
        assert_eq!(err.non_absolute_path(), "outside-base/y/z.rs");
    }

    #[test]
    fn canonicalize_within_base_accepts_existing_path() {
        let tmp = tempdir().unwrap();
        let child = tmp.path().join("file.txt");
        std::fs::write(&child, "data").unwrap();

        let resolved = canonicalize_within_base(tmp.path(), &child).unwrap();
        assert_eq!(resolved, child.canonicalize().unwrap());
    }

    #[test]
    fn canonicalize_within_base_accepts_nonexisting_child() {
        let tmp = tempdir().unwrap();
        let child = tmp.path().join("nested").join("file.txt");
        std::fs::create_dir(tmp.path().join("nested")).unwrap();

        let resolved = canonicalize_within_base(tmp.path(), &child).unwrap();
        assert_eq!(resolved, tmp.path().join("nested").join("file.txt"));
    }

    #[cfg(unix)]
    #[test]
    fn canonicalize_within_base_rejects_symlink_escape() {
        use std::os::unix::fs::symlink;

        let base = tempdir().unwrap();
        let outside = tempdir().unwrap();
        let escape = base.path().join("escape");
        symlink(outside.path(), &escape).unwrap();
        let target = escape.join("child.txt");

        let err = canonicalize_within_base(base.path(), &target).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::PermissionDenied);
    }
}
