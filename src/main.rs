mod cli;
mod client;
mod commands;
mod config;
mod error;
mod output;
mod secrets;

use clap::Parser;

use cli::{Cli, Command};
use commands::Ctx;
use config::Config;
use error::AppError;

fn main() {
    let cli = Cli::parse();
    let json_mode = cli.json;
    if let Err(e) = run(cli) {
        std::process::exit(output::fail(&e, json_mode));
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
        Command::Outages => commands::outages::run(&ctx),
        Command::Equipment(cmd) => commands::equipment::run(&ctx, cmd),
        Command::Api(args) => commands::api::run(&ctx, args),
        Command::SelfUpdate(args) => commands::self_update::run(&ctx, args),
        Command::Completions { shell } => {
            use clap::CommandFactory;
            clap_complete::generate(*shell, &mut Cli::command(), "xfin", &mut std::io::stdout());
            Ok(())
        }
        Command::Info => commands::info(&ctx),
    }
}
