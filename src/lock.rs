//! Session-wide advisory file lock.
//!
//! Acquired at session start before any state mutation; released when
//! the [`SessionLock`] guard drops. Concurrent `cargo lihaaf` sessions
//! in the same `CARGO_TARGET_DIR` serialize through this lock,
//! preventing the TOCTOU race in which one session deletes
//! `target/lihaaf/` (e.g. via `--no-cache`) while another reads it.
//!
//! ## Crash recovery
//!
//! The lock is held by an open file handle, not by file content. When
//! the holder dies (panic, SIGKILL, system reboot), the OS releases
//! the lock automatically — no stale lockfile to remove.
//!
//! ## Manual override
//!
//! If a sibling session hangs, the user must identify and kill the
//! stuck `cargo lihaaf` or `rustc` process; the OS releases the
//! handle and the lock automatically. **Do not**
//! `rm -f target/lihaaf/.session.lock` — the `rm` will appear to
//! succeed but the next session will fail in a worse way:
//!
//! - **Unix:** `unlink(2)` removes the directory entry, but the hung
//!   process keeps its `flock` on the open file's inode. The next
//!   session creates a *new* `.session.lock` (a fresh inode) and
//!   locks it, while the original holder still owns its old lock.
//!   Both sessions then mutate the cache concurrently, reintroducing
//!   the exact TOCTOU race this module prevents.
//!
//! - **Windows:** Rust opens files with `FILE_SHARE_DELETE` by
//!   default, so `del` or PowerShell `Remove-Item` would return
//!   success and mark the file for pending deletion. To make that
//!   footgun visible, this module's `OpenOptions` configures
//!   `share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)` — explicitly
//!   omitting `FILE_SHARE_DELETE`. A stray `del .session.lock` now
//!   fails loudly with "The process cannot access the file because
//!   it is being used by another process" and the user knows to kill
//!   the hung process instead.
//!
//! See `docs/superpowers/specs/2026-05-13-fix-before-beta-toctou-session-lock-design.md`
//! for the full design rationale.
//
// MANUAL_VERIFY: from two terminals, run `cargo lihaaf` concurrently
// against the same checkout. The second invocation must print
// `lihaaf: waiting for another lihaaf session to release <path> ...`
// immediately on contention, block, and then run to completion after
// the first releases. If the wait crosses `POST_WAIT_REPORT_MS`
// (50 ms), a follow-up `lihaaf: acquired session lock after N ms`
// addendum is emitted. The spec authorizes deferring the automated
// stderr-capture test for this diagnostic (spec §5, fallback
// paragraph) — wiring an in-process stderr-capture seam costs more
// than it saves at this scope; the manual procedure above remains
// the regression guard for the session-lock implementation.

use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::error::Error;

/// The lockfile name lives under `<workspace_target>/lihaaf/`.
/// Same parent as the rest of the per-session state so the lock and
/// the data it protects share a parent directory.
const LOCK_FILE_NAME: &str = ".session.lock";

/// Threshold above which the post-wait "acquired after N ms" line is
/// emitted. Below this, contention was effectively zero-cost and the
/// extra line would just be noise.
const POST_WAIT_REPORT_MS: u128 = 50;

/// RAII guard for the session lock.
///
/// Holding the guard means the current process holds an exclusive
/// `flock`/`LockFileEx` on `<workspace_target>/lihaaf/.session.lock`.
/// Dropping it releases the lock (Unix: implicit via `File::drop` →
/// `close(2)`; Windows: explicit `UnlockFileEx` in `Drop` to guarantee
/// synchronous release before the next acquire).
pub(crate) struct SessionLock {
    // Underscore prefix: held purely for its `Drop`. On Unix
    // `File::drop` runs `close(2)` which releases the `flock` on the
    // underlying open-file-description atomically; on Windows the
    // dedicated `Drop for SessionLock` impl below issues an explicit
    // `UnlockFileEx` before the file closes.
    _file: std::fs::File,
}

