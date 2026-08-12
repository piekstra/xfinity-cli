# Xfinity API notes (new account experience)

Xfinity publishes no official public API. As of mid-2026 accounts have been
migrated to a new account experience at `www.xfinity.com/account`; the legacy
`customer.xfinity.com/apis/*` surface (cookie + `x-xsrf-token`) is **dead** for
migrated accounts. This CLI targets the surface the new web app uses.

## Auth

Xfinity's login is behind Akamai bot protection, so the CLI does **not** replay
a password. It replays an **`Authorization: Bearer` token** captured from a
logged-in browser:

1. Sign in at <https://www.xfinity.com/account> in your browser.
2. Open DevTools → Network. Click any request to
   `www.xfinity.com/digital/service/api/...` (e.g. `BillingInfo/context`).
3. Copy its **`Authorization`** request header (`Bearer …`).
4. Store it:

   ```sh
   pbpaste | xfin auth login --stdin        # macOS
   ```

The stored token is sent as the `Authorization` header on every request. There
is no cookie and no CSRF token. When it expires (401/403), capture a fresh one
and re-run `xfin auth login --overwrite`. `xfin auth login` does a live
`BillingInfo/context` call to confirm the token before storing it (skip with
`--no-verify`). The host is overridable with `$XFINITY_API_HOST` for probing.

### Refreshing without hand-capture (`xfin auth refresh`)

Because Bearer tokens are short-lived, `xfin auth refresh` re-captures without
the DevTools dance: it runs a command **you** provide whose stdout is a fresh
token, verifies it (same live `BillingInfo/context` check, skip with
`--no-verify`), and stores it. No browser tooling ships with `xfin` — you bring
your own helper (e.g. a script that drives a headless browser), which keeps that
machinery out of the CLI. The command source, in precedence order, is:
`--command <cmd>` → `$XFINITY_REFRESH_COMMAND` → the saved `refresh_command`
config (persist `--command` with `--save`). It runs via `sh -c`.

`refresh` is an **xfin-specific** extension, not part of the piekstra-cli/1
standard `auth` surface — it exists only because Xfinity's browser-only login
forces frequent token expiry, unlike the family's password/guest CLIs.

## Endpoints

Base: `https://www.xfinity.com/digital/service/api/`, **all POST**, JSON bodies,
`Authorization: Bearer` auth. The surface consolidates into two "fat" endpoints
that most commands read from.

### `BillingInfo/billingSummary`

Body: `{"requestTypes":["CORE","XM"],"metadata":{"source":"web"}}`
→ `responseData.data.BBDS`:

| Command | Field |
|---|---|
| `summary` / `balance` | same fields as `billing summary`; with `--json` they emit the family's `utility-summary/v1` DTO (utility/v1 profile) |
| `billing summary` | `balance.balanceDue`, `dueDate`, `autopay.status/date`, `balance.pastDueBalance`, `balance.isDelinquent` |
| `billing due-dates` | `dueDate` |
| `billing statements` | `statementDetails` (a single summary: billStatus, lastStatementDate, statementBalance — not an id-addressable list, so `billing statement <id>` stays unmapped). With `--json` it emits `statement-list/v1`; the record id falls back to the statement date since this surface has no statement ids |
| `payments scheduled` | `schedulePayments` |
| `payments autopay` | `autopay` (status, method, autopayInstrument.{paymentInstrumentType,instrumentNumber last-4}, next `date`) |

Also present: `transactionHistory` (posted payments: amount, method, confirmation, masked instrument), `lateFeeDetails`, `currentCycleDetails`.

### `ssm/bill/pdf` (statement PDF) — **confirmed live 2026-08-12**

The new account experience UI doesn't own the statement-PDF download. The
"View statement history" link on `www.xfinity.com/account` deep-links into
the legacy billing app at `customer.xfinity.com/billing/services/statement/history`,
and clicking "Statement PDF" there hits the self-care (SSM) service on a
different host. So the download flow is:

**Host:** `https://api.sc.xfinity.com` (not `www.xfinity.com`).
Overridable with `$XFINITY_SC_API_HOST` for probing.

**Two requests, in order:**

1. `GET /session/ssm/bill/pdf?statementDate=MM-DD-YYYY&signed=true` with the
   same `Authorization: Bearer <token>` used by the account-experience API.
   The date format is **`MM-DD-YYYY`**, not the ISO `YYYY-MM-DD` the rest of
   the CLI passes around — the client reformats before sending. Omitting the
   query params returns whatever the "current" statement is; passing them
   picks an older statement by its issue date.
   → `application/json` `{"cloudfront_url": "https://ssm-prod-billpdf-cache.s3.amazonaws.com/…?AWSAccessKeyId=…&Signature=…&x-amz-security-token=…"}`.
   The URL is short-lived (the presigned `Expires=` is minutes out) and
   embeds an opaque per-account hash plus the account number in the path —
   both count as credentials; never log or commit them.
2. `GET <cloudfront_url>` **with no Authorization header** — the URL carries
   its own AWS signature and a bearer would fail S3 validation.
   → `application/pdf` bytes, or a redirect to the sign-in page if the
   upstream session died between step 1 and step 2.

| Command | Endpoint / field |
|---|---|
| `billing download <id>` / `--all` | step 1 above; the URL from `cloudfront_url` is fetched raw in step 2. Success iff the step-2 body starts with `%PDF`. |

