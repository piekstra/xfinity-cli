//! `xfin payments` — saved methods, scheduled payments, autopay, and making a
//! payment. These target the separate `payments.xfinity.com` app, which has its
//! own session captured via `xfin payments login`.
//!
//! `payments create` moves real money: a non-reversible mutation, so it
//! confirms by default and skips only with `--force`. A non-TTY run without
//! `--force` fails with a hint rather than auto-submitting.

use serde_json::{json, Value};

use crate::cli::{LoginArgs, PaymentsCommand};
use crate::client::Xfinity;
use crate::commands::{
    confirm, delete_payments_session, get_payments_session, prompt_username_if_needed,
    set_payments_session, stdin_is_tty, Ctx,
};
use crate::error::AppError;
use crate::output;
use crate::secrets;

pub fn run(ctx: &Ctx, cmd: &PaymentsCommand) -> Result<(), AppError> {
    match cmd {
        PaymentsCommand::Login(args) => login(ctx, args),
        PaymentsCommand::Logout => logout(ctx),
        PaymentsCommand::Methods => {
            output::render(&ctx.connect_payments()?.payment_methods()?);
            Ok(())
        }
        PaymentsCommand::Scheduled => {
            output::render(&ctx.connect_payments()?.payment_scheduled()?);
            Ok(())
        }
        PaymentsCommand::Autopay => {
            output::render(&ctx.connect_payments()?.autopay()?);
            Ok(())
        }
        PaymentsCommand::Create {
            amount,
            date,
            method,
            force,
        } => create(ctx, amount, date.as_deref(), method.as_deref(), *force),
    }
}

fn login(ctx: &Ctx, args: &LoginArgs) -> Result<(), AppError> {
    let username = prompt_username_if_needed(ctx, args.non_interactive)?;

    let secret = match (args.stdin, &args.from_env) {
        (true, Some(_)) => {
            return Err(AppError::Usage(
                "--stdin and --from-env are mutually exclusive".into(),
            ))
        }
        (true, None) => secrets::read_stdin()?,
        (false, Some(var)) => secrets::read_from_env(var)?,
        (false, None) => {
            return Err(AppError::Usage(
                "provide the payments session via --stdin or --from-env <VAR> \
                 (see `xfin payments login --help`)"
                    .into(),
            ))
        }
    };
    if secret.is_empty() {
        return Err(AppError::Usage("empty session — nothing stored".into()));
    }

    if !args.no_verify {
        // A cheap authenticated read confirms the payments session is live.
        Xfinity::from_payments_session(&secret)?.payment_methods()?;
        if ctx.verbose() {
            eprintln!("payments session verified against payments.xfinity.com");
        }
    }

    let existed = get_payments_session(&username)?.is_some();
    if existed && !args.overwrite {
        return Err(AppError::Usage(format!(
            "a payments session for {username:?} already exists — pass --overwrite to replace it"
        )));
    }
    set_payments_session(&username, &secret)?;

    if !ctx.cli.quiet {
        eprintln!("stored Xfinity payments session for {username} in the keychain");
    }
    if args.json || ctx.cli.json {
        output::json(&json!({
            "status": "stored",
            "username": username,
            "scope": "payments",
            "verified": !args.no_verify,
            "overwritten": existed,
        }));
    }
    Ok(())
}

fn logout(ctx: &Ctx) -> Result<(), AppError> {
    let username = ctx.resolve_username()?;
    let removed = delete_payments_session(&username)?;
    if !ctx.cli.quiet {
        eprintln!(
            "{}",
            if removed {
                "cleared stored payments session"
            } else {
                "no payments session was stored"
            }
        );
    }
    Ok(())
}

fn create(
    ctx: &Ctx,
    amount: &str,
    date: Option<&str>,
    method: Option<&str>,
    force: bool,
) -> Result<(), AppError> {
    let x = ctx.connect_payments()?;
    let pay_date = date
        .map(String::from)
        .unwrap_or_else(|| crate::dates::fmt_mm_dd_yyyy(crate::dates::today()));

    if !force {
        if !stdin_is_tty() {
            return Err(AppError::ConfirmationRequired(
                "stdin is not a TTY — pass --force to submit the payment non-interactively".into(),
            ));
        }
        eprintln!(
            "About to pay ${amount} on this Xfinity account (date {pay_date}{}).",
            method.map(|m| format!(", method {m}")).unwrap_or_default()
        );
        if !confirm("Submit this payment? [y/N] ")? {
            return Err(AppError::Usage("payment cancelled".into()));
        }
    }

    let mut body = json!({ "amount": amount, "paymentDate": pay_date });
    if let Some(m) = method {
        body["paymentMethodId"] = Value::String(m.to_string());
    }
    output::render(&x.make_payment(&body)?);
    Ok(())
}