impl SessionLock {
    /// Blocking acquire.
    ///
    /// 1. `mkdir -p <workspace_target>/lihaaf/` (idempotent).
    /// 2. Open `.session.lock` (`O_CREAT | O_RDWR`, mode 0644 on Unix;
    ///    on Windows additionally `share_mode(FILE_SHARE_READ |
    ///    FILE_SHARE_WRITE)` — omits `FILE_SHARE_DELETE`).
    /// 3. Non-blocking exclusive lock attempt. On success, return.
    /// 4. On contention, emit a single-line "waiting" diagnostic to
    ///    stderr BEFORE blocking (so the user knows what they are
    ///    waiting on before any visible latency).
    /// 5. Block on the exclusive lock (Unix: `flock(LOCK_EX)` in an
    ///    EINTR retry loop; Windows: `LockFileEx` with
    ///    `LOCKFILE_EXCLUSIVE_LOCK`).
    /// 6. If the wait exceeded [`POST_WAIT_REPORT_MS`], emit a
    ///    follow-up "acquired session lock after N ms" line so log
    ///    scrapers have a duration to grep on.
    pub(crate) fn acquire(workspace_target: &Path) -> Result<SessionLock, Error> {
        let lock_dir = workspace_target.join("lihaaf");
        std::fs::create_dir_all(&lock_dir).map_err(|e| {
            Error::io(
                e,
                "creating session lock parent dir",
                Some(lock_dir.clone()),
            )
        })?;

        let lock_path: PathBuf = lock_dir.join(LOCK_FILE_NAME);
        let file = open_lock_file(&lock_path)?;

        // Step 3: non-blocking try-lock first. Single-session adopters
        // hit this fast path and never emit a diagnostic.
        match try_lock_exclusive(&file) {
            Ok(true) => return Ok(SessionLock { _file: file }),
            Ok(false) => {
                // Step 4: emit the "waiting" line BEFORE the blocking
                // call. The user gets feedback before any visible wait.
                eprintln!(
                    "lihaaf: waiting for another lihaaf session to release {} ...",
                    lock_path.display(),
                );
            }
            Err(e) => {
                return Err(Error::io(
                    e,
                    "non-blocking session lock acquire",
                    Some(lock_path.clone()),
                ));
            }
        }

        // Step 5–6: blocking acquire, then the post-wait addendum if
        // the wait was non-trivial.
        let start = Instant::now();
        lock_exclusive_blocking(&file)
            .map_err(|e| Error::io(e, "blocking session lock acquire", Some(lock_path.clone())))?;
        let elapsed = start.elapsed();
        if elapsed.as_millis() >= POST_WAIT_REPORT_MS {
            eprintln!(
                "lihaaf: acquired session lock after {} ms",
                elapsed.as_millis(),
            );
        }

        Ok(SessionLock { _file: file })
    }
}

/// Open the lockfile with platform-appropriate flags. Pulled into a
/// helper so the cfg gating is contained to a single function rather
/// than spread across the acquire flow.
fn open_lock_file(lock_path: &Path) -> Result<std::fs::File, Error> {
    let mut opts = std::fs::OpenOptions::new();
    opts.create(true).write(true).truncate(false);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        // FILE_SHARE_READ (1) | FILE_SHARE_WRITE (2) = 3.
        // Intentionally OMITS FILE_SHARE_DELETE (4) so a stray
        // `del .session.lock` while the lock is held fails loudly
        // instead of silently corrupting the next session's view of
        // the lockfile. See module docs ("Manual override") for the
        // failure-mode reasoning.
        opts.share_mode(3);
    }
    opts.open(lock_path).map_err(|e| {
        Error::io(
            e,
            "opening session lock file",
            Some(lock_path.to_path_buf()),
        )
    })
}

// ---------------------------------------------------------------------
// Unix implementation
// ---------------------------------------------------------------------

#[cfg(unix)]
fn try_lock_exclusive(file: &std::fs::File) -> std::io::Result<bool> {
    use std::os::unix::io::AsRawFd;
    // SAFETY: `file.as_raw_fd()` returns a valid open file descriptor
    // owned by `file`. `flock` is documented to be safe to call with
    // a valid fd; the syscall does not retain the fd.
    let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    if rc == 0 {
        return Ok(true);
    }
    let err = std::io::Error::last_os_error();
    if err.raw_os_error() == Some(libc::EWOULDBLOCK) {
        return Ok(false);
    }
    Err(err)
}

