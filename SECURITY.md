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

### Reporting a vulnerability

Email TarunvirBains@kindnudge.app with a clear description. Do not file a
public GitHub issue with the vulnerability details.

### What this guard does NOT do

- Does not rewrite git history. Any secret already in the repo's
  history must be considered exposed.
- Does not scan GitHub issues / PRs / comments. That guard is a
  separate follow-up; until then, contributors must redact
  secrets manually before pasting into public GitHub text.
