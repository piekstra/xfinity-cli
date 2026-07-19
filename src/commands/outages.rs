//! `xfin outages` — service outage status (from `context` → `outageContext`).

use crate::commands::Ctx;
use crate::error::AppError;
use crate::output;

pub fn run(ctx: &Ctx) -> Result<(), AppError> {
    let x = ctx.connect()?;
    output::outages(&x.outages()?);
    Ok(())
}
