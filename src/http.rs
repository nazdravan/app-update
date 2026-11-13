//! HTTPS client for the GitHub API and release assets.
//!
//! TLS is rustls with the bundled webpki roots (no OS certificate store), so the
//! same code works on Windows 7. Plain `http://` is accepted only for loopback
//! (`127.0.0.1`, `localhost`), which the tests use.
//!
//! Redirects are followed by hand, not by ureq: every hop (the first URL and each
//! `Location`) goes through [`check_url`] before a request is made. ureq's own
//! `https_only` switch would also reject the loopback test server.

use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use sha2::{Digest, Sha256};
use ureq::Agent;
use ureq::config::Config;
use ureq::tls::{RootCerts, TlsConfig};

use crate::{AppInfo, Release, UpdateError, release, sums};

const API_BASE: &str = "https://api.github.com";
const MAX_REDIRECTS: usize = 5;
const MAX_TEXT: u64 = 1024 * 1024;
const SUMS_ASSET: &str = "SHA256SUMS.txt";
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const API_TIMEOUT: Duration = Duration::from_secs(30);
const STALL_TIMEOUT: Duration = Duration::from_secs(60);

/// A blocking HTTP client bound to one application and one API base URL.
pub struct Client {
    base: String,
    /// Short requests: API answers and small text files (30 s in total).
    api: Agent,
    /// Downloads: no total limit, only 60 s without data.
    files: Agent,
}

impl Client {
    /// Client for `https://api.github.com`.
    pub fn new(app: &AppInfo) -> Client {
        Client::with_base(app, API_BASE)
    }

    /// Client for another API base URL (tests point it at a loopback server).
    pub fn with_base(app: &AppInfo, base: &str) -> Client {
        let agent = |timeouts: &dyn Fn(
            ureq::config::ConfigBuilder<ureq::typestate::AgentScope>,
        ) -> ureq::config::ConfigBuilder<
            ureq::typestate::AgentScope,
        >| {
            let builder = Config::builder()
                .user_agent(format!("{}/{}", app.name, app.version))
                .tls_config(TlsConfig::builder().root_certs(RootCerts::WebPki).build())
                .http_status_as_error(false)
                .max_redirects(0)
                .timeout_connect(Some(CONNECT_TIMEOUT));
            Agent::new_with_config(timeouts(builder).build())
        };
        Client {
            base: format!("{}/repos/{}", base.trim_end_matches('/'), app.repo),
            api: agent(&|b| b.timeout_global(Some(API_TIMEOUT))),
            files: agent(&|b| b.timeout_per_read(Some(STALL_TIMEOUT))),
        }
    }

    /// Fetches and parses `GET {base}/repos/{repo}/releases/latest`.
    pub fn latest(&self) -> Result<Release, UpdateError> {
        let url = format!("{}/releases/latest", self.base);
        let mut resp = self.get(&self.api, &url, Some("application/vnd.github+json"))?;
        match resp.status().as_u16() {
            200 => {}
            429 => return Err(UpdateError::RateLimited),
            403 if resp
                .headers()
                .get("x-ratelimit-remaining")
                .and_then(|v| v.to_str().ok())
                .map(str::trim)
                == Some("0") =>
            {
                return Err(UpdateError::RateLimited);
            }
            404 => return Err(UpdateError::NotFound),
            s => return Err(UpdateError::BadResponse(format!("HTTP status {s}"))),
        }
        let body = read_text(&mut resp)?;
        release::parse_latest(&body)
    }

    /// Fetches a small text file (at most 1 MiB).
    pub fn text(&self, url: &str) -> Result<String, UpdateError> {
        let mut resp = self.get(&self.api, url, None)?;
        match resp.status().as_u16() {
            200 => read_text(&mut resp),
            404 => Err(UpdateError::NotFound),
            429 => Err(UpdateError::RateLimited),
            s => Err(UpdateError::BadResponse(format!("HTTP status {s}"))),
        }
    }

    /// Downloads `url` to `dest` and verifies its SHA-256 (`sha256`, lowercase hex).
    ///
    /// Data goes to `dest` with the extension `part`; only a complete file whose sum
    /// matches is renamed to `dest`. On a mismatch, a cancel or any error the `.part`
    /// file is deleted. `progress` gets (bytes so far, total if known).
    pub fn download(
        &self,
        url: &str,
        dest: &Path,
        sha256: &str,
        progress: &mut dyn FnMut(u64, Option<u64>),
        cancel: &AtomicBool,
    ) -> Result<(), UpdateError> {
        if cancel.load(Ordering::Relaxed) {
            return Err(UpdateError::Cancelled);
        }
        let part = dest.with_extension("part");
        let result = self.download_to(url, &part, sha256, progress, cancel);
        match &result {
            Ok(()) => fs::rename(&part, dest).map_err(|e| {
                let _ = fs::remove_file(&part);
                UpdateError::Network(format!("cannot save {}: {e}", dest.display()))
            }),
            Err(_) => {
                let _ = fs::remove_file(&part);
                result
            }
        }
    }

