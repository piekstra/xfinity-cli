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
### The payments surface lives on a separate host + session

The read surface above is served from `customer.xfinity.com`. The payment flow
lives on a **separate app, `payments.xfinity.com`**, with its **own session**
(a distinct cookie jar). In the browser it's reached via a silent OAuth
handshake — visiting it redirects through
`oauth.xfinity.com/oauth/authorize?...&passive=1` →
`payments.xfinity.com/oauth/passive_connect/` → `/oauth/callback`, which, given
a live `customer.xfinity.com` SSO session, completes **without re-login** and
sets the `payments.xfinity.com` session cookie.

`xfin` handles this with a **second stored session**: `xfin payments login`
captures the `payments.xfinity.com` cookie jar (see [§Payments session](#payments-session) below),
and the payment commands target that host. Verified endpoints (against a live
account):

| Purpose | Method | Path (host `payments.xfinity.com`) | Command | |
|---|---|---|---|---|
| Saved payment methods | GET | `/apis/payments/instruments-v4` | `xfin payments methods` | ✓ |
| Scheduled payments | GET | `/apis/payments/scheduled` | `xfin payments scheduled` | ✓ |
| Autopay | GET | `/apis/autopay` | `xfin payments autopay` | ✓ |
| Make a payment | POST | `/apis/payments` | `xfin payments create` | ~ |

`payments create` moves money — its submit path/shape is **not** confirmed, so
it's best-effort and keeps a confirm-by-default guard (`--force` to skip). Drive
it with `xfin api` against `payments.xfinity.com` first to pin the real shape.

**Why a headless client reaches these** (the barrier the read surface also
clears): (1) it needs the `payments.xfinity.com` cookie jar **including Akamai's
sensor cookies** (`_abck`, `bm_sz`, `ak_bmsc`, `bm_sv`), and (2) the
`Sec-Fetch-*` / `Sec-CH-UA*` client-hint headers — without them Akamai returns
`403 Access Denied` even with valid cookies. `client.rs` sends the headers on
every request; the sensor cookies come from the captured session.

#### Payments session

`payments.xfinity.com` is a *different login* from `xfin auth login`:

1. Sign in at <https://payments.xfinity.com> in your browser.
2. DevTools → Network, click a request to `payments.xfinity.com/apis/...`, copy
   its `Cookie` header.
3. `pbpaste | xfin payments login --stdin`.

Stored in the keychain under `<username>#payments` (separate from the customer
session). `xfin payments logout` clears it.

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
