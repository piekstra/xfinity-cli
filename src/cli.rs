use clap::{Args, Parser, Subcommand};
use clap_complete::Shell;
pub use pk_cli_selfupdate::SelfUpdateArgs;

/// Manage your Xfinity account from the command line.
///
/// Xfinity publishes no official API; this talks to the same
/// www.xfinity.com/digital/service/api services the new Xfinity account
/// experience uses. Because Xfinity's login is behind bot protection that
/// blocks non-browser clients, this CLI replays an `Authorization: Bearer`
/// token you capture from a logged-in browser rather than a password. Set it
/// up with `xfin auth login`, which reads the token from stdin or an env var —
/// never a flag. The token lives only in the OS keychain.
///
/// Output is human- and agent-friendly text by default; resource reads render
/// key/value blocks and pipe-delimited tables. For a raw JSON payload (handy
/// while Xfinity's response shapes are still being mapped), use `xfin api`.
#[derive(Parser, Debug)]
#[command(name = "xfin", version, about, long_about = None)]
pub struct Cli {
    /// Emit machine-readable JSON on stdout (diagnostics go to stderr).
    #[arg(long, global = true)]
    pub json: bool,

    /// Account number to act on. Overrides the active account and $XFINITY_ACCOUNT.
    #[arg(short = 'a', long, global = true, env = "XFINITY_ACCOUNT")]
    pub account: Option<String>,

    /// Xfinity login (email/username). Falls back to config, then $XFINITY_USERNAME.
    #[arg(long, global = true, env = "XFINITY_USERNAME")]
    pub username: Option<String>,

    /// Extra diagnostics on stderr (never secrets).
    #[arg(short, long, global = true)]
    pub verbose: bool,

    /// Suppress non-error stderr output.
    #[arg(short, long, global = true)]
    pub quiet: bool,

    /// Disable ANSI color (reserved; output is currently plain).
    #[arg(long, global = true)]
    pub no_color: bool,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Session and credential management (login, status, logout).
    #[command(subcommand)]
    Auth(AuthCommand),

    /// Write a single credential to the keychain (rotation / headless setup).
    ///
    /// Reads the secret from `--stdin` or `--from-env <VAR>` (exactly one).
    /// Refuses to replace an existing entry unless `--overwrite` is given.
    SetCredential(SetCredentialArgs),

    /// Account profile: holder, service address, account number, contact info.
    #[command(subcommand)]
    Account(AccountCommand),

    /// Billing: balance/due summary, statement history, statement detail.
    #[command(subcommand)]
    Billing(BillingCommand),

    /// Payments: history, saved methods, and making a payment.
    #[command(subcommand)]
    Payments(PaymentsCommand),

    /// Internet: data usage, plan/speeds, connected devices, gateway status.
    #[command(subcommand)]
    Internet(InternetCommand),

    /// Service outage status across internet/TV/voice/mobile.
    Outages,

    /// Equipment: pending returns.
    #[command(subcommand)]
    Equipment(EquipmentCommand),

    /// Raw authenticated request to a `digital/service/api` endpoint (POST-only).
    ///
    /// Example: `xfin api POST BillingInfo/context --data '{"eventNames":[...]}'`
    Api(ApiArgs),

    /// Update xfin to the latest release from GitHub.
    SelfUpdate(SelfUpdateArgs),

    /// Print a shell completion script (e.g. `xfin completions zsh`).
    Completions {
        /// Shell to generate completions for.
        #[arg(value_enum)]
        shell: Shell,
    },

    /// Machine-readable capability discovery (cli-info/v1).
    Info,
}

