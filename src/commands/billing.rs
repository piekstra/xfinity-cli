//! `xfin billing` — balance/due summary, statement history (from `billingSummary`).

use crate::cli::BillingCommand;
use crate::commands::Ctx;
use crate::error::AppError;
use crate::output;

pub fn run(ctx: &Ctx, cmd: &BillingCommand) -> Result<(), AppError> {
    // Short-circuit the not-yet-mapped command before any network/auth.
    if let BillingCommand::Statement { .. } = cmd {
        return Err(AppError::Other(
            "`billing statement <id>` isn't available yet on the new Xfinity account \
             experience — use `billing statements` — see docs/api.md"
                .into(),
        ));
    }
    let bbds = ctx.connect()?.bbds()?;
    match cmd {
        BillingCommand::Summary => output::billing_summary(&bbds),
        BillingCommand::DueDates => match bbds.get("dueDate").and_then(|v| v.as_str()) {
            Some(d) => println!("Due: {d}"),
            None => output::render(&bbds),
        },
        BillingCommand::Statements => output::render(bbds.get("statementDetails").unwrap_or(&bbds)),
        BillingCommand::Statement { .. } => unreachable!("handled above"),
    }
    Ok(())
}
