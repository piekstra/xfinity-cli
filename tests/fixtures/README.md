# tests/fixtures

Static response bodies used by the offline tests. Nothing here touches the
network, and nothing here is a live capture of a real account.

## What's in here

| File | What it stands in for | Provenance |
|---|---|---|
| `statement_min.pdf` | a statement PDF body (`Content-Type: application/pdf`) | **synthetic** — 611 bytes, generated, reads "SAMPLE STATEMENT - SYNTHETIC FIXTURE" |
| `download_statement_base64.json` | the JSON envelope shape carrying base64 PDF under `responseData.data.statementPdf` | **synthetic shape** — hand-built to the `responseData.data` envelope every mapped `BillingInfo/*` endpoint uses; see the UNVERIFIED-LIVE note in `docs/api.md` |
| `download_statement_login_page.html` | the sign-in page Xfinity serves (HTTP **200**) when the Bearer token has expired | **synthetic** — hand-written, no real markup copied |

The two download fixtures are deliberately *not* labelled captures. The
statement-PDF endpoint has not been confirmed against a live account (see
`docs/api.md`), so pretending these came off the wire would be the more
dangerous lie. When someone does capture the real thing, replace them, scrub
per the rules below, and change the provenance column in the same commit.

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
