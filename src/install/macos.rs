//! Replacing a `.app` bundle with the one inside a downloaded `.dmg`.

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::UpdateError;

fn fail(what: impl std::fmt::Display) -> UpdateError {
    UpdateError::Install(what.to_string())
}

/// `<bundle><suffix>`, e.g. `/Applications/Furnica.app.new`.
fn sibling(bundle: &Path, suffix: &str) -> PathBuf {
    let mut s: OsString = bundle.as_os_str().to_owned();
    s.push(suffix);
    PathBuf::from(s)
}

/// Installs the bundle found in `dmg` over `bundle` and returns `bundle`.
/// `bundle_name` is the name of the `.app` folder inside the image.
pub fn install(dmg: &Path, bundle: &Path, bundle_name: &str) -> Result<PathBuf, UpdateError> {
    let parent = bundle
        .parent()
        .ok_or_else(|| fail("the bundle has no parent folder"))?;
    if dir_writable(parent) {
        install_direct(dmg, bundle, bundle_name)?;
    } else {
        install_admin(dmg, bundle, bundle_name)?;
    }
    Ok(bundle.to_path_buf())
}

fn dir_writable(dir: &Path) -> bool {
    let probe = dir.join(format!(".app-update-probe-{}", std::process::id()));
    let ok = fs::File::create(&probe).is_ok();
    let _ = fs::remove_file(&probe);
    ok
}