With `--json`, single downloads emit `document-download/v1` and batches emit
`document-download-batch/v1` — cli-common's `documents/v1` profile, matching
`SavedDocument` (`schema`, `id`, `name`, `category`, `date`, `file`, `path`,
`bytes`) and `DownloadBatch` (`schema`, `count`, `bytes_total`, `dir`,
`items`) field-for-field.

Per that spec a document DTO carries **no financial fields**: the amount is
published by `billing statements` as `statement-list/v1`, while the download
shape describes the file. A partially-failed batch adds a `failed[]` array of
`{id, error}` — an xfin-local extension, omitted entirely when nothing failed,
so a clean run stays byte-identical to the family shape. The DTOs are
hand-built because `pk-cli-documents` is not in a released cli-common tag yet;
swap to the crate (and to the profile's `documents download` spelling, keeping
`billing download` as an alias) once one ships.

#### Trap: expiry arrives as HTTP 200 + HTML, not 401

SSM behaves like the rest of Xfinity's front end — an unauthenticated request
is **redirected to the sign-in page**, which `reqwest` follows, so a dead
token surfaces as `200 OK` with `text/html`. A downloader that trusts the
status code writes a login page into `statement.pdf` and reports success. So
the client never returns bytes it hasn't proved are a PDF:

1. Final URL of *either* request looks like a sign-in page
   (`login.xfinity.com`, `/oauth/`, `/login`, `/signin`) → **exit 3** (auth),
   before the body is even read.
2. Step 1 body isn't valid JSON, or is HTML (by `Content-Type` **or** by
   sniffing `<!doctype html` / `<html`) → **exit 3** (auth) or **exit 5**
   (upstream), depending on which.
3. Step 1 JSON has no `cloudfront_url` (also tolerates `cloudfrontUrl`,
   `url`, `pdfUrl`, `signedUrl` in case SSM reshapes) → **exit 5** (upstream),
   naming the field paths tried.
4. Step 2 body starts with `%PDF` → accepted, *whatever* the `Content-Type`
   claims. The magic number is evidence; the header is only a claim.
5. Step 2 body is HTML → **exit 3** (auth), never written.
6. Anything else on step 2 → **exit 5** (upstream).

Only step 4 ever reaches the filesystem. `download_statement`,
`extract_signed_url` and `decode_pdf` in `src/client.rs` own this and are
covered by offline tests over `tests/fixtures/` (a synthetic PDF, the
signed-URL envelope shape, and a sign-in page that must produce exit 3).

#### Re-confirming after an upstream shift

If step 1 starts returning 404s again, sign in at
<https://customer.xfinity.com/billing/services/statement/history>, open
DevTools → Network, click **Statement PDF** on a statement, and reconcile the
real request URL/query and JSON response against `Xfinity::download_statement`
and `extract_signed_url`'s field-name candidates in `src/client.rs`. Update
this section with the observed path and the date it was confirmed. If the
step-1 path itself moves, the CLI's 404 message already names this recipe.

### `BillingInfo/context`

Body: `{"eventNames":["call.getContext.Account","call.getContext.Subscription","call.getContext.Device","call.getContext.Outage","call.getContext.Indicator"],"data":{"metadata":{"source":"maw"}}}`
→ `responseData.data`:

| Command | Section / field |
|---|---|
| `account get`/`number`/`users`/`info` | `accountContext` (firstName, lastName, address, contactInfo.homePhone, accountNumber, status, users, loyalty.loyaltyTier) |
| `internet plan` | `subscriptionContext.customerPlanInfo.internet[0]` (plan e.g. `300Mbps`, planDescription) |
| `internet usage` (`--history`) | `subscriptionContext.customerPlanInfo.internet[0].usageMonths[]` (per-cycle homeUsage/allowableUsage in `unitOfMeasure`, startDate/endDate, policyName, per-device usage). ~12 months of history; last entry is the current cycle. `allowableUsage` 0 / >= 100000 / an "Unlimited …" policyName means uncapped. |
| `internet devices`/`status` | `deviceContext.equipment[]` (deviceMake, deviceModel, deviceStatus, macaddress, serialNumber) |
| `outages` | `outageContext` (isOutage, current.{internet,tv,voice,…}) |

## Not yet mapped to the new experience

These commands return a clear "not available yet" error until their new-surface
endpoints are mapped: `payments methods`/`create`/`login`/`logout`,
`account security`, `equipment returns`, `billing statement <id>` (the
metadata read; the PDF download is mapped separately, above, under
`ssm/bill/pdf`). The old payments app (`payments.xfinity.com`, separate
OAuth) likely still governs payment instruments/submission.

## Raw requests

```sh
xfin api POST BillingInfo/context --data '{"eventNames":["call.getContext.Account"],"data":{"metadata":{"source":"maw"}}}'
xfin api POST BillingInfo/billingSummary --data '{"requestTypes":["CORE"],"metadata":{"source":"web"}}'
```

## Dev note: macOS Keychain prompts

Each plain `cargo build` produces a new binary identity, so macOS Keychain
re-prompts on the first token read. Build with `make dev` (re-signs with the
stable `pk-cli-codesign` identity) when exercising keychain-touching commands.
