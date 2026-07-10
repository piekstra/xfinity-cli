//! `xfin payments` — history, saved methods, and making a payment.
//!
//! `payments create` moves real money: a non-reversible mutation, so it
//! confirms by default and skips only with `--force`. A non-TTY run without
//! `--force` fails with a hint rather than auto-submitting.

use serde_json::{json, Value};

use crate::cli::PaymentsCommand;
use crate::commands::{confirm, stdin_is_tty, Ctx};
use crate::error::AppError;
use crate::output;

pub fn run(ctx: &Ctx, cmd: &PaymentsCommand) -> Result<(), AppError> {
    let x = ctx.connect()?;
    match cmd {
        PaymentsCommand::List => output::render(&x.payment_history()?),
        PaymentsCommand::Methods => output::render(&x.payment_methods()?),
        PaymentsCommand::Create {
            amount,
            date,
            method,
            force,
        } => {
            let pay_date = date
                .clone()
                .unwrap_or_else(|| crate::dates::fmt_mm_dd_yyyy(crate::dates::today()));

            if !force {
                if !stdin_is_tty() {
                    return Err(AppError::Usage(
                        "stdin is not a TTY — pass --force to submit the payment \
                         non-interactively"
                            .into(),
                    ));
                }
                eprintln!(
                    "About to pay ${amount} on this Xfinity account (date {pay_date}{}).",
                    method
                        .as_deref()
                        .map(|m| format!(", method {m}"))
                        .unwrap_or_default()
                );
                if !confirm("Submit this payment? [y/N] ")? {
                    return Err(AppError::Usage("payment cancelled".into()));
                }
            }

            let mut body = json!({ "amount": amount, "paymentDate": pay_date });
            if let Some(m) = method {
                body["paymentMethodId"] = Value::String(m.clone());
            }
            output::render(&x.make_payment(&body)?);
        }
    }
    Ok(())
}
