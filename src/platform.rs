//! Detecting how the running copy was installed and picking its release asset.

use std::path::{Path, PathBuf};

use crate::{AppInfo, Asset, Release, UpdateError};

/// Which build of the application is running (decided by `cfg`, not by the OS).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edition {
    WinX64,
    WinX86,
    Win7X86,
    LinuxX64,
    MacUniversal,
    Other,
}

impl Edition {
    /// The edition this binary was built as.
    pub fn current() -> Edition {
        if cfg!(target_vendor = "win7") {
            Edition::Win7X86
        } else if cfg!(all(windows, target_arch = "x86_64")) {
            Edition::WinX64
        } else if cfg!(all(windows, target_arch = "x86")) {
            Edition::WinX86
        } else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
            Edition::LinuxX64
        } else if cfg!(target_os = "macos") {
            Edition::MacUniversal
        } else {
            Edition::Other
        }
    }

    fn is_windows(self) -> bool {
        matches!(self, Edition::WinX64 | Edition::WinX86 | Edition::Win7X86)
    }
}

/// How the running copy got onto the computer, and so how it can be updated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstallKind {
    Msi { dir: PathBuf },
    Portable { dir: PathBuf },
    Deb,
    AppImage { path: PathBuf },
    MacApp { bundle: PathBuf },
    LinkOnly,
}

/// Facts about the running copy, gathered by `detect` and judged by `classify`.
#[derive(Debug, Clone)]
pub struct Probe {
    pub edition: Edition,
    pub exe: PathBuf,
    pub msi_folder: Option<PathBuf>,
    /// `$APPIMAGE`, as inherited from the environment (may belong to a parent process).
    pub appimage: Option<PathBuf>,
    /// `$APPDIR`, the AppImage mount point (inherited the same way).
    pub appdir: Option<PathBuf>,
    pub deb_installed: bool,
}

/// A path in a form fit for comparison: canonical when it exists, without a
/// trailing separator, lowercase.
fn comparable(p: &Path) -> String {
    let canonical = p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
    canonical
        .to_string_lossy()
        .trim_end_matches(['/', '\\'])
        .to_lowercase()
}

/// Decides how the copy described by `p` was installed.
pub fn classify(app: &AppInfo, p: &Probe) -> InstallKind {
    if p.edition.is_windows() {
        let Some(dir) = p.exe.parent() else {
            return InstallKind::LinkOnly;
        };
        return match &p.msi_folder {
            Some(msi) if comparable(msi) == comparable(dir) => InstallKind::Msi {
                dir: dir.to_path_buf(),
            },
            _ => InstallKind::Portable {
                dir: dir.to_path_buf(),
            },
        };
    }
    match p.edition {
        Edition::LinuxX64 => {
            // The AppImage runtime exports APPIMAGE/APPDIR to its children, so a
            // different program started from an AppImage inherits them. Trust them
            // only when this very executable lives inside APPDIR.
            let own_appimage = match (&p.appimage, &p.appdir) {
                (Some(path), Some(dir)) if p.exe.starts_with(dir) => Some(path),
                _ => None,
            };
            if let Some(path) = own_appimage {
                InstallKind::AppImage { path: path.clone() }
            } else if p.deb_installed && p.exe == Path::new("/usr/bin").join(app.deb_package) {
                InstallKind::Deb
            } else {
                InstallKind::LinkOnly
            }
        }
        Edition::MacUniversal => mac_bundle(&p.exe).map_or(InstallKind::LinkOnly, |bundle| {
            InstallKind::MacApp { bundle }
        }),
        _ => InstallKind::LinkOnly,
    }
}

/// `<X>.app` when `exe` is `<X>.app/Contents/MacOS/<exe>` and the bundle is
/// not on a disk image or in a translocated (quarantined) location.
fn mac_bundle(exe: &Path) -> Option<PathBuf> {
    let macos = exe.parent()?;
    let contents = macos.parent()?;
    let bundle = contents.parent()?;
    if macos.file_name()? != "MacOS" || contents.file_name()? != "Contents" {
        return None;
    }
    if bundle.extension().is_none_or(|e| e != "app") {
        return None;
    }
    let text = bundle.to_string_lossy();
    if text.contains("/AppTranslocation/") || text.starts_with("/Volumes/") {
        return None;
    }
    Some(bundle.to_path_buf())
}

/// Inspects the running copy and classifies it.
pub fn detect(app: &AppInfo) -> InstallKind {
    let Ok(exe) = std::env::current_exe() else {
        return InstallKind::LinkOnly;
    };
    let edition = Edition::current();
    let probe = Probe {
        edition,
        exe,
        msi_folder: msi_folder(app),
        appimage: std::env::var_os("APPIMAGE")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from),
        appdir: std::env::var_os("APPDIR")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from),
        deb_installed: edition == Edition::LinuxX64 && deb_installed(app),
    };
    classify(app, &probe)
}

