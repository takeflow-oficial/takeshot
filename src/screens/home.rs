//! Main window state and behaviour, shared by both themes: the page on screen, the captures
//! library (filter, search, selection, thumbnails), what the user asked for (`MainAction`) and
//! carrying it out. `views` draws it.

use crate::Shared;
use crate::engine::render;
use crate::screens::overlay::color_image;
use crate::system::hotkeys::{Shot, Status, default_keys, keys_from_egui, label, needs_modifier};
use crate::system::library::{self, Entry, Period, Thumbs};
use crate::ui::i18n::{Msg, t as tr, tf};
use crate::ui::kit::{CardThumb, WindowControl};
use crate::ui::tokens::{ThemeId, Tokens, UiMode};
use egui::Ui;
use image::RgbaImage;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, channel};

#[derive(PartialEq, Clone, Copy)]
enum Page {
    Start,
    Style,
    Settings,
}

/// The sections of Settings.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum SettingsSection {
    #[default]
    General,
    Capture,
    Save,
    Shortcuts,
    About,
}

/// What the user asked for in the main window; `Home::apply` carries it out.
#[derive(Clone, PartialEq, Debug)]
pub enum MainAction {
    Page(crate::system::instance::Page),
    Capture(Shot),
    CaptureIn3s,
    OpenImage,
    OpenFolder,
    ChooseFolder,
    Filter(Period),
    Search(String),
    /// A picture pasted into the window (outside a text field): it opens in the editor.
    Paste,
    /// Style's "Restore default".
    RestoreStyle,
    /// Pick the watermark's logo file.
    ChooseLogo,
    Section(SettingsSection),
    SetTheme(ThemeId),
    SetLanguage(crate::ui::i18n::LangPref),
    SetAutostart(bool),
    /// Wait for the keys of a new shortcut.
    StartRebind(Shot),
    RestoreKeys(Shot),
    OpenWindowsSettings,
    /// Quit the resident (Linux, where the tray may be missing).
    Quit,
    ViewOnGitHub,
    Select(PathBuf),
    Copy(PathBuf),
    Edit(PathBuf),
    ShowInFolder(PathBuf),
    Pin(PathBuf),
    Trash(PathBuf),
    SetMode(UiMode),
    /// Início as a grid or a list.
    SetHomeView(crate::system::settings::HomeView),
    /// Show or hide Início's detail panel.
    SetHomeDetails(bool),
    About,
    Window(WindowControl),
}

/// A capture's thumbnail in the cache.
enum Thumbnail {
    Pending,
    Ready {
        tex: egui::TextureHandle,
        size: [u32; 2],
        used: u64,
        /// The capture's date and size it was made from: a capture saved over under the same
        /// name gets a new one.
        made_from: (std::time::SystemTime, u64),
    },
    Failed,
}

/// Thumbnails kept as textures; the least recently drawn go first.
const THUMB_CACHE: usize = 300;
/// How often the folder is read again while Home is on screen.
const RESCAN_SECONDS: f64 = 5.0;

pub struct Home {
    page: Page,
    section: SettingsSection,
    /// Whether the app starts with the system, read when Settings opens and after a change.
    autostart: bool,
    entries: Vec<Entry>,
    /// The positions in `entries` the filter and the search let through, and the filters' counts,
    /// worked out when the list, the filter or the search changes.
    shown: Vec<usize>,
    counts: [usize; 3],
    filter: Period,
    query: String,
    selected: Option<PathBuf>,
    /// The keys moved the choice: the gallery brings it into view.
    reveal: bool,
    /// A capture to choose by name once the folder is read (`--select`).
    pending_select: Option<String>,
    /// Cards in a gallery row (1 in the list), for the up and down keys.
    cols: usize,
    folder_missing: bool,
    thumbs: Thumbs,
    /// The thumbnails' disk cache (Delete drops a capture's thumbnail from it).
    cache: Option<crate::system::thumbcache::Cache>,
    textures: HashMap<PathBuf, Thumbnail>,
    /// The cards drawn in the last frame; a waiting thumbnail not among them is dropped.
    drawn: std::collections::HashSet<PathBuf>,
    frame: u64,
    scan_rx: Option<Receiver<(bool, Vec<Entry>)>>,
    last_scan: f64,
    mark: Option<((ThemeId, bool), egui::TextureHandle)>,
    sample: RgbaImage,
    preview: Option<egui::TextureHandle>,
    /// What the preview was drawn from (style and watermark), to draw it again only on a change.
    preview_key: Option<String>,
    #[cfg(test)]
    renders: usize,
    /// The capture shortcuts' state, given by the resident each frame.
    pub hotkey_status: Option<Status>,
    /// What the user asked of the resident this frame (About, Quit, a new shortcut…).
    pub requests: HomeRequests,
    /// Waiting for the keys of a new shortcut.
    capturing: Option<Shot>,
}

/// Requests from Home that the resident carries out.
#[derive(Default)]
pub struct HomeRequests {
    pub about: bool,
    pub quit: bool,
    pub rebind: Option<(Shot, String)>,
    pub open_windows_settings: bool,
}

impl Home {
    pub fn new(ctx: &egui::Context, shared: &Shared) -> Self {
        let mut h = Self::empty(ctx, crate::system::thumbcache::Cache::system());
        h.scan(ctx, shared.captures_dir(), 0.0);
        h
    }

