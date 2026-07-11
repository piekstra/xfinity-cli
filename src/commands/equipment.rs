//! `xfin equipment` — pending equipment returns.

use crate::cli::EquipmentCommand;
use crate::commands::Ctx;
use crate::error::AppError;
use crate::output;

pub fn run(ctx: &Ctx, cmd: &EquipmentCommand) -> Result<(), AppError> {
    let x = ctx.connect()?;
    match cmd {
        EquipmentCommand::Returns => output::render(&x.equipment_returns()?),
    }
    Ok(())
}
