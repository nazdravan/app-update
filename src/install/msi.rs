//! Updating an MSI install: a helper outlives the app, runs `msiexec`, restarts.

use std::path::Path;

/// `msiexec` arguments for upgrading with `msi`, keeping the desktop shortcut choice.
pub fn args(msi: &Path, desktop_shortcut: bool) -> Vec<String> {
    vec![
        "/i".into(),
        msi.to_string_lossy().into_owned(),
        "/passive".into(),
        "/norestart".into(),
        // The installer condition is "DESKTOPSHORTCUT is non-empty", so "off" must
        // reach msiexec as an explicitly empty value: PROP="" (a bare `PROP=` is
        // not a reliable way to say it).
        if desktop_shortcut {
            "DESKTOPSHORTCUT=1"
        } else {
            "DESKTOPSHORTCUT=\"\""
        }
        .into(),
    ]
}

/// A PowerShell single-quoted string literal. PowerShell treats the typographic
/// quotes U+2018..U+201B as single quotes too, so each of them is doubled.
fn ps_quote(s: &str) -> String {
    let mut out = String::from("'");
    for c in s.chars() {
        out.push(c);
        if matches!(c, '\'' | '\u{2018}' | '\u{2019}' | '\u{201A}' | '\u{201B}') {
            out.push(c);
        }
    }
    out.push('\'');
    out
}

/// PowerShell script: wait for `pid` to exit, run the installer, then start `exe`
/// (the new one if it installed, the old one if the user cancelled the UAC prompt).
pub fn helper_script(pid: u32, msi: &Path, exe: &Path, desktop_shortcut: bool) -> String {
    helper_script_with("msiexec.exe", pid, msi, exe, desktop_shortcut)
}

/// Same as [`helper_script`], with the installer program given (tests use a fake).
fn helper_script_with(
    msiexec: &str,
    pid: u32,
    msi: &Path,
    exe: &Path,
    desktop_shortcut: bool,
) -> String {
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
         Start-Process -FilePath {} -ArgumentList @({list}) -Wait\n\
         {start}\n",
        ps_quote(msiexec)
    )
}

