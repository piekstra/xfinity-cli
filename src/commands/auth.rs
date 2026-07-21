//! `xfin auth` — session capture, status, and logout.
//!
//! Login here does not send a password. Xfinity's login page rejects
//! non-browser clients, so you log in once in a real browser and hand the
//! resulting `Authorization: Bearer` token to `xfin auth login`, which stores
//! it in the OS keychain. The token enters via `--stdin` or `--from-env` —
//! never a flag (that leaks into `ps` and shell history).

use serde_json::json;

use pk_cli_auth::{AuthMethod, AuthStatus};

use crate::cli::{AuthCommand, LoginArgs, RefreshArgs};
use crate::client::Xfinity;
use crate::commands::{prompt_username_if_needed, Ctx, SERVICE};
use crate::config::Config;
use crate::error::AppError;
use crate::output;
use crate::secrets::{self, CredentialStore, Secret};

pub fn run(ctx: &Ctx, cmd: &AuthCommand) -> Result<(), AppError> {
    match cmd {
        AuthCommand::Login(args) => login(ctx, args),
        AuthCommand::Refresh(args) => refresh(ctx, args),
        AuthCommand::Status { json } => status(ctx, *json || ctx.cli.json),
        AuthCommand::SetCredential(args) => crate::commands::set_credential::run(ctx, args),
        AuthCommand::Logout { forget } => logout(ctx, *forget),
    }
}

const REFRESH_ENV: &str = "XFINITY_REFRESH_COMMAND";

/// Resolve the refresh command from (in order): the `--command` flag, the
/// `$XFINITY_REFRESH_COMMAND` env var, then the saved config value.
fn resolve_refresh_command(
    flag: Option<&str>,
    env: Option<&str>,
    cfg: Option<&str>,
) -> Option<String> {
    flag.or(env)
        .or(cfg)
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn refresh(ctx: &Ctx, args: &RefreshArgs) -> Result<(), AppError> {
    let username = prompt_username_if_needed(ctx, true)?;

    let env = std::env::var(REFRESH_ENV).ok();
    let command = resolve_refresh_command(
        args.command.as_deref(),
        env.as_deref(),
        ctx.cfg.refresh_command.as_deref(),
    )
    .ok_or_else(|| {
        AppError::Usage(format!(
            "no refresh command configured — pass `--command <cmd>` (optionally with \
             `--save`), set ${REFRESH_ENV}, or add `refresh_command` to config. \
             The command must print a fresh `Authorization: Bearer …` token on stdout."
        ))
    })?;

    // Persist the command if asked (only meaningful with an explicit --command).
    if args.save {
        if args.command.is_none() {
            return Err(AppError::Usage(
                "--save requires --command <cmd> to persist".into(),
            ));
        }
        let mut cfg = Config::load()?;
        cfg.refresh_command = Some(command.clone());
        cfg.save()?;
        if !ctx.cli.quiet {
            eprintln!("saved refresh command to config");
        }
    }

    if ctx.verbose() {
        eprintln!("running refresh command: {command}");
    }
    let output = std::process::Command::new("sh")
        .arg("-c")
        .arg(&command)
        .output()
        .map_err(|e| AppError::Other(format!("failed to run refresh command: {e}")))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let hint = stderr.trim().lines().next_back().unwrap_or("").trim();
        return Err(AppError::Other(format!(
            "refresh command failed ({}){}",
            output.status,
            if hint.is_empty() {
                String::new()
            } else {
                format!(": {hint}")
            }
        )));
    }
    let token = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let secret = Secret::new(&token);
    if secret.is_empty() {
        return Err(AppError::Other(
            "refresh command produced no token on stdout".into(),
        ));
    }

    if !args.no_verify {
        Xfinity::from_session(&secret)?.account()?;
        if ctx.verbose() {
            eprintln!("refreshed token verified against www.xfinity.com/digital/service/api");
        }
    }

    CredentialStore::new(SERVICE).set(&username, &secret)?;

    if !ctx.cli.quiet {
        eprintln!("refreshed Xfinity session for {username} in the keychain");
    }
    if args.json || ctx.cli.json {
        output::json(&json!({
            "status": "refreshed",
            "username": username,
            "verified": !args.no_verify,
        }));
    }
    Ok(())
}

