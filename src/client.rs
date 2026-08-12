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

/// Self-care (SSM) API host used by the statement-PDF download flow. The
/// legacy `customer.xfinity.com` billing surface still calls into this host
/// (`api.sc.xfinity.com`) even for accounts migrated to the new account
/// experience — it is where the signed CloudFront URLs live.
/// Overridable with `$XFINITY_SC_API_HOST` for probing.
pub fn sc_api_host() -> String {
    std::env::var("XFINITY_SC_API_HOST")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "https://api.sc.xfinity.com".to_string())
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
    /// Self-care (SSM) API host — see `sc_api_host()`. The download flow lives
    /// here; the two "fat" account endpoints live under `host`.
    sc_host: String,
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
            sc_host: sc_api_host().trim_end_matches('/').to_string(),
            bearer: normalize_bearer(session.expose()),
        })
    }

    /// Build a session pointed at an arbitrary host. Test-only: it lets the
    /// download guard be exercised over a real HTTP round trip to a loopback
    /// stub, without an env var (which would race other tests) and without
    /// ever touching the keychain or the network. Both the account host and
    /// the self-care (SSM) host point at the same stub — the stub can serve
    /// either endpoint path.
    #[cfg(test)]
    fn for_test(host: &str) -> Xfinity {
        let h = host.trim_end_matches('/').to_string();
        Xfinity {
            client: build_client().expect("test client"),
            host: h.clone(),
            sc_host: h,
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
    /// **Two-step flow, confirmed against a live account 2026-08-12** (see
    /// `docs/api.md`). The account-experience UI doesn't own the download —
    /// the legacy `customer.xfinity.com` billing app still does, and it goes
    /// through the self-care (SSM) service on a different host:
    ///
    /// 1. `GET https://api.sc.xfinity.com/session/ssm/bill/pdf
    ///    ?statementDate=MM-DD-YYYY&signed=true` with the same Bearer token
    ///    → JSON `{"cloudfront_url": "https://ssm-prod-billpdf-cache.s3
    ///    .amazonaws.com/…?AWSAccessKeyId=…&Signature=…&x-amz-security-token=…"}`.
    /// 2. `GET <cloudfront_url>` **with no Authorization header** (the S3
    ///    presigned URL carries its own signature; a bearer would fail
    ///    validation) → `application/pdf` bytes.
    ///
    /// Date format matters: the SSM endpoint expects `MM-DD-YYYY`, not the
    /// ISO `YYYY-MM-DD` the rest of the CLI passes around, so the ISO input
    /// is reformatted before being sent.
    ///
    /// Returns the decoded PDF bytes on success. As with the old shape, we
    /// never return bytes that don't start with `%PDF`: an expired token
    /// still gets answered with a sign-in page, and the guards in `decode_pdf`
    /// still apply — see the "Trap: expiry arrives as HTTP 200 + HTML" note
    /// in `docs/api.md`.
    pub fn download_statement(&self, statement_date: &str) -> Result<Vec<u8>, AppError> {
        // Step 1: ask SSM for the presigned CloudFront URL for this statement.
        let path = "ssm/bill/pdf";
        let statement_mmddyyyy = iso_to_mmddyyyy(statement_date).ok_or_else(|| {
            AppError::Usage(format!(
                "statement date {statement_date:?} is not ISO YYYY-MM-DD; \
                 see `xfin billing statements` for valid handles"
            ))
        })?;
        let url = format!("{}/session/ssm/bill/pdf", self.sc_host);
        let resp = self
            .client
            .get(&url)
            .query(&[
                ("statementDate", statement_mmddyyyy.as_str()),
                ("signed", "true"),
            ])
            .header("Authorization", &self.bearer)
            .header("Accept", "application/json, text/plain, */*")
            .header("Referer", "https://customer.xfinity.com/")
            .header("Sec-Fetch-Dest", "empty")
            .header("Sec-Fetch-Mode", "cors")
            .header("Sec-Fetch-Site", "same-site")
            .header("Sec-CH-UA-Mobile", "?0")
            .header("Sec-CH-UA-Platform", "\"macOS\"")
            .header("Sec-CH-UA", sec_ch_ua())
            .send()?;

        let status = resp.status();
        let content_type = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();
        // `reqwest` follows redirects, so an expired token shows up here as
        // the *final* URL being a sign-in page rather than as a 401 on the
        // original one. Capture it before the body is consumed.
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
                "{path} (HTTP 404) — the SSM statement-download endpoint may have moved. \
                 Sign in at https://customer.xfinity.com/billing/services/statement/history, \
                 open DevTools → Network, click \"Statement PDF\" on a statement, and check \
                 the request URL/query/response; then update `docs/api.md` and \
                 `Xfinity::download_statement` in `src/client.rs`."
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
        // A 200 + login page is the auth-expiry tell on this surface too.
        if looks_like_html(&bytes, &content_type) {
            return Err(AppError::Auth(expired_token_msg(
                path,
                "SSM returned an HTML page where a JSON envelope was expected \
                 (an expired session is answered with the sign-in page, not a 401)",
            )));
        }
        let signed_url = extract_signed_url(&bytes, path)?;

        // Step 2: fetch the presigned CloudFront URL. **No Authorization
        // header** — the URL carries its own AWS signature and a bearer would
        // fail validation.
        self.fetch_signed_pdf(&signed_url, path)
    }

    fn fetch_signed_pdf(&self, url: &str, path: &str) -> Result<Vec<u8>, AppError> {
        let resp = self
            .client
            .get(url)
            .header("Accept", "application/pdf, */*")
            .header("Referer", "https://customer.xfinity.com/")
            .header("Sec-Fetch-Dest", "document")
            .header("Sec-Fetch-Mode", "navigate")
            .header("Sec-Fetch-Site", "cross-site")
            .header("Sec-CH-UA-Mobile", "?0")
            .header("Sec-CH-UA-Platform", "\"macOS\"")
            .header("Sec-CH-UA", sec_ch_ua())
            .send()?;

        let status = resp.status();
        let content_type = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();
        let final_url = resp.url().to_string();
        if is_login_url(&final_url) {
            return Err(AppError::Auth(expired_token_msg(
                path,
                &format!(
                    "the presigned URL redirected to a sign-in page ({})",
                    redacted_url(&final_url)
                ),
            )));
        }
        let bytes = resp
            .bytes()
            .map_err(|e| AppError::Upstream(format!("reading presigned {path} body: {e}")))?
            .to_vec();
        if !status.is_success() {
            let hint = std::str::from_utf8(&bytes)
                .map(body_hint)
                .unwrap_or_default();
            return Err(AppError::Upstream(format!(
                "presigned URL for {path} returned HTTP {}{hint}",
                status.as_u16()
            )));
        }
        // Delegate to the same magic-number / HTML guard used everywhere else.
        // The signed URL is a static PDF, so any non-`%PDF` body is a failure.
        decode_pdf(&bytes, &content_type, path)
    }
}

/// Convert an ISO `YYYY-MM-DD` date to the `MM-DD-YYYY` shape the SSM billing
/// endpoint expects. Returns `None` for anything that doesn't fit the ISO
/// shape (all-numeric segments, `4-2-2` widths). Rejecting garbage here means
/// a typo shows up as a usage error, not a 400 from Xfinity's edge.
fn iso_to_mmddyyyy(iso: &str) -> Option<String> {
    let s = iso.trim();
    let mut parts = s.split('-');
    let y = parts.next()?;
    let m = parts.next()?;
    let d = parts.next()?;
    if parts.next().is_some() {
        return None;
    }
    if y.len() != 4 || m.len() != 2 || d.len() != 2 {
        return None;
    }
    if !y.chars().all(|c| c.is_ascii_digit())
        || !m.chars().all(|c| c.is_ascii_digit())
        || !d.chars().all(|c| c.is_ascii_digit())
    {
        return None;
    }
    Some(format!("{m}-{d}-{y}"))
}

/// Extract the presigned CloudFront URL out of the SSM `bill/pdf` response.
/// The observed envelope is `{"cloudfront_url": "https://…"}` — a flat object
/// with a single string field, not the `responseData.data` wrapping the rest
/// of the account-experience surface uses. Tolerate a couple of nearby field
/// names in case the SSM service reshapes the response slightly (`url`,
/// `pdfUrl`, `signedUrl`) — a rename is a common shape drift and the failure
/// mode we want is a loud diagnostic, not a silent 404.
pub(crate) fn extract_signed_url(bytes: &[u8], path: &str) -> Result<String, AppError> {
    let v: Value = serde_json::from_slice(bytes).map_err(|e| {
        AppError::Upstream(format!(
            "{path} returned undecodable JSON: {e} (first bytes: {:?})",
            bytes.iter().take(60).collect::<Vec<_>>()
        ))
    })?;
    let candidates = [
        "/cloudfront_url",
        "/cloudfrontUrl",
        "/url",
        "/pdfUrl",
        "/signedUrl",
    ];
    let url = candidates
        .iter()
        .find_map(|p| v.pointer(p).and_then(|x| x.as_str()))
        .ok_or_else(|| {
            AppError::Upstream(format!(
                "{path} returned JSON without a recognized signed-URL field \
                 (tried {candidates:?}) — inspect against a real request in the browser \
                 and add the field to `src/client.rs::extract_signed_url`"
            ))
        })?;
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return Err(AppError::Upstream(format!(
            "{path} returned a non-http signed URL: {url:?}"
        )));
    }
    Ok(url.to_string())
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

/// Recognize the presigned-URL response body as a PDF, reject an HTML sign-in
/// page as auth failure, and reject anything else as upstream error. Split
/// out for testability — the shape detection is the load-bearing bit of the
/// download flow.
///
/// Order matters: the `%PDF` magic number is authoritative (a real PDF is a
/// PDF whatever the `Content-Type` claims), then HTML is rejected. Nothing
/// reaches the caller — and so nothing reaches the filesystem — unless it
/// starts with `%PDF`. The two-step SSM flow means the JSON envelope lives
/// upstream of this call (see `extract_signed_url`); by the time we get here
/// the body should be raw PDF bytes from the presigned S3 URL.
pub(crate) fn decode_pdf(
    bytes: &[u8],
    content_type: &str,
    path: &str,
) -> Result<Vec<u8>, AppError> {
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

    Err(AppError::Upstream(format!(
        "{path}: unrecognized response (content-type {content_type:?}, {} bytes; \
         does not start with `%PDF`)",
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

    // ---- ISO-to-SSM date conversion: the endpoint expects MM-DD-YYYY, not
    // the ISO YYYY-MM-DD the CLI passes around. A typo here becomes a 400
    // from Xfinity's edge, so this is worth guarding at the boundary.

    #[test]
    fn iso_to_mmddyyyy_rewrites_a_valid_iso_date() {
        assert_eq!(iso_to_mmddyyyy("2026-08-09").as_deref(), Some("08-09-2026"));
        assert_eq!(iso_to_mmddyyyy("2026-01-01").as_deref(), Some("01-01-2026"));
    }

    #[test]
    fn iso_to_mmddyyyy_rejects_anything_that_isnt_iso_shape() {
        // Wrong widths, non-numeric, extra segments — all a usage error, not
        // something we should send to Xfinity and let 400 back.
        for bad in [
            "",
            "2026",
            "2026-08",
            "26-08-09",
            "2026-8-9",
            "2026/08/09",
            "08-09-2026",
            "abcd-ef-gh",
            "2026-08-09-01",
        ] {
            assert!(iso_to_mmddyyyy(bad).is_none(), "should reject {bad:?}");
        }
    }

    // ---- extract_signed_url: the SSM envelope is a flat object with a
    // `cloudfront_url`. A rename or a reshape here is the most likely mid-run
    // upstream drift; missing it must fail loud, not silently.

    #[test]
    fn extract_signed_url_reads_the_cloudfront_url_field() {
        let body = br#"{"cloudfront_url":"https://ssm-prod-billpdf-cache.s3.amazonaws.com/abc.pdf?sig=x"}"#;
        let url = extract_signed_url(body, "ssm/bill/pdf").unwrap();
        assert_eq!(
            url,
            "https://ssm-prod-billpdf-cache.s3.amazonaws.com/abc.pdf?sig=x"
        );
    }

    #[test]
    fn extract_signed_url_tolerates_camelcase_and_alt_field_names() {
        // Xfinity's other endpoints are camelCase; guard against a rename.
        for body in [
            r#"{"cloudfrontUrl":"https://x/a.pdf"}"#,
            r#"{"url":"https://x/a.pdf"}"#,
            r#"{"pdfUrl":"https://x/a.pdf"}"#,
            r#"{"signedUrl":"https://x/a.pdf"}"#,
        ] {
            let url = extract_signed_url(body.as_bytes(), "ssm/bill/pdf").unwrap();
            assert_eq!(url, "https://x/a.pdf", "for body {body}");
        }
    }

    #[test]
    fn extract_signed_url_errors_when_no_url_field_is_present() {
        let body = br#"{"unrelated":"x"}"#;
        let err = extract_signed_url(body, "ssm/bill/pdf").unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("recognized signed-URL field"),
            "unexpected error: {msg}"
        );
        assert_eq!(err.exit_code(), 5);
    }

    #[test]
    fn extract_signed_url_rejects_a_non_http_scheme() {
        // A stray `file://` or `javascript:` in the envelope must not become
        // a follow-up fetch — that would happily open a local file or worse.
        let body = br#"{"cloudfront_url":"file:///etc/passwd"}"#;
        let err = extract_signed_url(body, "ssm/bill/pdf").unwrap_err();
        assert_eq!(err.exit_code(), 5, "got {err}");
    }

    // ---- decode_pdf: the shape detection is the load-bearing bit of the
    // download flow. On the confirmed 2-step SSM path this only sees the
    // final presigned-URL body — raw PDF or an HTML sign-in page — so it's
    // simpler than the pre-fix branch was.

    #[test]
    fn decode_pdf_returns_raw_bytes_when_content_type_is_pdf() {
        let bytes = b"%PDF-1.7\nfake body";
        let out = decode_pdf(bytes, "application/pdf", "ssm/bill/pdf").unwrap();
        assert_eq!(out, bytes);
    }

    #[test]
    fn decode_pdf_errors_on_undecodable_response() {
        let err = decode_pdf(b"not a pdf", "application/octet-stream", "ssm/bill/pdf").unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("unrecognized response"),
            "unexpected error: {msg}"
        );
        // Garbage is an upstream problem (exit 5), not an auth problem.
        assert_eq!(err.exit_code(), 5);
    }

    // ---- The auth-expiry guard. This surface answers an expired token with a
    // 200 + the sign-in page, so "is this actually a PDF?" is the only thing
    // standing between the user and an HTML file named `statement.pdf`.

    const PDF_FIXTURE: &[u8] = include_bytes!("../tests/fixtures/statement_min.pdf");
    const LOGIN_HTML_FIXTURE: &str =
        include_str!("../tests/fixtures/download_statement_login_page.html");

    #[test]
    fn fixture_pdf_round_trips_as_raw_bytes() {
        let out = decode_pdf(PDF_FIXTURE, "application/pdf", "ssm/bill/pdf").unwrap();
        assert_eq!(out, PDF_FIXTURE);
        assert!(out.starts_with(b"%PDF"));
    }

    #[test]
    fn login_page_fixture_is_an_auth_failure_not_a_file() {
        let err = decode_pdf(
            LOGIN_HTML_FIXTURE.as_bytes(),
            "text/html; charset=utf-8",
            "ssm/bill/pdf",
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
            "ssm/bill/pdf",
        )
        .unwrap_err();
        assert_eq!(err.exit_code(), 3);
    }

    #[test]
    fn a_real_pdf_is_accepted_even_when_content_type_claims_html() {
        // The magic number is evidence; the header is only a claim. A CDN that
        // mislabels the body must not cost the user their download.
        let out = decode_pdf(PDF_FIXTURE, "text/html", "ssm/bill/pdf").unwrap();
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
        // The endpoints we actually want must never look like a login page.
        assert!(!is_login_url(
            "https://www.xfinity.com/digital/service/api/BillingInfo/billingSummary"
        ));
        assert!(!is_login_url(
            "https://api.sc.xfinity.com/session/ssm/bill/pdf"
        ));
    }

    // ---- End-to-end over a loopback stub. These drive the whole of
    // `download_statement` — real reqwest, real redirect following, real
    // header/body handling — so the guard is verified through the code path a
    // live call would take, not just through `decode_pdf` in isolation. No
    // network, no keychain: 127.0.0.1 only.
    //
    // The download flow is now two requests (SSM JSON → presigned CloudFront
    // URL → PDF), so stubs supply two responses and the presigned URL in the
    // first response points at the same stub base.

    use std::io::{BufRead, BufReader, Write as _};
    use std::net::TcpListener;

    /// Serve exactly `responses.len()` requests, in order, then stop.
    /// Returns the base URL to point a client at.
    fn stub_server(responses: Vec<String>) -> String {
        stub_server_from(|_base| responses)
    }

    /// Same, but the response list can reference the stub's own base URL —
    /// needed for the two-step download flow, where the SSM response embeds
    /// the presigned URL of the follow-up fetch and both live on this stub.
    fn stub_server_from(build: impl FnOnce(&str) -> Vec<String>) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
        let base = format!("http://{}", listener.local_addr().unwrap());
        let responses = build(&base);
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

    /// The step-1 SSM response that points at `signed_pdf_url` for step 2.
    fn ssm_json_response(signed_pdf_url: &str) -> String {
        let body = format!(r#"{{"cloudfront_url":"{signed_pdf_url}"}}"#);
        http_response("200 OK", "application/json", body.as_bytes())
    }

    #[test]
    fn e2e_two_step_flow_returns_the_pdf_bytes() {
        // The happy path: SSM answers with a signed URL that points back at
        // the same stub, the follow-up fetch delivers the PDF, and the bytes
        // survive the round trip intact.
        let base = stub_server_from(|b| {
            vec![
                ssm_json_response(&format!("{b}/signed/statement.pdf")),
                http_response("200 OK", "application/pdf", PDF_FIXTURE),
            ]
        });
        let out = Xfinity::for_test(&base)
            .download_statement("2026-08-09")
            .expect("PDF round trip");
        assert_eq!(out, PDF_FIXTURE);
    }

    #[test]
    fn e2e_html_login_page_on_ssm_step_exits_3_and_returns_no_bytes() {
        // Token expired, SSM answers 200 + the sign-in page (not JSON, not
        // 401). Must be auth (exit 3), and must not hand any bytes back.
        let base = stub_server(vec![http_response(
            "200 OK",
            "text/html; charset=utf-8",
            LOGIN_HTML_FIXTURE.as_bytes(),
        )]);
        let err = Xfinity::for_test(&base)
            .download_statement("2026-08-09")
            .unwrap_err();
        assert_eq!(err.exit_code(), 3, "expected auth exit 3, got {err}");
        assert!(matches!(err, AppError::Auth(_)));
    }

    #[test]
    fn e2e_html_login_page_on_presigned_step_exits_3() {
        // Rarer failure mode: SSM answers with a signed URL, but the S3-ish
        // follow-up hands back HTML anyway. Same guard, same exit.
        let base = stub_server_from(|b| {
            vec![
                ssm_json_response(&format!("{b}/signed/statement.pdf")),
                http_response(
                    "200 OK",
                    "text/html; charset=utf-8",
                    LOGIN_HTML_FIXTURE.as_bytes(),
                ),
            ]
        });
        let err = Xfinity::for_test(&base)
            .download_statement("2026-08-09")
            .unwrap_err();
        assert_eq!(err.exit_code(), 3, "expected auth exit 3, got {err}");
    }

    #[test]
    fn e2e_redirect_to_sign_in_on_ssm_step_exits_3() {
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
            .download_statement("2026-08-09")
            .unwrap_err();
        assert_eq!(err.exit_code(), 3, "expected auth exit 3, got {err}");
        // The redirect target's query string must not leak into the message.
        assert!(!format!("{err}").contains("state=abc"));
    }

    #[test]
    fn e2e_401_on_ssm_step_exits_3() {
        let base = stub_server(vec![http_response("401 Unauthorized", "text/plain", b"")]);
        let err = Xfinity::for_test(&base)
            .download_statement("2026-08-09")
            .unwrap_err();
        assert_eq!(err.exit_code(), 3);
    }

    #[test]
    fn e2e_404_on_ssm_step_exits_4_and_names_the_recipe() {
        let base = stub_server(vec![http_response("404 Not Found", "text/plain", b"")]);
        let err = Xfinity::for_test(&base)
            .download_statement("2026-08-09")
            .unwrap_err();
        // Wrong path is "not found" (exit 4). The message must name the recipe
        // for confirming the new URL — the last time the endpoint moved, the
        // agent chasing the fix needed exactly that pointer.
        assert_eq!(err.exit_code(), 4, "got {err}");
        assert!(format!("{err}").contains("DevTools"));
        assert!(format!("{err}").contains("Statement PDF"));
    }

    #[test]
    fn e2e_usage_error_on_bad_date_before_the_network() {
        // An ISO typo must never hit the network — Xfinity's 400 is opaque,
        // and the retry loop in `--all` would hammer it. Fail early, exit 2.
        // Point at 127.0.0.1:1 (closed) so any accidental network call fails
        // loudly rather than succeeding against a real host.
        let err = Xfinity::for_test("http://127.0.0.1:1")
            .download_statement("2026/08/09")
            .unwrap_err();
        assert_eq!(err.exit_code(), 2, "got {err}");
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
