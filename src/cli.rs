use clap::{Args, Parser, Subcommand};

/// Manage your Xfinity account from the command line.
///
/// Xfinity publishes no official API; this talks to the same
/// api.sc.xfinity.com self-care services the website and mobile app use.
/// Because Xfinity's login is behind bot protection that blocks non-browser
/// clients, this CLI replays a session you capture from a logged-in browser
/// rather than a password. Set it up with `xfin auth login`, which reads the
/// session from stdin or an env var — never a flag. The session lives only in
/// the OS keychain.
///
/// Output is human- and agent-friendly text by default; resource reads render
/// key/value blocks and pipe-delimited tables. For a raw JSON payload (handy
/// while Xfinity's response shapes are still being mapped), use `xfin api`.
#[derive(Parser, Debug)]
#[command(name = "xfin", version, about, long_about = None)]
pub struct Cli {
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

    /// Internet: data usage, plan/speeds, connected devices.
    #[command(subcommand)]
    Internet(InternetCommand),

    /// Raw authenticated request to any Xfinity endpoint (returns JSON).
    ///
    /// Example: `xfin api GET /session/csp/selfhelp/account/me`
    Api(ApiArgs),

    /// Update xfin to the latest release from GitHub.
    SelfUpdate(SelfUpdateArgs),
}

#[derive(Subcommand, Debug)]
pub enum AuthCommand {
    /// Store an Xfinity browser session in the keychain.
    ///
    /// Log in at https://www.xfinity.com in a browser, copy the `Cookie`
    /// request header sent to api.sc.xfinity.com (DevTools → Network), and
    /// pipe it in: `pbpaste | xfin auth login --stdin`. The session enters via
    /// `--stdin` or `--from-env <VAR>`; there is no session flag. See
    /// `docs/api.md` §Auth for the capture walkthrough.
    Login(LoginArgs),
    /// Show configured username, active account, and keychain state.
    Status {
        /// Emit as JSON.
        #[arg(long)]
        json: bool,
    },
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
    /// Show the account profile.
    Get,
}

#[derive(Subcommand, Debug)]
pub enum BillingCommand {
    /// Current balance, due date, and autopay/paperless status.
    Summary,
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
    /// Payment history.
    #[command(alias = "ls")]
    List,
    /// List saved payment methods.
    Methods,
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
}

#[derive(Args, Debug)]
pub struct ApiArgs {
    /// HTTP method: GET, POST, PUT, or DELETE.
    pub method: String,
    /// Path (leading slash, relative to the self-care host) or full URL.
    pub path: String,
    /// Request body as a JSON string (for POST/PUT).
    #[arg(long)]
    pub data: Option<String>,
}

#[derive(Args, Debug)]
pub struct SelfUpdateArgs {
    /// Report the latest available version without installing it.
    #[arg(long)]
    pub check: bool,
    /// Emit the result as JSON on stdout.
    #[arg(long)]
    pub json: bool,
}
