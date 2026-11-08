//! The resident's windows: the main window shown or hidden, the app-drawn tray menu, and the close, PrtSc and About dialogs.
//!
//! On Windows and macOS the root viewport is an invisible host, so egui keeps running passes and
//! can open the tray menu while the main window (a child viewport) is hidden. On Linux the root is
//! the main window itself and a window cannot hide on Wayland: hiding closes it, the resident goes
//! on with no window (`system::headless`, where the desktop draws the tray menu) and the next show
//! makes a new one.

use crate::Shared;
use crate::screens::about::{self, AboutAnswer, AboutInfo};
use crate::screens::home::Home;
use crate::screens::tray_menu::{MenuOutcome, TrayMenu};
use crate::system::actions;
use crate::system::hotkeys::{Shot, effective_status, should_offer_priority};
use crate::system::instance::Page;
use crate::system::resident::{
    Resident, ResidentEvent, XAction, left_click, on_wayland, on_x, remembered,
};
use crate::system::tray::{TrayAction, TrayEntry, TrayEvent, tray_entries};
use crate::ui::i18n::{Msg, t as tr};
use crate::ui::kit::{self, BtnKind};
use crate::ui::tokens::Tokens;
use egui::{ViewportCommand as Cmd, ViewportId};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// Whether the root is an invisible host (Windows, macOS) or the main window itself (Linux).
pub const HOSTED: bool = !cfg!(target_os = "linux");

