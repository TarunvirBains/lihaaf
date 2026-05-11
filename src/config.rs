//! `[package.metadata.lihaaf]` parsing + validation.
//!
//! The schema is fixed by spec §3.2; the validation rules are §3.4. This
//! module is the single point at which raw TOML becomes a typed [`Config`]
//! the rest of the harness consumes. Any new key requires both a field
//! here and a corresponding update to the manifest snapshot in
//! `manifest.rs` (verbatim copy of `[package.metadata.lihaaf]` per
//! §4.4).
//!
//! ## Why only TOML, no env, no auto-discovery
//!
//! Spec §3.1: "auto-discovery hides build-graph decisions in non-obvious
//! places and produces 'works on my machine' pathologies the moment two
//! adopters have slightly different layouts." A missing
//! `[package.metadata.lihaaf]` table is a hard error with the exact
//! message shape the spec dictates.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Error, Outcome};

/// Default value for `fixture_dirs` when the adopter omits it (spec
/// §3.2). Adopters whose layouts differ override the key.
pub const DEFAULT_FIXTURE_DIRS: &[&str] =
    &["tests/lihaaf/compile_fail", "tests/lihaaf/compile_pass"];

/// Default value for `compile_fail_marker` when the adopter omits it
/// (spec §3.2).
pub const DEFAULT_COMPILE_FAIL_MARKER: &str = "compile_fail";

/// Default value for `edition` when the adopter omits it (spec §3.2).
pub const DEFAULT_EDITION: &str = "2021";

/// Default value for `fixture_timeout_secs` when the adopter omits it
/// (spec §3.2).
pub const DEFAULT_FIXTURE_TIMEOUT_SECS: u32 = 90;

/// Default value for `per_fixture_memory_mb` when the adopter omits it
/// (spec §3.2). Picked to give a generic-heavy proc-macro fixture
/// substantial headroom while still firing the OOM guard before the
/// OS OOMkiller would.
pub const DEFAULT_PER_FIXTURE_MEMORY_MB: u32 = 1024;

/// Allowed editions (spec §3.4 — "edition not in the allowed set").
pub const ALLOWED_EDITIONS: &[&str] = &["2015", "2018", "2021", "2024"];

/// Parsed and validated `[package.metadata.lihaaf]` table.
///
/// All fields are non-`Option` after validation; defaults from §3.2
/// have been substituted into the optional keys.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// REQUIRED. Workspace member crate to build as the dylib.
    pub dylib_crate: String,

    /// REQUIRED. Crate names fixtures may `use` from. One `--extern`
    /// flag is emitted per entry. `extern_crates[0]` MUST equal
    /// `dylib_crate` per §3.4.
    pub extern_crates: Vec<String>,

    /// Directories scanned for `*.rs` fixtures (non-recursive within
    /// each). Defaults from §3.2.
    pub fixture_dirs: Vec<PathBuf>,

    /// Cargo features enabled for both dylib build and per-fixture
    /// rustc invocation.
    #[serde(default)]
    pub features: Vec<String>,

    /// Edition for the per-fixture rustc invocation.
    pub edition: String,

    /// Extra crates beyond `extern_crates` that fixtures import
    /// directly (e.g., serde, serde_json). Resolved via cargo metadata.
    #[serde(default)]
    pub dev_deps: Vec<String>,

    /// Substring that classifies a fixture's enclosing directory as
    /// compile_fail.
    pub compile_fail_marker: String,

    /// Per-fixture rustc wall-clock timeout in seconds.
    pub fixture_timeout_secs: u32,

    /// Max RSS in MB any single rustc worker may consume before being
    /// killed.
    pub per_fixture_memory_mb: u32,

    /// Verbatim copy of the raw `[package.metadata.lihaaf]` table for
    /// the manifest snapshot (spec §4.4). Always populated by [`parse`];
    /// the explicit `default` keeps round-tripping through serde
    /// possible for tests that synthesize a `Config` without going
    /// through the parser.
    #[serde(default = "empty_toml_table")]
    pub raw_metadata: toml::Value,
}

fn empty_toml_table() -> toml::Value {
    toml::Value::Table(toml::map::Map::new())
}

/// The intermediate "as parsed before validation" shape. `Option` fields
/// allow defaults from §3.2 to be applied uniformly. This struct never
/// escapes [`load`].
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
}

