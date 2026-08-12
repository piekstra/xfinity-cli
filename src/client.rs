//! Xfinity account HTTP client (new `www.xfinity.com/account` experience).
//!
//! Xfinity migrated accounts to a new account experience. The legacy
//! `customer.xfinity.com/apis/*` surface (cookie + `x-xsrf-token`) is dead for
//! migrated accounts, so this client targets the new surface the
//! `www.xfinity.com/account` web app uses:
//!
//! - Host/paths: `https://www.xfinity.com/digital/service/api/*`
//! - Method: **POST** with a small JSON body
//! - Auth: **`Authorization: Bearer <token>`** — no cookies, no CSRF token.
//!
//! Two "fat" endpoints cover most of the CLI:
//! - `BillingInfo/billingSummary` → balance, due date, autopay, statements,
//!   scheduled payments, transaction history.
//! - `BillingInfo/context` → account profile, users, devices/equipment,
//!   outages, plan/services.
//!
//! Auth model: the login flow is behind bot protection, so the CLI does not
//! replay a password. You capture the `Authorization: Bearer …` header from a
//! logged-in browser (DevTools → Network, any `digital/service/api` request)
//! and store it via `xfin auth login`. It's replayed here until it expires.
//! See `docs/api.md`.

use std::time::Duration;

use serde_json::{json, Value};

use crate::error::AppError;
use crate::secrets::Secret;

/// Account-experience API host. Overridable with `$XFINITY_API_HOST` for probing.
pub fn api_host() -> String {
    std::env::var("XFINITY_API_HOST")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "https://www.xfinity.com".to_string())
}

/// Major Chrome version we impersonate. Xfinity's Akamai edge cross-checks the
/// `User-Agent` against the `Sec-CH-UA` client hint, so both must report the
/// same version — keep this the single source of truth and derive both from it.
const CHROME_MAJOR: &str = "126";

fn user_agent() -> String {
    format!(
        "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 \
         (KHTML, like Gecko) Chrome/{CHROME_MAJOR}.0.0.0 Safari/537.36"
    )
}

fn sec_ch_ua() -> String {
    format!(
        "\"Chromium\";v=\"{CHROME_MAJOR}\", \"Google Chrome\";v=\"{CHROME_MAJOR}\", \
         \"Not?A_Brand\";v=\"24\""
    )
}

/// An authenticated Xfinity account-experience session.
pub struct Xfinity {
    client: reqwest::blocking::Client,
    host: String,
    /// `Authorization` header value, e.g. `Bearer <token>`.
    bearer: String,
}

fn build_client() -> Result<reqwest::blocking::Client, AppError> {
    reqwest::blocking::Client::builder()
        .user_agent(user_agent())
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(45))
        .build()
        .map_err(|e| AppError::Other(format!("failed to build HTTP client: {e}")))
}

/// Normalize a captured token into a full `Authorization` header value.
/// Accepts either `Bearer <tok>` or a bare `<tok>`.
fn normalize_bearer(raw: &str) -> String {
    let t = raw.trim();
    if t.to_ascii_lowercase().starts_with("bearer ") {
        t.to_string()
    } else {
        format!("Bearer {t}")
    }
}

