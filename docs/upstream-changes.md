# Observed Xfinity upstream changes

A running log of **Xfinity-side changes** that affected this CLI — the API
surface, auth model, or edge/bot-protection behavior. Xfinity ships no official
API and no changelog, so we track breakages here to (a) speed up the next fix
and (b) see how often the upstream breaks in practice.

Keep this **PII-free**: describe *what changed*, never real account numbers,
tokens, addresses, or cookies. Newest first. When a change forces a fix, link
the release/PR.

| Date observed | Change | Breaking? | Fixed in |
|---|---|---|---|
| 2026-08-12 | Statement-PDF download lives at `api.sc.xfinity.com/session/ssm/bill/pdf` (GET, two-step, signed S3 URL) — not on the account-experience surface at all | Yes — `billing download` (never worked in v0.8.0) | v0.9.0 |
| 2026-07-17 | Account migrated to the new `www.xfinity.com/account` experience; legacy `customer.xfinity.com/apis` surface went dead | **Yes — all commands** | v0.4.0 (#7) |
| 2026-07-11 | Payment surface confirmed on a separate `payments.xfinity.com` OAuth app | No (discovery) | v0.3.0 (#4) |
| 2026-07-10 | Akamai Bot Manager enforces `Sec-Fetch-*` / `Sec-CH-UA*` client hints (403 without them) | No (hardening) | v0.3.0 (#1) |
| 2026-07-10 | Baseline surface: `customer.xfinity.com/apis/*`, cookie + `x-xsrf-token` (double-submit CSRF) | — (initial) | v0.1.0 |

---

## 2026-08-12 — Statement-PDF download is on the SSM host, GET-based, and 2-step (breaking)

**What changed.** Not really a *change* on Xfinity's side — an incorrect
assumption in v0.8.0 (#19). The `BillingInfo/downloadStatement` path added
there returned 404 the first time it was tried against a live account,
because that endpoint doesn't exist: the account-experience surface at
`www.xfinity.com/digital/service/api/BillingInfo/*` doesn't own the
statement download.

**How we detected it.** First real run of `xfin billing download 2026-08-09`
after v0.8.0 shipped: `HTTP 404 for BillingInfo/downloadStatement — the
download endpoint may have moved.` (The 404-message recipe added in v0.8.0
for exactly this case did its job — pointed at DevTools, said what to check.)

**Real surface.** The "View statement history" link on
`www.xfinity.com/account` deep-links into the legacy billing app at
`customer.xfinity.com/billing/services/statement/history`. Clicking
"Statement PDF" there hits:

1. `GET https://api.sc.xfinity.com/session/ssm/bill/pdf?statementDate=MM-DD-YYYY&signed=true`
   with the same `Authorization: Bearer` token used by the account-experience
   API — a different host, but the same JWT. Returns
   `{"cloudfront_url": "https://ssm-prod-billpdf-cache.s3.amazonaws.com/…?AWSAccessKeyId=…&Signature=…&x-amz-security-token=…"}`.
2. `GET <cloudfront_url>` with no auth header (the URL carries its own AWS
   signature; a bearer would fail S3 validation). Returns
   `application/pdf`.

Note: the date format is **MM-DD-YYYY** on the SSM endpoint, not the ISO
`YYYY-MM-DD` the rest of the CLI passes around. The client reformats before
sending.

**Impact / fix.** v0.9.0 rewrites `Xfinity::download_statement` to the
two-step flow, adds `sc_api_host()` alongside `api_host()`,
`extract_signed_url` for the step-1 envelope, and drops the base64-in-JSON
decode path that was an inference from the surrounding endpoints. The
auth-expiry guard (HTML sign-in page arriving as HTTP 200) still applies —
now at both hops.

## 2026-07-17 — New account experience migration (breaking)

## 2026-07-17 — New account experience migration (breaking)

**What changed.** The account was moved to Xfinity's new account experience at
`www.xfinity.com/account`. The legacy self-care surface the CLI targeted
(`customer.xfinity.com/apis/*`, authenticated by a session cookie + a
`x-xsrf-token` double-submit CSRF header) stopped serving the account:
`customer.xfinity.com/` now redirects to `www.xfinity.com/account`, no
`XSRF-TOKEN` cookie is set, and every `/apis/*` call returns `401`.

**How we detected it.** Every command started returning `401`. A fresh login
reproduced it (not a stale session). `customer.xfinity.com/` redirected to
`www.xfinity.com/account` on every attempt, and a browser screenshot confirmed
the new UI.

**New surface.** `https://www.xfinity.com/digital/service/api/*`, **all POST**
with small JSON bodies, authenticated by **`Authorization: Bearer <token>`**
(captured from DevTools) — no cookies, no CSRF. Two "fat" endpoints cover most
reads: `BillingInfo/context` (account, devices, outages, plan) and
`BillingInfo/billingSummary` (balance, due date, autopay, statements, scheduled
payments). See [`api.md`](api.md).

**Impact / fix.** The credential model changed from a captured cookie to a
captured Bearer token. v0.4.0 (#7) re-points the client and ports the commands
that map to the two fat endpoints. Not-yet-remapped: `internet usage`, most
`payments`, `account security`, `equipment returns`, `billing statement <id>`.

## 2026-07-11 — Payments on a separate OAuth app (discovery)

**What changed / observed.** Payment methods, scheduled payments, and payment
submission are not on the main self-care host. They live on a separate app,
`payments.xfinity.com`, reached via a silent OAuth `passive_connect` handshake
off the main SSO session, and require the app's own cookie jar (including
Akamai sensor cookies). Not a breakage — a structural discovery while adding
payments. Handled in v0.3.0 (#4) with a second stored session. (Superseded for
migrated accounts by the 2026-07-17 change; payments remapping is pending.)

## 2026-07-10 — Akamai client-hint enforcement (hardening)

**What changed / observed.** Xfinity's Akamai edge returns `403 Access Denied`
to otherwise-authenticated requests that omit the `Sec-Fetch-*` / `Sec-CH-UA*`
browser client-hint headers. Adding them (matching the `User-Agent`) flips the
same request to `200`. Handled in v0.3.0 (#1); the headers carry forward to the
new surface.

## 2026-07-10 — Baseline

Initial mapped surface (v0.1.0): `customer.xfinity.com/apis/*`, GET, cookie +
`x-xsrf-token`. Documented for history; dead for migrated accounts as of
2026-07-17.
