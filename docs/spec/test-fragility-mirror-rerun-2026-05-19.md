# Diagnostic analysis: mirror_upstream_rerun_reconciles_stale_entries CI flake

**Date:** 2026-05-19
**Test:** `compat::overlay::tests::mirror_upstream_rerun_reconciles_stale_entries`
**Location:** `src/compat/overlay.rs:6757`
**Status:** Root cause confirmed. Fix options ranked below.

---

## 1. Failure signature

CI (GH Actions `ubuntu-latest`) at commit `895454b`:

```
assertion `left != right` failed: CASE 12: reintroduced wrong-target src/ must be re-created (CASE 3 reconcile)
  left: 8396849
 right: 8396849
```

The `assert_ne!(src_ino_before, src_ino_after)` at `overlay.rs:6757` fails because both sides
resolve to the same inode number. The test passes locally on WSL2 with the same commit.

---

## 2. Root cause confirmation

### Hypothesis: CONFIRMED — LIFO inode recycling on tmpfs/ext4 under GH Actions runner

**The exact inode lifecycle in CASE 12 (`overlay.rs:6715-6760`):**

1. **Line 6718** — `src_ino_before` is read: inode of the *canonical* `src/` symlink created by
   the first mirror run. Call this inode **A**.

2. **Line 6732** — `std::fs::remove_file(staged_overlay_dir.join("src"))` — inode **A** is freed.
   On Linux ext4/tmpfs with a LIFO inode freelist, **A** is immediately at the top of the free
   pool.

3. **Line 6734** — `std::os::unix::fs::symlink(unrelated2.path(), staged_overlay_dir.join("src"))`
   — a new wrong-target symlink is created. The kernel hands out inode **A** again (LIFO: last
   freed = first allocated).

4. **`mirror_upstream_into_overlay` (line 6737)** — `reconcile_one_entry` hits the CASE 3 branch
   (`overlay.rs:3562-3564`):
   ```rust
   std::fs::remove_file(staged_path)           // frees inode A again
       .map_err(|e| mirror_err("stale-symlink-unlink", e))?;
   create_canonical_mirror(upstream_path, staged_path)  // immediately allocates next free
   ```
   The new canonical `src/` symlink is created immediately after freeing inode **A**. On a
   LIFO freelist, **A** is again at the top — the new symlink receives inode **A**.

5. **Line 6756** — `src_ino_after` = inode **A**. The assertion `assert_ne!(A, A)` fails.

**Why WSL2 doesn't reproduce this:** WSL2's VirtIO-9P and Plan9 filesystem layers, combined with
its userspace inode numbering, do not implement simple LIFO recycling. The inode sequence is
monotonically incrementing or hashed from path components depending on the WSL2 FS driver in use.
Consecutive `unlink` + `symlink` on the same path yields a different number. On a native Linux
runner (ubuntu-latest uses an EC2/Azure VM with ext4 or a tmpfs-backed `/tmp`), LIFO recycling is
the default kernel behavior.

**Key code evidence:**

- `reconcile_one_entry` at `overlay.rs:3562-3564`:
  the `remove_file` and the immediately-following `create_canonical_mirror → symlink_platform`
  are back-to-back with no intervening allocations, giving the kernel maximum opportunity to recycle.
- The test's "before" and "after" frames both measure the same inode slot:
  `overlay.rs:6718` (before, reads inode A of canonical symlink),
  `overlay.rs:6756` (after, reads inode A of recreated canonical symlink).
  Inode A is freed and reallocated twice between those two lines.

**Filesystems where this is known to recycle:**
- ext4 (standard Linux block device): LIFO freelist in inode allocator since kernel 2.6
- tmpfs (used by many runners for `/tmp`): inode numbers are sequential / monotone *within a
  mount instance* but reuse freed numbers as the counter wraps or when the allocator recycles.
  On a fresh tmpfs with few inodes allocated the sequence looks monotone; under churn (many
  alloc/free cycles in a short window) recycling is observable, especially when the same
  directory's slot is freed and immediately re-requested.

**Note on the CI runner environment:**  
The workflow (`ci.yml:19`) uses `runs-on: ubuntu-latest`. GH Actions ubuntu-latest runners use
a Linux VM. `tempfile::tempdir()` creates directories under `$TMPDIR` or `/tmp`, which on GH
Actions runners is typically on the same ext4 filesystem as the OS — not a separate tmpfs mount.
On ext4, the block-group inode allocator favors locality and LIFO recycling within the same
directory group, making same-directory unlink+symlink very likely to recycle the same inode slot.

