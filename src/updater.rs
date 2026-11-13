//! The updater: a state machine driven from a worker thread, and its egui window.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use crate::http::{Client, expected_sum};
use crate::install::{self, Prepared};
use crate::platform::{self, Edition};
use crate::texts::{Lang, texts};
use crate::version::{self, offers_update};
use crate::{AppInfo, Release, UpdateError};

/// What the window shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum State {
    Idle,
    Checking,
    UpToDate,
    Available { release: Release, installable: bool },
    Downloading { done: u64, total: Option<u64> },
    Installing,
    Ready { version: String },
    Failed(UpdateError),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdaterEvent {
    /// The user asked to restart into the new version. The host app should ask about
    /// unsaved work, then call [`Updater::restart`].
    RestartRequested,
}

struct Inner {
    state: State,
    prepared: Option<Prepared>,
    /// Bumped whenever a run is abandoned, so a late thread cannot overwrite the state.
    generation: u64,
}

pub struct Updater {
    app: AppInfo,
    inner: Arc<Mutex<Inner>>,
    cancel: Arc<AtomicBool>,
}

fn lock(m: &Mutex<Inner>) -> MutexGuard<'_, Inner> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl Updater {
    pub fn new(app: AppInfo) -> Self {
        Updater {
            app,
            inner: Arc::new(Mutex::new(Inner {
                state: State::Idle,
                prepared: None,
                generation: 0,
            })),
            cancel: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn is_busy(&self) -> bool {
        matches!(
            lock(&self.inner).state,
            State::Checking | State::Downloading { .. } | State::Installing
        )
    }

    #[doc(hidden)]
    pub fn set_state_for_test(&mut self, s: State) {
        lock(&self.inner).state = s;
    }

    /// Starts a check on a thread. Ignored while a check, a download or an install is
    /// running, and in `Ready` (the prepared update stays; the window shows it again).
    pub fn check(&mut self, ctx: &egui::Context) {
        let generation = {
            let mut g = lock(&self.inner);
            // In `Ready` an update is already prepared: show it again, don't drop it.
            if matches!(
                g.state,
                State::Checking
                    | State::Downloading { .. }
                    | State::Installing
                    | State::Ready { .. }
            ) {
                return;
            }
            g.state = State::Checking;
            g.prepared = None;
            g.generation += 1;
            g.generation
        };
        self.cancel = Arc::new(AtomicBool::new(false));
        let (app, inner, ctx) = (self.app, self.inner.clone(), ctx.clone());
        std::thread::spawn(move || {
            let set = |s: State| set_state(&inner, &ctx, generation, s);
            match check_thread(&app) {
                Ok(s) => set(s),
                Err(e) => set(failed(e)),
            }
        });
    }

    fn start_install(&self, ctx: &egui::Context, release: Release) {
        let generation = {
            let mut g = lock(&self.inner);
            g.generation += 1;
            g.state = State::Downloading {
                done: 0,
                total: None,
            };
            g.generation
        };
        self.cancel.store(false, Ordering::Relaxed);
        let cancel = self.cancel.clone();
        let (app, inner, ctx) = (self.app, self.inner.clone(), ctx.clone());
        std::thread::spawn(move || {
            let result = install_thread(
                &app,
                &release,
                &cancel,
                &mut |done, total| {
                    set_state(&inner, &ctx, generation, State::Downloading { done, total });
                },
                &mut || {
                    let entered = enter_installing(&inner, generation, &cancel);
                    if entered {
                        ctx.request_repaint();
                    }
                    entered
                },
            );
            match result {
                Ok(p) => {
                    let mut g = lock(&inner);
                    if g.generation == generation {
                        g.prepared = Some(p);
                        g.state = State::Ready {
                            version: release.version.to_string(),
                        };
                    }
                    drop(g);
                    ctx.request_repaint();
                }
                Err(e) => set_state(&inner, &ctx, generation, failed(e)),
            }
        });
    }

    /// Replaces the program and starts the new one. Only valid in `Ready`.
    pub fn restart(&mut self) -> Result<(), UpdateError> {
        let prepared = {
            let g = lock(&self.inner);
            match (&g.state, &g.prepared) {
                (State::Ready { .. }, Some(p)) => p.clone(),
                _ => return Err(UpdateError::Install("no update is ready".into())),
            }
        };
        install::restart(&prepared)
    }

    /// Draws the window (nothing while `Idle`) and reports button presses.
    pub fn show(&mut self, ctx: &egui::Context, lang: Lang) -> Option<UpdaterEvent> {
        let t = texts(lang);
        let state = lock(&self.inner).state.clone();
        if state == State::Idle {
            return None;
        }
        let mut event = None;
        let mut next: Option<State> = None;
        let mut start: Option<Release> = None;
        let mut cancel_clicked = false;
        egui::Window::new(t.title)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .show(ctx, |ui| match &state {
                State::Idle => {}
                State::Checking => {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(t.checking);
                    });
                }
                State::UpToDate => {
                    ui.label(t.up_to_date.replace("{v}", self.app.version));
                    if ui.button(t.close).clicked() {
                        next = Some(State::Idle);
                    }
                }
                State::Available {
                    release,
                    installable,
                } => {
                    ui.label(t.available.replace("{v}", &release.version.to_string()));
                    egui::ScrollArea::vertical()
                        .max_height(200.0)
                        .show(ui, |ui| {
                            ui.label(release.notes.as_str());
                        });
                    if *installable {
                        ui.horizontal(|ui| {
                            if ui.button(t.update).clicked() {
                                start = Some(release.clone());
                            }
                            if ui.button(t.later).clicked() {
                                next = Some(State::Idle);
                            }
                        });
                    } else {
                        ui.label(t.link_only);
                        ui.horizontal(|ui| {
                            if ui.button(t.open_page).clicked() {
                                ctx.open_url(egui::OpenUrl::new_tab(release.page.as_str()));
                            }
                            if ui.button(t.close).clicked() {
                                next = Some(State::Idle);
                            }
                        });
                    }
                }
                State::Downloading { done, total } => {
                    let bar = match total {
                        Some(n) if *n > 0 => egui::ProgressBar::new(*done as f32 / *n as f32),
                        _ => egui::ProgressBar::new(0.0).animate(true),
                    };
                    ui.add(bar);
                    ui.label(t.downloading);
                    if ui.button(t.cancel).clicked() {
                        self.cancel.store(true, Ordering::Relaxed);
                        cancel_clicked = true;
                    }
                }
                State::Installing => {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(t.installing);
                    });
                }
                State::Ready { version } => {
                    let on_restart = matches!(
                        lock(&self.inner).prepared,
                        Some(Prepared::MsiAfterExit { .. })
                    );
                    let text = if on_restart {
                        t.ready_on_restart
                    } else {
                        t.ready
                    };
                    ui.label(text.replace("{v}", version));
                    ui.horizontal(|ui| {
                        if ui.button(t.restart).clicked() {
                            event = Some(UpdaterEvent::RestartRequested);
                        }
                        if ui.button(t.later).clicked() {
                            next = Some(State::Idle);
                        }
                    });
                }
                State::Failed(e) => {
                    if let Some(msg) = t.error(e) {
                        ui.label(msg);
                    }
                    if ui.button(t.close).clicked() {
                        next = Some(State::Idle);
                    }
                }
            });
        if cancel_clicked {
            // Back to Idle only if still downloading: once `Installing` was entered
            // the cancel came too late and the run finishes (or fails) by itself.
            let mut g = lock(&self.inner);
            if matches!(g.state, State::Downloading { .. }) {
                g.generation += 1;
                g.state = State::Idle;
            }
        } else if let Some(r) = start {
            self.start_install(ctx, r);
        } else if let Some(s) = next {
            let mut g = lock(&self.inner);
            // Leaving the window abandons any run still in flight.
            g.generation += 1;
            g.state = s;
        }
        event
    }
}

