//! `xfin account` — the signed-in customer's account profile.

use crate::cli::AccountCommand;
use crate::commands::Ctx;
use crate::error::AppError;
use crate::output;

pub fn run(ctx: &Ctx, cmd: &AccountCommand) -> Result<(), AppError> {
    let x = ctx.connect()?;
    match cmd {
        AccountCommand::Get => output::account(&x.account()?),
        AccountCommand::Number => {
            let v = x.default_account()?;
            match v.get("default_account").and_then(|a| a.as_str()) {
                Some(a) => println!("{a}"),
                None => output::render(&v),
            }
        }
        AccountCommand::Users => output::render(&x.users()?),
    }
    Ok(())
}
