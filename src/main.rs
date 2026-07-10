mod cli;
mod client;
mod commands;
mod config;
mod dates;
mod error;
mod output;
mod secrets;
mod selfupdate;

use clap::Parser;

use cli::{Cli, Command};
use commands::Ctx;
use config::Config;
use error::AppError;

fn main() {
    let cli = Cli::parse();
    let quiet = cli.quiet;
    if let Err(e) = run(cli) {
        if !quiet {
            eprintln!("error: {e}");
        }
        std::process::exit(e.exit_code());
    }
}

fn run(cli: Cli) -> Result<(), AppError> {
    let cfg = Config::load()?;
    let ctx = Ctx {
        cli: &cli,
        cfg: &cfg,
    };
    match &cli.command {
        Command::Auth(cmd) => commands::auth::run(&ctx, cmd),
        Command::SetCredential(args) => commands::set_credential::run(&ctx, args),
        Command::Account(cmd) => commands::account::run(&ctx, cmd),
        Command::Billing(cmd) => commands::billing::run(&ctx, cmd),
        Command::Payments(cmd) => commands::payments::run(&ctx, cmd),
        Command::Internet(cmd) => commands::internet::run(&ctx, cmd),
        Command::Api(args) => commands::api::run(&ctx, args),
        Command::SelfUpdate(args) => commands::self_update::run(&ctx, args),
    }
}
