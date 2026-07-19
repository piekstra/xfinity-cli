//! `xfin payments` — scheduled payments (from `billingSummary`).
//!
//! The old payments app (`payments.xfinity.com`, separate OAuth) and one-time
//! payment submission aren't mapped to the new account experience yet, so
//! `methods`/`autopay`/`create`/`login`/`logout` return a clear error. Scheduled
//! payments are read from the billing summary.

use crate::cli::PaymentsCommand;
use crate::commands::Ctx;
use crate::error::AppError;
use crate::output;

const UNMAPPED: &str =
    "isn't available yet on the new Xfinity account experience — see docs/api.md";

pub fn run(ctx: &Ctx, cmd: &PaymentsCommand) -> Result<(), AppError> {
    match cmd {
        PaymentsCommand::Scheduled => {
            let bbds = ctx.connect()?.bbds()?;
            output::render(bbds.get("schedulePayments").unwrap_or(&bbds));
            Ok(())
        }
        PaymentsCommand::Login(_) => Err(AppError::Other(format!("`payments login` {UNMAPPED}"))),
        PaymentsCommand::Logout => Err(AppError::Other(format!("`payments logout` {UNMAPPED}"))),
        PaymentsCommand::Methods => Err(AppError::Other(format!("`payments methods` {UNMAPPED}"))),
        PaymentsCommand::Autopay => Err(AppError::Other(format!("`payments autopay` {UNMAPPED}"))),
        PaymentsCommand::Create { .. } => {
            Err(AppError::Other(format!("`payments create` {UNMAPPED}")))
        }
    }
}
