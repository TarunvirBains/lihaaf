#!/usr/bin/env bash
# Run scripts/scan-secrets.sh against the fixture files under
# scripts/tests/ and verify expected pass/fail behavior. Used in CI.
set -e
ROOT="$(git rev-parse --show-toplevel)"
TMP_REPO="$(mktemp -d)"
trap 'rm -rf "$TMP_REPO"' EXIT

cd "$TMP_REPO"
git init -q
git config user.email "test@example.com"
git config user.name "Test Runner"

# Copy the scanner into a known place inside the tmp repo so the
# absolute path matches `git rev-parse --show-toplevel`.
mkdir -p scripts
cp "$ROOT/scripts/scan-secrets.sh" scripts/
chmod +x scripts/scan-secrets.sh

passed=0
failed=0

for fixture in "$ROOT/scripts/tests/"*.txt; do
    base="$(basename "$fixture")"
    expect_findings=0
    case "$base" in
        positive_*) expect_findings=1 ;;
        negative_*) expect_findings=0 ;;
        *) continue ;;
    esac
    # Stage the fixture content.
    cp "$fixture" staged.txt
    git add staged.txt
    set +e
    bash scripts/scan-secrets.sh > /dev/null 2>&1
    rc=$?
    set -e
    git reset -q HEAD staged.txt 2>/dev/null || true
    rm -f staged.txt
    if [ "$expect_findings" -eq 1 ] && [ "$rc" -ne 0 ]; then
        echo "PASS: $base (expected non-zero exit, got $rc)"
        passed=$((passed + 1))
    elif [ "$expect_findings" -eq 0 ] && [ "$rc" -eq 0 ]; then
        echo "PASS: $base (expected zero exit, got $rc)"
        passed=$((passed + 1))
    else
        echo "FAIL: $base (expected findings=$expect_findings, got rc=$rc)"
        failed=$((failed + 1))
    fi
done

echo ""
echo "Results: $passed passed, $failed failed"
if [ "$failed" -gt 0 ]; then
    exit 1
fi