/// Pull a short human hint out of an error response body.
fn body_hint(text: &str) -> String {
    let trimmed = text.trim();
    if trimmed.is_empty() || trimmed.starts_with('<') {
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

impl Xfinity {
    /// Build a session from a captured `Authorization: Bearer …` token. No
    /// network call — the token is validated lazily on the first request.
    pub fn from_session(session: &Secret) -> Result<Xfinity, AppError> {
        if session.is_empty() {
            return Err(AppError::Auth(
                "no Xfinity token stored — run `xfin auth login` (see `xfin auth login --help`)"
                    .into(),
            ));
        }
        Ok(Xfinity {
            client: build_client()?,
            host: api_host().trim_end_matches('/').to_string(),
            bearer: normalize_bearer(session.expose()),
        })
    }

    fn url_for(&self, path: &str) -> String {
        if path.starts_with("http://") || path.starts_with("https://") {
            path.to_string()
        } else {
            format!(
                "{}/digital/service/api/{}",
                self.host,
                path.trim_start_matches('/')
            )
        }
    }

    /// POST a JSON body to a `digital/service/api` endpoint and return the
    /// parsed response. All the account-experience endpoints are POSTs.
    pub fn post(&self, path: &str, body: &Value) -> Result<Value, AppError> {
        let resp = self
            .client
            .post(self.url_for(path))
            .header("Authorization", &self.bearer)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json, text/plain, */*")
            .header("Referer", format!("{}/account", self.host))
            .header("Sec-Fetch-Dest", "empty")
            .header("Sec-Fetch-Mode", "cors")
            .header("Sec-Fetch-Site", "same-origin")
            .header("Sec-CH-UA-Mobile", "?0")
            .header("Sec-CH-UA-Platform", "\"macOS\"")
            .header("Sec-CH-UA", sec_ch_ua())
            .json(body)
            .send()?;
        self.handle(resp, path)
    }

    fn handle(&self, resp: reqwest::blocking::Response, path: &str) -> Result<Value, AppError> {
        let status = resp.status();
        let text = resp.text().unwrap_or_default();
        if matches!(status.as_u16(), 401 | 403) {
            return Err(AppError::Auth(format!(
                "Xfinity returned {} for {path} — the stored token is expired or invalid. \
                 Capture a fresh `Authorization: Bearer …` in your browser and re-run \
                 `xfin auth login --overwrite`.",
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
                "Xfinity returned a non-JSON response for {path} (first bytes: {:?})",
                text.chars().take(60).collect::<String>()
            ))
        })
    }

    /// Raw request escape hatch used by `xfin api`. Only POST is supported on
    /// the account-experience surface; `body` defaults to `{}`.
    pub fn request(
        &self,
        method: &str,
        path: &str,
        body: Option<&Value>,
    ) -> Result<Value, AppError> {
        match method.to_uppercase().as_str() {
            "POST" => self.post(path, body.unwrap_or(&json!({}))),
            other => Err(AppError::Usage(format!(
                "the account-experience API is POST-only; got {other:?}. \
                 Example: xfin api POST BillingInfo/billingSummary \
                 --data '{{\"requestTypes\":[\"CORE\"],\"metadata\":{{\"source\":\"web\"}}}}'"
            ))),
        }
    }

    // ---- The two "fat" endpoints -------------------------------------------

    /// Billing summary: balance, due date, autopay, statements, scheduled
    /// payments, transaction history (under `responseData.data.BBDS`).
    pub fn billing_summary(&self) -> Result<Value, AppError> {
        self.post(
            "BillingInfo/billingSummary",
            &json!({"requestTypes": ["CORE", "XM"], "metadata": {"source": "web"}}),
        )
    }

    /// Account context: account profile, users, devices/equipment, outages,
    /// subscription (under `responseData.data.{accountContext,deviceContext,…}`).
    pub fn context(&self) -> Result<Value, AppError> {
        self.post(
            "BillingInfo/context",
            &json!({
                "eventNames": [
                    "call.getContext.Account",
                    "call.getContext.Subscription",
                    "call.getContext.Device",
                    "call.getContext.Outage",
                    "call.getContext.Indicator"
                ],
                "data": {"metadata": {"source": "maw"}}
            }),
        )
    }

    // ---- Typed accessors (extract a section from the fat endpoints) ---------

    /// `responseData.data.<key>` from a `context()` response.
    fn context_section(&self, key: &str) -> Result<Value, AppError> {
        let v = self.context()?;
        Ok(v.pointer(&format!("/responseData/data/{key}"))
            .cloned()
            .unwrap_or(Value::Null))
    }

    /// Account profile section (name, address, users, accountNumber, services,
    /// loyalty, productInfo).
    pub fn account(&self) -> Result<Value, AppError> {
        self.context_section("accountContext")
    }

    /// Device/equipment section.
    pub fn devices(&self) -> Result<Value, AppError> {
        self.context_section("deviceContext")
    }

    /// Outage section.
    pub fn outages(&self) -> Result<Value, AppError> {
        self.context_section("outageContext")
    }

    /// Subscription section (plan info for internet/video/voice/mobile, TV
    /// subscription, autoRefill). Internet plan + data usage live under
    /// `customerPlanInfo.internet[]`.
    pub fn subscription(&self) -> Result<Value, AppError> {
        self.context_section("subscriptionContext")
    }

    /// The primary internet plan object (`customerPlanInfo.internet[0]`), which
    /// carries `plan`/`planDescription` (speed) and `usageMonths[]` (per-cycle
    /// data usage). Returns `Null` if the account has no internet line.
    pub fn internet_plan(&self) -> Result<Value, AppError> {
        let sub = self.subscription()?;
        Ok(sub
            .pointer("/customerPlanInfo/internet/0")
            .cloned()
            .unwrap_or(Value::Null))
    }

    /// `responseData.data.BBDS` from a `billing_summary()` response (balance,
    /// dueDate, autopay, statementDetails, schedulePayments, transactionHistory).
    pub fn bbds(&self) -> Result<Value, AppError> {
        let v = self.billing_summary()?;
        Ok(v.pointer("/responseData/data/BBDS")
            .cloned()
            .unwrap_or(Value::Null))
    }

    /// Download the PDF bytes for one statement, keyed by its ISO date.
    ///
    /// The new account experience surfaces a single `statementDetails` record
    /// (see `docs/api.md`) with no first-class statement id — the date is the
    /// only stable handle the account app has for it. The downstream service
    /// answers on one of two shapes, so we tolerate both:
    ///
    /// 1. **Raw PDF bytes** (`Content-Type: application/pdf`), streamed directly.
    /// 2. **JSON envelope with base64** (`Content-Type: application/json`) —
    ///    the pattern `BillingInfo/*` uses everywhere else, with the payload
    ///    under `responseData.data.{statementPdf,pdfBytes,bytes,content}`.
    ///
    /// Returns the decoded PDF bytes on success. On an unrecognized response
    /// shape, surfaces a diagnostic that names the path — a `xfin api POST
    /// BillingInfo/…` probe is the intended follow-up.
    pub fn download_statement(&self, statement_date: &str) -> Result<Vec<u8>, AppError> {
        let path = "BillingInfo/downloadStatement";
        let body = json!({
            "statementDate": statement_date,
            "metadata": {"source": "web"}
        });
        let resp = self
            .client
            .post(self.url_for(path))
            .header("Authorization", &self.bearer)
            .header("Content-Type", "application/json")
            .header("Accept", "application/pdf, application/json, */*")
            .header("Referer", format!("{}/account", self.host))
            .header("Sec-Fetch-Dest", "empty")
            .header("Sec-Fetch-Mode", "cors")
            .header("Sec-Fetch-Site", "same-origin")
            .header("Sec-CH-UA-Mobile", "?0")
            .header("Sec-CH-UA-Platform", "\"macOS\"")
            .header("Sec-CH-UA", sec_ch_ua())
            .json(&body)
            .send()?;

        let status = resp.status();
        let content_type = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();

        if matches!(status.as_u16(), 401 | 403) {
            return Err(AppError::Auth(format!(
                "Xfinity returned {} for {path} — the stored token is expired or invalid. \
                 Capture a fresh `Authorization: Bearer …` in your browser and re-run \
                 `xfin auth login --overwrite`.",
                status.as_u16()
            )));
        }
        if status.as_u16() == 404 {
            return Err(AppError::NotFound(format!(
                "{path} (HTTP 404) — the download endpoint may have moved. \
                 Sign in at https://www.xfinity.com/account, open DevTools → Network, \
                 click \"Download PDF\" on a statement, and check the request path/body; \
                 then update `docs/api.md` and `Xfinity::download_statement` in `src/client.rs`."
            )));
        }
        let bytes = resp
            .bytes()
            .map_err(|e| AppError::Upstream(format!("reading {path} body: {e}")))?
            .to_vec();
        if !status.is_success() {
            let hint = std::str::from_utf8(&bytes)
                .map(body_hint)
                .unwrap_or_default();
            return Err(AppError::Upstream(format!(
                "Xfinity HTTP {} for {path}{hint}",
                status.as_u16()
            )));
        }
        decode_pdf(&bytes, &content_type, path)
    }
}

/// Recognize the download body as either raw PDF bytes or a JSON envelope
/// carrying base64-encoded PDF. Split out for testability — the shape
/// detection is the load-bearing bit of the download flow.
pub(crate) fn decode_pdf(
    bytes: &[u8],
    content_type: &str,
    path: &str,
) -> Result<Vec<u8>, AppError> {
    use base64::{engine::general_purpose::STANDARD, Engine as _};

    let is_pdf_bytes = bytes.starts_with(b"%PDF");
    let looks_json = content_type.contains("json")
        || bytes
            .iter()
            .position(|b| !b.is_ascii_whitespace())
            .and_then(|i| bytes.get(i))
            .is_some_and(|b| *b == b'{');

    if is_pdf_bytes && !looks_json {
        return Ok(bytes.to_vec());
    }

    if looks_json {
        let v: Value = serde_json::from_slice(bytes)
            .map_err(|e| AppError::Upstream(format!("{path} returned undecodable JSON: {e}")))?;
        // Xfinity's `BillingInfo/*` responses wrap under `responseData.data`;
        // tolerate the common field names for the PDF payload.
        let candidates = [
            "/responseData/data/statementPdf",
            "/responseData/data/pdfBytes",
            "/responseData/data/bytes",
            "/responseData/data/content",
            "/responseData/data/pdf",
            "/data/statementPdf",
            "/data/pdfBytes",
            "/data/bytes",
            "/statementPdf",
            "/pdfBytes",
            "/bytes",
            "/content",
        ];
        let b64 = candidates
            .iter()
            .find_map(|p| v.pointer(p).and_then(|x| x.as_str()))
            .ok_or_else(|| {
                AppError::Upstream(format!(
                    "{path} returned JSON without a recognized PDF field \
                     (tried {candidates:?}) — inspect with `xfin api POST {path}` \
                     and add the field path to `src/client.rs::decode_pdf`"
                ))
            })?;
        let stripped: String = b64
            .split_whitespace()
            .collect::<String>()
            .trim_start_matches("data:application/pdf;base64,")
            .to_string();
        let decoded = STANDARD
            .decode(stripped)
            .map_err(|e| AppError::Upstream(format!("{path}: base64 decode failed: {e}")))?;
        if !decoded.starts_with(b"%PDF") {
            return Err(AppError::Upstream(format!(
                "{path}: decoded payload is not a PDF (first bytes: {:?})",
                decoded.iter().take(8).collect::<Vec<_>>()
            )));
        }
        return Ok(decoded);
    }

    Err(AppError::Upstream(format!(
        "{path}: unrecognized response (content-type {content_type:?}, {} bytes; \
         does not start with `%PDF` or JSON `{{`)",
        bytes.len()
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bearer_normalizes() {
        assert_eq!(normalize_bearer("abc"), "Bearer abc");
        assert_eq!(normalize_bearer("Bearer abc"), "Bearer abc");
        assert_eq!(normalize_bearer("  bearer xyz  "), "bearer xyz");
    }

    #[test]
    fn url_builds_digital_service_path() {
        let s = Secret::new("tok");
        let x = Xfinity::from_session(&s).unwrap();
        assert_eq!(
            x.url_for("BillingInfo/billingSummary"),
            "https://www.xfinity.com/digital/service/api/BillingInfo/billingSummary"
        );
        assert_eq!(x.url_for("https://other/x"), "https://other/x");
    }

    // ---- decode_pdf: the shape detection is the load-bearing bit of the
    // download flow, and the two response shapes are what a mid-run upstream
    // shift is most likely to change.

    #[test]
    fn decode_pdf_returns_raw_bytes_when_content_type_is_pdf() {
        let bytes = b"%PDF-1.7\nfake body";
        let out = decode_pdf(bytes, "application/pdf", "BillingInfo/downloadStatement").unwrap();
        assert_eq!(out, bytes);
    }

    #[test]
    fn decode_pdf_decodes_base64_from_json_envelope() {
        use base64::{engine::general_purpose::STANDARD, Engine as _};
        let pdf = b"%PDF-1.4\nhello";
        let b64 = STANDARD.encode(pdf);
        let body = format!("{{\"responseData\":{{\"data\":{{\"statementPdf\":\"{b64}\"}}}}}}");
        let out = decode_pdf(
            body.as_bytes(),
            "application/json",
            "BillingInfo/downloadStatement",
        )
        .unwrap();
        assert_eq!(out, pdf);
    }

    #[test]
    fn decode_pdf_tolerates_data_url_prefix_and_whitespace() {
        use base64::{engine::general_purpose::STANDARD, Engine as _};
        let pdf = b"%PDF-1.5\nx";
        let b64 = STANDARD.encode(pdf);
        // Field name at the top level; base64 wrapped as a data URL and
        // sprinkled with whitespace the way some portals stream it.
        let body = format!(
            "{{\"pdfBytes\":\"data:application/pdf;base64,{}\\n{}\"}}",
            &b64[..b64.len() / 2],
            &b64[b64.len() / 2..]
        );
        let out = decode_pdf(
            body.as_bytes(),
            "application/json",
            "BillingInfo/downloadStatement",
        )
        .unwrap();
        assert_eq!(out, pdf);
    }

    #[test]
    fn decode_pdf_errors_when_json_has_no_recognized_field() {
        let body = br#"{"responseData":{"data":{"unrelated":"x"}}}"#;
        let err =
            decode_pdf(body, "application/json", "BillingInfo/downloadStatement").unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("recognized PDF field"),
            "unexpected error: {msg}"
        );
    }

    #[test]
    fn decode_pdf_errors_on_undecodable_response() {
        let err = decode_pdf(
            b"not a pdf, not json",
            "text/html",
            "BillingInfo/downloadStatement",
        )
        .unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("unrecognized response"),
            "unexpected error: {msg}"
        );
    }

    #[test]
    fn decode_pdf_errors_when_decoded_payload_isnt_a_pdf() {
        use base64::{engine::general_purpose::STANDARD, Engine as _};
        let b64 = STANDARD.encode(b"<html>login</html>");
        let body = format!("{{\"statementPdf\":\"{b64}\"}}");
        let err = decode_pdf(
            body.as_bytes(),
            "application/json",
            "BillingInfo/downloadStatement",
        )
        .unwrap_err();
        assert!(format!("{err}").contains("not a PDF"));
    }
}
