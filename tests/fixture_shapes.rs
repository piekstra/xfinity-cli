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
fn ssm_signed_url_fixture_keeps_the_field_the_download_path_reads() {
    let bytes = read("download_statement_signed_url.json");
    let v: Value = serde_json::from_slice(&bytes).expect("envelope fixture must be valid JSON");

    // `extract_signed_url` reads `cloudfront_url` first (the flat, snake_cased
    // shape observed live 2026-08-12). If the SSM response reshapes, that
    // fallback list picks up a rename — but this fixture must keep the shape
    // we've actually confirmed against a live account.
    let url = v
        .get("cloudfront_url")
        .and_then(Value::as_str)
        .expect("fixture must keep the cloudfront_url field extract_signed_url reads");
    assert!(
        url.starts_with("https://"),
        "cloudfront_url must be an https URL"
    );
    // The presigned URL shape S3 uses. If any of these keys disappear the
    // real endpoint has probably switched signing schemes (v2 → v4, etc.);
    // that would break the follow-up fetch even with a valid step-1 answer.
    for marker in ["AWSAccessKeyId=", "Signature=", "x-amz-security-token="] {
        assert!(
            url.contains(marker),
            "signed URL fixture missing `{marker}` — real SSM URLs carry it"
        );
    }
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
    // Positive assertion: every identity-bearing bit of the signed-URL fixture
    // must be an obvious placeholder. See tests/fixtures/README.md.
    let v: Value = serde_json::from_slice(&read("download_statement_signed_url.json")).unwrap();
    let url = v
        .get("cloudfront_url")
        .and_then(Value::as_str)
        .expect("cloudfront_url present");
    // The path segment of a real SSM URL embeds a per-account opaque hash and
    // the account number. Fixture value must be the placeholder.
    assert!(
        url.contains(PLACEHOLDER_ACCOUNT),
        "signed URL path must carry the placeholder account number, not a real one"
    );
    // The AWS access key id / signature / STS token are credentials. Every
    // one must start with the `PLACEHOLDER_` prefix so a real capture pasted
    // over the fixture fails this test loudly.
    for marker in [
        "AWSAccessKeyId=PLACEHOLDER_",
        "Signature=PLACEHOLDER_",
        "x-amz-security-token=PLACEHOLDER_",
    ] {
        assert!(
            url.contains(marker),
            "signed URL fixture is missing `{marker}` — a real credential may have been pasted in"
        );
    }
    // MM-DD-YYYY date in the filename — the SSM date format. Same pinning
    // rationale as before: a different date suggests a real one leaked.
    assert!(
        url.contains("07-15-2026"),
        "fixture filename must carry the single pinned example date (07-15-2026)"
    );
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
