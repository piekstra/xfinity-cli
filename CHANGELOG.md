# Changelog

All notable user-visible changes to `xfin`. See `docs/upstream-changes.md`
for the running log of Xfinity-side changes that forced fixes.

## 0.9.0 — 2026-08-12

### Fixed

- **`billing download` now actually works.** v0.8.0 shipped with an inferred
  path (`BillingInfo/downloadStatement`) that Xfinity's account-experience
  surface doesn't own — every live call returned HTTP 404. The real endpoint,
  captured against a live account on 2026-08-12, is a two-step flow on a
  different host:
  1. `GET https://api.sc.xfinity.com/session/ssm/bill/pdf?statementDate=MM-DD-YYYY&signed=true`
     (same Bearer token) → JSON with a presigned CloudFront URL.
  2. `GET <cloudfront_url>` (no auth header — S3 signature validates the URL
     itself) → the PDF bytes.

  Fixes #17. Reverts the inferred shape from #19. See `docs/api.md` and
  `docs/upstream-changes.md#2026-08-12` for the full flow and the DevTools
  recipe that produced it.

### Changed

- The auth-expiry guard now applies at *both* hops of the download —
  either request answering with the sign-in page still exits 3, and no
  bytes reach the filesystem unless they start with `%PDF`.
- New env var `XFINITY_SC_API_HOST` overrides the SSM host for probing
  (parallel to the existing `XFINITY_API_HOST`).

### Removed

- The base64-in-JSON decode path in `decode_pdf`. The real response is a
  presigned URL, not a base64 payload; the fallback tolerated a shape that
  never existed. Dropped `base64` as a dependency.
- Fixture `tests/fixtures/download_statement_base64.json`. Replaced with
  `download_statement_signed_url.json` — the shape actually observed live.

## 0.8.0 — earlier

`billing download <id>` / `--all` + auth refresh, mostly. See git log for
detail; releases before 0.9.0 don't have a CHANGELOG entry (the log was
introduced with this release).
