//! On-disk config for xfinity-cli. Stores only non-secret preferences (the
//! default Xfinity username and, optionally, a default account number) so
//! day-to-day commands don't need `--username`/`--account` every time. The
//! session secret itself never lands here — it lives in the OS keychain (see
//! [`crate::secrets`]). Storage and pathing come from `pk-cli-config`
//! (`${XDG_CONFIG_HOME:-~/.config}/xfinity-cli/config.json`).

use pk_cli_config::ConfigStore;
use serde::{Deserialize, Serialize};

use crate::error::AppError;

/// Existing installs keep their config dir name.
const APP_DIR: &str = "xfinity-cli";

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Config {
    /// Default Xfinity login (email / username). Not a secret.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    /// Default Xfinity account number, if the user pinned one. Not a secret,
    /// but account-scoped, so we only write it when explicitly set.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account: Option<String>,
    /// Optional shell command whose stdout is a fresh `Authorization: Bearer`
    /// token, used by `xfin auth refresh`. Lets you plug in your own
    /// browser-automation helper without baking it into the CLI. Not a secret
    /// itself (it's a command line), so it lives in plain config.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refresh_command: Option<String>,
    /// Silently invoke the configured `refresh_command` when a read fails
    /// with 401/403 (session expired), retry once, and — if that succeeds —
    /// continue. Defaults to on.
    ///
    /// Only kicks in when a `refresh_command` (or `$XFINITY_REFRESH_COMMAND`)
    /// is set; otherwise there is nothing to invoke and the CLI still exits 3
    /// with the "capture a fresh token" message. Turn it off (`false`) to
    /// require an explicit `xfin auth refresh` between expiries — useful when
    /// a refresh helper is slow or interactive, or you want the old failure
    /// loud instead of a silent recovery.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto_refresh: Option<bool>,
}

fn store() -> ConfigStore {
    ConfigStore::new(APP_DIR)
}

impl Config {
    pub fn load() -> Result<Config, AppError> {
        store().load()
    }

    /// The resolved config file path (`xfin config path`).
    pub fn path() -> Result<std::path::PathBuf, AppError> {
        store().path()
    }

    pub fn save(&self) -> Result<(), AppError> {
        store().save(self)
    }

    /// Remove the config file entirely (used by `logout --forget`).
    pub fn clear() -> Result<bool, AppError> {
        store().clear()
    }

    /// Whether to silently invoke the refresh command on session expiry.
    /// Defaults to on; only meaningful when a `refresh_command` (or
    /// `$XFINITY_REFRESH_COMMAND`) is actually configured.
    pub fn auto_refresh(&self) -> bool {
        self.auto_refresh.unwrap_or(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_refresh_defaults_on_but_is_overridable() {
        assert!(Config::default().auto_refresh());
        let off = Config {
            auto_refresh: Some(false),
            ..Default::default()
        };
        assert!(!off.auto_refresh());
    }
}
