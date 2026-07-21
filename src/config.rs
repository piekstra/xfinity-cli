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
}
