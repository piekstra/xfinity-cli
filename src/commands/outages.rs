//! `xfin outages` — consolidated service-outage status.

use crate::commands::Ctx;
use crate::error::AppError;
use crate::output;

pub fn run(ctx: &Ctx) -> Result<(), AppError> {
    let x = ctx.connect()?;
    output::render(&x.outages()?);
    Ok(())
}
