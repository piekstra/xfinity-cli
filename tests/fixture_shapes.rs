//! Contract tests over `tests/fixtures/`.
//!
//! Two jobs:
//!
//! 1. **Shape.** Assert the fields the download path actually reads still exist
//!    in the fixtures, so a rename fails loudly here instead of silently
//!    producing an empty download.
//! 2. **Scrubbing.** Enforce the redaction policy in
//!    `tests/fixtures/README.md`. This is written *positively* — identifying
//!    fields must equal the known dummy values — rather than as a denylist of
//!    real names, because a denylist would commit exactly what it exists to
//!    keep out.

use std::path::PathBuf;

use serde_json::Value;

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn read(name: &str) -> Vec<u8> {
    let p = fixtures_dir().join(name);
    std::fs::read(&p).unwrap_or_else(|e| panic!("reading fixture {}: {e}", p.display()))
}

/// The one account number allowed to appear anywhere in this repo.
const PLACEHOLDER_ACCOUNT: &str = "1234567890";

#[test]
fn pdf_fixture_is_a_real_pdf() {
    let bytes = read("statement_min.pdf");
    assert!(
        bytes.starts_with(b"%PDF"),
        "the PDF fixture must start with the %PDF magic number, or the guard \
         tests are asserting against something that isn't a PDF"
    );
    assert!(bytes.ends_with(b"%%EOF\n"), "PDF fixture is truncated");
    // Small enough to be obviously synthetic. A real statement is orders of
    // magnitude bigger; a big fixture here is a red flag in review.
    assert!(
        bytes.len() < 4096,
        "PDF fixture is {} bytes — too big to be the synthetic sample; a real \
         statement must never be committed",
        bytes.len()
    );
    let text = String::from_utf8_lossy(&bytes);
    assert!(
        text.contains("SYNTHETIC FIXTURE"),
        "the PDF fixture must announce itself as synthetic in its own content"
    );
}

#[test]
fn json_envelope_fixture_keeps_the_fields_the_download_path_reads() {
    let bytes = read("download_statement_base64.json");
    let v: Value = serde_json::from_slice(&bytes).expect("envelope fixture must be valid JSON");

    // `decode_pdf` looks under `responseData.data` first; that nesting is the
    // shape every mapped BillingInfo/* endpoint uses.
    let data = v
        .pointer("/responseData/data")
        .expect("fixture must keep the responseData.data envelope");
    let b64 = data
        .get("statementPdf")
        .and_then(Value::as_str)
        .expect("fixture must keep the statementPdf field decode_pdf reads");
    assert!(!b64.is_empty());
    assert_eq!(
        data.get("mimeType").and_then(Value::as_str),
        Some("application/pdf")
    );
    // null-vs-absent is part of the shape.
    assert!(v.get("errors").is_some(), "`errors` key must be present");
    assert!(v["errors"].is_null(), "`errors` must be null, not absent");
}

#[test]
fn json_envelope_base64_decodes_to_the_pdf_fixture() {
    use base64::{engine::general_purpose::STANDARD, Engine as _};

    let v: Value = serde_json::from_slice(&read("download_statement_base64.json")).unwrap();
    let b64 = v
        .pointer("/responseData/data/statementPdf")
        .and_then(Value::as_str)
        .unwrap();
    let decoded = STANDARD.decode(b64).expect("fixture base64 must decode");
    assert_eq!(
        decoded,
        read("statement_min.pdf"),
        "the envelope fixture and the PDF fixture must stay in sync"
    );
}

#[test]
fn login_page_fixture_is_html_and_carries_no_credentials() {
    let bytes = read("download_statement_login_page.html");
    let text = String::from_utf8(bytes).expect("HTML fixture must be UTF-8");
    let head = text.trim_start().to_ascii_lowercase();
    assert!(
        head.starts_with("<!doctype html"),
        "the login-page fixture must be detectable as HTML by its first bytes — \
         that sniff is what turns an expired session into exit 3"
    );
    // A captured sign-in page can carry a session in a hidden field. Ours must
    // not have anything token-shaped in it.
    for marker in ["Bearer ", "eyJ", "XSRF-TOKEN", "Set-Cookie"] {
        assert!(
            !text.contains(marker),
            "login-page fixture contains `{marker}` — that's a credential, not markup"
        );
    }
}

#[test]
fn fixtures_carry_only_placeholder_identity_values() {
    // Positive assertion: every identity-bearing field that exists must equal
    // the known dummy. See tests/fixtures/README.md.
    let v: Value = serde_json::from_slice(&read("download_statement_base64.json")).unwrap();
    let data = v.pointer("/responseData/data").unwrap();

    if let Some(acct) = data.get("accountNumber").and_then(Value::as_str) {
        assert_eq!(
            acct, PLACEHOLDER_ACCOUNT,
            "fixture account number must be the placeholder"
        );
    }
    if let Some(date) = data.get("statementDate").and_then(Value::as_str) {
        // Shape preserved (ISO), value invented.
        assert_eq!(date.len(), 10, "statementDate must stay ISO-shaped");
        assert!(date.starts_with("2026-"), "use an obviously-fake year");
    }
    if let Some(name) = data.get("fileName").and_then(Value::as_str) {
        assert!(
            name.ends_with(".pdf") && !name.contains('@'),
            "fileName must not carry an identifier"
        );
    }
}

#[test]
fn no_fixture_contains_anything_token_shaped() {
    // Cheap structural guard across every fixture: JWTs and Authorization
    // headers have unmistakable prefixes, and none of them belong in a repo
    // that is public.
    for entry in std::fs::read_dir(fixtures_dir()).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|e| e.to_str()) == Some("pdf") {
            continue; // binary; covered by its own synthetic-content test
        }
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        let text = String::from_utf8_lossy(&std::fs::read(&path).unwrap()).to_string();
        for marker in ["Authorization: Bearer", "eyJhbGciOi", "XSRF-TOKEN="] {
            assert!(
                !text.contains(marker),
                "fixture {name} contains `{marker}` — a credential, not a fixture"
            );
        }
    }
}
