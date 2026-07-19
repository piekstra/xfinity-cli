//! `xfin config` — non-secret preferences (standard family surface:
//! `path` / `show` / `set` / `unset`). The session secret never lives here —
//! it stays in the OS keychain (see `xfin auth login` / `set-credential`).

use crate::cli::ConfigCommand;
use crate::commands::Ctx;
use crate::config::Config;
use crate::error::AppError;
use crate::output;

pub fn run(ctx: &Ctx, cmd: &ConfigCommand) -> Result<(), AppError> {
    match cmd {
        ConfigCommand::Path => {
            println!("{}", Config::path()?.display());
            Ok(())
        }
        ConfigCommand::Show => {
            let v = serde_json::to_value(ctx.cfg).unwrap_or_default();
            if ctx.cli.json {
                output::json(&v);
            } else {
                pk_cli_core::output::render(&v);
            }
            Ok(())
        }
        ConfigCommand::Set { key, value } => set(ctx, key, Some(value)),
        ConfigCommand::Unset { key } => set(ctx, key, None),
    }
}

/// Set (or, with `None`, clear) one key and persist. Loads fresh from disk so
/// transient CLI overrides (`--username`/`--account`) never get written back.
fn set(ctx: &Ctx, key: &str, value: Option<&str>) -> Result<(), AppError> {
    let mut cfg = Config::load()?;
    apply_key(&mut cfg, key, value)?;
    cfg.save()?;
    if !ctx.cli.quiet {
        eprintln!("{} {key}", if value.is_some() { "set" } else { "unset" });
    }
    Ok(())
}

/// Apply one key/value to a [`Config`] in memory. Pure (no IO) so it's
/// unit-testable. The session is intentionally not settable here — it belongs
/// in the keychain via `xfin auth login`.
fn apply_key(cfg: &mut Config, key: &str, value: Option<&str>) -> Result<(), AppError> {
    match key {
        "username" => cfg.username = value.map(String::from),
        "account" => cfg.account = value.map(String::from),
        other => {
            return Err(AppError::Usage(format!(
                "unknown config key `{other}` (known: username, account)"
            )))
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::apply_key;
    use crate::config::Config;

    #[test]
    fn apply_key_sets_clears_and_rejects_unknown() {
        let mut cfg = Config::default();
        apply_key(&mut cfg, "username", Some("user@example.com")).unwrap();
        assert_eq!(cfg.username.as_deref(), Some("user@example.com"));
        apply_key(&mut cfg, "account", Some("1234567890")).unwrap();
        assert_eq!(cfg.account.as_deref(), Some("1234567890"));
        apply_key(&mut cfg, "username", None).unwrap();
        assert_eq!(cfg.username, None);
        assert!(apply_key(&mut cfg, "session", Some("x")).is_err());
        assert!(apply_key(&mut cfg, "nope", None).is_err());
    }
}
