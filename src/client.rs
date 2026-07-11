//! Xfinity self-care HTTP client.
//!
//! Xfinity exposes no official public API. Everything here targets the same
//! `customer.xfinity.com/apis/*` self-care JSON services that the My Account
//! web app calls. Endpoint paths were mapped from the web app's own network
//! traffic against a live account. See `docs/api.md`.
//!
//! Auth model: Xfinity's login (`login.xfinity.com`) sits behind aggressive
//! bot protection that rejects non-browser clients outright, so this CLI does
//! **not** replay a username/password. Instead you log in once in a real
//! browser and hand the resulting authenticated session to `xfin auth login`.
//! That session — the `Cookie` header from a logged-in `customer.xfinity.com`
//! request — is stored in the OS keychain and replayed here. The `/apis/*`
//! services use a double-submit CSRF check: the request must carry an
//! `x-xsrf-token` header whose value is the (URL-decoded) `XSRF-TOKEN` cookie.
//! We derive that from the stored cookie jar automatically. When the session
//! expires, log in again in the browser and re-run `xfin auth login`.

use std::time::Duration;

use serde_json::Value;

use crate::error::AppError;
use crate::secrets::Secret;

/// Self-care host. Overridable with `$XFINITY_API_HOST` for probing.
pub fn api_host() -> String {
    std::env::var("XFINITY_API_HOST")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "https://customer.xfinity.com".to_string())
}

/// A recent desktop Chrome UA. Xfinity's edge is picky about obviously-bot
/// clients, so mirror the browser the session was captured in.
const UA: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0.0.0 Safari/537.36";

/// An authenticated Xfinity self-care session.
pub struct Xfinity {
    client: reqwest::blocking::Client,
    /// Raw `Cookie` header value captured from a logged-in browser.
    session: String,
    /// The `x-xsrf-token` header value, derived from the `XSRF-TOKEN` cookie.
    xsrf: Option<String>,
}

fn build_client() -> Result<reqwest::blocking::Client, AppError> {
    reqwest::blocking::Client::builder()
        .user_agent(UA)
        .cookie_store(true)
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(45))
        .build()
        .map_err(|e| AppError::Other(format!("failed to build HTTP client: {e}")))
}