---

## 3. Sibling-test fragility audit

**Count of inode-comparison sites in `src/compat/overlay.rs`:**

```
overlay.rs:6242  — apply_self_patch_idempotent_second_materialize, src_ino_before
overlay.rs:6245  — apply_self_patch_idempotent_second_materialize, include_ino_before
overlay.rs:6260  — apply_self_patch_idempotent_second_materialize, src_ino_after
overlay.rs:6263  — apply_self_patch_idempotent_second_materialize, include_ino_after
overlay.rs:6720  — mirror_upstream_rerun_reconciles_stale_entries, src_ino_before     ← FAILING
overlay.rs:6723  — mirror_upstream_rerun_reconciles_stale_entries, inc_ino_before
overlay.rs:6726  — mirror_upstream_rerun_reconciles_stale_entries, build_ino_before
overlay.rs:6742  — mirror_upstream_rerun_reconciles_stale_entries, inc_ino_after
overlay.rs:6745  — mirror_upstream_rerun_reconciles_stale_entries, build_ino_after
overlay.rs:6756  — mirror_upstream_rerun_reconciles_stale_entries, src_ino_after      ← FAILING
```

**Fragility classification per assertion type:**

| Assertion | Location | Direction | Inode freed between before/after? | Fragile? |
|---|---|---|---|---|
| `assert_ne!(src_ino_before, src_ino_after)` | `overlay.rs:6757` | recreate | YES (freed at 6732, freed again in reconcile) | **YES — failing** |
| `assert_eq!(inc_ino_before, inc_ino_after)` | `overlay.rs:6746` | skip | NO (never unlinked) | NO |
| `assert_eq!(build_ino_before, build_ino_after)` | `overlay.rs:6750` | skip | NO (never unlinked) | NO |
| `assert_eq!(src_ino_before, src_ino_after)` | `overlay.rs:6264` | skip | NO (CASE 2, never unlinked) | NO |
| `assert_eq!(include_ino_before, include_ino_after)` | `overlay.rs:6268` | skip | NO (CASE 2, never unlinked) | NO |

**Result: 1 fragile assertion, 4 stable assertions.**

The "skip preserves inode" assertions (CASE 2 / idempotency) are NOT fragile — they check
stability of an inode that is never freed between measurement points. The LIFO recycling risk
is zero when no `unlink` occurs. These assertions can remain inode-based.

The "recreate changes inode" assertion is the sole fragile site: it captures an inode, frees
the path, recreates the path, and asserts the new inode differs. That premise is not guaranteed
by POSIX.

**Sibling tests at risk:** 1 (`mirror_upstream_rerun_reconciles_stale_entries` CASE 12 only;
no other test has a `assert_ne!` inode comparison after a delete-and-recreate).

---

## 4. Fix options (ranked)

### Option 1: Replace `assert_ne!(ino)` with `assert_eq!(read_link(...), upstream_path)` (recommended)

**One-line description:** After the second mirror run, assert via `std::fs::read_link` that `src/`
now targets `upstream_dir.join("src")` — the canonical target — instead of the wrong target.

**What it asserts instead of inode-inequality:**
- `src_ino_before` / `src_ino_after` variables: removed entirely, not measured.
- Replacement assertion (after line 6757):
  ```rust
  let src_target_after =
      std::fs::read_link(staged_overlay_dir.join("src")).expect("readlink src after rerun");
  assert_eq!(
      src_target_after,
      upstream_dir.join("src"),
      "CASE 12: reintroduced wrong-target src/ must be re-created with canonical target (CASE 3 reconcile)"
  );
  ```
  Optionally also assert `is_symlink()` on the `symlink_metadata` if belt-and-suspenders is desired.

**How it preserves the test's original intent:**
The test must prove that CASE 3 reconciliation *replaced the wrong-target symlink with the correct
one*. Reading the symlink target directly verifies exactly that — the wrong-target pointed at
`unrelated2.path()`, the correct target is `upstream_dir.join("src")`. If `reconcile_one_entry`
silently skipped the CASE 3 path (bug), the symlink would still point at `unrelated2.path()` and
the assertion would fail correctly. If the CASE 3 path ran but wrote the wrong target (different
bug), the assertion would also fail correctly.

