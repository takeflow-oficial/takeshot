#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
//! Takeshot: a screenshot tool with an editor for backgrounds, frames and watermarks.
//!
//! Modes (each runs in its own window/process):
//!   (none)                 main window, listens for the capture keys
//!   --capture [--full] [--delay S] [--wait] [--sample]   area capture overlay
//!   --edit [file]          editor for a picture (a sample when no file)
//!   --pin file             picture pinned on top of other windows
//!   --page inicio|estilo|ajustes   main window on that page
//!   --page componentes     component gallery of the design system
//!   --page editor|captura  the editor on the sample picture, the capture on the sample screen
//!   --screenshot out.png   draw ~120 frames, save the window as PNG, exit
//!   --theme takeui|coral --mode light|dark|system --lang en|pt|auto   override settings for this run only (every window)
//!   --native-titlebar      the system's title bar instead of the theme's (also TAKESHOT_NATIVE_TITLEBAR=1)
//!   --captures-dir path    the gallery reads this folder for this run only
//!   --select name          Início starts with this capture chosen (for screenshots)

mod engine;
mod screens;
mod system;
mod ui;
mod views;

use engine::{model, render};
use screens::{editor, home, notice, overlay, pin, widgets};
use system::{actions, capture, fonts, settings};

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

pub struct Shared {
    pub settings: settings::Settings,
    pub settings_dirty: bool,
    pub fonts: Arc<fonts::FontLib>,
    pub logo: widgets::LogoCache,
    toast: Option<(String, std::time::Instant)>,
    /// `--theme` and `--mode` for this run only; the saved settings stay as they are.
    pub overrides: RunOverrides,
}

#[derive(Clone, Default, Debug)]
pub struct RunOverrides {
    pub theme: Option<ui::tokens::ThemeId>,
    pub mode: Option<ui::tokens::UiMode>,
    /// `--captures-dir`: the gallery reads this folder instead of the saved one.
    pub captures_dir: Option<String>,
    /// `--select`: Início starts with this capture chosen.
    pub select: Option<String>,
}

impl Shared {
    pub fn new(settings: settings::Settings, overrides: RunOverrides) -> Self {
        Self {
            settings,
            settings_dirty: false,
            fonts: Arc::new(fonts::FontLib::load()),
            logo: widgets::LogoCache::new(),
            toast: None,
            overrides,
        }
    }

    /// A `Shared` for tests: default settings with a captures folder that does not exist.
    #[cfg(test)]
    pub fn for_tests() -> Self {
        Self {
            settings: settings::Settings {
                save_dir: std::env::temp_dir()
                    .join("takeshot-test-no-captures")
                    .to_string_lossy()
                    .into_owned(),
                ..Default::default()
            },
            settings_dirty: false,
            fonts: Arc::new(fonts::FontLib::load()),
            logo: widgets::LogoCache::new(),
            toast: None,
            overrides: RunOverrides::default(),
        }
    }

    /// The folder the gallery reads: the run's override, else the setting.
    pub fn captures_dir(&self) -> &str {
        self.overrides
            .captures_dir
            .as_deref()
            .unwrap_or(&self.settings.save_dir)
    }

    /// The look now: the run's override, else the setting; `system` decides the System mode.
    pub fn tokens(&self, system: Option<egui::Theme>) -> ui::tokens::Tokens {
        let theme = self.overrides.theme.unwrap_or(self.settings.ui_theme);
        let mode = self.overrides.mode.unwrap_or(self.settings.ui_mode);
        ui::tokens::tokens(theme, ui::tokens::effective_dark(mode, system))
    }

    pub fn toast(&mut self, msg: String) {
        self.toast = Some((msg, std::time::Instant::now()));
    }
}

pub enum Switch {
    None,
    ToEditor(Box<editor::Editor>),
    ToNotice(Box<notice::Notice>),
    Close,
}

enum Mode {
    Home(Box<home::Home>),
    /// The resident: main window, tray, hotkeys and the PrtSc hook. On Linux the main window
    /// comes and goes (`home` is its state, none while there is no window).
    Resident {
        home: Option<Box<home::Home>>,
        shell: Box<screens::shell::Shell>,
        resident: Box<system::resident::Resident>,
    },
    Overlay(Box<overlay::Overlay>),
    Editor(Box<editor::Editor>),
    Pin(Box<pin::Pin>),
    Notice(Box<notice::Notice>),
    Gallery(Box<ui::gallery::Gallery>),
}

enum Launch {
    Home,
    Capture {
        full: bool,
    },
    Edit(Option<PathBuf>),
    Pin(PathBuf),
    Gallery,
    /// The copy notice, for screenshots (`--page aviso`).
    Notice,
}

struct Args {
    launch: Launch,
    /// Start in the tray without a window (start with the system).
    minimized: bool,
    screenshot: Option<PathBuf>,
    delay: f32,
    sample: bool,
    theme: Option<ui::tokens::ThemeId>,
    mode: Option<ui::tokens::UiMode>,
    lang: Option<ui::i18n::LangPref>,
    /// The main window's page to open on (`--page inicio|estilo|ajustes`).
    page: Option<system::instance::Page>,
    captures_dir: Option<String>,
    select: Option<String>,
}

