//! `xfin account` — profile, account number, users (from `context`).

use crate::cli::AccountCommand;
use crate::commands::Ctx;
use crate::error::AppError;
use crate::output;

pub fn run(ctx: &Ctx, cmd: &AccountCommand) -> Result<(), AppError> {
    let x = ctx.connect()?;
    let acct = x.account()?;
    match cmd {
        AccountCommand::Get => output::account(&acct),
        AccountCommand::Number => match acct.get("accountNumber").and_then(|v| v.as_str()) {
            Some(n) => println!("{n}"),
            None => output::render(&acct),
        },
        AccountCommand::Users => output::render(acct.get("users").unwrap_or(&acct)),
        AccountCommand::Info => output::render(&acct),
        AccountCommand::Security => {
            return Err(AppError::Other(
                "`account security` isn't available yet on the new Xfinity account experience \
                 — see docs/api.md"
                    .into(),
            ))
        }
    }
    Ok(())
}
