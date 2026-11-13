//! Windows portable install: extract the zip next to the running copy, swap files
//! with rollback. Works on every OS; `prepare` limits it to Windows.

use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

use crate::{AppInfo, UpdateError};

const MAX_FILES: usize = 1000;
const OLD: &str = ".update-old";
const NEW: &str = ".update-new";
const MARKER: &str = "RESTORE-FAILED.txt";

fn io_err(dir: &Path, e: &io::Error) -> UpdateError {
    if e.kind() == io::ErrorKind::PermissionDenied {
        UpdateError::NotWritable(dir.to_path_buf())
    } else {
        UpdateError::Install(e.to_string())
    }
}

/// Replaces the files of the portable copy in `dir` with those of `zip`. Returns the
/// path of the new executable. On any failure the folder is left as it was.
pub fn install(zip: &Path, dir: &Path, exe_name: &str) -> Result<PathBuf, UpdateError> {
    let probe = dir.join(".update-probe");
    fs::write(&probe, b"").map_err(|_| UpdateError::NotWritable(dir.to_path_buf()))?;
    let _ = fs::remove_file(&probe);

    let new = dir.join(NEW);
    let old = dir.join(OLD);
    if old.join(MARKER).exists() {
        return Err(UpdateError::Install(format!(
            "a previous update failed to restore the old files; they are kept in {}, \
             restore them by hand and delete that folder",
            old.display()
        )));
    }
    let _ = fs::remove_dir_all(&new);
    let _ = fs::remove_dir_all(&old);

    let files = match extract(zip, &new, exe_name) {
        Ok(f) => f,
        Err(e) => {
            let _ = fs::remove_dir_all(&new);
            return Err(e);
        }
    };

    let mut done = Vec::new();
    if let Err(e) = swap(dir, &files, &mut done) {
        let restored = rollback(dir, done);
        return Err(swap_failure(dir, &e, restored));
    }
    let _ = fs::remove_dir_all(&new);
    Ok(dir.join(exe_name))
}

/// Builds the error for a failed swap. When the rollback worked, the work folders are
/// deleted; when it did not, `.update-old` is the only copy of the old files, so it is
/// kept together with a marker that stops install and cleanup from touching it.
fn swap_failure(dir: &Path, e: &io::Error, restored: bool) -> UpdateError {
    let old = dir.join(OLD);
    if restored {
        let _ = fs::remove_dir_all(dir.join(NEW));
        let _ = fs::remove_dir_all(&old);
        return if e.kind() == io::ErrorKind::PermissionDenied {
            io_err(dir, e)
        } else {
            UpdateError::Install(format!("{e}; the previous files were restored"))
        };
    }
    let _ = fs::create_dir_all(&old);
    let _ = fs::write(
        old.join(MARKER),
        format!(
            "An update failed and the old files could not all be put back.\r\n\
             Copy the files from this folder back into {} and delete this folder.\r\n",
            dir.display()
        ),
    );
    UpdateError::Install(format!(
        "{e}; restoring the previous files also failed. The old files are kept in {}",
        old.display()
    ))
}

/// Extracts `zip` into `new`, dropping the top folder. Returns the relative file paths.
fn extract(zip: &Path, new: &Path, exe_name: &str) -> Result<Vec<PathBuf>, UpdateError> {
    let bad = |m: String| UpdateError::Install(m);
    let file = fs::File::open(zip).map_err(|e| bad(format!("cannot open the update: {e}")))?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|e| bad(format!("the update is not a valid zip: {e}")))?;
    if archive.len() > MAX_FILES * 2 {
        return Err(bad("the update has too many entries".into()));
    }
    fs::create_dir_all(new).map_err(|e| io_err(new, &e))?;
    let mut files = Vec::new();
    let mut seen = HashSet::new();
    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| bad(format!("bad zip entry: {e}")))?;
        let name = entry
            .name()
            .map_err(|e| bad(format!("bad zip entry name: {e}")))?
            .to_string();
        if name.contains('\\') || name.contains(':') {
            return Err(bad(format!("unsafe path in the update: {name}")));
        }
        if entry.unix_mode().is_some_and(|m| m & 0o170000 == 0o120000) {
            return Err(bad(format!("link in the update: {name}")));
        }
        let mut rel = PathBuf::new();
        for (n, c) in Path::new(&name).components().enumerate() {
            match c {
                Component::Normal(p) => {
                    if n > 0 {
                        rel.push(p);
                    }
                }
                _ => return Err(bad(format!("unsafe path in the update: {name}"))),
            }
        }
        if rel.as_os_str().is_empty() {
            if entry.is_dir() {
                continue;
            }
            return Err(bad(format!("file outside the top folder: {name}")));
        }
        let target = new.join(&rel);
        if entry.is_dir() {
            fs::create_dir_all(&target).map_err(|e| io_err(new, &e))?;
            continue;
        }
        if !seen.insert(rel.to_string_lossy().to_lowercase()) {
            return Err(bad(format!("duplicate file in the update: {name}")));
        }
        if files.len() >= MAX_FILES {
            return Err(bad("the update has too many files".into()));
        }
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|e| io_err(new, &e))?;
        }
        let mut out = fs::File::create(&target).map_err(|e| io_err(new, &e))?;
        io::copy(&mut entry, &mut out).map_err(|e| io_err(new, &e))?;
        files.push(rel);
    }
    if !new.join(exe_name).is_file() {
        return Err(bad(format!("the update does not contain {exe_name}")));
    }
    files.sort();
    Ok(files)
}

