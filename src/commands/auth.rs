//! `xfin auth` — session capture, status, and logout.
//!
//! Login here does not send a password. Xfinity's login page rejects
//! non-browser clients, so you log in once in a real browser and hand the
//! resulting `Authorization: Bearer` token to `xfin auth login`, which stores
//! it in the OS keychain. The token enters via `--stdin` or `--from-env` —
//! never a flag (that leaks into `ps` and shell history).

use serde_json::json;

use pk_cli_auth::{token, AuthMethod, AuthStatus};

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

pub const REFRESH_ENV: &str = "XFINITY_REFRESH_COMMAND";

/// Resolve the refresh command from (in order): the `--command` flag, the
/// `$XFINITY_REFRESH_COMMAND` env var, then the saved config value.
pub fn resolve_refresh_command(
    flag: Option<&str>,
    env: Option<&str>,
    cfg: Option<&str>,
) -> Option<String> {
    flag.or(env)
        .or(cfg)
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Resolve the refresh command from `$XFINITY_REFRESH_COMMAND` and the saved
/// config (no `--command` flag), the two sources available to the on-401
/// auto-refresh path. Returns `None` when nothing is configured.
pub fn refresh_command_for_auto(cfg: &Config) -> Option<String> {
    let env = std::env::var(REFRESH_ENV).ok();
    resolve_refresh_command(None, env.as_deref(), cfg.refresh_command.as_deref())
}

/// Run a configured refresh command, verify the token, and store it in the
/// keychain — the shared machinery behind both the `xfin auth refresh`
/// command and the on-401 auto-refresh path.
///
/// `announce` gates the "refreshed session for …" stderr line. The verbose
/// echo is a separate concern the caller controls via `ctx.verbose()`.
pub fn run_refresh_command(
    ctx: &Ctx,
    username: &str,
    command: &str,
    verify: bool,
    announce: bool,
) -> Result<(), AppError> {
    if ctx.verbose() {
        eprintln!("running refresh command: {command}");
    }
    let output = std::process::Command::new("sh")
        .arg("-c")
        .arg(command)
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

    if verify {
        Xfinity::from_session(&secret)?.account()?;
        if ctx.verbose() {
            eprintln!("refreshed token verified against www.xfinity.com/digital/service/api");
        }
    }

    CredentialStore::new(SERVICE).set(username, &secret)?;

    if announce && !ctx.cli.quiet {
        eprintln!("refreshed Xfinity session for {username} in the keychain");
    }
    Ok(())
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

    run_refresh_command(ctx, &username, &command, !args.no_verify, true)?;

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
    let stored: Option<Secret> = match &username {
        Some(u) => crate::commands::get_session_migrating(u)?,
        None => None,
    };
    let has_session = stored.is_some();
    let account = ctx.cli.account.clone().or_else(|| ctx.cfg.account.clone());

    // If the stored token is a JWT (Xfinity's are, in practice), lift its
    // lifetime into `auth-status/v1`. Non-JWT / opaque tokens leave the
    // fields unset — `token::is_expired` treats an unreadable token as
    // "defer to the server" (see pk_cli_auth::token module docs), which is
    // the safe default.
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let expires_at = stored.as_ref().and_then(|s| token::expires_at(s.expose()));
    let session_valid = stored.as_ref().map(|s| {
        // A token with no readable `exp` reads as "not expired" from
        // `is_expired`, which matches "usable until proven otherwise".
        !token::is_expired(s.expose(), now, token::DEFAULT_SKEW_SECS)
    });

    let authenticated = has_session && session_valid != Some(false);

    let mut st = AuthStatus::new(true, authenticated, AuthMethod::BrowserSession);
    st.username = username;
    st.account = account;
    st.credential_in_keychain = Some(has_session);
    st.session_valid = session_valid;
    st.expires_at = expires_at;
    st.emit(json_flag);

    // Extra, non-DTO diagnostics — only meaningful outside JSON mode, where
    // the caller is reading with human eyes. In JSON mode we honour the
    // `auth-status/v1` schema shape and stay quiet.
    if !json_flag {
        let has_refresh = refresh_command_for_auto(ctx.cfg).is_some();
        println!(
            "Refresh:       {}",
            if has_refresh {
                if ctx.cfg.auto_refresh() {
                    "command configured (auto_refresh on)"
                } else {
                    "command configured (auto_refresh off)"
                }
            } else {
                "not configured — see `xfin auth refresh --help`"
            }
        );
    }
    Ok(())
}

fn logout(ctx: &Ctx, forget: bool) -> Result<(), AppError> {
    let name = ctx
        .cli
        .username
        .clone()
        .or_else(|| ctx.cfg.username.clone());
    let named = name.is_some();
    let session = match &name {
        Some(u) => crate::commands::delete_session(u),
        None => Ok(false),
    };
    if !forget {
        say_logged_out(session?, named, ctx.cli.quiet);
        return Ok(());
    }
    let token = CredentialStore::for_binary("xfin").delete(pk_cli_mcp::HTTP_TOKEN_ACCOUNT);
    // A retry needs the username to find the session, so a failed delete keeps the config.
    let config = if session.is_ok() {
        Config::clear()
    } else {
        Ok(false)
    };
    if let Ok(removed) = &session {
        say_logged_out(*removed, named, ctx.cli.quiet);
    }
    finish_forget(session.map(drop), token, config, ctx.cli.quiet)
}

/// A login can be gone while its pieces are not: an older build's `--forget` clears the name.
fn say_logged_out(removed: bool, named: bool, quiet: bool) {
    if !quiet {
        eprintln!(
            "logged out{}",
            if removed {
                " and cleared stored session"
            } else if named {
                ""
            } else {
                "; no username is known, so no stored session was looked for (--username names one)"
            }
        );
    }
}

/// The end of `auth logout --forget`. Nothing records whether `mcp --new-token` ran, so the token's
/// delete is ungated. Every step is attempted and every failure reported here: none hides another,
/// or the note that a removed token still needs, since a rerun finds no item.
fn finish_forget(
    session: Result<(), AppError>,
    token: Result<bool, AppError>,
    config: Result<bool, AppError>,
    quiet: bool,
) -> Result<(), AppError> {
    if let (Ok(removed), false) = (&token, quiet) {
        eprintln!("{}", pk_cli_mcp::forget_note("xfin", *removed));
    }
    match (session, token, config) {
        (Ok(()), Ok(_), Ok(_)) => Ok(()),
        (Ok(()), Ok(_), Err(e)) => Err(e),
        (session, token, config) => {
            let opening = if session.is_ok() {
                "logged out, but "
            } else {
                ""
            };
            let mut failed = Vec::new();
            if let Err(e) = session {
                failed.push(format!(
                    "the stored session could not be removed ({})",
                    e.message()
                ));
            }
            if let Err(e) = token {
                failed.push(format!(
                    "the connector token of `xfin mcp --http` could not be removed ({}) and would \
                     still open the connector",
                    e.message()
                ));
            }
            if let Err(e) = config {
                failed.push(format!("the config was not cleared ({})", e.message()));
            }
            Err(AppError::Keychain(format!(
                "{opening}{}; run `xfin auth logout --forget` again",
                failed.join(", and ")
            )))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::resolve_refresh_command;
    use crate::error::AppError;

    #[test]
    fn a_forget_that_could_not_remove_the_connector_token_says_so() {
        let denied = || AppError::Keychain("denied".into());
        let says = super::finish_forget(Ok(()), Err(denied()), Ok(true), true)
            .unwrap_err()
            .to_string();
        assert!(says.contains("logged out, but"), "{says}");
        assert!(says.contains("connector token"), "{says}");
        assert!(says.contains("`xfin auth logout --forget`"), "{says}");
        assert!(says.contains("denied"), "{says}");
        assert!(!says.contains("config"), "{says}");
    }

    #[test]
    fn no_failure_of_a_forget_hides_another() {
        let says = super::finish_forget(
            Err(AppError::Keychain("locked".into())),
            Err(AppError::Keychain("denied".into())),
            Err(AppError::Other("read-only".into())),
            true,
        )
        .unwrap_err()
        .to_string();
        for cause in ["locked", "denied", "read-only"] {
            assert!(says.contains(cause), "{says}");
        }
        assert!(!says.contains("logged out, but"), "{says}");
        let e = super::finish_forget(
            Ok(()),
            Ok(true),
            Err(AppError::Other("read-only".into())),
            true,
        )
        .unwrap_err();
        assert!(e.to_string().contains("read-only"), "{e}");
    }

    #[test]
    fn a_forget_with_a_token_or_without_one_finishes() {
        assert!(super::finish_forget(Ok(()), Ok(true), Ok(true), true).is_ok());
        assert!(super::finish_forget(Ok(()), Ok(false), Ok(false), false).is_ok());
    }

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
