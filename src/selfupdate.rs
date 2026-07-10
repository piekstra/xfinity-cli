//! `xfin self-update` — replace the running binary with the latest GitHub
//! release.
//!
//! Discovery uses the public GitHub Releases API for [`REPO`]; no auth needed
//! for a public repo. We pick the release asset whose name matches this
//! platform (`xfin-<os>-<arch>.tar.gz`), download it, extract the single `xfin`
//! entry, and atomically swap it in over the current executable (write to a
//! temp file on the same filesystem, then rename). A crash mid-flight leaves
//! either the old binary or the new one, never a half-written one.

use std::io::Read;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::error::AppError;

pub const REPO: &str = "piekstra/xfinity-cli";

/// The current binary version, from Cargo at build time.
pub const CURRENT: &str = env!("CARGO_PKG_VERSION");

pub struct UpdateOutcome {
    pub current: String,
    pub latest: String,
    pub updated: bool,
    pub already_current: bool,
    pub installed_at: Option<String>,
}

/// Asset basename for this platform, e.g. `xfin-macos-aarch64.tar.gz`.
fn asset_name() -> Result<String, AppError> {
    let os = match std::env::consts::OS {
        "macos" => "macos",
        "linux" => "linux",
        "windows" => "windows",
        other => {
            return Err(AppError::Other(format!(
                "unsupported OS for self-update: {other}"
            )))
        }
    };
    let arch = match std::env::consts::ARCH {
        "x86_64" => "x86_64",
        "aarch64" => "aarch64",
        other => {
            return Err(AppError::Other(format!(
                "unsupported architecture for self-update: {other}"
            )))
        }
    };
    Ok(format!("xfin-{os}-{arch}.tar.gz"))
}

fn http() -> Result<reqwest::blocking::Client, AppError> {
    reqwest::blocking::Client::builder()
        .user_agent(format!("xfin/{CURRENT}"))
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .map_err(|e| AppError::Other(format!("failed to build HTTP client: {e}")))
}

/// Normalize a version-ish string for comparison (`v0.2.0` -> `0.2.0`).
fn norm(v: &str) -> String {
    v.trim().trim_start_matches('v').to_string()
}

/// Query the latest release: returns (tag, assets array).
fn latest_release(client: &reqwest::blocking::Client) -> Result<(String, Vec<Value>), AppError> {
    let url = format!("https://api.github.com/repos/{REPO}/releases/latest");
    let resp = client
        .get(&url)
        .header("Accept", "application/vnd.github+json")
        .send()?;
    let status = resp.status();
    if status.as_u16() == 404 {
        return Err(AppError::NotFound(
            "no published release yet for xfinity-cli".into(),
        ));
    }
    if !status.is_success() {
        return Err(AppError::Network(format!(
            "GitHub releases API HTTP {}",
            status.as_u16()
        )));
    }
    let body: Value = resp.json()?;
    let tag = body
        .get("tag_name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| AppError::Other("release response had no tag_name".into()))?
        .to_string();
    let assets = body
        .get("assets")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    Ok((tag, assets))
}

fn download(client: &reqwest::blocking::Client, url: &str) -> Result<Vec<u8>, AppError> {
    let resp = client
        .get(url)
        .header("Accept", "application/octet-stream")
        .send()?;
    if !resp.status().is_success() {
        return Err(AppError::Network(format!(
            "downloading asset: HTTP {}",
            resp.status().as_u16()
        )));
    }
    Ok(resp.bytes()?.to_vec())
}

/// Extract the `xfin` binary bytes from a gzip'd tarball.
fn extract_binary(tar_gz: &[u8]) -> Result<Vec<u8>, AppError> {
    let gz = flate2::read::GzDecoder::new(tar_gz);
    let mut archive = tar::Archive::new(gz);
    let entries = archive
        .entries()
        .map_err(|e| AppError::Other(format!("reading tarball: {e}")))?;
    for entry in entries {
        let mut entry = entry.map_err(|e| AppError::Other(format!("reading tar entry: {e}")))?;
        let path = entry
            .path()
            .map_err(|e| AppError::Other(format!("reading tar entry path: {e}")))?;
        let is_bin = path
            .file_name()
            .and_then(|n| n.to_str())
            .map(|n| n == "xfin")
            .unwrap_or(false);
        if is_bin {
            let mut buf = Vec::new();
            entry
                .read_to_end(&mut buf)
                .map_err(|e| AppError::Other(format!("extracting xfin: {e}")))?;
            return Ok(buf);
        }
    }
    Err(AppError::Other(
        "release tarball did not contain an `xfin` binary".into(),
    ))
}

/// Atomically replace `dest` with `bytes` (write temp on same dir, then rename).
fn install(dest: &Path, bytes: &[u8]) -> Result<(), AppError> {
    let dir = dest
        .parent()
        .ok_or_else(|| AppError::Other("cannot resolve install directory".into()))?;
    let tmp = dir.join(format!(".xfin-update-{}", std::process::id()));
    std::fs::write(&tmp, bytes)
        .map_err(|e| AppError::Other(format!("writing update to {}: {e}", tmp.display())))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&tmp)
            .map_err(|e| AppError::Other(format!("stat temp binary: {e}")))?
            .permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&tmp, perms)
            .map_err(|e| AppError::Other(format!("chmod temp binary: {e}")))?;
    }
    std::fs::rename(&tmp, dest).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        AppError::Other(format!(
            "replacing {}: {e} (need write permission to that path)",
            dest.display()
        ))
    })
}

fn current_exe() -> Result<PathBuf, AppError> {
    let p = std::env::current_exe()
        .map_err(|e| AppError::Other(format!("cannot locate the running binary: {e}")))?;
    // Resolve symlinks (e.g. a Homebrew shim) so we replace the real file.
    Ok(std::fs::canonicalize(&p).unwrap_or(p))
}

pub fn run(check_only: bool) -> Result<UpdateOutcome, AppError> {
    let client = http()?;
    let (tag, assets) = latest_release(&client)?;
    let latest = norm(&tag);
    let already = norm(CURRENT) == latest;

    if check_only || already {
        return Ok(UpdateOutcome {
            current: CURRENT.to_string(),
            latest,
            updated: false,
            already_current: already,
            installed_at: None,
        });
    }

    let want = asset_name()?;
    let asset = assets
        .iter()
        .find(|a| a.get("name").and_then(|n| n.as_str()) == Some(want.as_str()))
        .ok_or_else(|| AppError::NotFound(format!("release {tag} has no asset named {want}")))?;
    let url = asset
        .get("browser_download_url")
        .and_then(|u| u.as_str())
        .ok_or_else(|| AppError::Other("asset had no download URL".into()))?;

    let tarball = download(&client, url)?;
    let binary = extract_binary(&tarball)?;
    let dest = current_exe()?;
    install(&dest, &binary)?;

    Ok(UpdateOutcome {
        current: CURRENT.to_string(),
        latest,
        updated: true,
        already_current: false,
        installed_at: Some(dest.display().to_string()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn norm_strips_v_prefix() {
        assert_eq!(norm("v1.2.3"), "1.2.3");
        assert_eq!(norm(" 1.2.3 "), "1.2.3");
    }

    #[test]
    fn asset_name_is_platform_shaped() {
        // Just assert it builds a plausible name on supported platforms.
        if let Ok(name) = asset_name() {
            assert!(name.starts_with("xfin-"));
            assert!(name.ends_with(".tar.gz"));
        }
    }
}