/// The main window's viewport when the root is the invisible host.
fn main_id() -> ViewportId {
    ViewportId::from_hash_of("takeshot-main")
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum OfferChoice {
    Later,
    Settings,
    Priority,
}

pub struct Shell {
    /// The main window is on screen (or about to be).
    pub main_open: bool,
    /// The root is an invisible host and the main window a child viewport (Windows, macOS).
    hosted: bool,
    menu: TrayMenu,
    recents: Arc<Mutex<Vec<PathBuf>>>,
    /// The first-X question, with its "Remember my choice" box.
    close_prompt: Option<bool>,
    about: bool,
    offer: bool,
    /// The About mark, for the theme and mode it was drawn in.
    mark: Option<((crate::ui::tokens::ThemeId, bool), egui::TextureHandle)>,
    /// The main window's icon, decoded once per theme.
    icon: Option<(crate::ui::tokens::ThemeId, Arc<egui::IconData>)>,
    quitting: bool,
    /// The window was told to close (hidden to the tray, or to be replaced): its close request
    /// is ours, not the user's X.
    leaving: bool,
    /// The window closes to be replaced by a new one.
    reopen: bool,
    /// The page the next window opens on, asked for while there was no window.
    page: Option<Page>,
    /// Quit was chosen: the next tick asks the other windows.
    quit_asked: bool,
    /// A Quit waiting for the editors, pinned pictures and capture to answer.
    quit_run: Option<crate::system::instance::QuitRun>,
    /// Bumped by the recents thread, so the Linux native menu is rebuilt when the list changes.
    recents_version: Arc<std::sync::atomic::AtomicU64>,
    menu_version: u64,
    next_recents: f64,
    /// The theme the tray icon shows, and the language the native menu was built in.
    tray_theme: Option<crate::ui::tokens::ThemeId>,
    lang: Option<crate::ui::i18n::Lang>,
}

impl Shell {
    pub fn new(hosted: bool, main_open: bool) -> Self {
        Self {
            main_open,
            hosted,
            menu: TrayMenu::default(),
            recents: Arc::new(Mutex::new(Vec::new())),
            close_prompt: None,
            about: false,
            offer: false,
            mark: None,
            icon: None,
            quitting: false,
            leaving: false,
            reopen: false,
            page: None,
            quit_asked: false,
            quit_run: None,
            recents_version: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            menu_version: u64::MAX,
            next_recents: 0.0,
            tray_theme: None,
            lang: None,
        }
    }

    /// The first window opens on `page` (`--page`), when one was named.
    pub fn with_page(mut self, page: Option<Page>) -> Self {
        self.page = page;
        self
    }

    /// A main window should be on screen (Linux: the loop opens one when there is none).
    pub fn wants_window(&self) -> bool {
        self.main_open
    }

    /// Quit was confirmed: the process is about to end.
    pub fn is_quitting(&self) -> bool {
        self.quitting
    }

    /// The page the next window opens on, if one was asked for while there was none.
    pub fn take_page(&mut self) -> Option<Page> {
        self.page.take()
    }

    /// The window is gone (Linux). A window nobody asked to replace stays gone.
    pub fn window_closed(&mut self) {
        self.leaving = false;
        if !std::mem::take(&mut self.reopen) {
            self.main_open = false;
        }
        self.close_prompt = None;
        // Textures belong to the window's context.
        self.mark = None;
    }

    /// Whether a close request is the user's X (and gets the close prompt or the setting's
    /// answer), not the close the resident asked for.
    fn intercepts_close(&self) -> bool {
        !self.quitting && !self.leaving
    }

    /// Keeps the recent captures fresh (every 30 s) and the Linux native menu in step with them
    /// and the mode. Called on every wake-up, window shown or not.
    pub fn tick(&mut self, ctx: &egui::Context, resident: &Resident, shared: &Shared) {
        if std::mem::take(&mut self.quit_asked) {
            self.start_quit(ctx, resident);
        }
        self.check_quit(ctx);
        let now = ctx.input(|i| i.time);
        if now >= self.next_recents {
            self.next_recents = now + 30.0;
            self.refresh_recents(ctx, shared);
            ctx.request_repaint_after(std::time::Duration::from_secs(30));
        }
        let theme = shared.overrides.theme.unwrap_or(shared.settings.ui_theme);
        if self.theme_changed(theme) {
            if let Some(tray) = &resident.tray {
                tray.set_theme(theme);
            }
            // The hosted main window takes its icon from its builder each frame.
            if !self.hosted {
                ctx.send_viewport_cmd_to(
                    ViewportId::ROOT,
                    Cmd::Icon(Some(Arc::new(crate::ui::brand::window_icon(theme)))),
                );
            }
        }
        let lang = crate::ui::i18n::lang();
        if self.lang != Some(lang) {
            self.lang = Some(lang);
            self.menu_version = u64::MAX;
        }
        let v = self
            .recents_version
            .load(std::sync::atomic::Ordering::Relaxed);
        if v != self.menu_version
            && let Some(tray) = &resident.tray
        {
            self.menu_version = v;
            tray.refresh_menu(&self.entries(shared));
        }
    }

    /// True once per theme change (and on the first call), when the icons must follow.
    fn theme_changed(&mut self, now: crate::ui::tokens::ThemeId) -> bool {
        if self.tray_theme == Some(now) {
            return false;
        }
        self.tray_theme = Some(now);
        true
    }

    fn theme(ctx: &egui::Context, shared: &Shared) -> Tokens {
        shared.tokens(ctx.system_theme())
    }

    /// The tray menu entries for now; capture items lose their shortcut text when shortcuts are off.
    fn entries(&self, shared: &Shared) -> Vec<TrayEntry> {
        let recents = self.recents.lock().map(|r| r.clone()).unwrap_or_default();
        let s = &shared.settings;
        let mut e = tray_entries(&recents, s.ui_mode, &s.hotkey_area, &s.hotkey_full);
        if !s.hotkeys {
            for x in &mut e {
                if let TrayEntry::Item { keys, .. } = x {
                    *keys = None;
                }
            }
        }
        e
    }

    /// Lists the recent captures off the UI thread (the folder may be large or on a network share).
    fn refresh_recents(&self, ctx: &egui::Context, shared: &Shared) {
        let dir = PathBuf::from(&shared.settings.save_dir);
        let (out, ctx) = (self.recents.clone(), ctx.clone());
        let version = self.recents_version.clone();
        std::thread::spawn(move || {
            let list = crate::system::recents::recent_captures(&dir, 5);
            if store_recents(&out, &version, list) {
                ctx.request_repaint();
            }
        });
    }

    /// Reacts to an event from the hotkeys, the hook, the tray or a second launch.
    pub fn handle(
        &mut self,
        ctx: &egui::Context,
        ev: ResidentEvent,
        home: Option<&mut Home>,
        shared: &mut Shared,
    ) {
        match ev {
            ResidentEvent::Show(p) => self.show(ctx, home, p),
            ResidentEvent::Shot(s) => self.capture(ctx, s),
            ResidentEvent::Tray(TrayEvent::LeftClick) => {
                let e = left_click(shared.settings.tray_click);
                self.handle(ctx, e, home, shared);
            }
            ResidentEvent::Tray(TrayEvent::MenuAt { icon }) => {
                if self.hosted {
                    self.refresh_recents(ctx, shared);
                    let ppp = ctx.pixels_per_point();
                    let points = |r: egui::Rect| {
                        egui::Rect::from_min_max(
                            (r.min.to_vec2() / ppp).to_pos2(),
                            (r.max.to_vec2() / ppp).to_pos2(),
                        )
                    };
                    // The menus stay off the taskbar: the work area of the icon's monitor.
                    let bounds = crate::system::capture::work_area(icon.center())
                        .map(points)
                        .unwrap_or_else(|| {
                            let size = ctx
                                .input(|i| i.viewport().monitor_size)
                                .unwrap_or(egui::vec2(1920.0, 1080.0));
                            egui::Rect::from_min_size(egui::Pos2::ZERO, size)
                        });
                    let entries = self.entries(shared);
                    self.menu.open(points(icon), bounds, &entries);
                }
            }
            ResidentEvent::Tray(TrayEvent::Action(a)) => self.run(ctx, a, home, shared),
        }
    }

    fn run(
        &mut self,
        ctx: &egui::Context,
        a: TrayAction,
        home: Option<&mut Home>,
        shared: &mut Shared,
    ) {
        match a {
            TrayAction::Capture(s) => self.capture(ctx, s),
            TrayAction::CaptureIn3s => {
                self.minimize_main(ctx);
                let _ = actions::spawn_self(&["--capture", "--delay", "3"]);
            }
            TrayAction::Show(p) => self.show(ctx, home, p),
            TrayAction::CopyRecent(p) => match image::open(&p) {
                Ok(img) => match actions::copy_image(&img.to_rgba8()) {
                    Ok(()) => shared.toast(tr(Msg::Copy).to_string()),
                    Err(e) => shared.toast(e),
                },
                Err(e) => shared.toast(e.to_string()),
            },
            TrayAction::EditRecent(p) => {
                let _ = actions::spawn_self(&["--edit", &p.to_string_lossy()]);
            }
            TrayAction::OpenCapturesFolder => {
                actions::open_folder(std::path::Path::new(&shared.settings.save_dir));
            }
            TrayAction::SetMode(m) => {
                shared.settings.ui_mode = m;
                shared.settings_dirty = true;
                // The native menu shows the new check on its next rebuild.
                self.menu_version = u64::MAX;
            }
            TrayAction::Quit => self.quit(ctx),
        }
    }

    /// The main window's viewport: a child of the host, or the root itself on Linux.
    fn main_viewport(&self) -> ViewportId {
        if self.hosted {
            main_id()
        } else {
            ViewportId::ROOT
        }
    }

    fn show(&mut self, ctx: &egui::Context, home: Option<&mut Home>, page: Page) {
        self.show_in(ctx, home, page, on_wayland());
    }

    /// Shows the main window on `page`: brought forward where it is, or replaced by a new one
    /// where the compositor will not raise it. With no window yet the page waits for the next.
    fn show_in(&mut self, ctx: &egui::Context, home: Option<&mut Home>, page: Page, wayland: bool) {
        let focused = ctx.input(|i| i.viewport().focused);
        let replace = !self.hosted && needs_new_window(self.main_open, focused, wayland);
        self.main_open = true;
        if replace {
            self.leaving = true;
            self.reopen = true;
            ctx.send_viewport_cmd(Cmd::Close);
        } else {
            let id = self.main_viewport();
            ctx.send_viewport_cmd_to(id, Cmd::Minimized(false));
            ctx.send_viewport_cmd_to(id, Cmd::Focus);
        }
        match home {
            _ if page == Page::About => self.about = true,
            Some(home) if !replace => home.open(page),
            _ => self.page = Some(page),
        }
    }

    /// Hides the main window to the tray: on Windows and macOS the child viewport just stops
    /// being drawn; on Linux the window closes, since Wayland cannot hide one. With no panel
    /// showing the tray icon (`tray_shown` false) it only minimizes, the way back.
    fn hide(&mut self, ctx: &egui::Context, tray_shown: bool) {
        if self.hosted {
            self.main_open = false;
        } else if tray_shown {
            self.main_open = false;
            self.leaving = true;
            ctx.send_viewport_cmd(Cmd::Close);
        } else {
            ctx.send_viewport_cmd(Cmd::Minimized(true));
        }
    }

    fn minimize_main(&self, ctx: &egui::Context) {
        if self.main_open {
            ctx.send_viewport_cmd_to(self.main_viewport(), Cmd::Minimized(true));
        }
    }

    fn capture(&self, ctx: &egui::Context, s: Shot) {
        self.minimize_main(ctx);
        let args: &[&str] = match s {
            Shot::Area => &["--capture", "--wait"],
            Shot::Full => &["--capture", "--full", "--wait"],
        };
        let _ = actions::spawn_self(args);
    }

    /// Quit: the next tick asks every window of its own to close first.
    fn quit(&mut self, ctx: &egui::Context) {
        self.quit_asked = true;
        ctx.request_repaint();
    }

    /// Asks every window of its own whether it may close (an editor with changes asks its user);
    /// once all have answered, they and the resident close, or all stay if one was cancelled.
    fn start_quit(&mut self, ctx: &egui::Context, resident: &Resident) {
        use crate::system::instance;
        if self.quit_run.is_some() {
            return;
        }
        let base = instance::endpoint();
        let names = resident
            .children()
            .iter()
            .map(|pid| instance::child_endpoint(&base, *pid))
            .collect();
        let wake = resident.sender();
        self.quit_run = Some(instance::QuitRun::start(names, move || wake.wake()));
        self.check_quit(ctx);
    }

    /// Reads the Quit's answers that came in and quits, or stays, once all are in.
    fn check_quit(&mut self, ctx: &egui::Context) {
        use crate::system::instance::QuitOutcome;
        let Some(run) = &mut self.quit_run else {
            return;
        };
        match run.poll() {
            QuitOutcome::Waiting => {}
            QuitOutcome::Stay => self.quit_run = None,
            QuitOutcome::Quit => {
                self.quit_run = None;
                self.quitting = true;
                ctx.send_viewport_cmd_to(ViewportId::ROOT, Cmd::Close);
            }
        }
    }

    /// Draws the resident's windows for this frame. `ui` is the root's: the host on Windows and
    /// macOS, the main window on Linux.
    pub fn ui(
        &mut self,
        ui: &mut egui::Ui,
        frame: &eframe::Frame,
        resident: &mut Resident,
        home: &mut Home,
        shared: &mut Shared,
    ) {
        let ctx = ui.ctx().clone();
        let hook = crate::system::resident::wants_hook(&shared.settings);
        if cfg!(windows) && resident.priority_on() != hook {
            resident.set_priority(hook);
        }
        // The Settings switch acts at once, like the priority above.
        if resident.hotkeys.enabled() != shared.settings.hotkeys {
            resident.hotkeys.set_enabled(shared.settings.hotkeys);
        }
        let raw = resident.hotkeys.status().clone();
        home.hotkey_status = Some(effective_status(&raw, resident.priority_on()));
        if cfg!(windows)
            && self.main_open
            && should_offer_priority(
                &raw,
                shared.settings.prtsc_offer_shown,
                resident.priority_on(),
            )
        {
            self.offer = true;
        }
        if self.hosted {
            if self.main_open {
                let theme = shared.tokens(ctx.system_theme()).theme;
                if self.icon.as_ref().is_none_or(|(t, _)| *t != theme) {
                    self.icon = Some((theme, Arc::new(crate::ui::brand::window_icon(theme))));
                }
                let icon = self
                    .icon
                    .as_ref()
                    .map(|(_, i)| i.clone())
                    .unwrap_or_default();
                let vp = crate::ui::chrome::frameless(egui::ViewportBuilder::default())
                    .with_title("Takeshot")
                    .with_app_id("takeshot")
                    .with_inner_size([1160.0, 740.0])
                    .with_min_inner_size([900.0, 600.0])
                    .with_drag_and_drop(true)
                    .with_icon(icon);
                ctx.show_viewport_immediate(main_id(), vp, |ui, _| {
                    self.main_contents(ui, frame, resident, home, shared);
                });
            }
            if self.menu.is_open() {
                let t = Self::theme(&ctx, shared);
                let entries = self.entries(shared);
                let (state, text) =
                    crate::screens::home::chip(home.hotkey_status.as_ref(), &shared.settings);
                let thumb = |p: &std::path::Path| home.thumbnail(p);
                if let MenuOutcome::Chosen(a) =
                    self.menu.show(&ctx, &t, &entries, (state, &text), &thumb)
                {
                    self.run(&ctx, a, Some(home), shared);
                }
            }
        } else {
            self.main_contents(ui, frame, resident, home, shared);
        }
    }

    /// The main window: Home, its requests, the X behaviour and the dialogs over it.
    fn main_contents(
        &mut self,
        ui: &mut egui::Ui,
        _frame: &eframe::Frame,
        resident: &mut Resident,
        home: &mut Home,
        shared: &mut Shared,
    ) {
        let ctx = ui.ctx().clone();
        if ui.input(|i| i.viewport().close_requested()) && self.intercepts_close() {
            ctx.send_viewport_cmd(Cmd::CancelClose);
            match on_x(shared.settings.on_close) {
                XAction::Ask => self.close_prompt = Some(true),
                XAction::Hide => self.hide(&ctx, resident.tray_shown()),
                XAction::Quit => self.quit(&ctx),
            }
        }
        let t = shared.tokens(ctx.system_theme());
        crate::views::begin_frame(&ctx, &t);
        for a in crate::views::main_window(ui, &t, home, shared) {
            home.apply(a, &ctx, shared);
        }
        let req = std::mem::take(&mut home.requests);
        if req.about {
            self.about = true;
        }
        if req.quit {
            self.quit(&ctx);
        }
        if let Some((shot, keys)) = req.rebind {
            let hook_on = resident.priority_on();
            let result = resident.hotkeys.rebind(shot, &keys, hook_on);
            let saved = result.is_ok();
            match crate::screens::home::apply_rebind(
                &mut shared.settings,
                shot,
                &keys,
                result,
                hook_on,
            ) {
                Some(notice) => shared.toast(notice),
                None if saved => shared.settings_dirty = true,
                None => {}
            }
        }
        if req.open_windows_settings {
            open_windows_keyboard_settings();
        }
        let t = Self::theme(&ctx, shared);
        self.dialogs(&ctx, &t, resident, shared);
        draw_toast(&ctx, shared);
    }

    fn dialogs(
        &mut self,
        ctx: &egui::Context,
        t: &Tokens,
        resident: &mut Resident,
        shared: &mut Shared,
    ) {
        if let Some(mut remember) = self.close_prompt {
            let answer = kit::confirm_dialog(
                ctx,
                t,
                "takeshot-close-app",
                tr(Msg::CloseAppTitle),
                tr(Msg::CloseAppBody),
                &mut |ui: &mut egui::Ui| {
                    kit::checkbox(ui, t, &mut remember, tr(Msg::RememberChoice));
                },
                &[
                    (tr(Msg::Quit), BtnKind::Secondary, XAction::Quit),
                    (tr(Msg::MinimizeToTray), BtnKind::Primary, XAction::Hide),
                ],
                XAction::Ask,
            );
            self.close_prompt = Some(remember);
            if let Some(choice) = answer {
                self.close_prompt = None;
                if let Some(on_close) = remembered(choice, remember) {
                    shared.settings.on_close = on_close;
                    shared.settings_dirty = true;
                }
                match choice {
                    XAction::Quit => self.quit(ctx),
                    XAction::Hide => self.hide(ctx, resident.tray_shown()),
                    XAction::Ask => {}
                }
            }
        }
        if self.about {
            let look = (t.theme, t.dark);
            if self.mark.as_ref().is_none_or(|(l, _)| *l != look) {
                self.mark = Some((look, crate::ui::brand::mark_texture(ctx, look.0, look.1)));
            }
            let info = AboutInfo::current();
            match about::dialog(ctx, t, &info, self.mark.as_ref().map(|(_, m)| m.id())) {
                Some(AboutAnswer::GitHub) => ctx.open_url(egui::OpenUrl::new_tab(info.github)),
                Some(AboutAnswer::Close) => self.about = false,
                None => {}
            }
        }
        if self.offer && self.close_prompt.is_none() && !self.about {
            let answer = kit::confirm_dialog(
                ctx,
                t,
                "takeshot-prtsc-offer",
                tr(Msg::PrtScBusy),
                tr(Msg::PrtScConflictHelp),
                &mut |_| {},
                &[
                    (
                        tr(Msg::OpenWindowsSettings),
                        BtnKind::Secondary,
                        OfferChoice::Settings,
                    ),
                    (
                        tr(Msg::PrtScPriority),
                        BtnKind::Primary,
                        OfferChoice::Priority,
                    ),
                ],
                OfferChoice::Later,
            );
            if let Some(choice) = answer {
                self.offer = false;
                shared.settings.prtsc_offer_shown = true;
                shared.settings_dirty = true;
                match choice {
                    OfferChoice::Priority => {
                        shared.settings.prtsc_priority = true;
                        resident.set_priority(true);
                    }
                    OfferChoice::Settings => open_windows_keyboard_settings(),
                    OfferChoice::Later => {}
                }
            }
        }
    }
}

/// Whether showing the main window means making a new one: on Wayland a client cannot focus its
/// window or bring it back from minimized, so one that is not in front is replaced (the new window
/// opens on the workspace the user is on).
fn needs_new_window(window_open: bool, focused: Option<bool>, wayland: bool) -> bool {
    wayland && window_open && focused != Some(true)
}

/// Stores a fresh list of recent captures; the version (which rebuilds the Linux native menu, closing
/// it if open) moves only when the list changed. True when it changed.
fn store_recents(
    out: &Mutex<Vec<PathBuf>>,
    version: &std::sync::atomic::AtomicU64,
    list: Vec<PathBuf>,
) -> bool {
    let Ok(mut r) = out.lock() else {
        return false;
    };
    if *r == list {
        return false;
    }
    *r = list;
    version.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    true
}

/// Windows' keyboard settings, where the Snipping Tool's PrtSc option lives.
fn open_windows_keyboard_settings() {
    #[cfg(windows)]
    {
        let _ = std::process::Command::new("explorer")
            .arg("ms-settings:easeofaccess-keyboard")
            .spawn();
    }
}

/// The short notice, drawn in whichever window is the main one.
fn draw_toast(ctx: &egui::Context, shared: &mut Shared) {
    if let Some((msg, at)) = &shared.toast {
        if at.elapsed().as_secs_f32() < crate::ui::kit::toast_seconds(msg) {
            crate::ui::kit::toast(ctx, &shared.tokens(ctx.system_theme()), msg);
            ctx.request_repaint();
        } else {
            shared.toast = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The commands `f` sent to the root window during one pass.
    fn commands(f: impl FnOnce(&egui::Context)) -> Vec<Cmd> {
        let ctx = egui::Context::default();
        let mut f = Some(f);
        let mut full = ctx.run_ui(egui::RawInput::default(), |ui| {
            if let Some(f) = f.take() {
                f(ui.ctx());
            }
        });
        full.textures_delta.clear();
        full.viewport_output
            .remove(&ViewportId::ROOT)
            .map(|v| v.commands)
            .unwrap_or_default()
    }

    #[test]
    fn hiding_closes_the_window_where_the_root_is_the_window() {
        // Linux: the window cannot hide on Wayland, so it is destroyed and made again.
        let mut shell = Shell::new(false, true);
        assert!(shell.intercepts_close(), "the X is the user's to answer");
        let sent = commands(|ctx| shell.hide(ctx, true));
        assert!(sent.contains(&Cmd::Close), "{sent:?}");
        assert!(!shell.main_open);
        assert!(
            !shell.intercepts_close(),
            "the close we asked for must not be asked about again"
        );
        shell.window_closed();
        assert!(shell.intercepts_close(), "the next window asks again");
    }

    #[test]
    fn without_a_tray_to_come_back_from_the_window_is_only_minimized() {
        // GNOME without the AppIndicator extension shows no tray icon: a window destroyed there
        // could only be brought back by starting the app again.
        let mut shell = Shell::new(false, true);
        let sent = commands(|ctx| shell.hide(ctx, false));
        assert!(sent.contains(&Cmd::Minimized(true)), "{sent:?}");
        assert!(!sent.contains(&Cmd::Close), "{sent:?}");
        assert!(shell.wants_window(), "the window is still there");
        assert!(shell.intercepts_close(), "and its X still asks");
    }

    #[test]
    fn hiding_the_hosted_window_closes_nothing() {
        // Windows and macOS: the child viewport just stops being drawn.
        let mut shell = Shell::new(true, true);
        let sent = commands(|ctx| shell.hide(ctx, true));
        assert!(!sent.contains(&Cmd::Close), "{sent:?}");
        assert!(!shell.main_open);
        assert!(shell.intercepts_close());
    }

    #[test]
    fn a_window_that_closed_on_its_own_stays_closed() {
        // Nothing asked for another one: no loop of windows opening and closing.
        let mut shell = Shell::new(false, true);
        shell.window_closed();
        assert!(!shell.main_open);
    }

    #[test]
    fn showing_without_a_window_asks_for_one_on_that_page() {
        let mut shell = Shell::new(false, false);
        let mut shared = Shared::for_tests();
        let ctx = egui::Context::default();
        shell.handle(&ctx, ResidentEvent::Show(Page::Settings), None, &mut shared);
        assert!(shell.wants_window());
        assert_eq!(shell.take_page(), Some(Page::Settings));
        assert_eq!(shell.take_page(), None, "the page is used once");
        shell.handle(&ctx, ResidentEvent::Show(Page::About), None, &mut shared);
        assert!(shell.about, "About is a dialog of the new window");
        assert_eq!(shell.take_page(), None);
    }

    #[test]
    fn a_window_the_compositor_will_not_raise_is_made_again() {
        // Wayland drops a client's request to focus itself (and to leave the minimized state);
        // a new window comes up on the workspace the user is on.
        assert!(needs_new_window(true, Some(false), true), "unfocused");
        assert!(needs_new_window(true, None, true), "unknown focus");
        assert!(
            !needs_new_window(true, Some(true), true),
            "already in front"
        );
        assert!(
            !needs_new_window(true, Some(false), false),
            "X11 can focus it"
        );
        assert!(!needs_new_window(false, None, true), "no window to replace");
    }

    #[test]
    fn showing_an_unfocused_wayland_window_replaces_it_on_the_page_asked() {
        let mut shell = Shell::new(false, true);
        let sent = commands(|ctx| shell.show_in(ctx, None, Page::Style, true));
        assert!(sent.contains(&Cmd::Close), "{sent:?}");
        assert!(shell.main_open, "a window is still wanted");
        assert!(!shell.intercepts_close());
        shell.window_closed();
        assert!(shell.wants_window(), "the loop opens the replacement");
        assert_eq!(shell.take_page(), Some(Page::Style));

        // Not on Wayland: the window is brought forward where it is.
        let mut shell = Shell::new(false, true);
        let sent = commands(|ctx| shell.show_in(ctx, None, Page::Start, false));
        assert!(!sent.contains(&Cmd::Close), "{sent:?}");
        assert!(sent.contains(&Cmd::Focus), "{sent:?}");
    }

    #[test]
    fn the_tray_icon_follows_the_theme() {
        use crate::ui::tokens::ThemeId;
        let mut shell = Shell::new(true, true);
        assert!(
            shell.theme_changed(ThemeId::TakeUi),
            "the first look sets the icon"
        );
        assert!(
            !shell.theme_changed(ThemeId::TakeUi),
            "same theme: nothing to do"
        );
        assert!(
            shell.theme_changed(ThemeId::Coral),
            "a new theme changes it"
        );
        assert!(!shell.theme_changed(ThemeId::Coral));
    }

    #[test]
    fn the_native_menu_is_rebuilt_only_when_the_recents_change() {
        // Rebuilding the Linux menu closes it under the pointer: same list, same version.
        let (out, version) = (Mutex::new(Vec::new()), std::sync::atomic::AtomicU64::new(0));
        let v = || version.load(std::sync::atomic::Ordering::Relaxed);
        store_recents(&out, &version, vec![PathBuf::from("a.png")]);
        assert_eq!(v(), 1);
        store_recents(&out, &version, vec![PathBuf::from("a.png")]);
        assert_eq!(v(), 1, "same list");
        store_recents(
            &out,
            &version,
            vec![PathBuf::from("b.png"), PathBuf::from("a.png")],
        );
        assert_eq!(v(), 2);
        assert_eq!(out.lock().unwrap().len(), 2);
    }
}