    /// An empty Início whose thumbnails go through the disk cache at `cache`.
    fn empty(ctx: &egui::Context, cache: Option<crate::system::thumbcache::Cache>) -> Self {
        Self {
            page: Page::Start,
            section: SettingsSection::General,
            autostart: false,
            entries: Vec::new(),
            shown: Vec::new(),
            counts: [0; 3],
            filter: Period::All,
            query: String::new(),
            selected: None,
            reveal: false,
            pending_select: None,
            cols: 1,
            folder_missing: false,
            thumbs: Thumbs::new(ctx, cache.clone()),
            cache,
            textures: HashMap::new(),
            drawn: std::collections::HashSet::new(),
            frame: 0,
            scan_rx: None,
            last_scan: -100.0,
            mark: None,
            sample: crate::system::capture::sample_window(1000, 620),
            preview: None,
            preview_key: None,
            #[cfg(test)]
            renders: 0,
            hotkey_status: None,
            requests: HomeRequests::default(),
            capturing: None,
        }
    }

    /// Home over these entries, never reading a folder, with a `Shared` for tests.
    #[cfg(test)]
    pub fn for_tests(entries: Vec<Entry>) -> (Self, Shared) {
        let mut h = Self::empty(&egui::Context::default(), None);
        h.last_scan = f64::INFINITY;
        h.set_entries(entries);
        (h, Shared::for_tests())
    }

    /// Shows a page of the main window (About is a dialog the resident opens).
    pub fn open(&mut self, page: crate::system::instance::Page) {
        use crate::system::instance::Page as P;
        // A shortcut waiting for its keys only waits while its row is on screen.
        self.capturing = None;
        match page {
            P::Start => self.page = Page::Start,
            P::Style => self.page = Page::Style,
            P::Settings => {
                self.page = Page::Settings;
                self.autostart = crate::system::autostart::autostart_enabled();
            }
            P::About => self.requests.about = true,
        }
    }

    pub fn section(&self) -> SettingsSection {
        self.section
    }

    /// The shortcut waiting for its new keys, if any.
    pub fn capturing(&self) -> Option<Shot> {
        self.capturing
    }

    pub fn autostart(&self) -> bool {
        self.autostart
    }

    /// The page on screen (About is a dialog, never a page).
    pub fn page(&self) -> crate::system::instance::Page {
        use crate::system::instance::Page as P;
        match self.page {
            Page::Start => P::Start,
            Page::Style => P::Style,
            Page::Settings => P::Settings,
        }
    }

    pub fn filter(&self) -> Period {
        self.filter
    }

    pub fn query(&self) -> &str {
        &self.query
    }

    /// How many captures each filter shows: all, today, this week.
    pub fn counts(&self) -> [usize; 3] {
        self.counts
    }

    /// The captures the filter and the search let through, newest first (indices into the
    /// entries).
    pub fn shown(&self) -> &[usize] {
        &self.shown
    }

    pub fn entry(&self, i: usize) -> &Entry {
        &self.entries[i]
    }

    pub fn selected(&self) -> Option<&Path> {
        self.selected.as_deref()
    }

    /// Chooses the capture named `name` now, or when the folder's listing brings it.
    pub fn select_by_name(&mut self, name: &str) {
        self.pending_select = Some(name.to_string());
        self.take_pending_select();
    }

    fn take_pending_select(&mut self) {
        let Some(name) = &self.pending_select else {
            return;
        };
        if let Some(e) = self.entries.iter().find(|e| e.name == *name) {
            self.selected = Some(e.path.clone());
            self.pending_select = None;
        }
    }

    /// The chosen capture, when it is in the list.
    pub fn selected_entry(&self) -> Option<Entry> {
        let s = self.selected.as_deref()?;
        self.entries.iter().find(|e| e.path == s).cloned()
    }

    /// The gallery's columns, for the up and down keys.
    pub fn set_columns(&mut self, cols: usize) {
        self.cols = cols.max(1);
    }

    /// Whether the keys moved the choice since the last call: the gallery scrolls to it.
    pub fn take_reveal(&mut self) -> bool {
        std::mem::take(&mut self.reveal)
    }

    pub fn folder_missing(&self) -> bool {
        self.folder_missing
    }

    pub fn has_captures(&self) -> bool {
        !self.entries.is_empty()
    }

    /// A card's picture and the capture's size in pixels; the first call asks for the thumbnail.
    pub fn card_thumb(&mut self, path: &Path) -> (CardThumb, Option<[u32; 2]>) {
        let frame = self.frame;
        self.drawn.insert(path.to_path_buf());
        match self.textures.get_mut(path) {
            Some(Thumbnail::Ready {
                tex, size, used, ..
            }) => {
                *used = frame;
                (CardThumb::Ready(tex.id(), tex.size_vec2()), Some(*size))
            }
            Some(Thumbnail::Pending) => (CardThumb::Loading, None),
            Some(Thumbnail::Failed) => (CardThumb::Failed, None),
            None => {
                self.textures.insert(path.to_path_buf(), Thumbnail::Pending);
                self.thumbs.request(path.to_path_buf());
                (CardThumb::Loading, None)
            }
        }
    }

    /// How many thumbnails were asked for (tests).
    #[cfg(test)]
    pub fn thumb_requests(&self) -> usize {
        self.textures.len()
    }

