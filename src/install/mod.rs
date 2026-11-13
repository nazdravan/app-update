//! Installing a downloaded update and restarting into it.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use crate::platform::InstallKind;
use crate::{AppInfo, UpdateError};

pub mod appimage;
pub mod deb;
pub mod macos;
pub mod msi;
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

/// Creates a NEW private folder for the files of one update:
/// `<temp>/<name>-update-<version>-<8 hex>`, mode 0700 on unix. It is created
/// exclusively (never reuses an existing path), because what is downloaded into it
/// is later installed with elevated rights.
pub fn download_dir(app: &AppInfo, version: &semver::Version) -> Result<PathBuf, UpdateError> {
    use std::sync::atomic::{AtomicU32, Ordering};
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let temp = std::env::temp_dir();
    let mut last = None;
    for _ in 0..8 {
        let nanos = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map_or(0, |d| d.subsec_nanos());
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let mix = nanos
            .wrapping_mul(2_654_435_761)
            .wrapping_add(std::process::id().wrapping_mul(40_503))
            .wrapping_add(n.wrapping_mul(2_246_822_519));
        let dir = temp.join(format!("{}-update-{version}-{mix:08x}", app.name));
        match create_private_dir(&dir) {
            Ok(()) => return Ok(dir),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => last = Some(e),
            Err(e) => {
                return Err(UpdateError::Install(format!(
                    "cannot create {}: {e}",
                    dir.display()
                )));
            }
        }
    }
    Err(UpdateError::Install(format!(
        "cannot create a download folder: {}",
        last.map(|e| e.to_string()).unwrap_or_default()
    )))
}

#[cfg(unix)]
fn create_private_dir(dir: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    std::fs::DirBuilder::new().mode(0o700).create(dir)
}

#[cfg(not(unix))]
fn create_private_dir(dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir(dir)
}

