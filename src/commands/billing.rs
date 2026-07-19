//! `xfin billing` — balance/due summary, statement history (from `billingSummary`).

use serde_json::Value;

use pk_cli_utility::{Paged, RangeArgs, Statement};

use crate::cli::BillingCommand;
use crate::commands::Ctx;
use crate::error::AppError;
use crate::output;
use crate::profile;

pub fn run(ctx: &Ctx, cmd: &BillingCommand) -> Result<(), AppError> {
    // Short-circuit the not-yet-mapped command before any network/auth.
    if let BillingCommand::Statement { .. } = cmd {
        return Err(AppError::Other(
            "`billing statement <id>` isn't available yet on the new Xfinity account \
             experience — use `billing statements` — see docs/api.md"
                .into(),
        ));
    }
    // Usage errors (malformed --since/--until) also come before any network.
    if let BillingCommand::Statements(range) = cmd {
        range.validate()?;
    }
    let bbds = ctx.connect()?.bbds()?;
    match cmd {
        BillingCommand::Summary => output::billing_summary(&bbds),
        BillingCommand::DueDates => match bbds.get("dueDate").and_then(|v| v.as_str()) {
            Some(d) => println!("Due: {d}"),
            None => output::render(&bbds),
        },
        BillingCommand::Statements(range) => statements(ctx, &bbds, range),
        BillingCommand::Statement { .. } => unreachable!("handled above"),
    }
    Ok(())
}

/// `billing statements` — the utility/v1 `statement-list/v1` envelope with
/// `--json`; the provider-shaped text rendering otherwise (byte-identical to
/// the pre-profile output when no range flag is given).
fn statements(ctx: &Ctx, bbds: &Value, range: &RangeArgs) {
    let unfiltered = range.limit.is_none() && range.since.is_none() && range.until.is_none();
    if !ctx.cli.json && unfiltered {
        // The pre-profile text path, unchanged.
        output::render(bbds.get("statementDetails").unwrap_or(bbds));
        return;
    }
    let mut records: Vec<(Value, Statement)> = profile::statement_values(bbds)
        .into_iter()
        .enumerate()
        .map(|(i, raw)| {
            let dto = profile::statement_dto(&raw, i + 1);
            (raw, dto)
        })
        .collect();
    records.retain(|(_, s)| {
        profile::in_range(
            s.date.as_deref(),
            range.since.as_deref(),
            range.until.as_deref(),
        )
    });
    if let Some(n) = range.limit {
        records.truncate(n as usize);
    }
    if ctx.cli.json {
        let items: Vec<Statement> = records.into_iter().map(|(_, dto)| dto).collect();
        Paged::new("statement", items).emit(true);
    } else if records.is_empty() {
        println!("(none)");
    } else {
        for (raw, _) in &records {
            output::render(raw);
        }
    }
}
