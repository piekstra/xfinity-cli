//! Command handlers, one module per resource group. Shared session/account
//! resolution and prompt helpers live here on [`Ctx`].

pub mod account;
pub mod api;
pub mod auth;
pub mod billing;
pub mod config_cmd;
pub mod equipment;
pub mod internet;
pub mod outages;
pub mod payments;
pub mod self_update;
pub mod set_credential;
pub mod summary;

use std::io::{IsTerminal, Write};

use pk_cli_auth::reauth::with_reauth;

use crate::cli::Cli;
use crate::client::Xfinity;
use crate::config::Config;
use crate::error::AppError;
use crate::secrets::CredentialStore;

/// Family keychain service name (SPEC v1: `piekstra.<bin>`).
pub const SERVICE: &str = "piekstra.xfin";
/// Pre-cli-common service name; entries are migrated on first read.
const LEGACY_SERVICE: &str = "xfinity-cli";

/// Read a stored session, transparently migrating a legacy-service entry to
/// the family service name on first use.
pub fn get_session_migrating(username: &str) -> Result<Option<crate::secrets::Secret>, AppError> {
    let store = CredentialStore::new(SERVICE);
    if let Some(s) = store.get(username)? {
        return Ok(Some(s));
    }
    let legacy = CredentialStore::new(LEGACY_SERVICE);
    if let Some(s) = legacy.get(username)? {
        store.set(username, &s)?;
        let _ = legacy.delete(username);
        return Ok(Some(s));
    }
    Ok(None)
}

/// Delete a stored session from both the family and legacy service names.
pub fn delete_session(username: &str) -> Result<bool, AppError> {
    let a = CredentialStore::new(SERVICE).delete(username)?;
    let b = CredentialStore::new(LEGACY_SERVICE).delete(username)?;
    Ok(a || b)
}

/// `xfin info` — cli-info/v1 capability discovery.
pub fn info(_ctx: &Ctx) -> Result<(), AppError> {
    use pk_cli_core::info::{AuthInfo, CliInfo};
    let info = CliInfo::new(
        "xfin",
        env!("CARGO_PKG_VERSION"),
        "https://github.com/piekstra/xfinity-cli",
        AuthInfo {
            required: true,
            method: "browser-session".into(),
            login_hint: Some("xfin auth login".into()),
        },
        &[
            "summary",
            "balance",
            "account",
            "billing",
            "payments",
            "internet",
            "outages",
            "equipment",
            "api",
        ],
    )
    .with_profiles(&[pk_cli_utility::PROFILE]);
    crate::output::json(&serde_json::to_value(&info).unwrap_or_default());
    Ok(())
}

/// Per-invocation context threaded to every command handler.
pub struct Ctx<'a> {
    pub cli: &'a Cli,
    pub cfg: &'a Config,
}

impl Ctx<'_> {
    pub fn resolve_username(&self) -> Result<String, AppError> {
        if let Some(u) = self.cli.username.clone().filter(|s| !s.is_empty()) {
            return Ok(u);
        }
        if let Some(u) = self.cfg.username.clone().filter(|s| !s.is_empty()) {
            return Ok(u);
        }
        Err(AppError::Auth(
            "no Xfinity username configured — run `xfin auth login` \
             (or pass --username / set $XFINITY_USERNAME)"
                .into(),
        ))
    }

    /// Open an authenticated session. Runtime secrets come only from the
    /// keychain; `xfin auth login` / `xfin set-credential` are how they get
    /// there.
    pub fn connect(&self) -> Result<Xfinity, AppError> {
        let username = self.resolve_username()?;
        let secret = get_session_migrating(&username)?.ok_or_else(|| {
            AppError::Auth(format!(
                "no stored session for {username:?} — run `xfin auth login`"
            ))
        })?;
        if self.cli.verbose && !self.cli.quiet {
            eprintln!("using stored Xfinity session for {username}");
        }
        Xfinity::from_session(&secret)
    }

    /// Run a read against the Xfinity API, silently re-capturing the token
    /// once with the configured refresh command if the session has lapsed.
    ///
    /// Reads only — the retry rails live in `pk_cli_auth::reauth`, and `op`
    /// runs twice on the recovery path (so it must be safe to run twice; API
    /// reads that only fetch data trivially are). A fresh `Xfinity` is built
    /// per attempt so the retry picks up the token the refresh command just
    /// wrote to the keychain.
    ///
    /// When `auto_refresh` is off, or nothing is configured to refresh with,
    /// the 401 comes back unchanged — same exit code and message as before,
    /// so nothing gets papered over silently.
    pub fn read<T>(&self, op: impl Fn(&Xfinity) -> Result<T, AppError>) -> Result<T, AppError> {
        with_reauth(|| op(&self.connect()?), || self.auto_refresh())
    }

    /// The `reauth` closure fed to `with_reauth` — mint a fresh session via
    /// the configured refresh command. Kept a separate method (rather than an
    /// inline closure) so its policy is easy to reach for testing and the
    /// error paths read straight through.
    fn auto_refresh(&self) -> Result<(), AppError> {
        let username = self.resolve_username()?;
        let command = crate::commands::auth::refresh_command_for_auto(self.cfg);
        if let Some(reason) = auto_refresh_blocked(self.cfg.auto_refresh(), command.as_deref()) {
            return Err(AppError::Auth(reason));
        }
        let command = command.expect("checked");
        if !self.cli.quiet {
            eprintln!("session expired — refreshing via configured helper");
        }
        crate::commands::auth::run_refresh_command(self, &username, &command, false, false)
    }

    pub fn verbose(&self) -> bool {
        self.cli.verbose && !self.cli.quiet
    }
}

