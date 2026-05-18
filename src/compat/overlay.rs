//! Phase 2 of compat mode (issue #11) — staged overlay manifest generator.
//!
//! Reads the upstream `Cargo.toml`, canonicalizes `[lib] crate-type` so the
//! lihaaf stage-3 dylib build can succeed without mutating the upstream
//! file, and writes the result to
//! `<upstream_dir>/target/lihaaf-overlay/Cargo.toml`.
//!
//! ## Why `target/lihaaf-overlay/Cargo.toml` and not `Cargo.lihaaf.toml`
//!
//! `cargo rustc --manifest-path` requires the target filename to be
//! literally `Cargo.toml`; any other filename is rejected with exit code 1
//! before cargo does any work. Staging the overlay under
//! `target/lihaaf-overlay/` satisfies that constraint while keeping the
//! file isolated from the upstream `Cargo.toml`. The `target/` subtree is
//! treated as implicitly ignored by the cleanup classifier
//! ([`crate::compat::cleanup::CleanupGuard`]), so the overlay never
//! pollutes the fork's worktree regardless of `.gitignore` state.
//!
//! Per `docs/compatibility-plan.md` §3.2.3, the overlay is:
//!
//! 1. Re-serialized through the existing `toml = "1"` dependency. No
//!    second TOML crate is introduced (the v0.1 surface forbids
//!    `toml_edit` — that's a v0.2 conversation).
//! 2. Written with table keys in cargo's canonical order: `package`,
//!    `lib`, `bin`, `dependencies`, `dev-dependencies`,
//!    `build-dependencies`, `features`, `workspace`, then alphabetical
//!    for the long tail.
//! 3. Stripped of comments. The `toml` crate's `Value` data model drops
//!    comments on parse; an upstream-text scan recovers them for the
//!    §3.3 envelope's `overlay.dropped_comments` field.
//! 4. Always LF line endings, never CRLF. No trailing whitespace.
//! 5. Idempotent: a second run from the same input produces a
//!    byte-identical output, and the write is skipped (preserving mtime)
//!    when the existing sibling matches.
//!
//! The atomic write reuses [`crate::util::write_file_atomic`] so the
//! staged overlay is either fully written or absent — a SIGKILL mid-write
//! cannot leave a half-formed file for the stage-3 `cargo rustc` to choke
//! on.
//!
//! ## Crate-type canonicalization
//!
//! `[lib] crate-type` is the only field the overlay modifies. The
//! semantics:
//!
//! | Input                                       | Output                                       |
//! |---------------------------------------------|----------------------------------------------|
//! | absent                                      | `["dylib", "rlib"]`                          |
//! | `["rlib"]`                                  | `["dylib", "rlib"]`                          |
//! | `["dylib"]`                                 | `["dylib", "rlib"]` (rlib appended)          |
//! | `["dylib", "rlib"]`                         | unchanged                                    |
//! | `["cdylib"]`                                | `["dylib", "rlib", "cdylib"]`                |
//! | `["rlib", "staticlib"]`                     | `["dylib", "rlib", "staticlib"]`             |
//!
//! `rlib` is retained on every output shape so the non-lihaaf
//! `cargo test` baseline (§3.4) keeps working. Other entries
//! (`cdylib`, `staticlib`, etc.) are preserved verbatim AFTER the
//! `dylib`/`rlib` pair, in their original order.
//!
//! ## What the overlay does and does NOT touch in `[patch]`
//!
//! - `[patch.<registry>.X]` `git`, `branch`, `tag`, `rev` keys — these
//!   identify a remote source and must pass through verbatim.  The spec
//!   (§3.2.3 risks section) is explicit that `[patch]` cannot add crate-type
//!   and the overlay code must not rewrite those fields.
//! - The `path` sub-key inside a `[patch.<registry>.X]` entry IS rewritten
//!   with the same absolutization semantics as `[dependencies.X].path`.
//!   Without this, a fork that carries `cxx = { path = "." }` in
//!   `[patch.crates-io]` would point cargo at the staged manifest dir after
//!   overlay materialization — either a self-reference or a nonexistent path.
//! - Every other top-level table (`dependencies`, `dev-dependencies`,
//!   `features`, `[[bin]]`, …) is preserved as parsed.
//!
//! ## Workspace-inheritance override (selective `[workspace]` rewrite)
//!
//! The staged overlay always carries a `[workspace]` table, regardless
//! of whether the upstream manifest declared one — but the rewrite is
//! SELECTIVE, not a full clobber. We keep every workspace-inheritance
//! TABLE the upstream declared (`workspace.dependencies`,
//! `workspace.package`, `workspace.lints`, `workspace.metadata`,
//! `workspace.resolver`, plus any future `[workspace.*]` cargo adds)
//! and strip only the MEMBERSHIP keys (`members`, `exclude`,
//! `default-members`). This is the workspace-identity fix for the
//! v0.1.0-beta.5 regression on workspace-style pilots (see issue #36)
//! combined with the R2 follow-up that preserves inheritance for
//! manifests that use `{ workspace = true }` references (issue #38 /
//! PR #37 Codex + Gemini panel BLOCK).
//!
//! **Why a `[workspace]` table at all (cargo walk-up).** Cargo
//! determines a manifest's workspace root by walking UP the filesystem
//! from the manifest until it finds another `Cargo.toml` with a
//! `[workspace]` table. For the staged overlay at
//! `<upstream>/target/lihaaf-overlay/Cargo.toml`, that walk reaches
//! `<upstream>/Cargo.toml` — and for workspace-style pilots (cxx,
//! serde-json, thiserror) the upstream IS a workspace root. Cargo
//! then tries to attach the overlay's package to the upstream
//! workspace, but the overlay's package name isn't in the upstream's
//! `members` array. Result: `package <X>/Cargo.toml is a member of the
//! wrong workspace` and the build fails. Declaring the overlay as its
//! own workspace root (any `[workspace]` table, even empty) makes
//! cargo stop the walk-up at the overlay manifest.
//!
//! **Why the inheritance tables are preserved (`{ workspace = true }`).**
//! Cargo's workspace-inheritance feature lets a member crate write
//! `[dependencies] foo = { workspace = true }` and inherit the actual
//! version/path/features from `[workspace.dependencies.foo]` on the
//! workspace root. The same pattern exists for `[package]
//! version.workspace = true` (inherits from `[workspace.package]`),
//! `[lints] rust.workspace = true` (inherits from `[workspace.lints]`),
//! `[dev-dependencies]`, `[build-dependencies]`, and
//! `[target.<cfg>.dependencies]`. If we clobber the upstream's
//! `[workspace.dependencies]` / `[workspace.package]` / `[workspace.lints]`
//! tables, any surviving `{ workspace = true }` reference in the
//! overlay manifest fails cargo's parser with `"workspace inheritance
//! was specified but [workspace.<X>] was not defined"`. R1
//! (v0.1.0-beta.6 attempt 1) clobbered these unconditionally and broke
//! every pilot fork that uses inheritance; R2 preserves them.
//!
//! **Why ONLY the membership keys are stripped.** If the overlay
//! claimed the upstream's `members = [...]` (even absolutized to abs
//! paths), the overlay AND the upstream would both claim those
//! path-dep crates as members → `package <X> is a member of the wrong
//! workspace`. Same trap for `exclude` and `default-members`.
//! Stripping these three keys leaves member-ownership exclusively with
//! the upstream workspace where it was originally declared.
//!
//! **Why unknown `[workspace.X]` tables pass through.** If cargo adds
//! a new `[workspace.<future>]` table in a later release, a hardcoded
//! preserve-list would silently drop it. The R2 implementation
//! preserves anything that is NOT one of the three membership keys, so
//! the overlay stays forward-compatible with future cargo additions.
//!
//! **Five branches of the override decision tree.** The
//! [`override_workspace_inheritance`] function classifies the upstream
//! manifest into one of five mutually-exclusive cases:
//!
//! 1. **Explicit workspace member** (`[package].workspace = "<path>"`):
//!    REJECTED with a directed diagnostic. The ancestor pointer
//!    declares the manifest as a member of an ancestor workspace; the
//!    overlay cannot self-declare as a workspace root and a member
//!    simultaneously. R1 silently stripped the pointer, which strands
//!    every surviving `{ workspace = true }` reference (the actual
//!    inheritance tables live in the ancestor). Copying the
//!    ancestor's tables down is out-of-scope for v0.1.0-beta.6 — see
//!    "Workspace-member cases are out-of-scope" in the function-level
//!    docs.
//!
//! 2. **Implicit workspace member via ancestor `Cargo.toml`**
//!    (no `[package].workspace`, no local `[workspace]`, AND any
//!    ancestor `Cargo.toml` on the filesystem walk-up carries
//!    `[workspace]`): REJECTED with a directed diagnostic naming the
//!    offending ancestor manifest path. This catches the case Codex
//!    flagged in PR #37 R3 review: an ancestor workspace carrying
//!    `[patch.crates-io]` / `[replace]` / `[profile]` / `resolver` /
//!    `[workspace.dependencies]` would change cargo's baseline
//!    resolution but the lihaaf overlay (which terminates cargo's
//!    walk-up at the staged manifest) would resolve against the
//!    REGISTRY versions of those deps — producing a divergent baseline
//!    vs. overlay graph and false compat verdicts. The R4 rejection
//!    runs even when the manifest has NO `{ workspace = true }`
//!    inheritance references; the ancestor-state divergence applies
//!    regardless of inheritance usage.
//!
//! 3. **Implicit workspace member via inheritance refs only**
//!    (no `[package].workspace`, no local `[workspace]`, no ancestor
//!    workspace detected, BUT one or more `{ workspace = true }`
//!    inheritance references present in `[package]` / `[dependencies]`
//!    / `[dev-dependencies]` / `[build-dependencies]` /
//!    `[target.<cfg>.<deps>]` / `[lints]`): REJECTED. R3 (PR #37
//!    R3) added this branch — the only way a manifest can carry
//!    `{ workspace = true }` references is if its workspace root
//!    lives elsewhere (either an ancestor we DIDN'T detect because
//!    it has no `Cargo.toml`, or a path-via-non-filesystem-walk that
//!    cargo somehow resolves). Without rejection the overlay would
//!    strand these refs at cargo parse time with the cryptic
//!    "workspace inheritance was specified but `[workspace.X]` was
//!    not defined" error.
//!
//! 4. **Workspace-root** (local `[workspace]` table present): the
//!    overlay CLONES the upstream's `[workspace]` table and strips
//!    only the MEMBERSHIP keys (`members`, `exclude`,
//!    `default-members`). Every inheritance table
//!    (`workspace.dependencies`, `workspace.package`, `workspace.lints`,
//!    `workspace.metadata`, `workspace.resolver`, plus any unknown
//!    `[workspace.X]` cargo may add in future releases) is preserved
//!    verbatim. This is the case the four Round-1 pilots (cxx,
//!    serde-json, anyhow, thiserror) all hit: each invokes lihaaf
//!    from the upstream ROOT, which carries both `[package]` and
//!    `[workspace]`.
//!
//! 5. **Standalone single-crate** (no local `[workspace]`, no
//!    inheritance refs, no ancestor workspace): the overlay INJECTS
//!    an empty `[workspace] = {}` so cargo terminates its walk-up at
//!    the staged manifest. This is the case for forks whose upstream
//!    `Cargo.toml` is a single-crate manifest with no workspace
//!    relationships.
//!
//! **R4 ancestor-walk: how it works.** When a manifest has no local
//! `[workspace]` and no `[package].workspace`, the override walks UP
//! the filesystem from the manifest's parent directory, checking each
//! ancestor directory for a `Cargo.toml`. If any ancestor `Cargo.toml`
//! parses as TOML AND contains a `[workspace]` table, branch 2 fires.
//! Unparseable ancestor manifests log a non-fatal warning and the walk
//! continues — we should not abort on a malformed ancestor manifest
//! the user does not control. I/O errors other than NotFound propagate
//! as `Error::Io`. The walk terminates at the filesystem root.
//!
//! **Why a CONSERVATIVE ancestor-rejection (any ancestor `[workspace]`,
//! not just one whose `members` claims the manifest).** Even when the
//! ancestor `[workspace]` does not name the descendant explicitly,
//! it can still carry `[patch.crates-io]`, `[replace]`, `[profile]`,
//! `resolver`, or `[workspace.dependencies]` tables that cargo applies
//! during dependency resolution from the descendant. The lihaaf overlay
//! at `<descendant>/target/lihaaf-overlay/Cargo.toml` declares
//! `[workspace]` so cargo stops the walk-up there, skipping the
//! ancestor's state entirely. The result: baseline `cargo test`
//! (from the descendant, walks up, applies the ancestor state) and
//! lihaaf overlay (terminates the walk-up at the overlay manifest,
//! does NOT apply the ancestor state) build against DIFFERENT
//! dependency graphs — producing false-positive and false-negative
//! compat results. Rejecting any ancestor workspace is the only
//! correct conservative behavior; a finer-grained check would require
//! reasoning about cargo's full resolution algorithm against the
//! ancestor's specific configuration, which is far more complex than
//! the value it adds for v0.1.0-beta.6.
//!
//! All four Round-1 pilots (cxx, serde-json, anyhow, thiserror) invoke
//! lihaaf from the upstream ROOT, which carries `[package]` +
//! `[workspace]` (case 4, workspace-root) — NOT from a sub-crate — so
//! none of cases 1, 2, or 3 affects any currently-enrolled pilot. The
//! ancestor-walk rejection is defense-in-depth for any future user
//! invoking lihaaf from a workspace-member sub-crate or a crate inside
//! a parent workspace tree: they get a clean diagnostic instead of a
//! cryptic cargo parse error OR a silent false compat verdict.

use std::path::{Path, PathBuf};

use crate::error::Error;
use crate::util;

/// One materialized overlay run. Constructed by [`materialize_overlay`]
/// after the sibling manifest is written (or skipped as idempotent).
///
/// The bundle is consumed by the §3.3 envelope writer — the
/// `overlay.generated` classification reads from this struct.
///
/// `pub` (with the parent module pinned at `pub(crate)`) so the crate
/// root can `#[doc(hidden)]` re-export this for the test crate. Not
/// part of any v0.1 stability contract.
#[derive(Debug)]
pub struct OverlayPlan {
    /// Path to the upstream `Cargo.toml` the overlay was derived from.
    pub upstream_manifest: PathBuf,
    /// Path to the staged overlay manifest. Always
    /// `<upstream_manifest_dir>/target/lihaaf-overlay/Cargo.toml` so that
    /// `cargo rustc --manifest-path` accepts the filename (cargo rejects
    /// any `--manifest-path` whose last component is not literally
    /// `Cargo.toml`).
    pub sibling_manifest: PathBuf,
    /// `true` when the upstream manifest already declared
    /// `[lib] crate-type = ["dylib", ...]`. The sibling is still
    /// written (idempotently) so the §3.3 envelope's
    /// `overlay.generated` classification is uniform; the flag lets the
    /// envelope record whether the dylib declaration was a real change
    /// or a redundant one.
    pub upstream_already_has_dylib: bool,
    /// Comment text dropped during canonicalization. The `toml` crate's
    /// `Value` model drops comments on parse, so the overlay code
    /// recovers them from the raw upstream bytes with a small
    /// state-machine scanner (no regex per spec §6.1) that tracks all
    /// four TOML string forms — basic, literal, multi-line basic, and
    /// multi-line literal — so a `#` inside any string is not surfaced
    /// here. Entries are stashed for the §3.3 envelope's
    /// `overlay.dropped_comments` field.
    ///
    /// Each entry is the raw comment text WITHOUT the leading `#` and
    /// WITHOUT surrounding whitespace, so the envelope can render the
    /// list directly.
    pub dropped_comments: Vec<String>,
    /// Upstream `[package].name` read out of the same Cargo.toml the
    /// overlay parsed. `Some(name)` when the upstream manifest has a
    /// non-empty string value at `package.name`; `None` for malformed
    /// manifests or workspace roots that lack the `[package]` table.
    ///
    /// Captured here so the compat driver does not have to read and
    /// parse Cargo.toml a second time to populate the §3.3 envelope's
    /// `crate_name` field — the overlay code already has the parsed
    /// `toml::Value` in hand.
    pub upstream_crate_name: Option<String>,
}

/// Synthetic `[package.metadata.lihaaf]` table the compat driver
/// injects into the sibling overlay so the upstream pilot fork does not
/// need to hand-author a metadata block.
///
/// Constructed by the compat driver after it has resolved the crate
/// name + converted-fixtures directory; passed to
/// [`materialize_overlay_with_metadata`]. The fields map 1:1 to the
/// keys the v0.1 [`crate::config::Config`] loader expects.
#[derive(Debug, Clone)]
pub struct SyntheticMetadata {
    /// `dylib_crate` — the workspace-member crate name. The compat
    /// driver reads this from upstream `[package].name`.
    pub dylib_crate: String,
    /// `extern_crates` — list of `--extern` names handed to per-fixture
    /// rustc. The compat driver always sets this to `[dylib_crate]`;
    /// the v0.1 config loader enforces `extern_crates[0] == dylib_crate`
    /// anyway.
    pub extern_crates: Vec<String>,
    /// `fixture_dirs` — list of directories to walk for fixtures. The
    /// compat driver populates this with the converted-fixtures path
    /// under `<compat_root>/target/lihaaf-compat-converted/`. Paths
    /// are written verbatim into the TOML.
    pub fixture_dirs: Vec<String>,
    /// `allow_lints` — rustc lints forwarded as `-A <lint>` on every
    /// per-fixture invocation. Defaults to `["unexpected_cfgs"]` in
    /// compat mode as **forward-only insurance**. Today, with rustc
    /// 1.95 not passing `--check-cfg` automatically and lihaaf not
    /// setting it (verified `src/worker.rs:916-919, 929-972`), this
    /// default is a no-op — the `unexpected_cfgs` lint is
    /// `--check-cfg`-gated and does not fire. Once `--check-cfg` is
    /// active in rustc (either by default or by lihaaf passing it
    /// explicitly in a future release), compat pilots would otherwise
    /// produce unavoidable `unexpected_cfgs` noise from their
    /// proc-macro-emitted `#[cfg(feature = "...")]` annotations. This
    /// default suppresses that noise preemptively so the toolchain
    /// shift is uneventful.
    ///
    /// This default does NOT address the v0.1-active default-on lints
    /// (`unused_imports`, `dead_code`, etc.) that fire under bare
    /// rustc today. Round-2 compat pilots that hit those add the
    /// relevant entries to their own fork's
    /// `[package.metadata.lihaaf].allow_lints` via the v0.1 TOML path.
    ///
    /// To override (e.g. add more lints, or empty for diagnostic
    /// debugging), the compat-driver caller passes a custom list when
    /// constructing `SyntheticMetadata`.
    pub allow_lints: Vec<String>,
}

/// Construct the `SyntheticMetadata` that the compat driver embeds in
/// the staged overlay for the named crate.
///
/// This is the **single authoritative source** for the driver's default
/// `allow_lints` list. The compat driver (`src/compat/mod.rs`) calls
/// this function instead of inlining the struct literal so that:
///
/// 1. A future change to the `allow_lints` default is a one-line edit
///    in one place.
/// 2. Test #17 (`synthetic_metadata_default_in_compat_driver`) can call
///    this same function and assert against an independently written
///    literal — any drift between the function and the expected default
///    is caught immediately.
///
/// `fixture_dirs` carries the two absolute converted-fixture directories
/// (`compile_pass` / `compile_fail`); callers compute these before
/// constructing the metadata.
pub(crate) fn compat_default_synthetic_metadata(
    name: &str,
    fixture_dirs: Vec<String>,
) -> SyntheticMetadata {
    SyntheticMetadata {
        dylib_crate: name.to_string(),
        extern_crates: vec![name.to_string()],
        fixture_dirs,
        // Forward-only insurance: suppresses `unexpected_cfgs` noise for
        // compat-mode pilots once `--check-cfg` becomes active (either via
        // lihaaf or a future rustc default). Under v0.1.0 today this is a
        // no-op — the lint is `--check-cfg`-gated and lihaaf does not pass
        // that flag (verified worker.rs:916-919, 929-972). See
        // `SyntheticMetadata.allow_lints` rustdoc for the full rationale.
        allow_lints: vec!["unexpected_cfgs".to_string()],
    }
}

/// Read the upstream `Cargo.toml`, materialize the sibling overlay, and
/// return the plan.
///
/// `upstream_manifest_path` must point at the upstream `Cargo.toml`
/// itself (not its parent directory). The sibling path is computed via
/// [`Path::with_file_name`] — the safest cross-platform way to swap
/// only the filename component.
///
/// **Pre-write idempotency check.** If the sibling already exists with
/// byte-identical contents, no write is performed. This preserves
/// mtime per the §3.2.3 "dirty-worktree rule" — repeated runs from
/// clean state must not churn the filesystem.
///
/// **Errors.** Returns [`Error::Io`] on read/write failure and
/// [`Error::TomlParse`] when the upstream manifest cannot be parsed as
/// TOML. The compat driver maps both into the §3.3 envelope's
/// `overlay.*` error category.
pub fn materialize_overlay(upstream_manifest_path: &Path) -> Result<OverlayPlan, Error> {
    materialize_overlay_with_metadata(upstream_manifest_path, None)
}

/// Variant of [`materialize_overlay`] that also injects a synthetic
/// `[package.metadata.lihaaf]` table into the sibling overlay.
///
/// When `synthetic_metadata` is `Some`, the table is spliced into the
/// parsed `package.metadata.lihaaf` location BEFORE the canonical
/// serializer runs, so the on-disk overlay carries the metadata block
/// the v0.1 [`crate::config::load`] entry needs to drive a compat-mode
/// inner session.
///
/// **Conflict policy.** If the upstream `Cargo.toml` already has a
/// `[package.metadata.lihaaf]` table, the synthetic metadata is
/// OVERWRITTEN with the synthetic values: compat mode owns the
/// inner-session config; an existing metadata block in a pilot fork
/// would have been written under v0.1 semantics that may not match
/// the compat-driver-synthesized `fixture_dirs` path under
/// `<compat_root>/target/lihaaf-compat-converted/`.
///
/// **Errors.** Same shape as [`materialize_overlay`].
pub fn materialize_overlay_with_metadata(
    upstream_manifest_path: &Path,
    synthetic_metadata: Option<&SyntheticMetadata>,
) -> Result<OverlayPlan, Error> {
    // Bridge to the builder-shaped entry: the builder ignores the
    // upstream name and returns the caller's pre-constructed metadata
    // (cloned because the builder owns the returned value).
    materialize_overlay_inner(upstream_manifest_path, |_name| synthetic_metadata.cloned())
}

/// Variant of [`materialize_overlay_with_metadata`] whose synthetic
/// metadata is constructed by a builder closure given the upstream
/// crate name. Lets the compat driver build the synthetic block
/// `[package.metadata.lihaaf]` using the crate name without parsing
/// `Cargo.toml` a second time — the overlay code passes the parsed
/// `[package].name` directly into the builder.
///
/// `builder` receives `Some(name)` when the upstream manifest carries a
/// non-empty `[package].name` string, and `None` for workspace roots /
/// malformed manifests (where the caller decides on a fallback). The
/// builder may return `None` to skip metadata injection entirely.
///
/// **Errors.** Same shape as [`materialize_overlay`].
pub fn materialize_overlay_with_synthetic_metadata_builder<F>(
    upstream_manifest_path: &Path,
    builder: F,
) -> Result<OverlayPlan, Error>
where
    F: FnOnce(Option<&str>) -> SyntheticMetadata,
{
    materialize_overlay_inner(upstream_manifest_path, |name| Some(builder(name)))
}

