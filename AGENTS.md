# AGENTS.md

`app-update` is a Rust library that updates our egui desktop apps from GitHub Releases.
It checks the latest release, downloads the asset that matches the running copy, verifies
it against `SHA256SUMS.txt`, installs it and restarts the app. It ships its own egui dialog
(RO/EN/RU) and a `--check-update` CLI mode. It does nothing on startup: an update runs only
when the user asks for it.

## Commands

- `cargo test`
- `cargo clippy --all-targets -- -D warnings`
- `cargo fmt`

CI runs the same checks on Linux, macOS and Windows, and builds for `i686-win7-windows-gnu`
with nightly and `-Z build-std` (see `.github/workflows/ci.yml`).

## Layout

- `src/lib.rs` — `AppInfo` (per-app constants) and `UpdateError`.
- `src/release.rs`, `src/http.rs` — GitHub "latest release" API and downloads (ureq + rustls).
- `src/sums.rs` — parsing `SHA256SUMS.txt`; an asset without a line is never installed.
- `src/version.rs` — semver comparison; no pre-releases, no downgrades.
- `src/platform.rs` — `Edition` (from `cfg`, not the OS), `InstallKind` detection, asset choice.
- `src/install/` — one module per install kind: `msi`, `portable`, `deb`, `rpm`, `appimage`, `macos`.
- `src/updater.rs` — the egui dialog and its state machine; `src/cli.rs` — `--check-update`.
- `src/texts.rs` — all dialog strings in Romanian, English and Russian.
- `tests/` — HTTP tests against a local server; fixtures in `tests/fixtures/`.

## Copy kinds

| Running copy | Asset suffix | Install |
|---|---|---|
| Windows MSI | `windows-<x64\|x86>.msi` | `msiexec` after the app exits (UAC), keeps folder and shortcut choice |
| Windows portable | `windows-<x64\|x86>-portable.zip`, `windows7-x86-portable.zip` | files replaced in place with rollback; only if a `LICENSE*` file sits next to the exe |
| Linux `.deb` | `linux-x86_64.deb` | `pkexec apt-get install` (or `dpkg -i`) |
| Linux `.rpm` | `linux-x86_64.rpm` | `pkexec` with `dnf`, else `zypper`, else `rpm -U` |
| Linux AppImage | `linux-x86_64.AppImage` | atomic rename over the old file, `pkexec` if the folder is read-only |
| macOS `.app` | `macos-universal.dmg` | mount, copy the bundle over the old one, unmount |
| anything else | none | link to the release page |

Assets are named `<name>-<version>-<suffix>`, the release tag is `v<version>`.
A `.deb`/`.rpm` copy is recognised only when the exe is `/usr/bin/<deb_package>` and the
package database lists it (dpkg is checked before rpm).

## Rules

- Windows 7: every dependency must compile for `i686-win7-windows-gnu`; TLS only with
  rustls and bundled webpki roots, never the system TLS.
- Anything downloaded is installed with elevated rights: keep downloads in a fresh private
  folder and never install a file whose SHA-256 was not verified.
- Code and comments are in English; dialog texts (RO/EN/RU) live in `src/texts.rs`.
- Changing `AppInfo` breaks every app that uses the library; prefer reusing fields.
- No attribution in commits or PRs: no trailer, no `Co-Authored-By`, no session link.
