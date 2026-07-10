//! `xfin self-update` — replace the running binary with the latest GitHub
//! release. Thin wrapper over [`crate::selfupdate`].

use serde_json::json;

use crate::cli::SelfUpdateArgs;
use crate::commands::Ctx;
use crate::error::AppError;
use crate::output;
use crate::selfupdate;

pub fn run(ctx: &Ctx, args: &SelfUpdateArgs) -> Result<(), AppError> {
    let outcome = selfupdate::run(args.check)?;

    if args.json {
        output::json(&json!({
            "current": outcome.current,
            "latest": outcome.latest,
            "updated": outcome.updated,
            "already_current": outcome.already_current,
            "installed_at": outcome.installed_at,
        }));
        return Ok(());
    }

    if ctx.cli.quiet {
        return Ok(());
    }

    if outcome.already_current {
        eprintln!("xfin {} is already the latest release", outcome.current);
    } else if args.check {
        eprintln!(
            "update available: {} → {} (run `xfin self-update`)",
            outcome.current, outcome.latest
        );
    } else if outcome.updated {
        eprintln!(
            "updated xfin {} → {}{}",
            outcome.current,
            outcome.latest,
            outcome
                .installed_at
                .as_deref()
                .map(|p| format!(" ({p})"))
                .unwrap_or_default()
        );
    }
    Ok(())
}