enum Step {
    /// `dir/p` was moved to `.update-old/p`.
    MovedOld(PathBuf),
    /// `.update-new/p` was moved to `dir/p`.
    Installed(PathBuf),
    /// A folder created under `dir`.
    CreatedDir(PathBuf),
}

fn swap(dir: &Path, files: &[PathBuf], done: &mut Vec<Step>) -> io::Result<()> {
    let new = dir.join(NEW);
    let old = dir.join(OLD);
    for p in files {
        let target = dir.join(p);
        if let Some(parent) = p.parent().filter(|x| !x.as_os_str().is_empty()) {
            make_dirs(dir, parent, done)?;
        }
        if target.is_file() {
            let saved = old.join(p);
            if let Some(parent) = saved.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::rename(&target, &saved)?;
            done.push(Step::MovedOld(p.clone()));
        }
        fs::rename(new.join(p), &target)?;
        done.push(Step::Installed(p.clone()));
    }
    Ok(())
}

/// Creates the folders of `rel` under `dir`, recording each one that did not exist.
fn make_dirs(dir: &Path, rel: &Path, done: &mut Vec<Step>) -> io::Result<()> {
    let mut cur = dir.to_path_buf();
    let mut so_far = PathBuf::new();
    for c in rel.components() {
        cur.push(c);
        so_far.push(c);
        if !cur.exists() {
            fs::create_dir(&cur)?;
            done.push(Step::CreatedDir(so_far.clone()));
        } else if !cur.is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!("{} is a file, not a folder", cur.display()),
            ));
        }
    }
    Ok(())
}

/// Undoes the recorded steps in reverse order. Returns whether everything went back.
fn rollback(dir: &Path, done: Vec<Step>) -> bool {
    let mut ok = true;
    for step in done.into_iter().rev() {
        let r = match step {
            Step::Installed(p) => fs::rename(dir.join(&p), dir.join(NEW).join(&p)),
            Step::MovedOld(p) => fs::rename(dir.join(OLD).join(&p), dir.join(&p)),
            Step::CreatedDir(p) => fs::remove_dir(dir.join(&p)),
        };
        ok &= r.is_ok();
    }
    ok
}

