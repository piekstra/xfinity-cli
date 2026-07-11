//! `xfin self-update` — replace the running binary with the latest GitHub
//! release, via the family updater (`pk-cli-selfupdate`).

use pk_cli_selfupdate::{os_arch, SelfUpdateArgs, Updater};

use crate::commands::Ctx;
use crate::error::AppError;

pub fn run(ctx: &Ctx, args: &SelfUpdateArgs) -> Result<(), AppError> {
    Updater {
        repo: "piekstra/xfinity-cli".into(),
        binary: "xfin".into(),
        target: os_arch(),
        current: env!("CARGO_PKG_VERSION").into(),
    }
    .run(args, ctx.cli.json, ctx.cli.quiet)
}
