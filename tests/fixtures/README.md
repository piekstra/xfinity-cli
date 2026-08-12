# tests/fixtures

Static response bodies used by the offline tests. Nothing here touches the
network, and nothing here is a live capture of a real account.

## What's in here

| File | What it stands in for | Provenance |
|---|---|---|
| `statement_min.pdf` | a statement PDF body (`Content-Type: application/pdf`) — the presigned-URL response in step 2 of the download flow | **synthetic** — 611 bytes, generated, reads "SAMPLE STATEMENT - SYNTHETIC FIXTURE" |
| `download_statement_signed_url.json` | the SSM step-1 response — flat `{"cloudfront_url":"…"}` envelope carrying a presigned CloudFront URL | **shape-only capture** — the shape (single flat field, S3-presigned URL with `AWSAccessKeyId` / `Signature` / `x-amz-security-token`) matches what `GET https://api.sc.xfinity.com/session/ssm/bill/pdf` returned 2026-08-12; every identifying value is a placeholder |
| `download_statement_login_page.html` | the sign-in page Xfinity serves (HTTP **200**) when the Bearer token has expired | **synthetic** — hand-written, no real markup copied |

The download fixtures are deliberately not raw captures. A real SSM response
carries a signed URL (a credential), and the URL path embeds an opaque
per-account hash — both are unsafe to commit. The shape here is confirmed
against live traffic; only the values are placeholders.

## Redaction rules — read before committing anything here

This repo is public. Every fixture must be free of personal data:

- account numbers → the placeholder `1234567890`
- names, emails, phone numbers, service addresses → strip, or replace with
  obviously-fake values
- session tokens, `Authorization` headers, cookies → **never commit**; a
  fixture carrying one is a leak, not a fixture. Pre-signed download URLs count
  as credentials too.
- statement PDFs → never commit a real one. A real statement is a service
  address, an account number and a payment history in a single file.
- amounts and dates → keep the *shape* (`45.33`, `2026-07-15`), invent the
  value.

Preserve response structure exactly — keys, nesting, types, null-vs-absent — so
that a provider rename fails a test loudly instead of silently emptying a
column. Change values, never shape.

`tests/fixture_shapes.rs` enforces the identity-bearing part of this policy on
every `cargo test` run. It is written **positively** — it asserts that
identifying fields match the known dummy values — rather than as a denylist of
real names, because a denylist would publish exactly what it exists to keep
out.
