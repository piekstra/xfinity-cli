//! Command handlers, one module per resource group. Shared session/account
//! resolution and prompt helpers live here on [`Ctx`].

pub mod account;
pub mod api;
pub mod auth;
pub mod billing;
pub mod equipment;
pub mod internet;
pub mod outages;
pub mod payments;
pub mod self_update;
pub mod set_credential;

use std::io::{IsTerminal, Write};

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
            "account",
            "billing",
            "payments",
            "internet",
            "outages",
            "equipment",
            "api",
        ],
    );
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

    pub fn verbose(&self) -> bool {
        self.cli.verbose && !self.cli.quiet
    }
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
