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

## Endpoints

Base: `https://www.xfinity.com/digital/service/api/`, **all POST**, JSON bodies,
`Authorization: Bearer` auth. The surface consolidates into two "fat" endpoints
that most commands read from.

### `BillingInfo/billingSummary`

Body: `{"requestTypes":["CORE","XM"],"metadata":{"source":"web"}}`
→ `responseData.data.BBDS`:

| Command | Field |
|---|---|
| `billing summary` | `balance.balanceDue`, `dueDate`, `autopay.status/date`, `balance.pastDueBalance`, `balance.isDelinquent` |
| `billing due-dates` | `dueDate` |
| `billing statements` | `statementDetails` (a single summary: billStatus, lastStatementDate, statementBalance — not an id-addressable list, so `billing statement <id>` stays unmapped) |
| `payments scheduled` | `schedulePayments` |
| `payments autopay` | `autopay` (status, method, autopayInstrument.{paymentInstrumentType,instrumentNumber last-4}, next `date`) |

Also present: `transactionHistory` (posted payments: amount, method, confirmation, masked instrument), `lateFeeDetails`, `currentCycleDetails`.

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
returns`, `billing statement <id>`. The old payments app
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
