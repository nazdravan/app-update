//! Installing an `.rpm` through `pkexec`.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::UpdateError;

/// The package tool found on this system, best first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    Dnf,
    Zypper,
    Rpm,
}

/// `true` if an executable called `name` is in one of the `PATH` folders.
fn on_path(name: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|paths| std::env::split_paths(&paths).any(|d| d.join(name).is_file()))
}

fn absolute(p: &Path) -> PathBuf {
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        std::env::current_dir()
            .map(|d| d.join(p))
            .unwrap_or_else(|_| p.to_path_buf())
    }
}

fn tool() -> Tool {
    if on_path("dnf") {
        Tool::Dnf
    } else if on_path("zypper") {
        Tool::Zypper
    } else {
        Tool::Rpm
    }
}

/// The command line that installs `rpm`.
pub fn command(rpm: &Path) -> Vec<String> {
    command_with(rpm, tool())
}

/// Like [`command`], with the tool lookup done by the caller.
///
/// The file was checked against `SHA256SUMS.txt`, so a package without a GPG
/// signature is accepted (dnf does not check local files by default).
pub fn command_with(rpm: &Path, tool: Tool) -> Vec<String> {
    let path = absolute(rpm).to_string_lossy().into_owned();
    let base: &[&str] = match tool {
        Tool::Dnf => &["pkexec", "dnf", "install", "-y"],
        Tool::Zypper => &[
            "pkexec",
            "zypper",
            "--non-interactive",
            "install",
            "--allow-unsigned-rpm",
        ],
        Tool::Rpm => &["pkexec", "rpm", "-U", "--replacepkgs"],
    };
    let mut v: Vec<String> = base.iter().map(|s| s.to_string()).collect();
    v.push(path);
    v
}

/// Installs `rpm`; returns the path of the packaged program to restart into.
pub fn install(rpm: &Path, package: &str) -> Result<PathBuf, UpdateError> {
    let argv = command(rpm);
    let mut cmd = Command::new(&argv[0]);
    cmd.args(&argv[1..]);
    super::run_privileged(&mut cmd)?;
    Ok(PathBuf::from("/usr/bin").join(package))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rpm_command_shape() {
        let abs = if cfg!(windows) {
            r"C:\x\a.rpm"
        } else {
            "/x/a.rpm"
        };
        assert_eq!(
            command_with(Path::new(abs), Tool::Dnf),
            ["pkexec", "dnf", "install", "-y", abs]
        );
        assert_eq!(
            command_with(Path::new(abs), Tool::Zypper),
            [
                "pkexec",
                "zypper",
                "--non-interactive",
                "install",
                "--allow-unsigned-rpm",
                abs
            ]
        );
        assert_eq!(
            command_with(Path::new(abs), Tool::Rpm),
            ["pkexec", "rpm", "-U", "--replacepkgs", abs]
        );
        let rel = command_with(Path::new("a.rpm"), Tool::Dnf);
        let last = Path::new(rel.last().unwrap());
        assert!(last.is_absolute(), "{last:?}");
        assert!(last.ends_with("a.rpm"));
    }
}
