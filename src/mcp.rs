//! `xfin mcp`: the shared `utility/v1` tools (`pk_cli_utility::mcp`) over
//! Xfinity.
//!
//! This module supplies what is Xfinity's own: [`Portal`], the reads the
//! commands already make, and [`XfinitySource`], which maps them with the
//! commands' mappers in `profile.rs`. Tests put fixtures behind [`Portal`], so
//! nothing here needs a token to test.
//!
//! The server reads the token and never writes one. So it does not use
//! `Ctx::read`, whose recovery runs the configured refresh command and stores
//! what that prints, nor `get_session_migrating`, which moves a legacy keychain
//! item: an MCP server must not run a sign-in or change the keychain.
//!
//! No posted-payments tool: BBDS's `transactionHistory` has never been captured
//! (docs/api.md names it, with no shape), and a mapping guessed at its fields
//! would answer "no payments" when the guess was wrong.
//!
//! The commands' mappers are best-effort, for a person who can see the raw
//! payload. A tool's answer goes to a model as fact, so here an answer Xfinity
//! did not give (a balance no field states, a list its answer no longer holds)
//! is an upstream error, never 0.00 or an empty list.

use pk_cli_mcp::Server;
use pk_cli_utility::mcp::{UtilityAccount, UtilitySource, UtilityTools};
use pk_cli_utility::{Payment, Statement, UsagePeriod, UtilitySummary};
use serde_json::Value;

use pk_cli_secrets::{KeychainRead, Resolved, SecretResolver, SecretSpec, SourceKind, SourceOrder};

use crate::cli::Cli;
use crate::client::Xfinity;
use crate::commands::{LEGACY_SERVICE, SERVICE};
use crate::config::Config;
use crate::error::AppError;
use crate::profile;
use crate::secrets::{CredentialStore, Secret};

/// The token's environment variable, read before the keychain.
const TOKEN_ENV: &str = "XFINITY_TOKEN";

/// What a person runs when the token is missing or has lapsed.
const SIGN_IN: &str = "xfin auth login --stdin --overwrite";

/// The tools `xfin mcp` serves. Nothing here reads the config: listing the tools
/// touches no file, and a server left running sees the sign-in the user does next.
pub fn tools(cli: &Cli) -> UtilityTools<XfinitySource<LivePortal>> {
    let token_in_env = std::env::var_os(TOKEN_ENV).is_some();
    with_source(
        XfinitySource {
            portal: LivePortal {
                username: cli.username.clone().filter(|u| !u.is_empty()),
            },
        },
        token_in_env,
    )
}

fn with_source<P: Portal>(
    source: XfinitySource<P>,
    token_in_env: bool,
) -> UtilityTools<XfinitySource<P>> {
    let mut server = Server::new("xfin", env!("CARGO_PKG_VERSION"), "Xfinity", "xfinity")
        .sign_in_command(SIGN_IN)
        .instructions(
            "Xfinity access here is read-only: no tool can make a payment or change anything. \
             Xfinity sign-in is a bearer token the user copies from a logged-in browser; it \
             expires, and a tool cannot renew it. Xfinity reports only the latest bill, so the \
             bills tool returns one. Usage is internet data by billing cycle, labelled with the \
             date the cycle started.",
        );
    // The keychain is not read while the variable is set, so the sign-in command
    // would store a token this server never sees.
    if token_in_env {
        server = server.hosted(
            "through the open-source `xfin` tool, on a token from the settings of the program \
             that started this server",
            "ask the user to replace or unset $XFINITY_TOKEN where it is set, then restart \
             this server",
        );
    }
    UtilityTools::new(server, source)
        .metering("internet data use")
        .without_payments()
        .single_account()
}

/// The Xfinity reads the tools need. The JSON is Xfinity's own, as `profile.rs`
/// maps it.
pub trait Portal {
    fn bbds(&self) -> Result<Value, AppError>;
    fn subscription(&self) -> Result<Value, AppError>;
}

/// `Portal` over xfinity.com, reading the config and the token on each call, so
/// a token stored in the keychain while the server runs takes effect at once.
pub struct LivePortal {
    /// `--username`, which wins over the config.
    username: Option<String>,
}

impl LivePortal {
    fn read(&self, op: impl Fn(&Xfinity) -> Result<Value, AppError>) -> Result<Value, AppError> {
        // A token in the variable needs no username, so a bad config file must
        // not cost it.
        let username = match &self.username {
            _ if std::env::var_os(TOKEN_ENV).is_some() => None,
            Some(username) => Some(username.clone()),
            None => Config::load()?.username,
        };
        let token = resolve_token(&Keychain, TOKEN_ENV, username.as_deref())?;
        op(&Xfinity::from_session(&token.secret)?).map_err(without_capture_steps)
    }
}

impl Portal for LivePortal {
    fn bbds(&self) -> Result<Value, AppError> {
        self.read(Xfinity::bbds)
    }

    fn subscription(&self) -> Result<Value, AppError> {
        self.read(Xfinity::subscription)
    }
}

