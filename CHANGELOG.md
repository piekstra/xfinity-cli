# Changelog

All notable changes to `xfinity-cli` are documented here. Newest first. The
project follows [SemVer](https://semver.org/); breaking changes bump the major
once we cut 1.0 — until then, minors carry the breaking-change signal.

## Unreleased

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
