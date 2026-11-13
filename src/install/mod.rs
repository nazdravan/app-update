//! Installing a downloaded update and restarting into it.

use std::path::{Path, PathBuf};

use crate::platform::InstallKind;
use crate::{AppInfo, UpdateError};

#[cfg(any(windows, test))]
pub mod portable;

/// What remains to do once the new copy is in place.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Prepared {
    Restart {
        exe: PathBuf,
    },
    MsiAfterExit {
        msi: PathBuf,
        exe: PathBuf,
        desktop_shortcut: bool,
    },
}

fn unsupported() -> UpdateError {
    UpdateError::Install("unsupported".into())
}

pub fn prepare(app: &AppInfo, kind: &InstallKind, file: &Path) -> Result<Prepared, UpdateError> {
    match kind {
        InstallKind::Portable { dir } => prepare_portable(app, dir, file),
        _ => Err(unsupported()),
    }
}

/// A portable swap is allowed only when the running exe is the shipped one, so a
/// development build (`target\debug\app.exe`) never has its folder replaced.
#[cfg(any(windows, test))]
fn is_shipped_exe(app: &AppInfo, current: &Path) -> bool {
    current
        .file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.eq_ignore_ascii_case(app.windows_exe))
}

#[cfg(windows)]
fn prepare_portable(app: &AppInfo, dir: &Path, file: &Path) -> Result<Prepared, UpdateError> {
    let current = std::env::current_exe()
        .map_err(|e| UpdateError::Install(format!("cannot locate the running program: {e}")))?;
    if !is_shipped_exe(app, &current) {
        return Err(UpdateError::Install(
            "the running program is not the installed copy".into(),
        ));
    }
    let exe = portable::install(file, dir, app.windows_exe)?;
    Ok(Prepared::Restart { exe })
}

#[cfg(not(windows))]
fn prepare_portable(_app: &AppInfo, _dir: &Path, _file: &Path) -> Result<Prepared, UpdateError> {
    Err(unsupported())
}

/// Starts the new process detached; the caller closes the app right after.
pub fn restart(p: &Prepared) -> Result<(), UpdateError> {
    match p {
        Prepared::Restart { exe } => spawn_detached(exe),
        Prepared::MsiAfterExit { .. } => Err(unsupported()),
    }
}

fn spawn_detached(exe: &Path) -> Result<(), UpdateError> {
    let mut cmd = std::process::Command::new(exe);
    if let Some(dir) = exe.parent() {
        cmd.current_dir(dir);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        cmd.creation_flags(DETACHED_PROCESS);
    }
    cmd.spawn()
        .map(drop)
        .map_err(|e| UpdateError::Install(format!("cannot start {}: {e}", exe.display())))
}

/// Removes leftovers of a previous update. Called at startup; never fails visibly.
pub fn cleanup(app: &AppInfo) {
    #[cfg(windows)]
    if let Ok(current) = std::env::current_exe() {
        // Only the shipped copy owns these folders.
        if let (true, Some(dir)) = (is_shipped_exe(app, &current), current.parent()) {
            portable::cleanup(app, dir, &std::env::temp_dir());
        }
    }
    #[cfg(not(windows))]
    let _ = app;
}

#[cfg(test)]
mod tests {
    use super::*;

    const APP: AppInfo = AppInfo {
        name: "furnica",
        display_name: "Furnica",
        repo: "nazdravan/furnica",
        version: "3.0.5",
        windows_exe: "Furnica.exe",
        msi_registry_key: "",
        deb_package: "furnica",
        mac_bundle: "Furnica.app",
    };

    #[test]
    fn shipped_exe_check() {
        assert!(is_shipped_exe(&APP, Path::new("/x/Furnica.exe")));
        assert!(is_shipped_exe(&APP, Path::new("/x/furnica.EXE")));
        assert!(!is_shipped_exe(
            &APP,
            Path::new("/x/target/debug/furnica_app.exe")
        ));
        assert!(!is_shipped_exe(&APP, Path::new("/x/")));
    }

    #[test]
    fn other_kinds_unsupported() {
        for kind in [InstallKind::Deb, InstallKind::LinkOnly] {
            assert_eq!(
                prepare(&APP, &kind, Path::new("x")),
                Err(UpdateError::Install("unsupported".into()))
            );
        }
        assert!(
            restart(&Prepared::MsiAfterExit {
                msi: "a".into(),
                exe: "b".into(),
                desktop_shortcut: false
            })
            .is_err()
        );
    }

    #[cfg(unix)]
    #[test]
    fn restart_spawns_in_exe_dir() {
        use std::os::unix::fs::PermissionsExt;
        let t = tempfile::tempdir().unwrap();
        let exe = t.path().join("app.sh");
        std::fs::write(&exe, "#!/bin/sh\npwd > out.txt\n").unwrap();
        std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
        restart(&Prepared::Restart { exe }).unwrap();
        let out = t.path().join("out.txt");
        for _ in 0..50 {
            if out.exists() && std::fs::metadata(&out).unwrap().len() > 0 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        let got = std::fs::read_to_string(out).unwrap();
        assert_eq!(
            std::fs::canonicalize(got.trim()).unwrap(),
            std::fs::canonicalize(t.path()).unwrap()
        );
    }
}
