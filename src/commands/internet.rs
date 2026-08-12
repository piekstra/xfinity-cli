//! `xfin internet` — plan, devices, gateway status (from `context`).

use crate::cli::InternetCommand;
use crate::commands::Ctx;
use crate::error::AppError;
use crate::output;

pub fn run(ctx: &Ctx, cmd: &InternetCommand) -> Result<(), AppError> {
    match cmd {
        InternetCommand::Plan => output::internet_plan(&ctx.read(|x| x.internet_plan())?),
        InternetCommand::Usage { history } => {
            let net = ctx.read(|x| x.internet_plan())?;
            if *history {
                output::internet_usage_history(&net);
            } else {
                output::internet_usage(&net);
            }
        }
        InternetCommand::Devices | InternetCommand::Status => {
            let dev = ctx.read(|x| x.devices())?;
            output::devices(dev.get("equipment").unwrap_or(&dev));
        }
    }
    Ok(())
}
