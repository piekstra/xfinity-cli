# Changelog

All notable changes to `xfinity-cli` are documented here. Newest first. The
project follows [SemVer](https://semver.org/); breaking changes bump the major
once we cut 1.0 — until then, minors carry the breaking-change signal.

## Unreleased

### Fixed

- **`billing download` now actually works.** v0.8.0 shipped with an inferred
  path (`BillingInfo/downloadStatement`) that Xfinity's account-experience
  surface doesn't own — every live call returned HTTP 404. The real endpoint,
  captured against a live account on 2026-08-12, is a two-step flow on a
  different host:
  1. `GET https://api.sc.xfinity.com/session/ssm/bill/pdf?statementDate=MM-DD-YYYY&signed=true`
     (same Bearer token) → JSON with a presigned CloudFront URL.
  2. `GET <cloudfront_url>` (no auth header — S3 signature validates the URL
     itself) → the PDF bytes.

  Fixes #17. Reverts the inferred shape from #19. See `docs/api.md` and
  `docs/upstream-changes.md#2026-08-12` for the full flow and the DevTools
  recipe that produced it.

### Changed

- The auth-expiry guard now applies at *both* hops of the download —
  either request answering with the sign-in page still exits 3, and no
  bytes reach the filesystem unless they start with `%PDF`.
- New env var `XFINITY_SC_API_HOST` overrides the SSM host for probing
  (parallel to the existing `XFINITY_API_HOST`).

### Removed

- The base64-in-JSON decode path in `decode_pdf`. The real response is a
  presigned URL, not a base64 payload; the fallback tolerated a shape that
  never existed. Dropped `base64` as a dependency.
- Fixture `tests/fixtures/download_statement_base64.json`. Replaced with
  `download_statement_signed_url.json` — the shape actually observed live.

## 0.10.0 — 2026-11-03

- `xfin mcp`: read-only tools for Claude, the `utility/v1` set from cli-common's
  `pk-cli-utility`: `xfinity_balance`, `xfinity_bills` and `xfinity_usage`. They run
  over stdio for Claude Code and Claude Desktop, and with `--http` on 127.0.0.1 behind
  a bearer token for Claude on the web and mobile through your own Cloudflare Worker
  and tunnel. There is no payments tool: the posted-payments shape has not been
  captured.
- Releases attach `xfin-<version>.mcpb`, a Claude Desktop extension, and a Windows
  x86_64 tarball.
- `xfin mcp` reads `$XFINITY_TOKEN` before the keychain, and never runs the refresh
  command or writes the keychain. It reads the config on each call, not at start. It
  returns no account number. Every other command is unchanged.
- `self-update` says to restart a running `xfin mcp --http` and to install the new
  `.mcpb`.
- cli-common v0.21.0, up from v0.4.0. Rust 1.89 or later.

## 0.9.0

### Added

- **Silent auto-recovery on session expiry (headless-friendly).** When a read
  fails with 401/403 and a `refresh_command` is configured (via
  `xfin auth refresh --command … --save`, `$XFINITY_REFRESH_COMMAND`, or
  `config set refresh_command`), `xfin` now invokes it, retries the read once,
  and continues on success — matching how sibling CLIs (`pmac`, `wabhoa`,
  `cpmfl`) recover. Closes #18 for headless workflows: agents no longer see
  exit-3 the moment Xfinity expires the token. ([`pk_cli_auth::reauth`] is the
  retry-once, no-loop rail.)
- `Config::auto_refresh` (default `on`). Turn it off with
  `xfin config set auto_refresh false` to require an explicit
  `xfin auth refresh` between expiries.
- `xfin config set refresh_command <cmd>` / `unset` — the refresh command is
  now a first-class config key alongside `username` / `account`.
- `xfin auth status` reports:
  - `expires_at` (RFC 3339) and `session_valid` in the `auth-status/v1` DTO
    when the stored token is a JWT that carries an `exp` claim (Xfinity's
    are), so agents can see when a re-auth is due offline.
  - A `Refresh:` line in text mode noting whether a refresh command is
    configured and whether `auto_refresh` is on.
- Bumped the cli-common pins from `v0.2.0` to `v0.4.0` to pick up
  `pk_cli_auth::{reauth, token}`.

### Notes

- Xfinity's login remains behind Akamai bot protection; a pure username/
  password flow is still not feasible (documented in
  [`docs/upstream-changes.md`](docs/upstream-changes.md) and
  [`docs/api.md`](docs/api.md) §Auth). This change closes the "detects
  expiry but can't recover" half of #18 for anyone who has an out-of-band
  capture helper. A future step could bundle a documented capture helper.

[`pk_cli_auth::reauth`]: https://github.com/piekstra/cli-common/blob/main/crates/pk-cli-auth/src/reauth.rs

## 0.8.0

Prior releases are captured in the GitHub Releases history.