/// Standard base64 (with padding) of `bytes`.
fn base64(bytes: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |n, (i, b)| n | (u32::from(*b) << (16 - 8 * i)));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(T[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// The `powershell.exe` arguments that run `script`. The script travels as
/// `-EncodedCommand` (UTF-16LE, base64), so no command-line quoting can break it.
pub fn helper_args(script: &str) -> Vec<String> {
    let utf16: Vec<u8> = script.encode_utf16().flat_map(u16::to_le_bytes).collect();
    vec![
        "-NoProfile".into(),
        "-NonInteractive".into(),
        "-WindowStyle".into(),
        "Hidden".into(),
        "-EncodedCommand".into(),
        base64(&utf16),
    ]
}

/// The hidden `powershell.exe` process that runs `script`. Windows only.
#[cfg(windows)]
fn helper_command(script: &str) -> std::process::Command {
    use std::os::windows::process::CommandExt;
    // A hidden console of its own. Not DETACHED_PROCESS: powershell.exe without a
    // console exits before running the script.
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let mut cmd = std::process::Command::new("powershell.exe");
    cmd.args(helper_args(script))
        .creation_flags(CREATE_NO_WINDOW);
    cmd
}

/// Starts the helper hidden; it outlives the app. Windows only.
#[cfg(windows)]
pub fn spawn_helper(
    msi: &Path,
    exe: &Path,
    desktop_shortcut: bool,
) -> Result<(), crate::UpdateError> {
    let script = helper_script(std::process::id(), msi, exe, desktop_shortcut);
    helper_command(&script)
        .spawn()
        .map(drop)
        .map_err(|e| crate::UpdateError::Install(format!("cannot start the installer: {e}")))
}

/// `true` if the MSI recorded that the desktop shortcut was installed.
#[cfg(windows)]
pub fn read_desktop_shortcut(registry_key: &str) -> bool {
    // The MSI writes this value only when the DESKTOPSHORTCUT property was set,
    // so "the value exists" means the shortcut was chosen.
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
                "DESKTOPSHORTCUT=\"\""
            ]
        );
    }

    #[test]
    fn ps_quote_doubles_typographic_quotes() {
        assert_eq!(ps_quote("O\u{2019}Brien"), "'O\u{2019}\u{2019}Brien'");
        assert_eq!(
            ps_quote("a\u{2018}\u{201A}\u{201B}b'"),
            "'a\u{2018}\u{2018}\u{201A}\u{201A}\u{201B}\u{201B}b'''",
        );
        let s = helper_script(
            1,
            Path::new("C:\\U\\O\u{2019}B\\a.msi"),
            Path::new("C:\\U\\O\u{2019}B\\f.exe"),
            false,
        );
        assert!(s.contains("'C:\\U\\O\u{2019}\u{2019}B\\f.exe'"), "{s}");
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
        assert!(s2.contains("'DESKTOPSHORTCUT=\"\"'"), "{s2}");
    }

    #[test]
    fn base64_matches_rfc4648() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn helper_args_encode_script_as_utf16le() {
        let a = helper_args("Hi");
        assert_eq!(a[a.len() - 2], "-EncodedCommand");
        // "Hi" in UTF-16LE is 48 00 69 00.
        assert_eq!(a[a.len() - 1], "SABpAA==");
        assert!(!a.iter().any(|x| x == "-Command"));
    }

    /// Runs the real helper command line with a dead PID, a fake `msiexec` and a
    /// restart target, and checks what they received.
    #[cfg(windows)]
    fn run_helper(desktop_shortcut: bool) -> String {
        use std::time::{Duration, Instant};
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("with space");
        std::fs::create_dir_all(&dir).unwrap();
        let args_out = dir.join("args.txt");
        let marker = dir.join("restarted.txt");
        let fake_msiexec = dir.join("fake msiexec.cmd");
        std::fs::write(
            &fake_msiexec,
            format!("@echo off\r\necho %* > \"{}\"\r\n", args_out.display()),
        )
        .unwrap();
        let target = dir.join("restart.cmd");
        std::fs::write(
            &target,
            format!("@echo off\r\necho x > \"{}\"\r\n", marker.display()),
        )
        .unwrap();
        let msi = dir.join("new it's.msi");

        // A process id that is certainly dead: a child that has been waited for.
        let mut child = std::process::Command::new("cmd")
            .args(["/c", "exit", "0"])
            .spawn()
            .unwrap();
        let dead_pid = child.id();
        child.wait().unwrap();

        let script = helper_script_with(
            &fake_msiexec.to_string_lossy(),
            dead_pid,
            &msi,
            &target,
            desktop_shortcut,
        );
        // Started exactly as spawn_helper starts it (same creation flags).
        let status = helper_command(&script).spawn().unwrap().wait().unwrap();
        assert!(status.success(), "{status:?}");
        let deadline = Instant::now() + Duration::from_secs(30);
        while !marker.exists() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(200));
        }
        assert!(marker.exists(), "the program was not restarted");
        let recorded = std::fs::read_to_string(&args_out).expect("msiexec was not run");
        let msi_arg = format!("\"{}\"", msi.display());
        for want in ["/i", msi_arg.as_str(), "/passive", "/norestart"] {
            assert!(recorded.contains(want), "{want} missing in {recorded}");
        }
        recorded.trim().to_string()
    }

    #[cfg(windows)]
    #[test]
    fn helper_runs_installer_then_restarts_with_shortcut() {
        let r = run_helper(true);
        assert!(r.ends_with("DESKTOPSHORTCUT=1"), "{r}");
        assert!(!r.contains("DESKTOPSHORTCUT=\"\""), "{r}");
    }

    #[cfg(windows)]
    #[test]
    fn helper_runs_installer_then_restarts_without_shortcut() {
        let r = run_helper(false);
        assert!(r.ends_with("DESKTOPSHORTCUT=\"\""), "{r}");
        assert!(!r.contains("DESKTOPSHORTCUT=1"), "{r}");
    }
}
