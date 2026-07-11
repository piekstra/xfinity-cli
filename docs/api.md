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

### The payments surface is gated separately

The read surface above authenticates with the `customer.xfinity.com` cookie +
`x-xsrf-token` pair. **Payments do not.** The My Account "Make a payment" flow
lives on a separate app, `payments.xfinity.com`, which runs its own OAuth
handshake (`oauth.xfinity.com/oauth/authorize` → `/oauth/callback`) and does
not accept the `customer.xfinity.com` session — `payments.xfinity.com/apis/*`
returns `403` with the read-surface cookie. The `/apis/ssm/payments/*` routes on
`customer.xfinity.com` are likewise more locked down (some need the macaroon
bearer `csp-authorization: LoginToken …` the SPA mints).

Because of this, `xfin payments list|methods|create` are **best-effort** and may
`403`. Fully supporting them means implementing the `payments.xfinity.com` OAuth
flow (a separate capture, and money-moving), which this CLI does not yet do.
Inspect what the browser actually calls with `xfin api` before relying on them.

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
