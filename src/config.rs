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
//! Suite-level keys default to inheriting from the top-level table when
//! omitted, except `name` (always required on a named suite) and
//! `fixture_dirs` (required, must be disjoint from every other suite's
//! `fixture_dirs` so snapshot files cannot collide between suites).

use std::collections::BTreeSet;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Error, Outcome};

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
#[derive(Debug, Clone, Serialize, Deserialize)]
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

/// One feature-subset suite. Each suite carries an independent
/// (features, fixture_dirs, …) bundle and triggers its own dylib build.
///
/// All keys except `name` and `fixture_dirs` may inherit from the
/// top-level `[package.metadata.lihaaf]` table by being omitted from a
/// named suite; the resolved values are baked in here so downstream
/// modules (discovery, worker, dylib build) take a `&Suite` and never
/// re-resolve inheritance.
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
}

impl Suite {
    /// True for the implicit suite built from the top-level table.
    /// Used by session reporting + manifest naming to keep the legacy
    /// (single-suite) output byte-identical for adopters who never add
    /// a named suite.
    pub fn is_default(&self) -> bool {
        self.name == DEFAULT_SUITE_NAME
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
    compile_fail_marker: Option<String>,
    fixture_timeout_secs: Option<u32>,
    per_fixture_memory_mb: Option<u32>,
    allow_lints: Option<Vec<String>>,
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
    compile_fail_marker: Option<String>,
    fixture_timeout_secs: Option<u32>,
    per_fixture_memory_mb: Option<u32>,
    allow_lints: Option<Vec<String>>,
    /// `dylib_crate` is intentionally NOT a per-suite key. Reading any
    /// value here is rejected at validation time so a typo can't be
    /// silently dropped.
    dylib_crate: Option<String>,
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

    // Build the default suite from the top-level keys. The default suite
    // is always present in `Config::suites` even when no `[[suite]]`
    // entries are declared, so legacy (single-suite) adopters see no
    // behavior change.
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

    Ok(Suite {
        name: DEFAULT_SUITE_NAME.to_string(),
        extern_crates,
        fixture_dirs,
        features: raw.features.clone().unwrap_or_default(),
        edition,
        dev_deps: raw.dev_deps.clone().unwrap_or_default(),
        compile_fail_marker: raw
            .compile_fail_marker
            .clone()
            .unwrap_or_else(|| DEFAULT_COMPILE_FAIL_MARKER.to_string()),
        fixture_timeout_secs,
        per_fixture_memory_mb,
        allow_lints,
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

    Ok(Suite {
        name,
        extern_crates,
        fixture_dirs,
        // Features intentionally do NOT inherit: a "spatial only" suite
        // shouldn't accidentally pull in the default suite's `testing`
        // feature. Adopters who want shared features must list them in
        // both places.
        features: raw.features.unwrap_or_default(),
        edition,
        dev_deps: raw
            .dev_deps
            .unwrap_or_else(|| default_suite.dev_deps.clone()),
        compile_fail_marker: raw
            .compile_fail_marker
            .unwrap_or_else(|| default_suite.compile_fail_marker.clone()),
        fixture_timeout_secs,
        per_fixture_memory_mb,
        allow_lints,
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

/// Validate every entry in `allow_lints` against the structural rules:
/// no empty strings, no leading `-` (caller must not supply the `-A`
/// prefix), and no whitespace / quote / backslash characters (would
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
}
