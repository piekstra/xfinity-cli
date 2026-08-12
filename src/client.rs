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

    /// Build a session pointed at an arbitrary host. Test-only: it lets the
    /// download guard be exercised over a real HTTP round trip to a loopback
    /// stub, without an env var (which would race other tests) and without
    /// ever touching the keychain or the network.
    #[cfg(test)]
    fn for_test(host: &str) -> Xfinity {
        Xfinity {
            client: build_client().expect("test client"),
            host: host.trim_end_matches('/').to_string(),
            bearer: normalize_bearer("test-token"),
        }
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
        // `reqwest` follows redirects, so an expired token shows up here as the
        // *final* URL being a sign-in page rather than as a 401 on the original
        // one. Capture it before the body is consumed.
        let final_url = resp.url().to_string();

        if is_login_url(&final_url) {
            return Err(AppError::Auth(expired_token_msg(
                path,
                &format!(
                    "the request was redirected to a sign-in page ({})",
                    redacted_url(&final_url)
                ),
            )));
        }
        if matches!(status.as_u16(), 401 | 403) {
            return Err(AppError::Auth(expired_token_msg(
                path,
                &format!("Xfinity returned {}", status.as_u16()),
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

/// The one message the user can act on when the stored Bearer token has gone
/// stale. `why` names how we detected it.
fn expired_token_msg(path: &str, why: &str) -> String {
    format!(
        "{why} for {path} — the stored token is expired or invalid. \
         Capture a fresh `Authorization: Bearer …` in your browser and re-run \
         `xfin auth login --overwrite` (or configure `xfin auth refresh`)."
    )
}

/// Strip query/fragment off a URL before it lands in an error message — a
/// sign-in redirect often carries the whole `state`/`redirect_uri` blob, and
/// error text ends up in transcripts and CI logs.
fn redacted_url(url: &str) -> String {
    url.split(['?', '#']).next().unwrap_or(url).to_string()
}

/// Does this URL look like an identity provider's sign-in page rather than the
/// API we asked for?
pub(crate) fn is_login_url(url: &str) -> bool {
    let u = url.to_ascii_lowercase();
    // Match on path/host segments, not bare substrings: `/digital/service/api`
    // must never look like a login page.
    [
        "login.xfinity.com",
        "oauth.xfinity.com",
        "/oauth/",
        "/login",
        "/signin",
        "/sign-in",
    ]
    .iter()
    .any(|needle| u.contains(needle))
}

/// Does this body look like an HTML document?
///
/// This is the load-bearing auth check. Xfinity answers an unauthenticated
/// document request by redirecting to the sign-in page, which `reqwest`
/// follows, so the failure arrives as **HTTP 200 with an HTML body** — not a
/// 401. Without this test the CLI would happily write a login page into
/// `statement.pdf` and report success.
pub(crate) fn looks_like_html(bytes: &[u8], content_type: &str) -> bool {
    if content_type.to_ascii_lowercase().contains("text/html") {
        return true;
    }
    let head: String = bytes
        .iter()
        .take(512)
        .map(|b| *b as char)
        .collect::<String>()
        .to_ascii_lowercase();
    let head = head.trim_start();
    head.starts_with("<!doctype html")
        || head.starts_with("<html")
        || head.starts_with("<?xml") && head.contains("<html")
}

/// Recognize the download body as either raw PDF bytes or a JSON envelope
/// carrying base64-encoded PDF. Split out for testability — the shape
/// detection is the load-bearing bit of the download flow.
///
/// Order matters: the `%PDF` magic number is authoritative (a real PDF is a
/// PDF whatever the `Content-Type` claims), then HTML is rejected as an auth
/// failure, and only then do we try the JSON envelope. Nothing reaches the
/// caller — and so nothing reaches the filesystem — unless it starts with
/// `%PDF`.
pub(crate) fn decode_pdf(
    bytes: &[u8],
    content_type: &str,
    path: &str,
) -> Result<Vec<u8>, AppError> {
    use base64::{engine::general_purpose::STANDARD, Engine as _};

    // 1. Magic number wins. `Content-Type` is a claim; `%PDF` is evidence.
    if bytes.starts_with(b"%PDF") {
        return Ok(bytes.to_vec());
    }

    // 2. HTML where a PDF was promised means the session died mid-flight.
    //    Exit 3 (auth), never a written file.
    if looks_like_html(bytes, content_type) {
        return Err(AppError::Auth(expired_token_msg(
            path,
            "Xfinity returned an HTML page where a PDF was expected (an expired \
             session is answered with the sign-in page, not a 401)",
        )));
    }

    let looks_json = content_type.contains("json")
        || bytes
            .iter()
            .position(|b| !b.is_ascii_whitespace())
            .and_then(|i| bytes.get(i))
            .is_some_and(|b| *b == b'{');

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
        // Same guard one layer down: the envelope can carry a base64'd login
        // page just as easily as a base64'd PDF.
        if looks_like_html(&decoded, "") {
            return Err(AppError::Auth(expired_token_msg(
                path,
                "the JSON envelope carried a base64 HTML page instead of a PDF \
                 (an expired session is answered with the sign-in page)",
            )));
        }
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
            "application/octet-stream",
            "BillingInfo/downloadStatement",
        )
        .unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("unrecognized response"),
            "unexpected error: {msg}"
        );
        // Garbage is an upstream problem (exit 5), not an auth problem.
        assert_eq!(err.exit_code(), 5);
    }

    #[test]
    fn decode_pdf_errors_when_decoded_payload_isnt_a_pdf() {
        use base64::{engine::general_purpose::STANDARD, Engine as _};
        let b64 = STANDARD.encode(b"\x89PNG\r\n\x1a\nnot a pdf");
        let body = format!("{{\"statementPdf\":\"{b64}\"}}");
        let err = decode_pdf(
            body.as_bytes(),
            "application/json",
            "BillingInfo/downloadStatement",
        )
        .unwrap_err();
        assert!(format!("{err}").contains("not a PDF"));
        assert_eq!(err.exit_code(), 5);
    }

    // ---- The auth-expiry guard. This surface answers an expired token with a
    // 200 + the sign-in page, so "is this actually a PDF?" is the only thing
    // standing between the user and an HTML file named `statement.pdf`.

    const PDF_FIXTURE: &[u8] = include_bytes!("../tests/fixtures/statement_min.pdf");
    const JSON_FIXTURE: &str = include_str!("../tests/fixtures/download_statement_base64.json");
    const LOGIN_HTML_FIXTURE: &str =
        include_str!("../tests/fixtures/download_statement_login_page.html");

    #[test]
    fn fixture_pdf_round_trips_as_raw_bytes() {
        let out = decode_pdf(
            PDF_FIXTURE,
            "application/pdf",
            "BillingInfo/downloadStatement",
        )
        .unwrap();
        assert_eq!(out, PDF_FIXTURE);
        assert!(out.starts_with(b"%PDF"));
    }

    #[test]
    fn fixture_json_envelope_yields_the_same_pdf() {
        let out = decode_pdf(
            JSON_FIXTURE.as_bytes(),
            "application/json",
            "BillingInfo/downloadStatement",
        )
        .unwrap();
        assert_eq!(
            out, PDF_FIXTURE,
            "the base64 in the envelope fixture must decode to the PDF fixture"
        );
    }

    #[test]
    fn login_page_fixture_is_an_auth_failure_not_a_file() {
        let err = decode_pdf(
            LOGIN_HTML_FIXTURE.as_bytes(),
            "text/html; charset=utf-8",
            "BillingInfo/downloadStatement",
        )
        .unwrap_err();
        // Exit 3, so callers never write it and never report success.
        assert_eq!(err.exit_code(), 3, "HTML login page must exit 3, got {err}");
        assert!(matches!(err, AppError::Auth(_)));
        assert!(format!("{err}").contains("auth login --overwrite"));
    }

    #[test]
    fn html_is_auth_failure_even_when_content_type_lies() {
        // Xfinity has been observed serving the sign-in page with a non-HTML
        // content type; the body sniff is what catches that.
        let err = decode_pdf(
            LOGIN_HTML_FIXTURE.as_bytes(),
            "application/pdf",
            "BillingInfo/downloadStatement",
        )
        .unwrap_err();
        assert_eq!(err.exit_code(), 3);
    }

    #[test]
    fn base64_login_page_inside_the_envelope_is_also_an_auth_failure() {
        use base64::{engine::general_purpose::STANDARD, Engine as _};
        let b64 = STANDARD.encode(LOGIN_HTML_FIXTURE.as_bytes());
        let body = format!("{{\"responseData\":{{\"data\":{{\"statementPdf\":\"{b64}\"}}}}}}");
        let err = decode_pdf(
            body.as_bytes(),
            "application/json",
            "BillingInfo/downloadStatement",
        )
        .unwrap_err();
        assert_eq!(err.exit_code(), 3, "got {err}");
    }

    #[test]
    fn a_real_pdf_is_accepted_even_when_content_type_claims_html() {
        // The magic number is evidence; the header is only a claim. A CDN that
        // mislabels the body must not cost the user their download.
        let out = decode_pdf(PDF_FIXTURE, "text/html", "BillingInfo/downloadStatement").unwrap();
        assert_eq!(out, PDF_FIXTURE);
    }

    #[test]
    fn login_urls_are_recognized_and_api_urls_are_not() {
        assert!(is_login_url(
            "https://login.xfinity.com/login?r=xfinity.com"
        ));
        assert!(is_login_url("https://www.xfinity.com/login"));
        assert!(is_login_url("https://oauth.xfinity.com/oauth/authorize"));
        assert!(is_login_url("https://idm.example.com/sign-in"));
        // The endpoint we actually want must never look like a login page.
        assert!(!is_login_url(
            "https://www.xfinity.com/digital/service/api/BillingInfo/downloadStatement"
        ));
        assert!(!is_login_url(
            "https://www.xfinity.com/digital/service/api/BillingInfo/billingSummary"
        ));
    }

    // ---- End-to-end over a loopback stub. These drive the whole of
    // `download_statement` — real reqwest, real redirect following, real
    // header/body handling — so the guard is verified through the code path a
    // live call would take, not just through `decode_pdf` in isolation. No
    // network, no keychain: 127.0.0.1 only.

    use std::io::{BufRead, BufReader, Write as _};
    use std::net::TcpListener;

    /// Serve exactly `responses.len()` requests, in order, then stop.
    /// Returns the base URL to point a client at.
    fn stub_server(responses: Vec<String>) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
        let base = format!("http://{}", listener.local_addr().unwrap());
        std::thread::spawn(move || {
            for raw in responses {
                let Ok((mut sock, _)) = listener.accept() else {
                    return;
                };
                // Drain the request head so the client isn't writing into a
                // closed socket; the body length doesn't matter to us.
                let mut reader = BufReader::new(sock.try_clone().unwrap());
                let mut line = String::new();
                let mut content_len = 0usize;
                while reader.read_line(&mut line).unwrap_or(0) > 0 {
                    let l = line.trim_end().to_string();
                    if let Some(v) = l.to_ascii_lowercase().strip_prefix("content-length:") {
                        content_len = v.trim().parse().unwrap_or(0);
                    }
                    line.clear();
                    if l.is_empty() {
                        break;
                    }
                }
                if content_len > 0 {
                    let mut body = vec![0u8; content_len];
                    use std::io::Read as _;
                    let _ = reader.read_exact(&mut body);
                }
                let _ = sock.write_all(raw.as_bytes());
                let _ = sock.flush();
            }
        });
        base
    }

    fn http_response(status_line: &str, content_type: &str, body: &[u8]) -> String {
        // The bodies under test are ASCII-safe (HTML, JSON, and a synthetic
        // PDF that is 7-bit clean), so building the frame as a String is fine.
        format!(
            "HTTP/1.1 {status_line}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            String::from_utf8_lossy(body)
        )
    }

    #[test]
    fn e2e_html_login_page_on_200_exits_3_and_returns_no_bytes() {
        // The exact failure this guard exists for: token expired, Xfinity
        // answers 200 + the sign-in page. Must be auth (exit 3), and must not
        // hand any bytes back to the writer.
        let base = stub_server(vec![http_response(
            "200 OK",
            "text/html; charset=utf-8",
            LOGIN_HTML_FIXTURE.as_bytes(),
        )]);
        let err = Xfinity::for_test(&base)
            .download_statement("2026-07-15")
            .unwrap_err();
        assert_eq!(err.exit_code(), 3, "expected auth exit 3, got {err}");
        assert!(matches!(err, AppError::Auth(_)));
    }

    #[test]
    fn e2e_redirect_to_sign_in_exits_3() {
        // reqwest follows the 302, so the tell is the *final* URL.
        let base = stub_server(vec![
            format!(
                "HTTP/1.1 302 Found\r\nLocation: /login?state=abc\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            ),
            http_response(
                "200 OK",
                "text/html; charset=utf-8",
                LOGIN_HTML_FIXTURE.as_bytes(),
            ),
        ]);
        let err = Xfinity::for_test(&base)
            .download_statement("2026-07-15")
            .unwrap_err();
        assert_eq!(err.exit_code(), 3, "expected auth exit 3, got {err}");
        // The redirect target's query string must not leak into the message.
        assert!(!format!("{err}").contains("state=abc"));
    }

    #[test]
    fn e2e_pdf_body_round_trips_intact() {
        let base = stub_server(vec![http_response(
            "200 OK",
            "application/pdf",
            PDF_FIXTURE,
        )]);
        let out = Xfinity::for_test(&base)
            .download_statement("2026-07-15")
            .expect("a real PDF body must succeed");
        assert_eq!(out, PDF_FIXTURE, "bytes must survive the round trip");
    }

    #[test]
    fn e2e_json_envelope_round_trips_intact() {
        let base = stub_server(vec![http_response(
            "200 OK",
            "application/json",
            JSON_FIXTURE.as_bytes(),
        )]);
        let out = Xfinity::for_test(&base)
            .download_statement("2026-07-15")
            .expect("a base64 envelope must succeed");
        assert_eq!(out, PDF_FIXTURE);
    }

    #[test]
    fn e2e_401_exits_3_and_404_exits_4() {
        let base = stub_server(vec![http_response("401 Unauthorized", "text/plain", b"")]);
        let err = Xfinity::for_test(&base)
            .download_statement("2026-07-15")
            .unwrap_err();
        assert_eq!(err.exit_code(), 3);

        let base = stub_server(vec![http_response("404 Not Found", "text/plain", b"")]);
        let err = Xfinity::for_test(&base)
            .download_statement("2026-07-15")
            .unwrap_err();
        // A wrong path is "not found" (exit 4), and the message must name the
        // DevTools recipe, since the endpoint is UNVERIFIED-LIVE.
        assert_eq!(err.exit_code(), 4, "got {err}");
        assert!(format!("{err}").contains("DevTools"));
    }

    #[test]
    fn redacted_url_drops_query_and_fragment() {
        // Sign-in redirects carry state/redirect_uri blobs; error text ends up
        // in transcripts, so the query string must not ride along.
        assert_eq!(
            redacted_url("https://login.xfinity.com/login?state=SECRET&redirect_uri=x#frag"),
            "https://login.xfinity.com/login"
        );
    }
}