/// What `--page` names: a page of the main window, or the component gallery.
pub enum PageArg {
    Main(system::instance::Page),
    Gallery,
    /// The editor with the sample picture.
    Editor,
    /// The capture overlay on the sample screen.
    Capture,
    /// The notice shown after a copy.
    Notice,
}

pub fn parse_page(s: &str) -> Option<PageArg> {
    use system::instance::Page;
    Some(match s {
        "inicio" => PageArg::Main(Page::Start),
        "estilo" => PageArg::Main(Page::Style),
        "ajustes" => PageArg::Main(Page::Settings),
        "componentes" => PageArg::Gallery,
        "editor" => PageArg::Editor,
        "captura" => PageArg::Capture,
        "aviso" => PageArg::Notice,
        _ => return None,
    })
}

fn parse_args() -> Args {
    let mut a = Args {
        launch: Launch::Home,
        minimized: false,
        screenshot: None,
        delay: 0.0,
        sample: false,
        theme: None,
        mode: None,
        lang: None,
        page: None,
        captures_dir: None,
        select: None,
    };
    let v: Vec<String> = std::env::args().skip(1).collect();
    let mut full = false;
    let mut capture = false;
    let mut i = 0;
    while i < v.len() {
        match v[i].as_str() {
            "--screenshot" => {
                a.screenshot = v.get(i + 1).map(PathBuf::from);
                i += 1;
            }
            "--capture" => capture = true,
            "--minimized" => a.minimized = true,
            "--native-titlebar" => ui::chrome::init(true),
            "--captures-dir" => {
                a.captures_dir = v.get(i + 1).cloned();
                i += 1;
            }
            "--select" => {
                a.select = v.get(i + 1).cloned();
                i += 1;
            }
            "--full" => full = true,
            "--sample" => a.sample = true,
            "--wait" => a.delay = a.delay.max(0.3),
            "--delay" => {
                a.delay = v.get(i + 1).and_then(|s| s.parse().ok()).unwrap_or(3.0);
                i += 1;
            }
            "--edit" => {
                let file = v
                    .get(i + 1)
                    .filter(|s| !s.starts_with("--"))
                    .map(PathBuf::from);
                if file.is_some() {
                    i += 1;
                }
                a.launch = Launch::Edit(file);
            }
            "--theme" => {
                a.theme = match v.get(i + 1).map(String::as_str) {
                    Some("takeui") => Some(ui::tokens::ThemeId::TakeUi),
                    Some("coral") => Some(ui::tokens::ThemeId::Coral),
                    other => unknown_value("--theme", other),
                };
                i += 1;
            }
            "--mode" => {
                a.mode = match v.get(i + 1).map(String::as_str) {
                    Some("light") => Some(ui::tokens::UiMode::Light),
                    Some("dark") => Some(ui::tokens::UiMode::Dark),
                    Some("system") => Some(ui::tokens::UiMode::System),
                    other => unknown_value("--mode", other),
                };
                i += 1;
            }
            "--lang" => {
                a.lang = match v.get(i + 1).map(String::as_str) {
                    Some("en") => Some(ui::i18n::LangPref::En),
                    Some("pt") => Some(ui::i18n::LangPref::Pt),
                    Some("auto") => Some(ui::i18n::LangPref::Auto),
                    other => unknown_value("--lang", other),
                };
                i += 1;
            }
            "--page" => {
                match v.get(i + 1).map(String::as_str).and_then(parse_page) {
                    Some(PageArg::Gallery) => a.launch = Launch::Gallery,
                    Some(PageArg::Main(p)) => a.page = Some(p),
                    Some(PageArg::Editor) => a.launch = Launch::Edit(None),
                    Some(PageArg::Notice) => a.launch = Launch::Notice,
                    Some(PageArg::Capture) => {
                        capture = true;
                        a.sample = true;
                    }
                    None => {
                        unknown_value::<()>("--page", v.get(i + 1).map(String::as_str));
                    }
                }
                i += 1;
            }
            "--pin" => {
                if let Some(f) = v.get(i + 1) {
                    a.launch = Launch::Pin(PathBuf::from(f));
                    i += 1;
                }
            }
            _ => {}
        }
        i += 1;
    }
    if capture {
        a.launch = Launch::Capture { full };
    }
    a
}

/// The main window's state, on the page `--page` named (Início otherwise).
fn main_home(
    ctx: &egui::Context,
    shared: &Shared,
    page: Option<system::instance::Page>,
) -> home::Home {
    let mut h = home::Home::new(ctx, shared);
    if let Some(p) = page {
        h.open(p);
    }
    if let Some(name) = &shared.overrides.select {
        h.select_by_name(name);
    }
    h
}

/// Reports a flag value the app does not know, so a typo never yields a silent fallback.
fn unknown_value<T>(flag: &str, value: Option<&str>) -> Option<T> {
    eprintln!("takeshot: unknown value {value:?} for {flag}; using the saved setting");
    None
}

struct App {
    mode: Mode,
    shared: Shared,
    screenshot: Option<PathBuf>,
    frames: u32,
    shot_saved: Arc<AtomicBool>,
    shot_requested: bool,
    editor_window: (egui::Pos2, egui::Vec2),
    /// Work area of the capture's monitor, in points (for placing the notice).
    notice_area: egui::Rect,
    /// The resident's Quit, waiting for this window's answer (editors, pinned pictures, capture).
    quit_request: Arc<std::sync::Mutex<Option<std::sync::mpsc::Sender<bool>>>>,
    /// Every window agreed to the Quit: this one closes.
    quit_close: Arc<AtomicBool>,
}

