//! Installing a `.deb` through `pkexec`.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::UpdateError;

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

/// The command line that installs `deb`.
pub fn command(deb: &Path) -> Vec<String> {
    command_with(deb, on_path("apt-get"))
}

/// Like [`command`], with the `apt-get` lookup done by the caller.
pub fn command_with(deb: &Path, has_apt_get: bool) -> Vec<String> {
    let path = absolute(deb).to_string_lossy().into_owned();
    let mut v: Vec<String> = if has_apt_get {
        ["pkexec", "apt-get", "install", "-y", "--allow-downgrades"]
            .map(String::from)
            .to_vec()
    } else {
        ["pkexec", "dpkg", "-i"].map(String::from).to_vec()
    };
    v.push(path);
    v
}

/// Installs `deb`; returns the path of the packaged program to restart into.
pub fn install(deb: &Path, package: &str) -> Result<PathBuf, UpdateError> {
    let argv = command(deb);
    let mut cmd = Command::new(&argv[0]);
    cmd.args(&argv[1..]);
    super::run_privileged(&mut cmd)?;
    Ok(PathBuf::from("/usr/bin").join(package))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deb_command_shape() {
        let abs = if cfg!(windows) {
            r"C:\x\a.deb"
        } else {
            "/x/a.deb"
        };
        assert_eq!(
            command_with(Path::new(abs), true),
            [
                "pkexec",
                "apt-get",
                "install",
                "-y",
                "--allow-downgrades",
                abs
            ]
        );
        assert_eq!(
            command_with(Path::new(abs), false),
            ["pkexec", "dpkg", "-i", abs]
        );
        let rel = command_with(Path::new("a.deb"), true);
        let last = Path::new(rel.last().unwrap());
        assert!(last.is_absolute(), "{last:?}");
        assert!(last.ends_with("a.deb"));
    }
}