fn materialize_overlay_inner<F>(
    upstream_manifest_path: &Path,
    synthetic_metadata: F,
) -> Result<OverlayPlan, Error>
where
    F: FnOnce(Option<&str>) -> Option<SyntheticMetadata>,
{
    let raw_bytes = std::fs::read(upstream_manifest_path).map_err(|e| {
        Error::io(
            e,
            "reading upstream Cargo.toml for overlay",
            Some(upstream_manifest_path.to_path_buf()),
        )
    })?;
    let raw_text = String::from_utf8(raw_bytes).map_err(|e| {
        Error::io(
            std::io::Error::new(std::io::ErrorKind::InvalidData, e),
            "decoding upstream Cargo.toml as UTF-8",
            Some(upstream_manifest_path.to_path_buf()),
        )
    })?;

    let dropped_comments = scan_dropped_comments(&raw_text);

    let mut value: toml::Value =
        toml::from_str(&raw_text).map_err(|e: toml::de::Error| Error::TomlParse {
            path: upstream_manifest_path.to_path_buf(),
            message: e.to_string(),
        })?;

    // Spec invariant: `--compat-root` is single-crate. A workspace-root
    // manifest (`[workspace]` table without a top-level `[package]`)
    // cannot host a `[lib] crate-type` rewrite; the lihaaf stage-3
    // dylib build would have nothing to compile. `[workspace.package]`
    // is inherited-metadata for member crates and does NOT make the
    // manifest itself buildable, so it is rejected uniformly.
    // Reject with a directed diagnostic pointing the adopter at a
    // member crate's Cargo.toml. The empty / unusual case (neither
    // `[package]` nor `[workspace]`) stays tolerant — that may be a
    // test fixture or a partial manifest the operator is constructing.
    if is_workspace_root_manifest(&value) {
        return Err(Error::Cli {
            clap_exit_code: 2,
            message: format!(
                "error: `--compat-root` must point to a single-crate Cargo.toml; \
                 `{}` is a workspace root (declares `[workspace]` without `[package]`). \
                 Pass a member crate's Cargo.toml as `--compat-root` instead.",
                upstream_manifest_path.display()
            ),
        });
    }

    let upstream_already_has_dylib = inspect_existing_crate_type(&value);
    let upstream_crate_name = read_upstream_crate_name(&value);
    let synthetic = synthetic_metadata(upstream_crate_name.as_deref());

    // Resolve the upstream crate directory once — every path
    // absolutization below joins against this so cargo can resolve the
    // overlay's path-bearing keys from the staged manifest dir (which is
    // two directories deeper than the upstream `Cargo.toml`).
    let upstream_dir: PathBuf = upstream_manifest_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));

    // Staged overlay dir, shared by `apply_self_patch_policy` (which
    // needs the absolutized path string for its INJECT / REMAP
    // emission) and the staged-mirror writer (which needs the same
    // dir to create symlinks into). Sharing the construction with
    // `sibling_path` below keeps the single source-of-truth shape:
    // `<upstream>/target/lihaaf-overlay/`. Shape A per
    // `docs/plans/issue-40-47-overlay-vs-registry.md` §4.2.
    let staged_overlay_dir: PathBuf = upstream_dir.join("target").join("lihaaf-overlay");

    if let toml::Value::Table(top) = &mut value {
        // Insert/extend [lib] crate-type. The canonicalization is
        // idempotent: a second run on the output is a no-op.
        let lib_table = top
            .entry("lib".to_string())
            .or_insert_with(|| toml::Value::Table(toml::map::Map::new()));
        if let toml::Value::Table(lib) = lib_table {
            canonicalize_crate_type(lib)?;
        } else {
            return Err(Error::TomlParse {
                path: upstream_manifest_path.to_path_buf(),
                message: "`[lib]` must be a table, not an inline value".to_string(),
            });
        }

        // Absolutize every path-bearing key against the upstream crate
        // directory. The staged overlay's parent dir is
        // `<upstream>/target/lihaaf-overlay/`, two levels deeper than the
        // upstream `Cargo.toml`; cargo resolves every path-bearing key
        // relative to the manifest's parent dir, so without
        // absolutization cargo searches the staged dir for files that
        // only exist under the upstream crate dir (and fails the build
        // with an opaque "can't find library" / "no targets" error). See
        // `absolutize_path_bearing_keys` for the full key inventory and
        // the workspace-members handling rationale.
        absolutize_path_bearing_keys(top, &upstream_dir);

        // Option H intent-aware self-patch policy for
        // `[patch.crates-io.<upstream-package-name>]` (issues #40 + #47).
        //
        // - cxx (#47) fails with `package cxx links to the native library
        //   cxxbridge1, but it conflicts with a previous package which
        //   links to cxxbridge1 as well` because cxx-test-suite declares
        //   `cxx = "1.0"` from crates.io while the overlay declares
        //   `[package] name = "cxx"` from `target/lihaaf-overlay/`: two
        //   distinct source-ids for the same `links` claim.
        // - serde_json (#40) fails with `specification serde_json is
        //   ambiguous` for the same root cause without the `links`
        //   collision detail.
        //
        // Rule 1 INJECT (clean upstream — anyhow / thiserror / serde_json
        // / clean Round-2 candidates) emits a `{ path =
        // "<staged-overlay-dir>" }` entry pointing at the overlay's own
        // package; cargo collapses both registry-name references to the
        // staged-overlay path-source-id and the conflict / ambiguity is
        // gone.
        //
        // Rule 2 REMAP (cxx upstream's `[patch.crates-io.cxx] = { path =
        // "." }`) replaces the upstream's self-patch entry with the same
        // staged-overlay-dir target — preserving the upstream's "patch
        // to root" intent in the overlay's manifest context.
        //
        // Rule 3 CONTINUE-ABSOLUTIZE leaves non-`<self>` `[patch.crates-
        // io.<X>]` entries alone — `absolutize_patch_paths` (above) has
        // already absolutized them against `upstream_dir`.
        //
        // Rule 4 REJECT (vendored fork / git source / non-root path)
        // surfaces `Error::CompatPatchOverrideConflict`; the
        // `--compat-allow-patch-override` escape hatch is deferred to
        // v0.2 / v1.1.
        //
        // Targets the STAGED OVERLAY DIR (not the upstream dir) to
        // avoid the R1 self-loop bug: pointing the patch at the upstream
        // dir IS the source-id cargo already aliases to crates.io. See
        // `apply_self_patch_policy` rustdoc and §2.1 / §2.6 of the
        // implementation plan for the cargo-anchoring reasoning.
        apply_self_patch_policy(
            top,
            upstream_crate_name.as_deref(),
            &upstream_dir,
            &staged_overlay_dir,
        )?;

        if let Some(meta) = synthetic.as_ref() {
            inject_synthetic_metadata(top, meta);
        }

        // Override workspace inheritance: declare the overlay as its own
        // workspace root (so cargo stops walking up to the upstream
        // workspace) but PRESERVE the upstream's `[workspace.dependencies]`
        // / `[workspace.package]` / `[workspace.lints]` / `[workspace.metadata]`
        // / `[workspace.resolver]` (and any unknown `[workspace.X]`) so any
        // `{ workspace = true }` inheritance reference in the overlay
        // continues to resolve. Only the membership keys (`members`,
        // `exclude`, `default-members`) are stripped — those are the keys
        // that cause the "wrong workspace" error. Runs AFTER
        // `absolutize_path_bearing_keys` so absolutized values inside the
        // preserved tables (e.g. `[workspace.dependencies.X].path`) are
        // carried through, while the absolutization of the stripped
        // `members` / `exclude` / `default-members` is harmlessly discarded.
        //
        // REJECTS workspace-member cases (EXPLICIT `[package].workspace
        // = "<path>"`, IMPLICIT no-`[workspace]` + ancestor `[workspace]`,
        // and IMPLICIT no-`[workspace]` + `{ workspace = true }` references).
        // See module-level docs (lines 130-238) and function-level docs
        // for the cargo-walk-up discovery rationale and the five-branch
        // decision tree.
        override_workspace_inheritance(top, upstream_manifest_path)?;
    }

    let serialized = serialize_canonical(&value)?;

    // Stage the overlay at `<upstream_dir>/target/lihaaf-overlay/Cargo.toml`.
    //
    // The filename MUST be `Cargo.toml`: `cargo rustc --manifest-path`
    // rejects any path whose last component is not literally `Cargo.toml`
    // (exit code 1, "the manifest-path must be a path to a Cargo.toml
    // file"). Staging under `target/lihaaf-overlay/` isolates the overlay
    // from the upstream `Cargo.toml` while satisfying that constraint.
    // `write_file_atomic` calls `create_dir_all` on the parent, so the
    // subdirectory is created on first use without a separate call here.
    let sibling_path = staged_overlay_dir.join("Cargo.toml");

    // Idempotent rerun guard — skip the write when bytes match. This
    // preserves mtime so a clean-state second invocation does not
    // appear as a worktree change to fork-CI greppers.
    let need_write = match std::fs::read(&sibling_path) {
        Ok(existing) => existing != serialized,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => true,
        Err(e) => {
            return Err(Error::io(
                e,
                "checking existing staged overlay for idempotent rerun",
                Some(sibling_path.clone()),
            ));
        }
    };

    if need_write {
        util::write_file_atomic(&sibling_path, &serialized)?;
    }

    // Staged package-root mirror (issues #40 + #47, §4.5). After the
    // overlay manifest is written, populate the staged-overlay dir with
    // a structural mirror of the upstream package root so build scripts
    // can read package-root files via `CARGO_MANIFEST_DIR` / cwd:
    //
    // - cxx `build.rs:143-148` reads `src/cxx.cc` via
    //   `manifest_dir.join(...)` (hard error without the mirror).
    // - cxx `build.rs:154-159` references `include/cxx.h`.
    // - anyhow `build.rs:255-257,323-367` probes
    //   `Path::new("src").join("nightly.rs")` from cwd (silent-false
    //   without the mirror — wrong cfg flags).
    // - thiserror `build.rs:261-263,328-371` probes
    //   `Path::new("build").join("probe.rs")` from cwd (same silent-
    //   false hazard).
    //
    // Exclusions: `target/` (disposable), `.git/` (must-be-absent),
    // `Cargo.toml` (overlay-generated, post-condition assertion),
    // `Cargo.lock` (must-be-absent). Idempotency contract Option B
    // (§4.5.6): skip-on-canonical-symlink, reconcile-by-replacement
    // for all other states, exact-sync copy fallback.
    mirror_upstream_into_overlay(&upstream_dir, &staged_overlay_dir)?;

    Ok(OverlayPlan {
        upstream_manifest: upstream_manifest_path.to_path_buf(),
        sibling_manifest: sibling_path,
        upstream_already_has_dylib,
        dropped_comments,
        upstream_crate_name,
    })
}

