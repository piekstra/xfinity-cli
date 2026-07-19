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
> `internet usage`, `account security`, `billing statement <id>`, `equipment
> returns`, and `payments methods|autopay|create|login|logout`. See
> [`docs/api.md`](docs/api.md) for the surface map and what's mapped.

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