This is strictly more specific than inode-inequality: inode inequality implies *something* changed;
target equality implies *the right thing* was written.

**Trade-offs:**
- False-negative risk: NONE. If reconcile skips the stale entry, `read_link` returns the wrong
  target and the assertion fails. This is exactly the bug the test is guarding against.
- False-positive risk: NONE. A different path could in theory produce the correct target string
  by coincidence, but `upstream_dir.join("src")` is the unique canonical target for this test.
- Complexity: minimal — `read_link` is already used at lines 6670 and 6698 in the same test for
  the first mirror run's CASE 3 and CASE 6 checks. This is a parallel assertion for the rerun.
- Sibling implications: none. The CASE 2 "skip" assertions (inc_ino, build_ino) remain inode-based
  and continue to be correct (no fragility there).
- `src_ino_before` / `src_ino_after` variable declarations and reads can be deleted outright.
  The three `ino()` captures in the CASE 12 block (lines 6718-6726) reduce to two (inc and build).

**Fix size: S** — delete 6 lines (src_ino_before capture + src_ino_after capture), replace the
`assert_ne!` block with a `read_link` + `assert_eq!` block.

---

### Option 2: Assert `is_symlink() && read_link() == upstream_path` (belt-and-suspenders)

**One-line description:** Same as Option 1 but also assert `is_symlink()` on the result, mirroring
the structure of the first-run CASE 3 assertion at lines 6666-6675.

**What it asserts:** Both that the entry is a symlink type AND that the target is the canonical
upstream path. Two assertions instead of one.

**How it preserves intent:** Identical to Option 1 for the target-check. The extra `is_symlink()`
guards against a future regression where `create_canonical_mirror` falls back to `copy_fallback`
and produces a real directory instead of a symlink on the second run — a scenario not covered by
target-string comparison alone (since `read_link` would error, not return the wrong target, in that
case, so the test would still fail, just with a different error message).

**Trade-offs:**
- Marginally better diagnostic output on `copy_fallback` regression: the `is_symlink()` assertion
  fires before `read_link` fails with `EINVAL`.
- Slightly more code than Option 1. The existing pattern at lines 6666-6675 means it is not novel.

**Fix size: S** — same as Option 1 plus one `assert!` line.

---

### Option 3: Assert symlink target changed from wrong-target to canonical target

**One-line description:** Read the symlink target *before* the second mirror run (to confirm the
wrong-target is present) and again after (to confirm the canonical target was written). Assert
`before_target != upstream_path && after_target == upstream_path`.

**What it asserts:** A two-sided target assertion: the wrong-target was actually in place before
the mirror, and the canonical target is in place after.

**How it preserves intent:** The "before" half proves the test setup (wrong-target was seeded
correctly); the "after" half proves reconciliation. Together they form the same logical claim as
Option 1's single assertion but with an explicit setup-validation step.

**Trade-offs:**
- Adds a `read_link` call before the `mirror_upstream_into_overlay` invocation. The test already
  reads the wrong-target symlink's metadata at line 6718 (for `src_ino_before`); the `read_link`
  would replace that `ino()` capture.
- More robust setup validation: if the wrong-target seeding at line 6734 failed silently in a
  hypothetical bug, the "before_target != upstream_path" assertion would catch it.
- Slightly more code than Option 1.

**Fix size: S** — comparable to Option 1 but replaces the `ino()` capture with a `read_link`.

---

### Option 4: Keep inode check, add `read_link` as mandatory secondary assertion

**One-line description:** Retain `assert_ne!(src_ino_before, src_ino_after)` but convert it to
`assert!(... || ...)` combined with `read_link` equality, so target correctness is always checked
regardless of inode recycling.

**What it asserts:**
```rust
let src_target_after = std::fs::read_link(...).expect("readlink");
// Inode may or may not have been recycled; what matters is the target.
assert_eq!(src_target_after, upstream_dir.join("src"), "CASE 12 target must be canonical");
// Retain inode inequality as an informational (non-blocking) signal.
// (In practice: remove the assert_ne entirely since it's fragile.)
```

Actually this collapses to Option 1 — the inode inequality provides no additional safety once
`read_link` equality is asserted. There is no correct behavior that passes `read_link` equality
and fails `assert_ne!` inode, nor vice versa (in the normal case). This option has no net benefit.

**Trade-offs:** Adds noise without correctness gain. `assert_ne!` would still be fragile on CI.
**Recommend against.** Included for completeness.

