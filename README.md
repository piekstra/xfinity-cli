# xfinity-cli

Manage your [Xfinity](https://www.xfinity.com) account from the command line —
account profile, billing, payments, and internet data usage. The binary is
`xfin`. Your session lives only in the OS keychain, output is human- and
agent-friendly text, and every command returns a stable exit code.

> **Unofficial.** Xfinity publishes no public API. `xfin` talks to the same
> `www.xfinity.com/digital/service/api/*` services the new Xfinity account
> experience uses. It is not affiliated with, endorsed by, or supported by
> Comcast/Xfinity. Use it on your own account.

## Install

```sh
cargo install --git https://github.com/piekstra/xfinity-cli
```

Or download a release binary from the
[Releases](https://github.com/piekstra/xfinity-cli/releases) page. Once
installed, `xfin self-update` upgrades in place.

## Authenticate

Xfinity's login is behind bot protection that blocks non-browser clients, so
`xfin` replays an `Authorization: Bearer` token you capture from a logged-in
browser rather than a password:

1. Sign in at <https://www.xfinity.com/account> in your browser.
2. Open DevTools → Network, click a request to
   `www.xfinity.com/digital/service/api/...`, and copy its `Authorization`
   request header (`Bearer …`).
3. Store it in the keychain:

   ```sh
   export XFINITY_USERNAME="you@example.com"
   pbpaste | xfin auth login --stdin        # macOS; reads the token from the clipboard
   ```

`xfin` sends that token as the `Authorization` header on every request.
`xfin auth login` verifies it before saving. When Xfinity expires it, capture a
fresh one and repeat with `--overwrite`. Full walkthrough:
[`docs/api.md`](docs/api.md).

The token never comes from a command-line flag (which would leak into `ps` and
shell history) — only `--stdin` or `--from-env <VAR>`.

### Refreshing without re-capturing by hand

Because the token is short-lived, re-copying it from DevTools gets old. If you
have your own way to obtain a fresh token — say a browser-automation script —
point `xfin` at it once, and every command auto-recovers when the session
expires:

```sh
xfin auth refresh --command '~/bin/xfin-token.sh' --save   # remember the command
xfin summary                                               # 401? xfin refreshes and retries silently
xfin auth refresh                                          # force a refresh right now
```

The command runs via `sh -c` and its stdout is taken as the token (verified
before saving, like `login`). The source is `--command`, then
`$XFINITY_REFRESH_COMMAND`, then the saved `refresh_command` config. No browser
tooling ships with `xfin` — you bring your own helper, so a scheduled job (or
[utiman](https://github.com/piekstra/utiman)) can keep the session live.

**Silent auto-recovery on expiry.** Once a `refresh_command` is configured (or
`$XFINITY_REFRESH_COMMAND` is set), any read that fails with 401/403 will
invoke it once, retry the read, and — if that succeeds — continue as if
nothing happened. The retry is one-shot: a persistent auth failure still exits
`3` with the same message, so a broken helper won't turn into a login storm.
Turn the behaviour off with `xfin config set auto_refresh false` to require an
explicit `xfin auth refresh` between expiries. Details:
[`docs/api.md`](docs/api.md) §Auth.

`xfin auth status` reports the token's expiry (when the token is a JWT that
carries one) and whether a refresh helper is configured, so an agent can see
when a re-auth is due without spending a request.

## Use

```sh
xfin summary                     # balance, due date, autopay (utility-summary/v1 with --json)
xfin balance                     # current balance (same DTO as summary with --json)
xfin account get                 # account holder, service address, account number
xfin account number              # account number
xfin account users               # users/contacts on the account
xfin account info                # account profile
xfin billing summary             # balance, due date, autopay status
xfin billing due-dates           # upcoming due date
xfin billing statements          # statement details
xfin billing download <id>       # save a statement PDF (document-download/v1 with --json)
xfin billing download --all -o . # every statement (document-download-batch/v1 with --json)
xfin internet plan               # subscribed plan
xfin internet devices            # gateway / equipment
xfin internet status             # gateway status
xfin outages                     # service outage status
xfin payments scheduled          # scheduled (upcoming) payments
xfin config show                 # stored preferences (username, default account)

# Raw request (POST-only against digital/service/api paths)
xfin api POST BillingInfo/context --data '{"eventNames":["call.getContext.Account"],"data":{"metadata":{"source":"maw"}}}'
```

> **Not yet on the new experience.** Xfinity migrated accounts to a new
> experience (see the banner above); a few commands don't have their new
> endpoints mapped yet and return an explicit *"isn't available yet"* error:
> `internet usage`, `account security`, `billing statement <id>` (the
> metadata read; the PDF download is available via `billing download`),
> `equipment returns`, and `payments methods|autopay|create|login|logout`.
> See [`docs/api.md`](docs/api.md) for the surface map and what's mapped.

### Downloading statement PDFs

```sh
xfin billing statements                       # find the statement to key by
xfin billing download 2026-07-15 -o bill.pdf  # save one
xfin billing download 2026-07-15 -o -         # stream to stdout for piping
xfin billing download --all -o ./statements   # every statement Xfinity exposes
```

Statements are keyed by their **ISO issue date** — Xfinity's new account
experience publishes no separate statement id, so the date *is* the id.

`billing download` never writes a file it hasn't proved is a PDF. An expired
session on this surface does not come back as a `401`: Xfinity redirects to
the sign-in page, so the download arrives as `200 OK` with HTML. The command
checks the `%PDF` magic number (and sniffs for HTML and sign-in redirects)
before writing, and exits **3** — "auth" — when the session is dead, rather
than saving a login page as `statement.pdf` and claiming success.

> **Heads up — the download endpoint is inferred, not confirmed.** It is
> marked **UNVERIFIED-LIVE** in [`docs/api.md`](docs/api.md): it has never
> been exercised against a live account, so a `404` is a plausible first
> result. The error message names the DevTools recipe for pinning down the
> real path. Everything else about the command — flags, exit codes, output
> DTOs, the PDF guard — is covered by offline tests.

`xfin auth status` shows what's configured. `xfin auth logout` clears the
stored session (`--forget` also drops saved prefs).

## Output & exit codes

Resource reads print `Key: value` blocks and pipe-delimited tables on stdout;
diagnostics go to stderr. JSON is reserved for control-plane commands
(`auth`/`set-credential`/`self-update` results and `xfin api`) plus the
[utility/v1 domain profile](https://github.com/piekstra/cli-common): with the
global `--json`, `summary` and `balance` emit `utility-summary/v1` and
`billing statements` emits `statement-list/v1`, the shared shapes drivers like
utiman consume without per-provider configuration.

`billing download` is on that same shared surface: it emits
`document-download/v1` for one statement and `document-download-batch/v1` for
`--all` (cli-common's `documents/v1` profile), so an archiver can run `billing
statements --json` then `billing download <id> -o <path>` without learning
anything Xfinity-specific. Those DTOs deliberately carry **no amount** — a
statement's money belongs to `statement-list/v1`; the download shape describes
the *file*. Two DTOs reporting one balance are two things free to disagree.

| Code | Meaning |
|---|---|
| 0 | success |
| 1 | other / keychain error |
| 2 | usage error |
| 3 | auth required or rejected |
| 4 | not found |
| 5 | network / upstream error |

## Development

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

Agent-oriented notes and conventions live in [AGENTS.md](AGENTS.md); the
endpoint map is in [docs/api.md](docs/api.md).

## License

Dual-licensed under either of [Apache-2.0](LICENSE-APACHE) or [MIT](LICENSE-MIT)
at your option.