fn login(ctx: &Ctx, args: &LoginArgs) -> Result<(), AppError> {
    let username = prompt_username_if_needed(ctx, args.non_interactive)?;

    let secret = match (args.stdin, &args.from_env) {
        (true, Some(_)) => {
            return Err(AppError::Usage(
                "--stdin and --from-env are mutually exclusive".into(),
            ))
        }
        (true, None) => secrets::read_stdin()?,
        (false, Some(var)) => secrets::read_from_env(var)?,
        (false, None) => {
            return Err(AppError::Usage(
                "provide the browser session via --stdin or --from-env <VAR> \
                 (see `xfin auth login --help`)"
                    .into(),
            ))
        }
    };
    if secret.is_empty() {
        return Err(AppError::Usage("empty session — nothing stored".into()));
    }

    if !args.no_verify {
        let client = Xfinity::from_session(&secret)?;
        // A cheap authenticated read confirms the session is live before we
        // commit it to the keychain.
        client.account()?;
        if ctx.verbose() {
            eprintln!("token verified against www.xfinity.com/digital/service/api");
        }
    }

    let store = CredentialStore::new(SERVICE);
    let existed = crate::commands::get_session_migrating(&username)?.is_some();
    if existed && !args.overwrite {
        return Err(AppError::Usage(format!(
            "a session for {username:?} already exists — pass --overwrite to replace it"
        )));
    }
    store.set(&username, &secret)?;

    if ctx.cfg.username.as_deref() != Some(username.as_str()) {
        let mut cfg = Config::load()?;
        cfg.username = Some(username.clone());
        cfg.save()?;
    }

    if !ctx.cli.quiet {
        eprintln!("stored Xfinity session for {username} in the keychain");
    }
    if args.json || ctx.cli.json {
        output::json(&json!({
            "status": "stored",
            "username": username,
            "verified": !args.no_verify,
            "overwritten": existed,
        }));
    }
    Ok(())
}

fn status(ctx: &Ctx, json_flag: bool) -> Result<(), AppError> {
    let username = ctx
        .cli
        .username
        .clone()
        .or_else(|| ctx.cfg.username.clone());
    let has_session = match &username {
        Some(u) => crate::commands::get_session_migrating(u)?.is_some(),
        None => false,
    };
    let account = ctx.cli.account.clone().or_else(|| ctx.cfg.account.clone());

    let mut st = AuthStatus::new(true, has_session, AuthMethod::BrowserSession);
    st.username = username;
    st.account = account;
    st.credential_in_keychain = Some(has_session);
    st.emit(json_flag);
    Ok(())
}

fn logout(ctx: &Ctx, forget: bool) -> Result<(), AppError> {
    let mut removed = false;
    if let Some(u) = ctx
        .cli
        .username
        .clone()
        .or_else(|| ctx.cfg.username.clone())
    {
        removed = crate::commands::delete_session(&u)?;
    }
    if forget {
        Config::clear()?;
    }
    if !ctx.cli.quiet {
        eprintln!(
            "logged out{}",
            if removed {
                " and cleared stored session"
            } else {
                ""
            }
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::resolve_refresh_command;

    #[test]
    fn refresh_command_precedence_flag_env_config() {
        // flag wins over everything
        assert_eq!(
            resolve_refresh_command(Some("flag"), Some("env"), Some("cfg")).as_deref(),
            Some("flag")
        );
        // env beats config
        assert_eq!(
            resolve_refresh_command(None, Some("env"), Some("cfg")).as_deref(),
            Some("env")
        );
        // config is the last resort
        assert_eq!(
            resolve_refresh_command(None, None, Some("cfg")).as_deref(),
            Some("cfg")
        );
        // nothing configured
        assert_eq!(resolve_refresh_command(None, None, None), None);
        // blank/whitespace-only sources are ignored
        assert_eq!(
            resolve_refresh_command(Some("   "), None, Some("cfg")),
            None
        );
    }
}
