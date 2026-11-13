//! Self-update for egui desktop apps from GitHub Releases.

use std::fmt;
use std::path::PathBuf;

pub mod http;
pub mod install;
pub mod platform;
pub mod release;
pub mod sums;
pub mod version;

pub use release::{Asset, Release};

/// Static description of the application that wants to update itself.
#[derive(Debug, Clone, Copy)]
pub struct AppInfo {
    /// Lowercase name, used in the User-Agent and asset names (`furnica`).
    pub name: &'static str,
    /// Name shown to the user (`Furnica`).
    pub display_name: &'static str,
    /// GitHub repository, `owner/name`.
    pub repo: &'static str,
    /// Running version, normally `env!("CARGO_PKG_VERSION")`.
    pub version: &'static str,
    /// File name of the Windows executable (`Furnica.exe`).
    pub windows_exe: &'static str,
    /// Registry key written by the MSI (`Software\owner\App`).
    pub msi_registry_key: &'static str,
    /// Debian package name.
    pub deb_package: &'static str,
    /// macOS bundle name (`Furnica.app`).
    pub mac_bundle: &'static str,
}

/// Everything that can go wrong while checking for or installing an update.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateError {
    Network(String),
    RateLimited,
    NotFound,
    BadResponse(String),
    NoAsset,
    NoChecksum,
    ChecksumMismatch,
    Cancelled,
    NotWritable(PathBuf),
    Denied,
    Install(String),
}

impl fmt::Display for UpdateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Network(e) => write!(f, "network error: {e}"),
            Self::RateLimited => f.write_str("GitHub request limit reached, try again later"),
            Self::NotFound => f.write_str("release not found"),
            Self::BadResponse(e) => write!(f, "unexpected response from GitHub: {e}"),
            Self::NoAsset => f.write_str("the release has no file for this edition"),
            Self::NoChecksum => f.write_str("the release has no checksum for this file"),
            Self::ChecksumMismatch => {
                f.write_str("the downloaded file is corrupted (checksum mismatch)")
            }
            Self::Cancelled => f.write_str("cancelled"),
            Self::NotWritable(p) => write!(f, "cannot write to {}", p.display()),
            Self::Denied => f.write_str("permission denied"),
            Self::Install(e) => write!(f, "installation failed: {e}"),
        }
    }
}

impl std::error::Error for UpdateError {}
