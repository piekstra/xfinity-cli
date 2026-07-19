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
| 2026-07-17 | Account migrated to the new `www.xfinity.com/account` experience; legacy `customer.xfinity.com/apis` surface went dead | **Yes — all commands** | v0.4.0 (#7) |
| 2026-07-11 | Payment surface confirmed on a separate `payments.xfinity.com` OAuth app | No (discovery) | v0.3.0 (#4) |
| 2026-07-10 | Akamai Bot Manager enforces `Sec-Fetch-*` / `Sec-CH-UA*` client hints (403 without them) | No (hardening) | v0.3.0 (#1) |
| 2026-07-10 | Baseline surface: `customer.xfinity.com/apis/*`, cookie + `x-xsrf-token` (double-submit CSRF) | — (initial) | v0.1.0 |

---

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
