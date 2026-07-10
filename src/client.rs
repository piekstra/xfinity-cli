//! Xfinity self-care HTTP client.
//!
//! Xfinity exposes no official public API. Everything here targets the same
//! `api.sc.xfinity.com` self-care JSON services that the my-account web app and
//! the Xfinity mobile app use. Endpoint paths were mapped from the web app's
//! own network traffic and cross-checked against the community Home Assistant
//! usage integrations. See `docs/api.md`.
//!
//! Auth model: Xfinity's login (`login.xfinity.com`) sits behind aggressive
//! bot protection that rejects non-browser clients outright, so this CLI does
//! **not** replay a username/password. Instead you log in once in a real
//! browser and hand the resulting authenticated session to `xfin auth login`.
//! That session (a `Cookie` header value) is stored in the OS keychain and
//! replayed on every request here. When it expires, log in again in the
//! browser and re-run `xfin auth login`. See `docs/api.md` §Auth.

use std::time::Duration;

use serde_json::Value;

use crate::error::AppError;
use crate::secrets::Secret;

/// Self-care API host. Overridable with `$XFINITY_API_HOST` for probing.
pub fn api_host() -> String {
    std::env::var("XFINITY_API_HOST")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "https://api.sc.xfinity.com".to_string())
}

/// A recent desktop Chrome UA. Xfinity's edge is picky about obviously-bot
/// clients, so mirror the browser the session was captured in.
const UA: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0.0.0 Safari/537.36";

/// An authenticated Xfinity self-care session.
pub struct Xfinity {
    client: reqwest::blocking::Client,
    /// Raw `Cookie` header value captured from a logged-in browser.
    session: String,
}

fn build_client() -> Result<reqwest::blocking::Client, AppError> {
    reqwest::blocking::Client::builder()
        .user_agent(UA)
        .cookie_store(true)
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| AppError::Other(format!("failed to build HTTP client: {e}")))
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
        Ok(Xfinity {
            client: build_client()?,
            session: session.expose().to_string(),
        })
    }

    fn auth(&self, req: reqwest::blocking::RequestBuilder) -> reqwest::blocking::RequestBuilder {
        req.header("Accept", "application/json")
            .header("Cookie", &self.session)
    }

    fn handle(&self, resp: reqwest::blocking::Response, path: &str) -> Result<Value, AppError> {
        let status = resp.status();
        let text = resp.text().unwrap_or_default();
        if matches!(status.as_u16(), 401 | 403) {
            return Err(AppError::Auth(format!(
                "Xfinity returned {} for {path} — the stored session is expired or invalid. \
                 Log in again in your browser and re-run `xfin auth login`.",
                status.as_u16()
            )));
        }
        if status.as_u16() == 404 {
            return Err(AppError::NotFound(format!("{path} (HTTP 404)")));
        }
        if !status.is_success() {
            return Err(AppError::Network(format!(
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

    /// The signed-in customer's account profile (holder, service address,
    /// account number, contact info).
    pub fn account(&self) -> Result<Value, AppError> {
        self.get("/session/csp/selfhelp/account/me")
    }

    // ---- Billing -----------------------------------------------------------

    /// Current balance, due date, autopay/paperless status.
    pub fn billing_summary(&self) -> Result<Value, AppError> {
        self.get("/session/csp/selfhelp/billing/summary")
    }

    /// Prior statements (period, amount, status).
    pub fn statements(&self) -> Result<Value, AppError> {
        self.get("/session/csp/selfhelp/billing/statements")
    }

    /// A single statement PDF's metadata / download reference.
    pub fn statement(&self, id: &str) -> Result<Value, AppError> {
        self.get(&format!("/session/csp/selfhelp/billing/statements/{id}"))
    }

    // ---- Payments ----------------------------------------------------------

    /// Recent payment history.
    pub fn payment_history(&self) -> Result<Value, AppError> {
        self.get("/session/csp/selfhelp/billing/payments")
    }

    /// Saved payment methods (masked bank/card tokens).
    pub fn payment_methods(&self) -> Result<Value, AppError> {
        self.get("/session/csp/selfhelp/billing/payment-methods")
    }

    /// Submit a one-time payment. `body` carries amount, date, and method token.
    pub fn make_payment(&self, body: &Value) -> Result<Value, AppError> {
        self.post("/session/csp/selfhelp/billing/payments", body)
    }

    // ---- Internet / usage --------------------------------------------------

    /// Current-cycle internet data usage (used/allowable GB, cycle dates).
    pub fn internet_usage(&self) -> Result<Value, AppError> {
        self.get("/session/csp/selfhelp/account/me/services/internet/usage")
    }

    /// The subscribed internet plan (tier, download/upload speeds).
    pub fn internet_plan(&self) -> Result<Value, AppError> {
        self.get("/session/csp/selfhelp/account/me/services/internet/plan")
    }

    /// Devices seen on the account's gateway.
    pub fn internet_devices(&self) -> Result<Value, AppError> {
        self.get("/session/csp/selfhelp/account/me/services/internet/devices")
    }
}
