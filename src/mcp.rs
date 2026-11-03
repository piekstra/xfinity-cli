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

use pk_cli_mcp::Server;
use pk_cli_utility::mcp::{UtilityAccount, UtilitySource, UtilityTools};
use pk_cli_utility::{Payment, Statement, UsagePeriod, UtilitySummary};
use serde_json::Value;

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

/// The tools `xfin mcp` serves.
pub fn tools(cli: &Cli, cfg: &Config) -> UtilityTools<XfinitySource<LivePortal>> {
    with_source(XfinitySource {
        portal: LivePortal {
            username: cli.username.clone().or_else(|| cfg.username.clone()),
        },
        account: cli
            .account
            .clone()
            .or_else(|| cfg.account.clone())
            .filter(|a| !a.is_empty()),
    })
}

fn with_source<P: Portal>(source: XfinitySource<P>) -> UtilityTools<XfinitySource<P>> {
    let server = Server::new("xfin", env!("CARGO_PKG_VERSION"), "Xfinity", "xfinity")
        .sign_in_command(SIGN_IN)
        .instructions(
            "Xfinity access here is read-only: no tool can make a payment or change anything. \
             Xfinity sign-in is a bearer token the user copies from a logged-in browser and \
             pipes to that command; it expires, and a tool cannot renew it. Xfinity reports \
             only the latest bill, so the bills tool returns one. Usage is internet data by \
             billing cycle, labelled with the date the cycle started.",
        );
    UtilityTools::new(server, source)
        .metering("internet data use")
        .without_payments()
        .single_account()
}

/// The Xfinity reads the tools need. The JSON is Xfinity's own, as `profile.rs`
/// maps it.
pub trait Portal {
    fn bbds(&self) -> Result<Value, AppError>;
    fn internet_plan(&self) -> Result<Value, AppError>;
}

/// `Portal` over xfinity.com, reading the token on each call, so a token the
/// user renews while the server runs takes effect at once.
pub struct LivePortal {
    username: Option<String>,
}

impl LivePortal {
    fn read(&self, op: impl Fn(&Xfinity) -> Result<Value, AppError>) -> Result<Value, AppError> {
        let token = token_from(std::env::var(TOKEN_ENV).ok(), || {
            read_keychain(self.username.as_deref())
        })?;
        op(&Xfinity::from_session(&token)?).map_err(without_capture_steps)
    }
}

impl Portal for LivePortal {
    fn bbds(&self) -> Result<Value, AppError> {
        self.read(Xfinity::bbds)
    }

    fn internet_plan(&self) -> Result<Value, AppError> {
        self.read(Xfinity::internet_plan)
    }
}

/// The environment variable when it holds something, else the keychain.
fn token_from(
    env: Option<String>,
    keychain: impl FnOnce() -> Result<Secret, AppError>,
) -> Result<Secret, AppError> {
    match env.filter(|v| !v.is_empty()) {
        Some(value) => Ok(Secret::new(value)),
        None => keychain(),
    }
}

fn read_keychain(username: Option<&str>) -> Result<Secret, AppError> {
    let username = username
        .filter(|u| !u.is_empty())
        .ok_or_else(|| AppError::Auth("no Xfinity username is configured".into()))?;
    if let Some(secret) = CredentialStore::new(SERVICE).get(username)? {
        return Ok(secret);
    }
    CredentialStore::new(LEGACY_SERVICE)
        .get(username)?
        .ok_or_else(|| AppError::Auth(format!("no stored token for {username:?}")))
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

/// [`UtilitySource`] over a [`Portal`].
pub struct XfinitySource<P> {
    portal: P,
    /// `--account`, `$XFINITY_ACCOUNT`, or the config's: BBDS carries no number.
    account: Option<String>,
}

impl<P: Portal> UtilitySource for XfinitySource<P> {
    /// Never called: the toolset is `single_account`.
    async fn accounts(&self) -> Result<Vec<UtilityAccount>, AppError> {
        Ok(Vec::new())
    }

    async fn summary(&self, _account: Option<&str>) -> Result<UtilitySummary, AppError> {
        let bbds = self.portal.bbds()?;
        Ok(profile::summary_dto(&bbds, self.account.clone()))
    }

    async fn statements(&self, _account: Option<&str>) -> Result<Vec<Statement>, AppError> {
        let bbds = self.portal.bbds()?;
        Ok(profile::statement_values(&bbds)
            .iter()
            .enumerate()
            .map(|(i, raw)| profile::statement_dto(raw, i + 1))
            .collect())
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
        let net = self.portal.internet_plan()?;
        Ok(profile::usage_periods(&net))
    }
}

#[cfg(test)]
mod tests;