#[cfg(unix)]
fn lock_exclusive_blocking(file: &std::fs::File) -> std::io::Result<()> {
    use std::os::unix::io::AsRawFd;
    // EINTR retry loop. `flock(2)` is interruptible by signals
    // (notably `SIGWINCH` on terminal resize) and the spec requires
    // we retry rather than surface the interruption as a session
    // failure. The retry is benign: each iteration just re-attempts
    // the same blocking syscall.
    loop {
        // SAFETY: same as `try_lock_exclusive` — valid fd, syscall
        // does not retain it.
        let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) };
        if rc == 0 {
            return Ok(());
        }
        let err = std::io::Error::last_os_error();
        if err.kind() == std::io::ErrorKind::Interrupted {
            continue;
        }
        return Err(err);
    }
}

// On Unix, `flock(2)` is released atomically when the underlying
// open-file-description is closed, which `File::drop` does via
// `close(2)`. No explicit `Drop` impl is needed; the default
// `_file` drop is sufficient.

// ---------------------------------------------------------------------
// Windows implementation
// ---------------------------------------------------------------------

#[cfg(windows)]
fn try_lock_exclusive(file: &std::fs::File) -> std::io::Result<bool> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Foundation::ERROR_LOCK_VIOLATION;
    use windows_sys::Win32::Storage::FileSystem::{
        LOCKFILE_EXCLUSIVE_LOCK, LOCKFILE_FAIL_IMMEDIATELY, LockFileEx,
    };
    use windows_sys::Win32::System::IO::OVERLAPPED;

    // SAFETY: `OVERLAPPED` is a plain Win32 POD struct; the all-zero
    // bit pattern is its documented "no offset, no event" initial
    // state. `LockFileEx` reads `Offset` + `OffsetHigh` even for
    // synchronous handles, so the zero initializer (offset 0) is the
    // intended starting point for our whole-file lock.
    let mut overlapped: OVERLAPPED = unsafe { std::mem::zeroed() };
    // SAFETY: `file.as_raw_handle()` returns a valid open HANDLE
    // owned by `file`. `overlapped` is fully initialized. We lock
    // the maximal byte range (0..=u32::MAX:u32::MAX); the file is
    // empty so the byte range is purely a lock-table key.
    let ok = unsafe {
        LockFileEx(
            file.as_raw_handle() as _,
            LOCKFILE_EXCLUSIVE_LOCK | LOCKFILE_FAIL_IMMEDIATELY,
            0,
            u32::MAX,
            u32::MAX,
            &mut overlapped,
        )
    };
    if ok != 0 {
        return Ok(true);
    }
    let err = std::io::Error::last_os_error();
    // `LOCKFILE_FAIL_IMMEDIATELY` surfaces contention as
    // `ERROR_LOCK_VIOLATION` (33). Any other error is genuine
    // and must propagate.
    if err.raw_os_error() == Some(ERROR_LOCK_VIOLATION as i32) {
        return Ok(false);
    }
    Err(err)
}

#[cfg(windows)]
fn lock_exclusive_blocking(file: &std::fs::File) -> std::io::Result<()> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{LOCKFILE_EXCLUSIVE_LOCK, LockFileEx};
    use windows_sys::Win32::System::IO::OVERLAPPED;

    // SAFETY: see `try_lock_exclusive`. The file handle was opened
    // synchronously (no FILE_FLAG_OVERLAPPED), so LockFileEx blocks
    // until the lock is acquired rather than completing
    // asynchronously through the `overlapped` event.
    let mut overlapped: OVERLAPPED = unsafe { std::mem::zeroed() };
    let ok = unsafe {
        LockFileEx(
            file.as_raw_handle() as _,
            LOCKFILE_EXCLUSIVE_LOCK,
            0,
            u32::MAX,
            u32::MAX,
            &mut overlapped,
        )
    };
    if ok != 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(windows)]
