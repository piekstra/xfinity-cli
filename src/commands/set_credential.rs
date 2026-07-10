//! `xfin set-credential` — low-level single-secret keyring write for rotation
//! and headless provisioning. Secret enters via `--stdin` or `--from-env`
//! (exactly one); an existing entry is replaced only with `--overwrite`.

use serde_json::json;

use crate::cli::SetCredentialArgs;
use crate::commands::{Ctx, SERVICE};
use crate::config::Config;
use crate::error::AppError;
use crate::output;
use crate::secrets::{self, CredentialStore};

pub fn run(ctx: &Ctx, args: &SetCredentialArgs) -> Result<(), AppError> {
    let username = ctx.resolve_username()?;

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
                "provide the secret via --stdin or --from-env <VAR>".into(),
            ))
        }
    };
    if secret.is_empty() {
        return Err(AppError::Usage("empty secret — nothing stored".into()));
    }

    let store = CredentialStore::new(SERVICE);
    let existed = store.get(&username)?.is_some();
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
        eprintln!("stored session for {username} in the keychain");
    }
    if args.json {
        output::json(&json!({
            "status": "stored",
            "username": username,
            "overwritten": existed,
        }));
    }
    Ok(())
}
