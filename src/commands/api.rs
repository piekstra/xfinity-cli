//! `xfin api` — raw authenticated request to any Xfinity endpoint. Round-trips
//! JSON, so it always emits JSON. The escape hatch for endpoints without a
//! first-class command, and for inspecting response shapes while they're being
//! mapped.

use serde_json::Value;

use crate::cli::ApiArgs;
use crate::commands::Ctx;
use crate::error::AppError;
use crate::output;

pub fn run(ctx: &Ctx, args: &ApiArgs) -> Result<(), AppError> {
    let body: Option<Value> = match &args.data {
        Some(s) => Some(
            serde_json::from_str(s)
                .map_err(|e| AppError::Usage(format!("--data is not valid JSON: {e}")))?,
        ),
        None => None,
    };

    // `Ctx::read` wraps `pk_cli_auth::reauth::with_reauth`, which re-issues
    // the request once on 401 — safe for reads (idempotent), unsafe for
    // mutations (a payment / update-of-record / any non-safe verb could run
    // twice against the server if the response looked like an auth failure).
    // The account-experience surface is POST-only *today*, and most of its
    // POSTs are reads, but `xfin api` can't tell a read-shaped POST from a
    // write-shaped POST — and the escape hatch shouldn't have to. Route the
    // RFC 7231 "safe" verbs through the retry rail, and everything else
    // through the plain path so a 401 on a mutation surfaces as exit 3 and
    // the user re-runs deliberately.
    let v = if method_is_safe(&args.method) {
        ctx.read(|x| x.request(&args.method, &args.path, body.as_ref()))?
    } else {
        ctx.connect()?
            .request(&args.method, &args.path, body.as_ref())?
    };
    output::json(&v);
    Ok(())
}

/// RFC 7231 §4.2.1 "safe" methods — no observable side effects on the
/// origin server, so replaying one on 401 recovery is fine. Everything else
/// is treated as a mutation and skips the auto-retry rail.
pub(crate) fn method_is_safe(method: &str) -> bool {
    matches!(
        method.trim().to_ascii_uppercase().as_str(),
        "GET" | "HEAD" | "OPTIONS"
    )
}

#[cfg(test)]
mod tests {
    use super::method_is_safe;

    /// The invariant load-bearing for the safety split: only the RFC 7231
    /// "safe" verbs get the auto-retry rail. Everything else falls to the
    /// no-retry path so a 401 on a mutation doesn't silently double-fire.
    #[test]
    fn method_safety_classification() {
        for safe in ["GET", "get", "Head", "HEAD", "OPTIONS", "options"] {
            assert!(method_is_safe(safe), "{safe} must be classified safe");
        }
        for unsafe_ in ["POST", "post", "PUT", "PATCH", "DELETE", "delete", ""] {
            assert!(
                !method_is_safe(unsafe_),
                "{unsafe_} must NOT be classified safe"
            );
        }
        // Case + whitespace tolerance: a shell alias that pipes " GET " with
        // surrounding spaces must not fall to the mutation path just because
        // of the whitespace.
        assert!(method_is_safe(" GET "));
        assert!(!method_is_safe(" POST "));
    }
}
