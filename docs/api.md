# Xfinity self-care API notes

Xfinity publishes no official public API. `xfin` targets the same
`customer.xfinity.com/apis/*` self-care JSON services the My Account web app
calls. Paths in this file were mapped from the web app's own network traffic
against a live account and verified end-to-end.

## Auth

Xfinity's login (`login.xfinity.com`) is behind Akamai bot protection. A plain
`curl`/`reqwest` request to the login page returns **HTTP 403** — no headless
password login is possible. `xfin` therefore replays a session captured from a
real, logged-in browser.

The `/apis/*` services authenticate with two things, both obtained from the
browser session:

1. The **cookie jar** for `customer.xfinity.com` (the `Cookie` request header).
2. A **double-submit CSRF token**: every request must send an `x-xsrf-token`
   header whose value is the URL-decoded `XSRF-TOKEN` cookie. `xfin` derives
   this from the stored cookie automatically — you only capture the cookie.

### Capturing the session

1. Sign in at <https://www.xfinity.com> / My Account in your browser.
2. Open DevTools → Network. Click any request to `customer.xfinity.com/apis/...`
   (e.g. `bill/current`) and copy its **`Cookie`** request-header value.
3. Store it:

   ```sh
   pbpaste | xfin auth login --stdin           # macOS
   # or from a secrets manager:
   op read "op://Private/Xfinity/session" | xfin auth login --stdin
   ```

`xfin auth login` does a live `GET /apis/ssm/account/default` to confirm the
session works before committing it to the keychain (skip with `--no-verify`).
When Xfinity expires the session (a 401/403 comes back), log in again in the
browser and re-run `xfin auth login --overwrite`.

The self-care host is overridable with `$XFINITY_API_HOST` for probing.

## Endpoints

Base host: `https://customer.xfinity.com` (override with `$XFINITY_API_HOST`).
All are cookie + `x-xsrf-token` authenticated. Verified (✓) or best-effort (~).

| Purpose | Method | Path | Command | |
|---|---|---|---|---|
| Account profile | GET | `/apis/macaroon` | `xfin account get` | ✓ |
| Default account number | GET | `/apis/ssm/account/default` | `xfin account number` | ✓ |
| Users on account | GET | `/apis/users` | `xfin account users` | ✓ |
| Account info / locality | GET | `/apis/info` | `xfin account info` | ✓ |
| 2FA / MFA enrollment | GET | `/apis/csp/account/me/user/{guid}/twoFactorAuth` + `/multiFactorAuth` | `xfin account security` | ✓ |
| Billing summary | GET | `/apis/bill/current` | `xfin billing summary` | ✓ |
| Due dates | GET | `/apis/ssm/bill/duedates` | `xfin billing duedates` | ✓ |
| Statement list | GET | `/apis/brite-bill/account/SELF/bills` | `xfin billing statements` | ✓ |
| One statement | GET | `/apis/brite-bill/account/SELF/bill/{id}` | `xfin billing statement <id>` | ✓ |
| Internet usage | GET | `/apis/csp/account/me/services/internet/usage` | `xfin internet usage` | ✓ |
| Internet plan | GET | `/apis/csp/account/me/services/internet/plan` | `xfin internet plan` | ✓ |
| Devices | GET | `/apis/csp/account/me/devices` | `xfin internet devices` | ✓ |
| Gateway/modem status | GET | `/apis/ssm/devices/status` | `xfin internet status` | ✓ |
| Service outages | GET | `/apis/ssm/outage/consolidated/lob` | `xfin outages` | ✓ |
| Equipment returns | GET | `/apis/ssm/dsm/returns` | `xfin equipment returns` | ✓ |
| Payment history | GET | `/apis/ssm/payments/history` | `xfin payments list` | ~ |
| Payment methods | GET | `/apis/ssm/bill/paymentmethods` | `xfin payments methods` | ~ |
| Make a payment | POST | `/apis/ssm/payments` | `xfin payments create` | ~ |

### The payments surface lives on a separate host

The read surface above is served from `customer.xfinity.com`. The "Make a
payment" flow lives on a **separate app, `payments.xfinity.com`**, reached via a
silent OAuth handshake: visiting it redirects through
`oauth.xfinity.com/oauth/authorize?...&passive=1` →
`payments.xfinity.com/oauth/passive_connect/` → `/oauth/callback`, which — given
a live `customer.xfinity.com` SSO session — completes **without re-login** and
sets a `payments.xfinity.com` session cookie.

Its APIs then return real data (verified against a live account):

| Purpose | Method | Path (host `payments.xfinity.com`) |
|---|---|---|
| Saved payment methods | GET | `/apis/payments/instruments-v4` |
| Scheduled payments | GET | `/apis/payments/scheduled` |
| Autopay | GET | `/apis/autopay` |
| Current bill | GET | `/apis/bill/current` |

**Two requirements for a headless client to reach these** (both now met by
`client.rs` for the read surface, and required for a payments client too):

1. **The `payments.xfinity.com` cookie jar**, including Akamai's sensor cookies
   (`_abck`, `bm_sz`, `ak_bmsc`, `bm_sv`) — a *different* jar from the
   `customer.xfinity.com` one. This is a second session capture.
2. **Browser client-hint headers** — `Sec-Fetch-*` and `Sec-CH-UA*`. Without
   them Akamai returns `403 Access Denied` even with valid cookies; with them
   the same request returns `200`. `client.rs` now sends these on every request.

So payments support is **feasible headlessly** but needs a second stored session
scoped to `payments.xfinity.com`. Until that lands, `xfin payments *` remain
best-effort against the `customer.xfinity.com` `/apis/ssm/payments/*` routes
(which are more locked down) — use `xfin api` to drive them. A follow-up would
add `xfin payments login` (capture the payments session) and point
`payments methods`/`list` at `instruments-v4`/`scheduled`. Note that
`payments create` moves money and must keep its confirm guard.

### Other observed apps (not yet wired)

Discovered during the surface crawl but not exposed as commands:
`/cotton/apis/appointment` and `/cotton/apis/account` (the "cotton" app shell,
appointments), and `www.xfinity.com/digital/service/api/BillingInfo/*` (an
alternate billing service). `xfin api` can reach the `customer.xfinity.com`-
hosted ones directly.

## Verifying and refining shapes

Use the raw escape hatch to inspect real responses:

```sh
xfin api GET /apis/bill/current
xfin api GET /apis/brite-bill/account/SELF/bills
```

## Dev note: macOS Keychain prompts

Each `cargo build` produces a new (unsigned) binary identity, so macOS Keychain
re-prompts for access the first time a freshly built `xfin` **reads** the stored
session — click "Always Allow". A headless/no-GUI invocation will block on that
prompt. This only affects local rebuilds; released binaries are stable.
