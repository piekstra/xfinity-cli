//! `xfin internet` — data usage, plan/speeds, connected devices.

use crate::cli::InternetCommand;
use crate::commands::Ctx;
use crate::error::AppError;
use crate::output;

pub fn run(ctx: &Ctx, cmd: &InternetCommand) -> Result<(), AppError> {
    let x = ctx.connect()?;
    match cmd {
        InternetCommand::Usage => output::render(&x.internet_usage()?),
        InternetCommand::Plan => output::render(&x.internet_plan()?),
        InternetCommand::Devices => output::render(&x.internet_devices()?),
        InternetCommand::Status => output::render(&x.devices_status()?),
    }
    Ok(())
}