/// The keychain as this server reads it: the current service, then a token an
/// older version stored, which stays where it is.
struct Keychain;

impl KeychainRead for Keychain {
    fn service(&self) -> &str {
        SERVICE
    }

    fn get(&self, account: &str) -> Result<Option<Secret>, AppError> {
        if let Some(secret) = CredentialStore::new(SERVICE).get(account)? {
            return Ok(Some(secret));
        }
        CredentialStore::new(LEGACY_SERVICE).get(account)
    }
}

/// The family's secret precedence, with the keychain left out when no username
/// names an item in it.
fn resolve_token(
    keychain: &impl KeychainRead,
    env: &str,
    username: Option<&str>,
) -> Result<Resolved, AppError> {
    let username = username.filter(|u| !u.is_empty());
    let spec = SecretSpec::new(username.unwrap_or_default())
        .env(env)
        .remedy(format!("`{SIGN_IN}`"));
    let resolver = SecretResolver::new(keychain);
    let found = match username {
        Some(_) => resolver.resolve(&spec),
        None => {
            let env_only = SourceOrder::new(&[SourceKind::Env]).map_err(AppError::Usage)?;
            resolver.order(env_only).resolve(&spec)
        }
    };
    // A variable set but empty is a usage error on a command line. Here no tool
    // argument can fix it, and the model would be told to try another.
    let found = found.map_err(|e| match e {
        AppError::Usage(message) => AppError::Auth(message),
        other => other,
    })?;
    found.ok_or_else(|| {
        AppError::Auth(match username {
            Some(username) => format!("no stored token for {username:?}"),
            None => "no Xfinity username is configured".into(),
        })
    })
}

/// A lapsed-token message for a terminal goes on to walk through capturing a
/// new one. The tool error's fix names the command instead.
fn without_capture_steps(e: AppError) -> AppError {
    match e {
        AppError::Auth(message) => AppError::Auth(
            message
                .split(" Capture a fresh")
                .next()
                .unwrap_or_default()
                .to_string(),
        ),
        other => other,
    }
}

fn shape_error(what: &str) -> AppError {
    AppError::Upstream(format!(
        "Xfinity's {what} did not have the expected shape, so it is not reported here"
    ))
}

/// [`UtilitySource`] over a [`Portal`].
pub struct XfinitySource<P> {
    portal: P,
}

impl<P: Portal> UtilitySource for XfinitySource<P> {
    /// Never called: the toolset is `single_account`.
    async fn accounts(&self) -> Result<Vec<UtilityAccount>, AppError> {
        Ok(Vec::new())
    }

    async fn summary(&self, _account: Option<&str>) -> Result<UtilitySummary, AppError> {
        let bbds = self.portal.bbds()?;
        if profile::money(bbds.pointer("/balance/balanceDue")).is_none() {
            return Err(shape_error("billing summary"));
        }
        // BBDS carries no account number, and with one account there is nothing
        // for a configured one to tell apart, so none leaves this machine.
        Ok(profile::summary_dto(&bbds, None))
    }

    async fn statements(&self, _account: Option<&str>) -> Result<Vec<Statement>, AppError> {
        let bbds = self.portal.bbds()?;
        // An explicit null is an account with no statement yet; a missing key is
        // an answer that changed.
        if bbds.get("statementDetails").is_none() {
            return Err(shape_error("billing summary"));
        }
        let rows = profile::statement_values(&bbds);
        let statements: Vec<Statement> = rows
            .iter()
            .enumerate()
            .filter(|(_, raw)| {
                profile::money(raw.get("statementBalance").or_else(|| raw.get("amount"))).is_some()
            })
            .map(|(i, raw)| profile::statement_dto(raw, i + 1))
            .collect();
        if !rows.is_empty() && statements.is_empty() {
            return Err(shape_error("billing summary"));
        }
        Ok(statements)
    }

    /// Never called: the toolset is `without_payments`.
    async fn payments(
        &self,
        _account: Option<&str>,
        _since: Option<&str>,
    ) -> Result<Vec<Payment>, AppError> {
        Ok(Vec::new())
    }

    async fn usage(&self, _account: Option<&str>) -> Result<Vec<UsagePeriod>, AppError> {
        let subscription = self.portal.subscription()?;
        // `internet` holding nothing is an account with no internet line. The key
        // missing, or the whole answer missing, is an answer that changed.
        let lines = subscription
            .pointer("/customerPlanInfo/internet")
            .ok_or_else(|| shape_error("internet plan"))?;
        let Some(net) = lines.get(0) else {
            return Ok(Vec::new());
        };
        let months = net
            .get("usageMonths")
            .and_then(Value::as_array)
            .ok_or_else(|| shape_error("internet plan"))?;
        let usage = profile::usage_periods(net);
        if !months.is_empty() && usage.is_empty() {
            return Err(shape_error("internet plan"));
        }
        Ok(usage)
    }
}

#[cfg(test)]
mod tests;
