#!/usr/bin/env bash
set -e
ROOT="$(git rev-parse --show-toplevel)"
HOOK_PATH="$ROOT/.git/hooks/pre-commit"
EXPECTED_CONTENT='#!/usr/bin/env bash
exec "$(git rev-parse --show-toplevel)/scripts/scan-secrets.sh"'

if [ -e "$HOOK_PATH" ]; then
    if [ "$(cat "$HOOK_PATH")" = "$EXPECTED_CONTENT" ]; then
        echo "Pre-commit hook already installed at $HOOK_PATH (idempotent re-run)."
        exit 0
    else
        echo "ERROR: a different pre-commit hook already exists at $HOOK_PATH."
        echo "Inspect it. To replace with lihaaf's secrets scanner, delete it first:"
        echo "  rm \"$HOOK_PATH\""
        echo "Then re-run this installer."
        exit 1
    fi
fi

mkdir -p "$ROOT/.git/hooks"
printf '%s\n' "$EXPECTED_CONTENT" > "$HOOK_PATH"
chmod +x "$HOOK_PATH"
echo "Installed pre-commit hook at $HOOK_PATH."
echo "It will call scripts/scan-secrets.sh on every git commit."
