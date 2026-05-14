#!/usr/bin/env python3
"""
Scan public-text surfaces (issue/PR/comment bodies) for sensitive-info
patterns and emit pattern CATEGORY names — never the matched value.

This is the GitHub Actions sibling of scripts/scan-secrets.sh. The two
guards share a pattern set and an allow-list convention so contributors
get the same answer locally and remotely.

Contract
--------

Input (preferred — env vars set by the workflow):

    TEXT_SCAN_TITLE   one-line title text (issue or PR title)
    TEXT_SCAN_BODY    multi-line body text (issue/PR/comment body)

Either may be unset / empty. If both are unset *and* stdin has data,
the script reads stdin instead (convenient for local development and
for the verification gate in the dispatch prompt).

Output:

    one line per finding, exactly:   CATEGORY: <category-name>
    distinct categories are deduplicated across the input.

If GITHUB_OUTPUT is set (the workflow runner), the same list is also
written to that file in heredoc form:

    findings<<DELIM
    CATEGORY: aws_access_key
    DELIM

That makes the categories available to the next step as
`steps.<id>.outputs.findings`. This addresses Gemini-3-Pro round-1
finding #1 against the original Spec F draft, which used
`print('FINDINGS::...')` and never wrote to $GITHUB_OUTPUT.

Exit codes:

    0   no findings
    1   findings detected (used by the workflow as the gate)

Pattern set + allow-list
------------------------

Patterns and category names mirror scripts/scan-secrets.sh exactly so
that a string that fails the local pre-commit hook also fails the
public-text guard (and vice versa). When updating one side, update the
other in the same PR.

Bash POSIX-ERE -> Python `re` translation: [[:space:]] -> \\s, the rest
is character-class identical.

Allow-list: a line containing `<word>`-style placeholder syntax (regex
`<[A-Za-z_][A-Za-z0-9_-]*>`) is treated as a documentation example and
skipped. This matches the local guard's behavior — see SECURITY.md.

The allow-list check looks at the WHOLE LINE, not the regex match
span. This addresses Gemini-3-Pro round-1 finding #4 against the
original Spec F draft, which used `m.group(0)` (only the matched
secret-shaped substring) and so could not see surrounding placeholder
markers on the same line.

Security discipline
-------------------

This script MUST NEVER print, log, or return the matched value. Only
the category name is exposed. Reviewers updating this file: search
for `print(` and verify each call site only emits literal strings or
category names from CATEGORIES. There is no `print(line)`,
`print(m.group(0))`, etc.
"""
from __future__ import annotations

import os
import re
import secrets
import sys
from typing import Iterable

# (category_name, compiled_pattern)
# Category names match scripts/scan-secrets.sh PATTERN_NAMES.
CATEGORIES: list[tuple[str, re.Pattern[str]]] = [
    (
        "database_url_with_creds",
        re.compile(r"(postgres|mysql|mongodb|redis|amqp)://[^/\s:]+:[^@/\s]+@"),
    ),
    (
        "env_var_with_secret_key",
        re.compile(
            r"(DATABASE_URL|PGPASSWORD|POSTGRES_PASSWORD|DB_PASSWORD|"
            r"MYSQL_PASSWORD|API_KEY|.*_SECRET|.*_TOKEN|CLIENT_SECRET|"
            r"WEBHOOK_SECRET)\s*=\s*['\"]?[^\s<\"'#]"
        ),
    ),
    (
        "private_key_block",
        re.compile(r"-----BEGIN (RSA |DSA |EC |OPENSSH |PGP )?PRIVATE KEY-----"),
    ),
    (
        "aws_access_key",
        re.compile(r"AKIA[0-9A-Z]{16}"),
    ),
]

PLACEHOLDER_RE = re.compile(r"<[A-Za-z_][A-Za-z0-9_-]*>")


def _collect_text() -> str:
    """Gather scan input from env vars, falling back to stdin."""
    title = os.environ.get("TEXT_SCAN_TITLE", "")
    body = os.environ.get("TEXT_SCAN_BODY", "")
    if title or body:
        # Title is treated as its own line so the line-level allow-list
        # logic applies cleanly to each input region.
        if title and body:
            return title + "\n" + body
        return title or body
    # No env input — fall through to stdin (used for local testing and
    # the dispatch's verification gate).
    if not sys.stdin.isatty():
        return sys.stdin.read()
    return ""


def _scan(text: str) -> list[str]:
    """Return distinct category names that fire on `text`.

    Order is preserved: the order in which a category first matches
    determines its order in the output. That makes the workflow's
    comment deterministic across multiple-finding inputs.
    """
    found: list[str] = []
    seen: set[str] = set()
    for raw_line in text.splitlines():
        # Allow-list: any line with a <word>-style placeholder is
        # treated as documentation and skipped. Check the WHOLE LINE,
        # not a per-match span (Gemini round-1 finding #4).
        if PLACEHOLDER_RE.search(raw_line):
            continue
        for name, pattern in CATEGORIES:
            if name in seen:
                continue
            if pattern.search(raw_line):
                # NOTE: never print the matched value. We append only
                # the category name. The matched line and substring
                # are intentionally discarded.
                found.append(name)
                seen.add(name)
    return found


def _emit_github_output(findings: Iterable[str]) -> None:
    """Write findings to $GITHUB_OUTPUT in heredoc form, if available.

    Heredoc delimiter is a fresh random hex token so it cannot collide
    with payload content (the payload is fixed-shape category names,
    but the defensive choice is cheap).
    """
    out_path = os.environ.get("GITHUB_OUTPUT")
    if not out_path:
        return
    delim = "EOF_" + secrets.token_hex(16)
    lines = list(findings)
    with open(out_path, "a", encoding="utf-8") as f:
        f.write(f"findings<<{delim}\n")
        for name in lines:
            f.write(f"CATEGORY: {name}\n")
        f.write(f"{delim}\n")


def main() -> int:
    text = _collect_text()
    if not text:
        return 0
    findings = _scan(text)
    for name in findings:
        # Only the category name leaves this process. Verified by
        # construction: `name` is a key in CATEGORIES.
        print(f"CATEGORY: {name}")
    _emit_github_output(findings)
    return 1 if findings else 0


if __name__ == "__main__":
    sys.exit(main())
