# AGENTS.md — xfinity-cli

Canonical agent entrypoint for this repo. `CLAUDE.md` is a one-line pointer here.

## What this is

A single-binary CLI (`xfin`) over Xfinity's **undocumented**
`api.sc.xfinity.com` self-care JSON services — account profile, billing, usage,
payments. No official API exists. Design ergonomics: a verb command surface,
keychain-only runtime secrets with stdin/env ingress, text-primary output, and
stable exit codes.

The endpoint map and auth flow are in [`docs/api.md`](docs/api.md) — read it
before touching `src/client.rs`.

## Auth reality (read this first)

Xfinity's login (`login.xfinity.com`) sits behind aggressive bot protection
that **403s any non-browser client** (plain `curl`/`reqwest` included). Every
working community tool drives a real browser. So this CLI does **not** replay a
username/password. Instead:

1. The user logs in at `xfinity.com` in a real browser.
2. They copy the `Cookie` request header the browser sends to
   `api.sc.xfinity.com` (DevTools → Network).
3. `xfin auth login --stdin` ingests it and stores it in the OS keychain.
4. Every request replays that cookie. When it expires, repeat.

This is the honest, robust model given the constraint. Do not add a
password-login path that pretends to authenticate — it will only ever hit a
bot wall.

## Local map

| Path | Responsibility |
|------|----------------|
| `src/main.rs` | thin entrypoint; parses args, dispatches |
| `src/cli.rs` | `clap` command tree (verbs + args) |
| `src/commands/*.rs` | one handler module per resource group |
| `src/client.rs` | `Xfinity` HTTP client: session replay, endpoints, raw `request` |
| `src/secrets.rs` | `Secret` (redacting/zeroizing) + `CredentialStore` + ingress |
| `src/config.rs` | `~/.config/xfinity-cli/config.json` (non-secret prefs) |
| `src/output.rs` | text rendering + control-plane JSON |
| `src/selfupdate.rs` | `self-update` from GitHub Releases |
| `src/error.rs` | `AppError` + exit codes |
| `src/dates.rs` | minimal date math (no calendar crate) |

## Durable conventions (do not drift)

- **Verb language.** Resource groups take fixed verbs: `get`, `list`,
  `summary`, `create`, `login`, `logout`, `status`. Domain reads that name a
  precise Xfinity concept are allowed. Don't coin a verb where a table verb
  fits.
- **Secrets: keychain-only at runtime.** The session is read only from the OS
  keychain. It gets there via `xfin auth login` / `xfin set-credential`, which
  ingest from `--stdin` or `--from-env <VAR>` — **never a `--value`/`--session`
  flag** (leaks to `ps`, history, transcripts). Replacement uses `--overwrite`.
  Wrap secrets in `secrets::Secret`; never log or serialize one.
- **Mutation safety.** `payments create` moves money — confirm by default, skip
  with `--force` (NOT `--yes`). A non-TTY run without `--force` fails with a
  hint.
- **Output: text is primary.** Resource reads render `Key: value` blocks and
  pipe-delimited (`ALL_CAPS`) tables. JSON is control-plane only
  (`auth login`/`set-credential`/`self-update` results, `auth status --json`,
  and the raw `api` payload). Do **not** add `--json` to resource reads. Data →
  stdout, diagnostics/confirmations → stderr.
- **Exit codes are a contract:** `0` ok, `1` other/keychain, `2` usage, `3`
  auth, `4` not found, `5` network. See `error.rs`.
- **Best-effort parsing.** Xfinity shapes vary by account type and drift. Never
  `unwrap()` on a response field; `output::render` flattens unknown shapes. Most
  typed endpoints in `client.rs` are mapped from the web app and not yet
  confirmed against a live account — use `xfin api` to verify shapes and refine.

## This repo is public

Never commit personal data — no real account number, service address, email,
cookie, session, or password, in code, tests, fixtures, comments, or commit
messages. Use the placeholder account `1234567890`. CI runs `gitleaks`.

## Local checks (must pass before pushing)

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

Authenticated paths can't run in CI (they need a live browser session); keep
their logic covered by unit tests on pure helpers and verify manually with
`xfin api`.