/// Why silent recovery from a 401 can't proceed, if it can't.
///
/// Split from the action so the policy is testable without a keychain or a
/// child process, and so every refusal names the one command that fixes it.
/// The message is deliberately close to the one the user would have seen from
/// the original 401 — turning `auto_refresh` on isn't the fix here, capturing
/// a fresh token (or configuring a refresh helper) is.
pub fn auto_refresh_blocked(auto_refresh: bool, refresh_command: Option<&str>) -> Option<String> {
    if !auto_refresh {
        return Some(
            "Xfinity session expired — capture a fresh `Authorization: Bearer …` in your \
             browser and re-run `xfin auth login --overwrite`, or run `xfin auth refresh` \
             (auto_refresh is off)"
                .into(),
        );
    }
    if refresh_command.is_none() {
        return Some(
            "Xfinity session expired — capture a fresh `Authorization: Bearer …` in your \
             browser and re-run `xfin auth login --overwrite`, or configure a refresh \
             command (`xfin auth refresh --help`) for silent recovery"
                .into(),
        );
    }
    None
}

/// Resolve the username for a setup command: explicit/config first, else prompt
/// on a TTY (unless `--non-interactive`).
pub fn prompt_username_if_needed(ctx: &Ctx, non_interactive: bool) -> Result<String, AppError> {
    if let Ok(u) = ctx.resolve_username() {
        return Ok(u);
    }
    if non_interactive || !stdin_is_tty() {
        return Err(AppError::Usage(
            "no username — pass --username, set $XFINITY_USERNAME, or run interactively".into(),
        ));
    }
    prompt_line("Xfinity username (email)")
}

/// Prompt for one line on a TTY (non-secret input, e.g. a username).
pub fn prompt_line(label: &str) -> Result<String, AppError> {
    eprint!("{label}: ");
    std::io::stderr().flush().ok();
    let mut s = String::new();
    std::io::stdin()
        .read_line(&mut s)
        .map_err(|e| AppError::Other(format!("reading input: {e}")))?;
    let s = s.trim().to_string();
    if s.is_empty() {
        return Err(AppError::Usage(format!("{label} cannot be empty")));
    }
    Ok(s)
}

pub fn stdin_is_tty() -> bool {
    std::io::stdin().is_terminal()
}

#[cfg(test)]
mod tests {
    use super::auto_refresh_blocked;

    #[test]
    fn auto_refresh_proceeds_when_command_configured_and_enabled() {
        assert_eq!(
            auto_refresh_blocked(true, Some("~/bin/xfin-token.sh")),
            None
        );
    }

    #[test]
    fn auto_refresh_declines_when_switched_off_even_with_command() {
        let why = auto_refresh_blocked(false, Some("~/bin/xfin-token.sh")).expect("blocked");
        assert!(why.contains("auto_refresh is off"), "{why}");
        // Even when declining, the user is told the two paths that fix it.
        assert!(why.contains("xfin auth login --overwrite"), "{why}");
        assert!(why.contains("xfin auth refresh"), "{why}");
    }

    /// The other blocker: nothing to run. Distinct message so the user knows
    /// they need to *configure* a helper, not toggle a flag.
    #[test]
    fn auto_refresh_declines_when_no_command_configured() {
        let why = auto_refresh_blocked(true, None).expect("blocked");
        assert!(why.contains("xfin auth refresh --help"), "{why}");
        assert!(why.contains("xfin auth login --overwrite"), "{why}");
    }

    /// Off wins over missing, so flipping it off is a real switch rather than
    /// a preference the "not configured" branch can silently override.
    #[test]
    fn off_takes_precedence_over_missing_command() {
        let why = auto_refresh_blocked(false, None).expect("blocked");
        assert!(why.contains("auto_refresh is off"), "{why}");
    }
}
