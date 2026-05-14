# Secrets-scan text fixtures

Manual-test fixtures for `.github/scripts/scan-text-for-secrets.py` —
the GitHub Actions sibling of `scripts/scan-secrets.sh`.

These files mirror the shape of `scripts/tests/*.txt` (the local
guard's fixtures) but live here because the public-text guard runs
against issue/PR/comment **bodies** (rendered text), not staged
diffs. Same pattern set, same allow-list convention; what differs is
the input source.

## How to run

```bash
# Expect exit 1, prints "CATEGORY: aws_access_key"
python3 .github/scripts/scan-text-for-secrets.py < tests/fixtures/secrets-scan-text/positive_aws_key.txt

# Expect exit 0
python3 .github/scripts/scan-text-for-secrets.py < tests/fixtures/secrets-scan-text/negative_placeholder.txt

# Expect exit 1 with multiple distinct categories (deduped)
python3 .github/scripts/scan-text-for-secrets.py < tests/fixtures/secrets-scan-text/positive_multi_category.txt
```

## Files

- `positive_aws_key.txt` — AWS access-key shape
- `positive_postgres_url.txt` — postgres URL with embedded creds
- `positive_env_secret.txt` — `*_SECRET` env-var assignment
- `positive_private_key.txt` — RSA private-key block header
- `positive_multi_category.txt` — multiple categories in one body,
  exercises the deduplication path
- `positive_secret_then_placeholder.txt` — secret on line 1, placeholder
  on line 2; expect line 1 to fire (allow-list is per-line)
- `negative_placeholder.txt` — `<word>` placeholders only; expect clean
- `negative_test_password.txt` — `DUMMY_PASSWORD` does not match the
  env-var key regex; expect clean

The fixtures intentionally include no real credentials. The
"AKIA..." string is the well-known AWS documentation example, the
postgres URL uses `hunter2`, and the private-key block is just the
ASCII header line. They are pattern-shaped strings, not secrets.