/// Removes `.update-old`/`.update-new` in `dir` and `<name>-*.part` files in `temp`.
pub fn cleanup(app: &AppInfo, dir: &Path, temp: &Path) {
    // After a failed restore `.update-old` holds the only copy of the old files.
    if !dir.join(OLD).join(MARKER).exists() {
        let _ = fs::remove_dir_all(dir.join(OLD));
        let _ = fs::remove_dir_all(dir.join(NEW));
    }
    let prefix = format!("{}-", app.name);
    let Ok(entries) = fs::read_dir(temp) else {
        return;
    };
    for e in entries.flatten() {
        let name = e.file_name();
        let name = name.to_string_lossy();
        if name.starts_with(&prefix) && name.ends_with(".part") {
            let _ = fs::remove_file(e.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::Write;
    use zip::write::SimpleFileOptions;

    const TOP: &str = "furnica-9.9.9-windows-x64-portable";

    fn make_zip(path: &Path, files: &[(&str, &str)]) {
        let f = fs::File::create(path).unwrap();
        let mut z = zip::ZipWriter::new(f);
        for (name, body) in files {
            z.start_file(*name, SimpleFileOptions::default()).unwrap();
            z.write_all(body.as_bytes()).unwrap();
        }
        z.finish().unwrap();
    }

    fn test_app() -> AppInfo {
        AppInfo {
            name: "furnica",
            display_name: "Furnica",
            repo: "nazdravan/furnica",
            version: "3.0.5",
            windows_exe: "Furnica.exe",
            msi_registry_key: "",
            deb_package: "furnica",
            mac_bundle: "Furnica.app",
        }
    }

    fn read(p: impl AsRef<Path>) -> String {
        fs::read_to_string(p).unwrap()
    }

    fn old_folder(root: &Path) -> PathBuf {
        let dir = root.join("app");
        fs::create_dir_all(dir.join("Exemple")).unwrap();
        fs::create_dir_all(dir.join("soft-gl")).unwrap();
        fs::write(dir.join("Furnica.exe"), "old exe").unwrap();
        fs::write(dir.join("Exemple/a.ant"), "old ant").unwrap();
        fs::write(dir.join("soft-gl/x.dll"), "old dll").unwrap();
        fs::write(dir.join("notes.txt"), "mine").unwrap();
        dir
    }

    fn snapshot(dir: &Path) -> Vec<(String, String)> {
        let mut out = Vec::new();
        fn walk(base: &Path, p: &Path, out: &mut Vec<(String, String)>) {
            for e in fs::read_dir(p).unwrap() {
                let e = e.unwrap().path();
                if e.is_dir() {
                    walk(base, &e, out);
                } else {
                    out.push((
                        e.strip_prefix(base).unwrap().to_string_lossy().into_owned(),
                        fs::read_to_string(&e).unwrap(),
                    ));
                }
            }
        }
        walk(dir, dir, &mut out);
        out.sort();
        out
    }

    #[test]
    fn portable_swap_replaces_files() {
        let t = tempfile::tempdir().unwrap();
        let dir = old_folder(t.path());
        let zip = t.path().join("new.zip");
        make_zip(
            &zip,
            &[
                (&format!("{TOP}/Furnica.exe"), "new exe"),
                (&format!("{TOP}/Exemple/a.ant"), "new ant"),
                (&format!("{TOP}/soft-gl/x.dll"), "new dll"),
            ],
        );
        let exe = install(&zip, &dir, "Furnica.exe").unwrap();
        assert_eq!(exe, dir.join("Furnica.exe"));
        assert_eq!(read(dir.join("Furnica.exe")), "new exe");
        assert_eq!(read(dir.join("Exemple/a.ant")), "new ant");
        assert_eq!(read(dir.join("soft-gl/x.dll")), "new dll");
        assert_eq!(read(dir.join("notes.txt")), "mine");
        assert_eq!(read(dir.join(".update-old/Furnica.exe")), "old exe");
        assert!(!dir.join(".update-new").exists());
    }

    #[test]
    fn portable_swap_rolls_back() {
        // `Exemple` sorts before `Furnica.exe` (fails at once); `soft-gl` sorts after
        // it (fails when other files are already moved and must go back).
        for blocked in ["Exemple", "soft-gl"] {
            let t = tempfile::tempdir().unwrap();
            let dir = old_folder(t.path());
            fs::remove_dir_all(dir.join(blocked)).unwrap();
            fs::write(dir.join(blocked), "i am a file").unwrap();
            let zip = t.path().join("new.zip");
            make_zip(
                &zip,
                &[
                    (&format!("{TOP}/Furnica.exe"), "new exe"),
                    (&format!("{TOP}/Exemple/a.ant"), "new ant"),
                    (&format!("{TOP}/soft-gl/x.dll"), "new dll"),
                ],
            );
            let before = snapshot(&dir);
            assert!(install(&zip, &dir, "Furnica.exe").is_err(), "{blocked}");
            assert_eq!(snapshot(&dir), before, "{blocked}");
            assert_eq!(read(dir.join("Furnica.exe")), "old exe");
            assert!(!dir.join(".update-new").exists());
        }
    }

    #[test]
    fn portable_bad_zip_untouched() {
        let t = tempfile::tempdir().unwrap();
        let dir = old_folder(t.path());
        let before = snapshot(&dir);

        let no_exe = t.path().join("noexe.zip");
        make_zip(&no_exe, &[(&format!("{TOP}/other.txt"), "x")]);
        assert!(matches!(
            install(&no_exe, &dir, "Furnica.exe"),
            Err(UpdateError::Install(_))
        ));
        assert_eq!(snapshot(&dir), before);
        assert!(!dir.join(".update-new").exists());

        let evil = t.path().join("evil.zip");
        make_zip(
            &evil,
            &[
                (&format!("{TOP}/Furnica.exe"), "n"),
                (&format!("{TOP}/../evil"), "x"),
            ],
        );
        assert!(install(&evil, &dir, "Furnica.exe").is_err());
        assert_eq!(snapshot(&dir), before);
        assert!(!dir.join(".update-new").exists());
        assert!(!t.path().join("evil").exists());
    }

    #[test]
    fn portable_not_writable_is_reported() {
        let t = tempfile::tempdir().unwrap();
        let zip = t.path().join("new.zip");
        make_zip(&zip, &[(&format!("{TOP}/Furnica.exe"), "n")]);
        let missing = t.path().join("nope");
        assert_eq!(
            install(&zip, &missing, "Furnica.exe"),
            Err(UpdateError::NotWritable(missing))
        );
    }

    #[test]
    fn cleanup_removes_leftovers() {
        let t = tempfile::tempdir().unwrap();
        let dir = old_folder(t.path());
        fs::create_dir_all(dir.join(".update-old/sub")).unwrap();
        fs::write(dir.join(".update-old/sub/f"), "x").unwrap();
        fs::create_dir_all(dir.join(".update-new")).unwrap();
        let tmp = t.path().join("tmp");
        fs::create_dir_all(&tmp).unwrap();
        fs::write(tmp.join("furnica-1.0.0.part"), "x").unwrap();
        fs::write(tmp.join("furnica-x.msi"), "x").unwrap();
        fs::write(tmp.join("other-1.part"), "x").unwrap();
        let app = test_app();
        cleanup(&app, &dir, &tmp);
        assert!(!dir.join(".update-old").exists());
        assert!(!dir.join(".update-new").exists());
        assert!(dir.join("Furnica.exe").exists());
        assert!(!tmp.join("furnica-1.0.0.part").exists());
        assert!(tmp.join("furnica-x.msi").exists());
        assert!(tmp.join("other-1.part").exists());
    }

    #[test]
    fn failed_restore_keeps_old_copy() {
        let t = tempfile::tempdir().unwrap();
        let dir = old_folder(t.path());
        fs::create_dir_all(dir.join(".update-old")).unwrap();
        fs::write(dir.join(".update-old/Furnica.exe"), "old exe").unwrap();
        fs::create_dir_all(dir.join(".update-new")).unwrap();
        let e = io::Error::other("boom");
        let err = swap_failure(&dir, &e, false);
        let UpdateError::Install(msg) = err else {
            panic!("expected Install");
        };
        assert!(msg.contains(".update-old"), "{msg}");
        assert_eq!(read(dir.join(".update-old/Furnica.exe")), "old exe");
        assert!(dir.join(".update-old/RESTORE-FAILED.txt").exists());
        assert!(dir.join(".update-new").exists());

        let app = test_app();
        cleanup(&app, &dir, t.path());
        assert_eq!(read(dir.join(".update-old/Furnica.exe")), "old exe");
        assert!(dir.join(".update-new").exists());

        let zip = t.path().join("new.zip");
        make_zip(&zip, &[(&format!("{TOP}/Furnica.exe"), "new exe")]);
        let before = snapshot(&dir);
        assert!(matches!(
            install(&zip, &dir, "Furnica.exe"),
            Err(UpdateError::Install(m)) if m.contains(".update-old")
        ));
        assert_eq!(snapshot(&dir), before);
        assert_eq!(read(dir.join("Furnica.exe")), "old exe");
    }

    #[test]
    fn duplicate_entries_rejected() {
        let t = tempfile::tempdir().unwrap();
        let dir = old_folder(t.path());
        let before = snapshot(&dir);
        let exe = format!("{TOP}/Furnica.exe");
        // Exact duplicates cannot even be written by the zip crate (and its reader
        // rejects them); case-only duplicates are the case our own check must catch.
        let cases: [[(String, &str); 3]; 1] = [[
            (exe.clone(), "n"),
            (format!("{TOP}/Exemple/a.ant"), "1"),
            (format!("{TOP}/exemple/A.ant"), "2"),
        ]];
        for (i, entries) in cases.iter().enumerate() {
            let zip = t.path().join(format!("dup{i}.zip"));
            let f = fs::File::create(&zip).unwrap();
            let mut z = zip::ZipWriter::new(f);
            for (name, body) in entries {
                z.start_file(name.as_str(), SimpleFileOptions::default())
                    .unwrap();
                z.write_all(body.as_bytes()).unwrap();
            }
            z.finish().unwrap();
            assert!(
                matches!(
                    install(&zip, &dir, "Furnica.exe"),
                    Err(UpdateError::Install(_))
                ),
                "case {i}"
            );
            assert_eq!(snapshot(&dir), before, "case {i}");
            assert!(!dir.join(".update-new").exists());
            assert!(!dir.join(".update-old").exists());
        }
    }
}
