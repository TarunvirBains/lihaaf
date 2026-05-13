//! `[package.metadata.lihaaf]` parsing + validation.
//!
//! This module is the single point where raw TOML becomes the typed [`Config`]
//! used by the rest of the harness. If you add a new key, add it here and in
//! `manifest.rs` so snapshot behavior stays aligned.
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
//! per_fixture_memory_mb). The top-level `[package.metadata.lihaaf]`
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
use std::path::{Path, PathBuf};

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
    validate_disjoint_fixture_dirs(&suites)?;

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

fn validate_disjoint_fixture_dirs(suites: &[Suite]) -> Result<(), Error> {
    // O(N²) in the number of (suite, dir) pairs; acceptable because
    // suites are small (single-digit) and fixture_dirs per suite are
    // also small. PathBuf equality is the literal-string check we want
    // — relative paths get resolved once against crate_root downstream
    // in discovery; the conflict at this layer is on the as-declared
    // path string (`tests/foo` and `./tests/foo` would both pass here
    // and then collide when resolved, which is fine — the discovery
    // layer surfaces a clear "duplicate fixture" error in that case).
    let mut seen: Vec<(&str, &Path)> = Vec::new();
    for suite in suites {
        for dir in &suite.fixture_dirs {
            for (other_suite, other_dir) in &seen {
                if *other_dir == dir.as_path() {
                    return Err(Error::Session(Outcome::ConfigInvalid {
                        message: format!(
                            "fixture_dirs path \"{}\" appears in both suite \"{other_suite}\" and suite \"{}\".\nWhy this matters: snapshot files (.stderr) live next to the .rs fixtures; two suites sharing a directory would write conflicting snapshots.",
                            dir.display(),
                            suite.name
                        ),
                    }));
                }
            }
            seen.push((suite.name.as_str(), dir.as_path()));
        }
    }
    Ok(())
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

    #[test]
    fn missing_table_is_session_outcome_with_exact_message() {
        let err = parse_str(
            r#"
            [package]
            name = "x"
            version = "0.1.0"
        "#,
        )
        .unwrap_err();
        let msg = unwrap_invalid(err);
        assert!(msg.contains("`[package.metadata.lihaaf]`"));
        assert!(msg.contains("minimum required keys"));
    }

    #[test]
    fn missing_dylib_crate_is_invalid() {
        let err = parse_str(
            r#"
            [package.metadata.lihaaf]
            extern_crates = ["foo"]
        "#,
        )
        .unwrap_err();
        let msg = unwrap_invalid(err);
        assert!(msg.contains("dylib_crate"));
    }

    #[test]
    fn extern_crates_first_must_equal_dylib() {
        let err = parse_str(
            r#"
            [package.metadata.lihaaf]
            dylib_crate = "consumer"
            extern_crates = ["other"]
        "#,
        )
        .unwrap_err();
        let msg = unwrap_invalid(err);
        assert!(msg.contains("extern_crates[0]"));
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
        let err = parse_str(
            r#"
            [package.metadata.lihaaf]
            dylib_crate = "consumer"
            extern_crates = ["consumer"]
            edition = "2026"
        "#,
        )
        .unwrap_err();
        let msg = unwrap_invalid(err);
        assert!(msg.contains("edition"));
        assert!(msg.contains("2024"));
    }

    #[test]
    fn zero_timeout_is_invalid() {
        let err = parse_str(
            r#"
            [package.metadata.lihaaf]
            dylib_crate = "consumer"
            extern_crates = ["consumer"]
            fixture_timeout_secs = 0
        "#,
        )
        .unwrap_err();
        let msg = unwrap_invalid(err);
        assert!(msg.contains("fixture_timeout_secs"));
    }

    #[test]
    fn zero_memory_ceiling_is_invalid() {
        let err = parse_str(
            r#"
            [package.metadata.lihaaf]
            dylib_crate = "consumer"
            extern_crates = ["consumer"]
            per_fixture_memory_mb = 0
        "#,
        )
        .unwrap_err();
        let msg = unwrap_invalid(err);
        assert!(msg.contains("per_fixture_memory_mb"));
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
        let err = parse_str(
            r#"
            [package.metadata.lihaaf]
            dylib_crate = "consumer"
            extern_crates = ["consumer"]

            [[package.metadata.lihaaf.suite]]
            name = "spatial"
            dylib_crate = "other"
            fixture_dirs = ["tests/lihaaf/spatial"]
        "#,
        )
        .unwrap_err();
        let msg = unwrap_invalid(err);
        assert!(msg.contains("dylib_crate"));
        assert!(msg.contains("not a per-suite key"));
    }

    #[test]
    fn named_suite_default_is_reserved() {
        let err = parse_str(
            r#"
            [package.metadata.lihaaf]
            dylib_crate = "consumer"
            extern_crates = ["consumer"]

            [[package.metadata.lihaaf.suite]]
            name = "default"
            fixture_dirs = ["tests/lihaaf/default_extra"]
        "#,
        )
        .unwrap_err();
        let msg = unwrap_invalid(err);
        assert!(msg.contains("\"default\""));
        assert!(msg.contains("reserved"));
    }

    #[test]
    fn named_suite_missing_name_is_rejected() {
        let err = parse_str(
            r#"
            [package.metadata.lihaaf]
            dylib_crate = "consumer"
            extern_crates = ["consumer"]

            [[package.metadata.lihaaf.suite]]
            fixture_dirs = ["tests/lihaaf/x"]
        "#,
        )
        .unwrap_err();
        let msg = unwrap_invalid(err);
        assert!(msg.contains("entry #0"));
        assert!(msg.contains("name"));
    }

    #[test]
    fn named_suite_invalid_chars_in_name_rejected() {
        let err = parse_str(
            r#"
            [package.metadata.lihaaf]
            dylib_crate = "consumer"
            extern_crates = ["consumer"]

            [[package.metadata.lihaaf.suite]]
            name = "with space"
            fixture_dirs = ["tests/lihaaf/space"]
        "#,
        )
        .unwrap_err();
        let msg = unwrap_invalid(err);
        assert!(msg.contains("ASCII alphanumeric"));
    }

    #[test]
    fn named_suite_missing_fixture_dirs_is_rejected() {
        let err = parse_str(
            r#"
            [package.metadata.lihaaf]
            dylib_crate = "consumer"
            extern_crates = ["consumer"]

            [[package.metadata.lihaaf.suite]]
            name = "spatial"
            features = ["spatial"]
        "#,
        )
        .unwrap_err();
        let msg = unwrap_invalid(err);
        assert!(msg.contains("fixture_dirs"));
        assert!(msg.contains("required"));
    }

    #[test]
    fn named_suite_empty_fixture_dirs_is_rejected() {
        let err = parse_str(
            r#"
            [package.metadata.lihaaf]
            dylib_crate = "consumer"
            extern_crates = ["consumer"]

            [[package.metadata.lihaaf.suite]]
            name = "spatial"
            fixture_dirs = []
        "#,
        )
        .unwrap_err();
        let msg = unwrap_invalid(err);
        assert!(msg.contains("empty array"));
    }

    #[test]
    fn duplicate_suite_names_rejected() {
        let err = parse_str(
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
        )
        .unwrap_err();
        let msg = unwrap_invalid(err);
        assert!(msg.contains("duplicate suite name"));
        assert!(msg.contains("\"spatial\""));
    }

    #[test]
    fn fixture_dirs_must_be_disjoint_across_suites() {
        let err = parse_str(
            r#"
            [package.metadata.lihaaf]
            dylib_crate = "consumer"
            extern_crates = ["consumer"]
            fixture_dirs = ["tests/lihaaf/shared"]

            [[package.metadata.lihaaf.suite]]
            name = "spatial"
            fixture_dirs = ["tests/lihaaf/shared"]
        "#,
        )
        .unwrap_err();
        let msg = unwrap_invalid(err);
        assert!(msg.contains("shared"));
        assert!(msg.contains("default"));
        assert!(msg.contains("spatial"));
    }

    #[test]
    fn fixture_dirs_must_be_disjoint_between_two_named_suites() {
        let err = parse_str(
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
        )
        .unwrap_err();
        let msg = unwrap_invalid(err);
        assert!(msg.contains("alpha"));
        assert!(msg.contains("beta"));
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
}