/// Minimal percent-decoder for the `XSRF-TOKEN` cookie value (it arrives
/// URL-encoded, e.g. `%2B` for `+`). No external crate needed.
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hi = (bytes[i + 1] as char).to_digit(16);
            let lo = (bytes[i + 2] as char).to_digit(16);
            if let (Some(hi), Some(lo)) = (hi, lo) {
                out.push((hi * 16 + lo) as u8);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Pull the `XSRF-TOKEN` value out of a `Cookie` header string and decode it.
fn xsrf_from_cookies(cookie_header: &str) -> Option<String> {
    for part in cookie_header.split(';') {
        let part = part.trim();
        if let Some(val) = part.strip_prefix("XSRF-TOKEN=") {
            if !val.is_empty() {
                return Some(percent_decode(val));
            }
        }
    }
    None
}

/// Pull a short human hint out of an error response body.
fn body_hint(text: &str) -> String {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    if let Ok(v) = serde_json::from_str::<Value>(trimmed) {
        for key in ["message", "error", "errorMessage", "userMessage"] {
            if let Some(m) = v.get(key).and_then(|x| x.as_str()) {
                if !m.is_empty() {
                    return format!(" — {m}");
                }
            }
        }
    }
    // Avoid dumping an HTML error page; only echo short text bodies.
    if trimmed.starts_with('<') {
        return String::new();
    }
    format!(" — {}", trimmed.chars().take(120).collect::<String>())
}

/// Turn a service path into a full URL. Accepts an absolute URL or a
/// leading-slash path relative to the self-care host.
fn url_for(path: &str) -> String {
    if path.starts_with("http://") || path.starts_with("https://") {
        path.to_string()
    } else {
        let host = api_host();
        if let Some(rest) = path.strip_prefix('/') {
            format!("{host}/{rest}")
        } else {
            format!("{host}/{path}")
        }
    }
}

impl Xfinity {
    /// Build a session from a captured browser `Cookie` header. No network
    /// call — the cookie is validated lazily on the first request.
    pub fn from_session(session: &Secret) -> Result<Xfinity, AppError> {
        if session.is_empty() {
            return Err(AppError::Auth(
                "no Xfinity session stored — run `xfin auth login` (see `xfin auth login --help`)"
                    .into(),
            ));
        }
        let cookie = session.expose().to_string();
        let xsrf = xsrf_from_cookies(&cookie);
        Ok(Xfinity {
            client: build_client()?,
            session: cookie,
            xsrf,
        })
    }

    fn auth(&self, req: reqwest::blocking::RequestBuilder) -> reqwest::blocking::RequestBuilder {
        let mut req = req
            .header("Accept", "application/json, text/plain, */*")
            .header("Cookie", &self.session)
            .header("Referer", format!("{}/", api_host()));
        if let Some(x) = &self.xsrf {
            req = req.header("x-xsrf-token", x);
        }
        req
    }

    fn handle(&self, resp: reqwest::blocking::Response, path: &str) -> Result<Value, AppError> {
        let status = resp.status();
        let text = resp.text().unwrap_or_default();
        if matches!(status.as_u16(), 401 | 403) {
            return Err(AppError::Auth(format!(
                "Xfinity returned {} for {path} — the stored session is expired or invalid. \
                 Log in again in your browser and re-run `xfin auth login --overwrite`.",
                status.as_u16()
            )));
        }
        if status.as_u16() == 404 {
            return Err(AppError::NotFound(format!("{path} (HTTP 404)")));
        }
        if !status.is_success() {
            return Err(AppError::Upstream(format!(
                "Xfinity HTTP {} for {path}{}",
                status.as_u16(),
                body_hint(&text)
            )));
        }
        if text.trim().is_empty() {
            return Ok(Value::Null);
        }
        serde_json::from_str(&text).map_err(|_| {
            AppError::Other(format!(
                "Xfinity returned a non-JSON response for {path} — the session may have been \
                 bounced to a login page (first bytes: {:?})",
                text.chars().take(60).collect::<String>()
            ))
        })
    }

    pub fn get(&self, path: &str) -> Result<Value, AppError> {
        let resp = self.auth(self.client.get(url_for(path))).send()?;
        self.handle(resp, path)
    }

    pub fn post(&self, path: &str, body: &Value) -> Result<Value, AppError> {
        let resp = self
            .auth(self.client.post(url_for(path)))
            .header("Content-Type", "application/json")
            .json(body)
            .send()?;
        self.handle(resp, path)
    }

    /// Raw request escape hatch used by `xfin api`. `method` is case-insensitive.
    pub fn request(
        &self,
        method: &str,
        path: &str,
        body: Option<&Value>,
    ) -> Result<Value, AppError> {
        match method.to_uppercase().as_str() {
            "GET" => self.get(path),
            "POST" => self.post(path, body.unwrap_or(&Value::Null)),
            "PUT" => {
                let resp = self
                    .auth(self.client.put(url_for(path)))
                    .header("Content-Type", "application/json")
                    .json(body.unwrap_or(&Value::Null))
                    .send()?;
                self.handle(resp, path)
            }
            "DELETE" => {
                let resp = self.auth(self.client.delete(url_for(path))).send()?;
                self.handle(resp, path)
            }
            other => Err(AppError::Usage(format!(
                "unsupported HTTP method {other:?} (use GET, POST, PUT, or DELETE)"
            ))),
        }
    }

    // ---- Account -----------------------------------------------------------

    /// The signed-in customer's profile (name, username, email, guid).
    pub fn account(&self) -> Result<Value, AppError> {
        self.get("/apis/macaroon")
    }

    /// The default account number on this login.
    pub fn default_account(&self) -> Result<Value, AppError> {
        self.get("/apis/ssm/account/default")
    }

    /// Users/contacts on the account.
    pub fn users(&self) -> Result<Value, AppError> {
        self.get("/apis/users")
    }

    /// Account locality / service info.
    pub fn info(&self) -> Result<Value, AppError> {
        self.get("/apis/info")
    }

    /// Two-factor and multi-factor auth enrollment for a user `guid`.
    pub fn security(&self, guid: &str) -> Result<Value, AppError> {
        let two = self
            .get(&format!("/apis/csp/account/me/user/{guid}/twoFactorAuth"))
            .unwrap_or(Value::Null);
        let multi = self
            .get(&format!("/apis/csp/account/me/user/{guid}/multiFactorAuth"))
            .unwrap_or(Value::Null);
        Ok(serde_json::json!({ "twoFactorAuth": two, "multiFactorAuth": multi }))
    }

    // ---- Billing -----------------------------------------------------------

    /// Current bill summary: balance, due date, autopay status.
    pub fn billing_summary(&self) -> Result<Value, AppError> {
        self.get("/apis/bill/current")
    }

    /// Upcoming due date and valid payment dates.
    pub fn due_dates(&self) -> Result<Value, AppError> {
        self.get("/apis/ssm/bill/duedates")
    }

    /// Prior statements (amounts, periods).
    pub fn statements(&self) -> Result<Value, AppError> {
        self.get("/apis/brite-bill/account/SELF/bills")
    }

    /// A single statement by id (from `statements`).
    pub fn statement(&self, id: &str) -> Result<Value, AppError> {
        self.get(&format!("/apis/brite-bill/account/SELF/bill/{id}"))
    }

    // ---- Payments ----------------------------------------------------------
    //
    // The payment surface is more locked down than the read surface (some
    // `/apis/ssm/payments/*` routes require the macaroon bearer the SPA mints,
    // not just the cookie+CSRF pair). These are best-effort; if one 403s, use
    // `xfin api` to inspect what the browser actually calls and refine.

    /// Recent payment history.
    pub fn payment_history(&self) -> Result<Value, AppError> {
        self.get("/apis/ssm/payments/history")
    }

    /// Saved payment methods (masked bank/card tokens).
    pub fn payment_methods(&self) -> Result<Value, AppError> {
        self.get("/apis/ssm/bill/paymentmethods")
    }

    /// Submit a one-time payment. `body` carries amount, date, and method token.
    pub fn make_payment(&self, body: &Value) -> Result<Value, AppError> {
        self.post("/apis/ssm/payments", body)
    }

    // ---- Internet / usage --------------------------------------------------

    /// Current-cycle internet data usage (used/allowable GB, cycle dates).
    pub fn internet_usage(&self) -> Result<Value, AppError> {
        self.get("/apis/csp/account/me/services/internet/usage")
    }

    /// The subscribed internet plan (tier, download/upload speeds).
    pub fn internet_plan(&self) -> Result<Value, AppError> {
        self.get("/apis/csp/account/me/services/internet/plan")
    }

    /// Devices seen on the account's gateway.
    pub fn internet_devices(&self) -> Result<Value, AppError> {
        self.get("/apis/csp/account/me/devices")
    }

    /// Gateway/modem online status.
    pub fn devices_status(&self) -> Result<Value, AppError> {
        self.get("/apis/ssm/devices/status")
    }

    // ---- Outages & equipment ----------------------------------------------

    /// Consolidated service-outage status across lines of business.
    pub fn outages(&self) -> Result<Value, AppError> {
        self.get("/apis/ssm/outage/consolidated/lob")
    }

    /// Pending equipment returns (device-shipping-manager).
    pub fn equipment_returns(&self) -> Result<Value, AppError> {
        self.get("/apis/ssm/dsm/returns")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xsrf_extracted_and_decoded() {
        let ck = "foo=1; XSRF-TOKEN=abc%2Bdef%2F123%3D%3D; bar=2";
        assert_eq!(xsrf_from_cookies(ck).as_deref(), Some("abc+def/123=="));
        assert_eq!(xsrf_from_cookies("no=token"), None);
    }

    #[test]
    fn percent_decode_passthrough() {
        assert_eq!(percent_decode("plain"), "plain");
        assert_eq!(percent_decode("a%20b"), "a b");
    }
}
