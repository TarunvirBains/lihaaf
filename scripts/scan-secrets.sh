#!/usr/bin/env bash
# Scan staged diff for sensitive information patterns.
# See SECURITY.md for the supported pattern set + placeholder convention.

set -e

# Pattern definitions (POSIX ERE). Each pattern is a single line; the
# pattern name is in the comment immediately above.
PATTERNS=(
    "(postgres|mysql|mongodb|redis|amqp)://[^/[:space:]:]+:[^@/[:space:]]+@"
    "(DATABASE_URL|PGPASSWORD|POSTGRES_PASSWORD|DB_PASSWORD|MYSQL_PASSWORD|API_KEY|.*_SECRET|.*_TOKEN|CLIENT_SECRET|WEBHOOK_SECRET)[[:space:]]*=[[:space:]]*['\"']?[^[:space:]<\"'#]"
    "-----BEGIN (RSA |DSA |EC |OPENSSH |PGP )?PRIVATE KEY-----"
    "AKIA[0-9A-Z]{16}"
)
PATTERN_NAMES=(
    "database_url_with_creds"
    "env_var_with_secret_key"
    "private_key_block"
    "aws_access_key"
)

findings=0
diff_output="$(git diff --cached --diff-filter=AM 2>/dev/null || true)"
if [ -z "$diff_output" ]; then
    exit 0  # Nothing staged; pass.
fi

i=0
while [ $i -lt ${#PATTERNS[@]} ]; do
    pattern="${PATTERNS[$i]}"
    name="${PATTERN_NAMES[$i]}"
    # Process the diff line by line so we can check the whole line
    # for the placeholder allow-list, not just the regex match.
    while IFS= read -r line; do
        # Only check added-content lines (begin with '+', not '+++').
        case "$line" in
            "+++"*) continue ;;
            "+"*) ;;
            *) continue ;;
        esac
        # Strip leading '+'.
        content="${line#+}"
        # Allow-list: line contains `<word>`-style placeholder syntax.
        if echo "$content" | grep -qE -- '<[A-Za-z_][A-Za-z0-9_-]*>'; then
            continue
        fi
        if echo "$content" | grep -qE -- "$pattern"; then
            findings=$((findings + 1))
            echo "scripts/scan-secrets.sh: FAILED — credential-like pattern detected"
            echo "  pattern: $name"
            echo "  (matched line + raw match REDACTED; review staged diff to locate)"
            echo ""
        fi
    done <<< "$diff_output"
    i=$((i + 1))
done

if [ $findings -gt 0 ]; then
    echo "Bypass for this commit (NOT recommended in shared repos): git commit --no-verify"
    echo "See SECURITY.md for the placeholder convention."
    exit 1
fi
exit 0