**Fix size: S** (but not recommended).

---

### Option 5: mtime/ctime comparison as a secondary guard

**One-line description:** Compare `modified()` timestamps before and after the second mirror run
to detect recreation.

**What it asserts:** `mtime_before != mtime_after` for `src/`.

**Why not recommended:**
- Clock resolution on Linux is 1 second for `mtime` in many contexts (FAT-era compat). On ext4
  with `relatime` mount option, mtime is only updated on access after the last modification, not
  on every access. A fast test could produce equal timestamps.
- `std::time::SystemTime` on Linux has nanosecond resolution via `statx`, but `metadata.modified()`
  in Rust maps to `stat.st_mtime` which is 1-second granular on many configurations.
- This trades one environmental dependency (inode allocation policy) for another (clock resolution).
  Strictly worse than target comparison.

**Fix size: S** — but fragile in a different dimension.

---

## 5. Recommendation

**Option 1: Replace `assert_ne!(ino)` with `assert_eq!(read_link(...), upstream_path)`.**

Rationale:

1. **Direct specification alignment.** CASE 3's contract is "wrong-target symlink is replaced
   with a canonical-target symlink pointing at the upstream path." `read_link() == upstream_path`
   asserts that contract verbatim. Inode inequality asserts a proxy property that does not appear
   in the spec.

2. **Precedent already in the test.** Lines 6670-6675 already assert `read_link() == upstream_dir.join("src")` for the first mirror run. The second-run assertion should mirror this
   structure exactly for consistency.

3. **Zero environmental dependency.** `read_link` returns the literal string stored in the
   symlink dentry. No kernel allocator, no FS type, no clock resolution involved.

4. **Minimal diff.** Remove `src_ino_before` capture (6718-6720), remove `src_ino_after` capture
   (6754-6756), replace `assert_ne!` block (6757-6760) with a `read_link` + `assert_eq!`. Net
   change: ~6 lines removed, ~4 lines added.

5. **Does not affect the CASE 2 "skip" assertions** (`inc_ino` and `build_ino`). Those are
   inode-stability checks on non-deleted entries — they are correct and should remain inode-based
   since `read_link` on skipped entries would also pass trivially and provides no additional
   discriminating power over the existing skip assertions.

Optional enhancement (Option 2): also add `assert!(symlink_metadata(...).file_type().is_symlink())`
before the `read_link` call, to mirror the style of lines 6666-6668 and provide better error
messages if the copy fallback path produces a directory instead of a symlink. Adds one line.
The implementer should use their judgment — Option 1 alone is sufficient.

---

## 6. Scope for follow-up dispatch

**The fix is fully scoped to a single test function in `src/compat/overlay.rs`.**

Dispatch `careful-coder-sonnet` with:
- File: `src/compat/overlay.rs`
- Function: `mirror_upstream_rerun_reconciles_stale_entries`
- Change: CASE 12 block only (lines 6715-6760)
- Action:
  1. Remove `src_ino_before` / `src_ino_after` variable declarations and reads (lines 6718-6720
     and 6754-6756).
  2. Replace `assert_ne!(src_ino_before, src_ino_after, ...)` (lines 6757-6760) with:
     ```rust
     let src_target_after =
         std::fs::read_link(staged_overlay_dir.join("src")).expect("readlink src after rerun");
     assert_eq!(
         src_target_after,
         upstream_dir.join("src"),
         "CASE 12: reintroduced wrong-target src/ must be re-created with canonical target (CASE 3 reconcile)"
     );
     ```
  3. Optionally add `assert!(is_symlink())` before the `read_link` call (Option 2 enhancement).
  4. Retain the `inc_ino_before`/`inc_ino_after` and `build_ino_before`/`build_ino_after`
     captures and their `assert_eq!` assertions unchanged.
  5. Verify with `cargo test --lib mirror_upstream_rerun_reconciles_stale_entries`.
  6. Run `cargo fmt --all -- --check` and `cargo clippy --all-targets -- -D warnings` to confirm
     no new lint or formatting regressions.

**No production source changes required.** The underlying `reconcile_one_entry` and
`create_canonical_mirror` implementations are correct. The bug is exclusively in the test's
choice of assertion proxy.

**No sibling tests require changes.** All other inode-comparison assertions are "skip preserves
inode" (equality on non-deleted entries) and are not fragile.