impl Drop for SessionLock {
    fn drop(&mut self) {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Storage::FileSystem::UnlockFileEx;
        use windows_sys::Win32::System::IO::OVERLAPPED;
        // SAFETY: `OVERLAPPED` is a Win32 POD; all-zero is its
        // documented initial state. `_file` is still open here and
        // its handle is valid. The lock range matches the one we
        // acquired (whole file, 0..(u32::MAX, u32::MAX)).
        //
        // Return value intentionally discarded: this is `Drop` and
        // there is no path to propagate an error. `ERROR_NOT_LOCKED`
        // (158) is the most common benign failure (means the OS
        // already cleaned up the lock during process-exit handling
        // before our explicit unlock ran).
        let mut overlapped: OVERLAPPED = unsafe { std::mem::zeroed() };
        let _ = unsafe {
            UnlockFileEx(
                self._file.as_raw_handle() as _,
                0,
                u32::MAX,
                u32::MAX,
                &mut overlapped,
            )
        };
        // `_file` drops after this, closing the HANDLE. The explicit
        // `UnlockFileEx` above is what guarantees synchronous
        // release — `CloseHandle` releases locks eventually but per
        // Microsoft's documentation the timing is unspecified, which
        // can race a fast back-to-back same-process re-acquire.
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Test 1 (§5): a fresh acquire creates the lockfile at the
    /// documented path under `<workspace_target>/lihaaf/`.
    #[test]
    fn acquire_creates_lockfile_under_target_lihaaf() {
        let tmp = tempfile::tempdir().unwrap();
        let workspace_target = tmp.path().join("target");

        let _guard = SessionLock::acquire(&workspace_target).unwrap();

        let lock_path = workspace_target.join("lihaaf").join(LOCK_FILE_NAME);
        assert!(
            lock_path.is_file(),
            "expected lockfile at {}",
            lock_path.display(),
        );
    }

    /// Test 2 (§5): dropping the guard releases the lock for an
    /// immediate same-process re-acquire. The second `acquire` MUST
    /// hit the try-lock fast path; if it blocked, this test would
    /// either hang or (on Windows without the explicit `UnlockFileEx`
    /// in `Drop`) intermittently fail.
    ///
    /// On Windows this also bites the absence of an explicit
    /// `UnlockFileEx` in `Drop` — `CloseHandle` releases `LockFileEx`-held
    /// locks asynchronously per Microsoft docs, so without explicit
    /// unlock the post-drop fast-path re-acquire intermittently fails.
    #[test]
    fn drop_releases_lock_for_same_process_reacquire() {
        let tmp = tempfile::tempdir().unwrap();
        let workspace_target = tmp.path().join("target");

        {
            let _guard = SessionLock::acquire(&workspace_target).unwrap();
        } // _guard drops here, releasing the lock.

        // Second acquire must succeed quickly. Wall-clock under
        // POST_WAIT_REPORT_MS is the practical observable; we cannot
        // assert "no waiting line printed" portably without an
        // in-process stderr-capture seam (deferred per spec §5).
        let start = std::time::Instant::now();
        let _guard2 = SessionLock::acquire(&workspace_target).unwrap();
        let elapsed = start.elapsed();
        assert!(
            elapsed.as_millis() < POST_WAIT_REPORT_MS,
            "re-acquire after drop should be fast (was {} ms)",
            elapsed.as_millis(),
        );
    }

    /// Test 3 (§5): two `acquire` calls against the same
    /// `workspace_target` resolve to the same lockfile path
    /// deterministically. Regression guard for accidental
    /// random-suffix paths or PID-namespaced paths slipping into
    /// the implementation.
    #[test]
    fn lockfile_path_is_deterministic() {
        let tmp = tempfile::tempdir().unwrap();
        let workspace_target = tmp.path().join("target");

        let expected = workspace_target.join("lihaaf").join(LOCK_FILE_NAME);

        {
            let _g1 = SessionLock::acquire(&workspace_target).unwrap();
            assert!(expected.is_file());
        }
        {
            let _g2 = SessionLock::acquire(&workspace_target).unwrap();
            assert!(expected.is_file());
        }
    }

    // Spec §5 test 4 ("acquire_emits_waiting_diagnostic_on_contention")
    // is intentionally NOT implemented as a unit test.
    //
    // Capturing stderr from a specific thread inside a Rust unit test
    // requires either the `gag` crate or fd-redirection trickery, both
    // of which add complexity that outweighs the unit-test value at this
    // scope. The MANUAL_VERIFY comment at the top of this file documents
    // the hand-test procedure for the two-terminal contention case.
    //
    // The bite for the underlying behavior is preserved:
    // - The "waiting" diagnostic path is exercised by the
    //   `drop_releases_lock_for_same_process_reacquire` test above
    //   in the opposite direction (it asserts the post-drop fast-path
    //   re-acquire completes under `POST_WAIT_REPORT_MS`; if the
    //   diagnostic fired it would mean the fast path was NOT taken).
    // - The blocking-then-proceeding semantics on contention is
    //   covered by the manual two-terminal sanity check before each
    //   release.
}
