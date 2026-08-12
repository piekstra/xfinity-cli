//! Offline black-box tests of the command surface. No network, no keychain:
//! every case here is `--help`, argument validation, or a discovery command,
//! so nothing prompts, hangs, or hits the portal.

use assert_cmd::Command;
use predicates::str::contains;

fn xfin() -> Command {
    Command::cargo_bin("xfin").expect("binary builds")
}

#[test]
fn top_level_help_lists_billing_download() {
    // The new download subcommand has to show up under `xfin billing --help`
    // — this catches a rename or a missing clap wiring.
    let out = xfin().args(["billing", "--help"]).assert().success();
    let text = String::from_utf8_lossy(&out.get_output().stdout).to_string();
    assert!(
        text.contains("download"),
        "billing --help missing download: {text}"
    );
    // The `get` visible alias mirrors pmac's downloads and is documented in
    // the family surface — keep it advertised.
    assert!(
        text.contains("get"),
        "billing --help missing `get` alias: {text}"
    );
}

#[test]
fn billing_download_help_documents_the_new_flags() {
    let out = xfin()
        .args(["billing", "download", "--help"])
        .assert()
        .success();
    let text = String::from_utf8_lossy(&out.get_output().stdout).to_string();
    for flag in ["--all", "--output", "--since", "--until", "--limit"] {
        assert!(
            text.contains(flag),
            "billing download --help missing {flag}: {text}"
        );
    }
}

#[test]
fn billing_download_get_alias_also_renders_help() {
    xfin().args(["billing", "get", "--help"]).assert().success();
}

#[test]
fn billing_download_requires_an_id_or_all() {
    // Neither an id nor --all is a usage error (exit code 2), caught before
    // any network or keychain access.
    xfin()
        .args(["billing", "download"])
        .assert()
        .code(2)
        .stderr(contains("--all"));
}

#[test]
fn billing_download_id_and_all_conflict() {
    // clap rejects the contradiction (exit code 2) before the command runs.
    xfin()
        .args(["billing", "download", "2026-07-15", "--all"])
        .assert()
        .code(2);
}

#[test]
fn billing_download_all_refuses_to_stream_to_stdout() {
    // A batch cannot write concatenated PDFs to a single stream — reject
    // early with a usage error rather than clobbering files.
    xfin()
        .args(["billing", "download", "--all", "-o", "-"])
        .assert()
        .code(2)
        .stderr(contains("stdout"));
}

#[test]
fn billing_download_rejects_a_non_iso_since() {
    xfin()
        .args(["billing", "download", "--all", "--since", "07/15/2026"])
        .assert()
        .code(2);
}

#[test]
fn billing_download_rejects_an_inverted_date_range() {
    xfin()
        .args([
            "billing",
            "download",
            "--all",
            "--since",
            "2026-08-01",
            "--until",
            "2026-01-01",
        ])
        .assert()
        .code(2)
        .stderr(contains("is after"));
}

#[test]
fn billing_statement_still_reports_not_available_and_points_to_download() {
    // The existing `statement <id>` metadata command is still not mapped on
    // the new account experience, but its error should now name the download
    // subcommand as the alternate — a regression here would drop the
    // discoverability of the new feature.
    xfin()
        .args(["billing", "statement", "any"])
        .assert()
        .code(1)
        .stderr(contains("billing download"));
}