#[cfg(windows)]
fn msi_folder(app: &AppInfo) -> Option<PathBuf> {
    use winreg::RegKey;
    use winreg::enums::HKEY_LOCAL_MACHINE;

    let key = RegKey::predef(HKEY_LOCAL_MACHINE)
        .open_subkey(app.msi_registry_key)
        .ok()?;
    let folder: String = key.get_value("InstallFolder").ok()?;
    Some(PathBuf::from(folder))
}

#[cfg(not(windows))]
fn msi_folder(_app: &AppInfo) -> Option<PathBuf> {
    None
}

fn deb_installed(app: &AppInfo) -> bool {
    std::process::Command::new("dpkg-query")
        .args(["-W", "-f=${Status}", app.deb_package])
        .output()
        .is_ok_and(|o| o.status.success() && o.stdout == b"install ok installed")
}

/// The release asset that updates a copy of this kind and edition.
///
/// `NoAsset` when the release lacks it, or when the copy cannot be updated in
/// place (`LinkOnly`, or an MSI on Windows 7, which has none).
pub fn asset_for<'a>(
    app: &AppInfo,
    kind: &InstallKind,
    edition: Edition,
    r: &'a Release,
) -> Result<&'a Asset, UpdateError> {
    let suffix = match (kind, edition) {
        (InstallKind::Msi { .. }, Edition::WinX64) => "windows-x64.msi",
        (InstallKind::Msi { .. }, Edition::WinX86) => "windows-x86.msi",
        (InstallKind::Portable { .. }, Edition::WinX64) => "windows-x64-portable.zip",
        (InstallKind::Portable { .. }, Edition::WinX86) => "windows-x86-portable.zip",
        (InstallKind::Portable { .. }, Edition::Win7X86) => "windows7-x86-portable.zip",
        (InstallKind::Deb, Edition::LinuxX64) => "linux-x86_64.deb",
        (InstallKind::AppImage { .. }, Edition::LinuxX64) => "linux-x86_64.AppImage",
        (InstallKind::MacApp { .. }, Edition::MacUniversal) => "macos-universal.dmg",
        _ => return Err(UpdateError::NoAsset),
    };
    let wanted = format!("{}-{}-{suffix}", app.name, r.version);
    r.assets
        .iter()
        .find(|a| a.name == wanted)
        .ok_or(UpdateError::NoAsset)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::release::parse_latest;

    const LATEST: &str = include_str!("../tests/fixtures/latest.json");

    const APP: AppInfo = AppInfo {
        name: "furnica",
        display_name: "Furnica",
        repo: "nazdravan/furnica",
        version: "3.0.5",
        windows_exe: "Furnica.exe",
        msi_registry_key: r"Software\nazdravan\Furnica",
        deb_package: "furnica",
        mac_bundle: "Furnica.app",
    };

    fn probe(edition: Edition, exe: impl Into<PathBuf>) -> Probe {
        Probe {
            edition,
            exe: exe.into(),
            msi_folder: None,
            appimage: None,
            appdir: None,
            deb_installed: false,
        }
    }

    fn temp_dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("app-update-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        d.canonicalize().unwrap()
    }

    #[test]
    fn classify_windows_msi_vs_portable() {
        let dir = temp_dir("msi");
        let other = temp_dir("msi-other");
        let exe = dir.join("Furnica.exe");
        std::fs::write(&exe, b"x").unwrap();
        let shouted = format!("{}\\", dir.to_string_lossy().to_uppercase());

        for edition in [Edition::WinX64, Edition::WinX86, Edition::Win7X86] {
            let mut p = probe(edition, &exe);
            p.msi_folder = Some(PathBuf::from(&shouted));
            assert_eq!(classify(&APP, &p), InstallKind::Msi { dir: dir.clone() });

            p.msi_folder = Some(other.clone());
            assert_eq!(
                classify(&APP, &p),
                InstallKind::Portable { dir: dir.clone() }
            );

            p.msi_folder = None;
            assert_eq!(
                classify(&APP, &p),
                InstallKind::Portable { dir: dir.clone() }
            );
        }
    }

    #[test]
    fn classify_linux() {
        let mut p = probe(Edition::LinuxX64, "/tmp/.mount_Furnic/usr/bin/furnica");
        p.appimage = Some("/home/x/Furnica.AppImage".into());
        p.appdir = Some("/tmp/.mount_Furnic".into());
        assert_eq!(
            classify(&APP, &p),
            InstallKind::AppImage {
                path: "/home/x/Furnica.AppImage".into()
            }
        );

        // APPIMAGE/APPDIR inherited from another AppImage that launched us:
        // our exe is outside APPDIR, so it falls through to Deb / LinkOnly.
        let mut p = probe(Edition::LinuxX64, "/usr/bin/furnica");
        p.appimage = Some("/home/x/Other.AppImage".into());
        p.appdir = Some("/tmp/.mount_Other".into());
        p.deb_installed = true;
        assert_eq!(classify(&APP, &p), InstallKind::Deb);
        let mut p = probe(Edition::LinuxX64, "/opt/furnica/furnica");
        p.appimage = Some("/home/x/Other.AppImage".into());
        p.appdir = Some("/tmp/.mount_Other".into());
        assert_eq!(classify(&APP, &p), InstallKind::LinkOnly);
        // APPIMAGE without APPDIR is not trusted either.
        let mut p = probe(Edition::LinuxX64, "/opt/furnica/furnica");
        p.appimage = Some("/home/x/Other.AppImage".into());
        assert_eq!(classify(&APP, &p), InstallKind::LinkOnly);

        let mut p = probe(Edition::LinuxX64, "/usr/bin/furnica");
        p.deb_installed = true;
        assert_eq!(classify(&APP, &p), InstallKind::Deb);

        // Same path, but dpkg does not know the package.
        p.deb_installed = false;
        assert_eq!(classify(&APP, &p), InstallKind::LinkOnly);

        let mut p = probe(Edition::LinuxX64, "/home/x/furnica");
        p.deb_installed = true;
        assert_eq!(classify(&APP, &p), InstallKind::LinkOnly);
    }

    #[test]
    fn classify_mac() {
        let p = probe(
            Edition::MacUniversal,
            "/Applications/Furnica.app/Contents/MacOS/furnica",
        );
        assert_eq!(
            classify(&APP, &p),
            InstallKind::MacApp {
                bundle: "/Applications/Furnica.app".into()
            }
        );

        let p = probe(
            Edition::MacUniversal,
            "/private/var/folders/x/AppTranslocation/y/d/Furnica.app/Contents/MacOS/furnica",
        );
        assert_eq!(classify(&APP, &p), InstallKind::LinkOnly);

        let p = probe(
            Edition::MacUniversal,
            "/Volumes/Furnica 3.0.5/Furnica.app/Contents/MacOS/furnica",
        );
        assert_eq!(classify(&APP, &p), InstallKind::LinkOnly);

        let p = probe(Edition::MacUniversal, "/Users/x/target/debug/furnica");
        assert_eq!(classify(&APP, &p), InstallKind::LinkOnly);

        let p = probe(Edition::Other, "/whatever/furnica");
        assert_eq!(classify(&APP, &p), InstallKind::LinkOnly);
    }

    fn name(kind: &InstallKind, edition: Edition) -> String {
        let r = parse_latest(LATEST).unwrap();
        asset_for(&APP, kind, edition, &r).unwrap().name.clone()
    }

    #[test]
    fn asset_names_per_edition() {
        let d = || PathBuf::from("/x");
        let portable = InstallKind::Portable { dir: d() };
        let msi = InstallKind::Msi { dir: d() };
        assert_eq!(
            name(&portable, Edition::WinX64),
            "furnica-3.0.5-windows-x64-portable.zip"
        );
        assert_eq!(
            name(&portable, Edition::WinX86),
            "furnica-3.0.5-windows-x86-portable.zip"
        );
        assert_eq!(
            name(&portable, Edition::Win7X86),
            "furnica-3.0.5-windows7-x86-portable.zip"
        );
        assert_eq!(name(&msi, Edition::WinX64), "furnica-3.0.5-windows-x64.msi");
        assert_eq!(name(&msi, Edition::WinX86), "furnica-3.0.5-windows-x86.msi");
        assert_eq!(
            name(&InstallKind::AppImage { path: d() }, Edition::LinuxX64),
            "furnica-3.0.5-linux-x86_64.AppImage"
        );
        assert_eq!(
            name(&InstallKind::MacApp { bundle: d() }, Edition::MacUniversal),
            "furnica-3.0.5-macos-universal.dmg"
        );
        assert_eq!(
            name(&InstallKind::Deb, Edition::LinuxX64),
            "furnica-3.0.5-linux-x86_64.deb"
        );
    }

    #[test]
    fn no_asset_for_edition() {
        let mut r = parse_latest(LATEST).unwrap();
        r.assets.retain(|a| !a.name.contains("windows7"));
        let portable = InstallKind::Portable { dir: "/x".into() };
        assert_eq!(
            asset_for(&APP, &portable, Edition::Win7X86, &r),
            Err(UpdateError::NoAsset)
        );
        assert_eq!(
            asset_for(&APP, &InstallKind::LinkOnly, Edition::WinX64, &r),
            Err(UpdateError::NoAsset)
        );
        // There is no MSI for Windows 7.
        let msi = InstallKind::Msi { dir: "/x".into() };
        assert_eq!(
            asset_for(&APP, &msi, Edition::Win7X86, &r),
            Err(UpdateError::NoAsset)
        );
    }
}
