//! `[package.metadata.lihaaf]` parsing + validation.
//!
//! This module is the single point where raw TOML becomes the typed [`Config`]
//! used by the rest of the harness. If you add a new TOP-LEVEL key (one that
//! lives directly in `[package.metadata.lihaaf]`, such as `dylib_crate`,
//! `extern_crates`, or `features`), also add it in `manifest.rs` so snapshot
//! behavior stays aligned. Per-suite keys (those in `Suite` / `RawSuite`,
//! such as `dev_deps`, `edition`, and `allow_lints`) are preserved verbatim
//! via the `raw_metadata` round-trip and do NOT require a `manifest.rs` change.
//!
//! ## Why only TOML
//!
//! Env-vars and auto-discovery fallbacks are avoided so configuration is explicit.
//! If `[package.metadata.lihaaf]` is missing, the harness fails early with a direct
//! message instead of inferring behavior from ambient layout.
//!
//! ## Suites
//!
//! A *suite* is a named bundle of (features, fixture_dirs, edition,
//! dev_deps, extern_crates, compile_fail_marker, fixture_timeout_secs,
//! per_fixture_memory_mb, allow_lints). The top-level `[package.metadata.lihaaf]`
//! table is always the implicit default suite; each
//! `[[package.metadata.lihaaf.suite]]` array entry contributes one
//! additional named suite. Each suite triggers an independent dylib
//! build with that suite's `features`, and each suite's fixtures are
//! compiled with that same feature set propagated to per-fixture rustc.
//!
//! Suite-level keys mostly inherit from the top-level table when omitted.
//! Exceptions are `name` (always required on a named suite), `fixture_dirs`
//! (required and disjoint across suites), `features` (replacement),
//! `build_targets` (replacement; omitted named suites get `[]`), and
//! `keep_foreign_span_bodies` (replacement; omitted named suites get
//! `false`, not the default suite's value).

use std::collections::BTreeSet;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Error, Outcome};
use crate::normalize::Substitution;

/// Default value for `fixture_dirs` when omitted. Callers with custom
/// layouts should override this key.
pub const DEFAULT_FIXTURE_DIRS: &[&str] =
    &["tests/lihaaf/compile_fail", "tests/lihaaf/compile_pass"];

/// Default value for `compile_fail_marker` when omitted.
pub const DEFAULT_COMPILE_FAIL_MARKER: &str = "compile_fail";

/// Default value for `edition` when omitted.
pub const DEFAULT_EDITION: &str = "2021";

/// Default value for `fixture_timeout_secs` when omitted.
pub const DEFAULT_FIXTURE_TIMEOUT_SECS: u32 = 90;

/// Default value for `per_fixture_memory_mb` when omitted. Chosen to give
/// heavy proc-macro fixtures headroom while still tripping the OOM guard
/// before the OS does.
pub const DEFAULT_PER_FIXTURE_MEMORY_MB: u32 = 1024;

/// Allowed editions.
pub const ALLOWED_EDITIONS: &[&str] = &["2015", "2018", "2021", "2024"];

/// Reserved name for the implicit suite that comes from the top-level
/// `[package.metadata.lihaaf]` table. A named suite that tries to claim
/// this name is rejected at validation time so CLI selection
/// (`--suite default`) is never ambiguous.
pub const DEFAULT_SUITE_NAME: &str = "default";

/// Parsed and validated `[package.metadata.lihaaf]` table.
///
/// After validation, all per-suite fields are concrete values with
/// defaults filled in. The session iterates [`Self::suites`] in the
/// stored order; index 0 is always the implicit "default" suite built
/// from the top-level table, and indices 1.. are named
/// `[[package.metadata.lihaaf.suite]]` entries in source order.
#[derive(Debug, Clone, Serialize)]
pub struct Config {
    /// Required workspace member crate to build as the dylib. One per
    /// session — `dylib_crate` is NOT overridable per-suite because the
    /// session-startup model assumes one consumer crate identity.
    pub dylib_crate: String,

    /// Verbatim copy of the raw `[package.metadata.lihaaf]` table for
    /// the manifest snapshot. Always populated by [`parse`]; `default`
    /// keeps serde round-tripping possible for tests that synthesize
    /// a `Config` without parsing text first.
    #[serde(default = "empty_toml_table")]
    pub raw_metadata: toml::Value,

    /// All suites in declared run order. `suites[0]` is always the
    /// implicit "default" suite (built from the top-level table);
    /// `suites[1..]` are named suites in source order. Validation
    /// guarantees `suites` is non-empty and every `name` is unique.
    pub suites: Vec<Suite>,
}

#[derive(Deserialize)]
struct SerdeConfig {
    dylib_crate: String,
    #[serde(default = "empty_toml_table")]
    raw_metadata: toml::Value,
    suites: Vec<Suite>,
}

impl<'de> Deserialize<'de> for Config {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = SerdeConfig::deserialize(deserializer)?;
        validate_build_targets_for_suites_for_serde(&raw.suites)
            .map_err(serde::de::Error::custom)?;
        Ok(Config {
            dylib_crate: raw.dylib_crate,
            raw_metadata: raw.raw_metadata,
            suites: raw.suites,
        })
    }
}

/// One feature-subset suite. Each suite carries an independent
/// (features, fixture_dirs, …) bundle and triggers its own dylib build.
///
/// Most keys inherit from the top-level `[package.metadata.lihaaf]` table
/// when omitted from a named suite. Replacement keys such as `features` and
/// `build_targets` resolve to their own defaults instead. The resolved values
/// are baked in here so downstream modules never re-resolve inheritance.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Suite {
    /// Suite name. The default suite is named [`DEFAULT_SUITE_NAME`].
    /// Named-suite names must be non-empty,
    /// `[A-Za-z0-9_-]`-only, and not equal to [`DEFAULT_SUITE_NAME`].
    pub name: String,

    /// Required crate names fixtures may `use` from. One `--extern` flag
    /// is emitted per entry. `extern_crates[0]` must equal
    /// [`Config::dylib_crate`]. Inherits from the default suite if
    /// omitted on a named suite.
    pub extern_crates: Vec<String>,

    /// Directories scanned for `*.rs` fixtures (non-recursive within
    /// each). Across suites these MUST be disjoint so snapshot files
    /// cannot collide between suites with different feature sets.
    pub fixture_dirs: Vec<PathBuf>,

    /// Cargo features enabled for both this suite's dylib build and its
    /// per-fixture rustc invocations. Empty by default. A named suite
    /// that omits `features` does NOT inherit the default suite's
    /// features — the explicit-replacement rule keeps a "spatial only"
    /// suite from accidentally pulling in a sibling testing feature.
    pub features: Vec<String>,

    /// Edition for the per-fixture rustc invocation. Inherits from the
    /// default suite if omitted on a named suite.
    pub edition: String,

    /// Extra crates beyond `extern_crates` that fixtures import directly
    /// (e.g., serde, serde_json). Resolved via the suite's deps dir and
    /// forwarded as `--extern` flags. Inherits from the default suite if
    /// omitted on a named suite.
    pub dev_deps: Vec<String>,

    /// Optional suite-level build targets that ask lihaaf to prebuild
    /// additional fixture support artifacts before per-fixture rustc
    /// runs. Empty by default. v0.1.0 accepts only `"tests"`.
    ///
    /// **Per-suite REPLACE semantics.** A named suite that omits
    /// `build_targets` gets `[]`, even if the default suite opts in.
    #[serde(default)]
    pub build_targets: BuildTargets,

    /// Substring that classifies a fixture's enclosing directory as
    /// compile_fail. Inherits from the default suite if omitted on a
    /// named suite.
    pub compile_fail_marker: String,

    /// Per-fixture rustc wall-clock timeout in seconds. Inherits from
    /// the default suite if omitted on a named suite.
    pub fixture_timeout_secs: u32,

    /// Max RSS in MB any single rustc worker may consume before being
    /// killed. Inherits from the default suite if omitted on a named
    /// suite.
    pub per_fixture_memory_mb: u32,

    /// rustc lints forwarded as `-A <lint>` on every per-fixture
    /// invocation. Empty by default. Inherits from the default suite
    /// if omitted on a named suite (same precedent as `dev_deps`).
    ///
    /// Each entry is passed verbatim as a single argv token via
    /// `std::process::Command::arg` — no shell expansion occurs.
    /// Unknown lint names are NOT pre-validated; rustc surfaces
    /// `warning: unknown lint: X` on the per-fixture stderr.
    ///
    /// Validation rejects: empty strings, entries starting with `-`
    /// (caller must not supply the `-A` prefix; lihaaf supplies it),
    /// and entries containing whitespace, double quotes, single quotes,
    /// or backslashes (would break argv tokenization).
    pub allow_lints: Vec<String>,

    /// Adopter-defined extra substitutions applied to normalized stderr
    /// AFTER built-in path placeholders and BEFORE TypeId collapse
    /// (see `docs/spec/lihaaf-v0.2.md` §6.6). Empty by default.
    ///
    /// **Per-suite REPLACE semantics.** A named suite that omits
    /// `extra_substitutions` does NOT inherit the default suite's
    /// substitutions; it gets `[]`. This matches the `features`
    /// precedent — see [`Self::features`].
    ///
    /// Validation runs at config-parse time:
    /// - Each entry's `from` must pass `is_path_like` (path-shaped:
    ///   contains `/`, `\`, or is a bare `$X` placeholder where `X`
    ///   starts with ASCII uppercase). This ensures substitutions
    ///   target path-shaped content only, not arbitrary diagnostic text.
    /// - Each entry's `to` must NOT contain a newline.
    pub extra_substitutions: Vec<Substitution>,

    /// Adopter-defined full-line exact-match drops applied to
    /// normalized stderr after trim-trailing-whitespace and BEFORE
    /// blank-line collapse. Empty by default.
    ///
    /// **Per-suite REPLACE semantics** — same as
    /// [`Self::extra_substitutions`]. Each entry must pass
    /// `is_path_like` OR `is_banner_shape` per config-parse validation
    /// (see `docs/spec/lihaaf-v0.2.md` §6.6).
    ///
    /// The [`StripPattern`] wrapper guarantees this invariant holds on
    /// every construction path, including direct serde deserialization.
    pub strip_lines: Vec<StripPattern>,

    /// Adopter-defined prefix-match drops applied to normalized
    /// stderr after trim-trailing-whitespace and BEFORE blank-line
    /// collapse. Empty by default.
    ///
    /// **Per-suite REPLACE semantics** — same as
    /// [`Self::extra_substitutions`]. Each entry must pass
    /// `is_path_like` OR `is_banner_shape` per config-parse validation.
    ///
    /// The [`StripPattern`] wrapper guarantees this invariant holds on
    /// every construction path, including direct serde deserialization.
    pub strip_line_prefixes: Vec<StripPattern>,

    /// Adopter-defined foreign-span body-suppression opt-out. When `false`
    /// (the default), the source body of any `-->`/`:::` span whose pointer
    /// resolves to a non-fixture placeholder (`$RUST` / `$CARGO` /
    /// `$WORKSPACE`) collapses to a single kind-matched placeholder line. Set
    /// `true` to PRESERVE those bodies (upstream-regression detection).
    /// Governs body CONTENT only — the pointer's `:LINE:COL` tail is stripped
    /// regardless of this flag.
    ///
    /// **Per-suite REPLACE semantics.** A named suite that omits it gets the
    /// default `false`, NOT the default suite's value — same precedent as
    /// `features`.
    #[serde(default)]
    pub keep_foreign_span_bodies: bool,
}

impl Suite {
    /// True for the implicit suite built from the top-level table.
    /// Used by session reporting + manifest naming to keep single-suite
    /// output byte-identical for adopters who never add a named suite.
    pub fn is_default(&self) -> bool {
        self.name == DEFAULT_SUITE_NAME
    }
}

/// A validated strip-pattern string. Wraps a `String` that has already
/// passed the `is_path_like(s) || is_banner_shape(s)` predicate.
///
/// Used for [`Suite::strip_lines`] and [`Suite::strip_line_prefixes`].
/// The `#[serde(try_from = "String")]` attribute ensures that ANY
/// deserialization path — TOML manifest, serde-JSON direct construction,
/// or any other format — routes through [`TryFrom<String>`], closing the
/// bypass that would otherwise let `serde_json::from_value::<Config>`
/// skip `validate_strip_patterns`.
///
/// # Invariant
///
/// Every `StripPattern` instance satisfies
/// `is_path_like(s) || is_banner_shape(s)` by construction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct StripPattern(String);

impl StripPattern {
    /// Borrow the inner validated string.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for StripPattern {
    type Error = String;

    /// Validate `s` against `is_path_like(s) || is_banner_shape(s)`.
    ///
    /// Called automatically by serde when deserializing `Suite.strip_lines`
    /// or `Suite.strip_line_prefixes` via any format (TOML, JSON, etc.).
    ///
    /// # Errors
    ///
    /// Returns an error string when `s` fails both predicates. The string
    /// is surfaced by serde as a deserialization error.
    fn try_from(s: String) -> Result<Self, Self::Error> {
        if s.contains('\n') {
            return Err(format!(
                "\"{s}\" contains a newline character; strip patterns must be single-line."
            ));
        }
        if !is_path_like(&s) && !is_banner_shape(&s) {
            return Err(format!(
                "\"{s}\" is neither path-shaped nor banner-shaped \
                 (must contain '/', '\\\\', start with a $X placeholder token where X is an \
                 ASCII uppercase letter, OR match the banner allowlist — \
                 see docs/spec/lihaaf-v0.2.md §6.6). \
                 Patterns starting with '$' must have an ASCII uppercase letter immediately after, \
                 regardless of path separators — '$lowercase/path' is rejected. \
                 Bare placeholder patterns are full-string anchored: \
                 '$DIR-', '$RUST.', '$A!' are rejected; '$DIR/x' is accepted \
                 via the path-separator branch. \
                 Strip patterns target path-shaped environment noise OR known banner shapes only."
            ));
        }
        Ok(StripPattern(s))
    }
}

impl From<StripPattern> for String {
    fn from(p: StripPattern) -> String {
        p.0
    }
}

/// Validated `build_targets` list. v0.1.0 accepts only `"tests"` and
/// rejects duplicate targets on every serde construction path.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "Vec<String>", into = "Vec<String>")]
pub struct BuildTargets(Vec<String>);

impl BuildTargets {
    /// True when the suite has no `build_targets` entries (uses the
    /// dylib-only build path).
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Borrow the validated target list.
    pub fn as_slice(&self) -> &[String] {
        &self.0
    }
}

impl TryFrom<Vec<String>> for BuildTargets {
    type Error = String;

    /// Validate `build_targets` against the v0.1.0 allow-list.
    ///
    /// # Errors
    ///
    /// Returns an error when an entry is not `"tests"` or when a target
    /// is duplicated. Serde surfaces this string as a deserialization
    /// error for direct JSON/TOML construction paths.
    fn try_from(targets: Vec<String>) -> Result<Self, Self::Error> {
        let mut seen = std::collections::BTreeSet::new();
        for target in &targets {
            if target != "tests" {
                return Err(format!(
                    "build_targets entry \"{target}\" is not supported. \
                     v0.1.0 accepts only \"tests\"."
                ));
            }
            if !seen.insert(target.as_str()) {
                return Err(format!(
                    "build_targets contains duplicate target \"{target}\". \
                     Each build target may appear at most once."
                ));
            }
        }
        Ok(BuildTargets(targets))
    }
}

impl From<BuildTargets> for Vec<String> {
    fn from(targets: BuildTargets) -> Vec<String> {
        targets.0
    }
}

fn empty_toml_table() -> toml::Value {
    toml::Value::Table(toml::map::Map::new())
}

/// The intermediate "as parsed before validation" shape for the
/// top-level table. `Option` fields allow defaults to be applied
/// uniformly. This struct never escapes [`parse`].
#[derive(Debug, Default, Deserialize)]
struct RawMetadata {
    dylib_crate: Option<String>,
    extern_crates: Option<Vec<String>>,
    fixture_dirs: Option<Vec<String>>,
    features: Option<Vec<String>>,
    edition: Option<String>,
    dev_deps: Option<Vec<String>>,
    build_targets: Option<Vec<String>>,
    compile_fail_marker: Option<String>,
    fixture_timeout_secs: Option<u32>,
    per_fixture_memory_mb: Option<u32>,
    allow_lints: Option<Vec<String>>,
    /// Adopter-defined extra substitutions.
    /// Per-suite REPLACE semantics; omission on a named suite gives `[]`.
    extra_substitutions: Option<Vec<RawSubstitution>>,
    /// Adopter-defined exact-match line drops.
    strip_lines: Option<Vec<String>>,
    /// Adopter-defined prefix-match line drops.
    strip_line_prefixes: Option<Vec<String>>,
    /// Adopter-defined foreign-span body-suppression opt-out.
    /// Per-suite REPLACE semantics; omission on a named suite gives `false`.
    keep_foreign_span_bodies: Option<bool>,
    /// `[[package.metadata.lihaaf.suite]]` array entries.
    #[serde(default)]
    suite: Vec<RawSuite>,
}