impl App {
    fn switch_to_editor(&mut self, ctx: &egui::Context, e: Box<editor::Editor>) {
        use egui::ViewportCommand as C;
        let (pos, size) = self.editor_window;
        if !cfg!(target_os = "macos") {
            ctx.send_viewport_cmd(C::Fullscreen(false));
        }
        ctx.send_viewport_cmd(C::WindowLevel(egui::WindowLevel::Normal));
        // The editor draws the theme's own bar; the system's only with the native fallback.
        ctx.send_viewport_cmd(C::Decorations(ui::chrome::native()));
        ctx.send_viewport_cmd(C::Resizable(true));
        ctx.send_viewport_cmd(C::InnerSize(size));
        ctx.send_viewport_cmd(C::OuterPosition(pos));
        ctx.send_viewport_cmd(C::Title(screens::editor::EDITOR_TITLE.into()));
        self.mode = Mode::Editor(e);
    }

    /// The resident's Quit, answered here so a minimized or hidden window answers too (only
    /// `logic` runs for those): an editor with changes comes to the front to ask its user; the
    /// rest agree at once. Every window closes only once all have agreed.
    fn answer_quit(&mut self, ctx: &egui::Context) {
        use egui::ViewportCommand as C;
        let root = egui::ViewportId::ROOT;
        if let Some(reply) = self.quit_request.lock().ok().and_then(|mut q| q.take()) {
            let asks = match &mut self.mode {
                Mode::Editor(e) => e.quit_requested(reply),
                _ => {
                    let _ = reply.send(true);
                    false
                }
            };
            if asks {
                ctx.send_viewport_cmd_to(root, C::Minimized(false));
                ctx.send_viewport_cmd_to(root, C::Focus);
                ctx.send_viewport_cmd_to(
                    root,
                    C::RequestUserAttention(egui::UserAttentionType::Informational),
                );
                ctx.request_repaint();
            }
        }
        if self.quit_close.swap(false, Ordering::SeqCst) {
            if let Mode::Editor(e) = &mut self.mode {
                e.close_for_quit();
            }
            self.shared.settings.save();
            ctx.send_viewport_cmd_to(root, C::Close);
        }
    }

    fn switch_to_notice(&mut self, ctx: &egui::Context, n: Box<notice::Notice>) {
        use egui::ViewportCommand as C;
        if !cfg!(target_os = "macos") {
            ctx.send_viewport_cmd(C::Fullscreen(false));
        }
        ctx.send_viewport_cmd(C::Decorations(false));
        ctx.send_viewport_cmd(C::WindowLevel(egui::WindowLevel::AlwaysOnTop));
        ctx.send_viewport_cmd(C::InnerSize(notice::NOTICE_SIZE));
        ctx.send_viewport_cmd(C::OuterPosition(notice::notice_pos(self.notice_area)));
        self.mode = Mode::Notice(n);
    }
}

impl App {
    /// What happens on every wake-up, whether a window is shown or not: the answer to the
    /// resident's Quit, and (for the resident) its events, its tray and its settings.
    fn pump(&mut self, ctx: &egui::Context) {
        self.answer_quit(ctx);
        if let Mode::Resident {
            home,
            shell,
            resident,
        } = &mut self.mode
        {
            for ev in resident.poll() {
                shell.handle(ctx, ev, home.as_deref_mut(), &mut self.shared);
            }
            shell.tick(ctx, resident, &self.shared);
            if self.shared.settings_dirty {
                self.shared.settings_dirty = false;
                self.shared.settings.save();
            }
        }
    }

    /// The resident's main window opens (Linux): its context gets the fonts, the look and a
    /// fresh Início, and the events wake it.
    fn attach(&mut self, ctx: &egui::Context) {
        self.shared.fonts.install(ctx);
        // Ctrl+0, Ctrl+1 and Ctrl+plus or minus belong to the editor's zoom, not egui's UI zoom.
        ctx.options_mut(|o| o.zoom_with_keyboard = false);
        self.shared.logo = widgets::LogoCache::new();
        self.shared.logo.sync(ctx, &self.shared.settings.watermark);
        views::begin_frame(ctx, &self.shared.tokens(ctx.system_theme()));
        if let Mode::Resident {
            home,
            shell,
            resident,
        } = &mut self.mode
        {
            resident.retarget(ctx);
            *home = Some(Box::new(main_home(ctx, &self.shared, shell.take_page())));
        }
    }

    /// The resident's main window closed (Linux): what belonged to its context goes, and the
    /// events wake the resident's loop again.
    fn detach(&mut self, headless: &egui::Context) {
        if let Mode::Resident {
            home,
            shell,
            resident,
        } = &mut self.mode
        {
            resident.retarget(headless);
            *home = None;
            shell.window_closed();
        }
    }

    /// A panel shows the resident's tray icon.
    fn tray_shown(&self) -> bool {
        matches!(&self.mode, Mode::Resident { resident, .. } if resident.tray_shown())
    }

    /// Opens the main window (Linux), as the tray's "Open" would.
    fn show_main(&mut self, ctx: &egui::Context) {
        if let Mode::Resident { home, shell, .. } = &mut self.mode {
            shell.handle(
                ctx,
                system::resident::ResidentEvent::Show(system::instance::Page::Start),
                home.as_deref_mut(),
                &mut self.shared,
            );
        }
    }