/// `Cancelled` is not an error to show: the window just closes.
fn failed(e: UpdateError) -> State {
    if e == UpdateError::Cancelled {
        State::Idle
    } else {
        State::Failed(e)
    }
}

fn set_state(inner: &Mutex<Inner>, ctx: &egui::Context, generation: u64, s: State) {
    let mut g = lock(inner);
    if g.generation == generation {
        g.state = s;
        drop(g);
        ctx.request_repaint();
    }
}

fn check_thread(app: &AppInfo) -> Result<State, UpdateError> {
    let release = Client::new(app).latest()?;
    let current = version::parse(app.version)
        .ok_or_else(|| UpdateError::Install(format!("bad running version {}", app.version)))?;
    if !offers_update(&release.version, &current) {
        return Ok(State::UpToDate);
    }
    let kind = platform::detect(app);
    let installable = platform::asset_for(app, &kind, Edition::current(), &release).is_ok();
    Ok(State::Available {
        release,
        installable,
    })
}

fn install_thread(
    app: &AppInfo,
    release: &Release,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(u64, Option<u64>),
    installing: &mut dyn FnMut() -> bool,
) -> Result<Prepared, UpdateError> {
    let kind = platform::detect(app);
    let asset = platform::asset_for(app, &kind, Edition::current(), release)?;
    let client = Client::new(app);
    let sum = expected_sum(&client, release, &asset.name)?;
    let file = install::download_dir(app, &release.version)?.join(&asset.name);
    client.download(&asset.url, &file, &sum, progress, cancel)?;
    after_download(installing, || install::prepare(app, &kind, &file))
}