/// As-parsed shape for one named `[[package.metadata.lihaaf.suite]]`
/// entry. Inheritance from the top-level table is applied in
/// [`finalize_named_suite`].
#[derive(Debug, Default, Deserialize)]
struct RawSuite {
    name: Option<String>,
    extern_crates: Option<Vec<String>>,
    fixture_dirs: Option<Vec<String>>,
    features: Option<Vec<String>>,
    edition: Option<String>,
    dev_deps: Option<Vec<String>>,
    build_targets: Option<Vec<String>>,
    compile_fail_marker: Option<String>,
    fixture_timeout_secs: Option<u32>,
    per_fixture_memory_mb: Option<u32>,
    allow_lints: Option<Vec<String>>,
    /// Per-suite REPLACE semantics; omission on a named suite gives `[]`
    /// regardless of the default suite's value (same precedent as
    /// `features`).
    extra_substitutions: Option<Vec<RawSubstitution>>,
    /// Per-suite REPLACE semantics.
    strip_lines: Option<Vec<String>>,
    /// Per-suite REPLACE semantics.
    strip_line_prefixes: Option<Vec<String>>,
    /// Per-suite REPLACE semantics.
    keep_foreign_span_bodies: Option<bool>,
    /// `dylib_crate` is intentionally NOT a per-suite key. Reading any
    /// value here is rejected at validation time so a typo can't be
    /// silently dropped.
    dylib_crate: Option<String>,
}

/// As-parsed shape for one `extra_substitutions` entry. Promoted to
/// the validated [`Substitution`] in [`build_default_suite`] /
/// [`finalize_named_suite`] after [`validate_extra_substitutions`]
/// passes.
///
/// Exposed as `pub(crate)` so that [`crate::normalize::Substitution`]'s
/// `#[serde(try_from = "RawSubstitution")]` impl can reference this type
/// and route all external-format deserialization through the same
/// `is_path_like` + no-newline validation that the TOML parse path uses.
#[derive(Debug, Default, Deserialize, Clone)]
pub(crate) struct RawSubstitution {
    pub(crate) from: Option<String>,
    pub(crate) to: Option<String>,
}

impl TryFrom<RawSubstitution> for Substitution {
    type Error = String;

    /// Validate a raw deserialized `extra_substitutions` entry and
    /// promote it to the typed [`Substitution`] value.
    ///
    /// Enforces exactly the rules that [`validate_extra_substitutions`]
    /// applies on the TOML parse path, so every construction route —
    /// TOML manifest, direct serde JSON, or any other format — goes
    /// through the same guards.
    ///
    /// Validation order mirrors [`finalize_substitutions`] →
    /// [`validate_extra_substitutions`]: both presence checks fire first,
    /// then shape checks, so multiply-invalid inputs produce the same
    /// first-error message on both paths.
    ///
    /// - `from` must be present.
    /// - `to` must be present (may be empty string).
    /// - `from` must be non-empty.
    /// - `from` must pass [`is_path_like`].
    /// - `to` must not contain a newline character.
    ///
    /// # Errors
    ///
    /// Returns an error string when any of the above rules are violated.
    /// The string is surfaced by serde as a deserialization error so
    /// adopters who construct `Config` / `Suite` from JSON or other
    /// formats see an actionable message.
    fn try_from(raw: RawSubstitution) -> Result<Self, Self::Error> {
        // Ordering mirrors the TOML path (finalize_substitutions →
        // validate_extra_substitutions): both presence checks fire before
        // either shape check, so multiply-invalid inputs produce the same
        // first error regardless of which deserialization path is taken.
        let from = raw.from.ok_or_else(|| {
            "extra_substitutions entry is missing the required `from` key. \
             Every entry must specify which substring to match."
                .to_string()
        })?;
        let to = raw.to.ok_or_else(|| {
            "extra_substitutions entry is missing the required `to` key. \
             Use an empty string `to = \"\"` to strip the match."
                .to_string()
        })?;
        if from.is_empty() {
            return Err("extra_substitutions entry `from` is empty. \
                 An empty `from` would match the start of every byte and rewrite arbitrary \
                 content. extra_substitutions is for path-shaped substitution only. \
                 See docs/spec/lihaaf-v0.2.md §6.6."
                .to_string());
        }
        if !is_path_like(&from) {
            return Err(format!(
                "extra_substitutions `from` = \"{from}\" is not path-like \
                 (must contain '/', '\\\\', or be a bare $X placeholder token \
                 where X is an ASCII uppercase letter). \
                 Patterns starting with '$' must have an ASCII uppercase letter \
                 immediately after, regardless of path separators — \
                 '$lowercase/path' is rejected. \
                 Bare placeholder patterns are full-string anchored: \
                 '$DIR-', '$RUST.', '$A!' are rejected; '$DIR/x' is accepted \
                 via the path-separator branch. \
                 extra_substitutions is for path-shaped substitution only, \
                 not arbitrary text rewriting. See docs/spec/lihaaf-v0.2.md §6.6.",
            ));
        }
        if to.contains('\n') {
            return Err("extra_substitutions `to` contains a newline character; \
                 replacements must be single-line. \
                 A multi-line `to` would inject blank lines into normalized \
                 stderr and break snapshot determinism."
                .to_string());
        }
        Ok(Substitution { from, to })
    }
}

/// Load the consumer crate's `Cargo.toml`, extract
/// `[package.metadata.lihaaf]`, and validate it.
///
/// `manifest_path` should point at the consumer crate's `Cargo.toml`
/// (not a workspace root). Caller is responsible for resolving
/// `--manifest-path` overrides and the cargo "current dir + parent
/// walk" default.
pub fn load(manifest_path: &Path) -> Result<Config, Error> {
    let bytes = std::fs::read_to_string(manifest_path).map_err(|e| {
        Error::io(
            e,
            "reading consumer Cargo.toml",
            Some(manifest_path.to_path_buf()),
        )
    })?;
    parse(&bytes, manifest_path)
}

/// Same as [`load`] but reads from a string. Used by tests.
pub fn parse(toml_text: &str, manifest_path: &Path) -> Result<Config, Error> {
    // toml 1.x: `FromStr for Value` parses a single value (not a
    // document). `toml::from_str::<Value>` keeps the document-parse
    // path explicit and serde-routed.
    let value: toml::Value =
        toml::from_str(toml_text).map_err(|e: toml::de::Error| Error::TomlParse {
            path: manifest_path.to_path_buf(),
            message: e.to_string(),
        })?;

    // Walk to `package.metadata.lihaaf`. Missing at any step is a hard config
    // failure; keep the failure direct and actionable.
    let raw_metadata_value = value
        .get("package")
        .and_then(|v| v.get("metadata"))
        .and_then(|v| v.get("lihaaf"))
        .cloned()
        .ok_or_else(|| {
            Error::Session(Outcome::ConfigInvalid {
                message: missing_metadata_message(),
            })
        })?;

    let raw: RawMetadata =
        raw_metadata_value
            .clone()
            .try_into()
            .map_err(|e: toml::de::Error| {
                Error::Session(Outcome::ConfigInvalid {
                    message: format!(
                        "[package.metadata.lihaaf] could not be parsed.\n  {e}\nWhy this matters: the harness needs typed values to dispatch fixtures."
                    ),
                })
            })?;

    let dylib_crate = raw.dylib_crate.clone().unwrap_or_default();
    if dylib_crate.is_empty() {
        return Err(Error::Session(Outcome::ConfigInvalid {
            message: format_invalid_key(
                "dylib_crate",
                "a non-empty workspace-member crate name",
                "lihaaf needs to know which crate to build as the dylib",
            ),
        }));
    }
    validate_dylib_crate(&dylib_crate)?;

    // Build the default suite from the top-level keys. The default suite
    // is always present in `Config::suites` even when no `[[suite]]`
    // entries are declared, so single-suite adopters see no behavior
    // change.
    let default_suite = build_default_suite(&dylib_crate, &raw)?;

    // Build named suites with inheritance from the default suite.
    let mut suites = Vec::with_capacity(1 + raw.suite.len());
    suites.push(default_suite);
    for (idx, raw_suite) in raw.suite.into_iter().enumerate() {
        // Borrow the default suite immutably for inheritance lookup.
        // suites[0] is the only entry at this point.
        let suite = {
            let default = &suites[0];
            finalize_named_suite(&dylib_crate, default, idx, raw_suite)?
        };
        suites.push(suite);
    }

    // Cross-suite invariants.
    validate_unique_suite_names(&suites)?;
    validate_disjoint_fixture_dirs(manifest_path, &suites)?;
    validate_build_targets_for_suites(&suites)?;

    Ok(Config {
        dylib_crate,
        raw_metadata: raw_metadata_value,
        suites,
    })
}

fn build_default_suite(dylib_crate: &str, raw: &RawMetadata) -> Result<Suite, Error> {
    let extern_crates = raw.extern_crates.clone().unwrap_or_default();
    if extern_crates.is_empty() {
        return Err(Error::Session(Outcome::ConfigInvalid {
            message: format_invalid_key(
                "extern_crates",
                "a non-empty array of crate names; the first must equal `dylib_crate`",
                "every fixture compiles with one --extern <name>=<path> per entry",
            ),
        }));
    }
    if extern_crates[0] != dylib_crate {
        return Err(Error::Session(Outcome::ConfigInvalid {
            message: format!(
                "extern_crates[0] (\"{}\") must equal dylib_crate (\"{}\").\nWhy this matters: the dylib's `--extern` flag is the link the fixture takes back to the consumer crate.",
                extern_crates[0], dylib_crate
            ),
        }));
    }

    let fixture_dirs: Vec<PathBuf> = raw
        .fixture_dirs
        .clone()
        .unwrap_or_else(|| DEFAULT_FIXTURE_DIRS.iter().map(|s| s.to_string()).collect())
        .into_iter()
        .map(PathBuf::from)
        .collect();

    let edition = raw
        .edition
        .clone()
        .unwrap_or_else(|| DEFAULT_EDITION.to_string());
    validate_edition(DEFAULT_SUITE_NAME, &edition)?;

    let fixture_timeout_secs = raw
        .fixture_timeout_secs
        .unwrap_or(DEFAULT_FIXTURE_TIMEOUT_SECS);
    if fixture_timeout_secs == 0 {
        return Err(Error::Session(Outcome::ConfigInvalid {
            message: format_invalid_key(
                "fixture_timeout_secs",
                "a positive integer (seconds of wall-clock per fixture)",
                "a zero timeout would kill every fixture immediately",
            ),
        }));
    }

    let per_fixture_memory_mb = raw
        .per_fixture_memory_mb
        .unwrap_or(DEFAULT_PER_FIXTURE_MEMORY_MB);
    if per_fixture_memory_mb == 0 {
        return Err(Error::Session(Outcome::ConfigInvalid {
            message: format_invalid_key(
                "per_fixture_memory_mb",
                "a positive integer (megabytes per fixture)",
                "a zero ceiling would kill every fixture instantly",
            ),
        }));
    }

    let allow_lints = raw.allow_lints.clone().unwrap_or_default();
    validate_allow_lints(DEFAULT_SUITE_NAME, &allow_lints)?;

    let features = raw.features.clone().unwrap_or_default();
    validate_features(DEFAULT_SUITE_NAME, &features)?;

    let extra_substitutions = finalize_substitutions(
        DEFAULT_SUITE_NAME,
        raw.extra_substitutions.clone().unwrap_or_default(),
    )?;
    validate_extra_substitutions(DEFAULT_SUITE_NAME, &extra_substitutions)?;

    let strip_lines_raw = raw.strip_lines.clone().unwrap_or_default();
    validate_strip_patterns(DEFAULT_SUITE_NAME, "strip_lines", &strip_lines_raw)?;
    // SAFETY: validate_strip_patterns just confirmed every entry passes
    // `is_path_like || is_banner_shape`; construct StripPattern values
    // directly to avoid re-running the predicate.
    let strip_lines: Vec<StripPattern> = strip_lines_raw.into_iter().map(StripPattern).collect();

    let strip_line_prefixes_raw = raw.strip_line_prefixes.clone().unwrap_or_default();
    validate_strip_patterns(
        DEFAULT_SUITE_NAME,
        "strip_line_prefixes",
        &strip_line_prefixes_raw,
    )?;
    // SAFETY: same rationale as strip_lines above.
    let strip_line_prefixes: Vec<StripPattern> = strip_line_prefixes_raw
        .into_iter()
        .map(StripPattern)
        .collect();

    Ok(Suite {
        name: DEFAULT_SUITE_NAME.to_string(),
        extern_crates,
        fixture_dirs,
        features,
        edition,
        dev_deps: raw.dev_deps.clone().unwrap_or_default(),
        build_targets: raw
            .build_targets
            .clone()
            .unwrap_or_default()
            .try_into()
            .map_err(|message| Error::Session(Outcome::ConfigInvalid { message }))?,
        compile_fail_marker: raw
            .compile_fail_marker
            .clone()
            .unwrap_or_else(|| DEFAULT_COMPILE_FAIL_MARKER.to_string()),
        fixture_timeout_secs,
        per_fixture_memory_mb,
        allow_lints,
        extra_substitutions,
        strip_lines,
        strip_line_prefixes,
        keep_foreign_span_bodies: raw.keep_foreign_span_bodies.unwrap_or(false),
    })
}

fn finalize_named_suite(
    dylib_crate: &str,
    default_suite: &Suite,
    index: usize,
    raw: RawSuite,
) -> Result<Suite, Error> {
    if raw.dylib_crate.is_some() {
        return Err(Error::Session(Outcome::ConfigInvalid {
            message: format!(
                "[[package.metadata.lihaaf.suite]] entry #{index} sets `dylib_crate`, which is not a per-suite key.\nWhy this matters: lihaaf builds one consumer crate per session; the suite system varies the FEATURE SET passed to that crate, not the crate identity."
            ),
        }));
    }

    let name = raw.name.unwrap_or_default();
    validate_named_suite_name(index, &name)?;

    let extern_crates = raw
        .extern_crates
        .unwrap_or_else(|| default_suite.extern_crates.clone());
    if extern_crates.is_empty() {
        return Err(Error::Session(Outcome::ConfigInvalid {
            message: format!(
                "suite \"{name}\".extern_crates is empty.\nWhy this matters: every fixture needs at least one --extern flag (the dylib_crate)."
            ),
        }));
    }
    if extern_crates[0] != dylib_crate {
        return Err(Error::Session(Outcome::ConfigInvalid {
            message: format!(
                "suite \"{name}\".extern_crates[0] (\"{}\") must equal dylib_crate (\"{}\").\nWhy this matters: the dylib's `--extern` flag is the link the fixture takes back to the consumer crate.",
                extern_crates[0], dylib_crate
            ),
        }));
    }

    let raw_dirs = raw.fixture_dirs.ok_or_else(|| {
        Error::Session(Outcome::ConfigInvalid {
            message: format!(
                "suite \"{name}\".fixture_dirs is required.\nWhy this matters: a named suite must declare its own fixture directories so its snapshot files don't collide with another suite's."
            ),
        })
    })?;
    if raw_dirs.is_empty() {
        return Err(Error::Session(Outcome::ConfigInvalid {
            message: format!(
                "suite \"{name}\".fixture_dirs is an empty array.\nWhy this matters: a named suite that runs zero fixtures contributes no signal."
            ),
        }));
    }
    let fixture_dirs: Vec<PathBuf> = raw_dirs.into_iter().map(PathBuf::from).collect();

    let edition = raw.edition.unwrap_or_else(|| default_suite.edition.clone());
    validate_edition(&name, &edition)?;

    let fixture_timeout_secs = raw
        .fixture_timeout_secs
        .unwrap_or(default_suite.fixture_timeout_secs);
    if fixture_timeout_secs == 0 {
        return Err(Error::Session(Outcome::ConfigInvalid {
            message: format!(
                "suite \"{name}\".fixture_timeout_secs must be a positive integer.\nWhy this matters: a zero timeout would kill every fixture immediately."
            ),
        }));
    }

    let per_fixture_memory_mb = raw
        .per_fixture_memory_mb
        .unwrap_or(default_suite.per_fixture_memory_mb);
    if per_fixture_memory_mb == 0 {
        return Err(Error::Session(Outcome::ConfigInvalid {
            message: format!(
                "suite \"{name}\".per_fixture_memory_mb must be a positive integer.\nWhy this matters: a zero ceiling would kill every fixture instantly."
            ),
        }));
    }

    let allow_lints = raw
        .allow_lints
        .unwrap_or_else(|| default_suite.allow_lints.clone());
    validate_allow_lints(&name, &allow_lints)?;

    // Features intentionally do NOT inherit: a "spatial only" suite
    // shouldn't accidentally pull in the default suite's `testing`
    // feature. Adopters who want shared features must list them in
    // both places.
    let features = raw.features.unwrap_or_default();
    validate_features(&name, &features)?;

    // Per-suite REPLACE semantics for the three custom normalizer keys
    // (extra_substitutions / strip_lines / strip_line_prefixes) —
    // matches the `features` precedent (see `Suite::features`
    // rustdoc). A named suite that omits the key gets `[]`, not the
    // default suite's value. This is documented in §3.6 of the spec
    // and pinned by regression tests
    // (`extra_substitutions_omitted_on_named_suite_is_empty`,
    // `strip_patterns_omitted_on_named_suite_is_empty`).
    let extra_substitutions =
        finalize_substitutions(&name, raw.extra_substitutions.unwrap_or_default())?;
    validate_extra_substitutions(&name, &extra_substitutions)?;

    let strip_lines_raw = raw.strip_lines.unwrap_or_default();
    validate_strip_patterns(&name, "strip_lines", &strip_lines_raw)?;
    // SAFETY: validate_strip_patterns just confirmed every entry passes
    // `is_path_like || is_banner_shape`; construct StripPattern values
    // directly to avoid re-running the predicate.
    let strip_lines: Vec<StripPattern> = strip_lines_raw.into_iter().map(StripPattern).collect();

    let strip_line_prefixes_raw = raw.strip_line_prefixes.unwrap_or_default();
    validate_strip_patterns(&name, "strip_line_prefixes", &strip_line_prefixes_raw)?;
    // SAFETY: same rationale as strip_lines above.
    let strip_line_prefixes: Vec<StripPattern> = strip_line_prefixes_raw
        .into_iter()
        .map(StripPattern)
        .collect();

    Ok(Suite {
        name,
        extern_crates,
        fixture_dirs,
        features,
        edition,
        dev_deps: raw
            .dev_deps
            .unwrap_or_else(|| default_suite.dev_deps.clone()),
        build_targets: raw
            .build_targets
            .unwrap_or_default()
            .try_into()
            .map_err(|message| Error::Session(Outcome::ConfigInvalid { message }))?,
        compile_fail_marker: raw
            .compile_fail_marker
            .unwrap_or_else(|| default_suite.compile_fail_marker.clone()),
        fixture_timeout_secs,
        per_fixture_memory_mb,
        allow_lints,
        extra_substitutions,
        strip_lines,
        strip_line_prefixes,
        keep_foreign_span_bodies: raw.keep_foreign_span_bodies.unwrap_or(false),
    })
}