    /// The resident wants a main window on screen.
    fn wants_window(&self) -> bool {
        matches!(&self.mode, Mode::Resident { shell, .. } if shell.wants_window())
    }

    /// The resident's Quit was confirmed.
    fn is_quitting(&self) -> bool {
        matches!(&self.mode, Mode::Resident { shell, .. } if shell.is_quitting())
    }
}

impl eframe::App for App {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.pump(ctx);
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        // The resident's root is a transparent host on Windows and macOS.
        if matches!(self.mode, Mode::Resident { .. }) && screens::shell::HOSTED {
            [0.0; 4]
        } else {
            // eframe's default, as before the resident.
            egui::Color32::from_rgba_unmultiplied(12, 12, 12, 180).to_normalized_gamma_f32()
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let sw = match &mut self.mode {
            Mode::Home(h) => {
                let t = self.shared.tokens(ctx.system_theme());
                views::begin_frame(&ctx, &t);
                for a in views::main_window(ui, &t, h, &mut self.shared) {
                    h.apply(a, &ctx, &mut self.shared);
                }
                Switch::None
            }
            Mode::Resident {
                home,
                shell,
                resident,
            } => {
                if let Some(home) = home {
                    shell.ui(ui, frame, resident, home, &mut self.shared);
                }
                Switch::None
            }
            Mode::Overlay(o) => {
                let t = self.shared.tokens(ctx.system_theme());
                views::begin_frame(&ctx, &t);
                o.ui(ui, Some(frame), &t, &mut self.shared)
            }
            Mode::Editor(e) => {
                let t = self.shared.tokens(ctx.system_theme());
                views::begin_frame(&ctx, &t);
                e.ui(ui, frame, &t, &mut self.shared)
            }
            Mode::Pin(p) => {
                let t = self.shared.tokens(ctx.system_theme());
                views::begin_frame(&ctx, &t);
                p.ui(ui, frame, &t, &mut self.shared)
            }
            Mode::Notice(n) => {
                let t = self.shared.tokens(ctx.system_theme());
                views::begin_frame(&ctx, &t);
                n.ui(ui, &t)
            }
            Mode::Gallery(g) => {
                let fill = g.tokens().shell;
                egui::CentralPanel::no_frame()
                    .frame(
                        egui::Frame::new()
                            .fill(fill)
                            .inner_margin(egui::Margin::same(24)),
                    )
                    .show(ui, |ui| g.ui(ui));
                Switch::None
            }
        };
        match sw {
            Switch::ToEditor(e) => self.switch_to_editor(&ctx, e),
            Switch::ToNotice(n) => {
                self.shared.settings.save();
                if system::notify::native_notices() {
                    // The capture window is full screen and Wayland would keep it so: the
                    // desktop shows the notice and the window leaves at once.
                    system::notify::show(n.for_desktop());
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                } else {
                    self.switch_to_notice(&ctx, n);
                }
            }
            Switch::Close => {
                self.shared.settings.save();
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            Switch::None => {}
        }

        // The resident draws its notices in the main window (the root may be an invisible host).
        if let Some((msg, at)) = &self.shared.toast
            && !matches!(self.mode, Mode::Resident { .. })
        {
            if at.elapsed().as_secs_f32() < ui::kit::toast_seconds(msg) {
                ui::kit::toast(&ctx, &self.shared.tokens(ctx.system_theme()), msg);
                ctx.request_repaint();
            } else {
                self.shared.toast = None;
            }
        }
        if self.shared.settings_dirty && !ctx.input(|i| i.pointer.any_down()) {
            self.shared.settings_dirty = false;
            if self.screenshot.is_none() {
                self.shared.settings.save();
            }
        }

        // self-check: draw ~120 frames, then save the window picture
        if let Some(path) = &self.screenshot {
            self.frames += 1;
            ctx.request_repaint();
            if self.frames >= 120 && !self.shot_requested {
                self.shot_requested = true;
                let path = path.clone();
                let done = self.shot_saved.clone();
                ctx.request_screenshot(move |img| {
                    let [w, h] = img.size;
                    let mut bytes = Vec::with_capacity(w * h * 4);
                    for p in &img.pixels {
                        bytes.extend_from_slice(&p.to_srgba_unmultiplied());
                    }
                    if let Some(rgba) = image::RgbaImage::from_raw(w as u32, h as u32, bytes)
                        && let Err(e) = actions::save_png(&rgba, &path)
                    {
                        eprintln!("screenshot failed: {e}");
                    }
                    done.store(true, Ordering::SeqCst);
                });
            }
            if self.shot_saved.load(Ordering::SeqCst) || self.frames > 600 {
                // A sample editor opens with marks, so it would ask before closing.
                if let Mode::Editor(e) = &mut self.mode {
                    e.force_close();
                }
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
    }
}

fn main() -> eframe::Result {
    ui::chrome::init(false);
    let args = parse_args();
    let settings = settings::Settings::load();
    // The language first: window titles are set before the first frame.
    ui::i18n::set_lang(ui::i18n::resolve(
        args.lang.unwrap_or(settings.lang),
        ui::i18n::system_locale().as_deref(),
    ));

    // The resident is one per user: a second launch shows the running one and leaves.
    let resident_mode = matches!(args.launch, Launch::Home) && args.screenshot.is_none();
    let mut listener = None;
    if resident_mode {
        let name = system::instance::endpoint();
        match system::instance::claim(&name) {
            system::instance::Claim::Secondary => {
                if let Some(page) = system::instance::second_launch_page(args.minimized) {
                    let _ = system::instance::send_show(&name, page);
                }
                return Ok(());
            }
            system::instance::Claim::Primary(l) => listener = Some(l),
            system::instance::Claim::Alone => {}
        }
    }

    // Linux: the main window comes and goes, so the resident runs its windows one at a time.
    if resident_mode && !screens::shell::HOSTED {
        let res = run_resident_windows(&args, settings, listener);
        actions::wait_clipboard_keepers();
        return res;
    }

    // One capture at a time: while a selection is on screen, another PrtSc is ignored.
    let mut capture_slot = None;
    if matches!(args.launch, Launch::Capture { .. }) && args.screenshot.is_none() && !args.sample {
        match system::instance::capture_slot(&system::instance::endpoint()) {
            Some(slot) => capture_slot = Some(slot),
            None => return Ok(()),
        }
    }

    // Grab the screen before any window of ours exists.
    let mut shot = None;
    let mut capture_error = None;
    if let Launch::Capture { .. } = args.launch {
        if args.delay > 0.0 {
            std::thread::sleep(std::time::Duration::from_secs_f32(args.delay));
        }
        if args.sample || args.screenshot.is_some() {
            shot = Some(capture::sample_shot());
        } else {
            match capture::grab() {
                Ok(s) => shot = Some(s),
                Err(e) => capture_error = Some(e),
            }
        }
    }

    let mut vp = base_viewport(args.theme.unwrap_or(settings.ui_theme));
    let mut editor_window = (egui::pos2(80.0, 60.0), egui::vec2(1280.0, 820.0));
    let mut notice_area = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1600.0, 900.0));
    let mut instant_notice = None;
    let mut pin_img = None;
    match (&args.launch, &shot) {
        (Launch::Capture { full }, Some(s)) => {
            let k = s.scale;
            let pos = egui::pos2(s.x as f32 / k, s.y as f32 / k);
            let size = egui::vec2(s.image.width() as f32 / k, s.image.height() as f32 / k);
            let ew = egui::vec2(1280.0f32.min(size.x - 40.0), 820.0f32.min(size.y - 60.0));
            editor_window = (pos + (size - ew) / 2.0, ew);
            // The notice keeps off the taskbar: the work area of this monitor, in points.
            let middle = egui::pos2(
                s.x as f32 + s.image.width() as f32 / 2.0,
                s.y as f32 + s.image.height() as f32 / 2.0,
            );
            notice_area = capture::work_area(middle)
                .map(|r| {
                    egui::Rect::from_min_max(
                        (r.min.to_vec2() / k).to_pos2(),
                        (r.max.to_vec2() / k).to_pos2(),
                    )
                })
                .unwrap_or(egui::Rect::from_min_size(pos, size));
            if *full && !settings.open_editor_after_full && args.screenshot.is_none() {
                // Shift+PrtSc saves the full screen at once.
                let fonts = fonts::FontLib::load();
                let logo = settings
                    .watermark
                    .logo_path
                    .as_deref()
                    .and_then(render::load_logo);
                let img = render::finish(
                    &s.image,
                    &settings,
                    logo.as_ref(),
                    &fonts,
                    settings.quick_style,
                );
                let path = actions::auto_path(&settings.save_dir);
                if let Err(e) = actions::save_png(&img, &path) {
                    eprintln!("save failed: {e}");
                    return Ok(());
                }
                if !settings.show_bubbles {
                    return Ok(());
                }
                if system::notify::native_notices() {
                    // No window at all for the notice: the desktop shows it.
                    system::notify::show(notice::Notice::saved(path).for_desktop());
                    system::notify::wait_pending();
                    return Ok(());
                }
                vp = vp
                    .with_title("Takeshot")
                    .with_decorations(false)
                    .with_always_on_top()
                    .with_resizable(false)
                    .with_inner_size(notice::NOTICE_SIZE)
                    .with_position(notice::notice_pos(notice_area));
                instant_notice = Some(notice::Notice::saved(path));
            } else if *full && settings.open_editor_after_full {
                vp = vp
                    .with_title("Takeshot · editor")
                    .with_inner_size(ew)
                    .with_position(editor_window.0);
            } else if args.screenshot.is_some() {
                vp = vp
                    .with_title(ui::i18n::t(ui::i18n::Msg::CaptureWindowTitle))
                    .with_inner_size([1280.0, 720.0])
                    .with_decorations(false);
            } else {
                vp = vp
                    .with_title(ui::i18n::t(ui::i18n::Msg::CaptureWindowTitle))
                    .with_decorations(false)
                    .with_position(pos)
                    .with_inner_size(size)
                    .with_always_on_top()
                    .with_resizable(false);
                if !cfg!(target_os = "macos") {
                    vp = vp.with_fullscreen(true);
                }
            }
        }
        (Launch::Edit(_), _) => {
            vp = ui::chrome::frameless(vp)
                .with_title(screens::editor::EDITOR_TITLE)
                .with_inner_size([1280.0, 820.0])
                .with_min_inner_size([900.0, 600.0]);
        }
        (Launch::Pin(p), _) => {
            if let Ok(img) = image::open(p) {
                let img = img.to_rgba8();
                let sz = egui::vec2(img.width() as f32, img.height() as f32);
                let k = (900.0 / sz.x).min(700.0 / sz.y).min(1.0);
                vp = vp
                    .with_title("Takeshot")
                    .with_decorations(false)
                    .with_always_on_top()
                    .with_inner_size(sz * k)
                    .with_resizable(false);
                pin_img = Some((img, sz * k));
                let _ = std::fs::remove_file(p);
            }
        }
        (Launch::Gallery, _) => {
            vp = vp
                .with_title(ui::i18n::t(ui::i18n::Msg::GalleryWindowTitle))
                .with_inner_size([1280.0, 900.0]);
        }
        (Launch::Notice, _) => {
            vp = vp
                .with_title("Takeshot")
                .with_decorations(false)
                .with_resizable(false)
                .with_inner_size(notice::NOTICE_SIZE);
        }
        _ if resident_mode => {
            // An invisible host: tiny, transparent, click-through, off screen and off the taskbar.
            // Its own title, so the window lookups by "Takeshot" find only the main window.
            vp = vp
                .with_title("Takeshot resident")
                .with_inner_size([1.0, 1.0])
                .with_position([-10000.0, -10000.0])
                .with_decorations(false)
                .with_transparent(true)
                .with_mouse_passthrough(true)
                .with_taskbar(false);
        }
        _ => vp = main_window_viewport(vp),
    }

    let options = eframe::NativeOptions {
        viewport: vp,
        ..Default::default()
    };
    let screenshot = args.screenshot.clone();
    let demo = args.sample;
    let res = eframe::run_native(
        "Takeshot",
        options,
        Box::new(move |cc| {
            let ctx = &cc.egui_ctx;
            // Ctrl+0, Ctrl+1 and Ctrl+plus or minus belong to the editor's zoom, not egui's UI zoom.
            ctx.options_mut(|o| o.zoom_with_keyboard = false);
            // Theme, mode and language: command-line overrides win for this run only.
            let theme = args.theme.unwrap_or(settings.ui_theme);
            let ui_mode = args.mode.unwrap_or(settings.ui_mode);
            let lang_pref = args.lang.unwrap_or(settings.lang);
            ui::i18n::set_lang(ui::i18n::resolve(
                lang_pref,
                ui::i18n::system_locale().as_deref(),
            ));
            let mut shared = Shared::new(settings, run_overrides(&args));
            shared.fonts.install(ctx);
            shared.logo.sync(ctx, &shared.settings.watermark);
            views::begin_frame(ctx, &shared.tokens(ctx.system_theme()));
            let mode = match args.launch {
                Launch::Capture { .. } if instant_notice.is_some() => {
                    Mode::Notice(Box::new(instant_notice.take().unwrap()))
                }
                Launch::Capture { full } => match shot {
                    Some(s) if full && shared.settings.open_editor_after_full => {
                        Mode::Editor(Box::new(editor::Editor::new(
                            ctx,
                            s.image,
                            engine::doc::Doc::new(),
                            &shared,
                        )))
                    }
                    Some(s) => {
                        // The overlay holds the slot; the editor and the notice paths drop it here.
                        let o = overlay::Overlay::new(ctx, s, &shared).hold(capture_slot.take());
                        Mode::Overlay(Box::new(if demo { o.demo() } else { o }))
                    }
                    None => {
                        shared.toast(ui::i18n::tf(
                            ui::i18n::Msg::CaptureFailed,
                            &[&capture_error.unwrap_or_default()],
                        ));
                        Mode::Home(Box::new(home::Home::new(ctx, &shared)))
                    }
                },
                Launch::Edit(file) => {
                    let img = file
                        .as_ref()
                        .and_then(|p| image::open(p).ok())
                        .map(|i| i.to_rgba8());
                    let (img, demo) = match img {
                        Some(i) => (i, false),
                        None => (capture::sample_window(1200, 740), true),
                    };
                    // A real file keeps its name in the title bar; a temporary hand-off is removed
                    // and shows the name a new capture would get.
                    let mut name = None;
                    if let Some(p) = &file {
                        if p.starts_with(std::env::temp_dir()) {
                            let _ = std::fs::remove_file(p);
                        } else if !demo {
                            name = p.file_name().map(|n| n.to_string_lossy().into_owned());
                        }
                    }
                    let anns = if demo { demo_annotations() } else { Vec::new() };
                    let ed =
                        editor::Editor::new(ctx, img, engine::doc::Doc::from_anns(anns), &shared);
                    Mode::Editor(Box::new(match name {
                        Some(n) => ed.with_name(n),
                        None => ed,
                    }))
                }
                Launch::Pin(_) => match pin_img {
                    Some((img, size)) => Mode::Pin(Box::new(pin::Pin::new(img, size))),
                    None => Mode::Home(Box::new(home::Home::new(ctx, &shared))),
                },
                Launch::Gallery => Mode::Gallery(Box::new(ui::gallery::Gallery::new(
                    ctx, theme, ui_mode, lang_pref,
                ))),
                Launch::Notice => Mode::Notice(Box::new(notice::Notice::copied())),
                Launch::Home if resident_mode => {
                    let resident =
                        system::resident::Resident::start(ctx, &shared.settings, listener.take());
                    // Only Windows and macOS get here: the root is an invisible host.
                    let shell = screens::shell::Shell::new(true, !args.minimized);
                    Mode::Resident {
                        home: Some(Box::new(main_home(ctx, &shared, args.page))),
                        shell: Box::new(shell),
                        resident: Box::new(resident),
                    }
                }
                Launch::Home => Mode::Home(Box::new(main_home(ctx, &shared, args.page))),
            };
            // A window of its own answers the resident's Quit.
            let quit_request = Arc::new(std::sync::Mutex::new(None));
            let quit_close = Arc::new(AtomicBool::new(false));
            if screenshot.is_none()
                && matches!(mode, Mode::Editor(_) | Mode::Pin(_) | Mode::Overlay(_))
            {
                let base = system::instance::endpoint();
                let pid = std::process::id();
                let (slot, ask_ctx) = (quit_request.clone(), ctx.clone());
                let (close, close_ctx) = (quit_close.clone(), ctx.clone());
                let listening = system::instance::listen_for_quit(
                    &system::instance::child_endpoint(&base, pid),
                    move |reply| {
                        if let Ok(mut q) = slot.lock() {
                            *q = Some(reply);
                        }
                        ask_ctx.request_repaint();
                    },
                    move || {
                        close.store(true, Ordering::SeqCst);
                        close_ctx.request_repaint();
                    },
                );
                if listening.is_ok() {
                    std::thread::spawn(move || system::instance::register_child(&base, pid));
                }
            }
            Ok(Box::new(App {
                mode,
                shared,
                screenshot,
                frames: 0,
                shot_saved: Arc::new(AtomicBool::new(false)),
                shot_requested: false,
                editor_window,
                notice_area,
                quit_request,
                quit_close,
            }))
        }),
    );
    actions::wait_clipboard_keepers();
    system::notify::wait_pending();
    res
}

/// The window every Takeshot window starts from: its icon (in the theme's look) and its app id.
fn base_viewport(theme: ui::tokens::ThemeId) -> egui::ViewportBuilder {
    egui::ViewportBuilder::default()
        .with_icon(Arc::new(ui::brand::window_icon(theme)))
        .with_app_id("takeshot")
}

/// The main window: the theme's own bar, 1160 by 740.
fn main_window_viewport(vp: egui::ViewportBuilder) -> egui::ViewportBuilder {
    ui::chrome::frameless(vp)
        .with_title("Takeshot")
        .with_inner_size([1160.0, 740.0])
        .with_min_inner_size([900.0, 600.0])
        .with_drag_and_drop(true)
}

/// `--theme`, `--mode`, `--captures-dir` and `--select`: for this run only.
fn run_overrides(args: &Args) -> RunOverrides {
    RunOverrides {
        theme: args.theme,
        mode: args.mode,
        captures_dir: args.captures_dir.clone(),
        select: args.select.clone(),
    }
}

/// How long the resident sleeps with no window when nothing wakes it: it also refreshes the
/// recent captures in the tray menu (every 30 s).
const HEARTBEAT: std::time::Duration = std::time::Duration::from_secs(30);

/// The resident of Linux. A window cannot hide on Wayland, so the main window is made when it is
/// wanted and destroyed to hide it; between windows the tray, the hotkeys and the second launches
/// are served here, one egui pass per wake-up (`system::headless`). Returns when Quit is
/// confirmed, or when a window cannot be made.
fn run_resident_windows(
    args: &Args,
    settings: settings::Settings,
    listener: Option<system::instance::Listener>,
) -> eframe::Result {
    use system::headless::{self, Next, TrayWait};
    let idle = headless::Idle::default();
    let headless_ctx = headless::context(&idle);
    let shared = Shared::new(settings, run_overrides(args));
    let resident = system::resident::Resident::start(&headless_ctx, &shared.settings, listener);
    let shell = screens::shell::Shell::new(false, !args.minimized).with_page(args.page);
    let app = Rc::new(RefCell::new(App {
        mode: Mode::Resident {
            home: None,
            shell: Box::new(shell),
            resident: Box::new(resident),
        },
        shared,
        screenshot: None,
        frames: 0,
        shot_saved: Arc::new(AtomicBool::new(false)),
        shot_requested: false,
        editor_window: (egui::pos2(80.0, 60.0), egui::vec2(1280.0, 820.0)),
        notice_area: egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1600.0, 900.0)),
        quit_request: Arc::new(std::sync::Mutex::new(None)),
        quit_close: Arc::new(AtomicBool::new(false)),
    }));
    let since = std::time::Instant::now();
    // A hidden start waits for a panel to show the tray icon: without one it is unseen for good.
    let mut waiting_for_tray = args.minimized;
    loop {
        let step = {
            let a = app.borrow();
            headless::next(a.wants_window(), a.is_quitting())
        };
        match step {
            Next::Quit => return Ok(()),
            Next::Window => {
                let theme = {
                    let a = app.borrow();
                    args.theme.unwrap_or(a.shared.settings.ui_theme)
                };
                let options = eframe::NativeOptions {
                    viewport: main_window_viewport(base_viewport(theme)),
                    ..Default::default()
                };
                let window = app.clone();
                eframe::run_native(
                    "Takeshot",
                    options,
                    Box::new(move |cc| {
                        window.borrow_mut().attach(&cc.egui_ctx);
                        Ok(Box::new(Window(window)))
                    }),
                )?;
                app.borrow_mut().detach(&headless_ctx);
            }
            Next::Wait => {
                let now = since.elapsed();
                headless::pass(&headless_ctx, now.as_secs_f64(), |ctx| {
                    app.borrow_mut().pump(ctx)
                });
                if waiting_for_tray {
                    let shown = app.borrow().tray_shown();
                    match headless::tray_wait(now, shown) {
                        TrayWait::Keep => {}
                        TrayWait::Done => waiting_for_tray = false,
                        TrayWait::GiveUp => {
                            waiting_for_tray = false;
                            headless::pass(&headless_ctx, now.as_secs_f64(), |ctx| {
                                app.borrow_mut().show_main(ctx)
                            });
                        }
                    }
                }
                let a = app.borrow();
                if headless::next(a.wants_window(), a.is_quitting()) == Next::Wait {
                    idle.sleep(if waiting_for_tray {
                        headless::TRAY_POLL
                    } else {
                        HEARTBEAT
                    });
                }
            }
        }
    }
}

