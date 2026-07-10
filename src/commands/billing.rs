//! `xfin billing` — balance/due summary, statement history, statement detail.

use crate::cli::BillingCommand;
use crate::commands::Ctx;
use crate::error::AppError;
use crate::output;

pub fn run(ctx: &Ctx, cmd: &BillingCommand) -> Result<(), AppError> {
    let x = ctx.connect()?;
    match cmd {
        BillingCommand::Summary => output::billing_summary(&x.billing_summary()?),
        BillingCommand::DueDates => output::render(&x.due_dates()?),
        BillingCommand::Statements => output::render(&x.statements()?),
        BillingCommand::Statement { id } => output::render(&x.statement(id)?),
    }
    Ok(())
}