    /// The gallery thumbnail of a capture, when it is loaded (the tray menu shows it).
    pub fn thumbnail(&self, path: &Path) -> Option<egui::TextureId> {
        match self.textures.get(path) {
            Some(Thumbnail::Ready { tex, .. }) => Some(tex.id()),
            _ => None,
        }
    }

    /// The theme's mark for the title bar, drawn once per look.
    pub fn mark(&mut self, ctx: &egui::Context, t: &Tokens) -> egui::TextureId {
        let look = (t.theme, t.dark);
        match &self.mark {
            Some((l, tex)) if *l == look => tex.id(),
            _ => {
                let tex = crate::ui::brand::mark_texture(ctx, t.theme, t.dark);
                let id = tex.id();
                self.mark = Some((look, tex));
                id
            }
        }
    }

    /// The live style preview: the sample picture through the current style and watermark, drawn
    /// again only when one of them (or the logo) changed.
    pub fn style_preview(
        &mut self,
        ctx: &egui::Context,
        shared: &Shared,
    ) -> Option<(egui::TextureId, egui::Vec2)> {
        let s = &shared.settings;
        let key = serde_json::to_string(&(&s.style, &s.watermark, shared.logo.image.is_some()))
            .unwrap_or_default();
        if self.preview.is_none() || self.preview_key.as_deref() != Some(key.as_str()) {
            let (img, _) = render::compose(
                &self.sample,
                &s.style,
                &s.watermark,
                shared.logo.image.as_ref(),
                &shared.fonts,
                1.0,
            );
            let small = image::imageops::resize(
                &img,
                (img.width() / 2).max(1),
                (img.height() / 2).max(1),
                image::imageops::FilterType::Triangle,
            );
            match &mut self.preview {
                Some(t) => t.set(color_image(&small), egui::TextureOptions::LINEAR),
                None => {
                    self.preview = Some(ctx.load_texture(
                        "style-preview",
                        color_image(&small),
                        egui::TextureOptions::LINEAR,
                    ))
                }
            }
            self.preview_key = Some(key);
            #[cfg(test)]
            {
                self.renders += 1;
            }
        }
        self.preview.as_ref().map(|t| (t.id(), t.size_vec2()))
    }

    #[cfg(test)]
    pub fn preview_renders(&self) -> usize {
        self.renders
    }

    fn set_entries(&mut self, entries: Vec<Entry>) {
        self.entries = entries;
        let present: std::collections::HashSet<&Path> =
            self.entries.iter().map(|e| e.path.as_path()).collect();
        let now: HashMap<&Path, (std::time::SystemTime, u64)> = self
            .entries
            .iter()
            .map(|e| (e.path.as_path(), (e.modified, e.bytes)))
            .collect();
        self.textures
            .retain(|p, t| match (now.get(p.as_path()), t) {
                (None, _) => false,
                (Some(stamp), Thumbnail::Ready { made_from, .. }) => made_from == stamp,
                _ => true,
            });
        if self
            .selected
            .as_deref()
            .is_some_and(|s| !present.contains(s))
        {
            self.selected = None;
        }
        self.take_pending_select();
        self.refilter();
    }

    fn refilter(&mut self) {
        let now = chrono::Local::now();
        self.shown = library::visible(&self.entries, self.filter, &self.query, now);
        self.counts = library::counts(&self.entries, now);
    }