/// One of the resident's windows over the app that outlives it (Linux).
struct Window(Rc<RefCell<App>>);

impl eframe::App for Window {
    fn logic(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        self.0.borrow_mut().logic(ctx, frame);
    }

    fn clear_color(&self, visuals: &egui::Visuals) -> [f32; 4] {
        self.0.borrow().clear_color(visuals)
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        self.0.borrow_mut().ui(ui, frame);
    }
}

/// A few marks on the sample picture so the demo editor shows the tools.
fn demo_annotations() -> Vec<model::Ann> {
    use egui::pos2;
    use model::{Ann, Shape};
    vec![
        Ann {
            shape: Shape::Ellipse(pos2(560.0, 150.0), pos2(830.0, 300.0)),
            color: [255, 0, 0, 255],
            width: 4.0,
            filled: false,
        },
        Ann {
            shape: Shape::Arrow(pos2(980.0, 470.0), pos2(820.0, 360.0)),
            color: [255, 0, 0, 255],
            width: 4.0,
            filled: false,
        },
        Ann {
            shape: Shape::Counter {
                center: pos2(1000.0, 490.0),
                n: 1,
            },
            color: [255, 0, 0, 255],
            width: 4.0,
            filled: true,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use ui::tokens::{ThemeId, UiMode};

    fn shared(theme: ThemeId, mode: UiMode) -> Shared {
        Shared {
            settings: settings::Settings {
                ui_theme: theme,
                ui_mode: mode,
                ..Default::default()
            },
            settings_dirty: false,
            fonts: Arc::new(fonts::FontLib::load()),
            logo: widgets::LogoCache::new(),
            toast: None,
            overrides: RunOverrides::default(),
        }
    }

    #[test]
    fn overrides_win_for_this_run_only() {
        let mut s = shared(ThemeId::TakeUi, UiMode::Dark);
        s.overrides.theme = Some(ThemeId::Coral);
        let t = s.tokens(None);
        assert_eq!(t.theme, ThemeId::Coral);
        assert!(t.dark);
        s.overrides.mode = Some(UiMode::Light);
        assert!(!s.tokens(None).dark);
        assert_eq!(s.settings.ui_theme, ThemeId::TakeUi, "settings untouched");
        assert_eq!(s.settings.ui_mode, UiMode::Dark, "settings untouched");
    }

    #[test]
    fn system_mode_follows_the_os() {
        let s = shared(ThemeId::TakeUi, UiMode::System);
        assert!(!s.tokens(Some(egui::Theme::Light)).dark);
        assert!(s.tokens(Some(egui::Theme::Dark)).dark);
        assert!(s.tokens(None).dark, "unknown: dark");
    }

    #[test]
    fn page_opens_the_main_window_on_it() {
        use system::instance::Page;
        let ctx = egui::Context::default();
        let mut s = shared(ThemeId::TakeUi, UiMode::Dark);
        s.settings.save_dir = std::env::temp_dir()
            .join("takeshot-test-no-captures")
            .to_string_lossy()
            .into_owned();
        assert_eq!(main_home(&ctx, &s, Some(Page::Style)).page(), Page::Style);
        assert_eq!(
            main_home(&ctx, &s, Some(Page::Settings)).page(),
            Page::Settings
        );
        assert_eq!(main_home(&ctx, &s, None).page(), Page::Start);
    }

    #[test]
    fn pages_parse() {
        use system::instance::Page;
        assert!(matches!(
            parse_page("inicio"),
            Some(PageArg::Main(Page::Start))
        ));
        assert!(matches!(
            parse_page("estilo"),
            Some(PageArg::Main(Page::Style))
        ));
        assert!(matches!(
            parse_page("ajustes"),
            Some(PageArg::Main(Page::Settings))
        ));
        assert!(matches!(parse_page("componentes"), Some(PageArg::Gallery)));
        assert!(matches!(parse_page("editor"), Some(PageArg::Editor)));
        assert!(matches!(parse_page("captura"), Some(PageArg::Capture)));
        assert!(matches!(parse_page("aviso"), Some(PageArg::Notice)));
        assert!(parse_page("x").is_none());
    }
}
