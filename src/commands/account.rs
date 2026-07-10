//! `xfin account` — the signed-in customer's account profile.

use crate::cli::AccountCommand;
use crate::commands::Ctx;
use crate::error::AppError;
use crate::output;

pub fn run(ctx: &Ctx, cmd: &AccountCommand) -> Result<(), AppError> {
    match cmd {
        AccountCommand::Get => {
            let x = ctx.connect()?;
            output::render(&x.account()?);
        }
    }
    Ok(())
}
