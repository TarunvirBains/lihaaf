/// Integration test for the `allow_lints` config key (GH #43).
///
/// Design rationale (plan §4c):
///
/// The fixture uses `unused_imports` — a default-on lint that fires under
/// bare rustc with no `--check-cfg`, survives alongside an E0308 type error
/// in the same compilation, and has a stable rendered string across rustc
/// versions.
///
/// The fixture contains:
///   - `use std::collections::HashMap;` — triggers `unused_imports` at the
///     name-resolution pass (before type-check)
///   - `let _x: u8 = "not a number";` — triggers E0308 at type-check
///
/// Assertion path A (passes when wired): with `-A unused_imports`, normalized
/// stderr contains only the E0308 error; diff is empty → `Verdict::Ok`.
///
/// Assertion path B (fails when broken): without `apply_allow_lints` wired,
/// normalized stderr contains both the warning and the error; diff is
/// non-empty → `Verdict::Diff`.
///
/// Guard ordering per plan §4c.3:
/// 1. Skip-guard: `LIHAAF_RUN_CARGO_BUILD_TESTS` absent → `eprintln!` + `return`.
/// 2. Panic-guard: env var present but `rustc` absent → `panic!`.
#[test]
fn allow_lints_suppresses_warning_in_compile_fail_fixture() {
    // Skip-guard — mirrors `cargo_accepts_staged_overlay_for_dylib_build`
    // at tests/compat/overlay_determinism.rs:694-701.
    if std::env::var_os("LIHAAF_RUN_CARGO_BUILD_TESTS").is_none() {
        eprintln!(
            "skipping allow_lints_suppresses_warning_in_compile_fail_fixture: \
             set LIHAAF_RUN_CARGO_BUILD_TESTS=1 to opt in (CI does this \
             automatically)"
        );
        return;
    }

    // Panic-guard: env var present + rustc absent = CI misconfiguration.
    let rustc_present = std::process::Command::new("rustc")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    assert!(
        rustc_present,
        "LIHAAF_RUN_CARGO_BUILD_TESTS is set but `rustc` is not on $PATH — \
         this is a CI misconfiguration, not an expected local skip"
    );

    let tmp = tempfile::tempdir().expect("creating tempdir for allow_lints test");
    let crate_root = tmp.path();

    // ---- Synthetic adopter crate ----
    //
    // Cargo.toml with allow_lints = ["unused_imports"].
    std::fs::write(
        crate_root.join("Cargo.toml"),
        r#"[package]
name = "allow_lints_fixture_adopter"
version = "0.1.0"
edition = "2021"

[lib]
crate-type = ["dylib", "rlib"]

[package.metadata.lihaaf]
dylib_crate = "allow_lints_fixture_adopter"
extern_crates = ["allow_lints_fixture_adopter"]
fixture_dirs = ["tests/lihaaf/compile_fail"]
allow_lints = ["unused_imports"]
"#,
    )
    .expect("writing adopter Cargo.toml");

    // src/lib.rs — minimal so cargo has something to compile as the dylib.
    let src_dir = crate_root.join("src");
    std::fs::create_dir_all(&src_dir).expect("creating src/");
    std::fs::write(src_dir.join("lib.rs"), "// minimal lib\n").expect("writing src/lib.rs");

    // tests/lihaaf/compile_fail/ — fixture directory.
    let fixture_dir = crate_root.join("tests").join("lihaaf").join("compile_fail");
    std::fs::create_dir_all(&fixture_dir).expect("creating fixture dir");

    // The fixture: `unused_imports` warning + E0308 type error.
    // Mirrors probe.rs from plan §4c.0:
    //   rustc --edition 2021 --crate-type=bin --error-format=json -o out probe.rs
    // produces (rustc 1.95.0):
    //   (1) warning: unused import: `std::collections::HashMap`
    //   (2) error[E0308]: mismatched types
    // With `-A unused_imports`, only (2) appears.
    std::fs::write(
        fixture_dir.join("unused_import_and_type_error.rs"),
        "use std::collections::HashMap;\n\
         fn main() { let _x: u8 = \"not a number\"; }\n",
    )
    .expect("writing fixture RS");

    // Pre-blessed snapshot: only the E0308 error, $DIR-normalized path.
    // Derived by running the probe under lihaaf's actual argv shape and
    // inspecting the normalized rendered output. The lihaaf normalizer
    // replaces the absolute source directory with $DIR (§6.2).
    std::fs::write(
        fixture_dir.join("unused_import_and_type_error.stderr"),
        "error[E0308]: mismatched types\n \
         --> $DIR/unused_import_and_type_error.rs:2:25\n\
         \nerror: aborting due to 1 previous error\n\
         \nFor more information about this error, try `rustc --explain E0308`.\n",
    )
    .expect("writing fixture snapshot");

    // ---- Run lihaaf via parse_from ----
    //
    // `lihaaf::cli::parse_from` constructs a `Cli` via clap; `inner_compat_normalize`
    // is a `#[arg(skip)]` field and defaults to `false` (correct for non-compat
    // sessions). This avoids the `pub(crate)` visibility gap for that field.
    let manifest_path = crate_root.join("Cargo.toml");
    let cli = lihaaf::cli::parse_from(vec![
        "cargo-lihaaf".to_string(),
        "lihaaf".to_string(),
        "--manifest-path".to_string(),
        manifest_path.to_string_lossy().to_string(),
        "--jobs".to_string(),
        "1".to_string(),
        "--quiet".to_string(),
    ])
    .expect("parse_from must succeed for valid argv");

    let report = lihaaf::run(cli).expect("lihaaf::run must succeed");

    assert_eq!(
        report.results.len(),
        1,
        "expected exactly one fixture result; got {}",
        report.results.len()
    );

    // compile_fail + snapshot matches → Verdict::Ok.
    // If allow_lints is not wired, the `unused_imports` warning leaks into
    // normalized stderr; diff is non-empty → Verdict::Diff → this assertion fails.
    let verdict = &report.results[0].verdict;
    assert!(
        matches!(verdict, lihaaf::Verdict::Ok),
        "expected Verdict::Ok but got {verdict:?};\n\
         if the verdict is Verdict::Diff, the `unused_imports` warning was NOT \
         suppressed — check that apply_allow_lints is wired in spawn_and_monitor"
    );
}
