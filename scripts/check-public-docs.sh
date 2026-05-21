#!/usr/bin/env bash
# Scan staged public prose for process-history scaffolding that should stay in
# PRs/issues/internal notes, not published docs or changelogs.

set -e

ALLOW_MARKER="public-docs-allow-process-history"
findings=0

diff_output="$(git diff --cached --diff-filter=AM --unified=0 -- \
    '*.md' \
    'docs/**/*.md' \
    'README*' \
    'CHANGELOG*' \
    2>/dev/null || true)"

if [ -z "$diff_output" ]; then
    exit 0
fi

current_file=""
current_line=""

while IFS= read -r line; do
    case "$line" in
        "+++ b/"*)
            current_file="${line#+++ b/}"
            current_line=""
            continue
            ;;
        @@*)
            # Extract the added-side start line from hunks like: @@ -1 +12,3 @@
            added="${line#* +}"
            added="${added%% @@*}"
            added="${added%%,*}"
            current_line="$added"
            continue
            ;;
        "+++")
            continue
            ;;
        "+"*)
            content="${line#+}"
            ;;
        *)
            continue
            ;;
    esac

    if [ -z "$current_file" ]; then
        continue
    fi

    if printf '%s\n' "$content" | grep -q -- "$ALLOW_MARKER"; then
        if [ -n "$current_line" ]; then
            current_line=$((current_line + 1))
        fi
        continue
    fi

    # Keep this list high-confidence to avoid blocking normal docs. These terms
    # identify agent/review provenance, review verdict labels, or future-version
    # planning claims that should be rewritten as current behavior/support status.
    if printf '%s\n' "$content" | grep -Eiq -- \
        'Codex|Claude|Gemini|Sonnet|Haiku|Opus|strict-swe|adversarial[- ]review|review rounds?|round[- ]?[0-9]+|BLOCK|ALLOW|COUNTER_SIGNAL|FIX_BEFORE|POST_BETA|Phase[[:space:]]+[0-9]+|deferred to v[0-9]|v[0-9]+\.[0-9]+ deliverable|future release|future versions?'; then
        findings=$((findings + 1))
        location="$current_file"
        if [ -n "$current_line" ]; then
            location="$location:$current_line"
        fi
        echo "scripts/check-public-docs.sh: FAILED - process-history wording in public prose"
        echo "  file: $location"
        echo "  line: ${content}"
        echo "  rewrite as current behavior/support status, or add '$ALLOW_MARKER' if this is intentional audit/security history"
        echo ""
    fi

    if [ -n "$current_line" ]; then
        current_line=$((current_line + 1))
    fi
done <<< "$diff_output"

if [ $findings -gt 0 ]; then
    echo "Bypass for this commit: git commit --no-verify"
    exit 1
fi

exit 0