    fn scan(&mut self, ctx: &egui::Context, dir: &str, now: f64) {
        self.last_scan = now;
        let dir = PathBuf::from(dir);
        let (tx, rx) = channel();
        self.scan_rx = Some(rx);
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send((dir.is_dir(), library::scan(&dir)));
            ctx.request_repaint();
        });
    }

    /// Drops the least recently drawn thumbnails past the cache size; they are asked again when
    /// they come back on screen.
    fn evict(&mut self) {
        let ready = |t: &Thumbnail| matches!(t, Thumbnail::Ready { .. });
        let n = self.textures.values().filter(|t| ready(t)).count();
        if n <= THUMB_CACHE {
            return;
        }
        let mut used: Vec<(u64, PathBuf)> = self
            .textures
            .iter()
            .filter_map(|(p, t)| match t {
                Thumbnail::Ready { used, .. } => Some((*used, p.clone())),
                _ => None,
            })
            .collect();
        used.sort();
        for (_, p) in used.into_iter().take(n - THUMB_CACHE) {
            self.textures.remove(&p);
        }
    }

    /// Once per frame, before the view draws: the folder's new listing, finished thumbnails,
    /// dropped files, pasted pictures and Ctrl+O.
    pub fn begin(&mut self, ui: &Ui, shared: &mut Shared) -> Vec<MainAction> {
        let ctx = ui.ctx().clone();
        self.frame += 1;
        let now = ui.input(|i| i.time);
        if let Some(rx) = &self.scan_rx
            && let Ok((exists, list)) = rx.try_recv()
        {
            self.scan_rx = None;
            self.folder_missing = !exists;
            self.set_entries(list);
        }
        if self.page == Page::Start {
            if self.scan_rx.is_none() && now - self.last_scan > RESCAN_SECONDS {
                self.scan(&ctx, shared.captures_dir(), now);
            }
            // The folder is read again while Home is on screen, input or not.
            ctx.request_repaint_after(std::time::Duration::from_secs_f64(RESCAN_SECONDS));
        }
        // Thumbnails still waiting for cards that left the screen are not made; they are asked
        // again when the cards come back.
        self.thumbs.keep(&self.drawn);
        let drawn = std::mem::take(&mut self.drawn);
        self.textures
            .retain(|p, t| !matches!(t, Thumbnail::Pending) || drawn.contains(p));
        for (path, result) in self.thumbs.poll() {
            if !self.textures.contains_key(&path) {
                continue;
            }
            let made_from = self
                .entries
                .iter()
                .find(|e| e.path == path)
                .map_or((std::time::UNIX_EPOCH, 0), |e| (e.modified, e.bytes));
            let thumb = match result {
                Ok((img, size)) => Thumbnail::Ready {
                    tex: ctx.load_texture(
                        format!("thumb-{}", path.display()),
                        color_image(&img),
                        egui::TextureOptions::LINEAR,
                    ),
                    size,
                    used: self.frame,
                    made_from,
                },
                Err(_) => Thumbnail::Failed,
            };
            self.textures.insert(path, thumb);
        }
        self.evict();
        shared.logo.sync(&ctx, &shared.settings.watermark);

        let mut acts = Vec::new();
        // "Change" waits for the next keys: Esc cancels; a key needing Ctrl or Alt keeps waiting.
        if let Some(shot) = self.capturing {
            let pressed = ui.input(|i| {
                i.events.iter().find_map(|e| match e {
                    egui::Event::Key {
                        key,
                        pressed: true,
                        modifiers,
                        ..
                    } => Some((*key, *modifiers)),
                    _ => None,
                })
            });
            if let Some((key, mods)) = pressed
                && (key == egui::Key::Escape || !needs_modifier(key, mods))
            {
                self.capturing = None;
                if let Some(keys) = keys_from_egui(key, mods) {
                    self.requests.rebind = Some((shot, keys));
                }
            }
            return acts;
        }
        let dropped: Vec<PathBuf> = ui.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .map(|f| f.path().to_path_buf())
                .filter(|p| !p.as_os_str().is_empty())
                .collect()
        });
        if let Some(p) = dropped.into_iter().next() {
            acts.push(MainAction::Edit(p));
        }
        // A text field keeps its own paste and shortcuts.
        if !ctx.text_edit_focused() {
            if ui.input(|i| {
                i.events
                    .iter()
                    .any(|e| matches!(e, egui::Event::Paste(_) | egui::Event::PasteImage(_)))
            }) {
                acts.push(MainAction::Paste);
            }
            if ui.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::O)) {
                acts.push(MainAction::OpenImage);
            }
            // A menu or a dialog over the gallery takes its keys.
            let covered =
                egui::Popup::is_any_open(&ctx) || ctx.memory(|m| m.top_modal_layer().is_some());
            if self.page == Page::Start && !covered {
                self.gallery_keys(ui, &mut acts);
            }
        }
        acts
    }

    /// The gallery's keys: the arrows move the choice (up and down by a row), Enter opens it in
    /// the editor, Ctrl+C copies it and Delete sends it to the trash.
    fn gallery_keys(&mut self, ui: &Ui, acts: &mut Vec<MainAction>) {
        use egui::Key;
        let step = ui.input(|i| {
            let cols = self.cols as isize;
            [
                (Key::ArrowLeft, -1),
                (Key::ArrowRight, 1),
                (Key::ArrowUp, -cols),
                (Key::ArrowDown, cols),
            ]
            .into_iter()
            .find(|(k, _)| i.key_pressed(*k))
            .map(|(_, d)| d)
        });
        if let Some(d) = step
            && !self.shown.is_empty()
        {
            let at = self
                .selected
                .as_deref()
                .and_then(|s| self.shown.iter().position(|&i| self.entries[i].path == s));
            let next = match at {
                Some(p) => (p as isize + d).clamp(0, self.shown.len() as isize - 1) as usize,
                None => 0,
            };
            self.selected = Some(self.entries[self.shown[next]].path.clone());
            self.reveal = true;
        }
        let Some(path) = self.selected.clone() else {
            return;
        };
        if ui.input(|i| i.key_pressed(Key::Enter)) {
            acts.push(MainAction::Edit(path.clone()));
        }
        if ui.input_mut(|i| crate::engine::annotator::consume_command(i, Key::C)) {
            acts.push(MainAction::Copy(path.clone()));
        }
        if ui.input(|i| i.key_pressed(Key::Delete)) {
            acts.push(MainAction::Trash(path));
        }
    }

    /// Carries out what the user asked for. Resident matters (About) go to `requests`.
    pub fn apply(&mut self, a: MainAction, ctx: &egui::Context, shared: &mut Shared) {
        use crate::system::actions;
        match a {
            MainAction::Page(p) => self.open(p),
            MainAction::Capture(Shot::Area) => self.capture(ctx, shared, &["--capture", "--wait"]),
            MainAction::Capture(Shot::Full) => {
                self.capture(ctx, shared, &["--capture", "--full", "--wait"])
            }
            MainAction::CaptureIn3s => self.capture(ctx, shared, &["--capture", "--delay", "3"]),
            MainAction::OpenImage => {
                if let Some(p) = rfd::FileDialog::new()
                    .add_filter(tr(Msg::OpenImage), &["png", "jpg", "jpeg", "webp", "bmp"])
                    .pick_file()
                {
                    self.apply(MainAction::Edit(p), ctx, shared);
                }
            }
            MainAction::OpenFolder => actions::open_folder(Path::new(shared.captures_dir())),
            MainAction::Paste => open_clipboard(shared),
            MainAction::ChooseFolder => {
                if let Some(d) = rfd::FileDialog::new().pick_folder() {
                    shared.settings.save_dir = d.to_string_lossy().into_owned();
                    shared.settings_dirty = true;
                    self.last_scan = f64::NEG_INFINITY;
                }
            }
            MainAction::Filter(p) => {
                self.filter = p;
                self.refilter();
            }
            MainAction::Search(q) => {
                self.query = q;
                self.refilter();
            }
            MainAction::Select(p) => self.selected = Some(p),
            MainAction::SetHomeView(v) => {
                shared.settings.home_view = v;
                shared.settings_dirty = true;
            }
            MainAction::SetHomeDetails(on) => {
                shared.settings.home_details = on;
                shared.settings_dirty = true;
            }
            MainAction::Section(s) => {
                self.section = s;
                self.capturing = None;
                if s == SettingsSection::General {
                    self.autostart = crate::system::autostart::autostart_enabled();
                }
            }
            MainAction::SetTheme(th) => {
                shared.settings.ui_theme = th;
                shared.settings_dirty = true;
            }
            MainAction::SetLanguage(l) => {
                shared.settings.lang = l;
                crate::ui::i18n::apply_language(l);
                shared.settings_dirty = true;
            }
            MainAction::SetAutostart(on) => {
                if let Err(e) = crate::system::autostart::set_autostart(on) {
                    shared.toast(e);
                }
                self.autostart = crate::system::autostart::autostart_enabled();
            }
            MainAction::StartRebind(shot) => self.capturing = Some(shot),
            MainAction::RestoreKeys(shot) => {
                self.requests.rebind = Some((shot, default_keys(shot).to_string()));
            }
            MainAction::OpenWindowsSettings => self.requests.open_windows_settings = true,
            MainAction::Quit => self.requests.quit = true,
            MainAction::ViewOnGitHub => {
                ctx.open_url(egui::OpenUrl::new_tab(
                    crate::screens::about::AboutInfo::current().github,
                ));
            }
            MainAction::RestoreStyle => {
                crate::system::settings::restore_default_style(&mut shared.settings);
                shared.settings_dirty = true;
            }
            MainAction::ChooseLogo => {
                if let Some(p) = rfd::FileDialog::new()
                    .add_filter("Logo", &["png", "jpg", "jpeg", "webp"])
                    .pick_file()
                {
                    shared.settings.watermark.logo_path = Some(p.to_string_lossy().into_owned());
                    shared.settings_dirty = true;
                }
            }
            MainAction::Copy(p) => match image::open(&p) {
                Ok(img) => match actions::copy_image(&img.to_rgba8()) {
                    Ok(()) => shared.toast(tr(Msg::Copied).to_string()),
                    Err(e) => shared.toast(e),
                },
                Err(e) => shared.toast(e.to_string()),
            },
            MainAction::Edit(p) => {
                if let Err(e) = actions::spawn_self(&["--edit", &p.to_string_lossy()]) {
                    shared.toast(e);
                }
            }
            MainAction::ShowInFolder(p) => actions::show_in_folder(&p),
            // `--pin` deletes the file it gets, so it gets a copy.
            MainAction::Pin(p) => match image::open(&p) {
                Ok(img) => match actions::temp_png(&img.to_rgba8()) {
                    Some(tmp) => {
                        if let Err(e) = actions::spawn_self(&["--pin", &tmp.to_string_lossy()]) {
                            shared.toast(e);
                        }
                    }
                    None => shared.toast(tf(Msg::CannotPin, &[&p.to_string_lossy()])),
                },
                Err(e) => shared.toast(e.to_string()),
            },
            MainAction::Trash(p) => {
                // Its thumbnail goes too: a small capture's thumbnail is the whole picture.
                if let Some(c) = &self.cache {
                    c.forget(&p);
                }
                match library::move_to_trash(&p) {
                    Ok(()) => {
                        shared.toast(tr(Msg::MovedToTrash).to_string());
                        let rest = self
                            .entries
                            .iter()
                            .filter(|e| e.path != p)
                            .cloned()
                            .collect();
                        self.set_entries(rest);
                    }
                    Err(e) => shared.toast(e),
                }
            }
            MainAction::SetMode(m) => {
                shared.settings.ui_mode = m;
                shared.settings_dirty = true;
            }
            MainAction::About => self.requests.about = true,
            MainAction::Window(c) => crate::ui::chrome::act(ctx, c),
        }
    }

    fn capture(&mut self, ctx: &egui::Context, shared: &mut Shared, args: &[&str]) {
        // The new capture shows as soon as the window is back.
        self.last_scan = f64::NEG_INFINITY;
        ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
        if let Err(e) = crate::system::actions::spawn_self(args) {
            shared.toast(e);
        }
    }
}