    fn download_to(
        &self,
        url: &str,
        part: &Path,
        sha256: &str,
        progress: &mut dyn FnMut(u64, Option<u64>),
        cancel: &AtomicBool,
    ) -> Result<(), UpdateError> {
        let resp = self.get(&self.files, url, None)?;
        match resp.status().as_u16() {
            200 => {}
            404 => return Err(UpdateError::NotFound),
            429 => return Err(UpdateError::RateLimited),
            s => return Err(UpdateError::BadResponse(format!("HTTP status {s}"))),
        }
        let total = resp.body().content_length();
        let mut reader = resp.into_body().into_reader();
        let mut file = File::create(part)
            .map_err(|e| UpdateError::Network(format!("cannot create {}: {e}", part.display())))?;
        let mut hasher = Sha256::new();
        let mut buf = [0u8; 64 * 1024];
        let mut done = 0u64;
        loop {
            if cancel.load(Ordering::Relaxed) {
                return Err(UpdateError::Cancelled);
            }
            let n = reader
                .read(&mut buf)
                .map_err(|e| UpdateError::Network(e.to_string()))?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
            file.write_all(&buf[..n])
                .map_err(|e| UpdateError::Network(format!("write failed: {e}")))?;
            done += n as u64;
            progress(done, total);
        }
        file.flush()
            .map_err(|e| UpdateError::Network(format!("write failed: {e}")))?;
        drop(file);
        if total.is_some_and(|t| t != done) {
            return Err(UpdateError::Network("download cut off".into()));
        }
        let actual: String = hasher
            .finalize()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        if actual.eq_ignore_ascii_case(sha256.trim()) {
            Ok(())
        } else {
            Err(UpdateError::ChecksumMismatch)
        }
    }

    /// GET with manual redirects; every URL is checked before it is requested.
    fn get(
        &self,
        agent: &Agent,
        url: &str,
        accept: Option<&str>,
    ) -> Result<ureq::http::Response<ureq::Body>, UpdateError> {
        let mut url = url.to_string();
        for _ in 0..=MAX_REDIRECTS {
            check_url(&url)?;
            let mut req = agent.get(&url);
            if let Some(a) = accept {
                req = req.header("Accept", a);
            }
            let resp = req
                .call()
                .map_err(|e| UpdateError::Network(e.to_string()))?;
            if matches!(resp.status().as_u16(), 301 | 302 | 303 | 307 | 308) {
                let loc = resp
                    .headers()
                    .get("location")
                    .and_then(|v| v.to_str().ok())
                    .ok_or_else(|| UpdateError::BadResponse("redirect without Location".into()))?;
                url = resolve(&url, loc);
                continue;
            }
            return Ok(resp);
        }
        Err(UpdateError::Network("too many redirects".into()))
    }
}

fn read_text(resp: &mut ureq::http::Response<ureq::Body>) -> Result<String, UpdateError> {
    let bytes = resp
        .body_mut()
        .with_config()
        .limit(MAX_TEXT)
        .read_to_vec()
        .map_err(|e| UpdateError::Network(e.to_string()))?;
    String::from_utf8(bytes).map_err(|_| UpdateError::BadResponse("answer is not UTF-8".into()))
}

/// Returns the `SHA256SUMS.txt` line value for `asset_name` in release `r`.
///
/// `NoChecksum` if the release has no `SHA256SUMS.txt` or the file has no line for the asset.
pub fn expected_sum(client: &Client, r: &Release, asset_name: &str) -> Result<String, UpdateError> {
    let sums_asset = r
        .assets
        .iter()
        .find(|a| a.name == SUMS_ASSET)
        .ok_or(UpdateError::NoChecksum)?;
    let text = client.text(&sums_asset.url)?;
    sums::sha256_for(&text, asset_name).ok_or(UpdateError::NoChecksum)
}

/// Accepts `https://` URLs, and `http://` only for loopback hosts.
fn check_url(url: &str) -> Result<(), UpdateError> {
    if url.starts_with("https://") {
        return Ok(());
    }
    if let Some(rest) = url.strip_prefix("http://") {
        let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
        let host = match authority.rsplit_once('@') {
            Some((_, h)) => h,
            None => authority,
        };
        let host = host.rsplit_once(':').map_or(host, |(h, _)| h);
        if host == "127.0.0.1" || host.eq_ignore_ascii_case("localhost") {
            return Ok(());
        }
    }
    Err(UpdateError::Network(format!(
        "refusing to use an insecure or unsupported URL: {url}"
    )))
}

/// Resolves a `Location` header against the URL it came from.
fn resolve(base: &str, location: &str) -> String {
    if location.contains("://") {
        return location.to_string();
    }
    let scheme_end = base.find("://").map_or(0, |i| i + 3);
    let origin_end = base[scheme_end..]
        .find('/')
        .map_or(base.len(), |i| scheme_end + i);
    if location.starts_with('/') {
        format!("{}{}", &base[..origin_end], location)
    } else {
        let dir_end = base[..base.find(['?', '#']).unwrap_or(base.len())]
            .rfind('/')
            .filter(|&i| i >= origin_end)
            .unwrap_or(origin_end);
        format!("{}/{}", &base[..dir_end], location)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_policy() {
        assert!(check_url("https://api.github.com/x").is_ok());
        assert!(check_url("http://127.0.0.1:8080/x").is_ok());
        assert!(check_url("http://localhost/x").is_ok());
        assert!(check_url("http://example.com/x").is_err());
        assert!(check_url("http://127.0.0.1.evil.com/x").is_err());
        assert!(check_url("http://127.0.0.1@evil.com/x").is_err());
        assert!(check_url("ftp://127.0.0.1/x").is_err());
        assert!(check_url("//evil.com/x").is_err());
    }

    #[test]
    fn resolve_locations() {
        assert_eq!(resolve("https://a.com/p/q", "/z"), "https://a.com/z");
        assert_eq!(resolve("https://a.com/p/q", "r"), "https://a.com/p/r");
        assert_eq!(resolve("https://a.com", "r"), "https://a.com/r");
        assert_eq!(
            resolve("https://a.com/p", "http://b.com/c"),
            "http://b.com/c"
        );
    }
}
