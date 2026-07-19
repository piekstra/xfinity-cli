//! `xfin internet` — plan, devices, gateway status (from `context`).

use crate::cli::InternetCommand;
use crate::commands::Ctx;
use crate::error::AppError;
use crate::output;

pub fn run(ctx: &Ctx, cmd: &InternetCommand) -> Result<(), AppError> {
    let x = ctx.connect()?;
    match cmd {
        InternetCommand::Plan => output::internet_plan(&x.internet_plan()?),
        InternetCommand::Usage => output::internet_usage(&x.internet_plan()?),
        InternetCommand::Devices | InternetCommand::Status => {
            let dev = x.devices()?;
            output::devices(dev.get("equipment").unwrap_or(&dev));
        }
    }
    Ok(())
}
