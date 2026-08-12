# tests/fixtures

Static response captures used by the offline tests in `tests/`.

**Redaction rules — read before committing anything here.** This repo is
public. Every fixture must be free of personal data:

- account numbers → replace with the placeholder `1234567890`
- names, emails, phone numbers, service addresses → strip or replace with
  obviously-fake values
- session tokens, `Authorization` headers, cookies → **never commit**; a
  fixture that carries one is a leak, not a fixture
- statement PDFs → do not commit real PDFs; use a tiny synthetic
  `%PDF-1.4\n...` byte string in the test itself instead

Prefer inline literals in the test body (small JSON envelopes, short PDF
headers) over separate files — the test reads more clearly and there's no
chance a fixture drifts out of sync with the test that describes it.