/// Runs a command that asks for rights (`pkexec`): exit 126 and 127 mean the user
/// refused or there is no agent to ask; any other failure is an install error.
pub(crate) fn run_privileged(cmd: &mut std::process::Command) -> Result<(), UpdateError> {
    let name = cmd.get_program().to_string_lossy().into_owned();
    let out = match cmd.output() {
        Ok(o) => o,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(UpdateError::Denied),
        Err(e) => return Err(UpdateError::Install(format!("cannot run {name}: {e}"))),
    };
    match out.status.code() {
        Some(0) => Ok(()),
        Some(126 | 127) => Err(UpdateError::Denied),
        _ => Err(UpdateError::Install(format!(
            "{name} failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ))),
    }
}

pub fn prepare(app: &AppInfo, kind: &InstallKind, file: &Path) -> Result<Prepared, UpdateError> {
    match kind {
        InstallKind::Portable { dir } => prepare_portable(app, dir, file),
        InstallKind::Msi { dir } => prepare_msi(app, dir, file),
        InstallKind::Deb => {
            deb::install(file, app.deb_package).map(|exe| Prepared::Restart { exe })
        }
        InstallKind::AppImage { path } => {
            appimage::install(file, path).map(|exe| Prepared::Restart { exe })
        }
        InstallKind::MacApp { bundle } => {
            macos::install(file, bundle, app.mac_bundle).map(|exe| Prepared::Restart { exe })
        }
        InstallKind::LinkOnly => Err(unsupported()),
    }
}

#[cfg(windows)]
fn prepare_msi(app: &AppInfo, dir: &Path, file: &Path) -> Result<Prepared, UpdateError> {
    Ok(Prepared::MsiAfterExit {
        msi: file.to_path_buf(),
        exe: dir.join(app.windows_exe),
        desktop_shortcut: msi::read_desktop_shortcut(app.msi_registry_key),
    })
}

#[cfg(not(windows))]
fn prepare_msi(_app: &AppInfo, _dir: &Path, _file: &Path) -> Result<Prepared, UpdateError> {
    Err(unsupported())
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
        Prepared::Restart { exe } if is_bundle(exe) => open_bundle(exe),
        Prepared::Restart { exe } => spawn_detached(exe),
        #[cfg(windows)]
        Prepared::MsiAfterExit {
            msi,
            exe,
            desktop_shortcut,
        } => msi::spawn_helper(msi, exe, *desktop_shortcut),
        #[cfg(not(windows))]
        Prepared::MsiAfterExit { .. } => Err(unsupported()),
    }
}

fn is_bundle(p: &Path) -> bool {
    p.extension().is_some_and(|e| e.eq_ignore_ascii_case("app"))
}

/// macOS: a new instance of the bundle (the old one is still closing).
fn open_bundle(bundle: &Path) -> Result<(), UpdateError> {
    let status = std::process::Command::new("open")
        .arg("-n")
        .arg(bundle)
        .status()
        .map_err(|e| UpdateError::Install(format!("cannot run open: {e}")))?;
    if status.success() {
        Ok(())
    } else {
        Err(UpdateError::Install(format!(
            "cannot start {}",
            bundle.display()
        )))
    }
}

fn spawn_detached(exe: &Path) -> Result<(), UpdateError> {
    let mut cmd = std::process::Command::new(exe);
    // The new copy must not think it still runs inside the old AppImage.
    for var in ["APPIMAGE", "APPDIR", "ARGV0", "OWD"] {
        cmd.env_remove(var);
    }
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
    cleanup_downloads(app, &std::env::temp_dir(), Duration::from_secs(24 * 3600));
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

/// Deletes `<name>-update-*` folders in `temp` last changed more than `max_age` ago.
/// (A downloaded MSI stays after the app exits, so it can only be removed later.)
pub fn cleanup_downloads(app: &AppInfo, temp: &Path, max_age: Duration) {
    let prefix = format!("{}-update-", app.name);
    let Ok(entries) = std::fs::read_dir(temp) else {
        return;
    };
    for e in entries.flatten() {
        if !e.file_name().to_string_lossy().starts_with(&prefix) {
            continue;
        }
        let Ok(meta) = e.metadata() else { continue };
        let old = meta
            .modified()
            .ok()
            .and_then(|m| SystemTime::now().duration_since(m).ok())
            .is_some_and(|age| age >= max_age);
        if meta.is_dir() && old {
            let _ = std::fs::remove_dir_all(e.path());
        }
    }
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
        assert_eq!(
            prepare(&APP, &InstallKind::LinkOnly, Path::new("x")),
            Err(UpdateError::Install("unsupported".into()))
        );
    }

    #[cfg(not(windows))]
    #[test]
    fn msi_only_on_windows() {
        let kind = InstallKind::Msi { dir: "d".into() };
        assert!(prepare(&APP, &kind, Path::new("x.msi")).is_err());
        assert!(
            restart(&Prepared::MsiAfterExit {
                msi: "a".into(),
                exe: "b".into(),
                desktop_shortcut: false
            })
            .is_err()
        );
    }

    #[test]
    fn bundle_paths_are_opened_not_executed() {
        assert!(is_bundle(Path::new("/Applications/Furnica.app")));
        assert!(is_bundle(Path::new("/x/Furnica.APP")));
        assert!(!is_bundle(Path::new("/usr/bin/furnica")));
        assert!(!is_bundle(Path::new("/x/Furnica.exe")));
    }

    #[cfg(unix)]
    #[test]
    fn appimage_prepare_restarts_into_target() {
        let t = tempfile::tempdir().unwrap();
        let target = t.path().join("F.AppImage");
        let new = t.path().join("dl");
        std::fs::write(&target, "old").unwrap();
        std::fs::write(&new, "new").unwrap();
        let kind = InstallKind::AppImage {
            path: target.clone(),
        };
        assert_eq!(
            prepare(&APP, &kind, &new),
            Ok(Prepared::Restart {
                exe: target.clone()
            })
        );
        assert_eq!(std::fs::read_to_string(target).unwrap(), "new");
    }

    #[test]
    fn download_dirs_are_new_and_private() {
        let v = semver::Version::new(3, 0, 6);
        let a = download_dir(&APP, &v).unwrap();
        let b = download_dir(&APP, &v).unwrap();
        assert_ne!(a, b);
        assert!(a.is_dir() && b.is_dir());
        assert_eq!(a.parent().unwrap(), std::env::temp_dir());
        assert!(
            a.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("furnica-update-3.0.6-")
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&a).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o700);
        }
        let _ = std::fs::remove_dir(a);
        let _ = std::fs::remove_dir(b);
    }

    #[cfg(unix)]
    #[test]
    fn restart_drops_appimage_env() {
        use std::os::unix::fs::PermissionsExt;
        // SAFETY: only this test sets these variables.
        unsafe {
            std::env::set_var("APPIMAGE", "/old.AppImage");
            std::env::set_var("OWD", "/old");
        }
        let t = tempfile::tempdir().unwrap();
        let exe = t.path().join("app.sh");
        std::fs::write(&exe, "#!/bin/sh\nenv > env.txt\necho done >> env.txt\n").unwrap();
        std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
        restart(&Prepared::Restart { exe }).unwrap();
        let out = t.path().join("env.txt");
        let mut got = String::new();
        for _ in 0..50 {
            got = std::fs::read_to_string(&out).unwrap_or_default();
            if got.contains("done") {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        assert!(got.contains("done"));
        assert!(!got.contains("APPIMAGE=") && !got.contains("OWD="), "{got}");
    }

    #[test]
    fn cleanup_downloads_respects_age() {
        let t = tempfile::tempdir().unwrap();
        let mine = t.path().join("furnica-update-3.0.6");
        let other = t.path().join("other-update-1.0.0");
        let unrelated = t.path().join("furnica-notes");
        for d in [&mine, &other, &unrelated] {
            std::fs::create_dir(d).unwrap();
            std::fs::write(d.join("f"), "x").unwrap();
        }
        cleanup_downloads(&APP, t.path(), Duration::from_secs(3600));
        assert!(mine.exists(), "fresh folder must stay");
        cleanup_downloads(&APP, t.path(), Duration::ZERO);
        assert!(!mine.exists());
        assert!(other.exists() && unrelated.exists());
    }

    #[cfg(unix)]
    #[test]
    fn privileged_exit_codes() {
        use std::process::Command;
        let sh =
            |code: &str| run_privileged(Command::new("sh").args(["-c", &format!("exit {code}")]));
        assert_eq!(sh("0"), Ok(()));
        assert_eq!(sh("126"), Err(UpdateError::Denied));
        assert_eq!(sh("127"), Err(UpdateError::Denied));
        assert!(matches!(sh("1"), Err(UpdateError::Install(_))));
        assert_eq!(
            run_privileged(&mut Command::new("/nonexistent/pkexec")),
            Err(UpdateError::Denied)
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