fn run(cmd: &mut Command) -> Result<(), UpdateError> {
    let name = cmd.get_program().to_string_lossy().into_owned();
    let out = cmd
        .output()
        .map_err(|e| fail(format!("cannot run {name}: {e}")))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(fail(format!(
            "{name} failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        )))
    }
}

/// A mounted image; detached on drop, so every error path lets go of it.
struct Mounted {
    mount: PathBuf,
}

impl Mounted {
    fn attach(dmg: &Path) -> Result<Self, UpdateError> {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.subsec_nanos());
        let mount =
            std::env::temp_dir().join(format!("app-update-mnt-{}-{nanos}", std::process::id()));
        fs::create_dir(&mount).map_err(|e| fail(format!("cannot create mount point: {e}")))?;
        let attached = run(Command::new("hdiutil")
            .args([
                "attach",
                "-nobrowse",
                "-readonly",
                "-noautoopen",
                "-mountpoint",
            ])
            .arg(&mount)
            .arg(dmg));
        let m = Mounted { mount };
        attached?; // on error `m` drops: detach (no-op) and remove the folder
        Ok(m)
    }
}

impl Drop for Mounted {
    fn drop(&mut self) {
        let detach = |force: bool| {
            let mut c = Command::new("hdiutil");
            c.arg("detach");
            if force {
                c.arg("-force");
            }
            c.arg(&self.mount);
            c.output().is_ok_and(|o| o.status.success())
        };
        if !detach(false) {
            detach(true);
        }
        let _ = fs::remove_dir(&self.mount);
    }
}

fn install_direct(dmg: &Path, bundle: &Path, bundle_name: &str) -> Result<(), UpdateError> {
    let new = sibling(bundle, ".new");
    let _ = fs::remove_dir_all(&new);
    {
        let mounted = Mounted::attach(dmg)?;
        let copied = run(Command::new("ditto")
            .arg(mounted.mount.join(bundle_name))
            .arg(&new));
        if copied.is_err() {
            let _ = fs::remove_dir_all(&new);
        }
        copied?;
    } // detached here, before the swap
    swap(bundle, &new)
}

/// `new` becomes `bundle`; the previous `bundle` is deleted. Puts everything back on error.
fn swap(bundle: &Path, new: &Path) -> Result<(), UpdateError> {
    let old = sibling(bundle, ".old");
    let had_old_copy = bundle.exists();
    if had_old_copy {
        let _ = fs::remove_dir_all(&old);
        fs::rename(bundle, &old).map_err(|e| {
            let _ = fs::remove_dir_all(new);
            fail(format!("cannot move {} aside: {e}", bundle.display()))
        })?;
    }
    if let Err(e) = fs::rename(new, bundle) {
        let restored = !had_old_copy || fs::rename(&old, bundle).is_ok();
        let _ = fs::remove_dir_all(new);
        return Err(fail(format!(
            "cannot put the new copy in place: {e}{}",
            if restored {
                ""
            } else {
                " (the old copy is in the .old folder)"
            }
        )));
    }
    let _ = fs::remove_dir_all(&old);
    Ok(())
}

/// A POSIX shell single-quoted word.
fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// The whole install as one shell script, to run as root. `owner` (`uid:gid`)
/// gets the new files so later updates need no password.
pub fn admin_script(dmg: &Path, bundle: &Path, bundle_name: &str, owner: Option<&str>) -> String {
    let chown = owner
        .filter(|o| o.chars().all(|c| c.is_ascii_digit() || c == ':'))
        .map(|o| format!("/usr/sbin/chown -R {o} \"$NEW\"\n"))
        .unwrap_or_default();
    format!(
        "set -e\n\
         DMG={dmg}\n\
         BUNDLE={bundle}\n\
         NEW=\"$BUNDLE.new\"\n\
         OLD=\"$BUNDLE.old\"\n\
         MNT=$(/usr/bin/mktemp -d)\n\
         trap '/usr/bin/hdiutil detach \"$MNT\" >/dev/null 2>&1 || /usr/bin/hdiutil detach -force \"$MNT\" >/dev/null 2>&1; rmdir \"$MNT\" 2>/dev/null || true' EXIT\n\
         /usr/bin/hdiutil attach -nobrowse -readonly -noautoopen -mountpoint \"$MNT\" \"$DMG\" >/dev/null\n\
         rm -rf \"$NEW\"\n\
         /usr/bin/ditto \"$MNT\"/{name} \"$NEW\"\n\
         {chown}\
         /usr/bin/hdiutil detach \"$MNT\" >/dev/null\n\
         rm -rf \"$OLD\"\n\
         if [ -e \"$BUNDLE\" ]; then mv \"$BUNDLE\" \"$OLD\"; fi\n\
         if ! mv \"$NEW\" \"$BUNDLE\"; then\n\
         \x20 if [ -e \"$OLD\" ]; then mv \"$OLD\" \"$BUNDLE\"; fi\n\
         \x20 rm -rf \"$NEW\"\n\
         \x20 exit 1\n\
         fi\n\
         rm -rf \"$OLD\"\n",
        dmg = sh_quote(&dmg.to_string_lossy()),
        bundle = sh_quote(&bundle.to_string_lossy()),
        name = sh_quote(bundle_name),
    )
}

/// The AppleScript that runs `script` with administrator rights.
pub fn osascript_source(script: &str) -> String {
    let escaped = script
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n");
    format!("do shell script \"{escaped}\" with administrator privileges")
}

fn id_arg(flag: &str) -> Option<String> {
    let out = Command::new("id").arg(flag).output().ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn install_admin(dmg: &Path, bundle: &Path, bundle_name: &str) -> Result<(), UpdateError> {
    let owner = id_arg("-u")
        .zip(id_arg("-g"))
        .map(|(u, g)| format!("{u}:{g}"));
    let script = admin_script(dmg, bundle, bundle_name, owner.as_deref());
    let out = Command::new("osascript")
        .arg("-e")
        .arg(osascript_source(&script))
        .output()
        .map_err(|e| fail(format!("cannot run osascript: {e}")))?;
    if out.status.success() {
        return Ok(());
    }
    let err = String::from_utf8_lossy(&out.stderr);
    if err.contains("(-128)") {
        Err(UpdateError::Denied)
    } else {
        Err(fail(format!(
            "administrator install failed: {}",
            err.trim()
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn macos_admin_script_quotes() {
        let bundle = Path::new("/Applications/Fur'nica.app");
        let script = admin_script(
            Path::new("/tmp/x y/f.dmg"),
            bundle,
            "Fur'nica.app",
            Some("501:20"),
        );
        assert!(
            script.contains("BUNDLE='/Applications/Fur'\\''nica.app'\n"),
            "{script}"
        );
        assert!(script.contains("DMG='/tmp/x y/f.dmg'\n"), "{script}");
        assert!(script.contains("\"$MNT\"/'Fur'\\''nica.app'"), "{script}");
        assert!(script.contains("chown -R 501:20 \"$NEW\""), "{script}");
        assert!(!admin_script(Path::new("d"), bundle, "n", Some("1; rm -rf /")).contains("chown"));

        let osa = osascript_source(&script);
        assert!(osa.starts_with("do shell script \""));
        assert!(osa.ends_with("\" with administrator privileges"));
        // The shell's backslash is doubled for AppleScript, double quotes escaped, no raw newline.
        assert!(osa.contains("'/Applications/Fur'\\\\''nica.app'"), "{osa}");
        assert!(osa.contains("\\\"$NEW\\\""), "{osa}");
        assert!(!osa.contains('\n'));
    }

    #[test]
    fn swap_replaces_and_cleans() {
        let t = tempfile::tempdir().unwrap();
        let bundle = t.path().join("A.app");
        let new = t.path().join("A.app.new");
        fs::create_dir_all(&bundle).unwrap();
        fs::create_dir_all(&new).unwrap();
        fs::write(bundle.join("v"), "old").unwrap();
        fs::write(new.join("v"), "new").unwrap();
        swap(&bundle, &new).unwrap();
        assert_eq!(fs::read_to_string(bundle.join("v")).unwrap(), "new");
        assert!(!new.exists());
        assert!(!t.path().join("A.app.old").exists());
    }

    #[test]
    fn swap_failure_puts_old_back() {
        let t = tempfile::tempdir().unwrap();
        let bundle = t.path().join("A.app");
        fs::create_dir_all(&bundle).unwrap();
        fs::write(bundle.join("v"), "old").unwrap();
        let missing = t.path().join("A.app.new"); // never created: second rename fails
        assert!(swap(&bundle, &missing).is_err());
        assert_eq!(fs::read_to_string(bundle.join("v")).unwrap(), "old");
        assert!(!t.path().join("A.app.old").exists());
    }

    /// Manual: `APP_UPDATE_TEST_DMG=<dmg> [APP_UPDATE_TEST_EXPECT=3.0.5] cargo test macos_real_dmg -- --ignored`
    #[cfg(target_os = "macos")]
    #[test]
    #[ignore]
    fn macos_real_dmg() {
        let dmg = std::env::var("APP_UPDATE_TEST_DMG").expect("set APP_UPDATE_TEST_DMG");
        let expect = std::env::var("APP_UPDATE_TEST_EXPECT").unwrap_or_else(|_| "3.0.5".into());
        let t = tempfile::tempdir().unwrap();
        let bundle = t.path().join("Furnica.app");
        fs::create_dir_all(bundle.join("Contents")).unwrap();
        fs::write(
            bundle.join("Contents/Info.plist"),
            r#"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0"><dict><key>CFBundleShortVersionString</key><string>0.0.1</string></dict></plist>"#,
        )
        .unwrap();

        let got = install(Path::new(&dmg), &bundle, "Furnica.app").unwrap();
        assert_eq!(got, bundle);

        let v = Command::new("defaults")
            .arg("read")
            .arg(bundle.join("Contents/Info"))
            .arg("CFBundleShortVersionString")
            .output()
            .unwrap();
        let v = String::from_utf8_lossy(&v.stdout).trim().to_string();
        println!("installed version: {v}");
        assert_eq!(v, expect);
        assert!(!t.path().join("Furnica.app.new").exists());
        assert!(!t.path().join("Furnica.app.old").exists());
        let info = Command::new("hdiutil").arg("info").output().unwrap();
        assert!(
            !String::from_utf8_lossy(&info.stdout).contains("app-update-mnt"),
            "an image is still mounted"
        );
    }
}