#[derive(Subcommand, Debug)]
pub enum AuthCommand {
    /// Store an Xfinity `Authorization: Bearer` token in the keychain.
    ///
    /// Log in at https://www.xfinity.com/account in a browser, open DevTools →
    /// Network, click any request to `digital/service/api/...`, copy its
    /// `Authorization: Bearer …` request header, and pipe it in:
    /// `pbpaste | xfin auth login --stdin`. The token enters via `--stdin` or
    /// `--from-env <VAR>`; there is no token flag. See `docs/api.md` §Auth.
    Login(LoginArgs),
    /// Show configured username, active account, and keychain state.
    Status {
        /// Emit as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Write a single credential to the keychain (rotation / headless setup).
    SetCredential(SetCredentialArgs),
    /// Remove the stored session from the keychain.
    Logout {
        /// Also clear the saved username/account from config.
        #[arg(long)]
        forget: bool,
    },
}

#[derive(Args, Debug)]
pub struct LoginArgs {
    /// Read the session from stdin.
    #[arg(long)]
    pub stdin: bool,
    /// Read the session from a named environment variable.
    #[arg(long, value_name = "VAR")]
    pub from_env: Option<String>,
    /// Replace an existing stored session instead of failing.
    #[arg(long)]
    pub overwrite: bool,
    /// Skip the live session check (store without verifying).
    #[arg(long)]
    pub no_verify: bool,
    /// Never prompt; fail if a required input is missing.
    #[arg(long)]
    pub non_interactive: bool,
    /// Emit the result as JSON on stdout (confirmation still goes to stderr).
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct SetCredentialArgs {
    /// Read the secret from stdin.
    #[arg(long)]
    pub stdin: bool,
    /// Read the secret from a named environment variable.
    #[arg(long, value_name = "VAR")]
    pub from_env: Option<String>,
    /// Replace an existing entry instead of failing.
    #[arg(long)]
    pub overwrite: bool,
    /// Emit the result as JSON on stdout (confirmation still goes to stderr).
    #[arg(long)]
    pub json: bool,
}

#[derive(Subcommand, Debug)]
pub enum AccountCommand {
    /// Show the account profile (name, contact, service address).
    Get,
    /// Print the default account number on this login.
    Number,
    /// List the users/contacts on the account.
    #[command(alias = "ls")]
    Users,
    /// Account locality / service info.
    Info,
    /// Two-factor / multi-factor auth enrollment status.
    Security,
}

#[derive(Subcommand, Debug)]
pub enum BillingCommand {
    /// Current balance, due date, and autopay status.
    Summary,
    /// Upcoming due date and the valid days you can schedule a payment for.
    DueDates,
    /// Prior statements (period, amount, status).
    #[command(alias = "ls")]
    Statements,
    /// Show one statement by id.
    Statement {
        /// Statement id (from `billing statements`).
        id: String,
    },
}

#[derive(Subcommand, Debug)]
pub enum PaymentsCommand {
    /// Store the separate `payments.xfinity.com` browser session.
    ///
    /// The payment surface is a separate app from the rest of My Account with
    /// its own session. Log in at <https://payments.xfinity.com>, copy the
    /// `Cookie` request header sent to `payments.xfinity.com/apis/...`
    /// (DevTools → Network), and pipe it in: `pbpaste | xfin payments login
    /// --stdin`. Enters via `--stdin` or `--from-env <VAR>` — never a flag.
    Login(LoginArgs),
    /// Remove the stored payments session.
    Logout,
    /// List saved payment methods (instruments).
    Methods,
    /// Scheduled (upcoming) payments.
    #[command(alias = "ls", alias = "list")]
    Scheduled,
    /// Autopay enrollment.
    Autopay,
    /// Make a payment. Prompts for confirmation unless `--force` is given.
    Create {
        /// Amount in dollars, e.g. 123.45.
        #[arg(long)]
        amount: String,
        /// Payment date as MM/DD/YYYY (default: today).
        #[arg(long)]
        date: Option<String>,
        /// Saved payment-method token id (from `payments methods`).
        #[arg(long)]
        method: Option<String>,
        /// Skip the confirmation prompt (submits the payment).
        #[arg(long)]
        force: bool,
    },
}

#[derive(Subcommand, Debug)]
pub enum InternetCommand {
    /// Current-cycle data usage (used/allowable GB, cycle dates).
    Usage,
    /// Subscribed plan (tier, download/upload speeds).
    Plan,
    /// Devices seen on the account gateway.
    #[command(alias = "ls")]
    Devices,
    /// Gateway/modem online status.
    Status,
}

#[derive(Subcommand, Debug)]
pub enum EquipmentCommand {
    /// Pending equipment returns.
    #[command(alias = "ls")]
    Returns,
}

#[derive(Args, Debug)]
pub struct ApiArgs {
    /// HTTP method — POST only on the account-experience surface.
    pub method: String,
    /// `digital/service/api` path (e.g. `BillingInfo/context`) or a full URL.
    pub path: String,
    /// Request body as a JSON string.
    #[arg(long)]
    pub data: Option<String>,
}
