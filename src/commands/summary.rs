//! `xfin summary` / `xfin balance` — the utility/v1 profile entry points.
//! Both read the same `BBDS` payload `billing summary` uses; with `--json`
//! they emit the canonical `utility-summary/v1` DTO, in text they stay in the
//! familiar `billing summary` style.

use pk_cli_core::output::scalar;

use crate::commands::Ctx;
use crate::error::AppError;
use crate::output;
use crate::profile;

/// The account override in effect (`--account` / `$XFINITY_ACCOUNT` / config),
/// if any — the BBDS payload doesn't carry the account number.
fn account_override(ctx: &Ctx) -> Option<String> {
    ctx.cli
        .account
        .clone()
        .or_else(|| ctx.cfg.account.clone())
        .filter(|a| !a.is_empty())
}

pub fn summary(ctx: &Ctx) -> Result<(), AppError> {
    let bbds = ctx.connect()?.bbds()?;
    if ctx.cli.json {
        pk_cli_utility::emit(&profile::summary_dto(&bbds, account_override(ctx)), true);
    } else {
        output::billing_summary(&bbds);
    }
    Ok(())
}

/// Same DTO as `summary` — the profile's second entry point.
pub fn balance(ctx: &Ctx) -> Result<(), AppError> {
    let bbds = ctx.connect()?.bbds()?;
    if ctx.cli.json {
        pk_cli_utility::emit(&profile::summary_dto(&bbds, account_override(ctx)), true);
    } else {
        match bbds
            .pointer("/balance/balanceDue")
            .map(scalar)
            .filter(|s| !s.is_empty())
        {
            Some(bal) => println!("Balance:  ${bal}"),
            None => println!("Balance not available."),
        }
    }
    Ok(())
}
