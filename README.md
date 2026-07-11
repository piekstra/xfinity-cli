# xfinity-cli

Manage your [Xfinity](https://www.xfinity.com) account from the command line —
account profile, billing, payments, and internet data usage. The binary is
`xfin`. Your session lives only in the OS keychain, output is human- and
agent-friendly text, and every command returns a stable exit code.

> **Unofficial.** Xfinity publishes no public API. `xfin` talks to the same
> `customer.xfinity.com/apis/*` self-care services the My Account website uses.
> It is not affiliated with, endorsed by, or supported by Comcast/Xfinity. Use
> it on your own account.

## Install

```sh
cargo install --git https://github.com/piekstra/xfinity-cli
```

Or download a release binary from the
[Releases](https://github.com/piekstra/xfinity-cli/releases) page. Once
installed, `xfin self-update` upgrades in place.

## Authenticate

Xfinity's login is behind bot protection that blocks non-browser clients, so
`xfin` replays a session you capture from a logged-in browser rather than a
password:

1. Sign in at <https://www.xfinity.com> / My Account in your browser.
2. Open DevTools → Network, click a request to `customer.xfinity.com/apis/...`,
   and copy its `Cookie` request header.
3. Store it in the keychain:

   ```sh
   export XFINITY_USERNAME="you@example.com"
   pbpaste | xfin auth login --stdin        # macOS; reads the session from the clipboard
   ```

`xfin` replays that cookie plus the CSRF token it carries on every request.
`xfin auth login` verifies the session before saving it. When Xfinity expires
it, repeat with `--overwrite`. Full walkthrough: [`docs/api.md`](docs/api.md).

The session never comes from a command-line flag (which would leak into `ps`
and shell history) — only `--stdin` or `--from-env <VAR>`.

## Use

```sh
xfin account get                 # account holder, service address, account number
xfin account number              # default account number
xfin account users               # users/contacts on the account
xfin account info                # account locality / service info
xfin account security            # 2FA / MFA enrollment status
xfin billing summary             # balance, due date, autopay status
xfin billing duedates            # upcoming due date + valid payment days
xfin billing statements          # prior statements
xfin billing statement <id>      # one statement
xfin internet usage              # current-cycle data usage
xfin internet plan               # subscribed plan / speeds
xfin internet devices            # devices on the gateway
xfin internet status             # gateway/modem online status
xfin outages                     # service outage status
xfin equipment returns           # pending equipment returns

# Payments (best-effort — see note below)
xfin payments list               # payment history
xfin payments methods            # saved payment methods
xfin payments create --amount 50.00 --method <token>   # confirms first; --force to skip

# Raw request to any endpoint (always JSON) — handy while shapes are mapped
xfin api GET /apis/bill/current
```

> **Payments are best-effort.** The read commands above use the
> `customer.xfinity.com` session. The payments flow lives on a separate
> OAuth-gated app (`payments.xfinity.com`) that this session doesn't cover, so
> `payments list|methods|create` may return an auth error. See
> [`docs/api.md`](docs/api.md) for details.

`xfin auth status` shows what's configured. `xfin auth logout` clears the
stored session (`--forget` also drops saved prefs).

## Output & exit codes

Resource reads print `Key: value` blocks and pipe-delimited tables on stdout;
diagnostics go to stderr. JSON is reserved for control-plane commands
(`auth`/`set-credential`/`self-update` results and `xfin api`).

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
