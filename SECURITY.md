# Security

## Sensitive information

This repo has a local pre-commit guard at `scripts/scan-secrets.sh` that
scans staged diffs for credential-like patterns. Install via:

    scripts/install-pre-commit-hook.sh

(Per-clone; git does not sync hooks.)

### Patterns detected

- Database URLs with embedded credentials
- Environment-variable assignments with secret-shaped keys
- Private key blocks
- AWS access keys

### Placeholder convention for examples

Use `<...>` placeholders in docs and examples:

    postgres://<user>:<password>@<host>:<port>/<database>

Lines containing `<word>`-style placeholder syntax are treated as
documentation examples and skipped by the scanner.

### Bypass

For legitimate false positives:

    git commit --no-verify

The bypass is stateless (no session leak) and standard git practice.
Use it sparingly — in shared repos, a bypassed commit still reaches
the remote git history where it cannot be cleanly removed.

### What the local guard does NOT do

- Does not rewrite git history. Any secret already in the repo's
  history must be considered exposed.
- Does not scan GitHub issues / PRs / comments — that surface is
  covered by the GitHub Actions public-text guard below.

## Public-text guard (GitHub Actions)

The `.github/workflows/secrets-scan.yml` workflow scans the text of
new issues, PR descriptions, comments, and review comments for the
same sensitive-info patterns as the local pre-commit guard. On a
match, the workflow posts a single comment naming the pattern
**category** (never the matched value), asking the author to redact
and edit.

The workflow uses `pull_request_target` for PR events so it runs with
the upstream token rather than the read-only fork-PR token. It does
NOT check out fork-PR head code — only the event payload's text is
scanned.

False positives can be cleared by editing the offending text to
include a `<word>`-style placeholder (the same allow-list marker the
local guard recognizes — see the convention above). For example, a
postgres URL pasted with real credentials can be redacted to the
`postgres://<user>:<password>@<host>:<port>/<database>` shape; the
line is then treated as documentation and skipped.

The pattern set lives in two places — `scripts/scan-secrets.sh` for
the local guard and `.github/scripts/scan-text-for-secrets.py` for
the workflow. When updating one side, update the other in the same
PR; the parity check in `scripts/run-scan-tests.sh` and the manual
fixtures under `tests/fixtures/secrets-scan-text/` help confirm both
sides still agree.

### What the public-text guard does NOT do

- Does not scan attachments, images, or external links — only the
  inline text of issues, PRs, and comments.
- Does not block the event. The workflow posts an advisory comment;
  the author must edit the offending text manually.

## Reporting a vulnerability

Email TarunvirBains@kindnudge.app with a clear description. Do not file a
public GitHub issue with the vulnerability details.