/// Read the upstream `[package].name` out of an already-parsed
/// `toml::Value`. Returns `Some(name)` when the field is a non-empty
/// string; `None` for missing tables, non-string values, or
/// workspace-root manifests that lack `[package]`. The caller falls
/// back to a basename heuristic in those cases.
fn read_upstream_crate_name(value: &toml::Value) -> Option<String> {
    value
        .get("package")
        .and_then(|p| p.get("name"))
        .and_then(|n| n.as_str())
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// Membership keys of `[workspace]` that must be stripped from the
/// staged overlay. Every OTHER key in `[workspace]` is preserved.
///
/// Keeping this list as a single source of truth makes the
/// selective-rewrite intent explicit: an addition to this list strips
/// more, a removal preserves more. The R1 implementation effectively
/// listed every workspace key (full clobber); the R2 implementation
/// lists only the three keys that actually cause the "wrong workspace"
/// error.
const WORKSPACE_MEMBERSHIP_KEYS: &[&str] = &["members", "exclude", "default-members"];

/// Override the overlay's workspace inheritance: declare the overlay
/// as its own workspace root, but PRESERVE the upstream's workspace
/// inheritance tables.
///
/// **What this does (in order — five mutually-exclusive branches):**
///
/// 1. **Explicit member.** If the upstream has `[package].workspace =
///    "<ancestor>"`, REJECT with a directed diagnostic. The overlay
///    cannot self-declare as a workspace root and an explicit member
///    of another workspace simultaneously.
/// 2. **Implicit member via ancestor workspace (R4).** If the
///    upstream has NO local `[workspace]` and an ancestor `Cargo.toml`
///    on the filesystem walk-up carries `[workspace]`, REJECT with a
///    directed diagnostic naming the offending ancestor manifest path.
///    The ancestor workspace may carry `[patch.crates-io]`, `[replace]`,
///    `[profile]`, `resolver`, or `[workspace.dependencies]` tables
///    that affect baseline cargo's dependency resolution; the lihaaf
///    overlay terminates cargo's walk-up at the staged manifest and
///    skips the ancestor entirely, producing a divergent dependency
///    graph and false compat verdicts. See module-level docs for the
///    "conservative reject any ancestor workspace" rationale.
/// 3. **Implicit member via inheritance refs only (R3).** If the
///    upstream has NO local `[workspace]`, NO ancestor workspace
///    detected on the walk-up, BUT any `{ workspace = true }`
///    inheritance reference is present, REJECT with a directed
///    diagnostic. This catches manifests whose ancestor workspace
///    exists outside the filesystem walk-up's reach (or in a
///    Cargo.toml we cannot parse).
/// 4. **Workspace-root.** If the upstream had `[workspace]`, CLONE
///    it and strip only the membership keys (`members`, `exclude`,
///    `default-members`). Every other key — `dependencies`, `package`,
///    `lints`, `metadata`, `resolver`, plus any unknown
///    `[workspace.X]` cargo may add in future releases — is preserved
///    verbatim.
/// 5. **Standalone.** Otherwise (no `[workspace]`, no inheritance
///    references, no ancestor workspace), inject an empty
///    `[workspace] = {}` so cargo treats the overlay as its own
///    workspace root.
///
/// **Why this is necessary (cargo walk-up).** When cargo resolves the
/// staged overlay at `<upstream>/target/lihaaf-overlay/Cargo.toml`, it
/// walks UP the filesystem to find the overlay's workspace root. For
/// workspace-style upstreams (cxx, serde-json, thiserror) it reaches
/// the upstream `Cargo.toml` first, which declares `[workspace]`. The
/// overlay's package isn't in the upstream's `members`, so cargo errors
/// with `package <X>/Cargo.toml is a member of the wrong workspace`.
/// See issue #36 for the v0.1.0-beta.5 GitHub Actions run that surfaced
/// this on every workspace-style pilot.
///
/// **Why we don't simply clobber (the R1 failure mode).** Cargo's
/// workspace-inheritance feature lets a manifest write
/// `[dependencies] foo = { workspace = true }` and inherit the actual
/// dep spec from `[workspace.dependencies.foo]`. The same pattern
/// applies to `[package].version.workspace = true` (from
/// `[workspace.package]`), `[lints].rust.workspace = true` (from
/// `[workspace.lints]`), and all dep tables (`dev-dependencies`,
/// `build-dependencies`, `target.<cfg>.dependencies`). R1 replaced
/// `[workspace]` with an empty table — which is correct for the
/// `members` problem but wrong because surviving `{ workspace = true }`
/// references in `[dependencies]` / `[package]` / `[lints]` then fail
/// cargo's parser with "workspace inheritance was specified but
/// `[workspace.<X>]` was not defined". This R2 implementation
/// preserves the inheritance tables.
///
/// **Workspace-member cases are out of scope (explicit AND implicit).**
/// When the overlay manifest itself carries `[package].workspace =
/// "<path>"`, that declares the manifest as an EXPLICIT MEMBER of an
/// ANCESTOR workspace. When the manifest has NO local `[workspace]`
/// table AND an ancestor `Cargo.toml` carries `[workspace]`, OR has
/// at least one `{ workspace = true }` inheritance reference, it is
/// an IMPLICIT MEMBER. In all of these cases, the actual
/// `[workspace.dependencies]` / `[workspace.package]` /
/// `[workspace.lints]` tables live in the ancestor — to preserve the
/// inheritance references we would need to read that ancestor and
/// copy the tables down into the overlay. That cross-manifest read is
/// out-of-scope for v0.1.0-beta.6; we reject all three cases with
/// directed diagnostics instead. None of the four Round-1 pilots
/// (cxx, serde-json, anyhow, thiserror) invokes lihaaf from a
/// workspace member — they all invoke from upstream ROOT (which
/// carries both `[package]` and `[workspace]`, the workspace-root
/// case) — so none of the rejections affect any currently-enrolled
/// pilot. The R3 + R4 rejections are defense-in-depth for any future
/// invocation from a workspace sub-crate. The follow-up to enable
/// workspace-member overlays (copying ancestor inheritance tables
/// down) will land separately.
///
/// **Why this runs LAST.** The earlier `absolutize_path_bearing_keys`
/// pass has already rewritten `[workspace.dependencies.X].path`,
/// `[workspace.package]` fields (if any path-bearing), and the
/// membership arrays. Since R2 preserves the inheritance tables, the
/// earlier absolutization is now LOAD-BEARING — the preserved
/// `[workspace.dependencies.X].path` is consumed by cargo to resolve
/// `{ workspace = true }` references from `[dependencies]`. The
/// absolutization of `members` / `exclude` / `default-members` is
/// harmlessly stripped on this pass.
///
/// Idempotent: a second call on already-overridden output is a no-op
/// (the membership keys are already absent and `[package].workspace`
/// is absent). The R4 ancestor walk re-reads the filesystem on each
/// call but never mutates it; the walk's result is the same on a
/// second invocation.
///
/// **Errors.** Returns `Error::Cli` with `clap_exit_code = 2` when the
/// upstream manifest is a workspace member — explicit
/// (`[package].workspace = "<path>"`), implicit-via-ancestor (no local
/// `[workspace]` but ancestor `Cargo.toml` carries `[workspace]`), or
/// implicit-via-inheritance-refs (no local `[workspace]` but at least
/// one `{ workspace = true }` reference). May also return `Error::Io`
/// when an ancestor `Cargo.toml` exists but cannot be read due to a
/// non-NotFound I/O error (permissions, etc.). All other shapes
/// succeed.
fn override_workspace_inheritance(
    top: &mut toml::map::Map<String, toml::Value>,
    upstream_manifest_path: &Path,
) -> Result<(), Error> {
    // 1. Reject the EXPLICIT workspace-member case. A package
    //    declaring itself as a member of an ancestor workspace
    //    (`[package].workspace = "<path>"`) cannot simultaneously be
    //    declared as a workspace root — and copying the ancestor's
    //    inheritance tables into the overlay is out-of-scope for
    //    v0.1.0-beta.6 (see function-level docs above for the full
    //    rationale).
    if let Some(toml::Value::Table(pkg)) = top.get("package")
        && pkg.contains_key("workspace")
    {
        return Err(Error::Cli {
            clap_exit_code: 2,
            message: format!(
                "error: `--compat-root` `{}` is a workspace member: \
                 `[package].workspace = \"...\"` declares membership in \
                 an ancestor workspace, which compat mode cannot reach. \
                 Compat mode currently supports only single-crate \
                 manifests and workspace-root manifests (where \
                 `[workspace]` lives in the same Cargo.toml). \
                 Pass the workspace-ROOT Cargo.toml as `--compat-root` \
                 instead; it will still resolve `{{ workspace = true }}` \
                 references in its own manifest because \
                 `[workspace.dependencies]` / `[workspace.package]` / \
                 `[workspace.lints]` are preserved in the staged overlay.",
                upstream_manifest_path.display()
            ),
        });
    }

    let has_local_workspace = top.get("workspace").is_some_and(|v| v.is_table());

    // 2. Reject the IMPLICIT workspace-member case via ancestor
    //    `Cargo.toml` walk-up (R4 — Codex BLOCK fixup in PR #37 R3
    //    review). If the manifest has no local `[workspace]` table
    //    AND any ancestor `Cargo.toml` on the filesystem walk-up
    //    carries `[workspace]`, REJECT. The ancestor workspace may
    //    carry `[patch.crates-io]`, `[replace]`, `[profile]`,
    //    `resolver`, or `[workspace.dependencies]` tables that
    //    affect baseline cargo's dependency resolution; the lihaaf
    //    overlay terminates cargo's walk-up at the staged manifest
    //    and skips the ancestor's state entirely, producing a
    //    divergent dependency graph between baseline and overlay and
    //    therefore false compat verdicts. The check is CONSERVATIVE:
    //    any ancestor workspace triggers rejection regardless of
    //    whether its `members` array explicitly names this manifest
    //    (see module-level docs for the rationale).
    if !has_local_workspace
        && let Some(ancestor_manifest) = detect_implicit_ancestor_workspace(upstream_manifest_path)?
    {
        return Err(Error::Cli {
            clap_exit_code: 2,
            message: format!(
                "error: `--compat-root` `{}` is an implicit workspace member: \
                 it has no local `[workspace]` table but an ancestor manifest \
                 at `{}` carries `[workspace]`. Cargo's baseline build walks \
                 up the filesystem and would apply the ancestor's `[patch]` / \
                 `[replace]` / `[profile]` / `resolver` / \
                 `[workspace.dependencies]` tables during dependency \
                 resolution, but the lihaaf overlay declares its own \
                 `[workspace]` and terminates cargo's walk-up at the staged \
                 manifest — producing a divergent dependency graph and \
                 false compat verdicts. Compat mode currently cannot copy \
                 the ancestor workspace state down into the overlay. \
                 Either invoke `cargo lihaaf --compat` from the workspace \
                 ROOT (`{}` or its containing directory), or restructure \
                 the fork so the crate-under-test has no ancestor workspace.",
                upstream_manifest_path.display(),
                ancestor_manifest.display(),
                ancestor_manifest.display(),
            ),
        });
    }

    // 3. Reject the IMPLICIT workspace-member case via inheritance
    //    references only (R3 fixup). If the manifest has no local
    //    `[workspace]` table but contains any `{ workspace = true }`
    //    inheritance reference, it is a workspace member whose
    //    membership is declared in an ancestor `Cargo.toml`'s
    //    `members = [...]` array. The R4 ancestor-walk above
    //    catches the common case where the ancestor exists as a
    //    parseable `Cargo.toml`; this branch catches the residual
    //    case where the ancestor is unreachable on the walk-up
    //    (e.g., outside the working tree, behind a symlink we did
    //    not follow, or a Cargo.toml we could not parse). Injecting
    //    an empty `[workspace]` here would strand every such
    //    reference at cargo parse time with the cryptic "workspace
    //    inheritance was specified but `[workspace.X]` was not
    //    defined" error. Reject with the same directed diagnostic
    //    family as branches 1 and 2 so the user gets actionable
    //    output.
    if !has_local_workspace && manifest_has_inheritance_reference(top) {
        return Err(Error::Cli {
            clap_exit_code: 2,
            message: format!(
                "error: `--compat-root` `{}` is an implicit workspace member: \
                 it has no local `[workspace]` table but uses workspace \
                 inheritance (one or more `{{ workspace = true }}` \
                 references in `[package]` / `[dependencies]` / \
                 `[dev-dependencies]` / `[build-dependencies]` / \
                 `[target.<cfg>.<deps>]` / `[lints]`). Cargo discovers \
                 the ancestor workspace by walking up the filesystem, \
                 but compat mode cannot reach into that ancestor to \
                 copy down the `[workspace.dependencies]` / \
                 `[workspace.package]` / `[workspace.lints]` tables \
                 the inheritance references resolve against. \
                 Pass the workspace-ROOT Cargo.toml as `--compat-root` \
                 instead; the staged overlay preserves its \
                 `[workspace.*]` inheritance tables verbatim.",
                upstream_manifest_path.display()
            ),
        });
    }

    // 4. Build the overlay's `[workspace]` table. If the upstream
    //    had one, clone it and strip ONLY the membership keys.
    //    Otherwise inject an empty table so cargo treats the overlay
    //    as its own workspace root (terminating the walk-up).
    let mut new_workspace = if let Some(toml::Value::Table(existing)) = top.get("workspace") {
        let mut cloned = existing.clone();
        for key in WORKSPACE_MEMBERSHIP_KEYS {
            cloned.remove(*key);
        }
        cloned
    } else {
        toml::map::Map::new()
    };

    // 5. Idempotency / belt-and-braces: if a future pass re-introduces
    //    one of the membership keys, this re-strips. Cheap; preserves
    //    the documented idempotency contract.
    for key in WORKSPACE_MEMBERSHIP_KEYS {
        new_workspace.remove(*key);
    }

    top.insert("workspace".to_string(), toml::Value::Table(new_workspace));
    Ok(())
}

/// Walk UP the filesystem from `upstream_manifest_path`'s parent
/// directory, looking for an ancestor `Cargo.toml` that declares a
/// `[workspace]` table. Returns `Some(ancestor_manifest_path)` on the
/// first such ancestor found; returns `None` if the walk reaches the
/// filesystem root without finding any ancestor workspace.
///
/// Used by [`override_workspace_inheritance`] (branch 2) to detect the
/// implicit-workspace-member case where the descendant manifest has no
/// local `[workspace]` table but is contained within an ancestor
/// workspace that affects baseline cargo's dependency resolution.
///
/// **Walk-up semantics.** Starts at `parent_of(parent_of(upstream))`,
/// i.e., one level above the directory containing the manifest. This
/// avoids re-checking the upstream manifest itself (which we already
/// have parsed in [`override_workspace_inheritance`]) and is what
/// cargo's own walk-up does. Each iteration:
///
/// - If `<dir>/Cargo.toml` does not exist (`NotFound`), continue
///   walking up. This is the common case — most directories on a
///   typical Linux filesystem do not contain a `Cargo.toml`.
/// - If `<dir>/Cargo.toml` exists but fails to parse as TOML, emit a
///   non-fatal warning on stderr and continue walking. The user does
///   not control ancestor manifests they did not author; we should
///   not abort compat-mode entirely because a third-party Cargo.toml
///   somewhere above is malformed.
/// - If `<dir>/Cargo.toml` exists, parses, and contains
///   `[workspace]`, return the ancestor manifest path immediately.
/// - If `<dir>/Cargo.toml` exists, parses, but does NOT contain
///   `[workspace]`, continue walking. Cargo's own walk-up does the
///   same — it does not stop at the first Cargo.toml but at the first
///   `[workspace]`.
/// - Other I/O errors (permission denied, etc.) propagate as
///   [`Error::Io`]. These are not silent skip cases — a permissions
///   problem reading an ancestor is something the user should see.
///
/// The walk terminates at the filesystem root (`Path::parent` returns
/// `None`). The `Path::canonicalize` call is intentionally NOT made:
/// the upstream manifest path is already absolutized at the CLI layer
/// ([`crate::compat::args::CompatArgs::from_cli`]), and re-canonicalizing
/// would require the path to exist on disk (which fails for test
/// dummies and for legitimate non-existent ancestor manifests).
fn detect_implicit_ancestor_workspace(
    upstream_manifest_path: &Path,
) -> Result<Option<PathBuf>, Error> {
    let Some(manifest_dir) = upstream_manifest_path.parent() else {
        // No parent — e.g. root-level manifest. Nothing to walk.
        return Ok(None);
    };
    let mut current = manifest_dir.parent();
    while let Some(dir) = current {
        let candidate = dir.join("Cargo.toml");
        match std::fs::read_to_string(&candidate) {
            Ok(text) => {
                match toml::from_str::<toml::Value>(&text) {
                    Ok(value) => {
                        if value.get("workspace").is_some_and(|v| v.is_table()) {
                            return Ok(Some(candidate));
                        }
                        // Ancestor manifest exists and parses but has
                        // no `[workspace]`. Cargo's walk-up does not
                        // stop here; we don't either. Continue.
                    }
                    Err(e) => {
                        // Malformed ancestor manifest. Log and
                        // continue — we should not abort compat mode
                        // entirely because of a third-party manifest
                        // the user did not author.
                        eprintln!(
                            "lihaaf: warning: skipping ancestor Cargo.toml `{}` during \
                             workspace detection: TOML parse error: {}",
                            candidate.display(),
                            e
                        );
                    }
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                // Most directories on the walk-up have no Cargo.toml.
                // Continue silently.
            }
            Err(e) => {
                return Err(Error::io(
                    e,
                    "reading ancestor Cargo.toml during workspace detection",
                    Some(candidate),
                ));
            }
        }
        current = dir.parent();
    }
    Ok(None)
}

/// Return `true` when `top` contains any `{ workspace = true }`
/// inheritance reference at any of the cargo-recognized inheritance
/// sites. Used by [`override_workspace_inheritance`] to detect
/// implicit workspace members (manifests with no local `[workspace]`
/// table but at least one inheritance reference whose target lives
/// in an ancestor manifest).
///
/// Detection sites, per the cargo book:
///
/// - `[package].<key>` where `<key>` is any field that cargo allows
///   to inherit from `[workspace.package]` — `version`, `edition`,
///   `rust-version`, `authors`, `license`, `repository`, `homepage`,
///   `description`, `readme`, `keywords`, `categories`, `publish`,
///   `documentation`, `include`, `exclude`. To stay forward-compatible
///   with any future cargo addition, we scan ALL sub-keys of
///   `[package]` and flag any whose value is a table containing
///   `workspace = true`.
/// - `[dependencies.X]`, `[dev-dependencies.X]`,
///   `[build-dependencies.X]` — any dep table containing
///   `workspace = true` (with or without other keys like `features`).
/// - `[target.<cfg>.dependencies.X]`, same for `dev-dependencies` and
///   `build-dependencies` — platform-conditional analogues of the
///   above.
/// - `[lints]` — the top-level form is `[lints] workspace = true`
///   (a `workspace` key at the lints table root, NOT nested under
///   `lints.rust` / `lints.clippy` / `lints.rustdoc`). Cargo
///   currently supports inheritance only at this top level (all or
///   nothing); we also defensively scan one level deeper so a future
///   cargo extension that allows per-namespace inheritance does not
///   silently bypass the rejection.
///
/// **What `workspace = true` looks like in the parsed TOML tree.**
/// The two surface syntaxes — `foo = { workspace = true }` (inline
/// table) and `foo.workspace = true` (dotted path) — both decode to
/// the same shape: a sub-table at the named key whose `workspace`
/// entry is the boolean `true`. We only need to check for that
/// shape; the parser handles both surface syntaxes uniformly.
fn manifest_has_inheritance_reference(top: &toml::map::Map<String, toml::Value>) -> bool {
    // Helper: a table-typed sub-value contains `workspace = true`.
    let is_inheritance_table = |v: &toml::Value| -> bool {
        v.as_table()
            .and_then(|t| t.get("workspace"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
    };

    // Helper: scan every entry of a dep-style table (the value at
    // each key is a per-dep table) for an inheritance reference.
    let deps_table_has_inheritance =
        |top: &toml::map::Map<String, toml::Value>, key: &str| -> bool {
            let Some(toml::Value::Table(t)) = top.get(key) else {
                return false;
            };
            t.values().any(is_inheritance_table)
        };

    // 1. `[package].<key>` — every sub-key of `[package]`. We scan
    //    all sub-keys (not just the cargo-documented inheritable
    //    fields) so a future cargo addition does not silently bypass
    //    the rejection.
    //
    //    Skip the `workspace` sub-key itself: `[package].workspace`
    //    is the EXPLICIT workspace-member pointer, not an inheritance
    //    reference. (It is a String pointing at the ancestor dir,
    //    not a Table containing `workspace = true`.) The explicit
    //    case is handled upstream of this helper.
    if let Some(toml::Value::Table(pkg)) = top.get("package") {
        for (k, v) in pkg.iter() {
            if k == "workspace" {
                continue;
            }
            if is_inheritance_table(v) {
                return true;
            }
        }
    }

    // 2. Top-level dep tables.
    for section in ["dependencies", "dev-dependencies", "build-dependencies"] {
        if deps_table_has_inheritance(top, section) {
            return true;
        }
    }

    // 3. Platform-conditional `[target.<cfg>.<deps>]`. The shape is
    //    a table-of-tables: each cfg key maps to a table that may
    //    contain `dependencies` / `dev-dependencies` /
    //    `build-dependencies` sub-tables.
    if let Some(toml::Value::Table(targets)) = top.get("target") {
        for cfg_value in targets.values() {
            let Some(cfg_table) = cfg_value.as_table() else {
                continue;
            };
            for section in ["dependencies", "dev-dependencies", "build-dependencies"] {
                if deps_table_has_inheritance(cfg_table, section) {
                    return true;
                }
            }
        }
    }

    // 4. `[lints]`. The cargo-recognized form is `[lints]
    //    workspace = true` (top-level `workspace` key). We also
    //    defensively scan one level deeper (`[lints.rust].workspace`,
    //    `[lints.clippy].workspace`, etc.) for forward-compat: if
    //    cargo adds per-namespace inheritance, the existing form will
    //    keep being detected here.
    if let Some(toml::Value::Table(lints)) = top.get("lints") {
        // 4a. Top-level form: `[lints] workspace = true`.
        if lints
            .get("workspace")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            return true;
        }
        // 4b. Forward-compat nested form: `[lints.<namespace>] workspace = true`.
        if lints.values().any(is_inheritance_table) {
            return true;
        }
    }

    false
}

/// Splice the synthetic `[package.metadata.lihaaf]` table into `top`.
///
/// Creates the `[package]` and `[package.metadata]` parent tables as
/// needed; replaces any pre-existing `[package.metadata.lihaaf]` entry
/// in full (the v0.1 config loader treats the table as a single typed
/// bundle, so partial merging would produce undefined behavior when the
/// adopter's pre-existing table has different `extern_crates` or
/// `fixture_dirs`).
///
/// The inserted values are typed: `dylib_crate` is a string,
/// `extern_crates` is an array of strings, `fixture_dirs` is an array of
/// strings. These match the v0.1 [`crate::config::RawMetadata`] schema.
fn inject_synthetic_metadata(
    top: &mut toml::map::Map<String, toml::Value>,
    meta: &SyntheticMetadata,
) {
    let package_entry = top
        .entry("package".to_string())
        .or_insert_with(|| toml::Value::Table(toml::map::Map::new()));
    let toml::Value::Table(package) = package_entry else {
        return;
    };
    let metadata_entry = package
        .entry("metadata".to_string())
        .or_insert_with(|| toml::Value::Table(toml::map::Map::new()));
    let toml::Value::Table(metadata) = metadata_entry else {
        return;
    };

    let mut lihaaf_table = toml::map::Map::new();
    lihaaf_table.insert(
        "dylib_crate".to_string(),
        toml::Value::String(meta.dylib_crate.clone()),
    );
    lihaaf_table.insert(
        "extern_crates".to_string(),
        toml::Value::Array(
            meta.extern_crates
                .iter()
                .cloned()
                .map(toml::Value::String)
                .collect(),
        ),
    );
    lihaaf_table.insert(
        "fixture_dirs".to_string(),
        toml::Value::Array(
            meta.fixture_dirs
                .iter()
                .cloned()
                .map(toml::Value::String)
                .collect(),
        ),
    );
    lihaaf_table.insert(
        "allow_lints".to_string(),
        toml::Value::Array(
            meta.allow_lints
                .iter()
                .cloned()
                .map(toml::Value::String)
                .collect(),
        ),
    );

    metadata.insert("lihaaf".to_string(), toml::Value::Table(lihaaf_table));
}

/// Absolutize every path-bearing key in the parsed manifest against
/// `upstream_dir` so the staged overlay (whose parent dir is
/// `<upstream_dir>/target/lihaaf-overlay/`, two levels deeper than the
/// upstream `Cargo.toml`) resolves them correctly.
///
/// **Why this exists.** Cargo resolves every path-bearing manifest key
/// — `[lib] path`, `[[bin]] path`, `[[example]] path`, `[[test]] path`,
/// `[[bench]] path`, `[dependencies.<name>] path`,
/// `[dev-dependencies.<name>] path`, `[build-dependencies.<name>] path`,
/// `[target.*.<deps>] path`, `[workspace] members`, `[workspace] exclude`,
/// `[package] build` — against the parent directory of the manifest
/// being parsed. The staged overlay lives two dirs deeper than the
/// upstream `Cargo.toml`, so any relative path stays attached to its
/// SOURCE intent only after absolutization.
///
/// **Why explicit `[lib] path` injection is load-bearing.** If `[lib]`
/// has no `path` set, cargo defaults to `<manifest_dir>/src/lib.rs` —
/// which for the staged overlay points at the (empty)
/// `<upstream>/target/lihaaf-overlay/src/lib.rs`. We inject
/// `path = "<abs upstream>/src/lib.rs"` so cargo finds the real library.
///
/// **Why we disable auto-discovery for non-lib targets.** Cargo also
/// auto-discovers `src/bin/`, `examples/`, `tests/`, `benches/` under
/// the manifest's parent dir. The staged dir contains only `Cargo.toml`,
/// so auto-discovery would silently produce no targets — but a future
/// cargo version could surface a warning or error. Setting
/// `autobins = false`, `autoexamples = false`, `autotests = false`,
/// `autobenches = false` makes the overlay's "lib-only" intent explicit
/// and forward-compatible.
///
/// **Why `[package] build` is injected when `<upstream>/build.rs` exists.**
/// Cargo auto-discovers `<manifest_dir>/build.rs` when `[package] build`
/// is unset — which for the staged overlay would miss the real build
/// script. We inject `build = "<abs>/build.rs"` so a fork with a build
/// script still compiles correctly under the overlay.
///
/// Idempotent: absolute paths in the input are left unchanged. Missing
/// keys are not invented (except the four `auto*` flags and the
/// implicit-build injection, which are always emitted to make the
/// overlay's intent explicit).
fn absolutize_path_bearing_keys(
    top: &mut toml::map::Map<String, toml::Value>,
    upstream_dir: &Path,
) {
    // Helper: stringify an absolute path with forward-slash separators
    // so the overlay TOML stays cross-platform-stable (Windows
    // backslashes inside a TOML basic string are escape sequences;
    // cargo accepts forward-slash on every platform).
    let to_abs_string = |relative: &str| -> String {
        let joined = upstream_dir.join(relative);
        // `to_string_lossy` is fine here — the upstream path is whatever
        // shape the OS produced for the manifest path the user passed
        // in. We then convert backslashes to forward-slashes for cargo's
        // forward-slash-preferring resolver.
        crate::util::to_forward_slash(&joined.to_string_lossy())
    };

    // Helper: rewrite `table[key]` in place if it is a relative string.
    let absolutize_string_at = |table: &mut toml::map::Map<String, toml::Value>,
                                key: &str,
                                upstream_dir: &Path| {
        if let Some(toml::Value::String(s)) = table.get(key) {
            let p = Path::new(s);
            if !p.is_absolute() {
                let abs = crate::util::to_forward_slash(&upstream_dir.join(p).to_string_lossy());
                table.insert(key.to_string(), toml::Value::String(abs));
            }
        }
    };

    // Helper: iterate a `[[target]]` array (e.g. `[[bin]]`, `[[test]]`)
    // and absolutize each entry's `path` key. Cargo allows both
    // `path = "..."` (relative or absolute) here; relative paths are
    // resolved against the manifest dir.
    let absolutize_array_table_paths =
        |top: &mut toml::map::Map<String, toml::Value>, section: &str, upstream_dir: &Path| {
            if let Some(toml::Value::Array(entries)) = top.get_mut(section) {
                for entry in entries.iter_mut() {
                    if let toml::Value::Table(t) = entry {
                        absolutize_string_at(t, "path", upstream_dir);
                    }
                }
            }
        };

    // Helper: walk a deps table (`[dependencies]` etc.) and absolutize
    // any `path = "..."` sub-key of an inline-table or explicit-table
    // dependency.
    let absolutize_deps_paths =
        |top: &mut toml::map::Map<String, toml::Value>, section: &str, upstream_dir: &Path| {
            if let Some(toml::Value::Table(deps)) = top.get_mut(section) {
                for (_name, dep) in deps.iter_mut() {
                    if let toml::Value::Table(t) = dep {
                        absolutize_string_at(t, "path", upstream_dir);
                    }
                }
            }
        };

    // 1. `[lib] path`. The `[lib]` table is guaranteed to exist by the
    //    caller (`canonicalize_crate_type` ran before us and inserted
    //    the table if absent), so this is an unconditional rewrite.
    //    If `path` is unset, inject the conventional
    //    `<upstream>/src/lib.rs` so cargo doesn't auto-discover against
    //    the empty staged dir.
    if let Some(toml::Value::Table(lib)) = top.get_mut("lib") {
        let needs_inject = !lib.contains_key("path");
        if needs_inject {
            // Conventional default per cargo's auto-discovery rules.
            // We always inject — cargo would otherwise look for
            // `<staged_manifest_dir>/src/lib.rs` and fail to find the
            // library.
            lib.insert(
                "path".to_string(),
                toml::Value::String(to_abs_string("src/lib.rs")),
            );
        } else {
            absolutize_string_at(lib, "path", upstream_dir);
        }
    }

    // 2. `[package] build`. Cargo auto-discovers
    //    `<manifest_dir>/build.rs` when this key is unset — which would
    //    miss the upstream build script. We inject only when
    //    `<upstream>/build.rs` exists, so this is a no-op on most
    //    pilots (none of cxx / serde-json / anyhow / thiserror carry a
    //    build script for the macro crate itself).
    let upstream_build_rs = upstream_dir.join("build.rs");
    if let Some(toml::Value::Table(pkg)) = top.get_mut("package") {
        if pkg.contains_key("build") {
            absolutize_string_at(pkg, "build", upstream_dir);
        } else if upstream_build_rs.is_file() {
            pkg.insert(
                "build".to_string(),
                toml::Value::String(to_abs_string("build.rs")),
            );
        }
    }

    // 3. Explicit `path = "..."` on every `[[bin]]` / `[[example]]` /
    //    `[[test]]` / `[[bench]]` entry. Auto-discovery is disabled
    //    below, but explicit entries still need their paths fixed up.
    absolutize_array_table_paths(top, "bin", upstream_dir);
    absolutize_array_table_paths(top, "example", upstream_dir);
    absolutize_array_table_paths(top, "test", upstream_dir);
    absolutize_array_table_paths(top, "bench", upstream_dir);

    // 4. Disable auto-discovery for non-lib targets. The staged
    //    overlay's parent dir contains only `Cargo.toml`; auto-discovery
    //    would silently produce no targets, but making the overlay's
    //    "lib-only" intent explicit guards against future cargo
    //    versions that might warn or error on the empty case.
    //
    //    We unconditionally write `false` regardless of any pre-existing
    //    value — the overlay's target surface is the lib only, by
    //    construction. The autolib flag is intentionally NOT set because
    //    we explicitly set `[lib] path`, which already overrides auto-
    //    discovery for the lib target.
    if let Some(toml::Value::Table(pkg)) = top.get_mut("package") {
        pkg.insert("autobins".to_string(), toml::Value::Boolean(false));
        pkg.insert("autoexamples".to_string(), toml::Value::Boolean(false));
        pkg.insert("autotests".to_string(), toml::Value::Boolean(false));
        pkg.insert("autobenches".to_string(), toml::Value::Boolean(false));
    }

    // 5. `path = "..."` inside `[dependencies]`, `[dev-dependencies]`,
    //    `[build-dependencies]`. Path-deps are how workspace-style
    //    pilots (cxx's `cxx-build`/`cxx-gen`/etc, thiserror's
    //    `thiserror-impl = { path = "impl" }`) reference sibling
    //    crates; without absolutization the overlay would point cargo
    //    at non-existent dirs under `target/lihaaf-overlay/`.
    absolutize_deps_paths(top, "dependencies", upstream_dir);
    absolutize_deps_paths(top, "dev-dependencies", upstream_dir);
    absolutize_deps_paths(top, "build-dependencies", upstream_dir);

    // 6. Same for the platform-conditional `[target.<cfg>.dependencies]`
    //    family. `target` is a table-of-tables; each inner table has
    //    its own `dependencies` / `dev-dependencies` / `build-dependencies`
    //    sub-tables.
    if let Some(toml::Value::Table(targets)) = top.get_mut("target") {
        for (_cfg, cfg_value) in targets.iter_mut() {
            if let toml::Value::Table(cfg_table) = cfg_value {
                absolutize_deps_paths(cfg_table, "dependencies", upstream_dir);
                absolutize_deps_paths(cfg_table, "dev-dependencies", upstream_dir);
                absolutize_deps_paths(cfg_table, "build-dependencies", upstream_dir);
            }
        }
    }

    // 7. `[workspace] members` / `[workspace] exclude`. These are
    //    string arrays; each entry is a glob or a sub-directory name
    //    relative to the manifest dir. Absolutize each so cargo can
    //    locate workspace members from the staged manifest.
    if let Some(toml::Value::Table(ws)) = top.get_mut("workspace") {
        for key in ["members", "exclude"] {
            if let Some(toml::Value::Array(arr)) = ws.get_mut(key) {
                for entry in arr.iter_mut() {
                    if let toml::Value::String(s) = entry {
                        let p = Path::new(s.as_str());
                        if !p.is_absolute() {
                            let abs = crate::util::to_forward_slash(
                                &upstream_dir.join(p).to_string_lossy(),
                            );
                            *entry = toml::Value::String(abs);
                        }
                    }
                }
            }
        }

        // 7b. `[workspace].default-members` — another string array, same
        //     absolutization semantics as `members`.
        if let Some(toml::Value::Array(arr)) = ws.get_mut("default-members") {
            for entry in arr.iter_mut() {
                if let toml::Value::String(s) = entry {
                    let p = Path::new(s.as_str());
                    if !p.is_absolute() {
                        let abs =
                            crate::util::to_forward_slash(&upstream_dir.join(p).to_string_lossy());
                        *entry = toml::Value::String(abs);
                    }
                }
            }
        }

        // 7c. `[workspace.dependencies.<name>].path` — workspace-inherited
        //     dependency paths.  These have the same shape as the top-level
        //     `[dependencies.X] path` entries handled by `absolutize_deps_paths`,
        //     but live one table level deeper inside `[workspace]`.
        absolutize_deps_paths(ws, "dependencies", upstream_dir);
    }

    // 8. `[package].workspace` — explicit workspace root pointer.  A single
    //    path string; the member crate declares `[package] workspace = "../"` to
    //    point at its containing workspace.  Absolutize so cargo can resolve the
    //    workspace root from the staged manifest dir.
    if let Some(toml::Value::Table(pkg)) = top.get_mut("package") {
        absolutize_string_at(pkg, "workspace", upstream_dir);
    }

    // 9. `[patch.<registry>.X].path` — path-form patch overrides.  For
    //    example, cxx carries `cxx = { path = "." }` and
    //    `cxx-build = { path = "gen/build" }` in `[patch.crates-io]`.
    //    After staging the overlay two dirs deeper, those relative paths
    //    would resolve against the staged manifest dir and either form a
    //    self-reference (`path = "."`) or point at a nonexistent dir.
    //    Only the `path` sub-key is rewritten; `git`, `branch`, `tag`, and
    //    `rev` pass through verbatim per spec §3.2.3.
    absolutize_patch_paths(top, upstream_dir);

    // 10. `[replace."<source-id>"].path` — the older replacement form
    //     (`[patch]` superseded it but `[replace]` is still valid cargo
    //     grammar).  The structure is a flat table where each key is a
    //     source-id string (`"<package_name>:<version>"`) and the value is
    //     a table possibly containing a `path` sub-key.  Without
    //     absolutization, a relative `path = "vendor/cxx"` entry would
    //     resolve against the staged manifest dir — the same failure mode
    //     `[patch]` had (Round-2 FIX class C). Only `path` is rewritten;
    //     `git`, `branch`, `tag`, and `rev` pass through verbatim.
    absolutize_replace_paths(top, upstream_dir);
}

/// Absolutize `[patch.<registry>.X].path` entries in the top-level manifest
/// table.
///
/// `[patch]` is a table-of-registries: each registry key (e.g. `crates-io`)
/// maps to a table of crate overrides, and each override may carry a `path`
/// sub-key.  This function walks all registries and all overrides, absolutizing
/// only the `path` key.  All other sub-keys (`git`, `branch`, `tag`, `rev`, …)
/// are passed through verbatim — this is intentional and matches the spec
/// §3.2.3 promise that `[patch]` remote-source fields are never rewritten.
///
/// Registry-agnostic: the same walk covers `[patch.crates-io]`,
/// `[patch.https://my-registry.example.com/]`, or any other registry key.
fn absolutize_patch_paths(top: &mut toml::map::Map<String, toml::Value>, upstream_dir: &Path) {
    let Some(toml::Value::Table(patch)) = top.get_mut("patch") else {
        return;
    };
    for (_registry, registry_value) in patch.iter_mut() {
        if let toml::Value::Table(registry_table) = registry_value {
            for (_krate, krate_value) in registry_table.iter_mut() {
                if let toml::Value::Table(krate_table) = krate_value {
                    // Only rewrite `path`; leave `git`, `branch`, `tag`, `rev`
                    // untouched per spec §3.2.3.
                    let needs_rewrite = krate_table
                        .get("path")
                        .and_then(|v| v.as_str())
                        .is_some_and(|s| !Path::new(s).is_absolute());
                    if needs_rewrite {
                        let s = krate_table
                            .get("path")
                            .and_then(|v| v.as_str())
                            .expect("needs_rewrite implies path exists");
                        let abs =
                            crate::util::to_forward_slash(&upstream_dir.join(s).to_string_lossy());
                        krate_table.insert("path".to_string(), toml::Value::String(abs));
                    }
                }
            }
        }
    }
}

/// Absolutize `[replace."<source-id>"].path` entries in the top-level manifest
/// table.
///
/// `[replace]` is the older, soft-deprecated replacement form that `[patch]`
/// superseded in Cargo. It is still valid grammar and must be absolutized
/// for the same reason as `[patch]`: relative `path` values would resolve
/// against the staged manifest dir after the overlay is written to
/// `target/lihaaf-overlay/`, not against the upstream crate root.
///
/// Structure: `[replace]` is a flat table where each key is a source-id
/// string (`"<package_name>:<version>"`, e.g. `"cxx:0.3.0"`) and the value
/// is a table possibly containing a `path` sub-key.  Only `path` is
/// rewritten; `git`, `branch`, `tag`, and `rev` pass through verbatim (same
/// policy as `[patch]`).
///
/// This is intentionally a mirror of [`absolutize_patch_paths`] for the
/// simpler (one-level-deep) `[replace]` structure.
fn absolutize_replace_paths(top: &mut toml::map::Map<String, toml::Value>, upstream_dir: &Path) {
    let Some(toml::Value::Table(replace)) = top.get_mut("replace") else {
        return;
    };
    for (_source_id, entry_value) in replace.iter_mut() {
        if let toml::Value::Table(entry_table) = entry_value {
            // Only rewrite `path`; leave `git`, `branch`, `tag`, `rev`
            // untouched (same policy as [patch]).
            let needs_rewrite = entry_table
                .get("path")
                .and_then(|v| v.as_str())
                .is_some_and(|s| !Path::new(s).is_absolute());
            if needs_rewrite {
                let s = entry_table
                    .get("path")
                    .and_then(|v| v.as_str())
                    .expect("needs_rewrite implies path exists");
                let abs = crate::util::to_forward_slash(&upstream_dir.join(s).to_string_lossy());
                entry_table.insert("path".to_string(), toml::Value::String(abs));
            }
        }
    }
}

/// Lexically normalize a path: drop `Component::CurDir` (`.`) entries
/// and preserve every other component (`Normal`, `ParentDir`,
/// `RootDir`, `Prefix`).
///
/// This is the helper Rule 2 (REMAP) detection uses to decide whether
/// the upstream's `[patch.crates-io.<self>].path` entry, when joined
/// against the upstream manifest dir, resolves to the upstream root
/// crate. Two paths are lexically equal iff their component vectors
/// (after `.`-filtering) are equal.
///
/// **Scope:** lexical only. `..` (`Component::ParentDir`) is preserved,
/// not collapsed — collapsing `..` would change semantics on a
/// filesystem with symlinks, and lihaaf is explicit about NOT calling
/// `canonicalize()` here (see [`crate::compat::overlay`] module docs
/// and issue #40/#47 plan §6.11). Symlinked-equivalent paths compare
/// lexically unequal.
///
/// Tests in this module pin the supported equivalences:
/// - `<dir>` == `<dir>/.` (one CurDir filtered)
/// - `<dir>` == `<dir>/` (trailing slash handled by `Path::components`)
/// - `<dir>//<sub>` == `<dir>/<sub>` (repeated separators collapse)
/// - `<dir>/..` != `<dir>` (ParentDir preserved)
/// - real path != symlinked path (no `canonicalize()`)
fn lexical_path_normalize_path(p: &Path) -> Vec<std::path::Component<'_>> {
    p.components()
        .filter(|c| !matches!(c, std::path::Component::CurDir))
        .collect()
}

/// Apply the Option H intent-aware self-patch policy to
/// `[patch.crates-io.<self>]` in the overlay's parsed manifest table.
///
/// `self` is the upstream's `[package].name`, captured by
/// [`read_upstream_crate_name`] before this function runs.
///
/// **Why this exists (issue #40 / #47).** The staged overlay manifest
/// at `<upstream>/target/lihaaf-overlay/Cargo.toml` declares
/// `[package].name = "<self>"` with the upstream's version. From
/// cargo's POV the staged-overlay package lives at a path-source-id
/// distinct from the upstream's path-source-id; without a self-patch
/// redirect, downstream resolution sees two competing sources for the
/// same crate-name+version pair and fails with either:
///
/// - `package <X> links to the native library <L>, but it conflicts
///   with a previous package which links to <L> as well` (cxx-shape,
///   issue #47; fires when any path-dep / workspace member references
///   `<self>` by registry-name AND `<self>` declares `links = "<L>"`),
///   OR
/// - `error: specification <X> is ambiguous` (serde-json-shape, issue
///   #40; fires when any in-graph entity references `<self>` by
///   registry-name).
///
/// The fix injects (or remaps) a `[patch.crates-io.<self>] = { path =
/// "<absolutized staged-overlay-dir>" }` entry so cargo's resolver
/// redirects every "registry <self>" reference to the staged-overlay
/// path-source — the same source-id as the overlay's own `[package]`.
/// The two references then collapse to one Package in the resolved
/// graph; both failure shapes disappear.
///
/// # The Option H 4-rule decision tree
///
/// Rules are mutually exclusive and exhaustive; the first matching
/// rule fires.
///
/// **Rule 1 (INJECT)** — `[patch.crates-io.<self>]` is absent. Insert
/// `{ path = "<absolutized staged-overlay-dir>" }`. Pilots:
/// anyhow / thiserror / serde-json / clean Round-2 candidates.
///
/// **Rule 2 (REMAP)** — `[patch.crates-io.<self>]` is present with a
/// `.path` key and NO `git`/`branch`/`tag`/`rev`, AND the resolved
/// target (path lexically-normalized after joining against the
/// upstream manifest dir) IS the upstream root crate. Replace the
/// entire entry with a clean `{ path = "<absolutized
/// staged-overlay-dir>" }` (matching the §6.1 Rule 2 normative
/// emission). The upstream's "self-patch to root" intent is preserved
/// — translated to the overlay's manifest context, the equivalent
/// root is the staged-overlay-dir. Pilots: cxx (`path = "."`).
///
/// **Rule 3 (CONTINUE-ABSOLUTIZE)** — no-op fallthrough for the
/// `<self>` key. Non-target `[patch.crates-io.<X>]` entries where
/// `<X> != <self>` are NOT touched by this function. The pre-existing
/// [`absolutize_patch_paths`] pass (run before this function) already
/// absolutized those entries against the upstream dir. Documented
/// here so the test surface pins the orthogonality contract.
///
/// **Rule 4 (REJECT)** — `[patch.crates-io.<self>]` is present but the
/// target is external: (a) `.path` resolves to a non-root dir
/// (vendored fork), (b) `git`/`branch`/`tag`/`rev` keys present
/// (registry-name aliased to git source), or (c) both `.path` and
/// `git`/etc. Return [`Error::CompatPatchOverrideConflict`]; the
/// overlay materialization fails fast. The
/// `--compat-allow-patch-override` escape hatch is deferred to v0.2 /
/// v1.1.
///
/// # Why REMAP over PRESERVE-AS-IS
///
/// Cargo anchors `[patch.crates-io.X].path` relative to the manifest
/// declaring the patch (= the staged overlay manifest in our case).
/// Verbatim-preserving `path = "."` from the upstream into the
/// overlay would let cargo re-anchor `.` to the staged-overlay-dir at
/// READ time, which happens to give the correct source-id for the
/// cxx case (`path = "."` resolves to upstream root). But the
/// general case (`path = "../my-fork"` resolves to a sibling dir)
/// would silently misroute under PRESERVE-AS-IS: cargo would
/// re-anchor `..` against `<staged-overlay>/`, NOT `<upstream>/`,
/// producing `<staged-overlay>/../my-fork = <upstream>/target/my-fork`
/// — a dir the adopter never intended. REMAP unifies the emission
/// form across all path-bearing self-patches: every emitted byte
/// shape is the absolutized staged-overlay-dir, robust to cargo /
/// `absolutize_patch_paths` future changes.
///
/// # Ordering
///
/// This function runs AFTER [`absolutize_patch_paths`] and BEFORE
/// [`inject_synthetic_metadata`] / [`override_workspace_inheritance`].
/// Running after `absolutize_patch_paths` means non-self `[patch]`
/// entries (Rule 3 fallthrough) are already absolutized. Rule 2
/// detection re-joins the upstream's `.path` value against
/// `upstream_dir`; `upstream_dir.join("/abs/path")` returns
/// `/abs/path` on Unix (the prefix wins) so the join is correct
/// regardless of whether the value is pre-absolutized.
///
/// # Errors
///
/// Returns [`Error::CompatPatchOverrideConflict`] on Rule 4. Returns
/// `Ok(())` on Rule 1 (INJECT), Rule 2 (REMAP), Rule 3 (no-op for
/// `<self>` key), or when `upstream_crate_name` is `None`.
fn apply_self_patch_policy(
    top: &mut toml::map::Map<String, toml::Value>,
    upstream_crate_name: Option<&str>,
    upstream_dir: &Path,
    staged_overlay_dir: &Path,
) -> Result<(), Error> {
    // Step 1: bail when the upstream has no crate name. Workspace-root
    // manifests are already rejected by `is_workspace_root_manifest`
    // at the materializer's top; this is defense-in-depth for partial
    // / malformed manifests.
    let Some(self_name) = upstream_crate_name else {
        return Ok(());
    };
    if self_name.is_empty() {
        return Ok(());
    }

    // Step 2: compute the absolutized staged-overlay path string,
    // matching the absolutization shape used by every other
    // path-bearing key (forward-slash form via `to_forward_slash`).
    let staged_overlay_abs =
        crate::util::to_forward_slash(&staged_overlay_dir.to_string_lossy());

    // Step 3-4: ensure `top["patch"]` and `top["patch"]["crates-io"]`
    // exist as tables.
    let patch_entry = top
        .entry("patch".to_string())
        .or_insert_with(|| toml::Value::Table(toml::map::Map::new()));
    let toml::Value::Table(patch) = patch_entry else {
        // Defensive: `[patch]` was declared as a non-table value in
        // the upstream. Surface as a TOML parse error; cargo would
        // also reject this.
        return Err(Error::TomlParse {
            path: PathBuf::from("<overlay>"),
            message: "`[patch]` must be a table".to_string(),
        });
    };
    let crates_io_entry = patch
        .entry("crates-io".to_string())
        .or_insert_with(|| toml::Value::Table(toml::map::Map::new()));
    let toml::Value::Table(crates_io) = crates_io_entry else {
        return Err(Error::TomlParse {
            path: PathBuf::from("<overlay>"),
            message: "`[patch.crates-io]` must be a table".to_string(),
        });
    };

    // Step 5: Option H 4-rule dispatch on
    // `top["patch"]["crates-io"][<self>]`.
    match crates_io.get(self_name).cloned() {
        // Rule 1: INJECT. No upstream entry; create a fresh one.
        None => {
            let mut entry = toml::map::Map::new();
            entry.insert(
                "path".to_string(),
                toml::Value::String(staged_overlay_abs),
            );
            crates_io.insert(self_name.to_string(), toml::Value::Table(entry));
            Ok(())
        }
        // Entry present — Rules 2 / 4 dispatch.
        Some(toml::Value::Table(existing_entry)) => {
            let has_git = existing_entry.contains_key("git");
            let has_branch = existing_entry.contains_key("branch");
            let has_tag = existing_entry.contains_key("tag");
            let has_rev = existing_entry.contains_key("rev");
            let any_git_keys = has_git || has_branch || has_tag || has_rev;
            let path_raw = existing_entry.get("path").and_then(|v| v.as_str());

            // Rule 2 fires only when (a) the entry has `.path`,
            // (b) NO git-source keys, AND (c) the joined-and-
            // lexical-normalized path equals the upstream manifest
            // dir.
            if let Some(path_raw) = path_raw
                && !any_git_keys
            {
                let joined = upstream_dir.join(path_raw);
                let joined_normalized = lexical_path_normalize_path(&joined);
                let upstream_normalized = lexical_path_normalize_path(upstream_dir);
                if joined_normalized == upstream_normalized {
                    // Rule 2 REMAP: replace the entire entry with a
                    // clean `{ path = "<staged-overlay-dir>" }`.
                    // Clearing (vs. upsert-path) is intentional per
                    // §6.1 "Overwrite the entry": Rule 2's entry
                    // condition guarantees no git/branch/tag/rev
                    // keys today, but a future cargo manifest key
                    // would otherwise survive untouched. We want a
                    // clean overlay byte shape.
                    let mut entry = toml::map::Map::new();
                    entry.insert(
                        "path".to_string(),
                        toml::Value::String(staged_overlay_abs),
                    );
                    crates_io.insert(self_name.to_string(), toml::Value::Table(entry));
                    return Ok(());
                }
            }

            // Rule 4: REJECT. Falls here on (a) git-source keys
            // present, (b) `.path` resolves to a non-root dir, OR
            // (c) the entry has neither `.path` nor git keys
            // (malformed / empty entry — we are conservative).
            Err(Error::CompatPatchOverrideConflict {
                crate_name: self_name.to_string(),
                upstream_entry: format!("{:?}", toml::Value::Table(existing_entry)),
                expected_resolution: format!(
                    "lihaaf would inject [patch.crates-io.{self_name}] = \
                     {{ path = \"{staged_overlay_abs}\" }} (Rule 1 INJECT) \
                     or remap an upstream self-patch to that path (Rule 2 \
                     REMAP), but the upstream's existing entry declares an \
                     external target (vendored fork, git source, or non-root \
                     path). To opt into overwriting, use \
                     --compat-allow-patch-override (deferred to v0.2 / v1.1; \
                     see issues #40 + #47 for tracking)."
                ),
            })
        }
        // Entry present but not a table (e.g. inline string — invalid
        // for `[patch.crates-io.<X>]`). Reject with the same Rule-4
        // shape so the operator sees the same actionable message.
        Some(other) => Err(Error::CompatPatchOverrideConflict {
            crate_name: self_name.to_string(),
            upstream_entry: format!("{other:?}"),
            expected_resolution: format!(
                "lihaaf would inject [patch.crates-io.{self_name}] = \
                 {{ path = \"{staged_overlay_abs}\" }} (Rule 1 INJECT), \
                 but the upstream's existing entry is not a table — cargo \
                 requires `[patch.crates-io.<X>] = {{ ... }}`."
            ),
        }),
    }
}

/// Top-level upstream entries that the staged package-root mirror MUST
/// NOT touch.
///
/// Each entry falls into one of two categories. The mirror loop reads
/// this list to decide whether an upstream top-level entry should be
/// mirrored; the stale-cleanup pass uses it to decide whether a name
/// it sees in the staged overlay dir is a known excluded entry.
///
/// - **Disposable** (`target`): may or may not be present in the
///   staged overlay; the mirror leaves it alone. `target/` belongs to
///   cargo; mirroring it would either create circular artifact paths
///   or thrash I/O on large projects.
/// - **Must-be-absent-or-removed** (`Cargo.toml`, `Cargo.lock`,
///   `.git`): never mirrored, and if present in the staged overlay
///   from a prior buggy run or manual placement, must be removed by
///   the stale-cleanup pass. `Cargo.toml` is the overlay's own
///   generated manifest (the post-condition assertion guards type);
///   `Cargo.lock` would interfere with cargo's fresh-resolve
///   semantics; `.git` is irrelevant to build-script execution.
///
/// See [`crate::compat::overlay::mirror_upstream_into_overlay`] for
/// the full rule table.
const MIRROR_EXCLUDED_TOP_LEVEL: &[&str] =
    &["target", ".git", "Cargo.toml", "Cargo.lock"];

/// Top-level upstream entries that, if found in the staged overlay
/// dir, the stale-cleanup pass MUST remove (CASE 14b in the §4.5.6
/// rerun-state table).
///
/// `target/` is NOT in this list — it is "disposable" (CASE 14a):
/// neither mirrored nor removed. Only `Cargo.toml` is checked
/// separately by the [`mirror_upstream_into_overlay`] post-condition
/// assertion (it must remain a regular file written by
/// `write_file_atomic`).
const MIRROR_MUST_REMOVE_IF_PRESENT: &[&str] = &[".git", "Cargo.lock"];

/// Populate the staged overlay dir with a structural mirror of the
/// upstream package root: for each non-excluded top-level entry in
/// `<upstream>/`, create a symlink (or copy under fallback) at the
/// matching path under `<staged-overlay>/`.
///
/// # Why this exists (issue #40 / #47, §4.5)
///
/// When cargo builds the overlay package via `cargo rustc
/// --manifest-path <staged-overlay>/Cargo.toml`, it sets
/// `CARGO_MANIFEST_DIR` and the build-script cwd to the staged
/// overlay dir. Build scripts in real upstream pilots access
/// package-root-relative files through that dir:
///
/// - `cxx build.rs`: reads `src/cxx.cc` via `manifest_dir.join(...)`
///   and references `include/cxx.h` — hard error (`No such file or
///   directory`) if the staged dir is empty.
/// - `anyhow build.rs`: probes `Path::new("src").join("nightly.rs")`
///   from cwd — silent-false (returns `false` and disables nightly
///   cfg) if missing.
/// - `thiserror build.rs`: probes `Path::new("build").join("probe.rs")`
///   from cwd — same silent-false hazard.
///
/// The fix is structural: after the overlay manifest is written, this
/// function creates symlinks at each `<staged-overlay>/<entry>` →
/// `<upstream>/<entry>` for every non-excluded `<entry>`. A build
/// script reading `manifest_dir.join("src/cxx.cc")` then follows the
/// symlink and finds the real upstream file.
///
/// # Excluded entries (§4.5.4)
///
/// - `target/` — disposable (CASE 14a); left alone in either direction.
/// - `.git/` — must be absent (CASE 14b); removed if present.
/// - `Cargo.toml` — must remain the overlay's generated regular file
///   (post-condition assertion).
/// - `Cargo.lock` — must be absent (CASE 14b); removed if present.
///
/// # Idempotency contract (Option B, §4.5.6)
///
/// Skip an entry only when the current state is the canonical symlink
/// to the correct `<upstream>/<entry>` (CASE 2). For all other states,
/// reconcile by replacing the stale state with the canonical mirror.
/// 15-case rerun-state table:
///
/// - CASEs 1, 10: absent at destination → create canonical symlink.
/// - CASE 2: canonical symlink already present → skip (idempotent
///   inode-identity guard).
/// - CASE 3: wrong-target symlink → unlink + create canonical.
/// - CASE 4: broken symlink → unlink (and recreate if upstream still
///   present).
/// - CASE 5: real file in staged vs file in upstream → remove + create
///   canonical symlink (symlink mode) or byte-check (copy mode).
/// - CASE 6: real directory in staged vs dir in upstream → remove tree
///   + create canonical symlink (symlink mode) or exact-sync copy
///     (copy mode — MUST remove destination-only files).
/// - CASE 7: type mismatch (file ↔ dir) → remove + create canonical
///   with current type.
/// - CASE 8: manual placement at a mirror-eligible path → replace
///   with canonical (no preservation semantics).
/// - CASE 9: stale entry in staged with no upstream counterpart →
///   remove (forward stale-cleanup).
/// - CASE 11: upstream content changed since prior run → symlink mode
///   passes through; copy mode byte-checks.
/// - CASE 12: mixed partial state → per-entry reconciliation.
/// - CASE 13: entire overlay stale from different upstream →
///   reconcile every entry (per-entry, not per-manifest).
/// - CASE 14a (`target/`): never touched.
/// - CASE 14b (`.git/`, `Cargo.lock`): removed by stale-cleanup if
///   present.
/// - CASE 15: post-condition — `<staged-overlay>/Cargo.toml` must be a
///   regular file, not a symlink. Type-only structural check;
///   manifest content correctness is `write_file_atomic`'s contract.
///
/// # Copy fallback (§4.5.3)
///
/// On platforms / configurations where symlink creation fails
/// (`PermissionDenied`, `Unsupported`, Windows without symlink
/// privilege), each entry falls back to a recursive copy. Copies
/// follow exact-sync semantics for directories (removed-upstream
/// files MUST NOT persist in the staged overlay — see decision 5 of
/// §4.5.6).
///
/// # Errors
///
/// Returns [`Error::OverlayMirrorFailed`] on any I/O failure during
/// symlink creation, copy fallback, stale-state removal, or the
/// CASE 15 post-condition assertion.
fn mirror_upstream_into_overlay(
    upstream_dir: &Path,
    staged_overlay_dir: &Path,
) -> Result<(), Error> {
    // Per-entry forward pass: reconcile every non-excluded top-level
    // upstream entry into the staged overlay dir.
    let upstream_entries = std::fs::read_dir(upstream_dir).map_err(|e| {
        Error::overlay_mirror_failed(
            upstream_dir.to_path_buf(),
            staged_overlay_dir.to_path_buf(),
            "read-upstream-dir",
            Some(e),
        )
    })?;

    let mut upstream_names: std::collections::BTreeSet<String> =
        std::collections::BTreeSet::new();

    for entry_res in upstream_entries {
        let entry = entry_res.map_err(|e| {
            Error::overlay_mirror_failed(
                upstream_dir.to_path_buf(),
                staged_overlay_dir.to_path_buf(),
                "iter-upstream-dir",
                Some(e),
            )
        })?;
        let name_os = entry.file_name();
        let Some(name) = name_os.to_str() else {
            // Skip non-UTF-8 names. The overlay corpus is Linux /
            // macOS; a non-UTF-8 entry is a fork-side anomaly the
            // mirror does not try to reproduce. Document explicitly:
            // such entries are intentionally not mirrored.
            continue;
        };
        upstream_names.insert(name.to_string());

        if MIRROR_EXCLUDED_TOP_LEVEL.contains(&name) {
            // CASE 14a (target/) and the Cargo.toml / Cargo.lock /
            // .git exclusions: do not mirror. The
            // CASE 14b stale-cleanup pass (below) handles the must-
            // be-absent case for `.git` and `Cargo.lock` if they
            // already exist in the staged overlay.
            continue;
        }

        let upstream_path = upstream_dir.join(name);
        let staged_path = staged_overlay_dir.join(name);
        reconcile_one_entry(&upstream_path, &staged_path)?;
    }

    // Stale-cleanup pass (CASE 9 + CASE 14b): remove staged entries
    // that have no upstream counterpart, plus the must-be-absent
    // entries even if they have an upstream counterpart.
    if staged_overlay_dir.is_dir() {
        let staged_iter = std::fs::read_dir(staged_overlay_dir).map_err(|e| {
            Error::overlay_mirror_failed(
                staged_overlay_dir.to_path_buf(),
                staged_overlay_dir.to_path_buf(),
                "read-staged-dir",
                Some(e),
            )
        })?;
        for entry_res in staged_iter {
            let entry = entry_res.map_err(|e| {
                Error::overlay_mirror_failed(
                    staged_overlay_dir.to_path_buf(),
                    staged_overlay_dir.to_path_buf(),
                    "iter-staged-dir",
                    Some(e),
                )
            })?;
            let name_os = entry.file_name();
            let Some(name) = name_os.to_str() else {
                continue;
            };

            // Keep the overlay's generated Cargo.toml (post-condition
            // CASE 15 below asserts type) and the disposable target/.
            if name == "Cargo.toml" || name == "target" {
                continue;
            }

            // CASE 14b: explicit must-be-absent removal even if the
            // upstream carries one.
            if MIRROR_MUST_REMOVE_IF_PRESENT.contains(&name) {
                let stale = staged_overlay_dir.join(name);
                remove_path_any(&stale).map_err(|e| {
                    Error::overlay_mirror_failed(
                        upstream_dir.join(name),
                        stale.clone(),
                        "stale-cleanup-must-absent",
                        Some(e),
                    )
                })?;
                continue;
            }

            // CASE 9: staged entry without an upstream counterpart.
            if !upstream_names.contains(name) {
                let stale = staged_overlay_dir.join(name);
                remove_path_any(&stale).map_err(|e| {
                    Error::overlay_mirror_failed(
                        upstream_dir.join(name),
                        stale.clone(),
                        "stale-cleanup-orphan",
                        Some(e),
                    )
                })?;
            }
        }
    }

    // CASE 15 post-condition: `<staged-overlay>/Cargo.toml` MUST be a
    // regular file, not a symlink. Type-only structural check; content
    // correctness is `write_file_atomic`'s contract (overlay.rs:527-543
    // bytes-match skip path).
    let manifest = staged_overlay_dir.join("Cargo.toml");
    let meta = std::fs::symlink_metadata(&manifest).map_err(|e| {
        Error::overlay_mirror_failed(
            upstream_dir.join("Cargo.toml"),
            manifest.clone(),
            "post-condition-stat",
            Some(e),
        )
    })?;
    if meta.file_type().is_symlink() {
        return Err(Error::overlay_mirror_failed(
            upstream_dir.join("Cargo.toml"),
            manifest.clone(),
            "post-condition-cargo-toml-is-symlink",
            None,
        ));
    }
    if !meta.file_type().is_file() {
        return Err(Error::overlay_mirror_failed(
            upstream_dir.join("Cargo.toml"),
            manifest.clone(),
            "post-condition-cargo-toml-not-regular-file",
            None,
        ));
    }

    Ok(())
}

/// Reconcile a single staged-overlay mirror entry against its upstream
/// counterpart, applying the §4.5.6 Option B per-case decision tree.
///
/// Used by [`mirror_upstream_into_overlay`] for each non-excluded
/// upstream top-level entry; the case classification is per the
/// 15-case rerun-state table (CASEs 1 / 2 / 3 / 4 / 5 / 6 / 7 / 8 in
/// this function; CASE 9 + CASE 14b in the parent function's stale-
/// cleanup pass).
fn reconcile_one_entry(upstream_path: &Path, staged_path: &Path) -> Result<(), Error> {
    // Helper: structured error wrapper for I/O calls below.
    let mirror_err = |stage: &str, e: std::io::Error| {
        Error::overlay_mirror_failed(
            upstream_path.to_path_buf(),
            staged_path.to_path_buf(),
            stage.to_string(),
            Some(e),
        )
    };

    let staged_meta = std::fs::symlink_metadata(staged_path);
    match staged_meta {
        // CASE 1 / CASE 10: staged path absent → create canonical
        // mirror.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            create_canonical_mirror(upstream_path, staged_path)
        }
        Err(e) => Err(mirror_err("stat-staged", e)),
        Ok(meta) => {
            let ftype = meta.file_type();
            if ftype.is_symlink() {
                // Symlink already exists; decide whether it is
                // canonical (CASE 2 skip) or needs reconciliation
                // (CASE 3 wrong-target, CASE 4 broken).
                let link_target = std::fs::read_link(staged_path)
                    .map_err(|e| mirror_err("readlink-staged", e))?;
                // Canonical state: the symlink target matches the
                // upstream path exactly. We compare against the
                // absolute upstream path because `create_canonical_
                // mirror` always emits an absolute target.
                if link_target == upstream_path {
                    // CASE 2: idempotent skip.
                    return Ok(());
                }
                // CASE 3 or CASE 4: stale symlink (wrong target or
                // broken). Unlink and recreate.
                std::fs::remove_file(staged_path)
                    .map_err(|e| mirror_err("stale-symlink-unlink", e))?;
                create_canonical_mirror(upstream_path, staged_path)
            } else if ftype.is_file() {
                // CASE 5 or CASE 7: real file in staged path. If
                // upstream is also a file, this is CASE 5; otherwise
                // CASE 7 (type mismatch). Either way we remove and
                // recreate canonically.
                std::fs::remove_file(staged_path)
                    .map_err(|e| mirror_err("stale-file-remove", e))?;
                create_canonical_mirror(upstream_path, staged_path)
            } else if ftype.is_dir() {
                // CASE 6 or CASE 7: real directory in staged path.
                // Remove and recreate canonically.
                std::fs::remove_dir_all(staged_path)
                    .map_err(|e| mirror_err("stale-dir-remove", e))?;
                create_canonical_mirror(upstream_path, staged_path)
            } else {
                // CASE 8: unrecognised file type (block device, fifo,
                // etc. — extremely unusual at a Cargo package root).
                // Treat as stale and remove.
                std::fs::remove_file(staged_path)
                    .map_err(|e| mirror_err("stale-other-remove", e))?;
                create_canonical_mirror(upstream_path, staged_path)
            }
        }
    }
}

/// Create the canonical mirror entry for one upstream path: a symlink
/// from `staged_path` → `upstream_path`, with copy fallback on
/// platforms / configurations where symlink creation fails.
///
/// The fallback is selected at I/O time per-entry, not by an upfront
/// platform check, because symlink availability is a runtime property
/// (Windows Developer Mode, `nosymlink` mounts, filesystem
/// configuration, container restrictions).
fn create_canonical_mirror(
    upstream_path: &Path,
    staged_path: &Path,
) -> Result<(), Error> {
    let mirror_err = |stage: &str, e: std::io::Error| {
        Error::overlay_mirror_failed(
            upstream_path.to_path_buf(),
            staged_path.to_path_buf(),
            stage.to_string(),
            Some(e),
        )
    };

    // Try symlink first; on failure fall back to a recursive copy.
    match symlink_platform(upstream_path, staged_path) {
        Ok(()) => Ok(()),
        Err(e)
            if matches!(
                e.kind(),
                std::io::ErrorKind::PermissionDenied
                    | std::io::ErrorKind::Unsupported
                    | std::io::ErrorKind::AlreadyExists
            ) =>
        {
            // PermissionDenied / Unsupported: copy fallback.
            // AlreadyExists is treated as a race window — we
            // unlink and retry once via copy fallback to keep the
            // operation idempotent (`reconcile_one_entry` should
            // have cleared the staged path, but a concurrent
            // process might have rewritten it).
            if e.kind() == std::io::ErrorKind::AlreadyExists {
                let _ = std::fs::remove_file(staged_path);
            }
            copy_fallback(upstream_path, staged_path).map_err(|e| mirror_err("copy-fallback", e))
        }
        Err(e) => Err(mirror_err("symlink", e)),
    }
}

/// Recursive copy fallback used when symlink creation is unavailable
/// (§4.5.3).
///
/// Copies one upstream entry (file or directory tree) into the staged
/// overlay. Directory copies are exact-sync: any pre-existing staged
/// subdirectory is removed before the copy to honour decision 5 of
/// the idempotency contract (no merge — destination-only files must
/// not persist).
fn copy_fallback(src: &Path, dst: &Path) -> std::io::Result<()> {
    let meta = std::fs::symlink_metadata(src)?;
    let ftype = meta.file_type();
    if ftype.is_file() {
        // Ensure parent dir exists; on the staged-overlay top level
        // the parent is the staged-overlay-dir itself, but the helper
        // is also used recursively for nested entries below.
        if let Some(parent) = dst.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::copy(src, dst)?;
        Ok(())
    } else if ftype.is_dir() {
        // Exact-sync: if `dst` already exists (e.g. from a partial
        // prior run), remove it before re-copying. Decision 5 of
        // §4.5.6: NO MERGE — destination-only files must not
        // persist.
        if dst.exists() {
            std::fs::remove_dir_all(dst)?;
        }
        std::fs::create_dir_all(dst)?;
        for entry in std::fs::read_dir(src)? {
            let entry = entry?;
            let child_src = entry.path();
            let child_dst = dst.join(entry.file_name());
            copy_fallback(&child_src, &child_dst)?;
        }
        Ok(())
    } else if ftype.is_symlink() {
        // Resolve the symlink target and copy the dereferenced
        // content. Build scripts that read package-root files don't
        // care whether the underlying file came from a symlink; the
        // dereferenced contents are what they read.
        let target_meta = std::fs::metadata(src)?;
        if target_meta.is_file() {
            std::fs::copy(src, dst)?;
        } else {
            // Target is a dir; recurse.
            std::fs::create_dir_all(dst)?;
            for entry in std::fs::read_dir(src)? {
                let entry = entry?;
                let child_src = entry.path();
                let child_dst = dst.join(entry.file_name());
                copy_fallback(&child_src, &child_dst)?;
            }
        }
        Ok(())
    } else {
        // Other (block device, fifo, socket): unrecognised at the
        // Cargo-package level. Surface as an I/O error.
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "copy-fallback: unsupported file type at upstream path",
        ))
    }
}

