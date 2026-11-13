mod support;

use std::sync::atomic::{AtomicBool, Ordering};

use app_update::http::{Client, expected_sum};
use app_update::release::parse_latest;
use app_update::{AppInfo, UpdateError};
use sha2::{Digest, Sha256};
use support::serve;

const FURNICA: AppInfo = AppInfo {
    name: "furnica",
    display_name: "Furnica",
    repo: "nazdravan/furnica",
    version: "3.0.4",
    windows_exe: "Furnica.exe",
    msi_registry_key: "Software\\nazdravan\\Furnica",
    deb_package: "furnica",
    mac_bundle: "Furnica.app",
};

const LATEST: &str = include_str!("fixtures/latest.json");
const LATEST_PATH: &str = "/repos/nazdravan/furnica/releases/latest";

fn hex(data: &[u8]) -> String {
    Sha256::digest(data)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn tmp(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("app-update-http-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn latest_from_server() {
    let base = serve(vec![(
        LATEST_PATH,
        200,
        vec![("Content-Type", "application/json".into())],
        LATEST.as_bytes().to_vec(),
    )]);
    let r = Client::with_base(&FURNICA, &base).latest().unwrap();
    assert_eq!(r.version, semver::Version::new(3, 0, 5));
}

#[test]
fn rate_limited_is_reported() {
    let base = serve(vec![(
        LATEST_PATH,
        403,
        vec![("x-ratelimit-remaining", "0".into())],
        b"{}".to_vec(),
    )]);
    assert_eq!(
        Client::with_base(&FURNICA, &base).latest().unwrap_err(),
        UpdateError::RateLimited
    );
}

#[test]
fn plain_403_is_not_rate_limit() {
    let base = serve(vec![(LATEST_PATH, 403, vec![], b"{}".to_vec())]);
    assert!(matches!(
        Client::with_base(&FURNICA, &base).latest().unwrap_err(),
        UpdateError::BadResponse(_)
    ));
}

#[test]
fn not_found_is_reported() {
    let base = serve(vec![(LATEST_PATH, 404, vec![], b"{}".to_vec())]);
    assert_eq!(
        Client::with_base(&FURNICA, &base).latest().unwrap_err(),
        UpdateError::NotFound
    );
}

#[test]
fn connection_refused_is_network() {
    // Port 1 on loopback: nothing listens there.
    let c = Client::with_base(&FURNICA, "http://127.0.0.1:1");
    assert!(matches!(c.latest().unwrap_err(), UpdateError::Network(_)));
}

fn data_300k() -> Vec<u8> {
    (0..300_000u32).map(|i| (i % 251) as u8).collect()
}

#[test]
fn download_ok() {
    let data = data_300k();
    let base = serve(vec![("/f.bin", 200, vec![], data.clone())]);
    let dir = tmp("ok");
    let dest = dir.join("f.bin");
    let mut last = (0u64, None);
    let cancel = AtomicBool::new(false);
    Client::with_base(&FURNICA, &base)
        .download(
            &format!("{base}/f.bin"),
            &dest,
            &hex(&data),
            &mut |d, t| last = (d, t),
            &cancel,
        )
        .unwrap();
    assert_eq!(std::fs::read(&dest).unwrap(), data);
    assert!(!dest.with_extension("part").exists());
    assert_eq!(last, (300_000, Some(300_000)));
}

#[test]
fn download_follows_redirect() {
    let data = data_300k();
    let base = serve(vec![
        ("/a", 302, vec![("Location", "/b".into())], vec![]),
        ("/b", 200, vec![], data.clone()),
    ]);
    let dir = tmp("redir");
    let dest = dir.join("f.bin");
    let cancel = AtomicBool::new(false);
    Client::with_base(&FURNICA, &base)
        .download(
            &format!("{base}/a"),
            &dest,
            &hex(&data),
            &mut |_, _| {},
            &cancel,
        )
        .unwrap();
    assert_eq!(std::fs::read(&dest).unwrap(), data);
}

#[test]
fn redirect_to_remote_http_is_refused() {
    let base = serve(vec![(
        "/a",
        302,
        vec![("Location", "http://example.com/x".into())],
        vec![],
    )]);
    let dir = tmp("redir-bad");
    let dest = dir.join("f.bin");
    let cancel = AtomicBool::new(false);
    let e = Client::with_base(&FURNICA, &base)
        .download(&format!("{base}/a"), &dest, "00", &mut |_, _| {}, &cancel)
        .unwrap_err();
    assert!(matches!(e, UpdateError::Network(_)), "{e:?}");
    assert!(!dest.exists() && !dest.with_extension("part").exists());
}

#[test]
fn download_mismatch_leaves_nothing() {
    let data = data_300k();
    let base = serve(vec![("/f.bin", 200, vec![], data)]);
    let dir = tmp("mismatch");
    let dest = dir.join("f.bin");
    let cancel = AtomicBool::new(false);
    let e = Client::with_base(&FURNICA, &base)
        .download(
            &format!("{base}/f.bin"),
            &dest,
            &"0".repeat(64),
            &mut |_, _| {},
            &cancel,
        )
        .unwrap_err();
    assert_eq!(e, UpdateError::ChecksumMismatch);
    assert!(!dest.exists());
    assert!(!dest.with_extension("part").exists());
}

#[test]
fn download_cancel() {
    let data = data_300k();
    let base = serve(vec![("/f.bin", 200, vec![], data.clone())]);
    let dir = tmp("cancel");
    let dest = dir.join("f.bin");
    let cancel = AtomicBool::new(true);
    let e = Client::with_base(&FURNICA, &base)
        .download(
            &format!("{base}/f.bin"),
            &dest,
            &hex(&data),
            &mut |_, _| {},
            &cancel,
        )
        .unwrap_err();
    assert_eq!(e, UpdateError::Cancelled);
    assert!(!dest.exists());
    assert!(!dest.with_extension("part").exists());
}

#[test]
fn download_cancel_midway() {
    let data = data_300k();
    let base = serve(vec![("/f.bin", 200, vec![], data.clone())]);
    let dir = tmp("cancel-mid");
    let dest = dir.join("f.bin");
    let cancel = AtomicBool::new(false);
    let e = Client::with_base(&FURNICA, &base)
        .download(
            &format!("{base}/f.bin"),
            &dest,
            &hex(&data),
            &mut |_, _| cancel.store(true, Ordering::SeqCst),
            &cancel,
        )
        .unwrap_err();
    assert_eq!(e, UpdateError::Cancelled);
    assert!(!dest.exists());
    assert!(!dest.with_extension("part").exists());
}

#[test]
fn missing_sum_is_error() {
    let base = serve(vec![("/sums", 200, vec![], b"abc  other-file\n".to_vec())]);
    let client = Client::with_base(&FURNICA, &base);

    // Release without a SHA256SUMS.txt asset.
    let r = parse_latest(LATEST).unwrap();
    let mut no_file = r.clone();
    no_file.assets.retain(|a| a.name != "SHA256SUMS.txt");
    assert_ne!(
        no_file.assets.len(),
        r.assets.len(),
        "fixture has the sums asset"
    );
    assert_eq!(
        expected_sum(&client, &no_file, "furnica-3.0.5-linux-x86_64.deb").unwrap_err(),
        UpdateError::NoChecksum
    );

    // Sums file without the line.
    let mut with_file = r.clone();
    for a in &mut with_file.assets {
        if a.name == "SHA256SUMS.txt" {
            a.url = format!("{base}/sums");
        }
    }
    assert_eq!(
        expected_sum(&client, &with_file, "furnica-3.0.5-linux-x86_64.deb").unwrap_err(),
        UpdateError::NoChecksum
    );
}

#[test]
fn expected_sum_found() {
    let sums = include_bytes!("fixtures/SHA256SUMS.txt").to_vec();
    let base = serve(vec![("/sums", 200, vec![], sums)]);
    let client = Client::with_base(&FURNICA, &base);
    let mut r = parse_latest(LATEST).unwrap();
    for a in &mut r.assets {
        if a.name == "SHA256SUMS.txt" {
            a.url = format!("{base}/sums");
        }
    }
    assert_eq!(
        expected_sum(&client, &r, "furnica-3.0.5-linux-x86_64.deb").unwrap(),
        "398257f06c85b7d3a430e8f46b0193606236ab9fcb99c2024439b1e83ab33224"
    );
}

#[test]
fn refuses_plain_http_to_remote() {
    let c = Client::new(&FURNICA);
    assert!(matches!(
        c.text("http://example.com/x").unwrap_err(),
        UpdateError::Network(_)
    ));
    let dir = tmp("http-remote");
    let dest = dir.join("x");
    let e = c
        .download(
            "http://example.com/x",
            &dest,
            "00",
            &mut |_, _| {},
            &AtomicBool::new(false),
        )
        .unwrap_err();
    assert!(matches!(e, UpdateError::Network(_)));
}

#[test]
#[ignore = "talks to the real GitHub"]
fn real_github_latest() {
    let r = Client::new(&FURNICA).latest().unwrap();
    assert!(r.version >= semver::Version::new(3, 0, 5), "{}", r.version);
    assert!(!r.assets.is_empty());
}
