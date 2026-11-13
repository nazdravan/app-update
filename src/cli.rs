//! `--check-update`: prints what an update check finds, without starting a GUI.

use crate::http::Client;
use crate::platform::{self, Edition};
use crate::version::{self, offers_update};
use crate::{AppInfo, UpdateError};

/// Returns `None` unless the arguments contain `--check-update`; otherwise prints
/// the result and returns the exit code (0 on success, 1 on error).
pub fn run_cli(app: &AppInfo) -> Option<i32> {
    if !std::env::args_os().any(|a| a == "--check-update") {
        return None;
    }
    attach_console();
    println!("current {}", app.version);
    Some(match check(app) {
        Ok(lines) => {
            for l in lines {
                println!("{l}");
            }
            0
        }
        Err(e) => {
            println!("error {e}");
            1
        }
    })
}

fn check(app: &AppInfo) -> Result<Vec<String>, UpdateError> {
    let release = Client::new(app).latest()?;
    let mut lines = vec![format!("latest {}", release.version)];
    let current = version::parse(app.version)
        .ok_or_else(|| UpdateError::Install(format!("bad running version {}", app.version)))?;
    let kind = platform::detect(app);
    let asset = if offers_update(&release.version, &current) {
        platform::asset_for(app, &kind, Edition::current(), &release)
            .ok()
            .map(|a| a.name.clone())
    } else {
        None
    };
    lines.push(format!("kind {kind:?}"));
    lines.push(format!("asset {}", asset.as_deref().unwrap_or("-")));
    Ok(lines)
}

#[cfg(windows)]
fn attach_console() {
    use windows_sys::Win32::System::Console::{ATTACH_PARENT_PROCESS, AttachConsole};
    // SAFETY: plain Win32 call without pointers; failure (no parent console) is fine.
    unsafe {
        AttachConsole(ATTACH_PARENT_PROCESS);
    }
}

#[cfg(not(windows))]
fn attach_console() {}
