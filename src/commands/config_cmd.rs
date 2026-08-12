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
            let v = serde_json::to_value(ctx.cfg)
                .map_err(|e| AppError::Other(format!("serialize config: {e}")))?;
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
        "refresh_command" => cfg.refresh_command = value.map(String::from),
        "auto_refresh" => {
            cfg.auto_refresh = value.map(parse_bool).transpose()?;
        }
        other => {
            return Err(AppError::Usage(format!(
                "unknown config key `{other}` \
                 (known: username, account, refresh_command, auto_refresh)"
            )))
        }
    }
    Ok(())
}

/// Accept the same booleans `pmac`/`wabhoa` do for parity across the family.
fn parse_bool(v: &str) -> Result<bool, AppError> {
    match v.trim().to_ascii_lowercase().as_str() {
        "true" | "yes" | "on" | "1" => Ok(true),
        "false" | "no" | "off" | "0" => Ok(false),
        other => Err(AppError::Usage(format!(
            "expected a boolean (true/false), got {other:?}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::{apply_key, parse_bool};
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

    #[test]
    fn apply_key_handles_refresh_command_and_auto_refresh() {
        let mut cfg = Config::default();
        apply_key(&mut cfg, "refresh_command", Some("~/bin/xfin-token.sh")).unwrap();
        assert_eq!(cfg.refresh_command.as_deref(), Some("~/bin/xfin-token.sh"));
        apply_key(&mut cfg, "refresh_command", None).unwrap();
        assert_eq!(cfg.refresh_command, None);

        apply_key(&mut cfg, "auto_refresh", Some("false")).unwrap();
        assert_eq!(cfg.auto_refresh, Some(false));
        apply_key(&mut cfg, "auto_refresh", Some("on")).unwrap();
        assert_eq!(cfg.auto_refresh, Some(true));
        apply_key(&mut cfg, "auto_refresh", None).unwrap();
        assert_eq!(cfg.auto_refresh, None);

        // Bad boolean is a usage error, not a silent no-op.
        assert!(apply_key(&mut cfg, "auto_refresh", Some("maybe")).is_err());
    }

    #[test]
    fn parse_bool_accepts_the_family_aliases() {
        for t in ["true", "TRUE", "yes", "on", "1"] {
            assert!(parse_bool(t).unwrap(), "{t}");
        }
        for f in ["false", "FALSE", "no", "off", "0"] {
            assert!(!parse_bool(f).unwrap(), "{f}");
        }
        assert!(parse_bool("maybe").is_err());
        assert!(parse_bool("").is_err());
    }
}