fn open_clipboard(shared: &mut Shared) {
    match crate::system::actions::clipboard_image() {
        Some(img) => {
            if let Some(p) = crate::system::actions::temp_png(&img) {
                let _ = crate::system::actions::spawn_self(&["--edit", &p.to_string_lossy()]);
            }
        }
        None => shared.toast(tr(Msg::NoImageToPaste).into()),
    }
}

/// The status line under the navigation: which keys capture, or why they do not.
/// The status chip's state and short text: the area shortcut ready or in use, or the shortcuts
/// off or unavailable (never "in use" for those).
/// A gallery group's title: Today, Yesterday, or the date ("5 de novembro", "November 5"), with
/// the year when it is not this one.
pub(crate) fn day_label(day: library::Day, now: chrono::DateTime<chrono::Local>) -> String {
    day_text(day, now, Msg::Months, Msg::DayMonth, Msg::DayMonthYear)
}

/// A capture's day in a few letters beside it, as mail lists do: the time today ("14:32"),
/// Yesterday, else the date ("5 nov", "Nov 5"), with the year when it is not this one.
pub(crate) fn when_label(
    at: std::time::SystemTime,
    now: chrono::DateTime<chrono::Local>,
) -> String {
    match library::day_of(at, now) {
        library::Day::Today => chrono::DateTime::<chrono::Local>::from(at)
            .format("%H:%M")
            .to_string(),
        day => day_text(
            day,
            now,
            Msg::MonthsShort,
            Msg::ShortDayMonth,
            Msg::ShortDayMonthYear,
        ),
    }
}