/// Platform-dispatched symlink creation.
///
/// Unix: always `std::os::unix::fs::symlink` (single-call API).
///
/// Windows: `symlink_dir` for directories, `symlink_file` for files —
/// Windows distinguishes the two at the kernel level. Both may fail
/// with `ERROR_PRIVILEGE_NOT_HELD` (`PermissionDenied` in Rust terms)
/// on machines without Developer Mode; the caller falls back to copy.
#[cfg(unix)]
fn symlink_platform(target: &Path, link: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

/// Windows variant of [`symlink_platform`].
#[cfg(windows)]
fn symlink_platform(target: &Path, link: &Path) -> std::io::Result<()> {
    let meta = std::fs::metadata(target)?;
    if meta.is_dir() {
        std::os::windows::fs::symlink_dir(target, link)
    } else {
        std::os::windows::fs::symlink_file(target, link)
    }
}

/// Remove a filesystem entry regardless of its type (file / dir /
/// symlink). Used by the stale-cleanup pass which may encounter any
/// of these types at a single name.
fn remove_path_any(path: &Path) -> std::io::Result<()> {
    let meta = std::fs::symlink_metadata(path)?;
    let ftype = meta.file_type();
    if ftype.is_dir() {
        std::fs::remove_dir_all(path)
    } else {
        // Files and symlinks both go through `remove_file` — the file-
        // type check above ensured we are not asked to remove a dir
        // through this path.
        std::fs::remove_file(path)
    }
}

/// Return `true` when `value` is a workspace-root manifest: declares a
/// `[workspace]` table AND lacks a top-level `[package]`. The
/// `[workspace.package]` table is INHERITED metadata for member crates
/// (the `package.version.workspace = true` pattern) — its presence does
/// NOT make the manifest itself a buildable library; the actual crate
/// the adopter wants overlayed lives in a member directory.
fn is_workspace_root_manifest(value: &toml::Value) -> bool {
    let Some(top) = value.as_table() else {
        return false;
    };
    let has_workspace = top.get("workspace").is_some_and(|v| v.is_table());
    let has_package = top.get("package").is_some_and(|v| v.is_table());
    has_workspace && !has_package
}

/// Return `true` when the upstream `[lib] crate-type` already contains
/// `dylib`. Used only for envelope classification — the overlay rewrite
/// runs unconditionally.
fn inspect_existing_crate_type(value: &toml::Value) -> bool {
    let Some(lib) = value.get("lib") else {
        return false;
    };
    let Some(ct) = lib.get("crate-type") else {
        return false;
    };
    let Some(arr) = ct.as_array() else {
        return false;
    };
    arr.iter().filter_map(|v| v.as_str()).any(|s| s == "dylib")
}

/// Canonicalize the `[lib] crate-type` array on a `[lib]` table.
///
/// Per §3.2.3 the output array must:
/// - Start with `"dylib"`.
/// - Contain `"rlib"` (so the non-lihaaf `cargo test` baseline still
///   works).
/// - Preserve any other entries (`cdylib`, `staticlib`, …) AFTER the
///   `dylib`/`rlib` pair, in their original order.
///
/// Non-string entries in the input array trigger [`Error::TomlParse`]
/// with a directed diagnostic; downstream code can rely on the output
/// being a homogeneous string array.
pub(crate) fn canonicalize_crate_type(
    table: &mut toml::map::Map<String, toml::Value>,
) -> Result<(), Error> {
    let existing: Vec<String> = match table.get("crate-type") {
        None => Vec::new(),
        Some(toml::Value::Array(arr)) => {
            let mut out = Vec::with_capacity(arr.len());
            for (idx, v) in arr.iter().enumerate() {
                match v.as_str() {
                    Some(s) => out.push(s.to_string()),
                    None => {
                        return Err(Error::TomlParse {
                            path: PathBuf::from("<overlay>"),
                            message: format!(
                                "`[lib] crate-type` element at index {idx} is not a string; \
                                 the overlay accepts only string crate-type entries"
                            ),
                        });
                    }
                }
            }
            out
        }
        Some(other) => {
            return Err(Error::TomlParse {
                path: PathBuf::from("<overlay>"),
                message: format!(
                    "`[lib] crate-type` must be an array of strings, got `{}`",
                    type_name_of(other)
                ),
            });
        }
    };

    // Strategy: a stable interleave that always puts `dylib`/`rlib`
    // first (in that order), then everything else in input order with
    // dups removed. We do NOT alphabetize the long tail — the spec
    // says "preserved verbatim AFTER the dylib/rlib pair".
    let mut out: Vec<String> = Vec::with_capacity(existing.len() + 2);
    out.push("dylib".to_string());
    out.push("rlib".to_string());
    for entry in &existing {
        if entry == "dylib" || entry == "rlib" {
            continue;
        }
        if !out.contains(entry) {
            out.push(entry.clone());
        }
    }

    let array = out.into_iter().map(toml::Value::String).collect::<Vec<_>>();
    table.insert("crate-type".to_string(), toml::Value::Array(array));
    Ok(())
}

/// Cargo's canonical table key order. The long tail (anything not in
/// this slice) is sorted alphabetically when serialized.
///
/// This list is intentionally hardcoded; a configuration option would
/// expand the v0.1 surface for no adopter benefit.
pub(crate) fn canonical_key_order() -> &'static [&'static str] {
    &[
        "package",
        "lib",
        "bin",
        "example",
        "test",
        "bench",
        "dependencies",
        "dev-dependencies",
        "build-dependencies",
        "target",
        "features",
        "patch",
        "replace",
        "profile",
        "workspace",
    ]
}

