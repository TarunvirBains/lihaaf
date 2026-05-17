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
//! ## Workspace-inheritance override (`[workspace] = {}` injection)
//!
//! The staged overlay always ends with an empty `[workspace]` table,
//! regardless of whether the upstream manifest declared one. This is
//! the workspace-identity fix for the v0.1.0-beta.5 regression on
//! workspace-style pilots (see issue #36).
//!
//! **Why this is necessary.** Cargo determines a manifest's workspace
//! root by walking UP the filesystem from the manifest until it finds
//! another `Cargo.toml` with a `[workspace]` table. For the staged
//! overlay at `<upstream>/target/lihaaf-overlay/Cargo.toml`, that walk
//! reaches `<upstream>/Cargo.toml` — and for workspace-style pilots
//! (cxx, serde-json, thiserror) the upstream IS a workspace root. Cargo
//! then tries to attach the overlay's package to the upstream
//! workspace, but the overlay's package name isn't in the upstream's
//! `members` array. Result: `package <X>/Cargo.toml is a member of the
//! wrong workspace` and the build fails.
//!
//! Adding `[workspace]` (even an empty table) to the overlay makes
//! cargo treat the overlay AS ITS OWN workspace root and stop walking
//! up. The overlay is then a standalone, self-contained workspace
//! whose path-deps reference packages in OTHER workspaces (the
//! upstream's) — which is valid in cargo.
//!
//! **Why the table is empty (no members inherited).** If the overlay's
//! `[workspace]` carried the upstream's `members = [...]` (absolutized
//! to abs paths), the overlay AND the upstream would both claim those
//! path-dep crates as members → `package <X> is a member of the wrong
//! workspace`. An empty `[workspace]` declares no members, leaving
//! ownership exclusively with the upstream workspace where it was
//! originally declared.
//!
//! **Why `[package].workspace` is removed.** A package cannot
//! simultaneously declare itself as a workspace root (`[workspace]`)
//! AND as a member of an ancestor workspace (`[package] workspace =
//! "..."`). The overlay always elects the workspace-root role, so the
//! ancestor-pointer is stripped at the same time.

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

        if let Some(meta) = synthetic.as_ref() {
            inject_synthetic_metadata(top, meta);
        }

        // Override workspace inheritance: declare the overlay as its own
        // workspace root with no members, and strip any `[package].workspace`
        // ancestor pointer. Runs AFTER `absolutize_path_bearing_keys` so the
        // earlier absolutization of `[workspace] members`/`exclude`/
        // `default-members`/`dependencies.<X>.path` is harmlessly clobbered;
        // those values are not needed because cargo resolves the overlay's
        // path-deps from `[dependencies.<X>] path` directly, and the
        // upstream's actual workspace still owns those member crates from
        // their own perspective. See the module-level "Workspace-inheritance
        // override" section and issue #36 for the full rationale.
        override_workspace_inheritance(top);
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
    let crate_dir = upstream_manifest_path
        .parent()
        .unwrap_or(upstream_manifest_path);
    let sibling_path = crate_dir
        .join("target")
        .join("lihaaf-overlay")
        .join("Cargo.toml");

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

/// Override the overlay's workspace inheritance: replace any existing
/// `[workspace]` table with an empty one, and remove any
/// `[package].workspace` ancestor pointer.
///
/// **Why this is necessary.** When cargo resolves the staged overlay
/// at `<upstream>/target/lihaaf-overlay/Cargo.toml`, it walks UP the
/// filesystem to find the overlay's workspace root. For
/// workspace-style upstreams (cxx, serde-json, thiserror) it reaches
/// the upstream `Cargo.toml` first, which declares `[workspace]`. The
/// overlay's package isn't in the upstream's `members`, so cargo errors
/// with `package <X>/Cargo.toml is a member of the wrong workspace`.
/// See issue #36 for the v0.1.0-beta.5 GitHub Actions run that surfaced
/// this on every workspace-style pilot.
///
/// **Mechanism.** A manifest containing `[workspace]` is treated by
/// cargo as its own workspace root — cargo stops the walk-up at that
/// point. We write an EMPTY `[workspace]` table so the overlay declares
/// no members, leaving member ownership of any path-dep packages
/// exclusively with the upstream workspace they were originally
/// declared in. Cargo handles this cross-workspace path-dep pattern
/// correctly.
///
/// **`[package].workspace` removal.** This ancestor-pointer key
/// requests that the package be a member of the named workspace. It
/// contradicts the `[workspace]` self-declaration we just made, so we
/// strip it. The compat driver does not need the upstream's
/// workspace-membership relationship preserved in the overlay — the
/// overlay's sole purpose is to compile the dylib_crate as a `dylib`,
/// not to faithfully reproduce the upstream workspace topology.
///
/// **Why this runs LAST.** The earlier `absolutize_path_bearing_keys`
/// pass had already rewritten `[workspace] members`/`exclude`/
/// `default-members`/`dependencies.<X>.path` against the upstream dir.
/// Clobbering at the end discards that work, but it was not load-bearing
/// for the staged overlay's build — cargo resolves the overlay's deps
/// from `[dependencies.<X>] path` (already absolutized), not from
/// `[workspace.dependencies]`. The post-absolutize clobber is the
/// cleanest layering: the earlier pass keeps its existing tests green
/// at the unit level, and the higher-level override is the new
/// workspace-identity contract.
///
/// Idempotent: a second call on already-overridden output is a no-op.
fn override_workspace_inheritance(top: &mut toml::map::Map<String, toml::Value>) {
    // 1. Replace any existing `[workspace]` (or absent key) with an
    //    empty table. Cargo accepts an empty workspace table as the
    //    canonical "this manifest is its own workspace root" declaration
    //    (https://doc.rust-lang.org/cargo/reference/workspaces.html).
    top.insert(
        "workspace".to_string(),
        toml::Value::Table(toml::map::Map::new()),
    );

    // 2. Strip `[package].workspace` if present. The overlay always
    //    elects the workspace-root role (step 1); a simultaneous
    //    `[package] workspace = "..."` declaration would point at an
    //    ancestor workspace, contradicting the self-declaration and
    //    triggering a cargo error.
    if let Some(toml::Value::Table(pkg)) = top.get_mut("package") {
        pkg.remove("workspace");
    }
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
}