fn validate_named_suite_name(index: usize, name: &str) -> Result<(), Error> {
    if name.is_empty() {
        return Err(Error::Session(Outcome::ConfigInvalid {
            message: format!(
                "[[package.metadata.lihaaf.suite]] entry #{index} is missing the required `name` key.\nWhy this matters: lihaaf addresses suites by name on the CLI (`--suite NAME`) and in per-suite manifest paths."
            ),
        }));
    }
    if name == DEFAULT_SUITE_NAME {
        return Err(Error::Session(Outcome::ConfigInvalid {
            message: format!(
                "[[package.metadata.lihaaf.suite]] name \"{DEFAULT_SUITE_NAME}\" is reserved for the implicit suite built from the top-level [package.metadata.lihaaf] table.\nWhy this matters: a CLI invocation `--suite default` would be ambiguous if a named suite also claimed the name."
            ),
        }));
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(Error::Session(Outcome::ConfigInvalid {
            message: format!(
                "[[package.metadata.lihaaf.suite]] name \"{name}\" must contain only ASCII alphanumeric characters, hyphens, or underscores.\nWhy this matters: the suite name is used in filesystem paths (`target/lihaaf/manifest-<name>.json`, `target/lihaaf-build-<name>/`) and on the CLI."
            ),
        }));
    }
    Ok(())
}

fn validate_edition(suite_label: &str, edition: &str) -> Result<(), Error> {
    if !ALLOWED_EDITIONS.contains(&edition) {
        return Err(Error::Session(Outcome::ConfigInvalid {
            message: format!(
                "{suite_label}.edition \"{edition}\" is not in the allowed set ({}).\nWhy this matters: rustc's `--edition` accepts only those values.",
                ALLOWED_EDITIONS.join(", ")
            ),
        }));
    }
    Ok(())
}

fn validate_build_targets_for_suites(suites: &[Suite]) -> Result<(), Error> {
    validate_build_targets_for_suites_for_serde(suites)
        .map_err(|message| Error::Session(Outcome::ConfigInvalid { message }))
}

fn validate_build_targets_for_suites_for_serde(suites: &[Suite]) -> Result<(), String> {
    for suite in suites {
        if !suite.build_targets.is_empty() && suite.dev_deps.is_empty() {
            return Err(format!(
                "suite \"{}\" sets build_targets = {:?} but has no final dev_deps.\n\
                     Why this matters: build_targets = [\"tests\"] asks lihaaf to collect \
                     fixture-only dependency artifacts, but there is no dependency graph to collect.",
                suite.name,
                suite.build_targets.as_slice()
            ));
        }
    }
    Ok(())
}

/// Validate every entry in `allow_lints` against the structural rules:
/// no empty strings, no NUL bytes, no leading `-` (caller must not supply
/// the `-A` prefix), and no whitespace / quote / backslash characters (would
/// break argv tokenization or smuggle extra flags past rustc).
///
/// Unknown lint names are NOT pre-validated; rustc surfaces
/// `warning: unknown lint: X` itself.
fn validate_allow_lints(suite_label: &str, lints: &[String]) -> Result<(), Error> {
    for lint in lints {
        if lint.is_empty() {
            return Err(Error::Session(Outcome::ConfigInvalid {
                message: format!(
                    "{suite_label}.allow_lints contains an empty string.\n\
                     Why this matters: an empty string is not a valid lint name and would produce an unrecognized flag on the rustc argv."
                ),
            }));
        }
        if lint.contains('\0') {
            return Err(Error::Session(Outcome::ConfigInvalid {
                message: format!(
                    "{suite_label}.allow_lints entry contains a NUL byte.\n\
                     Why this matters: an interior NUL byte cannot appear in a POSIX argv token; \
                     spawn would reject the argv and the failure would surface as WORKER_CRASHED \
                     instead of an actionable CONFIG_INVALID."
                ),
            }));
        }
        if lint.starts_with('-') {
            return Err(Error::Session(Outcome::ConfigInvalid {
                message: format!(
                    "{suite_label}.allow_lints entry \"{lint}\" starts with `-`.\n\
                     Why this matters: lihaaf supplies the `-A` prefix itself; including it in the entry would produce `-A -A <lint>` on the rustc argv."
                ),
            }));
        }
        if lint
            .chars()
            .any(|c| c.is_whitespace() || c == '"' || c == '\'' || c == '\\')
        {
            return Err(Error::Session(Outcome::ConfigInvalid {
                message: format!(
                    "{suite_label}.allow_lints entry \"{lint}\" contains whitespace, quotes, or a backslash.\n\
                     Why this matters: each entry must be a single argv token; whitespace or shell-meta characters would either break argv tokenization or smuggle extra flags past rustc's argument parser."
                ),
            }));
        }
    }
    Ok(())
}

/// Validate every entry in `features` against the structural rules:
/// no empty strings and no NUL bytes.
///
/// Each entry is forwarded as a single argv token to cargo (`--features <f>`)
/// and to rustc (`--cfg feature="<f>"`). Empty strings and NUL bytes cannot
/// appear in a POSIX argv token; an interior NUL would cause spawn to reject
/// the argv and the failure would surface as WORKER_CRASHED instead of an
/// actionable CONFIG_INVALID.
///
/// Other character restrictions (whitespace, shell-meta) are intentionally
/// out of scope here — Cargo itself validates feature-name syntax and will
/// surface those errors with precise diagnostics.
fn validate_features(suite_label: &str, features: &[String]) -> Result<(), Error> {
    for feature in features {
        if feature.is_empty() {
            return Err(Error::Session(Outcome::ConfigInvalid {
                message: format!(
                    "{suite_label}.features contains an empty string.\n\
                     Why this matters: each entry must be a single argv token forwarded as \
                     `--features` to cargo and `--cfg feature=\"...\"` to rustc; an empty \
                     string is not a valid feature name."
                ),
            }));
        }
        if feature.contains('\0') {
            return Err(Error::Session(Outcome::ConfigInvalid {
                message: format!(
                    "{suite_label}.features entry contains a NUL byte.\n\
                     Why this matters: each entry must be a single argv token forwarded as \
                     `--features` to cargo and `--cfg feature=\"...\"` to rustc; an interior \
                     NUL byte cannot appear in a POSIX argv token and spawn would reject it, \
                     surfacing as WORKER_CRASHED instead of an actionable CONFIG_INVALID."
                ),
            }));
        }
    }
    Ok(())
}

/// True iff `s` is path-shaped per `docs/spec/lihaaf-v0.2.md` §6.6:
///
/// 1. `s.len() >= 2` (bytes).
/// 2. `s` contains no `\n` byte.
/// 3. **Leading-`$` guard**: if `s.as_bytes()[0] == b'$'`, then
///    `s.as_bytes()[1]` MUST be ASCII uppercase. Fires BEFORE the
///    disjunction below and is unconditional — a leading-`$` pattern
///    whose second byte is not ASCII uppercase is rejected even if
///    the string also contains `/` or `\`. Rule 1 already guarantees
///    `s.len() >= 2`, so accessing `s.as_bytes()[1]` is safe.
///
///    Rule 3 only fires on the LEADING `$` byte. Interior
///    `$lowercase` (e.g., `/path/$nix/sub`) is path text and passes
///    via rule 4(a) — see Class 9 in the test matrix.
/// 4. At least one of:
///    - (a) `s.contains('/')`, OR
///    - (b) `s.contains('\\')`, OR
///    - (c) `s` matches `^\$[A-Z][A-Za-z0-9_]*$` (full-string-anchored,
///      NOT a prefix match: `s` starts with `$`, second byte is
///      `[A-Z]`, every byte after that is in `[A-Za-z0-9_]`, AND no
///      extra bytes follow the placeholder tail). Admits `$DIR`,
///      `$WORKSPACE`, `$NIX_STORE`; rejects `$DIR-`, `$A!`.
///      (`$DIR/x` still passes via (4a) because it contains `/`.)
///
/// This predicate gates `extra_substitutions.from` and is one half
/// of the strip-key disjunction (`is_path_like || is_banner_shape`).
/// Diagnostic-text patterns (`error`, `error:`, `  |`,
/// `expected due to this`) fail every disjunction alternative, so
/// adopter substitutions cannot match diagnostic prose.
fn is_path_like(s: &str) -> bool {
    let bytes = s.as_bytes();
    if bytes.len() < 2 {
        return false;
    }
    if s.contains('\n') {
        return false;
    }
    // Rule 3: leading-`$` guard. Unconditional; fires before the
    // disjunction so a `$lowercase/path` shape is rejected even with
    // a path separator present.
    if bytes[0] == b'$' && !bytes[1].is_ascii_uppercase() {
        return false;
    }
    // Disjunction.
    if s.contains('/') || s.contains('\\') {
        return true;
    }
    is_complete_placeholder_token(s)
}

/// True iff `s` matches the rule (4c) "complete placeholder token"
/// shape: `^\$[A-Z][A-Za-z0-9_]*$` (full-string anchored).
///
/// Separate helper so tests can verify that `$DIR-`, `$A!`, `$RUST.`,
/// `$DIR ` (trailing space), `$WORKSPACE,`, and `$DIR/x` all fail
/// (4c) when isolated — proving rule (4c) is full-string anchored,
/// not a prefix match. (`$DIR/x` still passes `is_path_like` overall
/// because it contains `/` — see rule 4(a).)
fn is_complete_placeholder_token(s: &str) -> bool {
    let bytes = s.as_bytes();
    if bytes.len() < 2 {
        return false;
    }
    if bytes[0] != b'$' {
        return false;
    }
    if !bytes[1].is_ascii_uppercase() {
        return false;
    }
    // Every byte after `$<uppercase>` must be in [A-Za-z0-9_]. No
    // additional non-token characters allowed (`-`, `.`, ` `, `,`,
    // `!`, `/`, `\`, etc.).
    for &b in &bytes[2..] {
        if !(b.is_ascii_alphanumeric() || b == b'_') {
            return false;
        }
    }
    true
}

/// Anti-prefix list for [`is_banner_shape`]. REJECT if `s` starts
/// with any of these case-sensitive strings. Catches the
/// diagnostic-message-body shape family rustc commonly emits.
///
/// `error[` blocks `error[E0277]`-style code lines without
/// rejecting `error: aborting due to ...` (which uses a colon-space
/// separator and is handled in the banner-prefix list below).
const BANNER_ANTI_PREFIXES: &[&str] = &[
    "expected ",
    "found ",
    "the trait ",
    "the type ",
    "cannot find ",
    "mismatched types",
    "consider ",
    "help: ",
    "warning: ",
    "error[",
    "  ", // two spaces — span context (also caught by (A.3); defense-in-depth)
];

/// Enumerated banner-prefix list for [`is_banner_shape`] rule (C.1).
/// `s` is accepted via (C.1) when it starts with one of these
/// case-sensitive strings.
///
/// Covers the rustc-emitted banner trailers (#1–3) and the non-rustc
/// tool-version banner shape (`info:`, `linker version:`). The
/// shorter `"linker: "` prefix was removed because it admitted
/// strings below the 20-byte length floor; see plan §3.3.2 (C.1).
///
/// This list is closed; a future release may add an adopter-extensible
/// prefix list.
const BANNER_PREFIXES: &[&str] = &[
    "For more information about this error",
    "error: aborting due to ",
    "note: this error originates from ",
    "info: ",
    "linker version: ",
];

/// Structural-banner-shape marker set for [`is_banner_shape`] rule
/// (C.2). `s` matches (C.2) only when it satisfies the shared
/// preconditions, has uppercase first byte, is 40+ bytes long, has a
/// space, AND contains one of these markers (case-sensitive).
///
/// (C.2) admits CI-runner deprecation banners, proc-macro / code-gen
/// deprecation banners, and build-system migration banners. The
/// uppercase-first-byte requirement carves the (C.2) alternative out
/// from rustc's lowercase-first-byte convention (`error`, `warning`,
/// `note`, `help`). See plan §3.3.2 (C.2) for the design rationale.
const STRUCTURAL_BANNER_MARKERS: &[&str] = &[
    "deprecated",
    "deprecation",
    "Please update",
    "actions to use",
    "EOL",
    "end-of-life",
];

/// True iff `s` is banner-shaped per `docs/spec/lihaaf-v0.2.md` §6.6
/// (planner design from `docs/spec/extra-substitutions-plan-2026-05-19.md`
/// §3.3.2):
///
/// **(A) Shared preconditions — ALL must hold.**
/// 1. `s.len() >= 20` (bytes).
/// 2. `!s.contains('\n')`.
/// 3. `s` does NOT start with whitespace (`' '` or `'\t'`).
/// 4. `s` does NOT start with `'^'`, `'='`, or `'|'`.
///
/// **(B) Anti-prefix list** — REJECT if `s` starts with any entry in
/// [`BANNER_ANTI_PREFIXES`].
///
/// **(C) Disjunction — at least ONE must hold:**
/// - (C.1) `s` starts with any entry in [`BANNER_PREFIXES`].
/// - (C.2) STRUCTURAL BANNER SHAPE — `s.len() >= 40` AND
///   `s.as_bytes()[0].is_ascii_uppercase()` AND `s.contains(' ')`
///   AND `s` contains at least one entry from
///   [`STRUCTURAL_BANNER_MARKERS`].
///
/// This predicate gates strip patterns alongside [`is_path_like`]
/// (the strip-key validator is the disjunction
/// `is_path_like || is_banner_shape`). See the plan for the
/// defense-in-depth argument: `error: cannot find type` passes (A)
/// and (B) (no matching anti-prefix), then fails (C.1) (banner
/// prefixes are anchored to specific tails) and (C.2) (lowercase
/// first byte). Nothing resembling a rustc diagnostic message body
/// can pass.
fn is_banner_shape(s: &str) -> bool {
    let bytes = s.as_bytes();
    // (A) Shared preconditions.
    if bytes.len() < 20 {
        return false;
    }
    if s.contains('\n') {
        return false;
    }
    // (A.3) leading whitespace.
    if bytes[0] == b' ' || bytes[0] == b'\t' {
        return false;
    }
    // (A.4) span-context first byte.
    if bytes[0] == b'^' || bytes[0] == b'=' || bytes[0] == b'|' {
        return false;
    }
    // (B) Anti-prefix list — REJECT if any matches.
    if BANNER_ANTI_PREFIXES.iter().any(|p| s.starts_with(*p)) {
        return false;
    }
    // (C.1) ENUMERATED BANNER PREFIX.
    if BANNER_PREFIXES.iter().any(|p| s.starts_with(*p)) {
        return true;
    }
    // (C.2) STRUCTURAL BANNER SHAPE.
    if bytes.len() >= 40
        && bytes[0].is_ascii_uppercase()
        && s.contains(' ')
        && STRUCTURAL_BANNER_MARKERS.iter().any(|m| s.contains(*m))
    {
        return true;
    }
    false
}

