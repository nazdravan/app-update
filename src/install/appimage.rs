//! Replacing a running AppImage in place.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::UpdateError;

/// Copies `new` over `target` (same folder, then `rename`, which is allowed while
/// `target` runs). If the folder is not writable, asks for rights with `pkexec`.
pub fn install(new: &Path, target: &Path) -> Result<PathBuf, UpdateError> {
    match replace(new, target) {
        Ok(()) => Ok(target.to_path_buf()),
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
            super::run_privileged(&mut pkexec_command(new, target))?;
            Ok(target.to_path_buf())
        }
        Err(e) => Err(UpdateError::Install(format!(
            "cannot replace {}: {e}",
            target.display()
        ))),
    }
}

fn pkexec_command(new: &Path, target: &Path) -> Command {
    let mut cmd = Command::new("pkexec");
    cmd.args(["install", "-m", "755"]).arg(new).arg(target);
    cmd
}

/// Name of the temporary copy: `.<name>.new` next to `target`.
fn temp_path(target: &Path) -> Result<PathBuf, std::io::Error> {
    let name = target
        .file_name()
        .ok_or_else(|| std::io::Error::other("target has no file name"))?;
    let mut tmp_name = std::ffi::OsString::from(".");
    tmp_name.push(name);
    tmp_name.push(".new");
    Ok(target.with_file_name(tmp_name))
}

#[cfg(unix)]
fn make_executable(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o755))
}

#[cfg(not(unix))]
fn make_executable(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

fn replace(new: &Path, target: &Path) -> std::io::Result<()> {
    let tmp = temp_path(target)?;
    let result = fs::copy(new, &tmp)
        .and_then(|_| make_executable(&tmp))
        .and_then(|_| fs::rename(&tmp, target));
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn appimage_replaces_in_place() {
        let t = tempfile::tempdir().unwrap();
        let target = t.path().join("Furnica.AppImage");
        fs::write(&target, "old").unwrap();
        fs::set_permissions(&target, fs::Permissions::from_mode(0o700)).unwrap();
        let new = t.path().join("download.AppImage");
        fs::write(&new, "new content").unwrap();
        fs::set_permissions(&new, fs::Permissions::from_mode(0o644)).unwrap();

        let got = install(&new, &target).unwrap();

        assert_eq!(got, target);
        assert_eq!(fs::read_to_string(&target).unwrap(), "new content");
        let mode = fs::metadata(&target).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o755);
        assert!(!t.path().join(".Furnica.AppImage.new").exists());
        assert!(new.exists(), "the download is not consumed");
    }

    #[test]
    fn appimage_missing_source_keeps_old() {
        let t = tempfile::tempdir().unwrap();
        let target = t.path().join("Furnica.AppImage");
        fs::write(&target, "old").unwrap();
        let err = install(&t.path().join("nope"), &target).unwrap_err();
        assert!(matches!(err, UpdateError::Install(_)));
        assert_eq!(fs::read_to_string(&target).unwrap(), "old");
        assert!(!t.path().join(".Furnica.AppImage.new").exists());
    }
}
