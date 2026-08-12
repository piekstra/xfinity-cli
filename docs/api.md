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

### Silent auto-refresh on 401

Once a refresh command is configured (any of the three sources above), a read
that fails with 401/403 will invoke it, retry the read once, and — on success
— continue silently. This is what makes headless/agent workflows viable: the
family's shared retry rail (`pk_cli_auth::reauth::with_reauth`) runs `reauth`
at most once, never loops, and returns the *second* error unchanged if the
retry also fails — a broken helper can't turn into a login storm. Only
`CliError::Auth` (401/403) triggers recovery; upstream, not-found, and usage
errors surface as-is.

Turn the behaviour off with `xfin config set auto_refresh false` (or unset it
to fall back to the on-by-default). With `auto_refresh` off, or with no
refresh command configured, a 401 exits `3` with the same "capture a fresh
token" message the tool has always emitted.

`xfin auth status` lifts the token's `exp` claim into the standard
`auth-status/v1` DTO (`expires_at`, `session_valid`) when the stored token is
a JWT; Xfinity's account-experience tokens carry `exp`, so an agent can see
when a re-auth is due without spending a request. Text mode adds a `Refresh:`
line noting whether a helper is configured and whether `auto_refresh` is on.

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

### `BillingInfo/downloadStatement` (statement PDF) — **UNVERIFIED-LIVE**

> **UNVERIFIED-LIVE.** Everything in this section is *inferred*, not observed.
> No request to this path has ever been made against a live account from this
> CLI — the token in the keychain was expired throughout the work that added
> it, and re-capturing one needs a human at a browser. The path name, the
> request body and both response shapes below are reasoned from the
> surrounding `BillingInfo/*` surface (all POST, all `{…, "metadata":
> {"source":"web"}}`, all answering under `responseData.data`), **not**
> captured from DevTools. Treat a 404 here as expected until someone confirms
> it. See "Confirming it" below.

Body: `{"statementDate":"YYYY-MM-DD","metadata":{"source":"web"}}`
→ the statement PDF for that billing period. The account app keys statement
downloads by the statement's ISO issue date (there is no first-class statement
id on this surface — `statementDetails.lastStatementDate` is the handle, which
is also why `billing statement <id>` stays unmapped).

| Command | Field |
|---|---|
| `billing download <id>` / `--all` | the response body — raw PDF bytes when `Content-Type: application/pdf`, or JSON `{"responseData":{"data":{"statementPdf":"<base64>"}}}` (also tolerates `pdfBytes`, `bytes`, `content` under `responseData.data`) |

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

The document surface behaves like the rest of Xfinity's front end — an
unauthenticated request is **redirected to the sign-in page**, which `reqwest`
follows, so a dead token surfaces as `200 OK` with `text/html`. A downloader
that trusts the status code writes a login page into `statement.pdf` and
reports success. So the client never returns bytes it hasn't proved are a PDF:

1. Final URL looks like a sign-in page (`login.xfinity.com`, `/oauth/`,
   `/login`, `/signin`) → **exit 3** (auth), before the body is even read.
2. Body starts with `%PDF` → accepted, *whatever* the `Content-Type` claims.
   The magic number is evidence; the header is only a claim.
3. Body is HTML (by `Content-Type` **or** by sniffing `<!doctype html` /
   `<html`) → **exit 3**, never written.
4. JSON envelope → base64-decode, then re-apply (2) and (3) to the decoded
   bytes; an envelope can carry a base64'd login page just as easily.
5. Anything else → **exit 5** (upstream), naming the field paths tried.

Only step 2 and a clean step 4 ever reach the filesystem. `decode_pdf` in
`src/client.rs` owns this and is covered by offline tests over
`tests/fixtures/` (a synthetic PDF, the JSON envelope shape, and a sign-in
page that must produce exit 3).

#### Confirming it

Sign in at <https://www.xfinity.com/account>, open DevTools → Network, click
**Download PDF** on a statement, then reconcile the real request/response
against `Xfinity::download_statement` and `decode_pdf`'s field-name candidates
in `src/client.rs`. When it's confirmed, drop the UNVERIFIED-LIVE banner, note
the capture date, and replace the synthetic fixtures with scrubbed real ones
(see `tests/fixtures/README.md`). If the path turns out to be wrong, the CLI's
404 message already names this recipe.

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
endpoints are mapped: `payments
methods`/`create`/`login`/`logout`, `account security`, `equipment
returns`, `billing statement <id>` (the metadata read; the PDF download is
mapped separately, above, under `BillingInfo/downloadStatement` — and is
**UNVERIFIED-LIVE**). The old payments app
(`payments.xfinity.com`, separate OAuth) likely still governs payment
instruments/submission.

## Raw requests

```sh
xfin api POST BillingInfo/context --data '{"eventNames":["call.getContext.Account"],"data":{"metadata":{"source":"maw"}}}'
xfin api POST BillingInfo/billingSummary --data '{"requestTypes":["CORE"],"metadata":{"source":"web"}}'
```

## Dev note: macOS Keychain prompts

Each plain `cargo build` produces a new binary identity, so macOS Keychain
re-prompts on the first token read. Build with `make dev` (re-signs with the
stable `pk-cli-codesign` identity) when exercising keychain-touching commands.