/// Validate every entry in `extra_substitutions`:
///
/// - `from` is non-empty and `is_path_like`.
/// - `to` is present (may be empty) and contains no newline.
///
/// Error messages name the offending index and the failing value so
/// adopters can locate the offending entry in their `Cargo.toml`.
fn validate_extra_substitutions(suite_label: &str, subs: &[Substitution]) -> Result<(), Error> {
    for (i, sub) in subs.iter().enumerate() {
        if sub.from.is_empty() {
            return Err(Error::Session(Outcome::ConfigInvalid {
                message: format!(
                    "{suite_label}.extra_substitutions[{i}].from is empty.\n\
                     Why this matters: an empty `from` would match the start of every byte and rewrite arbitrary content.\n\
                     extra_substitutions is for path-shaped substitution only. See docs/spec/lihaaf-v0.2.md §6.6."
                ),
            }));
        }
        if !is_path_like(&sub.from) {
            return Err(Error::Session(Outcome::ConfigInvalid {
                message: format!(
                    "{suite_label}.extra_substitutions[{i}].from = \"{from}\" is not path-like \
                     (must contain '/', '\\\\', or be a bare $X placeholder token, \
                     where X is an ASCII uppercase letter). \
                     Patterns starting with '$' must have an ASCII uppercase letter \
                     immediately after, regardless of path separators — '$lowercase/path' is rejected. \
                     Bare placeholder patterns are full-string anchored: '$DIR-', '$RUST.', '$A!' \
                     are rejected; '$DIR/x' is accepted via the path-separator branch. \
                     extra_substitutions is for path-shaped substitution only, \
                     not arbitrary text rewriting. See docs/spec/lihaaf-v0.2.md §6.6.",
                    from = sub.from,
                ),
            }));
        }
        if sub.to.contains('\n') {
            return Err(Error::Session(Outcome::ConfigInvalid {
                message: format!(
                    "{suite_label}.extra_substitutions[{i}].to contains a newline character; \
                     replacements must be single-line.\n\
                     Why this matters: a multi-line `to` would inject blank lines into normalized stderr and break snapshot determinism."
                ),
            }));
        }
    }
    Ok(())
}

/// Validate every entry in `strip_lines` / `strip_line_prefixes`:
/// each must pass `is_path_like(s) || is_banner_shape(s)`.
///
/// `key_label` is the literal config key name (`strip_lines` /
/// `strip_line_prefixes`) interpolated into the error message so
/// adopters know which key to edit.
fn validate_strip_patterns(
    suite_label: &str,
    key_label: &str,
    patterns: &[String],
) -> Result<(), Error> {
    for (i, pat) in patterns.iter().enumerate() {
        if pat.contains('\n') {
            return Err(Error::Session(Outcome::ConfigInvalid {
                message: format!(
                    "{suite_label}.{key_label}[{i}] contains a newline character; \
                     strip patterns must be single-line."
                ),
            }));
        }
        if !is_path_like(pat) && !is_banner_shape(pat) {
            return Err(Error::Session(Outcome::ConfigInvalid {
                message: format!(
                    "{suite_label}.{key_label}[{i}] = \"{pat}\" is neither path-shaped nor banner-shaped \
                     (must contain '/', '\\\\', start with a $X placeholder token where X is an \
                     ASCII uppercase letter, OR match the banner allowlist — see docs/spec/lihaaf-v0.2.md §6.6). \
                     Patterns starting with '$' must have an ASCII uppercase letter immediately after, \
                     regardless of path separators — '$lowercase/path' is rejected. \
                     Bare placeholder patterns are full-string anchored: '$DIR-', '$RUST.', '$A!' \
                     are rejected; '$DIR/x' is accepted via the path-separator branch. \
                     Strip patterns target path-shaped environment noise OR known banner shapes only."
                ),
            }));
        }
    }
    Ok(())
}

/// Promote raw substitution entries into validated [`Substitution`].
/// Returns a `ConfigInvalid` outcome if any entry is missing `from`
/// or `to`; otherwise passes the typed list through (validation of
/// `from` shape and `to` no-newline lives in
/// [`validate_extra_substitutions`]).
fn finalize_substitutions(
    suite_label: &str,
    raw: Vec<RawSubstitution>,
) -> Result<Vec<Substitution>, Error> {
    let mut out = Vec::with_capacity(raw.len());
    for (i, entry) in raw.into_iter().enumerate() {
        let from = entry.from.ok_or_else(|| {
            Error::Session(Outcome::ConfigInvalid {
                message: format!(
                    "{suite_label}.extra_substitutions[{i}] is missing the required `from` key.\n\
                     Why this matters: every entry must specify which substring to match."
                ),
            })
        })?;
        let to = entry.to.ok_or_else(|| {
            Error::Session(Outcome::ConfigInvalid {
                message: format!(
                    "{suite_label}.extra_substitutions[{i}] is missing the required `to` key.\n\
                     Why this matters: every entry must specify the replacement string \
                     (use an empty string `to = \"\"` to strip the match)."
                ),
            })
        })?;
        out.push(Substitution { from, to });
    }
    Ok(out)
}

/// Validate the `dylib_crate` value after the empty-string check.
///
/// Currently rejects interior NUL bytes. The crate name is forwarded as a
/// `-p` argv token to cargo; an interior NUL byte cannot appear in a POSIX
/// argv token and spawn would reject it, surfacing as WORKER_CRASHED instead
/// of an actionable CONFIG_INVALID.
///
/// Factored out so it can be unit-tested directly — the TOML decoder already
/// rejects NUL bytes before `parse` sees them, but programmatic construction
/// paths (e.g., integration tests that build `Config` by hand) can still
/// reach this validator.
fn validate_dylib_crate(dylib_crate: &str) -> Result<(), Error> {
    if dylib_crate.contains('\0') {
        return Err(Error::Session(Outcome::ConfigInvalid {
            message: "[package.metadata.lihaaf].dylib_crate contains a NUL byte.\n\
                 Why this matters: the crate name is forwarded as a `-p` argv token to cargo; \
                 an interior NUL byte cannot appear in a POSIX argv token and spawn would reject \
                 it, surfacing as WORKER_CRASHED instead of an actionable CONFIG_INVALID."
                .to_string(),
        }));
    }
    Ok(())
}

fn validate_unique_suite_names(suites: &[Suite]) -> Result<(), Error> {
    let mut seen: BTreeSet<&str> = BTreeSet::new();
    for s in suites {
        if !seen.insert(s.name.as_str()) {
            return Err(Error::Session(Outcome::ConfigInvalid {
                message: format!(
                    "duplicate suite name \"{}\".\nWhy this matters: suite names are how the CLI selects which suite to run.",
                    s.name
                ),
            }));
        }
    }
    Ok(())
}

fn validate_disjoint_fixture_dirs(manifest_path: &Path, suites: &[Suite]) -> Result<(), Error> {
    // O(N²) in the number of (suite, dir) pairs; acceptable because
    // suites are small (single-digit) and fixture_dirs per suite are
    // also small. Compare lexical keys resolved against the manifest
    // root, not raw strings: `tests/foo`, `./tests/foo`, and
    // `/crate/tests/foo` all point at the same snapshot siblings and
    // must not be accepted in different suites.
    let crate_root = derive_manifest_root(manifest_path);
    let mut seen: Vec<(&str, PathBuf, PathBuf)> = Vec::new();
    for suite in suites {
        for dir in &suite.fixture_dirs {
            let key = fixture_dir_key(&crate_root, dir);
            for (other_suite, other_dir, other_key) in &seen {
                if *other_key == key {
                    return Err(Error::Session(Outcome::ConfigInvalid {
                        message: format!(
                            "fixture_dirs path \"{}\" in suite \"{}\" resolves to the same directory as \"{}\" in suite \"{other_suite}\".\nWhy this matters: snapshot files (.stderr) live next to the .rs fixtures; two suites sharing a directory would write conflicting snapshots.",
                            dir.display(),
                            suite.name,
                            other_dir.display()
                        ),
                    }));
                }
            }
            seen.push((suite.name.as_str(), dir.clone(), key));
        }
    }
    Ok(())
}

fn derive_manifest_root(manifest_path: &Path) -> PathBuf {
    match manifest_path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.to_path_buf(),
        _ => PathBuf::from("."),
    }
}

fn fixture_dir_key(crate_root: &Path, dir: &Path) -> PathBuf {
    let joined = if dir.is_absolute() {
        dir.to_path_buf()
    } else {
        crate_root.join(dir)
    };
    lexical_normalize(&joined)
}

fn lexical_normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    out.push(component.as_os_str());
                }
            }
            Component::Normal(_) | Component::RootDir | Component::Prefix(_) => {
                out.push(component.as_os_str());
            }
        }
    }
    if out.as_os_str().is_empty() {
        PathBuf::from(".")
    } else {
        out
    }
}

fn format_invalid_key(key: &str, expected: &str, why: &str) -> String {
    format!("[package.metadata.lihaaf].{key} must be {expected}.\nWhy this matters: {why}.")
}