/// Moves to `Installing` and only then runs `prepare`. If the run was abandoned or
/// cancelled while the download finished, `prepare` is never reached: it may show
/// a password prompt or replace the program.
fn after_download<T>(
    installing: &mut dyn FnMut() -> bool,
    prepare: impl FnOnce() -> Result<T, UpdateError>,
) -> Result<T, UpdateError> {
    if !installing() {
        return Err(UpdateError::Cancelled);
    }
    prepare()
}

/// Sets `Installing` if this run is still current and was not cancelled. The check
/// and the write happen under one lock, so a Cancel pressed earlier always wins.
fn enter_installing(inner: &Mutex<Inner>, generation: u64, cancel: &AtomicBool) -> bool {
    let mut g = lock(inner);
    if g.generation != generation || cancel.load(Ordering::Relaxed) {
        return false;
    }
    g.state = State::Installing;
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::texts::Texts;
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;

    const APP: AppInfo = AppInfo {
        name: "furnica",
        display_name: "Furnica",
        repo: "nazdravan/furnica",
        version: "3.0.5",
        windows_exe: "Furnica.exe",
        msi_registry_key: "Software\\nazdravan\\Furnica",
        deb_package: "furnica",
        mac_bundle: "Furnica.app",
    };

    fn release() -> Release {
        crate::release::parse_latest(include_str!("../tests/fixtures/latest.json")).unwrap()
    }

    fn harness<'a>(s: State) -> Harness<'a, (Updater, Option<UpdaterEvent>)> {
        let mut u = Updater::new(APP);
        u.set_state_for_test(s);
        Harness::new_ui_state(
            |ui, (u, ev)| {
                if let Some(e) = u.show(ui.ctx(), Lang::Ro) {
                    *ev = Some(e);
                }
            },
            (u, None),
        )
    }

    fn available(installable: bool) -> State {
        State::Available {
            release: release(),
            installable,
        }
    }

    #[test]
    fn available_installable_shows_update() {
        let h = harness(available(true));
        h.get_by_label("Actualizează");
        h.get_by_label("Mai târziu");
        h.get_by_label(&format!("Versiunea {} e disponibilă.", release().version));
    }

    #[test]
    fn available_link_only_shows_page() {
        let h = harness(available(false));
        h.get_by_label("Deschide pagina versiunii");
        assert!(h.query_by_label("Actualizează").is_none());
    }

    #[test]
    fn ready_restart_emits_event() {
        let mut h = harness(State::Ready {
            version: "3.0.6".into(),
        });
        h.get_by_label("Repornește").click();
        h.run();
        assert_eq!(h.state().1, Some(UpdaterEvent::RestartRequested));
    }

    fn msi_prepared() -> Prepared {
        Prepared::MsiAfterExit {
            msi: "a.msi".into(),
            exe: "f.exe".into(),
            desktop_shortcut: true,
        }
    }

    #[test]
    fn ready_text_depends_on_prepared() {
        let mut h = harness(State::Ready {
            version: "3.0.6".into(),
        });
        h.get_by_label("Versiunea 3.0.6 e instalată. Repornește programul ca s-o folosești.");
        h.state_mut().0.inner.lock().unwrap().prepared = Some(msi_prepared());
        h.run();
        h.get_by_label("Versiunea 3.0.6 se instalează la repornire.");
    }

    #[test]
    fn check_in_ready_keeps_prepared() {
        let mut u = Updater::new(APP);
        u.set_state_for_test(State::Ready {
            version: "3.0.6".into(),
        });
        lock(&u.inner).prepared = Some(msi_prepared());
        u.check(&egui::Context::default());
        let g = lock(&u.inner);
        assert_eq!(
            g.state,
            State::Ready {
                version: "3.0.6".into()
            }
        );
        assert_eq!(g.prepared, Some(msi_prepared()));
        assert_eq!(g.generation, 0);
        drop(g);
        assert!(!u.is_busy());
    }

    fn downloading() -> Updater {
        let u = Updater::new(APP);
        {
            let mut g = lock(&u.inner);
            g.generation = 5;
            g.state = State::Downloading {
                done: 1,
                total: None,
            };
        }
        u
    }

    #[test]
    fn enter_installing_guard() {
        let u = downloading();
        let no = AtomicBool::new(false);
        // Stale generation: refused, state untouched.
        assert!(!enter_installing(&u.inner, 4, &no));
        assert!(matches!(lock(&u.inner).state, State::Downloading { .. }));
        // Cancel set: refused, state untouched.
        assert!(!enter_installing(&u.inner, 5, &AtomicBool::new(true)));
        assert!(matches!(lock(&u.inner).state, State::Downloading { .. }));
        // Current and not cancelled.
        assert!(enter_installing(&u.inner, 5, &no));
        assert_eq!(lock(&u.inner).state, State::Installing);
    }

    #[test]
    fn cancelled_after_download_never_prepares() {
        let u = downloading();
        let mut prepared = false;
        let cancel = AtomicBool::new(true);
        let r = after_download(&mut || enter_installing(&u.inner, 5, &cancel), || {
            prepared = true;
            Ok(())
        });
        assert_eq!(r, Err(UpdateError::Cancelled));
        assert!(!prepared);
        // Not cancelled: prepare runs.
        let r = after_download(
            &mut || enter_installing(&u.inner, 5, &AtomicBool::new(false)),
            || {
                prepared = true;
                Ok(())
            },
        );
        assert_eq!(r, Ok(()));
        assert!(prepared);
    }

    #[test]
    fn restart_outside_ready_is_error() {
        let mut u = Updater::new(APP);
        assert!(matches!(u.restart(), Err(UpdateError::Install(_))));
    }

    #[test]
    fn texts_complete() {
        for l in [Lang::Ro, Lang::En, Lang::Ru] {
            let t: &Texts = texts(l);
            for s in [
                t.check_button,
                t.title,
                t.checking,
                t.up_to_date,
                t.available,
                t.update,
                t.later,
                t.link_only,
                t.open_page,
                t.downloading,
                t.installing,
                t.ready,
                t.ready_on_restart,
                t.restart,
                t.cancel,
                t.close,
                t.err_network,
                t.err_rate,
                t.err_not_found,
                t.err_checksum,
                t.err_denied,
                t.err_not_writable,
                t.err_other,
            ] {
                assert!(!s.is_empty(), "{l:?}");
            }
            assert!(t.error(&UpdateError::Cancelled).is_none());
            assert!(t.error(&UpdateError::NoAsset).unwrap().contains("release"));
        }
    }
}