/// Today, Yesterday, or the date in `months` ("|"-separated) by `this_year` or `other_year`.
fn day_text(
    day: library::Day,
    now: chrono::DateTime<chrono::Local>,
    months: Msg,
    this_year: Msg,
    other_year: Msg,
) -> String {
    use chrono::Datelike;
    match day {
        library::Day::Today => tr(Msg::Today).to_string(),
        library::Day::Yesterday => tr(Msg::Yesterday).to_string(),
        library::Day::Date(d) => {
            let month = tr(months)
                .split('|')
                .nth(d.month0() as usize)
                .unwrap_or_default();
            let day = d.day().to_string();
            if d.year() == now.year() {
                tf(this_year, &[&day, month])
            } else {
                tf(other_year, &[&day, month, &d.year().to_string()])
            }
        }
    }
}

pub(crate) fn chip(
    status: Option<&Status>,
    settings: &crate::system::settings::Settings,
) -> (crate::ui::kit::ChipState, String) {
    use crate::ui::kit::ChipState;
    match status {
        Some(Status::Ready) | None if settings.hotkeys => (
            ChipState::Ready,
            tf(Msg::ShortcutReady, &[&label(&settings.hotkey_area)]),
        ),
        Some(Status::Busy { keys }) => {
            (ChipState::Busy, tf(Msg::ShortcutBusy, &[&keys.join(" + ")]))
        }
        Some(Status::Unavailable(_)) => (ChipState::Off, tr(Msg::ShortcutsUnavailable).to_string()),
        Some(Status::Ready) | Some(Status::Off) | None => {
            (ChipState::Off, tr(Msg::ShortcutsOff).to_string())
        }
    }
}

/// Carries out a shortcut change's result: the new keys go to the settings, or the notice for the
/// user comes back (the system's own text only reaches the log). `hook_on`: Takeshot's PrtSc
/// priority is running, so it is not offered again.
pub(crate) fn apply_rebind(
    settings: &mut crate::system::settings::Settings,
    shot: crate::system::hotkeys::Shot,
    keys: &str,
    result: Result<(), crate::system::hotkeys::RebindError>,
    hook_on: bool,
) -> Option<String> {
    use crate::system::hotkeys::{RebindError, Shot, label};
    match result {
        Ok(()) => {
            match shot {
                Shot::Area => settings.hotkey_area = keys.to_string(),
                Shot::Full => settings.hotkey_full = keys.to_string(),
            }
            None
        }
        Err(e) => {
            if let RebindError::Refused(why) = &e {
                eprintln!("takeshot: shortcut {keys}: {why}");
            }
            let shown = label(keys);
            let priority =
                cfg!(windows) && !hook_on && matches!(shown.as_str(), "PrtSc" | "Shift+PrtSc");
            Some(rebind_notice(&shown, &e, priority))
        }
    }
}

/// The notice for a shortcut that could not be set, in the user's language. `priority` offers
/// Takeshot's PrtSc priority (Windows, PrtSc alone).
pub(crate) fn rebind_notice(
    keys: &str,
    e: &crate::system::hotkeys::RebindError,
    priority: bool,
) -> String {
    use crate::system::hotkeys::RebindError as E;
    match e {
        E::InUse if priority => tf(Msg::KeyInUsePriority, &[keys]),
        E::InUse => tf(Msg::KeyInUse, &[keys]),
        E::Invalid => tf(Msg::KeyInvalid, &[keys]),
        E::Refused(_) => tf(Msg::KeyRefused, &[keys]),
    }
}