/// Re-serialize a parsed [`toml::Value`] into bytes with the canonical
/// table order, no comments, no trailing whitespace, LF line endings.
///
/// **Why a custom shim:** `toml = "1"`'s default serializer emits
/// table keys in `BTreeMap` (alphabetical) order, which is NOT the
/// cargo-canonical order the spec mandates. We work around this by
/// serializing each top-level key as its own single-key wrapper
/// (preserving the crate's stable inline-key ordering for inner
/// tables) and concatenating the segments with the canonical order
/// applied.
///
/// **Why the segments are concatenated with `\n` separators:** each
/// `toml::ser::to_string` call ends with `\n`, so prepending another
/// `\n` produces exactly one blank line between sections. Post-
/// processing collapses any accidental triple-newlines back to a
/// single blank line for the byte-determinism guarantee.
pub(crate) fn serialize_canonical(value: &toml::Value) -> Result<Vec<u8>, Error> {
    let top = match value {
        toml::Value::Table(t) => t,
        other => {
            return Err(Error::TomlParse {
                path: PathBuf::from("<overlay>"),
                message: format!(
                    "overlay serializer expected a TOML document (table) at the top level, got `{}`",
                    type_name_of(other)
                ),
            });
        }
    };

    // Build the canonical key sequence: every canonical key that is
    // present in the input, in canonical order, followed by every
    // other key in alphabetical order.
    let mut emitted: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
    let mut order: Vec<String> = Vec::with_capacity(top.len());
    for canonical in canonical_key_order() {
        if top.contains_key(*canonical) {
            order.push((*canonical).to_string());
            emitted.insert(*canonical);
        }
    }
    let mut leftovers: Vec<&String> = top
        .keys()
        .filter(|k| !emitted.contains(k.as_str()))
        .collect();
    leftovers.sort();
    for k in leftovers {
        order.push(k.clone());
    }

    let mut segments: Vec<String> = Vec::with_capacity(order.len());
    for key in &order {
        let v = top.get(key).expect("key came from `top`'s own iteration");
        let mut wrapper = toml::map::Map::new();
        wrapper.insert(key.clone(), v.clone());
        let segment =
            toml::ser::to_string(&toml::Value::Table(wrapper)).map_err(|e: toml::ser::Error| {
                Error::TomlParse {
                    path: PathBuf::from("<overlay>"),
                    message: format!("overlay serializer failed for `{key}`: {e}"),
                }
            })?;
        segments.push(segment);
    }

    let joined = segments.join("\n");
    let normalized = post_process_output(&joined);
    Ok(normalized.into_bytes())
}

/// Apply the §3.2.3 byte-shape invariants:
///
/// - LF line endings only (strip any `\r`).
/// - No trailing whitespace on any line.
/// - Collapse two-or-more consecutive blank lines down to one blank
///   line, so segment concatenation can't produce churning whitespace
///   when one segment ends with an internal blank line.
/// - Exactly one trailing `\n`.
fn post_process_output(input: &str) -> String {
    // First pass: normalize line endings and strip trailing whitespace.
    let mut lines: Vec<&str> = Vec::with_capacity(input.lines().count());
    for line in input.lines() {
        // `lines()` already strips both `\n` and `\r\n`, so the only
        // `\r` we can see is one embedded mid-line (extremely rare in
        // hand-edited TOML). We re-trim per-line to belt-and-suspenders
        // against a Windows-checkout `\r\n` upstream.
        let trimmed = line.trim_end_matches([' ', '\t', '\r']);
        lines.push(trimmed);
    }

    // Second pass: collapse runs of blank lines down to one blank.
    let mut out = String::with_capacity(input.len());
    let mut prev_blank = false;
    for line in &lines {
        let is_blank = line.is_empty();
        if is_blank && prev_blank {
            continue;
        }
        out.push_str(line);
        out.push('\n');
        prev_blank = is_blank;
    }

    // Strip a trailing blank line (we always end with a single `\n`,
    // not a `\n\n`), then make sure exactly one trailing `\n` is
    // present.
    while out.ends_with("\n\n") {
        out.pop();
    }
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out
}

/// Walk the raw TOML bytes once and pull every `#`-prefixed comment
/// out, both line-leading (`# foo`) and trailing (`name = "x" # foo`).
///
/// **Why this is fixed-string, not regex:** spec §6.1 forbids a regex
/// engine in this crate. The scanner is a single byte-stream walk that
/// tracks four kinds of string state — basic (`"..."`), literal
/// (`'...'`), multi-line basic (`"""..."""`), and multi-line literal
/// (`'''...'''`) — so a `#` inside any TOML string form is never
/// recorded as a comment. The multi-line forms are line-spanning, so
/// the walker explicitly cannot be a line-by-line split: a `#` inside
/// `"""..."""` on a continuation line must still be treated as content.
fn scan_dropped_comments(text: &str) -> Vec<String> {
    let bytes = text.as_bytes();
    let mut out: Vec<String> = Vec::new();
    let mut i = 0usize;
    // Mutually exclusive: at most one of these is `true` at any time.
    let mut in_basic = false;
    let mut in_literal = false;
    let mut in_multi_basic = false;
    let mut in_multi_literal = false;

    while i < bytes.len() {
        let b = bytes[i];

        // Inside a multi-line basic string (`"""..."""`). Honors `\`
        // escapes per TOML basic-string rules; the close is the first
        // un-escaped `"""`.
        if in_multi_basic {
            if b == b'\\' && i + 1 < bytes.len() {
                i += 2;
                continue;
            }
            if b == b'"' && i + 2 < bytes.len() && bytes[i + 1] == b'"' && bytes[i + 2] == b'"' {
                in_multi_basic = false;
                i += 3;
                continue;
            }
            i += 1;
            continue;
        }

        // Inside a multi-line literal string (`'''...'''`). No escapes;
        // the close is the first `'''`.
        if in_multi_literal {
            if b == b'\'' && i + 2 < bytes.len() && bytes[i + 1] == b'\'' && bytes[i + 2] == b'\'' {
                in_multi_literal = false;
                i += 3;
                continue;
            }
            i += 1;
            continue;
        }

        // Inside a single-line basic string. Honors `\` escapes; newline
        // closes the scope defensively (TOML forbids unescaped newlines
        // in basic strings, but a malformed input must not strand the
        // scanner in the wrong mode).
        if in_basic {
            if b == b'\\' && i + 1 < bytes.len() {
                i += 2;
                continue;
            }
            if b == b'"' {
                in_basic = false;
                i += 1;
                continue;
            }
            if b == b'\n' {
                in_basic = false;
                i += 1;
                continue;
            }
            i += 1;
            continue;
        }

        // Inside a single-line literal string. No escapes; newline
        // closes defensively (same reasoning as basic strings).
        if in_literal {
            if b == b'\'' {
                in_literal = false;
                i += 1;
                continue;
            }
            if b == b'\n' {
                in_literal = false;
                i += 1;
                continue;
            }
            i += 1;
            continue;
        }

        // Out of any string. Three openers and one comment marker to
        // recognize, plus newline as the boundary that lets `extract`
        // capture per-line comment bodies.
        if b == b'#' {
            let start = i + 1;
            let mut end = start;
            while end < bytes.len() && bytes[end] != b'\n' {
                end += 1;
            }
            // `start..end` is ASCII-safe inside the slice because we
            // only consumed `#` (a single ASCII byte) and stopped at
            // either end-of-input or `\n` (another single ASCII byte);
            // we never split a multibyte UTF-8 codepoint. `text` is
            // valid UTF-8, so the slice is too.
            let body = &text[start..end];
            out.push(body.trim().to_string());
            i = end;
            continue;
        }

        if b == b'"' {
            if i + 2 < bytes.len() && bytes[i + 1] == b'"' && bytes[i + 2] == b'"' {
                in_multi_basic = true;
                i += 3;
                continue;
            }
            in_basic = true;
            i += 1;
            continue;
        }

        if b == b'\'' {
            if i + 2 < bytes.len() && bytes[i + 1] == b'\'' && bytes[i + 2] == b'\'' {
                in_multi_literal = true;
                i += 3;
                continue;
            }
            in_literal = true;
            i += 1;
            continue;
        }

        i += 1;
    }

    out
}

/// Single-line variant kept for the unit tests that exercise the
/// per-line classification logic. Real scanning goes through
/// [`scan_dropped_comments`] which handles multi-line strings.
#[cfg(test)]
fn extract_unquoted_comment(line: &str) -> Option<String> {
    let comments = scan_dropped_comments(line);
    comments.into_iter().next()
}

