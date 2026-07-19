//! `xfin internet` — plan, devices, gateway status (from `context`).

use crate::cli::InternetCommand;
use crate::commands::Ctx;
use crate::error::AppError;
use crate::output;

pub fn run(ctx: &Ctx, cmd: &InternetCommand) -> Result<(), AppError> {
    let x = ctx.connect()?;
    match cmd {
        InternetCommand::Plan => output::internet_plan(&x.internet_plan()?),
        InternetCommand::Usage { history } => {
            let net = x.internet_plan()?;
            if *history {
                output::internet_usage_history(&net);
            } else {
                output::internet_usage(&net);
            }
        }
        InternetCommand::Devices | InternetCommand::Status => {
            let dev = x.devices()?;
            output::devices(dev.get("equipment").unwrap_or(&dev));
        }
    }
    Ok(())
}