fn missing_metadata_message() -> String {
    "lihaaf needs `[package.metadata.lihaaf]` to know what to build.\n\
       Add the table to your Cargo.toml. See the lihaaf README for the\n\
       minimum required keys."
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_str(toml_text: &str) -> Result<Config, Error> {
        parse(toml_text, Path::new("Cargo.toml"))
    }

    fn unwrap_invalid(err: Error) -> String {
        match err {
            Error::Session(Outcome::ConfigInvalid { message }) => message,
            other => panic!("expected ConfigInvalid, got {other:?}"),
        }
    }

    /// Parse `toml`, assert it produces a `ConfigInvalid` outcome, and
    /// assert the rendered error message contains every entry in
    /// `expected_substrings`. Used by the cluster of negative-path tests
    /// below that all assert "this invalid TOML produces an error message
    /// naming these specific identifiers".
    fn assert_parse_rejects_with(toml: &str, expected_substrings: &[&str]) {
        let err = parse_str(toml).unwrap_err();
        let msg = unwrap_invalid(err);
        for expected in expected_substrings {
            assert!(
                msg.contains(expected),
                "error message `{msg}` did not contain expected substring `{expected}`",
            );
        }
    }

    #[test]
    fn missing_table_is_session_outcome_with_exact_message() {
        assert_parse_rejects_with(
            r#"
            [package]
            name = "x"
            version = "0.1.0"
        "#,
            &["`[package.metadata.lihaaf]`", "minimum required keys"],
        );
    }

    #[test]
    fn missing_dylib_crate_is_invalid() {
        assert_parse_rejects_with(
            r#"
            [package.metadata.lihaaf]
            extern_crates = ["foo"]
        "#,
            &["dylib_crate"],
        );
    }

    #[test]
    fn extern_crates_first_must_equal_dylib() {
        assert_parse_rejects_with(
            r#"
            [package.metadata.lihaaf]
            dylib_crate = "consumer"
            extern_crates = ["other"]
        "#,
            &["extern_crates[0]"],
        );
    }

    #[test]
    fn defaults_apply_to_optional_keys_and_yield_one_default_suite() {
        let cfg = parse_str(
            r#"
            [package.metadata.lihaaf]
            dylib_crate = "consumer"
            extern_crates = ["consumer"]
        "#,
        )
        .unwrap();
        assert_eq!(cfg.dylib_crate, "consumer");
        assert_eq!(cfg.suites.len(), 1);
        let s = &cfg.suites[0];
        assert!(s.is_default());
        assert_eq!(s.name, DEFAULT_SUITE_NAME);
        assert_eq!(s.edition, "2021");
        assert_eq!(s.compile_fail_marker, "compile_fail");
        assert_eq!(s.fixture_timeout_secs, 90);
        assert_eq!(s.per_fixture_memory_mb, 1024);
        assert_eq!(s.fixture_dirs.len(), 2);
        assert!(s.features.is_empty());
        assert!(s.dev_deps.is_empty());
    }

    #[test]
    fn edition_must_be_in_allowed_set() {
        assert_parse_rejects_with(
            r#"
            [package.metadata.lihaaf]
            dylib_crate = "consumer"
            extern_crates = ["consumer"]
            edition = "2026"
        "#,
            &["edition", "2024"],
        );
    }

    #[test]
    fn zero_timeout_is_invalid() {
        assert_parse_rejects_with(
            r#"
            [package.metadata.lihaaf]
            dylib_crate = "consumer"
            extern_crates = ["consumer"]
            fixture_timeout_secs = 0
        "#,
            &["fixture_timeout_secs"],
        );
    }

    #[test]
    fn zero_memory_ceiling_is_invalid() {
        assert_parse_rejects_with(
            r#"
            [package.metadata.lihaaf]
            dylib_crate = "consumer"
            extern_crates = ["consumer"]
            per_fixture_memory_mb = 0
        "#,
            &["per_fixture_memory_mb"],
        );
    }

    #[test]
    fn raw_metadata_is_preserved_verbatim() {
        let cfg = parse_str(
            r#"
            [package.metadata.lihaaf]
            dylib_crate = "consumer"
            extern_crates = ["consumer", "consumer-macros"]
            features = ["testing"]
            dev_deps = ["serde", "serde_json"]
        "#,
        )
        .unwrap();
        // The raw metadata is what the manifest will snapshot. It must
        // include every key the user typed, even those also mapped into
        // typed fields above.
        let table = cfg.raw_metadata.as_table().unwrap();
        assert!(table.contains_key("dylib_crate"));
        assert!(table.contains_key("extern_crates"));
        assert!(table.contains_key("features"));
        assert!(table.contains_key("dev_deps"));
    }

    // ---- Multi-suite parsing ----

    #[test]
    fn named_suite_inherits_unspecified_keys_from_default() {
        let cfg = parse_str(
            r#"
            [package.metadata.lihaaf]
            dylib_crate = "consumer"
            extern_crates = ["consumer", "consumer-macros"]
            edition = "2024"
            dev_deps = ["serde"]
            compile_fail_marker = "compile_fail"
            fixture_timeout_secs = 120
            per_fixture_memory_mb = 2048
            allow_lints = ["dead_code"]

            [[package.metadata.lihaaf.suite]]
            name = "spatial"
            features = ["spatial"]
            fixture_dirs = ["tests/lihaaf/compile_pass_spatial"]
        "#,
        )
        .unwrap();
        assert_eq!(cfg.suites.len(), 2);
        let spatial = &cfg.suites[1];
        assert_eq!(spatial.name, "spatial");
        assert_eq!(spatial.features, vec!["spatial".to_string()]);
        assert_eq!(spatial.edition, "2024");
        assert_eq!(spatial.dev_deps, vec!["serde".to_string()]);
        assert_eq!(spatial.compile_fail_marker, "compile_fail");
        assert_eq!(spatial.fixture_timeout_secs, 120);
        assert_eq!(spatial.per_fixture_memory_mb, 2048);
        assert_eq!(
            spatial.extern_crates,
            vec!["consumer".to_string(), "consumer-macros".to_string()]
        );
        // allow_lints inherits from the default suite when omitted on a
        // named suite — same precedent as dev_deps (config.rs:461-463).
        assert_eq!(spatial.allow_lints, vec!["dead_code".to_string()]);
    }

    #[test]
    fn named_suite_features_do_not_inherit_from_default() {
        // Explicit-replacement rule: a named suite that omits `features`
        // gets `[]`, not the default suite's features. Documented in
        // `Suite::features` rustdoc.
        let cfg = parse_str(
            r#"
            [package.metadata.lihaaf]
            dylib_crate = "consumer"
            extern_crates = ["consumer"]
            features = ["testing"]

            [[package.metadata.lihaaf.suite]]
            name = "isolated"
            fixture_dirs = ["tests/lihaaf/iso"]
        "#,
        )
        .unwrap();
        assert_eq!(cfg.suites[0].features, vec!["testing".to_string()]);
        assert!(cfg.suites[1].features.is_empty());
    }

    #[test]
    fn config_keep_foreign_span_bodies_defaults_false() {
        let cfg = parse_str(
            r#"
            [package.metadata.lihaaf]
            dylib_crate = "consumer"
            extern_crates = ["consumer"]
        "#,
        )
        .unwrap();
        assert!(!cfg.suites[0].keep_foreign_span_bodies);
    }

    #[test]
    fn config_keep_foreign_span_bodies_parses_true() {
        let cfg = parse_str(
            r#"
            [package.metadata.lihaaf]
            dylib_crate = "consumer"
            extern_crates = ["consumer"]
            keep_foreign_span_bodies = true
        "#,
        )
        .unwrap();
        assert!(cfg.suites[0].keep_foreign_span_bodies);
    }

    #[test]
    fn config_keep_foreign_span_bodies_does_not_inherit_on_named_suite() {
        let cfg = parse_str(
            r#"
            [package.metadata.lihaaf]
            dylib_crate = "consumer"
            extern_crates = ["consumer"]
            keep_foreign_span_bodies = true

            [[package.metadata.lihaaf.suite]]
            name = "spatial"
            fixture_dirs = ["tests/lihaaf/spatial"]
        "#,
        )
        .unwrap();
        assert!(cfg.suites[0].keep_foreign_span_bodies);
        assert!(
            !cfg.suites[1].keep_foreign_span_bodies,
            "named suite must NOT inherit; REPLACE scope resolves to false",
        );
    }

    #[test]
    fn config_keep_foreign_span_bodies_non_boolean_is_invalid() {
        assert_parse_rejects_with(
            r#"
            [package.metadata.lihaaf]
            dylib_crate = "consumer"
            extern_crates = ["consumer"]
            keep_foreign_span_bodies = "yes"
        "#,
            &[],
        );
    }

    #[test]
    fn serde_deserialize_suite_defaults_keep_foreign_span_bodies_false() {
        let v = serde_json::json!({
            "dylib_crate": "consumer",
            "raw_metadata": {},
            "suites": [{
                "name": "default",
                "extern_crates": ["consumer"],
                "fixture_dirs": ["tests/lihaaf/compile_fail"],
                "features": [],
                "edition": "2021",
                "dev_deps": [],
                "compile_fail_marker": "compile_fail",
                "fixture_timeout_secs": 90,
                "per_fixture_memory_mb": 1024,
                "allow_lints": [],
                "extra_substitutions": [],
                "strip_lines": [],
                "strip_line_prefixes": []
            }]
        });
        let cfg: Config =
            serde_json::from_value(v).expect("Suite must deserialize with the key omitted");
        assert!(!cfg.suites[0].keep_foreign_span_bodies);
    }

    #[test]
    fn serde_deserialize_suite_rejects_non_boolean_keep_foreign_span_bodies() {
        let v = serde_json::json!({
            "dylib_crate": "consumer",
            "raw_metadata": {},
            "suites": [{
                "name": "default",
                "extern_crates": ["consumer"],
                "fixture_dirs": ["tests/lihaaf/compile_fail"],
                "features": [],
                "edition": "2021",
                "dev_deps": [],
                "compile_fail_marker": "compile_fail",
                "fixture_timeout_secs": 90,
                "per_fixture_memory_mb": 1024,
                "allow_lints": [],
                "extra_substitutions": [],
                "strip_lines": [],
                "strip_line_prefixes": [],
                "keep_foreign_span_bodies": "yes"
            }]
        });
        let result: Result<Config, _> = serde_json::from_value(v);
        assert!(
            result.is_err(),
            "present non-boolean must fail typed decode"
        );
    }

    #[test]
    fn named_suite_can_override_features() {
        let cfg = parse_str(
            r#"
            [package.metadata.lihaaf]
            dylib_crate = "consumer"
            extern_crates = ["consumer"]
            features = ["testing"]

            [[package.metadata.lihaaf.suite]]
            name = "spatial"
            features = ["spatial"]
            fixture_dirs = ["tests/lihaaf/compile_pass_spatial"]
        "#,
        )
        .unwrap();
        assert_eq!(cfg.suites[1].features, vec!["spatial".to_string()]);
    }

    #[test]
    fn named_suite_dylib_crate_is_rejected() {
        assert_parse_rejects_with(
            r#"
            [package.metadata.lihaaf]
            dylib_crate = "consumer"
            extern_crates = ["consumer"]

            [[package.metadata.lihaaf.suite]]
            name = "spatial"
            dylib_crate = "other"
            fixture_dirs = ["tests/lihaaf/spatial"]
        "#,
            &["dylib_crate", "not a per-suite key"],
        );
    }

    #[test]
    fn named_suite_default_is_reserved() {
        assert_parse_rejects_with(
            r#"
            [package.metadata.lihaaf]
            dylib_crate = "consumer"
            extern_crates = ["consumer"]

            [[package.metadata.lihaaf.suite]]
            name = "default"
            fixture_dirs = ["tests/lihaaf/default_extra"]
        "#,
            &["\"default\"", "reserved"],
        );
    }

    #[test]
    fn named_suite_missing_name_is_rejected() {
        assert_parse_rejects_with(
            r#"
            [package.metadata.lihaaf]
            dylib_crate = "consumer"
            extern_crates = ["consumer"]

            [[package.metadata.lihaaf.suite]]
            fixture_dirs = ["tests/lihaaf/x"]
        "#,
            &["entry #0", "name"],
        );
    }

    #[test]
    fn named_suite_invalid_chars_in_name_rejected() {
        assert_parse_rejects_with(
            r#"
            [package.metadata.lihaaf]
            dylib_crate = "consumer"
            extern_crates = ["consumer"]

            [[package.metadata.lihaaf.suite]]
            name = "with space"
            fixture_dirs = ["tests/lihaaf/space"]
        "#,
            &["ASCII alphanumeric"],
        );
    }

    #[test]
    fn named_suite_missing_fixture_dirs_is_rejected() {
        assert_parse_rejects_with(
            r#"
            [package.metadata.lihaaf]
            dylib_crate = "consumer"
            extern_crates = ["consumer"]

            [[package.metadata.lihaaf.suite]]
            name = "spatial"
            features = ["spatial"]
        "#,
            &["fixture_dirs", "required"],
        );
    }

    #[test]
    fn named_suite_empty_fixture_dirs_is_rejected() {
        assert_parse_rejects_with(
            r#"
            [package.metadata.lihaaf]
            dylib_crate = "consumer"
            extern_crates = ["consumer"]

            [[package.metadata.lihaaf.suite]]
            name = "spatial"
            fixture_dirs = []
        "#,
            &["empty array"],
        );
    }

    #[test]
    fn duplicate_suite_names_rejected() {
        assert_parse_rejects_with(
            r#"
            [package.metadata.lihaaf]
            dylib_crate = "consumer"
            extern_crates = ["consumer"]

            [[package.metadata.lihaaf.suite]]
            name = "spatial"
            fixture_dirs = ["tests/lihaaf/a"]

            [[package.metadata.lihaaf.suite]]
            name = "spatial"
            fixture_dirs = ["tests/lihaaf/b"]
        "#,
            &["duplicate suite name", "\"spatial\""],
        );
    }

    #[test]
    fn fixture_dirs_must_be_disjoint_across_suites() {
        assert_parse_rejects_with(
            r#"
            [package.metadata.lihaaf]
            dylib_crate = "consumer"
            extern_crates = ["consumer"]
            fixture_dirs = ["tests/lihaaf/shared"]

            [[package.metadata.lihaaf.suite]]
            name = "spatial"
            fixture_dirs = ["tests/lihaaf/shared"]
        "#,
            &["shared", "default", "spatial"],
        );
    }

    #[test]
    fn fixture_dirs_must_be_disjoint_after_dot_normalization() {
        assert_parse_rejects_with(
            r#"
            [package.metadata.lihaaf]
            dylib_crate = "consumer"
            extern_crates = ["consumer"]
            fixture_dirs = ["tests/lihaaf/shared"]

            [[package.metadata.lihaaf.suite]]
            name = "spatial"
            fixture_dirs = ["./tests/lihaaf/shared"]
        "#,
            &["resolves to the same directory", "default", "spatial"],
        );
    }

    #[test]
    fn fixture_dirs_must_be_disjoint_after_absolute_resolution() {
        let root = std::env::current_dir().unwrap();
        let abs = root.join("tests/lihaaf/shared");
        let toml = format!(
            r#"
            [package.metadata.lihaaf]
            dylib_crate = "consumer"
            extern_crates = ["consumer"]
            fixture_dirs = ["tests/lihaaf/shared"]

            [[package.metadata.lihaaf.suite]]
            name = "spatial"
            fixture_dirs = ['{}']
        "#,
            abs.display()
        );
        let err = parse(&toml, &root.join("Cargo.toml")).unwrap_err();
        let msg = unwrap_invalid(err);
        assert!(msg.contains("resolves to the same directory"));
        assert!(msg.contains("default"));
        assert!(msg.contains("spatial"));
    }

    #[test]
    fn fixture_dirs_must_be_disjoint_between_two_named_suites() {
        assert_parse_rejects_with(
            r#"
            [package.metadata.lihaaf]
            dylib_crate = "consumer"
            extern_crates = ["consumer"]

            [[package.metadata.lihaaf.suite]]
            name = "alpha"
            fixture_dirs = ["tests/lihaaf/x"]

            [[package.metadata.lihaaf.suite]]
            name = "beta"
            fixture_dirs = ["tests/lihaaf/x"]
        "#,
            &["alpha", "beta"],
        );
    }

    #[test]
    fn declared_suite_order_is_preserved() {
        let cfg = parse_str(
            r#"
            [package.metadata.lihaaf]
            dylib_crate = "consumer"
            extern_crates = ["consumer"]

            [[package.metadata.lihaaf.suite]]
            name = "second"
            fixture_dirs = ["tests/lihaaf/b"]

            [[package.metadata.lihaaf.suite]]
            name = "first"
            fixture_dirs = ["tests/lihaaf/a"]
        "#,
        )
        .unwrap();
        assert_eq!(cfg.suites[0].name, DEFAULT_SUITE_NAME);
        assert_eq!(cfg.suites[1].name, "second");
        assert_eq!(cfg.suites[2].name, "first");
    }

    // ---- allow_lints tests ----

    #[test]
    fn allow_lints_default_is_empty() {
        let cfg = parse_str(
            r#"
            [package.metadata.lihaaf]
            dylib_crate = "consumer"
            extern_crates = ["consumer"]
        "#,
        )
        .unwrap();
        assert!(
            cfg.suites[0].allow_lints.is_empty(),
            "allow_lints must default to an empty vec when the key is absent"
        );
    }

    #[test]
    fn allow_lints_accepts_simple_lint_names() {
        let cfg = parse_str(
            r#"
            [package.metadata.lihaaf]
            dylib_crate = "consumer"
            extern_crates = ["consumer"]
            allow_lints = ["unexpected_cfgs", "dead_code"]
        "#,
        )
        .unwrap();
        assert_eq!(
            cfg.suites[0].allow_lints,
            vec!["unexpected_cfgs".to_string(), "dead_code".to_string()]
        );
    }

    #[test]
    fn allow_lints_accepts_clippy_namespaced_lints() {
        // Confirms `::` is not rejected by structural validation — namespaced
        // lints are passed verbatim to rustc which accepts them when the
        // relevant tool is registered.
        let cfg = parse_str(
            r#"
            [package.metadata.lihaaf]
            dylib_crate = "consumer"
            extern_crates = ["consumer"]
            allow_lints = ["clippy::needless_collect"]
        "#,
        )
        .unwrap();
        assert_eq!(
            cfg.suites[0].allow_lints,
            vec!["clippy::needless_collect".to_string()]
        );
    }

    #[test]
    fn allow_lints_rejects_empty_string() {
        assert_parse_rejects_with(
            r#"
            [package.metadata.lihaaf]
            dylib_crate = "consumer"
            extern_crates = ["consumer"]
            allow_lints = [""]
        "#,
            &["allow_lints", "empty string"],
        );
    }

    #[test]
    fn allow_lints_rejects_leading_dash() {
        assert_parse_rejects_with(
            r#"
            [package.metadata.lihaaf]
            dylib_crate = "consumer"
            extern_crates = ["consumer"]
            allow_lints = ["-A unexpected_cfgs"]
        "#,
            &["allow_lints", "starts with `-`"],
        );
    }

    #[test]
    fn allow_lints_rejects_whitespace() {
        assert_parse_rejects_with(
            r#"
            [package.metadata.lihaaf]
            dylib_crate = "consumer"
            extern_crates = ["consumer"]
            allow_lints = ["dead code"]
        "#,
            &["allow_lints", "whitespace"],
        );
    }

    #[test]
    fn allow_lints_rejects_quote_and_backslash() {
        // Three parametric assertions: double-quote, single-quote, backslash.
        //
        // TOML encoding notes:
        //   - `'a"b'` is a TOML raw (single-quoted) string containing a
        //     literal double-quote → Rust string "a\"b".
        //   - `"a'b"` is a TOML basic (double-quoted) string containing a
        //     literal single-quote → Rust string "a'b".
        //   - `'a\b'` is a TOML raw string (no TOML escapes) containing a
        //     literal backslash → Rust string "a\\b".
        for (toml_value, bad_label) in &[
            (r#"'a"b'"#, r#"a"b"#), // double-quote via TOML raw string
            (r#""a'b""#, "a'b"),    // single-quote via TOML basic string
            (r#"'a\b'"#, r"a\b"),   // backslash via TOML raw string
        ] {
            let toml = format!(
                r#"
                [package.metadata.lihaaf]
                dylib_crate = "consumer"
                extern_crates = ["consumer"]
                allow_lints = [{toml_value}]
                "#,
            );
            let err = parse_str(&toml).unwrap_err();
            let msg = unwrap_invalid(err);
            assert!(
                msg.contains("allow_lints"),
                "error for entry `{bad_label}` did not mention `allow_lints`: {msg}"
            );
            assert!(
                msg.contains("whitespace"),
                "error for entry `{bad_label}` did not mention `whitespace`: {msg}"
            );
        }
    }

    // ---- NUL-byte / argv-safety tests ----

    #[test]
    fn allow_lints_rejects_nul_byte() {
        // TOML's basic-string spec disallows control characters including NUL,
        // so the TOML decoder would reject a NUL in a TOML literal before the
        // validator sees it. We test the validator directly here — this covers
        // any programmatic path that constructs a Vec<String> without going
        // through TOML (e.g. tests that synthesise Config by hand).
        let lints = vec![format!("bad{}lint", '\u{0}')];
        let err = validate_allow_lints("default", &lints).unwrap_err();
        let msg = unwrap_invalid(err);
        assert!(
            msg.contains("allow_lints"),
            "error did not mention `allow_lints`: {msg}"
        );
        assert!(msg.contains("NUL"), "error did not mention `NUL`: {msg}");
    }

    #[test]
    fn features_rejects_nul_byte() {
        // Same TOML-bypass rationale as allow_lints_rejects_nul_byte above.
        let features = vec![format!("bad{}feat", '\u{0}')];
        let err = validate_features("default", &features).unwrap_err();
        let msg = unwrap_invalid(err);
        assert!(
            msg.contains("features"),
            "error did not mention `features`: {msg}"
        );
        assert!(msg.contains("NUL"), "error did not mention `NUL`: {msg}");
    }

    #[test]
    fn features_rejects_empty_string() {
        assert_parse_rejects_with(
            r#"
            [package.metadata.lihaaf]
            dylib_crate = "consumer"
            extern_crates = ["consumer"]
            features = [""]
        "#,
            &["features", "empty string"],
        );
    }

    #[test]
    fn dylib_crate_rejects_nul_byte() {
        // Same TOML-bypass rationale as allow_lints_rejects_nul_byte above.
        let name = format!("con{}sumer", '\u{0}');
        let err = validate_dylib_crate(&name).unwrap_err();
        let msg = unwrap_invalid(err);
        assert!(
            msg.contains("dylib_crate"),
            "error did not mention `dylib_crate`: {msg}"
        );
        assert!(msg.contains("NUL"), "error did not mention `NUL`: {msg}");
    }

    #[test]
    fn allow_lints_named_suite_overrides_default() {
        let cfg = parse_str(
            r#"
            [package.metadata.lihaaf]
            dylib_crate = "consumer"
            extern_crates = ["consumer"]
            allow_lints = ["dead_code"]

            [[package.metadata.lihaaf.suite]]
            name = "extra"
            fixture_dirs = ["tests/lihaaf/extra"]
            allow_lints = ["unused"]
        "#,
        )
        .unwrap();
        // Named suite explicitly sets allow_lints: replacement, not merge.
        assert_eq!(
            cfg.suites[1].allow_lints,
            vec!["unused".to_string()],
            "named suite allow_lints must replace the default, not merge"
        );
        // Default suite value is unchanged.
        assert_eq!(cfg.suites[0].allow_lints, vec!["dead_code".to_string()]);
    }

    #[test]
    fn allow_lints_named_suite_empty_array_overrides_to_empty() {
        // An adopter who sets allow_lints = [] on a named suite must get [],
        // not the default suite's lints — this is the per-suite opt-out path.
        let cfg = parse_str(
            r#"
            [package.metadata.lihaaf]
            dylib_crate = "consumer"
            extern_crates = ["consumer"]
            allow_lints = ["dead_code"]

            [[package.metadata.lihaaf.suite]]
            name = "strict"
            fixture_dirs = ["tests/lihaaf/strict"]
            allow_lints = []
        "#,
        )
        .unwrap();
        assert!(
            cfg.suites[1].allow_lints.is_empty(),
            "explicit empty allow_lints on named suite must override to empty"
        );
    }

    #[test]
    fn raw_metadata_preserves_allow_lints() {
        let cfg = parse_str(
            r#"
            [package.metadata.lihaaf]
            dylib_crate = "consumer"
            extern_crates = ["consumer"]
            allow_lints = ["unused_imports", "dead_code"]
        "#,
        )
        .unwrap();
        let table = cfg.raw_metadata.as_table().unwrap();
        assert!(
            table.contains_key("allow_lints"),
            "raw_metadata must preserve the allow_lints key verbatim for the manifest snapshot"
        );
        let lints = table["allow_lints"].as_array().unwrap();
        let lint_strs: Vec<&str> = lints.iter().filter_map(|v| v.as_str()).collect();
        assert_eq!(lint_strs, vec!["unused_imports", "dead_code"]);
    }

    // ====================================================================
    // `extra_substitutions`, `strip_lines`, `strip_line_prefixes` tests
    //
    // Per plan §7.2 / §7.3 (`docs/spec/extra-substitutions-plan-2026-05-19.md`).
    // ====================================================================

    // ---- §7.2 Config parse + per-suite REPLACE semantics ----

    #[test]
    fn build_targets_default_and_named_omission_parse_as_empty() {
        let cfg = parse_str(
            r#"
            [package.metadata.lihaaf]
            dylib_crate = "consumer"
            extern_crates = ["consumer"]

            [[package.metadata.lihaaf.suite]]
            name = "named"
            fixture_dirs = ["tests/lihaaf/named"]
        "#,
        )
        .unwrap();

        assert!(cfg.suites[0].build_targets.is_empty());
        assert!(cfg.suites[1].build_targets.is_empty());
    }

    #[test]
    fn build_targets_uses_replacement_semantics_while_dev_deps_inherit() {
        let cfg = parse_str(
            r#"
            [package.metadata.lihaaf]
            dylib_crate = "consumer"
            extern_crates = ["consumer"]
            dev_deps = ["serde"]
            build_targets = ["tests"]

            [[package.metadata.lihaaf.suite]]
            name = "named"
            fixture_dirs = ["tests/lihaaf/named"]
        "#,
        )
        .unwrap();

        assert_eq!(cfg.suites[0].build_targets.as_slice(), ["tests"]);
        assert!(cfg.suites[1].build_targets.is_empty());
        assert_eq!(cfg.suites[1].dev_deps, vec!["serde"]);
    }

    #[test]
    fn build_targets_validation_runs_after_dev_dep_inheritance() {
        let cfg = parse_str(
            r#"
            [package.metadata.lihaaf]
            dylib_crate = "consumer"
            extern_crates = ["consumer"]
            dev_deps = ["serde"]

            [[package.metadata.lihaaf.suite]]
            name = "named"
            fixture_dirs = ["tests/lihaaf/named"]
            build_targets = ["tests"]
        "#,
        )
        .unwrap();

        assert_eq!(cfg.suites[1].build_targets.as_slice(), ["tests"]);
        assert_eq!(cfg.suites[1].dev_deps, vec!["serde"]);
    }

    #[test]
    fn build_targets_reject_empty_dev_deps_unknown_and_duplicates() {
        let empty_deps = parse_str(
            r#"
            [package.metadata.lihaaf]
            dylib_crate = "consumer"
            extern_crates = ["consumer"]
            build_targets = ["tests"]
        "#,
        )
        .unwrap_err();
        assert!(unwrap_invalid(empty_deps).contains("dev_deps"));

        let unknown = parse_str(
            r#"
            [package.metadata.lihaaf]
            dylib_crate = "consumer"
            extern_crates = ["consumer"]
            dev_deps = ["serde"]
            build_targets = ["benches"]
        "#,
        )
        .unwrap_err();
        assert!(unwrap_invalid(unknown).contains("tests"));

        let duplicate = parse_str(
            r#"
            [package.metadata.lihaaf]
            dylib_crate = "consumer"
            extern_crates = ["consumer"]
            dev_deps = ["serde"]
            build_targets = ["tests", "tests"]
        "#,
        )
        .unwrap_err();
        assert!(unwrap_invalid(duplicate).contains("duplicate"));
    }

    #[test]
    fn extra_substitutions_per_suite_replace_not_merge() {
        // Named suite overrides extra_substitutions: REPLACE, not merge.
        // Matches the `features` precedent. The default suite's entry
        // does NOT leak into the named suite's vector.
        let cfg = parse_str(
            r#"
            [package.metadata.lihaaf]
            dylib_crate = "consumer"
            extern_crates = ["consumer"]
            extra_substitutions = [
                { from = "/default/path", to = "$WORKSPACE/d" },
            ]

            [[package.metadata.lihaaf.suite]]
            name = "extra"
            fixture_dirs = ["tests/lihaaf/extra"]
            extra_substitutions = [
                { from = "/named/path", to = "$WORKSPACE/n" },
            ]
        "#,
        )
        .unwrap();
        // Default suite has the default entry.
        assert_eq!(cfg.suites[0].extra_substitutions.len(), 1);
        assert_eq!(cfg.suites[0].extra_substitutions[0].from, "/default/path");
        // Named suite REPLACED, did not merge.
        assert_eq!(cfg.suites[1].extra_substitutions.len(), 1);
        assert_eq!(cfg.suites[1].extra_substitutions[0].from, "/named/path");
    }

    #[test]
    fn extra_substitutions_omitted_on_named_suite_is_empty() {
        // Per-suite REPLACE semantics: omission on a named suite gives
        // `[]`, NOT the default suite's value. Mirrors `features` precedent.
        let cfg = parse_str(
            r#"
            [package.metadata.lihaaf]
            dylib_crate = "consumer"
            extern_crates = ["consumer"]
            extra_substitutions = [
                { from = "/default/path", to = "$WORKSPACE/d" },
            ]

            [[package.metadata.lihaaf.suite]]
            name = "isolated"
            fixture_dirs = ["tests/lihaaf/iso"]
        "#,
        )
        .unwrap();
        assert_eq!(cfg.suites[0].extra_substitutions.len(), 1);
        assert!(
            cfg.suites[1].extra_substitutions.is_empty(),
            "named suite that omits extra_substitutions must get [], not inherit",
        );
    }

    #[test]
    fn extra_substitutions_manifest_snapshot_round_trips() {
        // raw_metadata round-trip: the manifest snapshot must preserve
        // the adopter-typed shape verbatim so manifest-snapshot freshness
        // tracking sees adopter overrides as part of the watched state.
        let cfg = parse_str(
            r#"
            [package.metadata.lihaaf]
            dylib_crate = "consumer"
            extern_crates = ["consumer"]
            extra_substitutions = [
                { from = "/nix/store/abc", to = "$RUST/lib" },
            ]
            strip_lines = ["error: aborting due to 1 previous error"]
            strip_line_prefixes = ["For more information about this error"]
        "#,
        )
        .unwrap();
        let table = cfg.raw_metadata.as_table().unwrap();
        assert!(
            table.contains_key("extra_substitutions"),
            "raw_metadata must preserve extra_substitutions verbatim",
        );
        assert!(table.contains_key("strip_lines"));
        assert!(table.contains_key("strip_line_prefixes"));
    }

    #[test]
    fn strip_patterns_per_suite_replace_not_merge() {
        let cfg = parse_str(
            r#"
            [package.metadata.lihaaf]
            dylib_crate = "consumer"
            extern_crates = ["consumer"]
            strip_lines = ["/default/strip"]
            strip_line_prefixes = ["/default/prefix/"]

            [[package.metadata.lihaaf.suite]]
            name = "extra"
            fixture_dirs = ["tests/lihaaf/extra"]
            strip_lines = ["/named/strip"]
            strip_line_prefixes = ["/named/prefix/"]
        "#,
        )
        .unwrap();
        assert_eq!(
            cfg.suites[0]
                .strip_lines
                .iter()
                .map(|p| p.as_str())
                .collect::<Vec<_>>(),
            vec!["/default/strip"]
        );
        assert_eq!(
            cfg.suites[0]
                .strip_line_prefixes
                .iter()
                .map(|p| p.as_str())
                .collect::<Vec<_>>(),
            vec!["/default/prefix/"]
        );
        assert_eq!(
            cfg.suites[1]
                .strip_lines
                .iter()
                .map(|p| p.as_str())
                .collect::<Vec<_>>(),
            vec!["/named/strip"]
        );
        assert_eq!(
            cfg.suites[1]
                .strip_line_prefixes
                .iter()
                .map(|p| p.as_str())
                .collect::<Vec<_>>(),
            vec!["/named/prefix/"]
        );
    }

    #[test]
    fn strip_patterns_omitted_on_named_suite_is_empty() {
        let cfg = parse_str(
            r#"
            [package.metadata.lihaaf]
            dylib_crate = "consumer"
            extern_crates = ["consumer"]
            strip_lines = ["/default/strip"]
            strip_line_prefixes = ["/default/prefix/"]

            [[package.metadata.lihaaf.suite]]
            name = "isolated"
            fixture_dirs = ["tests/lihaaf/iso"]
        "#,
        )
        .unwrap();
        assert!(cfg.suites[1].strip_lines.is_empty());
        assert!(cfg.suites[1].strip_line_prefixes.is_empty());
    }

    // ---- §7.3.1 `is_path_like` predicate-level tests ----
    //
    // Acceptance classes 1-9 + rejection classes A-K from plan §7.3.1.

    #[test]
    fn is_path_like_accepts_absolute_unix_path() {
        // Class 1.
        assert!(is_path_like("/nix/store/abc123"));
        assert!(is_path_like("/build/sandbox"));
    }

    #[test]
    fn is_path_like_accepts_absolute_windows_path() {
        // Class 2.
        assert!(is_path_like(r"C:\Users\runner\.cargo"));
        assert!(is_path_like(r"D:\build\target"));
    }

    #[test]
    fn is_path_like_accepts_relative_path_with_separator() {
        // Class 3.
        assert!(is_path_like("target/release"));
        assert!(is_path_like(r"src\compat"));
    }

    #[test]
    fn is_path_like_accepts_path_segment() {
        // Class 4.
        assert!(is_path_like("nix/store"));
        assert!(is_path_like("vendor/cargo-cache"));
    }

    #[test]
    fn is_path_like_accepts_builtin_placeholder_bare() {
        // Class 5 — every built-in lihaaf placeholder.
        for placeholder in &[
            "$DIR",
            "$WORKSPACE",
            "$RUST",
            "$CARGO",
            "$TYPEID",
            "$LONGTYPE_FILE",
        ] {
            assert!(
                is_path_like(placeholder),
                "expected built-in placeholder {placeholder} to pass is_path_like",
            );
        }
    }

    #[test]
    fn is_path_like_accepts_builtin_placeholder_with_suffix() {
        // Class 6.
        assert!(is_path_like("$DIR/test.rs"));
        assert!(is_path_like("$RUST/lib/rustlib"));
        assert!(is_path_like("$CARGO/registry/src"));
    }

    #[test]
    fn is_path_like_accepts_adopter_placeholder() {
        // Class 7 — adopter-introduced placeholder bare.
        assert!(is_path_like("$NIX_STORE"));
        assert!(is_path_like("$SANDBOX_ROOT"));
        assert!(is_path_like("$VENDOR_CACHE_2026"));
    }

    #[test]
    fn is_path_like_accepts_adopter_placeholder_with_suffix() {
        // Class 8.
        assert!(is_path_like("$NIX_STORE/rust"));
        assert!(is_path_like("$SANDBOX_ROOT/target/release"));
    }

    #[test]
    fn is_path_like_accepts_interior_lowercase_dollar_within_path() {
        // Class 9. Interior `$lowercase` within paths is path text, not a
        // placeholder reference. Rule 3 only fires on LEADING `$`; these
        // strings pass via rule 4(a) because they contain `/`.
        assert!(is_path_like("/path/$nix/sub"));
        assert!(is_path_like("/some/$cache/dir"));
        assert!(is_path_like("$WORKSPACE/$tmp/run"));
    }

    #[test]
    fn is_path_like_rejects_diagnostic_text_plain() {
        // Class A.
        for s in &["error", "warning", "help", "note", "error:", "E0277", ":"] {
            assert!(
                !is_path_like(s),
                "is_path_like must reject diagnostic-text plain pattern {s:?}",
            );
        }
    }

    #[test]
    fn is_path_like_rejects_round2_block1_surface() {
        // Class B.
        for s in &[
            "  |",
            "For more information about this error",
            "expected due to this",
        ] {
            assert!(!is_path_like(s), "Class-B surface {s:?} must be rejected");
        }
    }

    #[test]
    fn is_path_like_rejects_round2_block2_surface() {
        // Class C.
        for s in &["error[", "warning[", "error: aborting due to"] {
            assert!(!is_path_like(s), "Class-C surface {s:?} must be rejected");
        }
    }

    #[test]
    fn is_path_like_rejects_length_one_separator() {
        // Class D.
        assert!(!is_path_like("/"));
        assert!(!is_path_like(r"\"));
    }

    #[test]
    fn is_path_like_rejects_length_one_dollar() {
        // Class E.
        assert!(!is_path_like("$"));
    }

    #[test]
    fn is_path_like_rejects_bare_dollar_patterns() {
        // Class F — `$` not followed by `[A-Z]`.
        for s in &["$ ", "$1", "$lowercase", "$ NAME", "$_"] {
            assert!(
                !is_path_like(s),
                "bare `$` pattern {s:?} must be rejected by leading-`$` guard",
            );
        }
    }

    #[test]
    fn is_path_like_rejects_lowercase_dollar_with_separator() {
        // Class F' — leading-`$` guard (rule 3) catches `$lowercase/path`
        // shapes that would otherwise pass via the `/` disjunction branch.
        // Each sample must be rejected.
        for s in &[
            "$nix/path",
            "$lowercase/anything",
            "$_path/x",
            "$1/path",
            "$ /space-then-slash",
            r"$nix\path",
        ] {
            assert!(
                !is_path_like(s),
                "lowercase `$` + separator {s:?} must be rejected by rule-3 leading-`$` guard",
            );
        }
    }

    #[test]
    fn is_path_like_rejects_placeholder_with_trailing_junk() {
        // Class F'' — rule (4c) is full-string anchored (`^\$[A-Z][A-Za-z0-9_]*$`);
        // trailing chars outside `[A-Za-z0-9_]` make (4c) fail. None
        // of (4a)/(4b) match because no `/` or `\` is present.
        for s in &["$DIR-", "$A!", "$RUST.", "$DIR ", "$WORKSPACE,"] {
            assert!(
                !is_path_like(s),
                "placeholder with trailing junk {s:?} must be rejected: \
                 rule (4c) is full-string anchored",
            );
        }
        // Companion assertion: `$DIR/x` passes overall (via rule 4(a))
        // but the (4c)-isolation helper rejects it — proving (4c) is
        // the "complete placeholder token" alternative, not a prefix
        // of path content.
        assert!(is_path_like("$DIR/x"));
        assert!(
            !is_complete_placeholder_token("$DIR/x"),
            "(4c) alone must reject `$DIR/x`; that pattern passes is_path_like via rule 4(a)",
        );
    }

    #[test]
    fn is_path_like_rejects_empty_and_whitespace() {
        // Classes G, H.
        assert!(!is_path_like(""));
        assert!(!is_path_like(" "));
        assert!(!is_path_like("\t\t"));
    }

    #[test]
    fn is_path_like_rejects_embedded_markers_no_path() {
        // Class I.
        assert!(!is_path_like("errored"));
        assert!(!is_path_like("aborted"));
    }

    #[test]
    fn is_path_like_rejects_markers_with_trailing_colon() {
        // Class J.
        assert!(!is_path_like("error:"));
        assert!(!is_path_like("note:"));
        assert!(!is_path_like("help:"));
    }

    #[test]
    fn is_path_like_rejects_newline_bearing() {
        // Class K.
        assert!(!is_path_like("a\nb"));
        assert!(!is_path_like("$DIR\n"));
    }

    // ---- §7.3.2 `is_banner_shape` predicate-level tests ----

    #[test]
    fn is_banner_shape_accepts_explain_footer() {
        // Class α — multiple error codes + alternate phrasing variants
        // that all start with the anchored prefix.
        assert!(is_banner_shape(
            "For more information about this error, try `rustc --explain E0277`.",
        ));
        assert!(is_banner_shape(
            "For more information about this error, try `rustc --explain E0001`.",
        ));
        assert!(is_banner_shape(
            "For more information about this error, try `rustc --explain E9999`.",
        ));
        // `--explain`-less variant — still passes via prefix-anchor
        // (the prefix is shorter than the explain-clause).
        assert!(is_banner_shape(
            "For more information about this error in the documentation.",
        ));
    }

    #[test]
    fn is_banner_shape_accepts_macro_origin_trailer() {
        // Class β — full + shorter variants.
        assert!(is_banner_shape(
            "note: this error originates from the macro `m` in the crate `c` \
             (in Nightly builds, run with -Z macro-backtrace for more info)",
        ));
        assert!(is_banner_shape(
            "note: this error originates from the attribute macro `derive_more::Display`",
        ));
    }

    #[test]
    fn is_banner_shape_accepts_error_count_summary() {
        // Class γ.
        assert!(is_banner_shape("error: aborting due to 1 previous error"));
        assert!(is_banner_shape("error: aborting due to 42 previous errors"));
        assert!(is_banner_shape(
            "error: aborting due to 1 previous error; 2 warnings emitted",
        ));
    }

    #[test]
    fn is_banner_shape_accepts_vendored_toolchain_info() {
        // Class δ.
        assert!(is_banner_shape(
            "info: using rustc from /opt/vendored/rust-1.95.0/bin/rustc",
        ));
        assert!(is_banner_shape(
            "info: switching to nightly toolchain to satisfy unstable feature",
        ));
    }

    #[test]
    fn is_banner_shape_accepts_linker_version() {
        // Class ε — long linker version forms only. The shorter
        // `linker: lld-15.0.7` form is rejected; see banner-rejection
        // class α'.
        assert!(is_banner_shape(
            "linker version: GNU ld (GNU Binutils) 2.40",
        ));
        assert!(is_banner_shape("linker version: rust-lld 15.0.7"));
    }

    #[test]
    fn is_banner_shape_accepts_structural_banner_shape() {
        // Class ζ — covers CI-runner deprecation banners AND non-CI
        // structural banner subfamilies (proc-macro / code-gen
        // deprecation banners, build-system migration banners).
        //
        // CI-runner deprecation variants.
        assert!(is_banner_shape(
            "Node.js 16 actions are deprecated. Please update the following actions to use \
             Node.js 20: actions/checkout@v3",
        ));
        assert!(is_banner_shape(
            "GitHub Actions deprecation: please migrate to Node.js 20 by end-of-life date",
        ));
        // Non-CI structural banner variants — demonstrate that (C.2)
        // admits more than CI-runner banners.
        assert!(is_banner_shape(
            "Deprecated generator output: Please update the generated API before release",
        ));
    }

    #[test]
    fn is_banner_shape_rejects_length_floor() {
        // Class α' — regression guard, banner surface. Includes
        // `linker: lld-15.0.7` (18 bytes, below the 20-byte floor).
        for s in &["error", "note", ": ", "linker: lld-15.0.7"] {
            assert!(
                !is_banner_shape(s),
                "banner-shape length-floor rejection: {s:?} (len={})",
                s.len(),
            );
        }
    }

    #[test]
    fn is_banner_shape_rejects_whitespace_leading() {
        // Class β' — regression guard, banner surface.
        for s in &["  |", "   ^^^^^", "\t= note: ..."] {
            assert!(
                !is_banner_shape(s),
                "whitespace-leading {s:?} must be rejected by (A.3)",
            );
        }
    }

    #[test]
    fn is_banner_shape_rejects_span_context_first_byte() {
        // Class γ' — regression guard, banner surface.
        // Span context whose first byte is `^`, `=`, or `|`.
        for s in &[
            "^^^^^^^^^^^^^^^^^^^^^",            // 21 chars to clear (A.1)
            "= note: some content here for it", // long enough
            "| trait bound goes in here too",
        ] {
            assert!(
                !is_banner_shape(s),
                "span-context-first-byte {s:?} must be rejected by (A.4)",
            );
        }
    }

    #[test]
    fn is_banner_shape_rejects_diagnostic_body_anti_prefix() {
        // Class δ' — anti-prefix list (B) catches the diagnostic-body
        // shape family.
        for s in &[
            "expected one of: cascade, set null",
            "found type `u32` but wanted f64",
            "the trait bound `X: Y` is not satisfied",
            "the type `Foo` cannot be sent across threads",
            "cannot find function `foo` in scope",
            "mismatched types when expected was right",
            "consider importing this trait for the call site",
            "help: try adding `as_ref` for the call site",
        ] {
            assert!(
                !is_banner_shape(s),
                "anti-prefix diagnostic body {s:?} must be rejected by (B)",
            );
        }
    }

    #[test]
    fn is_banner_shape_rejects_diagnostic_keyword_with_colon() {
        // Class ε' — critical carve-out test. `warning: use of
        // deprecated function` MUST be rejected even though it
        // contains `deprecated`: the lowercase-leading `warning:`
        // anti-prefix fires before the (C.2) marker check.
        assert!(
            !is_banner_shape("warning: use of deprecated function `f`"),
            "`warning: ...` MUST be rejected even though it contains `deprecated`",
        );
        // Other variants in this class.
        assert!(!is_banner_shape(
            "error[E0277]: trait not implemented for this type",
        ));
        // Length-floor failure first; either path is fine.
        assert!(!is_banner_shape("error[E0277]"));
    }

    #[test]
    fn is_banner_shape_rejects_lowercase_leading_non_rustc() {
        // Class ζ'.
        assert!(!is_banner_shape("gitlab ci deprecation banner here"));
    }

    #[test]
    fn is_banner_shape_rejects_uppercase_leading_short() {
        // Class η' — uppercase first byte but below (A.1) and/or (C.2)
        // length floors.
        assert!(!is_banner_shape("Node 16 deprecated"));
    }

    #[test]
    fn is_banner_shape_rejects_diagnostic_looking_like_banner() {
        // Class θ' — defense-in-depth test. The string passes (A) and
        // (B) (no matching anti-prefix), then fails (C.1) (banner
        // prefixes are anchored to specific tails) and (C.2)
        // (lowercase first byte).
        assert!(
            !is_banner_shape("error: cannot find type `Foo` in scope"),
            "diagnostic-looking-like-banner must be rejected (defense-in-depth)",
        );
    }

    // ---- §7.3.3 Field-level wiring tests ----
    //
    // The strip-key validator is the disjunction
    // `is_path_like || is_banner_shape`. These tests exercise that
    // disjunction through the config-parse layer end-to-end.

    /// Build a minimum-viable TOML with one entry on
    /// `extra_substitutions.from` so we can isolate-test the validator
    /// without triggering unrelated errors.
    ///
    /// `from` is interpolated into a TOML LITERAL string (single
    /// quotes) so backslash-bearing patterns (Windows paths like
    /// `C:\Users\runner\.cargo`) survive without TOML-level
    /// escape-sequence reinterpretation. Adopters writing such
    /// patterns in real `Cargo.toml` use the same mechanism.
    fn extra_subs_toml(from: &str) -> String {
        format!(
            r#"
            [package.metadata.lihaaf]
            dylib_crate = "consumer"
            extern_crates = ["consumer"]
            extra_substitutions = [
                {{ from = '{from}', to = "$WORKSPACE/x" }},
            ]
        "#,
        )
    }

    #[test]
    fn validate_extra_substitutions_rejects_non_path_from() {
        // Rejection classes A/B/C/E/F/F'/F''/H/I/J on `from`.
        let bad: Vec<&str> = vec![
            // A. Diagnostic-text plain.
            "error",
            // B. Class-B surface.
            "expected due to this",
            // C. Class-C surface.
            "error: aborting due to",
            // E. Length-1 placeholder-start.
            "$",
            // F. Bare `$` patterns.
            "$lowercase",
            // F'. Lowercase `$` + separator.
            "$nix/path",
            // F''. Placeholder with trailing junk.
            "$DIR-",
            "$RUST.",
            "$WORKSPACE,",
            // H. Whitespace-only.
            " ",
            // I. Embedded marker no path.
            "errored",
            // J. Markers + colon.
            "error:",
        ];
        for from in &bad {
            let toml = extra_subs_toml(from);
            let err = parse_str(&toml).unwrap_err();
            let msg = unwrap_invalid(err);
            assert!(
                msg.contains("not path-like")
                    || msg.contains("is_path_like")
                    || msg.contains("extra_substitutions"),
                "rejection of {from:?} should mention validator: {msg}",
            );
        }
    }

    #[test]
    fn validate_extra_substitutions_accepts_path_from() {
        // Acceptance classes 1-9 on `from`, including Class 9 (interior
        // `$lowercase` within paths).
        let good: Vec<&str> = vec![
            // 1. Absolute Unix path.
            "/nix/store/abc123",
            // 2. Absolute Windows path.
            r"C:\Users\runner\.cargo",
            // 3. Relative path with separator.
            "target/release",
            // 4. Path segment.
            "nix/store",
            // 5. Built-in placeholder bare.
            "$DIR",
            "$WORKSPACE",
            "$RUST",
            // 6. Placeholder + path suffix.
            "$DIR/test.rs",
            // 7. Adopter-defined placeholder.
            "$NIX_STORE",
            // 8. Adopter placeholder + suffix.
            "$NIX_STORE/rust",
            // 9. Interior `$lowercase` within paths.
            "/path/$nix/sub",
            "/some/$cache/dir",
            "$WORKSPACE/$tmp/run",
        ];
        for from in &good {
            let toml = extra_subs_toml(from);
            let cfg = parse_str(&toml).unwrap_or_else(|e| {
                panic!("expected {from:?} to pass is_path_like, but got error: {e:?}");
            });
            // Round-trip: validated config should contain the entry.
            assert_eq!(cfg.suites[0].extra_substitutions.len(), 1);
            assert_eq!(cfg.suites[0].extra_substitutions[0].from, *from);
        }
    }

    #[test]
    fn validate_extra_substitutions_rejects_newline_in_to() {
        // `to` field gets the no-newline guard. Synthesize the bytes
        // via TOML's escape rules — newline as `\n` in a basic string.
        let toml = r#"
            [package.metadata.lihaaf]
            dylib_crate = "consumer"
            extern_crates = ["consumer"]
            extra_substitutions = [
                { from = "/nix/store", to = "alpha\nbeta" },
            ]
        "#;
        let err = parse_str(toml).unwrap_err();
        let msg = unwrap_invalid(err);
        assert!(
            msg.contains("newline") || msg.contains("single-line"),
            "newline-in-to error should mention newline: {msg}",
        );
    }

    #[test]
    fn validate_extra_substitutions_accepts_compound_to() {
        // `to` is unconstrained beyond no-newline.
        // `to = ""`, `to = "$RUST"`, `to = "$RUST/lib/rustlib"` all pass.
        for to_value in &["", "$RUST", "$RUST/lib/rustlib"] {
            let toml = format!(
                r#"
                [package.metadata.lihaaf]
                dylib_crate = "consumer"
                extern_crates = ["consumer"]
                extra_substitutions = [
                    {{ from = "/nix/store", to = "{to_value}" }},
                ]
            "#,
            );
            let cfg = parse_str(&toml).unwrap_or_else(|e| {
                panic!("expected to={to_value:?} to pass, got: {e:?}");
            });
            assert_eq!(cfg.suites[0].extra_substitutions[0].to, *to_value);
        }
    }

    /// Build a TOML with one entry on `strip_lines` for isolation.
    /// Interpolates `line` into a TOML literal-string so Windows
    /// paths and other backslash-bearing patterns survive verbatim.
    fn strip_lines_toml(line: &str) -> String {
        format!(
            r#"
            [package.metadata.lihaaf]
            dylib_crate = "consumer"
            extern_crates = ["consumer"]
            strip_lines = ['{line}']
        "#,
        )
    }

    /// Build a TOML with one entry on `strip_line_prefixes` for
    /// isolation. Same literal-string rationale as
    /// [`strip_lines_toml`].
    fn strip_prefixes_toml(prefix: &str) -> String {
        format!(
            r#"
            [package.metadata.lihaaf]
            dylib_crate = "consumer"
            extern_crates = ["consumer"]
            strip_line_prefixes = ['{prefix}']
        "#,
        )
    }

    #[test]
    fn validate_strip_patterns_accepts_path() {
        // Classes 1-9 on both strip keys (path-shaped acceptance
        // through `is_path_like`).
        let good: Vec<&str> = vec![
            "/nix/store/abc123",
            r"C:\Users\runner\.cargo",
            "target/release",
            "nix/store",
            "$DIR",
            "$DIR/test.rs",
            "$NIX_STORE",
            "$NIX_STORE/rust",
            // Class 9 — interior `$lowercase` within paths.
            "/path/$nix/sub",
        ];
        for pat in &good {
            let toml_strip = strip_lines_toml(pat);
            parse_str(&toml_strip).unwrap_or_else(|e| {
                panic!("strip_lines accept {pat:?}: {e:?}");
            });
            let toml_prefix = strip_prefixes_toml(pat);
            parse_str(&toml_prefix).unwrap_or_else(|e| {
                panic!("strip_line_prefixes accept {pat:?}: {e:?}");
            });
        }
    }

    #[test]
    fn validate_strip_patterns_accepts_banner() {
        // Classes α-ζ on both strip keys (banner-shaped acceptance
        // through `is_banner_shape`).
        let good: Vec<&str> = vec![
            "For more information about this error, try x",
            "note: this error originates from the macro `m` here",
            "error: aborting due to 1 previous error",
            "info: using rustc from /opt/vendored/rust-1.95.0/bin/rustc",
            "linker version: GNU ld (GNU Binutils) 2.40",
            // ζ — structural banner (CI deprecation variant).
            "Node.js 16 actions are deprecated. Please update the actions",
        ];
        for pat in &good {
            let toml_strip = strip_lines_toml(pat);
            parse_str(&toml_strip).unwrap_or_else(|e| {
                panic!("strip_lines accept banner {pat:?}: {e:?}");
            });
            let toml_prefix = strip_prefixes_toml(pat);
            parse_str(&toml_prefix).unwrap_or_else(|e| {
                panic!("strip_line_prefixes accept banner {pat:?}: {e:?}");
            });
        }
    }

    #[test]
    fn validate_strip_patterns_rejects_span_context() {
        // Classes β', γ' on both strip keys.
        let bad: Vec<&str> = vec![
            "  |",
            "   ^^^^^",
            "^^^^^^^^^^^^^^^^^^^^^",
            "= note: some content here for it",
        ];
        for pat in &bad {
            let toml_strip = strip_lines_toml(pat);
            assert!(
                parse_str(&toml_strip).is_err(),
                "strip_lines must reject span-context {pat:?}",
            );
            let toml_prefix = strip_prefixes_toml(pat);
            assert!(
                parse_str(&toml_prefix).is_err(),
                "strip_line_prefixes must reject span-context {pat:?}",
            );
        }
    }

    #[test]
    fn validate_strip_patterns_rejects_diagnostic_keywords() {
        // Classes A, C, ε' on both strip keys.
        let bad: Vec<&str> = vec![
            "error",
            "warning",
            "note",
            "error[",
            "error: aborting due to",
            "error[E0277]: trait not implemented for this type",
        ];
        for pat in &bad {
            let toml_strip = strip_lines_toml(pat);
            assert!(
                parse_str(&toml_strip).is_err(),
                "strip_lines must reject diagnostic keyword {pat:?}",
            );
            let toml_prefix = strip_prefixes_toml(pat);
            assert!(
                parse_str(&toml_prefix).is_err(),
                "strip_line_prefixes must reject diagnostic keyword {pat:?}",
            );
        }
    }

    #[test]
    fn validate_strip_patterns_rejects_diagnostic_body() {
        // Class δ' on both strip keys.
        let bad: Vec<&str> = vec![
            "the trait bound `X: Y` is not satisfied",
            "cannot find function `foo` in scope",
            "mismatched types when comparing two structs",
            "consider importing this trait for the call site",
        ];
        for pat in &bad {
            let toml_strip = strip_lines_toml(pat);
            assert!(
                parse_str(&toml_strip).is_err(),
                "strip_lines must reject diagnostic body {pat:?}",
            );
            let toml_prefix = strip_prefixes_toml(pat);
            assert!(
                parse_str(&toml_prefix).is_err(),
                "strip_line_prefixes must reject diagnostic body {pat:?}",
            );
        }
    }

    #[test]
    fn validate_strip_patterns_rejects_disguised_diagnostic() {
        // Class θ' on both strip keys. Defense-in-depth: the string
        // passes (A) and (B) of the banner predicate, then fails the
        // disjunction (C.1 anchored prefix; C.2 lowercase first byte).
        // ALSO fails is_path_like (no separator, no `$X`).
        let pat = "error: cannot find type `Foo` in scope";
        let toml_strip = strip_lines_toml(pat);
        assert!(
            parse_str(&toml_strip).is_err(),
            "disguised diagnostic must be rejected by strip_lines (defense-in-depth)",
        );
        let toml_prefix = strip_prefixes_toml(pat);
        assert!(
            parse_str(&toml_prefix).is_err(),
            "disguised diagnostic must be rejected by strip_line_prefixes",
        );
    }

    #[test]
    fn validate_strip_patterns_rejects_short_and_dollar() {
        // Classes D, E, F, F', F'', G, H on both strip keys.
        let bad: Vec<&str> = vec![
            // D. Length-1 separator.
            "/",
            r"\",
            // E. Length-1 dollar.
            "$",
            // F. Bare dollar patterns.
            "$lowercase",
            "$1",
            // F'. Lowercase $ + separator.
            "$nix/path",
            "$lowercase/anything",
            // F''. Placeholder with trailing junk.
            "$DIR-",
            "$WORKSPACE,",
            // G. Empty handled at TOML level — skip.
            // H. Whitespace-only.
            " ",
        ];
        for pat in &bad {
            let toml_strip = strip_lines_toml(pat);
            assert!(
                parse_str(&toml_strip).is_err(),
                "strip_lines must reject {pat:?}",
            );
            let toml_prefix = strip_prefixes_toml(pat);
            assert!(
                parse_str(&toml_prefix).is_err(),
                "strip_line_prefixes must reject {pat:?}",
            );
        }
    }

    #[test]
    fn validate_strip_patterns_rejects_newline_bearing() {
        // Class K on both strip keys.
        let toml = r#"
            [package.metadata.lihaaf]
            dylib_crate = "consumer"
            extern_crates = ["consumer"]
            strip_lines = ["alpha\nbeta"]
        "#;
        let err = parse_str(toml).unwrap_err();
        let msg = unwrap_invalid(err);
        assert!(
            msg.contains("newline") || msg.contains("single-line"),
            "newline in strip_lines must surface as newline error: {msg}",
        );
        let toml2 = r#"
            [package.metadata.lihaaf]
            dylib_crate = "consumer"
            extern_crates = ["consumer"]
            strip_line_prefixes = ["alpha\nbeta"]
        "#;
        let err2 = parse_str(toml2).unwrap_err();
        let msg2 = unwrap_invalid(err2);
        assert!(
            msg2.contains("newline") || msg2.contains("single-line"),
            "newline in strip_line_prefixes must surface as newline error: {msg2}",
        );
    }

    // ---- §7.3.4 Composition + interaction tests ----

    #[test]
    fn extra_substitutions_collision_with_builtin_placeholder() {
        // Adopter `{ from = "$DIR", to = "$NOT_DIR" }`. Pins
        // composition order + non-reservation (OQ-3): built-in `$DIR`
        // fires first, then the adopter rule rewrites the resulting
        // `$DIR` literal to `$NOT_DIR`.
        let cfg = parse_str(
            r#"
            [package.metadata.lihaaf]
            dylib_crate = "consumer"
            extern_crates = ["consumer"]
            extra_substitutions = [
                { from = "$DIR", to = "$NOT_DIR" },
            ]
        "#,
        )
        .unwrap();
        assert_eq!(cfg.suites[0].extra_substitutions[0].from, "$DIR");
        assert_eq!(cfg.suites[0].extra_substitutions[0].to, "$NOT_DIR");
    }

    #[test]
    fn extra_substitutions_per_suite_override_interaction() {
        // Default suite 2 entries, named suite 1 different entry;
        // fixtures see only the 1 (REPLACE, OQ-1).
        let cfg = parse_str(
            r#"
            [package.metadata.lihaaf]
            dylib_crate = "consumer"
            extern_crates = ["consumer"]
            extra_substitutions = [
                { from = "/default/a", to = "$WORKSPACE/a" },
                { from = "/default/b", to = "$WORKSPACE/b" },
            ]

            [[package.metadata.lihaaf.suite]]
            name = "named"
            fixture_dirs = ["tests/lihaaf/named"]
            extra_substitutions = [
                { from = "/named/x", to = "$WORKSPACE/x" },
            ]
        "#,
        )
        .unwrap();
        assert_eq!(cfg.suites[0].extra_substitutions.len(), 2);
        assert_eq!(cfg.suites[1].extra_substitutions.len(), 1);
        assert_eq!(cfg.suites[1].extra_substitutions[0].from, "/named/x");
    }

    #[test]
    fn strip_patterns_per_suite_override_interaction() {
        // Per-suite REPLACE for strip keys, with one path-shaped + one
        // banner-shaped strip per suite.
        let cfg = parse_str(
            r#"
            [package.metadata.lihaaf]
            dylib_crate = "consumer"
            extern_crates = ["consumer"]
            strip_lines = [
                "/default/path",
                "error: aborting due to 1 previous error",
            ]

            [[package.metadata.lihaaf.suite]]
            name = "named"
            fixture_dirs = ["tests/lihaaf/named"]
            strip_lines = [
                "/named/path",
            ]
        "#,
        )
        .unwrap();
        assert_eq!(cfg.suites[0].strip_lines.len(), 2);
        assert_eq!(cfg.suites[1].strip_lines.len(), 1);
        assert_eq!(cfg.suites[1].strip_lines[0].as_str(), "/named/path");
    }

    // ── Serde Deserialize bypass regression tests ─────────────────────────
    //
    // These tests prove that the serde Deserialize path for Config/Suite/
    // Substitution/StripPattern routes through the same validation guards
    // as the TOML parse path. Without the `try_from` attribute, an adopter
    // could call `serde_json::from_value::<Config>(…)` and obtain a Config
    // with invalid state (non-path-like `from`, newline in `to`, invalid
    // strip patterns) without triggering any validation error.

    /// Helper that builds the minimal JSON value for a Config with a single
    /// default suite carrying `extra_substitutions = [{ from, to }]`.
    fn serde_json_config_with_sub(from: &str, to: &str) -> serde_json::Value {
        serde_json::json!({
            "dylib_crate": "consumer",
            "raw_metadata": {},
            "suites": [{
                "name": "default",
                "extern_crates": ["consumer"],
                "fixture_dirs": ["tests/lihaaf/compile_fail"],
                "features": [],
                "edition": "2021",
                "dev_deps": [],
                "compile_fail_marker": "compile_fail",
                "fixture_timeout_secs": 90,
                "per_fixture_memory_mb": 1024,
                "allow_lints": [],
                "extra_substitutions": [{ "from": from, "to": to }],
                "strip_lines": [],
                "strip_line_prefixes": []
            }]
        })
    }

    /// Helper that builds the minimal JSON value for a Config with
    /// `strip_lines = [pattern]` in the default suite.
    fn serde_json_config_with_strip_lines(pattern: &str) -> serde_json::Value {
        serde_json::json!({
            "dylib_crate": "consumer",
            "raw_metadata": {},
            "suites": [{
                "name": "default",
                "extern_crates": ["consumer"],
                "fixture_dirs": ["tests/lihaaf/compile_fail"],
                "features": [],
                "edition": "2021",
                "dev_deps": [],
                "compile_fail_marker": "compile_fail",
                "fixture_timeout_secs": 90,
                "per_fixture_memory_mb": 1024,
                "allow_lints": [],
                "extra_substitutions": [],
                "strip_lines": [pattern],
                "strip_line_prefixes": []
            }]
        })
    }

    /// Helper that builds the minimal JSON value for a Config with
    /// `strip_line_prefixes = [pattern]` in the default suite.
    fn serde_json_config_with_strip_line_prefixes(pattern: &str) -> serde_json::Value {
        serde_json::json!({
            "dylib_crate": "consumer",
            "raw_metadata": {},
            "suites": [{
                "name": "default",
                "extern_crates": ["consumer"],
                "fixture_dirs": ["tests/lihaaf/compile_fail"],
                "features": [],
                "edition": "2021",
                "dev_deps": [],
                "compile_fail_marker": "compile_fail",
                "fixture_timeout_secs": 90,
                "per_fixture_memory_mb": 1024,
                "allow_lints": [],
                "extra_substitutions": [],
                "strip_lines": [],
                "strip_line_prefixes": [pattern]
            }]
        })
    }

    fn serde_json_config_with_build_targets(
        dev_deps: Vec<&str>,
        build_targets: Vec<&str>,
    ) -> serde_json::Value {
        serde_json::json!({
            "dylib_crate": "consumer",
            "raw_metadata": {},
            "suites": [{
                "name": "default",
                "extern_crates": ["consumer"],
                "fixture_dirs": ["tests/lihaaf/compile_fail"],
                "features": [],
                "edition": "2021",
                "dev_deps": dev_deps,
                "build_targets": build_targets,
                "compile_fail_marker": "compile_fail",
                "fixture_timeout_secs": 90,
                "per_fixture_memory_mb": 1024,
                "allow_lints": [],
                "extra_substitutions": [],
                "strip_lines": [],
                "strip_line_prefixes": []
            }]
        })
    }

    #[test]
    fn serde_deserialize_rejects_invalid_substitution_from() {
        // serde_json::from_value::<Config> with a non-path-like `from`
        // must fail, proving the `try_from` serde bypass guard is active.
        let bad_from_values = [
            "error",
            "error:",
            "expected due to this",
            "  |",
            "warning: x",
        ];
        for bad_from in &bad_from_values {
            let v = serde_json_config_with_sub(bad_from, "x");
            let result = serde_json::from_value::<Config>(v);
            assert!(
                result.is_err(),
                "serde_json::from_value should reject from={bad_from:?} \
                 but succeeded",
            );
            let msg = result.unwrap_err().to_string();
            assert!(
                msg.contains("not path-like")
                    || msg.contains("path-shaped")
                    || msg.contains("is_path_like")
                    || msg.contains("extra_substitutions"),
                "error for from={bad_from:?} should mention path validation: {msg}",
            );
        }
    }

    #[test]
    fn serde_deserialize_rejects_invalid_substitution_to_newline() {
        // `to` containing a newline must be rejected via the serde path.
        let v = serde_json_config_with_sub("/valid/path", "alpha\nbeta");
        let result = serde_json::from_value::<Config>(v);
        assert!(
            result.is_err(),
            "serde_json::from_value should reject to with newline but succeeded",
        );
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("newline") || msg.contains("single-line"),
            "error for newline-in-to should mention newline: {msg}",
        );
    }

    #[test]
    fn serde_deserialize_rejects_invalid_strip_lines() {
        // A strip_lines entry that is neither path-like nor banner-shaped
        // must be rejected via the serde path.
        let bad_patterns = ["error", "  |", "expected due to this", "warning: x", "foo"];
        for bad_pat in &bad_patterns {
            let v = serde_json_config_with_strip_lines(bad_pat);
            let result = serde_json::from_value::<Config>(v);
            assert!(
                result.is_err(),
                "serde_json::from_value should reject strip_lines={bad_pat:?} \
                 but succeeded",
            );
            let msg = result.unwrap_err().to_string();
            assert!(
                msg.contains("path-shaped")
                    || msg.contains("banner-shaped")
                    || msg.contains("neither")
                    || msg.contains("not path-like"),
                "error for strip_lines={bad_pat:?} should mention validation: {msg}",
            );
        }
    }

    #[test]
    fn serde_deserialize_rejects_invalid_strip_line_prefixes() {
        // A strip_line_prefixes entry that is neither path-like nor
        // banner-shaped must be rejected via the serde path.
        let bad_patterns = ["error", "  |", "expected due to this", "mismatched types"];
        for bad_pat in &bad_patterns {
            let v = serde_json_config_with_strip_line_prefixes(bad_pat);
            let result = serde_json::from_value::<Config>(v);
            assert!(
                result.is_err(),
                "serde_json::from_value should reject strip_line_prefixes={bad_pat:?} \
                 but succeeded",
            );
            let msg = result.unwrap_err().to_string();
            assert!(
                msg.contains("path-shaped")
                    || msg.contains("banner-shaped")
                    || msg.contains("neither")
                    || msg.contains("not path-like"),
                "error for strip_line_prefixes={bad_pat:?} should mention validation: {msg}",
            );
        }
    }

    #[test]
    fn serde_deserialize_rejects_invalid_build_targets() {
        let unknown = serde_json_config_with_build_targets(vec!["serde"], vec!["benches"]);
        let result = serde_json::from_value::<Config>(unknown);
        assert!(
            result.is_err(),
            "serde_json::from_value should reject unknown build_targets but succeeded",
        );
        assert!(result.unwrap_err().to_string().contains("tests"));

        let duplicate = serde_json_config_with_build_targets(vec!["serde"], vec!["tests", "tests"]);
        let result = serde_json::from_value::<Config>(duplicate);
        assert!(
            result.is_err(),
            "serde_json::from_value should reject duplicate build_targets but succeeded",
        );
        assert!(result.unwrap_err().to_string().contains("duplicate"));
    }

    #[test]
    fn serde_deserialize_rejects_build_targets_without_dev_deps() {
        let v = serde_json_config_with_build_targets(Vec::new(), vec!["tests"]);
        let result = serde_json::from_value::<Config>(v);
        assert!(
            result.is_err(),
            "serde_json::from_value should reject build_targets with empty dev_deps but succeeded",
        );
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("build_targets"));
        assert!(msg.contains("dev_deps"));
    }

    #[test]
    fn serde_deserialize_accepts_valid_build_targets() {
        let v = serde_json_config_with_build_targets(vec!["serde"], vec!["tests"]);
        let cfg = serde_json::from_value::<Config>(v).unwrap_or_else(|e| {
            panic!("serde_json::from_value should accept valid build_targets but failed: {e}")
        });
        assert_eq!(cfg.suites[0].build_targets.as_slice(), ["tests"]);
    }

    #[test]
    fn serde_deserialize_accepts_valid_substitution() {
        // Valid path-shaped `from` and single-line `to` must deserialize
        // successfully and preserve the values.
        let v = serde_json_config_with_sub("/path/$DIR", "$RUST/lib");
        let cfg = serde_json::from_value::<Config>(v).unwrap_or_else(|e| {
            panic!("serde_json::from_value should accept valid substitution but failed: {e}")
        });
        assert_eq!(cfg.suites[0].extra_substitutions.len(), 1);
        assert_eq!(cfg.suites[0].extra_substitutions[0].from, "/path/$DIR");
        assert_eq!(cfg.suites[0].extra_substitutions[0].to, "$RUST/lib");
    }

    #[test]
    fn serde_deserialize_accepts_valid_strip_pattern() {
        // Path-shaped and banner-shaped strip patterns must deserialize
        // successfully via the serde path and preserve values.
        let path_pattern = "/build/sandbox/cc-wrapper";
        let v = serde_json_config_with_strip_lines(path_pattern);
        let cfg = serde_json::from_value::<Config>(v).unwrap_or_else(|e| {
            panic!("serde_json::from_value should accept valid strip_lines but failed: {e}")
        });
        assert_eq!(cfg.suites[0].strip_lines.len(), 1);
        assert_eq!(cfg.suites[0].strip_lines[0].as_str(), path_pattern);

        let banner_pattern = "error: aborting due to 1 previous error";
        let v2 = serde_json_config_with_strip_line_prefixes(banner_pattern);
        let cfg2 = serde_json::from_value::<Config>(v2).unwrap_or_else(|e| {
            panic!("serde_json::from_value should accept valid strip_line_prefixes but failed: {e}")
        });
        assert_eq!(cfg2.suites[0].strip_line_prefixes.len(), 1);
        assert_eq!(
            cfg2.suites[0].strip_line_prefixes[0].as_str(),
            banner_pattern
        );
    }

    /// Helper that builds the minimal JSON value for a Config whose default
    /// suite has a single `extra_substitutions` entry with only a `from`
    /// field — `to` is absent — so we can exercise the "to missing" error
    /// path in `TryFrom<RawSubstitution>`.
    fn serde_json_config_with_sub_from_only(from: &str) -> serde_json::Value {
        serde_json::json!({
            "dylib_crate": "consumer",
            "raw_metadata": {},
            "suites": [{
                "name": "default",
                "extern_crates": ["consumer"],
                "fixture_dirs": ["tests/lihaaf/compile_fail"],
                "features": [],
                "edition": "2021",
                "dev_deps": [],
                "compile_fail_marker": "compile_fail",
                "fixture_timeout_secs": 90,
                "per_fixture_memory_mb": 1024,
                "allow_lints": [],
                "extra_substitutions": [{ "from": from }],
                "strip_lines": [],
                "strip_line_prefixes": []
            }]
        })
    }

    #[test]
    fn serde_and_toml_paths_agree_on_first_error_for_multiply_invalid() {
        // Input: from = "error" (not path-like) with `to` absent.  This is
        // multiply-invalid: the `from` shape is bad AND `to` is missing.
        // After the fix the TryFrom impl checks presence of both keys before
        // running shape validation, so both paths report "to is missing" as
        // the first error rather than one reporting shape and the other
        // reporting presence.

        // TOML path — via parse_str (finalize_substitutions →
        // validate_extra_substitutions).
        let toml_err = parse_str(
            r#"
            [package.metadata.lihaaf]
            dylib_crate = "consumer"
            extern_crates = ["consumer"]
            extra_substitutions = [{ from = "error" }]
        "#,
        )
        .unwrap_err();
        let toml_msg = unwrap_invalid(toml_err);

        // serde path — via serde_json::from_value::<Config>.
        let v = serde_json_config_with_sub_from_only("error");
        let serde_result = serde_json::from_value::<Config>(v);
        assert!(
            serde_result.is_err(),
            "serde_json::from_value should reject missing `to` but succeeded",
        );
        let serde_msg = serde_result.unwrap_err().to_string();

        // Both paths must agree that the first-reported failure is the
        // missing `to` key, not the non-path-like `from`.  The exact
        // wording differs between paths, so we assert on shared substrings.
        let to_missing_markers = ["to", "missing"];
        for marker in &to_missing_markers {
            assert!(
                toml_msg.contains(marker),
                "TOML-path error should reference `{marker}` but got: {toml_msg}",
            );
            assert!(
                serde_msg.contains(marker),
                "serde-path error should reference `{marker}` but got: {serde_msg}",
            );
        }
        // Neither path should fire the is_path_like guard as the first error.
        assert!(
            !toml_msg.contains("not path-like"),
            "TOML path fired is_path_like before reporting missing `to`: {toml_msg}",
        );
        assert!(
            !serde_msg.contains("not path-like"),
            "serde path fired is_path_like before reporting missing `to`: {serde_msg}",
        );
    }
}