/// Load the consumer crate's `Cargo.toml`, extract
/// `[package.metadata.lihaaf]`, and validate per spec §3.4.
///
/// `manifest_path` MUST point at the consumer crate's `Cargo.toml`
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
    let value: toml::Value = toml_text
        .parse()
        .map_err(|e: toml::de::Error| Error::TomlParse {
            path: manifest_path.to_path_buf(),
            message: e.to_string(),
        })?;

    // Walk to `package.metadata.lihaaf`. Missing at any step is the
    // spec §3.1 hard-error.
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
                        "[package.metadata.lihaaf] could not be parsed against the v0.1 schema:\n  {e}\nWhy this matters: the harness needs typed values to dispatch fixtures."
                    ),
                })
            })?;

    let dylib_crate = raw.dylib_crate.unwrap_or_default();
    if dylib_crate.is_empty() {
        return Err(Error::Session(Outcome::ConfigInvalid {
            message: format_invalid_key(
                "dylib_crate",
                "a non-empty workspace-member crate name",
                "lihaaf needs to know which crate to build as the dylib",
            ),
        }));
    }

    let extern_crates = raw.extern_crates.unwrap_or_default();
    if extern_crates.is_empty() {
        return Err(Error::Session(Outcome::ConfigInvalid {
            message: format_invalid_key(
                "extern_crates",
                "a non-empty array of crate names; the first MUST equal `dylib_crate`",
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
        .unwrap_or_else(|| DEFAULT_FIXTURE_DIRS.iter().map(|s| s.to_string()).collect())
        .into_iter()
        .map(PathBuf::from)
        .collect();

    let edition = raw.edition.unwrap_or_else(|| DEFAULT_EDITION.to_string());
    if !ALLOWED_EDITIONS.iter().any(|&e| e == edition) {
        return Err(Error::Session(Outcome::ConfigInvalid {
            message: format!(
                "edition \"{edition}\" is not in the allowed set ({}).\nWhy this matters: rustc's `--edition` accepts only those values.",
                ALLOWED_EDITIONS.join(", ")
            ),
        }));
    }

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

    // `fixture_dirs` validation deferred until session startup — at this
    // point we don't yet know the crate root the relative paths resolve
    // against. `discovery::collect` performs the existence check against
    // the resolved paths and emits a `ConfigInvalid` outcome if zero
    // exist.

    Ok(Config {
        dylib_crate,
        extern_crates,
        fixture_dirs,
        features: raw.features.unwrap_or_default(),
        edition,
        dev_deps: raw.dev_deps.unwrap_or_default(),
        compile_fail_marker: raw
            .compile_fail_marker
            .unwrap_or_else(|| DEFAULT_COMPILE_FAIL_MARKER.to_string()),
        fixture_timeout_secs,
        per_fixture_memory_mb,
        raw_metadata: raw_metadata_value,
    })
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
        match err {
            Error::Session(Outcome::ConfigInvalid { message }) => {
                assert!(message.contains("`[package.metadata.lihaaf]`"));
                assert!(message.contains("minimum required keys"));
            }
            other => panic!("expected ConfigInvalid, got {other:?}"),
        }
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
        match err {
            Error::Session(Outcome::ConfigInvalid { message }) => {
                assert!(message.contains("dylib_crate"));
            }
            other => panic!("expected ConfigInvalid, got {other:?}"),
        }
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
        match err {
            Error::Session(Outcome::ConfigInvalid { message }) => {
                assert!(message.contains("extern_crates[0]"));
            }
            other => panic!("expected ConfigInvalid, got {other:?}"),
        }
    }

    #[test]
    fn defaults_apply_to_optional_keys() {
        let cfg = parse_str(
            r#"
            [package.metadata.lihaaf]
            dylib_crate = "consumer"
            extern_crates = ["consumer"]
        "#,
        )
        .unwrap();
        assert_eq!(cfg.edition, "2021");
        assert_eq!(cfg.compile_fail_marker, "compile_fail");
        assert_eq!(cfg.fixture_timeout_secs, 90);
        assert_eq!(cfg.per_fixture_memory_mb, 1024);
        assert_eq!(cfg.fixture_dirs.len(), 2);
        assert!(cfg.features.is_empty());
        assert!(cfg.dev_deps.is_empty());
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
        match err {
            Error::Session(Outcome::ConfigInvalid { message }) => {
                assert!(message.contains("edition"));
                assert!(message.contains("2024"));
            }
            other => panic!("expected ConfigInvalid, got {other:?}"),
        }
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
        match err {
            Error::Session(Outcome::ConfigInvalid { message }) => {
                assert!(message.contains("fixture_timeout_secs"));
            }
            other => panic!("expected ConfigInvalid, got {other:?}"),
        }
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
        match err {
            Error::Session(Outcome::ConfigInvalid { message }) => {
                assert!(message.contains("per_fixture_memory_mb"));
            }
            other => panic!("expected ConfigInvalid, got {other:?}"),
        }
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
        // include every key the adopter typed, even ones we read into
        // typed fields above.
        let table = cfg.raw_metadata.as_table().unwrap();
        assert!(table.contains_key("dylib_crate"));
        assert!(table.contains_key("extern_crates"));
        assert!(table.contains_key("features"));
        assert!(table.contains_key("dev_deps"));
    }
}
