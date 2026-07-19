//! `xfin equipment` — pending returns.

use crate::cli::EquipmentCommand;
use crate::commands::Ctx;
use crate::error::AppError;

pub fn run(_ctx: &Ctx, cmd: &EquipmentCommand) -> Result<(), AppError> {
    match cmd {
        EquipmentCommand::Returns => Err(AppError::Other(
            "`equipment returns` isn't available yet on the new Xfinity account experience \
             — see docs/api.md"
                .into(),
        )),
    }
}