pub(crate) fn hotkey_text(
    status: Option<&Status>,
    settings: &crate::system::settings::Settings,
) -> Result<String, String> {
    let ready = || {
        Ok(tf(
            Msg::HotkeysReady,
            &[&label(&settings.hotkey_area), &label(&settings.hotkey_full)],
        ))
    };
    match status {
        None if settings.hotkeys => ready(),
        None | Some(Status::Off) => Err(tr(Msg::HotkeysOff).to_string()),
        Some(Status::Ready) => ready(),
        Some(Status::Busy { keys }) => Err(tf(Msg::HotkeysBusy, &[&keys.join(" + ")])),
        Some(Status::Unavailable(e)) => Err(tf(Msg::HotkeysUnavailable, &[e])),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_capture_s_day_is_short_beside_it() {
        use crate::ui::i18n::{Lang, set_lang};
        use chrono::TimeZone;
        let now = chrono::Local
            .with_ymd_and_hms(2026, 11, 8, 15, 0, 0)
            .unwrap();
        let at = |y, m, d, h, min| {
            std::time::SystemTime::from(chrono::Local.with_ymd_and_hms(y, m, d, h, min, 0).unwrap())
        };
        set_lang(Lang::Pt);
        assert_eq!(when_label(at(2026, 11, 8, 9, 5), now), "09:05");
        assert_eq!(when_label(at(2026, 11, 7, 22, 0), now), "Ontem");
        assert_eq!(when_label(at(2026, 11, 5, 10, 0), now), "5 nov");
        assert_eq!(when_label(at(2025, 12, 20, 10, 0), now), "20 dez 2025");
        set_lang(Lang::En);
        assert_eq!(when_label(at(2026, 11, 7, 22, 0), now), "Yesterday");
        assert_eq!(when_label(at(2026, 11, 5, 10, 0), now), "Nov 5");
        assert_eq!(when_label(at(2025, 12, 20, 10, 0), now), "Dec 20, 2025");
    }

    #[test]
    fn a_day_reads_in_the_user_s_language() {
        use crate::system::library::Day;
        use crate::ui::i18n::{Lang, set_lang};
        use chrono::{NaiveDate, TimeZone};
        let now = chrono::Local
            .with_ymd_and_hms(2026, 11, 8, 15, 0, 0)
            .unwrap();
        let date = |y, m, d| Day::Date(NaiveDate::from_ymd_opt(y, m, d).unwrap());
        set_lang(Lang::Pt);
        assert_eq!(day_label(Day::Today, now), "Hoje");
        assert_eq!(day_label(Day::Yesterday, now), "Ontem");
        assert_eq!(day_label(date(2026, 11, 5), now), "5 de novembro");
        assert_eq!(day_label(date(2025, 12, 20), now), "20 de dezembro de 2025");
        set_lang(Lang::En);
        assert_eq!(day_label(date(2026, 11, 5), now), "November 5");
        assert_eq!(day_label(date(2025, 12, 20), now), "December 20, 2025");
    }

    #[test]
    fn a_rebind_saves_the_keys_or_explains_in_the_user_s_words() {
        use crate::system::hotkeys::{RebindError, Shot};
        use crate::ui::i18n::{Lang, set_lang};
        set_lang(Lang::Pt);
        let mut s = crate::system::settings::Settings::default();
        assert_eq!(apply_rebind(&mut s, Shot::Area, "F9", Ok(()), false), None);
        assert_eq!(s.hotkey_area, "F9");
        let before = s.hotkey_full.clone();
        let refused = RebindError::Refused("HotKey { mods: Modifiers(0x0) }".into());
        let n = apply_rebind(&mut s, Shot::Full, "shift+PrintScreen", Err(refused), false).unwrap();
        assert!(n.starts_with("Shift+PrtSc") && !n.contains("HotKey"), "{n}");
        assert_eq!(s.hotkey_full, before, "a failed rebind keeps the old keys");
        let off = apply_rebind(
            &mut s,
            Shot::Full,
            "shift+PrintScreen",
            Err(RebindError::InUse),
            false,
        )
        .unwrap();
        let on = apply_rebind(
            &mut s,
            Shot::Full,
            "shift+PrintScreen",
            Err(RebindError::InUse),
            true,
        )
        .unwrap();
        assert_eq!(off.contains("prioridade"), cfg!(windows), "{off}");
        assert!(!on.contains("prioridade"), "already on: {on}");
        set_lang(Lang::En);
    }

    #[test]
    fn a_capture_chosen_by_name_waits_for_the_folder() {
        use std::time::SystemTime;
        let e = |n: &str| Entry {
            path: PathBuf::from(n),
            name: n.into(),
            modified: SystemTime::now(),
            bytes: 1,
        };
        let (mut home, _) = Home::for_tests(vec![]);
        home.select_by_name("b.png");
        assert_eq!(home.selected(), None, "not read yet");
        home.set_entries(vec![e("a.png"), e("b.png")]);
        assert_eq!(home.selected(), Some(Path::new("b.png")));
    }

    #[test]
    fn the_rebind_notice_is_translated() {
        use crate::system::hotkeys::RebindError;
        use crate::ui::i18n::{Lang, set_lang};
        set_lang(Lang::Pt);
        assert_eq!(
            rebind_notice("PrtSc", &RebindError::InUse, true),
            "PrtSc está em uso por outro app. Feche o outro app ou ligue a prioridade do Takeshot em Ajustes › Atalhos."
        );
        assert_eq!(
            rebind_notice("F8", &RebindError::InUse, false),
            "F8 está em uso por outro app. Feche o outro app ou escolha outra tecla."
        );
        set_lang(Lang::En);
        assert_eq!(
            rebind_notice("PrtSc", &RebindError::InUse, true),
            "PrtSc is in use by another app. Close that app, or turn on Takeshot priority in Settings › Shortcuts."
        );
        for e in [
            RebindError::InUse,
            RebindError::Invalid,
            RebindError::Refused("HotKey { mods: Modifiers(0x0) }".into()),
        ] {
            let n = rebind_notice("Ctrl+Q", &e, false);
            assert!(n.starts_with("Ctrl+Q"), "{n}");
            assert!(!n.contains("HotKey"), "no library text: {n}");
        }
    }

    #[test]
    fn the_status_chip_tells_off_and_unavailable_from_busy() {
        use crate::ui::kit::ChipState;
        let s = crate::system::settings::Settings::default();
        let area = label(&s.hotkey_area);
        assert_eq!(
            chip(Some(&Status::Ready), &s),
            (ChipState::Ready, tf(Msg::ShortcutReady, &[&area]))
        );
        let busy = Status::Busy {
            keys: vec!["PrtSc".into()],
        };
        assert_eq!(
            chip(Some(&busy), &s),
            (ChipState::Busy, tf(Msg::ShortcutBusy, &["PrtSc"]))
        );
        assert_eq!(
            chip(Some(&Status::Off), &s),
            (ChipState::Off, tr(Msg::ShortcutsOff).to_string())
        );
        assert_eq!(
            chip(Some(&Status::Unavailable("no display".into())), &s),
            (ChipState::Off, tr(Msg::ShortcutsUnavailable).to_string())
        );
    }

    #[test]
    fn leaving_shortcuts_stops_waiting_for_keys() {
        // Otherwise the next F-key pressed on any page would become the shortcut.
        let ctx = egui::Context::default();
        use crate::system::instance::Page as P;
        let (mut home, mut shared) = Home::for_tests(vec![]);
        home.open(P::Settings);
        home.apply(MainAction::StartRebind(Shot::Area), &ctx, &mut shared);
        assert_eq!(home.capturing(), Some(Shot::Area));
        home.apply(
            MainAction::Section(SettingsSection::General),
            &ctx,
            &mut shared,
        );
        assert_eq!(home.capturing(), None, "another section");
        home.apply(MainAction::StartRebind(Shot::Full), &ctx, &mut shared);
        home.apply(MainAction::Page(P::Start), &ctx, &mut shared);
        assert_eq!(home.capturing(), None, "another page");
    }

    #[test]
    fn a_capture_changed_under_its_name_gets_a_new_thumbnail() {
        let t1 = std::time::SystemTime::now() - std::time::Duration::from_secs(60);
        let shot = |modified, bytes| Entry {
            path: PathBuf::from("a.png"),
            name: "a.png".into(),
            modified,
            bytes,
        };
        let (mut home, _shared) = Home::for_tests(vec![shot(t1, 100)]);
        let ctx = egui::Context::default();
        let tex = ctx.load_texture(
            "a",
            egui::ColorImage::new([1, 1], vec![egui::Color32::WHITE]),
            egui::TextureOptions::LINEAR,
        );
        home.textures.insert(
            PathBuf::from("a.png"),
            Thumbnail::Ready {
                tex,
                size: [1, 1],
                used: 0,
                made_from: (t1, 100),
            },
        );
        home.set_entries(vec![shot(t1, 100)]);
        assert!(
            home.textures.contains_key(Path::new("a.png")),
            "unchanged: kept"
        );
        home.set_entries(vec![shot(t1 + std::time::Duration::from_secs(5), 100)]);
        assert!(
            !home.textures.contains_key(Path::new("a.png")),
            "saved over: made again"
        );
    }

    #[test]
    fn the_thumbnail_cache_keeps_the_most_recently_drawn() {
        let (mut home, _shared) = Home::for_tests(vec![]);
        let ctx = egui::Context::default();
        for i in 0..THUMB_CACHE + 10 {
            let tex = ctx.load_texture(
                format!("t{i}"),
                egui::ColorImage::new([1, 1], vec![egui::Color32::WHITE]),
                egui::TextureOptions::LINEAR,
            );
            home.textures.insert(
                PathBuf::from(format!("{i}.png")),
                Thumbnail::Ready {
                    tex,
                    size: [1, 1],
                    used: i as u64,
                    made_from: (std::time::UNIX_EPOCH, 0),
                },
            );
        }
        home.evict();
        assert_eq!(home.textures.len(), THUMB_CACHE);
        for i in 0..10 {
            assert!(
                !home
                    .textures
                    .contains_key(&PathBuf::from(format!("{i}.png"))),
                "{i} is the oldest"
            );
        }
        assert!(
            home.textures
                .contains_key(&PathBuf::from(format!("{}.png", THUMB_CACHE + 9)))
        );
    }

    #[test]
    fn home_wakes_up_to_read_the_folder_again() {
        let (mut home, mut shared) = Home::for_tests(vec![]);
        let ctx = egui::Context::default();
        let mut full = ctx.run_ui(egui::RawInput::default(), |ui| {
            home.begin(ui, &mut shared);
        });
        full.textures_delta.clear();
        let delay = full.viewport_output[&egui::ViewportId::ROOT].repaint_delay;
        assert!(
            delay <= std::time::Duration::from_secs_f64(RESCAN_SECONDS),
            "{delay:?}"
        );
    }

    #[test]
    fn the_preview_renders_once_per_change() {
        let (mut home, mut shared) = Home::for_tests(vec![]);
        let ctx = egui::Context::default();
        home.style_preview(&ctx, &shared);
        home.style_preview(&ctx, &shared);
        assert_eq!(home.preview_renders(), 1, "nothing changed");
        shared.settings.style.padding += 3.0;
        home.style_preview(&ctx, &shared);
        assert_eq!(home.preview_renders(), 2, "the margin changed");
        shared.settings.watermark.enabled = !shared.settings.watermark.enabled;
        home.style_preview(&ctx, &shared);
        assert_eq!(home.preview_renders(), 3, "the watermark changed");
    }
}
