#[test]
fn suite_demo_fixture_fails_when_per_fixture_cfg_is_missing() {
    let source =
        std::fs::read_to_string("tests/lihaaf/compile_pass_suite_demo/uses_suite_demo_marker.rs")
            .expect("suite_demo fixture should be readable");

    assert!(
        source.contains("#[cfg(feature = \"suite_demo\")]"),
        "fixture must have the feature-enabled marker branch"
    );
    assert!(
        source.contains("lihaaf::SUITE_DEMO_MARKER"),
        "fixture must reference the dylib feature-gated marker"
    );
    assert!(
        source.contains("#[cfg(not(feature = \"suite_demo\"))]"),
        "fixture must explicitly handle missing per-fixture cfg"
    );
    assert!(
        source.contains("compile_error!"),
        "missing per-fixture cfg must fail the compile_pass fixture"
    );
    assert!(
        !source.contains("#[cfg(not(feature = \"suite_demo\"))]\nfn main() {}"),
        "fixture must not silently pass when per-fixture cfg is missing"
    );
}
