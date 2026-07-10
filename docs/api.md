# Xfinity self-care API notes

Xfinity publishes no official public API. `xfin` targets the same
`api.sc.xfinity.com` self-care JSON services the my-account web app and the
Xfinity mobile app use. This file is the reference `src/client.rs` implements
against. Paths were mapped from the web app's own network traffic and
cross-checked against the community Home Assistant usage integrations; treat
the typed billing/account/payment endpoints as **best-effort and unconfirmed**
until validated against a live account with `xfin api`.

## Auth

Xfinity's login (`login.xfinity.com`) is behind Akamai bot protection. A plain
`curl`/`reqwest` request to the login page returns **HTTP 403** — no headless
password login is possible. `xfin` therefore replays a session captured from a
real, logged-in browser:

1. Sign in at <https://www.xfinity.com> in your browser.
2. Open DevTools → Network. Trigger a page that loads account/billing data
   (e.g. the billing overview) and find a request to `api.sc.xfinity.com`.
3. Copy the entire **`Cookie`** request header value for that request.
4. Store it:

   ```sh
   pbpaste | xfin auth login --stdin           # macOS
   # or, from an env var / secrets manager:
   op read "op://Private/Xfinity/session" | xfin auth login --stdin
   ```

`xfin auth login` does a live `GET /session/csp/selfhelp/account/me` to confirm
the session works before committing it to the keychain (skip with
`--no-verify`). The session is replayed as a `Cookie` header on every request.
When Xfinity expires it (a 401/403 comes back), log in again in the browser and
re-run `xfin auth login --overwrite`.

The self-care host is overridable with `$XFINITY_API_HOST` for probing.

## Endpoints

Base host: `https://api.sc.xfinity.com` (override with `$XFINITY_API_HOST`).

| Purpose | Method | Path | Command |
|---|---|---|---|
| Account profile | GET | `/session/csp/selfhelp/account/me` | `xfin account get` |
| Billing summary | GET | `/session/csp/selfhelp/billing/summary` | `xfin billing summary` |
| Statement history | GET | `/session/csp/selfhelp/billing/statements` | `xfin billing statements` |
| One statement | GET | `/session/csp/selfhelp/billing/statements/{id}` | `xfin billing statement <id>` |
| Payment history | GET | `/session/csp/selfhelp/billing/payments` | `xfin payments list` |
| Saved methods | GET | `/session/csp/selfhelp/billing/payment-methods` | `xfin payments methods` |
| Make a payment | POST | `/session/csp/selfhelp/billing/payments` | `xfin payments create` |
| Internet usage | GET | `/session/csp/selfhelp/account/me/services/internet/usage` | `xfin internet usage` |
| Internet plan | GET | `/session/csp/selfhelp/account/me/services/internet/plan` | `xfin internet plan` |
| Devices | GET | `/session/csp/selfhelp/account/me/services/internet/devices` | `xfin internet devices` |

The internet-usage path is the one most heavily exercised by community tools
and is the most likely to be exactly right. The billing and payment paths are
mapped by analogy and should be confirmed with `xfin api` before relying on
`xfin payments create`.

## Verifying and refining shapes

Use the raw escape hatch to inspect real responses and correct the map:

```sh
xfin api GET /session/csp/selfhelp/account/me
xfin api GET /session/csp/selfhelp/billing/summary
xfin api POST /session/csp/selfhelp/billing/payments \
  --data '{"amount":"50.00","paymentDate":"07/15/2026","paymentMethodId":"<token>"}'
```

When a real shape is confirmed, add a purpose-built renderer next to
`output::render` and pin the field names in the relevant `client.rs` method.
