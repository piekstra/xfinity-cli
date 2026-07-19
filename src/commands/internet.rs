//! `xfin internet` — plan, devices, gateway status (from `context`).

use crate::cli::InternetCommand;
use crate::commands::Ctx;
use crate::error::AppError;
use crate::output;

pub fn run(ctx: &Ctx, cmd: &InternetCommand) -> Result<(), AppError> {
    let x = ctx.connect()?;
    match cmd {
        InternetCommand::Plan => {
            let acct = x.account()?;
            let plan = acct.pointer("/services/INTERNET").cloned().unwrap_or(acct);
            output::render(&plan);
        }
        InternetCommand::Devices | InternetCommand::Status => {
            let dev = x.devices()?;
            output::devices(dev.get("equipment").unwrap_or(&dev));
        }
        InternetCommand::Usage => {
            return Err(AppError::Other(
                "`internet usage` isn't available yet on the new Xfinity account experience \
                 — see docs/api.md"
                    .into(),
            ))
        }
    }
    Ok(())
}
