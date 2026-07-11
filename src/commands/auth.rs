//! `xfin auth` — session capture, status, and logout.
//!
//! Login here does not send a password. Xfinity's login page rejects
//! non-browser clients, so you log in once in a real browser and hand the
//! resulting session (a `Cookie` header value) to `xfin auth login`, which
//! stores it in the OS keychain. The session enters via `--stdin` or
//! `--from-env` — never a flag (that leaks into `ps` and shell history).

use serde_json::json;

use pk_cli_auth::{AuthMethod, AuthStatus};

use crate::cli::{AuthCommand, LoginArgs};
use crate::client::Xfinity;
use crate::commands::{prompt_username_if_needed, Ctx, SERVICE};
use crate::config::Config;
use crate::error::AppError;
use crate::output;
use crate::secrets::{self, CredentialStore};

pub fn run(ctx: &Ctx, cmd: &AuthCommand) -> Result<(), AppError> {
    match cmd {
        AuthCommand::Login(args) => login(ctx, args),
        AuthCommand::Status { json } => status(ctx, *json || ctx.cli.json),
        AuthCommand::SetCredential(args) => crate::commands::set_credential::run(ctx, args),
        AuthCommand::Logout { forget } => logout(ctx, *forget),
    }
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
        client.default_account()?;
        if ctx.verbose() {
            eprintln!("session verified against customer.xfinity.com");
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
