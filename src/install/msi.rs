//! Updating an MSI install: a helper outlives the app, runs `msiexec`, restarts.

use std::path::Path;

/// `msiexec` arguments for upgrading with `msi`, keeping the desktop shortcut choice.
pub fn args(msi: &Path, desktop_shortcut: bool) -> Vec<String> {
    vec![
        "/i".into(),
        msi.to_string_lossy().into_owned(),
        "/passive".into(),
        "/norestart".into(),
        if desktop_shortcut {
            "DESKTOPSHORTCUT=1"
        } else {
            "DESKTOPSHORTCUT="
        }
        .into(),
    ]
}

/// A PowerShell single-quoted string literal.
fn ps_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

/// PowerShell script: wait for `pid` to exit, run the installer, then start `exe`
/// (the new one if it installed, the old one if the user cancelled the UAC prompt).
pub fn helper_script(pid: u32, msi: &Path, exe: &Path, desktop_shortcut: bool) -> String {
    let list = args(msi, desktop_shortcut)
        .into_iter()
        .enumerate()
        .map(|(i, a)| {
            // Start-Process joins the list with spaces: the path needs its own quotes.
            if i == 1 {
                ps_quote(&format!("\"{a}\""))
            } else {
                ps_quote(&a)
            }
        })
        .collect::<Vec<_>>()
        .join(",");
    let mut start = format!(
        "Start-Process -FilePath {}",
        ps_quote(&exe.to_string_lossy())
    );
    if let Some(dir) = exe.parent().filter(|d| !d.as_os_str().is_empty()) {
        start.push_str(&format!(
            " -WorkingDirectory {}",
            ps_quote(&dir.to_string_lossy())
        ));
    }
    format!(
        "Wait-Process -Id {pid} -ErrorAction SilentlyContinue\n\
         Start-Process -FilePath 'msiexec.exe' -ArgumentList @({list}) -Wait\n\
         {start}\n"
    )
}

/// Starts the helper detached and hidden. Windows only.
#[cfg(windows)]
pub fn spawn_helper(
    msi: &Path,
    exe: &Path,
    desktop_shortcut: bool,
) -> Result<(), crate::UpdateError> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    const DETACHED_PROCESS: u32 = 0x0000_0008;
    let script = helper_script(std::process::id(), msi, exe, desktop_shortcut);
    std::process::Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-WindowStyle",
            "Hidden",
            "-Command",
        ])
        .arg(script)
        .creation_flags(CREATE_NO_WINDOW | DETACHED_PROCESS)
        .spawn()
        .map(drop)
        .map_err(|e| crate::UpdateError::Install(format!("cannot start the installer: {e}")))
}

/// `true` if the MSI recorded that the desktop shortcut was installed.
#[cfg(windows)]
pub fn read_desktop_shortcut(registry_key: &str) -> bool {
    use winreg::RegKey;
    use winreg::enums::{HKEY_LOCAL_MACHINE, KEY_READ};
    RegKey::predef(HKEY_LOCAL_MACHINE)
        .open_subkey_with_flags(registry_key, KEY_READ)
        .and_then(|k| k.get_raw_value("DesktopShortcut"))
        .is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn msi_args_keep_shortcut_choice() {
        let m = Path::new("C:\\t\\a.msi");
        assert_eq!(
            args(m, true),
            [
                "/i",
                "C:\\t\\a.msi",
                "/passive",
                "/norestart",
                "DESKTOPSHORTCUT=1"
            ]
        );
        assert_eq!(
            args(m, false),
            [
                "/i",
                "C:\\t\\a.msi",
                "/passive",
                "/norestart",
                "DESKTOPSHORTCUT="
            ]
        );
    }

    #[test]
    fn msi_script_quotes_paths() {
        let s = helper_script(
            1234,
            Path::new("C:\\Temp\\it's\\furnica.msi"),
            Path::new("C:\\Program Files\\Fur'nica\\Furnica.exe"),
            true,
        );
        assert!(
            s.contains("Wait-Process -Id 1234 -ErrorAction SilentlyContinue"),
            "{s}"
        );
        assert!(
            s.contains("Start-Process -FilePath 'C:\\Program Files\\Fur''nica\\Furnica.exe'"),
            "{s}"
        );
        assert!(s.contains("'\"C:\\Temp\\it''s\\furnica.msi\"'"), "{s}");
        assert!(s.contains("'DESKTOPSHORTCUT=1'"), "{s}");
        assert!(s.contains("-Wait"), "{s}");
        // The program is started after the installer, whatever its result.
        assert!(s.find("msiexec").unwrap() < s.rfind("Furnica.exe").unwrap());
        let s2 = helper_script(1, Path::new("a.msi"), Path::new("b.exe"), false);
        assert!(s2.contains("'DESKTOPSHORTCUT='"), "{s2}");
    }
}