/// Human-readable name for a [`toml::Value`] variant. Used in error
/// messages so the diagnostic names the actual shape encountered rather
/// than echoing a generic "wrong type".
fn type_name_of(v: &toml::Value) -> &'static str {
    match v {
        toml::Value::String(_) => "string",
        toml::Value::Integer(_) => "integer",
        toml::Value::Float(_) => "float",
        toml::Value::Boolean(_) => "boolean",
        toml::Value::Datetime(_) => "datetime",
        toml::Value::Array(_) => "array",
        toml::Value::Table(_) => "table",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonicalize_inserts_dylib_rlib_when_absent() {
        let mut t = toml::map::Map::new();
        canonicalize_crate_type(&mut t).unwrap();
        let ct = t.get("crate-type").unwrap().as_array().unwrap();
        let strs: Vec<&str> = ct.iter().filter_map(|v| v.as_str()).collect();
        assert_eq!(strs, vec!["dylib", "rlib"]);
    }

    #[test]
    fn canonicalize_prepends_dylib_to_rlib_only() {
        let mut t = toml::map::Map::new();
        t.insert(
            "crate-type".into(),
            toml::Value::Array(vec![toml::Value::String("rlib".into())]),
        );
        canonicalize_crate_type(&mut t).unwrap();
        let ct = t.get("crate-type").unwrap().as_array().unwrap();
        let strs: Vec<&str> = ct.iter().filter_map(|v| v.as_str()).collect();
        assert_eq!(strs, vec!["dylib", "rlib"]);
    }

    #[test]
    fn canonicalize_appends_rlib_when_only_dylib() {
        let mut t = toml::map::Map::new();
        t.insert(
            "crate-type".into(),
            toml::Value::Array(vec![toml::Value::String("dylib".into())]),
        );
        canonicalize_crate_type(&mut t).unwrap();
        let ct = t.get("crate-type").unwrap().as_array().unwrap();
        let strs: Vec<&str> = ct.iter().filter_map(|v| v.as_str()).collect();
        assert_eq!(strs, vec!["dylib", "rlib"]);
    }

    #[test]
    fn canonicalize_preserves_cdylib_after_pair() {
        let mut t = toml::map::Map::new();
        t.insert(
            "crate-type".into(),
            toml::Value::Array(vec![toml::Value::String("cdylib".into())]),
        );
        canonicalize_crate_type(&mut t).unwrap();
        let ct = t.get("crate-type").unwrap().as_array().unwrap();
        let strs: Vec<&str> = ct.iter().filter_map(|v| v.as_str()).collect();
        assert_eq!(strs, vec!["dylib", "rlib", "cdylib"]);
    }

    #[test]
    fn canonicalize_dedups_duplicates() {
        let mut t = toml::map::Map::new();
        t.insert(
            "crate-type".into(),
            toml::Value::Array(vec![
                toml::Value::String("rlib".into()),
                toml::Value::String("dylib".into()),
                toml::Value::String("rlib".into()),
                toml::Value::String("cdylib".into()),
            ]),
        );
        canonicalize_crate_type(&mut t).unwrap();
        let ct = t.get("crate-type").unwrap().as_array().unwrap();
        let strs: Vec<&str> = ct.iter().filter_map(|v| v.as_str()).collect();
        assert_eq!(strs, vec!["dylib", "rlib", "cdylib"]);
    }

    #[test]
    fn canonicalize_rejects_non_string_element() {
        let mut t = toml::map::Map::new();
        t.insert(
            "crate-type".into(),
            toml::Value::Array(vec![toml::Value::Integer(1)]),
        );
        let err = canonicalize_crate_type(&mut t).unwrap_err();
        let s = format!("{err:?}");
        assert!(
            s.contains("not a string"),
            "diagnostic must name the failure: {s}"
        );
    }

    #[test]
    fn canonical_key_order_starts_with_package() {
        assert_eq!(canonical_key_order()[0], "package");
    }

    #[test]
    fn extract_unquoted_comment_strips_leading_hash() {
        assert_eq!(
            extract_unquoted_comment("# a leading comment"),
            Some("a leading comment".into())
        );
    }

    #[test]
    fn extract_unquoted_comment_handles_trailing() {
        assert_eq!(
            extract_unquoted_comment(r#"name = "demo" # trailing"#),
            Some("trailing".into())
        );
    }

    #[test]
    fn extract_unquoted_comment_ignores_hash_inside_string() {
        assert_eq!(
            extract_unquoted_comment(r#"url = "http://example.com/#anchor""#),
            None
        );
    }

    #[test]
    fn extract_unquoted_comment_ignores_hash_inside_single_quote() {
        assert_eq!(extract_unquoted_comment(r#"name = 'foo#bar'"#), None);
    }

    #[test]
    fn scan_ignores_hash_inside_multiline_basic_string() {
        let text = "description = \"\"\"\nline with #notacomment\n\"\"\"\n";
        let comments = scan_dropped_comments(text);
        assert!(
            comments.iter().all(|c| !c.contains("notacomment")),
            "multi-line basic string body must not be classified as a comment; got {comments:?}",
        );
    }

    #[test]
    fn scan_ignores_hash_inside_multiline_literal_string() {
        let text = "description = '''\nline with #stillnotacomment\n'''\n";
        let comments = scan_dropped_comments(text);
        assert!(
            comments.iter().all(|c| !c.contains("stillnotacomment")),
            "multi-line literal string body must not be classified as a comment; got {comments:?}",
        );
    }

    #[test]
    fn scan_recognizes_comment_after_multiline_string_closes() {
        // A trailing `#real comment` after the closing `"""` must still
        // surface — the close pop is load-bearing.
        let text = "description = \"\"\"\nblock\n\"\"\" # real comment\n";
        let comments = scan_dropped_comments(text);
        assert!(
            comments.iter().any(|c| c == "real comment"),
            "comment AFTER the multi-line string close must be captured; got {comments:?}",
        );
        assert!(
            comments.iter().all(|c| !c.contains("block")),
            "multi-line body must never appear as a comment; got {comments:?}",
        );
    }

    #[test]
    fn scan_basic_string_escape_does_not_strand_state() {
        // `"foo \" bar"` is a single basic string; the escaped `"` must
        // not flip the in-string flag off and let a subsequent `#` leak
        // as a comment.
        let text = "name = \"foo \\\" #notacomment\"\n# real\n";
        let comments = scan_dropped_comments(text);
        assert!(
            !comments.iter().any(|c| c.contains("notacomment")),
            "escaped quote inside basic string must keep scanner in-string; got {comments:?}",
        );
        assert!(
            comments.iter().any(|c| c == "real"),
            "comment on the following line must still be captured; got {comments:?}",
        );
    }

    #[test]
    fn post_process_strips_trailing_whitespace() {
        let raw = "foo = 1  \nbar = 2\t\n";
        let out = post_process_output(raw);
        assert!(out.lines().all(|l| !l.ends_with(' ') && !l.ends_with('\t')));
    }

    #[test]
    fn post_process_strips_cr() {
        let raw = "foo = 1\r\nbar = 2\r\n";
        let out = post_process_output(raw);
        assert!(!out.contains('\r'));
    }

    #[test]
    fn post_process_collapses_blank_runs() {
        let raw = "foo = 1\n\n\n\nbar = 2\n";
        let out = post_process_output(raw);
        assert_eq!(out, "foo = 1\n\nbar = 2\n");
    }

    #[test]
    fn serialize_canonical_emits_package_first() {
        let input = r#"
[features]
default = []

[dependencies]
serde = "1"

[package]
name = "demo"
version = "0.1.0"
"#;
        let val: toml::Value = toml::from_str(input).unwrap();
        let bytes = serialize_canonical(&val).unwrap();
        let out = String::from_utf8(bytes).unwrap();
        let first_header = out.lines().find(|l| l.starts_with('[')).unwrap();
        assert_eq!(first_header, "[package]", "got:\n{out}");
    }

    /// **Path absolutization injects `[lib] path` when absent.**
    ///
    /// The staged overlay lives at
    /// `<upstream>/target/lihaaf-overlay/Cargo.toml`. Cargo
    /// auto-discovers `[lib] path = "<manifest_dir>/src/lib.rs"` when
    /// the key is unset; in the staged layout that points at the empty
    /// `target/lihaaf-overlay/src/lib.rs`, which doesn't exist. The
    /// absolutizer injects an absolute path pointing at the upstream
    /// `src/lib.rs` to fix this.
    #[test]
    fn absolutize_injects_lib_path_when_absent() {
        let upstream_dir = Path::new("/work/demo");
        let mut top = toml::map::Map::new();
        let mut lib = toml::map::Map::new();
        lib.insert(
            "crate-type".to_string(),
            toml::Value::Array(vec![toml::Value::String("dylib".into())]),
        );
        top.insert("lib".to_string(), toml::Value::Table(lib));
        let mut pkg = toml::map::Map::new();
        pkg.insert("name".to_string(), toml::Value::String("demo".into()));
        top.insert("package".to_string(), toml::Value::Table(pkg));

        absolutize_path_bearing_keys(&mut top, upstream_dir);

        let lib = top.get("lib").and_then(|v| v.as_table()).unwrap();
        let path = lib.get("path").and_then(|v| v.as_str()).unwrap();
        assert_eq!(
            path, "/work/demo/src/lib.rs",
            "[lib] path must be the absolute upstream src/lib.rs; got `{path}`"
        );
    }

    /// **Path absolutization preserves an absolute `[lib] path` that
    /// the upstream already declared.**
    ///
    /// If the upstream manifest already declared
    /// `[lib] path = "/some/absolute/path"`, the absolutizer must leave
    /// it alone — `is_absolute()` is true, so `upstream_dir.join(p)`
    /// would no-op on POSIX (Path::join returns the absolute right-hand
    /// side verbatim).
    #[test]
    fn absolutize_leaves_absolute_lib_path_unchanged() {
        let upstream_dir = Path::new("/work/demo");
        let mut top = toml::map::Map::new();
        let mut lib = toml::map::Map::new();
        lib.insert(
            "crate-type".to_string(),
            toml::Value::Array(vec![toml::Value::String("dylib".into())]),
        );
        lib.insert(
            "path".to_string(),
            toml::Value::String("/elsewhere/src/lib.rs".into()),
        );
        top.insert("lib".to_string(), toml::Value::Table(lib));

        absolutize_path_bearing_keys(&mut top, upstream_dir);

        let lib = top.get("lib").and_then(|v| v.as_table()).unwrap();
        let path = lib.get("path").and_then(|v| v.as_str()).unwrap();
        assert_eq!(
            path, "/elsewhere/src/lib.rs",
            "an absolute [lib] path must be preserved; got `{path}`"
        );
    }

    /// **Path absolutization rewrites a relative `[lib] path`.**
    #[test]
    fn absolutize_rewrites_relative_lib_path() {
        let upstream_dir = Path::new("/work/demo");
        let mut top = toml::map::Map::new();
        let mut lib = toml::map::Map::new();
        lib.insert(
            "crate-type".to_string(),
            toml::Value::Array(vec![toml::Value::String("dylib".into())]),
        );
        lib.insert(
            "path".to_string(),
            toml::Value::String("custom/lib.rs".into()),
        );
        top.insert("lib".to_string(), toml::Value::Table(lib));

        absolutize_path_bearing_keys(&mut top, upstream_dir);

        let lib = top.get("lib").and_then(|v| v.as_table()).unwrap();
        let path = lib.get("path").and_then(|v| v.as_str()).unwrap();
        assert_eq!(
            path, "/work/demo/custom/lib.rs",
            "a relative [lib] path must be absolutized against upstream_dir; got `{path}`"
        );
    }

    /// **Path absolutization rewrites `[dependencies.X].path`.**
    #[test]
    fn absolutize_rewrites_dependencies_path() {
        let upstream_dir = Path::new("/work/demo");
        let mut top = toml::map::Map::new();
        let mut lib = toml::map::Map::new();
        lib.insert(
            "crate-type".to_string(),
            toml::Value::Array(vec![toml::Value::String("dylib".into())]),
        );
        top.insert("lib".to_string(), toml::Value::Table(lib));
        let mut deps = toml::map::Map::new();
        let mut inner = toml::map::Map::new();
        inner.insert("path".to_string(), toml::Value::String("impl".into()));
        deps.insert("inner-impl".to_string(), toml::Value::Table(inner));
        top.insert("dependencies".to_string(), toml::Value::Table(deps));

        absolutize_path_bearing_keys(&mut top, upstream_dir);

        let deps = top.get("dependencies").and_then(|v| v.as_table()).unwrap();
        let inner = deps.get("inner-impl").and_then(|v| v.as_table()).unwrap();
        let path = inner.get("path").and_then(|v| v.as_str()).unwrap();
        assert_eq!(path, "/work/demo/impl");
    }

    /// **Path absolutization rewrites `[target.cfg.dependencies.X].path`.**
    ///
    /// Platform-conditional deps are a common shape in cross-platform
    /// crates; the absolutizer must walk into the `[target.*.<deps>]`
    /// sub-tables the same way it walks the top-level deps tables.
    #[test]
    fn absolutize_rewrites_target_conditional_dependencies_path() {
        let upstream_dir = Path::new("/work/demo");
        let mut top = toml::map::Map::new();
        let mut lib = toml::map::Map::new();
        lib.insert(
            "crate-type".to_string(),
            toml::Value::Array(vec![toml::Value::String("dylib".into())]),
        );
        top.insert("lib".to_string(), toml::Value::Table(lib));
        let mut targets = toml::map::Map::new();
        let mut linux = toml::map::Map::new();
        let mut deps = toml::map::Map::new();
        let mut platform_dep = toml::map::Map::new();
        platform_dep.insert("path".to_string(), toml::Value::String("linux-impl".into()));
        deps.insert(
            "platform-bits".to_string(),
            toml::Value::Table(platform_dep),
        );
        linux.insert("dependencies".to_string(), toml::Value::Table(deps));
        targets.insert(
            r#"cfg(target_os = "linux")"#.to_string(),
            toml::Value::Table(linux),
        );
        top.insert("target".to_string(), toml::Value::Table(targets));

        absolutize_path_bearing_keys(&mut top, upstream_dir);

        let targets = top.get("target").and_then(|v| v.as_table()).unwrap();
        let linux = targets
            .get(r#"cfg(target_os = "linux")"#)
            .and_then(|v| v.as_table())
            .unwrap();
        let deps = linux
            .get("dependencies")
            .and_then(|v| v.as_table())
            .unwrap();
        let platform_dep = deps
            .get("platform-bits")
            .and_then(|v| v.as_table())
            .unwrap();
        let path = platform_dep.get("path").and_then(|v| v.as_str()).unwrap();
        assert_eq!(
            path, "/work/demo/linux-impl",
            "[target.*.dependencies.X].path must be absolutized; got `{path}`"
        );
    }

    /// **Path absolutization rewrites `[workspace] members` and
    /// `[workspace] exclude`.**
    #[test]
    fn absolutize_rewrites_workspace_members_and_exclude() {
        let upstream_dir = Path::new("/work/demo");
        let mut top = toml::map::Map::new();
        let mut lib = toml::map::Map::new();
        lib.insert(
            "crate-type".to_string(),
            toml::Value::Array(vec![toml::Value::String("dylib".into())]),
        );
        top.insert("lib".to_string(), toml::Value::Table(lib));
        let mut pkg = toml::map::Map::new();
        pkg.insert("name".to_string(), toml::Value::String("demo".into()));
        top.insert("package".to_string(), toml::Value::Table(pkg));
        let mut ws = toml::map::Map::new();
        ws.insert(
            "members".to_string(),
            toml::Value::Array(vec![
                toml::Value::String("crate-a".into()),
                toml::Value::String("crate-b".into()),
                // Already-absolute entry stays untouched.
                toml::Value::String("/elsewhere/crate-c".into()),
            ]),
        );
        ws.insert(
            "exclude".to_string(),
            toml::Value::Array(vec![toml::Value::String("scratch".into())]),
        );
        top.insert("workspace".to_string(), toml::Value::Table(ws));

        absolutize_path_bearing_keys(&mut top, upstream_dir);

        let ws = top.get("workspace").and_then(|v| v.as_table()).unwrap();
        let members = ws.get("members").and_then(|v| v.as_array()).unwrap();
        let member_strs: Vec<&str> = members.iter().filter_map(|v| v.as_str()).collect();
        assert_eq!(
            member_strs,
            vec![
                "/work/demo/crate-a",
                "/work/demo/crate-b",
                "/elsewhere/crate-c"
            ],
            "[workspace] members must be absolutized, leaving already-absolute entries alone"
        );
        let exclude = ws.get("exclude").and_then(|v| v.as_array()).unwrap();
        let exclude_strs: Vec<&str> = exclude.iter().filter_map(|v| v.as_str()).collect();
        assert_eq!(exclude_strs, vec!["/work/demo/scratch"]);
    }

    /// **Path absolutization injects `[package] build` only when
    /// `<upstream>/build.rs` exists.**
    #[test]
    fn absolutize_does_not_inject_build_when_upstream_has_no_build_rs() {
        let upstream_dir = Path::new("/work/demo-no-build-rs");
        let mut top = toml::map::Map::new();
        let mut lib = toml::map::Map::new();
        lib.insert(
            "crate-type".to_string(),
            toml::Value::Array(vec![toml::Value::String("dylib".into())]),
        );
        top.insert("lib".to_string(), toml::Value::Table(lib));
        let mut pkg = toml::map::Map::new();
        pkg.insert("name".to_string(), toml::Value::String("demo".into()));
        top.insert("package".to_string(), toml::Value::Table(pkg));

        absolutize_path_bearing_keys(&mut top, upstream_dir);

        let pkg = top.get("package").and_then(|v| v.as_table()).unwrap();
        assert!(
            !pkg.contains_key("build"),
            "build key must not be injected when no upstream build.rs exists; \
             got pkg keys {:?}",
            pkg.keys().collect::<Vec<_>>()
        );
    }

    /// **Path absolutization disables auto-discovery for non-lib targets.**
    ///
    /// The staged overlay's parent dir contains only `Cargo.toml`;
    /// auto-discovery would find no `[[bin]]` / `[[test]]` /
    /// `[[example]]` / `[[bench]]` targets but a future cargo version
    /// could surface a warning or error on the empty case. The
    /// absolutizer always writes `autoX = false` to make the
    /// "lib-only" intent explicit.
    #[test]
    fn absolutize_disables_non_lib_auto_discovery() {
        let upstream_dir = Path::new("/work/demo");
        let mut top = toml::map::Map::new();
        let mut lib = toml::map::Map::new();
        lib.insert(
            "crate-type".to_string(),
            toml::Value::Array(vec![toml::Value::String("dylib".into())]),
        );
        top.insert("lib".to_string(), toml::Value::Table(lib));
        let mut pkg = toml::map::Map::new();
        pkg.insert("name".to_string(), toml::Value::String("demo".into()));
        top.insert("package".to_string(), toml::Value::Table(pkg));

        absolutize_path_bearing_keys(&mut top, upstream_dir);

        let pkg = top.get("package").and_then(|v| v.as_table()).unwrap();
        for key in ["autobins", "autoexamples", "autotests", "autobenches"] {
            let val = pkg.get(key).and_then(|v| v.as_bool());
            assert_eq!(
                val,
                Some(false),
                "[package] {key} must be `false` to disable cargo auto-discovery; \
                 got {val:?}",
            );
        }
    }

    /// **Path absolutization rewrites explicit `[[bin]] path`,
    /// `[[example]] path`, `[[test]] path`, and `[[bench]] path`
    /// entries.**
    ///
    /// Auto-discovery is disabled, but a manifest may declare these
    /// targets explicitly via array-of-tables; relative paths still
    /// need absolutization so cargo's manifest parser doesn't error
    /// even when the lib-only build won't use them.
    #[test]
    fn absolutize_rewrites_array_table_paths() {
        let upstream_dir = Path::new("/work/demo");
        let mut top = toml::map::Map::new();
        let mut lib = toml::map::Map::new();
        lib.insert(
            "crate-type".to_string(),
            toml::Value::Array(vec![toml::Value::String("dylib".into())]),
        );
        top.insert("lib".to_string(), toml::Value::Table(lib));

        for (section, value) in [
            ("bin", "src/bin/foo.rs"),
            ("example", "examples/eg.rs"),
            ("test", "tests/it.rs"),
            ("bench", "benches/bench.rs"),
        ] {
            let mut entry = toml::map::Map::new();
            entry.insert("name".to_string(), toml::Value::String("target".into()));
            entry.insert("path".to_string(), toml::Value::String(value.into()));
            top.insert(
                section.to_string(),
                toml::Value::Array(vec![toml::Value::Table(entry)]),
            );
        }

        absolutize_path_bearing_keys(&mut top, upstream_dir);

        for (section, original) in [
            ("bin", "src/bin/foo.rs"),
            ("example", "examples/eg.rs"),
            ("test", "tests/it.rs"),
            ("bench", "benches/bench.rs"),
        ] {
            let arr = top.get(section).and_then(|v| v.as_array()).unwrap();
            let entry = arr[0].as_table().unwrap();
            let path = entry.get("path").and_then(|v| v.as_str()).unwrap();
            let expected = format!("/work/demo/{original}");
            assert_eq!(
                path, expected,
                "[[{section}]] path must be absolutized to `{expected}`; got `{path}`"
            );
        }
    }

    // ── FIX class B unit tests ──────────────────────────────────────────────

    /// **`[package].workspace` explicit pointer is absolutized.**
    ///
    /// A member crate may declare `[package] workspace = "../"` to name its
    /// containing workspace root explicitly.  Without absolutization the
    /// staged overlay would carry a relative pointer that cargo resolves
    /// against the staged manifest dir — two dirs deeper than the crate root.
    #[test]
    fn absolutizes_package_workspace_pointer() {
        let upstream_dir = Path::new("/work/cxx");
        let mut top = toml::map::Map::new();
        let mut lib = toml::map::Map::new();
        lib.insert(
            "crate-type".to_string(),
            toml::Value::Array(vec![toml::Value::String("rlib".into())]),
        );
        top.insert("lib".to_string(), toml::Value::Table(lib));
        let mut pkg = toml::map::Map::new();
        pkg.insert("name".to_string(), toml::Value::String("cxx".into()));
        // Relative workspace pointer — the production shape.
        pkg.insert("workspace".to_string(), toml::Value::String("../".into()));
        top.insert("package".to_string(), toml::Value::Table(pkg));

        absolutize_path_bearing_keys(&mut top, upstream_dir);

        let pkg = top.get("package").and_then(|v| v.as_table()).unwrap();
        let ws_ptr = pkg.get("workspace").and_then(|v| v.as_str()).unwrap();
        // `Path::join` does not normalize: `..` and `.` are preserved in the output
        // (use canonicalize() for normalization). Cargo's manifest resolver treats
        // `/work/cxx/.` and `/work/cxx/../` as equivalent to `/work/cxx` and the parent
        // dir respectively. Verified end-to-end by `cargo_accepts_rich_overlay_for_dylib_build`.
        assert_eq!(
            ws_ptr, "/work/cxx/../",
            "[package].workspace must be absolutized as Path::join (no normalization); got `{ws_ptr}`"
        );
    }

    /// **`[workspace].default-members` array entries are absolutized.**
    ///
    /// `default-members` is an array of paths (strings), parallel to
    /// `members` and `exclude`.  The existing `members`/`exclude` rewrite
    /// was already tested; this test pins the extension to `default-members`.
    #[test]
    fn absolutizes_workspace_default_members() {
        let upstream_dir = Path::new("/work/repo");
        let mut top = toml::map::Map::new();
        let mut lib = toml::map::Map::new();
        lib.insert(
            "crate-type".to_string(),
            toml::Value::Array(vec![toml::Value::String("rlib".into())]),
        );
        top.insert("lib".to_string(), toml::Value::Table(lib));
        let mut pkg = toml::map::Map::new();
        pkg.insert("name".to_string(), toml::Value::String("repo".into()));
        top.insert("package".to_string(), toml::Value::Table(pkg));
        let mut ws = toml::map::Map::new();
        ws.insert(
            "default-members".to_string(),
            toml::Value::Array(vec![
                toml::Value::String("crate-a".into()),
                toml::Value::String("crate-b".into()),
            ]),
        );
        top.insert("workspace".to_string(), toml::Value::Table(ws));

        absolutize_path_bearing_keys(&mut top, upstream_dir);

        let ws = top.get("workspace").and_then(|v| v.as_table()).unwrap();
        let dm = ws
            .get("default-members")
            .and_then(|v| v.as_array())
            .unwrap();
        let dm_strs: Vec<&str> = dm.iter().filter_map(|v| v.as_str()).collect();
        assert_eq!(
            dm_strs,
            vec!["/work/repo/crate-a", "/work/repo/crate-b"],
            "[workspace].default-members must be absolutized; got {dm_strs:?}"
        );
    }

    /// **`[workspace.dependencies.<name>].path` entries are absolutized.**
    ///
    /// Workspace-inherited dependency paths (e.g. `[workspace.dependencies]
    /// my-dep = { path = "impl" }`) have the same shape as top-level dep
    /// entries and require the same absolutization so cargo can locate
    /// them from the staged manifest dir.
    #[test]
    fn absolutizes_workspace_dependencies_path() {
        let upstream_dir = Path::new("/work/monorepo");
        let mut top = toml::map::Map::new();
        let mut lib = toml::map::Map::new();
        lib.insert(
            "crate-type".to_string(),
            toml::Value::Array(vec![toml::Value::String("rlib".into())]),
        );
        top.insert("lib".to_string(), toml::Value::Table(lib));
        let mut pkg = toml::map::Map::new();
        pkg.insert("name".to_string(), toml::Value::String("monorepo".into()));
        top.insert("package".to_string(), toml::Value::Table(pkg));
        let mut ws = toml::map::Map::new();
        let mut ws_deps = toml::map::Map::new();
        let mut impl_dep = toml::map::Map::new();
        impl_dep.insert("path".to_string(), toml::Value::String("impl".into()));
        ws_deps.insert("my-impl".to_string(), toml::Value::Table(impl_dep));
        let mut proc_macro_dep = toml::map::Map::new();
        proc_macro_dep.insert("path".to_string(), toml::Value::String("proc-macro".into()));
        ws_deps.insert(
            "my-proc-macro".to_string(),
            toml::Value::Table(proc_macro_dep),
        );
        ws.insert("dependencies".to_string(), toml::Value::Table(ws_deps));
        top.insert("workspace".to_string(), toml::Value::Table(ws));

        absolutize_path_bearing_keys(&mut top, upstream_dir);

        let ws = top.get("workspace").and_then(|v| v.as_table()).unwrap();
        let ws_deps = ws.get("dependencies").and_then(|v| v.as_table()).unwrap();
        let impl_path = ws_deps
            .get("my-impl")
            .and_then(|v| v.as_table())
            .and_then(|t| t.get("path"))
            .and_then(|v| v.as_str())
            .unwrap();
        assert_eq!(
            impl_path, "/work/monorepo/impl",
            "[workspace.dependencies.my-impl].path must be absolutized; got `{impl_path}`"
        );
        let pm_path = ws_deps
            .get("my-proc-macro")
            .and_then(|v| v.as_table())
            .and_then(|t| t.get("path"))
            .and_then(|v| v.as_str())
            .unwrap();
        assert_eq!(
            pm_path, "/work/monorepo/proc-macro",
            "[workspace.dependencies.my-proc-macro].path must be absolutized; got `{pm_path}`"
        );
    }

    // ── FIX class C unit tests ──────────────────────────────────────────────

    /// **`[patch.<registry>.X].path` entries are absolutized.**
    ///
    /// Mirrors the cxx pilot shape: `[patch.crates-io] cxx = { path = "." }`
    /// and `cxx-build = { path = "gen/build" }`.  After staging the overlay
    /// two dirs deeper, those relative paths would resolve against the staged
    /// manifest dir and fail.  The absolutizer must rewrite `path` but leave
    /// `git`, `branch`, `tag`, and `rev` untouched.
    #[test]
    fn absolutizes_patch_registry_path() {
        let upstream_dir = Path::new("/work/cxx");
        let mut top = toml::map::Map::new();
        let mut lib = toml::map::Map::new();
        lib.insert(
            "crate-type".to_string(),
            toml::Value::Array(vec![toml::Value::String("rlib".into())]),
        );
        top.insert("lib".to_string(), toml::Value::Table(lib));
        let mut pkg = toml::map::Map::new();
        pkg.insert("name".to_string(), toml::Value::String("cxx".into()));
        top.insert("package".to_string(), toml::Value::Table(pkg));

        // Build [patch.crates-io] with two path-form entries (mirrors cxx).
        let mut cxx_entry = toml::map::Map::new();
        cxx_entry.insert("path".to_string(), toml::Value::String(".".into()));
        let mut cxx_build_entry = toml::map::Map::new();
        cxx_build_entry.insert("path".to_string(), toml::Value::String("gen/build".into()));
        // Also include a git-form entry to verify it is NOT touched.
        let mut serde_entry = toml::map::Map::new();
        serde_entry.insert(
            "git".to_string(),
            toml::Value::String("https://github.com/serde-rs/serde".into()),
        );
        serde_entry.insert("branch".to_string(), toml::Value::String("master".into()));

        let mut crates_io = toml::map::Map::new();
        crates_io.insert("cxx".to_string(), toml::Value::Table(cxx_entry));
        crates_io.insert("cxx-build".to_string(), toml::Value::Table(cxx_build_entry));
        crates_io.insert("serde".to_string(), toml::Value::Table(serde_entry));

        let mut patch = toml::map::Map::new();
        patch.insert("crates-io".to_string(), toml::Value::Table(crates_io));
        top.insert("patch".to_string(), toml::Value::Table(patch));

        absolutize_path_bearing_keys(&mut top, upstream_dir);

        let patch = top.get("patch").and_then(|v| v.as_table()).unwrap();
        let crates_io = patch.get("crates-io").and_then(|v| v.as_table()).unwrap();

        // cxx path = "." → "/work/cxx/." (Path::join preserves `.`; cargo treats
        // `/work/cxx/.` as equivalent to `/work/cxx`).
        let cxx_path = crates_io
            .get("cxx")
            .and_then(|v| v.as_table())
            .and_then(|t| t.get("path"))
            .and_then(|v| v.as_str())
            .unwrap();
        assert_eq!(
            cxx_path, "/work/cxx/.",
            "[patch.crates-io.cxx].path absolutized via Path::join preserves the `.`; \
             cargo treats `/work/cxx/.` as equivalent to `/work/cxx`; got `{cxx_path}`"
        );

        // cxx-build path = "gen/build" → "/work/cxx/gen/build"
        let cxx_build_path = crates_io
            .get("cxx-build")
            .and_then(|v| v.as_table())
            .and_then(|t| t.get("path"))
            .and_then(|v| v.as_str())
            .unwrap();
        assert_eq!(
            cxx_build_path, "/work/cxx/gen/build",
            "[patch.crates-io.cxx-build].path must be absolutized; got `{cxx_build_path}`"
        );

        // serde entry has no `path` key — must be unchanged.
        let serde = crates_io.get("serde").and_then(|v| v.as_table()).unwrap();
        assert!(
            !serde.contains_key("path"),
            "git-form patch entry must not gain a path key"
        );
        assert_eq!(
            serde.get("git").and_then(|v| v.as_str()),
            Some("https://github.com/serde-rs/serde"),
            "git URL in git-form patch entry must be unchanged"
        );
        assert_eq!(
            serde.get("branch").and_then(|v| v.as_str()),
            Some("master"),
            "branch in git-form patch entry must be unchanged"
        );
    }

    /// **An already-absolute `[patch.<registry>.X].path` is left unchanged.**
    #[test]
    fn absolutize_leaves_absolute_patch_path_unchanged() {
        let upstream_dir = Path::new("/work/cxx");
        let mut top = toml::map::Map::new();
        let mut lib = toml::map::Map::new();
        lib.insert(
            "crate-type".to_string(),
            toml::Value::Array(vec![toml::Value::String("rlib".into())]),
        );
        top.insert("lib".to_string(), toml::Value::Table(lib));
        let mut pkg = toml::map::Map::new();
        pkg.insert("name".to_string(), toml::Value::String("cxx".into()));
        top.insert("package".to_string(), toml::Value::Table(pkg));

        let mut abs_entry = toml::map::Map::new();
        abs_entry.insert(
            "path".to_string(),
            toml::Value::String("/absolute/path/to/cxx".into()),
        );
        let mut crates_io = toml::map::Map::new();
        crates_io.insert("cxx".to_string(), toml::Value::Table(abs_entry));
        let mut patch = toml::map::Map::new();
        patch.insert("crates-io".to_string(), toml::Value::Table(crates_io));
        top.insert("patch".to_string(), toml::Value::Table(patch));

        absolutize_path_bearing_keys(&mut top, upstream_dir);

        let path = top
            .get("patch")
            .and_then(|v| v.as_table())
            .and_then(|t| t.get("crates-io"))
            .and_then(|v| v.as_table())
            .and_then(|t| t.get("cxx"))
            .and_then(|v| v.as_table())
            .and_then(|t| t.get("path"))
            .and_then(|v| v.as_str())
            .unwrap();
        assert_eq!(
            path, "/absolute/path/to/cxx",
            "an absolute [patch.*.*].path must be left unchanged; got `{path}`"
        );
    }

    // ── FIX class IV unit tests ─────────────────────────────────────────────

    /// **`[replace."<source-id>"].path` entries are absolutized.**
    ///
    /// `[replace]` is cargo's older, soft-deprecated replacement form.
    /// Its structure differs from `[patch]`: the keys are source-id strings
    /// (`"<name>:<version>"`) rather than crate names under a registry table.
    /// Without absolutization, a `path = "vendor/cxx"` entry would resolve
    /// against the staged manifest dir after overlay materialization — the
    /// same failure mode `[patch]` had before Round-2 FIX class C.
    ///
    /// This test would fail if `absolutize_replace_paths` were removed from
    /// `absolutize_path_bearing_keys`.
    #[test]
    fn absolutizes_replace_path() {
        let upstream_dir = Path::new("/work/project");
        let mut top = toml::map::Map::new();
        let mut lib = toml::map::Map::new();
        lib.insert(
            "crate-type".to_string(),
            toml::Value::Array(vec![toml::Value::String("rlib".into())]),
        );
        top.insert("lib".to_string(), toml::Value::Table(lib));
        let mut pkg = toml::map::Map::new();
        pkg.insert("name".to_string(), toml::Value::String("project".into()));
        top.insert("package".to_string(), toml::Value::Table(pkg));

        // A path-form [replace] entry (source-id key, path-dep value).
        let mut cxx_entry = toml::map::Map::new();
        cxx_entry.insert("path".to_string(), toml::Value::String("vendor/cxx".into()));

        // A git-form [replace] entry — must be left untouched.
        let mut serde_entry = toml::map::Map::new();
        serde_entry.insert(
            "git".to_string(),
            toml::Value::String("https://github.com/serde-rs/serde".into()),
        );
        serde_entry.insert("rev".to_string(), toml::Value::String("abc123".into()));

        // An already-absolute path — must be left unchanged.
        let mut abs_entry = toml::map::Map::new();
        abs_entry.insert(
            "path".to_string(),
            toml::Value::String("/pre-existing/absolute/path".into()),
        );

        let mut replace = toml::map::Map::new();
        replace.insert("cxx:0.3.0".to_string(), toml::Value::Table(cxx_entry));
        replace.insert("serde:1.0.0".to_string(), toml::Value::Table(serde_entry));
        replace.insert("abs-dep:0.1.0".to_string(), toml::Value::Table(abs_entry));
        top.insert("replace".to_string(), toml::Value::Table(replace));

        absolutize_path_bearing_keys(&mut top, upstream_dir);

        let replace_out = top.get("replace").and_then(|v| v.as_table()).unwrap();

        // path-form entry: "vendor/cxx" → "/work/project/vendor/cxx"
        let cxx_path = replace_out
            .get("cxx:0.3.0")
            .and_then(|v| v.as_table())
            .and_then(|t| t.get("path"))
            .and_then(|v| v.as_str())
            .unwrap();
        assert_eq!(
            cxx_path, "/work/project/vendor/cxx",
            "[replace.\"cxx:0.3.0\"].path must be absolutized; got `{cxx_path}`"
        );

        // git-form entry: no `path` key must appear.
        let serde_t = replace_out
            .get("serde:1.0.0")
            .and_then(|v| v.as_table())
            .unwrap();
        assert!(
            !serde_t.contains_key("path"),
            "git-form [replace] entry must not gain a `path` key"
        );
        assert_eq!(
            serde_t.get("git").and_then(|v| v.as_str()),
            Some("https://github.com/serde-rs/serde"),
            "git URL in git-form [replace] entry must be unchanged"
        );

        // already-absolute entry must not be modified.
        let abs_path = replace_out
            .get("abs-dep:0.1.0")
            .and_then(|v| v.as_table())
            .and_then(|t| t.get("path"))
            .and_then(|v| v.as_str())
            .unwrap();
        assert_eq!(
            abs_path, "/pre-existing/absolute/path",
            "an already-absolute [replace] path must be left unchanged; got `{abs_path}`"
        );
    }

    // ── R2 (PR #37 fixup) unit tests for `override_workspace_inheritance` ────

    /// Test helper: a non-existent upstream path used only to populate
    /// the error-diagnostic string. The override function never reads
    /// the file — only the path's `display()` is used when constructing
    /// the workspace-member rejection diagnostic.
    fn dummy_upstream_manifest_path() -> std::path::PathBuf {
        std::path::PathBuf::from("/tmp/lihaaf-test-upstream/Cargo.toml")
    }

    /// **R2 invariant: only membership keys are stripped from `[workspace]`.**
    ///
    /// R1 replaced the entire `[workspace]` table with `{}`. R2
    /// preserves every key EXCEPT `members`, `exclude`,
    /// `default-members`. This test exercises the full preserve-list:
    /// `dependencies`, `package`, `lints`, `metadata`, `resolver`.
    #[test]
    fn override_workspace_preserves_inheritance_tables() {
        let mut top = toml::map::Map::new();
        // Synthesize a fully-populated `[workspace]` table.
        let mut ws = toml::map::Map::new();
        ws.insert(
            "members".to_string(),
            toml::Value::Array(vec![toml::Value::String("crate-a".into())]),
        );
        ws.insert(
            "exclude".to_string(),
            toml::Value::Array(vec![toml::Value::String("scratch".into())]),
        );
        ws.insert(
            "default-members".to_string(),
            toml::Value::Array(vec![toml::Value::String("crate-a".into())]),
        );
        ws.insert("resolver".to_string(), toml::Value::String("2".into()));

        // `[workspace.dependencies]` — the key R1 stranded for
        // `{ workspace = true }` references.
        let mut ws_deps = toml::map::Map::new();
        let mut shared = toml::map::Map::new();
        shared.insert("path".to_string(), toml::Value::String("/abs/utils".into()));
        ws_deps.insert("shared-utils".to_string(), toml::Value::Table(shared));
        ws.insert("dependencies".to_string(), toml::Value::Table(ws_deps));

        // `[workspace.package]` — inherited `[package]` fields.
        let mut ws_pkg = toml::map::Map::new();
        ws_pkg.insert("edition".to_string(), toml::Value::String("2021".into()));
        ws_pkg.insert("version".to_string(), toml::Value::String("0.1.0".into()));
        ws.insert("package".to_string(), toml::Value::Table(ws_pkg));

        // `[workspace.lints]` — inherited `[lints]` rulesets.
        let mut ws_lints = toml::map::Map::new();
        let mut ws_lints_rust = toml::map::Map::new();
        ws_lints_rust.insert(
            "unsafe_code".to_string(),
            toml::Value::String("forbid".into()),
        );
        ws_lints.insert("rust".to_string(), toml::Value::Table(ws_lints_rust));
        ws.insert("lints".to_string(), toml::Value::Table(ws_lints));

        // `[workspace.metadata]` — tool-owned namespaced metadata.
        let mut ws_meta = toml::map::Map::new();
        let mut ws_meta_tool = toml::map::Map::new();
        ws_meta_tool.insert("key".to_string(), toml::Value::String("value".into()));
        ws_meta.insert("my-tool".to_string(), toml::Value::Table(ws_meta_tool));
        ws.insert("metadata".to_string(), toml::Value::Table(ws_meta));

        // Unknown future `[workspace.X]` table — must pass through
        // verbatim so the override stays forward-compatible.
        let mut ws_future = toml::map::Map::new();
        ws_future.insert(
            "key".to_string(),
            toml::Value::String("future-value".into()),
        );
        ws.insert(
            "future-cargo-feature".to_string(),
            toml::Value::Table(ws_future),
        );

        top.insert("workspace".to_string(), toml::Value::Table(ws));

        let mut pkg = toml::map::Map::new();
        pkg.insert("name".to_string(), toml::Value::String("test".into()));
        top.insert("package".to_string(), toml::Value::Table(pkg));

        override_workspace_inheritance(&mut top, &dummy_upstream_manifest_path())
            .expect("workspace-root case must succeed");

        let ws_out = top.get("workspace").and_then(|v| v.as_table()).unwrap();

        // Membership keys stripped.
        for stripped in ["members", "exclude", "default-members"] {
            assert!(
                !ws_out.contains_key(stripped),
                "membership key `{stripped}` MUST be stripped; got keys: {:?}",
                ws_out.keys().collect::<Vec<_>>()
            );
        }

        // Inheritance tables preserved.
        assert!(
            ws_out.contains_key("dependencies"),
            "workspace.dependencies must survive"
        );
        assert!(
            ws_out.contains_key("package"),
            "workspace.package must survive"
        );
        assert!(ws_out.contains_key("lints"), "workspace.lints must survive");
        assert!(
            ws_out.contains_key("metadata"),
            "workspace.metadata must survive"
        );
        assert!(
            ws_out.contains_key("resolver"),
            "workspace.resolver must survive"
        );
        assert!(
            ws_out.contains_key("future-cargo-feature"),
            "unknown `[workspace.X]` table must pass through (forward-compat)"
        );

        // Deep-equality on a couple of preserved entries to confirm
        // the rewrite is structure-preserving (not a stub).
        let ws_deps_out = ws_out
            .get("dependencies")
            .and_then(|v| v.as_table())
            .unwrap();
        let shared_out = ws_deps_out
            .get("shared-utils")
            .and_then(|v| v.as_table())
            .unwrap();
        assert_eq!(
            shared_out.get("path").and_then(|v| v.as_str()),
            Some("/abs/utils"),
            "workspace.dependencies.shared-utils.path must pass through verbatim"
        );

        let ws_pkg_out = ws_out.get("package").and_then(|v| v.as_table()).unwrap();
        assert_eq!(
            ws_pkg_out.get("edition").and_then(|v| v.as_str()),
            Some("2021"),
            "workspace.package.edition must pass through verbatim"
        );
    }

    /// **R2 invariant: missing `[workspace]` injects an empty one.**
    ///
    /// For single-crate forks the upstream `Cargo.toml` may have no
    /// `[workspace]` declaration of its own. The overlay still needs
    /// `[workspace]` to terminate cargo's walk-up.
    #[test]
    fn override_workspace_injects_empty_when_absent() {
        let mut top = toml::map::Map::new();
        let mut pkg = toml::map::Map::new();
        pkg.insert("name".to_string(), toml::Value::String("test".into()));
        top.insert("package".to_string(), toml::Value::Table(pkg));

        // No `[workspace]` in input.
        assert!(!top.contains_key("workspace"));

        override_workspace_inheritance(&mut top, &dummy_upstream_manifest_path())
            .expect("missing `[workspace]` must inject an empty one");

        let ws_out = top.get("workspace").and_then(|v| v.as_table()).unwrap();
        assert!(
            ws_out.is_empty(),
            "injected `[workspace]` must be empty when upstream had none; got: {:?}",
            ws_out.keys().collect::<Vec<_>>()
        );
    }

    /// **R2 invariant: EXPLICIT workspace-member case is REJECTED.**
    ///
    /// `[package].workspace = "<path>"` declares the manifest as a
    /// member of an ANCESTOR workspace. Copying the ancestor's
    /// inheritance tables into the overlay is out-of-scope for
    /// v0.1.0-beta.6, so the manifest is rejected with a directed
    /// diagnostic instead of being silently overlayed (with stripped
    /// inheritance) or silently emptied (R1's behavior, which stranded
    /// `{ workspace = true }` references).
    ///
    /// **R3 tightening (PR #37, strict-swe Finding 1):** the
    /// rejection MUST surface as `Error::Cli { clap_exit_code: 2,
    /// message }`, not as a different `Error` variant that happens
    /// to have a Debug repr containing "workspace member". A loose
    /// `format!("{err:?}").contains(...)` test would pass even if a
    /// future refactor changed the error variant to (say) `TomlParse`
    /// — which would silently regress the clap-conforming exit-code
    /// contract this rejection is supposed to enforce.
    #[test]
    fn override_workspace_rejects_workspace_member_manifest() {
        let mut top = toml::map::Map::new();
        let mut pkg = toml::map::Map::new();
        pkg.insert("name".to_string(), toml::Value::String("member".into()));
        pkg.insert("workspace".to_string(), toml::Value::String("../".into()));
        top.insert("package".to_string(), toml::Value::Table(pkg));

        let err = override_workspace_inheritance(&mut top, &dummy_upstream_manifest_path())
            .expect_err("workspace-member manifest must be rejected");

        match err {
            Error::Cli {
                clap_exit_code,
                message,
            } => {
                assert_eq!(
                    clap_exit_code, 2,
                    "exit code must be the clap usage code (2)"
                );
                assert!(
                    message.contains("workspace member"),
                    "rejection diagnostic must name the failure category; got: {message}"
                );
                assert!(
                    message.contains("[package].workspace"),
                    "rejection diagnostic must name the offending key; got: {message}"
                );
                assert!(
                    message.contains("/tmp/lihaaf-test-upstream/Cargo.toml"),
                    "rejection diagnostic must include the offending manifest path; got: {message}"
                );
                // Distinguish from the implicit-member rejection: the
                // explicit case must NOT use the word "implicit".
                assert!(
                    !message.contains("implicit"),
                    "explicit rejection must not use the implicit-case wording; got: {message}"
                );
            }
            other => panic!("expected Error::Cli for workspace-member rejection, got {other:?}"),
        }
    }

    /// **R3 invariant: IMPLICIT workspace-member case is REJECTED.**
    ///
    /// When the upstream manifest has NO local `[workspace]` table
    /// but DOES carry any `{ workspace = true }` inheritance
    /// reference, it is an implicit workspace member — cargo
    /// discovers the ancestor workspace by walking up the filesystem
    /// to find a `Cargo.toml` containing `[workspace]` whose
    /// `members = [...]` array names the current crate. Without
    /// this rejection, the overlay would inject `[workspace] = {}`
    /// and strand the inheritance reference at cargo parse time
    /// ("workspace inheritance was specified but `[workspace.X]` was
    /// not defined"). R3 (PR #37 Codex + Gemini BLOCK fixup) extends
    /// the rejection to this case so the user gets a clean directed
    /// diagnostic instead of a cryptic cargo error.
    ///
    /// This is the SMALLEST reproducible shape — a single
    /// `[dependencies] foo = { workspace = true }` reference is
    /// enough to trigger the rejection. The broader detection
    /// surface (all four `[package]` / dep / target / lints
    /// families) is exercised by
    /// `manifest_has_inheritance_reference_*` below.
    ///
    /// **Test environment caveat**: like the R4 standalone-allows
    /// test, this assertion depends on no `Cargo.toml` existing
    /// along the filesystem walk-up from
    /// `/tmp/lihaaf-test-upstream/Cargo.toml` (i.e., no
    /// `/tmp/Cargo.toml` or `/Cargo.toml` declaring `[workspace]`
    /// on the runner). If such a file exists, R4's ancestor-walk
    /// branch (`detect_implicit_ancestor_workspace`) fires before
    /// this rejection branch and produces a diagnostic naming the
    /// ancestor path instead of the "implicit workspace member"
    /// category — the inner `message.contains("no local
    /// \`[workspace]\`")` assertion would then fail. The
    /// constraint holds on standard CI runners and developer
    /// machines.
    #[test]
    fn override_workspace_rejects_implicit_workspace_member_manifest() {
        let mut top = toml::map::Map::new();
        let mut pkg = toml::map::Map::new();
        pkg.insert("name".to_string(), toml::Value::String("member".into()));
        top.insert("package".to_string(), toml::Value::Table(pkg));
        // No local `[workspace]`. A single inheritance reference
        // through `[dependencies]` is the shortest path to the
        // implicit-member shape.
        let mut deps = toml::map::Map::new();
        let mut foo = toml::map::Map::new();
        foo.insert("workspace".to_string(), toml::Value::Boolean(true));
        deps.insert("foo".to_string(), toml::Value::Table(foo));
        top.insert("dependencies".to_string(), toml::Value::Table(deps));

        let err = override_workspace_inheritance(&mut top, &dummy_upstream_manifest_path())
            .expect_err("implicit workspace-member manifest must be rejected");

        match err {
            Error::Cli {
                clap_exit_code,
                message,
            } => {
                assert_eq!(
                    clap_exit_code, 2,
                    "exit code must match the explicit-rejection contract (clap usage code 2)"
                );
                assert!(
                    message.contains("implicit workspace member"),
                    "rejection diagnostic must name the implicit-member category; got: {message}"
                );
                assert!(
                    message.contains("no local `[workspace]`"),
                    "diagnostic must name the diagnostic structural signal; got: {message}"
                );
                assert!(
                    message.contains("workspace = true"),
                    "diagnostic must point at the inheritance-reference shape; got: {message}"
                );
                assert!(
                    message.contains("/tmp/lihaaf-test-upstream/Cargo.toml"),
                    "diagnostic must include the offending manifest path; got: {message}"
                );
                // The original `top` must NOT have been mutated: the
                // override is supposed to abort BEFORE writing
                // `[workspace]`. Idempotency guarantee under failure.
                assert!(
                    !top.contains_key("workspace"),
                    "rejection must not leave a half-mutated `[workspace]` entry in place"
                );
            }
            other => {
                panic!("expected Error::Cli for implicit workspace-member rejection, got {other:?}")
            }
        }
    }

    /// **R4 invariant: IMPLICIT workspace-member case via ancestor
    /// `Cargo.toml` is REJECTED.**
    ///
    /// The Codex R3 review flagged a correctness gap: a manifest with
    /// NO local `[workspace]` AND NO `{ workspace = true }`
    /// inheritance references could still be contained within an
    /// ancestor workspace that carries `[patch.crates-io]`,
    /// `[replace]`, `[profile]`, `resolver`, or
    /// `[workspace.dependencies]` tables. Baseline cargo walks up the
    /// filesystem from the descendant and applies the ancestor state;
    /// the lihaaf overlay declares its own `[workspace]` and
    /// terminates the walk-up at the staged manifest, skipping the
    /// ancestor state entirely. Result: divergent dependency graphs
    /// and false compat verdicts. R4 (PR #37 R3 BLOCK fixup) walks
    /// up the filesystem from the manifest's parent and rejects on
    /// any ancestor `Cargo.toml` carrying `[workspace]`.
    ///
    /// **What this test pins:** when the upstream Cargo.toml has
    /// neither a local `[workspace]` table nor any inheritance
    /// references, but lives inside a directory whose parent
    /// `Cargo.toml` carries `[workspace] members = ["<dir>"]`,
    /// `override_workspace_inheritance` rejects with `Error::Cli {
    /// clap_exit_code: 2, ... }` whose message names the implicit-
    /// member category AND the ancestor manifest path.
    ///
    /// **Defense-in-depth:** this is the case the R3 implicit-
    /// inheritance-refs rejection does NOT catch (no `{ workspace =
    /// true }` is required to trigger the failure mode), so without
    /// R4 the overlay silently produced a manifest with a divergent
    /// resolved graph relative to baseline — the worst failure mode
    /// (false compat verdict, no error surfaced).
    #[test]
    fn override_workspace_rejects_manifest_with_ancestor_workspace() {
        let tmp = tempfile::tempdir().expect("tempdir for ancestor-workspace rejection test");

        // Parent dir: workspace ROOT carrying `[workspace]` +
        // `[patch.crates-io]`. The Codex repro shape exactly.
        let parent_manifest = tmp.path().join("Cargo.toml");
        std::fs::write(
            &parent_manifest,
            r#"[workspace]
members = ["sub"]

[patch.crates-io]
foo = { path = "../my-foo-fork" }
"#,
        )
        .expect("writing parent Cargo.toml");

        // Sub-crate: no local `[workspace]`, no inheritance refs.
        // This is the implicit-member-via-ancestor shape.
        let sub_dir = tmp.path().join("sub");
        std::fs::create_dir_all(&sub_dir).expect("creating sub/ dir");
        let sub_manifest = sub_dir.join("Cargo.toml");
        std::fs::write(
            &sub_manifest,
            r#"[package]
name = "sub"
version = "0.1.0"
"#,
        )
        .expect("writing sub/Cargo.toml");

        // Now exercise `override_workspace_inheritance` on a parsed
        // top representing the sub manifest. We build the top
        // directly (rather than going through `materialize_overlay`)
        // because this is the structural unit test; the integration
        // test below exercises the full pipeline.
        let mut top = toml::map::Map::new();
        let mut pkg = toml::map::Map::new();
        pkg.insert("name".to_string(), toml::Value::String("sub".into()));
        top.insert("package".to_string(), toml::Value::Table(pkg));

        let err = override_workspace_inheritance(&mut top, &sub_manifest)
            .expect_err("manifest with ancestor workspace must be rejected");

        match err {
            Error::Cli {
                clap_exit_code,
                message,
            } => {
                assert_eq!(
                    clap_exit_code, 2,
                    "exit code must match the rejection contract (clap usage code 2)"
                );
                assert!(
                    message.contains("implicit workspace member"),
                    "diagnostic must name the implicit-member category; got: {message}"
                );
                assert!(
                    message.contains("ancestor manifest"),
                    "diagnostic must name the ancestor-detection signal; got: {message}"
                );
                let parent_str = parent_manifest.display().to_string();
                assert!(
                    message.contains(&parent_str),
                    "diagnostic must include the ancestor manifest path `{parent_str}`; got: {message}"
                );
                // The diagnostic must NOT use the inheritance-refs
                // wording: this case has no `{ workspace = true }`
                // references, and conflating the two would mislead
                // users about which signal triggered the rejection.
                assert!(
                    !message.contains("workspace = true"),
                    "ancestor-workspace rejection must not mention inheritance refs (this case has none); got: {message}"
                );
                // No half-mutated workspace key on failure.
                assert!(
                    !top.contains_key("workspace"),
                    "rejection must not leave a half-mutated `[workspace]` entry in place"
                );
            }
            other => {
                panic!("expected Error::Cli for ancestor-workspace rejection, got {other:?}")
            }
        }
    }

    /// **R4 invariant: STANDALONE single-crate manifest (no ancestor
    /// workspace) is ALLOWED — branch 5 still works.**
    ///
    /// The R4 ancestor-walk must NOT produce false-positive rejections
    /// for the standard standalone single-crate case: a fork whose
    /// `Cargo.toml` has no local `[workspace]`, no inheritance refs,
    /// AND lives in a directory tree whose ancestors have no
    /// `Cargo.toml` at all. This is the most common compat-mode shape
    /// for adopters who haven't enrolled in a workspace pattern
    /// (single-crate libraries like `anyhow`, `thiserror`).
    ///
    /// **What this test pins:** a tempdir with ONLY a single
    /// `Cargo.toml` in its root (no ancestor Cargo.toml on the walk-
    /// up; tempdirs live under `/tmp/...` on Linux and `~/Library/
    /// Caches/.../` on macOS — neither path typically has a `Cargo.
    /// toml` along the way to the filesystem root) produces a
    /// successful `override_workspace_inheritance` call with an
    /// injected empty `[workspace]`.
    ///
    /// **Defense-in-depth:** without R4 this test would still pass
    /// (the standalone case has always worked); with R4 it confirms
    /// that the ancestor-walk does not regress the standalone case.
    /// The test asserts the SPECIFIC absence of the ancestor-walk
    /// rejection AND the presence of the injected empty `[workspace]`
    /// — a regression where the ancestor-walk spuriously triggered
    /// rejection on a path with no real ancestor workspace would
    /// fail this test by producing `Err(Error::Cli)` instead of
    /// `Ok(())`.
    ///
    /// **Test-environment caveat:** this test relies on no
    /// `Cargo.toml` existing at any ancestor of the OS temp dir
    /// (typically `/tmp/Cargo.toml`, `/Cargo.toml`, etc.). On any
    /// reasonable CI runner or developer machine this holds; on a
    /// weirdly-configured box that happens to have such a file, this
    /// test would surface the issue cleanly (the rejection diagnostic
    /// would name the offending path).
    #[test]
    fn override_workspace_allows_standalone_with_no_ancestor_workspace() {
        let tmp = tempfile::tempdir().expect("tempdir for standalone-allows negative-case test");

        // Single standalone Cargo.toml at the tempdir root. NO
        // local `[workspace]`, NO inheritance refs, NO sibling or
        // ancestor Cargo.toml.
        let manifest = tmp.path().join("Cargo.toml");
        std::fs::write(
            &manifest,
            r#"[package]
name = "standalone"
version = "0.1.0"
"#,
        )
        .expect("writing standalone Cargo.toml");

        let mut top = toml::map::Map::new();
        let mut pkg = toml::map::Map::new();
        pkg.insert("name".to_string(), toml::Value::String("standalone".into()));
        top.insert("package".to_string(), toml::Value::Table(pkg));

        // No `[workspace]` in input.
        assert!(!top.contains_key("workspace"));

        // The override MUST succeed — no ancestor workspace on the
        // walk-up, no inheritance refs, no local workspace. Branch 5
        // (standalone injection) fires.
        override_workspace_inheritance(&mut top, &manifest).unwrap_or_else(|err| {
            panic!(
                "standalone manifest with no ancestor workspace must NOT be rejected; \
                 got: {err:?} (this would indicate an R4 regression — the ancestor walk \
                 spuriously detected a workspace where there is none, OR the test \
                 environment has an unexpected `Cargo.toml` somewhere above the temp dir)"
            )
        });

        // Branch 5 outcome: empty `[workspace]` table injected.
        let ws_out = top.get("workspace").and_then(|v| v.as_table()).unwrap();
        assert!(
            ws_out.is_empty(),
            "branch 5 (standalone) must inject an empty `[workspace]`; got keys: {:?}",
            ws_out.keys().collect::<Vec<_>>()
        );
    }

    /// **R4 helper: `detect_implicit_ancestor_workspace` returns
    /// `None` when no ancestor Cargo.toml exists on the walk-up.**
    ///
    /// Direct unit test on the helper function — the negative case
    /// for the simplest possible filesystem layout. The integration
    /// behavior is verified by
    /// `override_workspace_allows_standalone_with_no_ancestor_workspace`
    /// above; this is the unit-level confirmation that the helper
    /// itself does the right thing.
    #[test]
    fn detect_implicit_ancestor_workspace_returns_none_for_standalone() {
        let tmp = tempfile::tempdir().expect("tempdir for ancestor-walk None negative case");
        let manifest = tmp.path().join("Cargo.toml");
        std::fs::write(&manifest, "[package]\nname = \"standalone\"\n")
            .expect("writing standalone Cargo.toml");

        let result = detect_implicit_ancestor_workspace(&manifest)
            .expect("ancestor walk on a clean tempdir must not return Err");
        assert!(
            result.is_none(),
            "ancestor walk from a standalone tempdir manifest must return None; got: {result:?}"
        );
    }

    /// **R4 helper: `detect_implicit_ancestor_workspace` returns
    /// `Some(path)` when an ancestor Cargo.toml carries
    /// `[workspace]`.**
    ///
    /// Direct unit test on the helper — confirms the walk finds the
    /// nearest ancestor with `[workspace]` AND returns that
    /// manifest's path (not the descendant's, not the grandparent's).
    #[test]
    fn detect_implicit_ancestor_workspace_finds_nearest_ancestor() {
        let tmp = tempfile::tempdir().expect("tempdir for ancestor-walk Some positive case");

        // Parent: workspace root.
        let parent_manifest = tmp.path().join("Cargo.toml");
        std::fs::write(&parent_manifest, "[workspace]\nmembers = [\"sub\"]\n")
            .expect("writing parent Cargo.toml");

        // Sub: implicit member.
        let sub_dir = tmp.path().join("sub");
        std::fs::create_dir_all(&sub_dir).expect("creating sub/");
        let sub_manifest = sub_dir.join("Cargo.toml");
        std::fs::write(
            &sub_manifest,
            "[package]\nname = \"sub\"\nversion = \"0.1.0\"\n",
        )
        .expect("writing sub/Cargo.toml");

        let result =
            detect_implicit_ancestor_workspace(&sub_manifest).expect("ancestor walk must succeed");
        let found = result.expect("ancestor walk must find the parent workspace");
        assert_eq!(
            found, parent_manifest,
            "ancestor walk must return the parent manifest path verbatim"
        );
    }

    /// Verify `manifest_has_inheritance_reference` returns `false`
    /// for the negative cases: empty manifest, manifest with only
    /// `[package].name` (no inheritance), manifest with regular
    /// deps that lack `workspace = true`, and the EXPLICIT-member
    /// case where `[package].workspace = "<path>"` is a String
    /// (not an inheritance reference — handled by the explicit
    /// rejection upstream).
    #[test]
    fn manifest_has_inheritance_reference_returns_false_for_non_inheriting_shapes() {
        // Empty manifest.
        let top = toml::map::Map::new();
        assert!(
            !manifest_has_inheritance_reference(&top),
            "empty manifest has no inheritance references"
        );

        // `[package].name` only.
        let mut top = toml::map::Map::new();
        let mut pkg = toml::map::Map::new();
        pkg.insert("name".to_string(), toml::Value::String("demo".into()));
        top.insert("package".to_string(), toml::Value::Table(pkg));
        assert!(
            !manifest_has_inheritance_reference(&top),
            "manifest with `[package].name` only has no inheritance references"
        );

        // Regular dep without `workspace = true`.
        let mut deps = toml::map::Map::new();
        let mut foo = toml::map::Map::new();
        foo.insert("version".to_string(), toml::Value::String("1.0".into()));
        deps.insert("foo".to_string(), toml::Value::Table(foo));
        top.insert("dependencies".to_string(), toml::Value::Table(deps));
        assert!(
            !manifest_has_inheritance_reference(&top),
            "regular dep without `workspace = true` does not count as inheritance"
        );

        // `[package].workspace = "../"` is the EXPLICIT-member
        // String pointer, NOT an inheritance reference. The helper
        // must distinguish these two cases.
        let mut pkg2 = toml::map::Map::new();
        pkg2.insert("name".to_string(), toml::Value::String("member".into()));
        pkg2.insert("workspace".to_string(), toml::Value::String("../".into()));
        let mut top2 = toml::map::Map::new();
        top2.insert("package".to_string(), toml::Value::Table(pkg2));
        assert!(
            !manifest_has_inheritance_reference(&top2),
            "`[package].workspace = \"...\"` is the explicit-member pointer, not inheritance"
        );
    }

    /// Verify `manifest_has_inheritance_reference` returns `true`
    /// for inheritance references in every supported family:
    /// `[package].<key>`, `[dependencies]`, `[dev-dependencies]`,
    /// `[build-dependencies]`, `[target.<cfg>.<deps>]`, and `[lints]`
    /// (both top-level and nested forms).
    #[test]
    fn manifest_has_inheritance_reference_detects_every_family() {
        // 1. `[package].version = { workspace = true }`.
        let mut top = toml::map::Map::new();
        let mut pkg = toml::map::Map::new();
        let mut version = toml::map::Map::new();
        version.insert("workspace".to_string(), toml::Value::Boolean(true));
        pkg.insert("version".to_string(), toml::Value::Table(version));
        top.insert("package".to_string(), toml::Value::Table(pkg));
        assert!(
            manifest_has_inheritance_reference(&top),
            "`[package].version = {{ workspace = true }}` must be detected"
        );

        // 2. `[dependencies] foo = { workspace = true }`.
        let mut top = toml::map::Map::new();
        let mut deps = toml::map::Map::new();
        let mut foo = toml::map::Map::new();
        foo.insert("workspace".to_string(), toml::Value::Boolean(true));
        deps.insert("foo".to_string(), toml::Value::Table(foo));
        top.insert("dependencies".to_string(), toml::Value::Table(deps));
        assert!(
            manifest_has_inheritance_reference(&top),
            "`[dependencies] foo = {{ workspace = true }}` must be detected"
        );

        // 3. `[dev-dependencies] foo = { workspace = true }`.
        let mut top = toml::map::Map::new();
        let mut deps = toml::map::Map::new();
        let mut foo = toml::map::Map::new();
        foo.insert("workspace".to_string(), toml::Value::Boolean(true));
        deps.insert("foo".to_string(), toml::Value::Table(foo));
        top.insert("dev-dependencies".to_string(), toml::Value::Table(deps));
        assert!(
            manifest_has_inheritance_reference(&top),
            "`[dev-dependencies] foo = {{ workspace = true }}` must be detected"
        );

        // 4. `[build-dependencies] foo = { workspace = true }`.
        let mut top = toml::map::Map::new();
        let mut deps = toml::map::Map::new();
        let mut foo = toml::map::Map::new();
        foo.insert("workspace".to_string(), toml::Value::Boolean(true));
        deps.insert("foo".to_string(), toml::Value::Table(foo));
        top.insert("build-dependencies".to_string(), toml::Value::Table(deps));
        assert!(
            manifest_has_inheritance_reference(&top),
            "`[build-dependencies] foo = {{ workspace = true }}` must be detected"
        );

        // 5. `[target.'cfg(unix)'.dependencies] foo = { workspace = true }`.
        let mut top = toml::map::Map::new();
        let mut targets = toml::map::Map::new();
        let mut cfg = toml::map::Map::new();
        let mut deps = toml::map::Map::new();
        let mut foo = toml::map::Map::new();
        foo.insert("workspace".to_string(), toml::Value::Boolean(true));
        deps.insert("foo".to_string(), toml::Value::Table(foo));
        cfg.insert("dependencies".to_string(), toml::Value::Table(deps));
        targets.insert("cfg(unix)".to_string(), toml::Value::Table(cfg));
        top.insert("target".to_string(), toml::Value::Table(targets));
        assert!(
            manifest_has_inheritance_reference(&top),
            "`[target.<cfg>.dependencies]` inheritance must be detected"
        );

        // 6. `[target.'cfg(windows)'.dev-dependencies]`.
        let mut top = toml::map::Map::new();
        let mut targets = toml::map::Map::new();
        let mut cfg = toml::map::Map::new();
        let mut deps = toml::map::Map::new();
        let mut foo = toml::map::Map::new();
        foo.insert("workspace".to_string(), toml::Value::Boolean(true));
        deps.insert("foo".to_string(), toml::Value::Table(foo));
        cfg.insert("dev-dependencies".to_string(), toml::Value::Table(deps));
        targets.insert("cfg(windows)".to_string(), toml::Value::Table(cfg));
        top.insert("target".to_string(), toml::Value::Table(targets));
        assert!(
            manifest_has_inheritance_reference(&top),
            "`[target.<cfg>.dev-dependencies]` inheritance must be detected"
        );

        // 7. `[target.'cfg(target_arch = "wasm32")'.build-dependencies]`.
        let mut top = toml::map::Map::new();
        let mut targets = toml::map::Map::new();
        let mut cfg = toml::map::Map::new();
        let mut deps = toml::map::Map::new();
        let mut foo = toml::map::Map::new();
        foo.insert("workspace".to_string(), toml::Value::Boolean(true));
        deps.insert("foo".to_string(), toml::Value::Table(foo));
        cfg.insert("build-dependencies".to_string(), toml::Value::Table(deps));
        targets.insert(
            "cfg(target_arch = \"wasm32\")".to_string(),
            toml::Value::Table(cfg),
        );
        top.insert("target".to_string(), toml::Value::Table(targets));
        assert!(
            manifest_has_inheritance_reference(&top),
            "`[target.<cfg>.build-dependencies]` inheritance must be detected"
        );

        // 8. `[lints] workspace = true` (top-level form, the only
        //    form cargo currently supports for lints inheritance).
        let mut top = toml::map::Map::new();
        let mut lints = toml::map::Map::new();
        lints.insert("workspace".to_string(), toml::Value::Boolean(true));
        top.insert("lints".to_string(), toml::Value::Table(lints));
        assert!(
            manifest_has_inheritance_reference(&top),
            "`[lints] workspace = true` (top-level form) must be detected"
        );

        // 9. `[lints.rust] workspace = true` — forward-compat
        //    nested form. Cargo doesn't currently support this, but
        //    the detector flags it defensively to stay
        //    forward-compatible.
        let mut top = toml::map::Map::new();
        let mut lints = toml::map::Map::new();
        let mut rust = toml::map::Map::new();
        rust.insert("workspace".to_string(), toml::Value::Boolean(true));
        lints.insert("rust".to_string(), toml::Value::Table(rust));
        top.insert("lints".to_string(), toml::Value::Table(lints));
        assert!(
            manifest_has_inheritance_reference(&top),
            "`[lints.rust] workspace = true` (forward-compat nested form) must be detected"
        );

        // 10. Unknown future `[package].<future-key> = { workspace
        //     = true }`. Forward-compat: the detector scans all
        //     `[package]` sub-keys, not just the cargo-documented
        //     inheritable ones, so a future cargo addition gets the
        //     correct rejection on day one.
        let mut top = toml::map::Map::new();
        let mut pkg = toml::map::Map::new();
        let mut future = toml::map::Map::new();
        future.insert("workspace".to_string(), toml::Value::Boolean(true));
        pkg.insert(
            "future-inheritable-key".to_string(),
            toml::Value::Table(future),
        );
        top.insert("package".to_string(), toml::Value::Table(pkg));
        assert!(
            manifest_has_inheritance_reference(&top),
            "unknown `[package].<future-key>` inheritance must be detected (forward-compat)"
        );
    }

    /// **R3 invariant: implicit-member detection coexists with the
    /// workspace-root case.**
    ///
    /// A manifest with BOTH a local `[workspace]` table AND
    /// `{ workspace = true }` references is the standard
    /// workspace-root shape (root cargo manifest that hosts both
    /// `[workspace.dependencies]` and its OWN `[package]` with
    /// inheritance refs back to itself). The R3 implicit-member
    /// check must NOT fire here — the inheritance references resolve
    /// against the LOCAL `[workspace.*]` tables, which the overlay
    /// preserves.
    #[test]
    fn override_workspace_allows_root_with_local_workspace_and_inheritance_refs() {
        // Shape: `[package] version = { workspace = true }` +
        // `[workspace.package] version = "0.1.0"` — the root crate
        // inherits from its own `[workspace.package]`. This is
        // legitimate cargo and the overlay must preserve it.
        let mut top = toml::map::Map::new();

        let mut pkg = toml::map::Map::new();
        pkg.insert("name".to_string(), toml::Value::String("root".into()));
        let mut version = toml::map::Map::new();
        version.insert("workspace".to_string(), toml::Value::Boolean(true));
        pkg.insert("version".to_string(), toml::Value::Table(version));
        top.insert("package".to_string(), toml::Value::Table(pkg));

        let mut ws = toml::map::Map::new();
        let mut ws_pkg = toml::map::Map::new();
        ws_pkg.insert("version".to_string(), toml::Value::String("0.1.0".into()));
        ws.insert("package".to_string(), toml::Value::Table(ws_pkg));
        top.insert("workspace".to_string(), toml::Value::Table(ws));

        override_workspace_inheritance(&mut top, &dummy_upstream_manifest_path())
            .expect("root with local [workspace] + inheritance refs must succeed (not implicit)");

        // The inheritance reference must survive.
        let pkg_out = top.get("package").and_then(|v| v.as_table()).unwrap();
        let version_out = pkg_out.get("version").and_then(|v| v.as_table()).unwrap();
        assert_eq!(
            version_out.get("workspace").and_then(|v| v.as_bool()),
            Some(true),
            "inheritance reference must pass through verbatim for workspace-root case"
        );

        // The `[workspace.package]` table must survive.
        let ws_out = top.get("workspace").and_then(|v| v.as_table()).unwrap();
        assert!(
            ws_out.contains_key("package"),
            "workspace.package must survive for the workspace-root case"
        );
    }

    /// **R2 invariant: idempotent on already-overridden output.**
    ///
    /// Running the override twice must produce the same result as
    /// running it once. R1's full-clobber was trivially idempotent;
    /// R2's selective rewrite requires verification because the
    /// preserved tables flow through unmodified on the second call.
    #[test]
    fn override_workspace_is_idempotent() {
        let mut top = toml::map::Map::new();
        let mut ws = toml::map::Map::new();
        ws.insert(
            "members".to_string(),
            toml::Value::Array(vec![toml::Value::String("crate-a".into())]),
        );
        let mut ws_deps = toml::map::Map::new();
        let mut shared = toml::map::Map::new();
        shared.insert("path".to_string(), toml::Value::String("/abs/utils".into()));
        ws_deps.insert("shared".to_string(), toml::Value::Table(shared));
        ws.insert("dependencies".to_string(), toml::Value::Table(ws_deps));
        top.insert("workspace".to_string(), toml::Value::Table(ws));
        let mut pkg = toml::map::Map::new();
        pkg.insert("name".to_string(), toml::Value::String("test".into()));
        top.insert("package".to_string(), toml::Value::Table(pkg));

        override_workspace_inheritance(&mut top, &dummy_upstream_manifest_path()).unwrap();
        let after_first = top.clone();
        override_workspace_inheritance(&mut top, &dummy_upstream_manifest_path()).unwrap();
        assert_eq!(
            top, after_first,
            "second call must be a no-op on already-overridden output"
        );
    }

    // ---- inject_synthetic_metadata tests ----

    /// Helper: extract the `[package.metadata.lihaaf]` sub-table from a
    /// post-`inject_synthetic_metadata` map.
    fn extract_lihaaf_table(
        top: &toml::map::Map<String, toml::Value>,
    ) -> &toml::map::Map<String, toml::Value> {
        top["package"].as_table().unwrap()["metadata"]
            .as_table()
            .unwrap()["lihaaf"]
            .as_table()
            .unwrap()
    }

    #[test]
    fn synthetic_metadata_injects_allow_lints() {
        // Given an empty TOML map, inject_synthetic_metadata must write the
        // allow_lints array into [package.metadata.lihaaf].
        let mut top = toml::map::Map::new();
        let meta = SyntheticMetadata {
            dylib_crate: "demo".into(),
            extern_crates: vec!["demo".into()],
            fixture_dirs: vec!["/abs/pass".into(), "/abs/fail".into()],
            allow_lints: vec!["unexpected_cfgs".to_string()],
        };
        inject_synthetic_metadata(&mut top, &meta);

        let lihaaf = extract_lihaaf_table(&top);
        let lints = lihaaf["allow_lints"]
            .as_array()
            .expect("allow_lints must be an array");
        let lint_strs: Vec<&str> = lints.iter().filter_map(|v| v.as_str()).collect();
        assert_eq!(
            lint_strs,
            vec!["unexpected_cfgs"],
            "inject_synthetic_metadata must write the allow_lints array verbatim"
        );
    }

    #[test]
    fn synthetic_metadata_default_in_compat_driver() {
        // Pin the compat-driver default via the SAME helper the driver calls
        // (`compat_default_synthetic_metadata`). The assertion is against an
        // independently written literal so that a future change to the helper
        // (e.g. `allow_lints: vec![]`) fails this test — the helper return
        // and the expected value are decoupled.
        //
        // If you change the helper's `allow_lints` default you MUST also
        // update spec §3.2/C.4, CHANGELOG, and this assertion.
        let meta = compat_default_synthetic_metadata("demo", vec![]);
        assert_eq!(
            meta.allow_lints,
            vec!["unexpected_cfgs".to_string()],
            "compat-driver default allow_lints must be [\"unexpected_cfgs\"]; \
             changes also require spec §3.2/C.4 + CHANGELOG updates",
        );
    }

    #[test]
    fn synthetic_metadata_replaces_upstream_allow_lints() {
        // The "compat owns inner config" invariant: inject_synthetic_metadata
        // must REPLACE any pre-existing [package.metadata.lihaaf] block —
        // including a pre-existing allow_lints key — with the synthetic values.
        // Verified at overlay.rs:1051-1058 in comment; this test makes it
        // executable so a future partial-merge regression is caught.
        let mut top = toml::map::Map::new();

        // Build the upstream lihaaf table with a conflicting allow_lints.
        let mut upstream_lihaaf = toml::map::Map::new();
        upstream_lihaaf.insert(
            "allow_lints".to_string(),
            toml::Value::Array(vec![toml::Value::String("some_other_lint".to_string())]),
        );
        // Nest: top["package"]["metadata"]["lihaaf"] = upstream_lihaaf
        let mut upstream_metadata = toml::map::Map::new();
        upstream_metadata.insert("lihaaf".to_string(), toml::Value::Table(upstream_lihaaf));
        let mut upstream_pkg = toml::map::Map::new();
        upstream_pkg.insert(
            "metadata".to_string(),
            toml::Value::Table(upstream_metadata),
        );
        top.insert("package".to_string(), toml::Value::Table(upstream_pkg));

        // Inject synthetic metadata with allow_lints = ["unexpected_cfgs"].
        let meta = SyntheticMetadata {
            dylib_crate: "demo".into(),
            extern_crates: vec!["demo".into()],
            fixture_dirs: vec!["/abs/pass".into()],
            allow_lints: vec!["unexpected_cfgs".to_string()],
        };
        inject_synthetic_metadata(&mut top, &meta);

        let lihaaf = extract_lihaaf_table(&top);
        let lints = lihaaf["allow_lints"]
            .as_array()
            .expect("allow_lints must be an array after injection");
        let lint_strs: Vec<&str> = lints.iter().filter_map(|v| v.as_str()).collect();
        assert_eq!(
            lint_strs,
            vec!["unexpected_cfgs"],
            "inject_synthetic_metadata must REPLACE upstream allow_lints, not merge or preserve it"
        );
    }
}
