# app-update

Self-update for egui desktop apps from GitHub Releases. The app shows a small
window with an "Check for updates" button; nothing runs at startup or in the
background. It looks at `https://api.github.com/repos/<repo>/releases/latest`,
downloads the asset for the running copy, verifies its SHA-256 against
`SHA256SUMS.txt` of the same release, installs it and offers to restart. If
anything fails, the old copy stays usable.

No pre-releases, no downgrades, no proxy, no token, no server of its own.
TLS is rustls with bundled roots, so it also builds for Windows 7
(`i686-win7-windows-gnu`).

## Copy kinds

| Running copy | Asset | Install |
|---|---|---|
| Windows MSI | `windows-<x64\|x86>.msi` | `msiexec` on the new MSI (UAC), keeps folder and shortcut choice |
| Windows portable | `windows-<x64\|x86>-portable.zip` (`windows-x86_64-portable.zip` also accepted), `windows7-x86-portable.zip` for the Win7 build | files replace those in the exe folder, with rollback; only when a `LICENSE*` file sits next to the exe (a lone exe gets a link) |
| Linux `.deb` | `linux-x86_64.deb` | system install through `pkexec` |
| Linux AppImage | `linux-x86_64.AppImage` | atomic rename over the old file, `pkexec` if the folder is read-only |
| macOS `.app` | `macos-universal.dmg` | mount, copy the new bundle over the old one, unmount |
| `.tar.gz`, unknown, translocated macOS | none | message with a link to the release page |

The edition (x64 / x86 / win7) comes from the build's `cfg`, not from the OS.

## Usage

```toml
[dependencies]
app-update = { git = "https://github.com/nazdravan/app-update", rev = "<sha>" }
```

```rust
const APP: app_update::AppInfo = app_update::AppInfo {
    name: "furnica",
    display_name: "Furnica",
    repo: "nazdravan/furnica",
    version: env!("CARGO_PKG_VERSION"),
    windows_exe: "Furnica.exe",
    msi_registry_key: r"Software\nazdravan\Furnica",
    deb_package: "furnica",
    mac_bundle: "Furnica.app",
};

fn main() {
    // `--check-update` prints what a check finds, without a GUI.
    if let Some(code) = app_update::run_cli(&APP) {
        std::process::exit(code);
    }
    // Removes leftovers of a previous update (old files, downloads).
    app_update::cleanup(&APP);
    // ... start eframe ...
}
```

In the UI, keep an `Updater` in the app state:

```rust
let mut updater = app_update::Updater::new(APP);

// Button handler:
updater.check(ctx);

// Every frame, while the window is open:
if let Some(app_update::UpdaterEvent::RestartRequested) = updater.show(ctx, lang) {
    // Ask about unsaved work first, then:
    let _ = updater.restart();
}
```

`Lang` selects the dialog language (RO, EN, RU).

## Release assets

The release tag is `v<version>`. Asset names are `<name>-<version>-<suffix>`:

```
furnica-3.0.5-windows-x64.msi
furnica-3.0.5-windows-x86.msi
furnica-3.0.5-windows-x64-portable.zip
furnica-3.0.5-windows-x86-portable.zip
furnica-3.0.5-windows7-x86-portable.zip
furnica-3.0.5-linux-x86_64.deb
furnica-3.0.5-linux-x86_64.AppImage
furnica-3.0.5-linux-x86_64.tar.gz
furnica-3.0.5-macos-universal.dmg
SHA256SUMS.txt
```

`SHA256SUMS.txt` has one `<hex>  <file name>` line per asset (the output of
`sha256sum`). An asset without a matching line is never installed.

## License

MIT OR Apache-2.0.
