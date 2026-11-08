//! The tray: its menu entries in spec order (shared by the app-drawn and the native menu), the icon and its events.

use crate::system::hotkeys::{Shot, label};
use crate::system::instance::Page;
use crate::ui::i18n::{Msg, t};
use crate::ui::tokens::{ThemeId, UiMode};
use std::path::PathBuf;

/// What choosing a tray entry does.
#[derive(Clone, PartialEq, Debug)]
pub enum TrayAction {
    Capture(Shot),
    CaptureIn3s,
    Show(Page),
    /// Click on a recent capture copies it; Shift+click opens it in the editor.
    CopyRecent(PathBuf),
    EditRecent(PathBuf),
    OpenCapturesFolder,
    SetMode(UiMode),
    Quit,
}

/// One row of the tray menu.
#[derive(Clone, PartialEq, Debug)]
pub enum TrayEntry {
    Item {
        label: String,
        keys: Option<String>,
        action: TrayAction,
    },
    /// The "Recent captures" submenu: these files, then "Open captures folder".
    Recents(Vec<PathBuf>),
    /// Light, Dark and System, with the current one marked.
    Mode(UiMode),
    Separator,
}

fn item(m: Msg, keys: Option<String>, action: TrayAction) -> TrayEntry {
    TrayEntry::Item {
        label: t(m).to_string(),
        keys,
        action,
    }
}

/// The menu, in the order of spec 5.1: captures, then the app and its pages, then About and Quit.
pub fn tray_entries(
    recents: &[PathBuf],
    mode: UiMode,
    area_keys: &str,
    full_keys: &str,
) -> Vec<TrayEntry> {
    vec![
        item(
            Msg::CaptureArea,
            Some(label(area_keys)),
            TrayAction::Capture(Shot::Area),
        ),
        item(
            Msg::FullScreen,
            Some(label(full_keys)),
            TrayAction::Capture(Shot::Full),
        ),
        item(Msg::CaptureIn3s, None, TrayAction::CaptureIn3s),
        TrayEntry::Separator,
        item(Msg::OpenTakeshot, None, TrayAction::Show(Page::Start)),
        TrayEntry::Recents(recents.to_vec()),
        item(Msg::StyleAndWatermark, None, TrayAction::Show(Page::Style)),
        item(Msg::Settings, None, TrayAction::Show(Page::Settings)),
        TrayEntry::Mode(mode),
        TrayEntry::Separator,
        item(Msg::About, None, TrayAction::Show(Page::About)),
        item(Msg::Quit, None, TrayAction::Quit),
    ]
}

/// What the tray icon reports to the resident.
#[derive(Clone, PartialEq, Debug)]
pub enum TrayEvent {
    // AppIndicator (Linux) reports no clicks: its menu is the desktop's own.
    #[cfg_attr(target_os = "linux", allow(dead_code))]
    LeftClick,
    /// Right click: open the app-drawn menu next to the icon (`icon` in physical pixels).
    #[cfg_attr(target_os = "linux", allow(dead_code))]
    MenuAt { icon: egui::Rect },
    /// A native menu entry (Linux).
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    Action(TrayAction),
}

/// The id of a native menu entry for an action; recents are numbered by position.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn id_for(a: &TrayAction, recents: &[PathBuf]) -> String {
    match a {
        TrayAction::Capture(Shot::Area) => "capture-area".into(),
        TrayAction::Capture(Shot::Full) => "capture-full".into(),
        TrayAction::CaptureIn3s => "capture-3s".into(),
        TrayAction::Show(p) => format!("show-{}", p.as_str()),
        TrayAction::CopyRecent(f) | TrayAction::EditRecent(f) => {
            let i = recents.iter().position(|r| r == f).unwrap_or(usize::MAX);
            format!("recent-{i}")
        }
        TrayAction::OpenCapturesFolder => "folder".into(),
        TrayAction::SetMode(UiMode::Light) => "mode-light".into(),
        TrayAction::SetMode(UiMode::Dark) => "mode-dark".into(),
        TrayAction::SetMode(UiMode::System) => "mode-system".into(),
        TrayAction::Quit => "quit".into(),
    }
}

/// The action of a native menu id; a recent opens in the editor when Shift is held.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn action_for_id(id: &str, recents: &[PathBuf], shift: bool) -> Option<TrayAction> {
    Some(match id {
        "capture-area" => TrayAction::Capture(Shot::Area),
        "capture-full" => TrayAction::Capture(Shot::Full),
        "capture-3s" => TrayAction::CaptureIn3s,
        "folder" => TrayAction::OpenCapturesFolder,
        "mode-light" => TrayAction::SetMode(UiMode::Light),
        "mode-dark" => TrayAction::SetMode(UiMode::Dark),
        "mode-system" => TrayAction::SetMode(UiMode::System),
        "quit" => TrayAction::Quit,
        _ => {
            if let Some(p) = id.strip_prefix("show-") {
                TrayAction::Show(Page::parse(p)?)
            } else {
                let f = recents.get(id.strip_prefix("recent-")?.parse::<usize>().ok()?)?;
                if shift {
                    TrayAction::EditRecent(f.clone())
                } else {
                    TrayAction::CopyRecent(f.clone())
                }
            }
        }
    })
}

/// Whether a panel will show the tray icon. Linux: a StatusNotifier host is registered on the
/// session bus (KDE, XFCE, GNOME with the AppIndicator extension; stock GNOME has none). The
/// icon registers anyway and appears once a panel does, but until then hiding the window to the
/// tray would leave no way back. Elsewhere the system always has one.
pub fn shown() -> bool {
    #[cfg(target_os = "linux")]
    {
        // The bus answers in milliseconds; a stuck one must not hold the UI, and "no" is the safe answer.
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(host_registered());
        });
        rx.recv_timeout(std::time::Duration::from_millis(500))
            .unwrap_or(false)
    }
    #[cfg(not(target_os = "linux"))]
    true
}

#[cfg(target_os = "linux")]
fn host_registered() -> bool {
    use zbus::blocking::{Connection, Proxy};
    let Ok(bus) = Connection::session() else {
        return false;
    };
    Proxy::new(
        &bus,
        "org.kde.StatusNotifierWatcher",
        "/StatusNotifierWatcher",
        "org.kde.StatusNotifierWatcher",
    )
    .and_then(|watcher| watcher.get_property::<bool>("IsStatusNotifierHostRegistered"))
    .unwrap_or(false)
}

/// Whether the Windows taskbar is dark (it decides the tray glyph colour). Elsewhere the panel is
/// assumed dark; macOS uses a template icon that the system tints.
#[cfg(windows)]
pub fn taskbar_is_dark() -> bool {
    use winreg::RegKey;
    use winreg::enums::HKEY_CURRENT_USER;
    RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize")
        .and_then(|k| k.get_value::<u32, _>("SystemUsesLightTheme"))
        .map_or(true, |v| v == 0)
}

#[cfg(not(windows))]
pub fn taskbar_is_dark() -> bool {
    !cfg!(target_os = "macos")
}

fn tray_icon_image(theme: ThemeId) -> Result<tray_icon::Icon, String> {
    let (rgba, w, h) = crate::ui::brand::tray_rgba(theme, taskbar_is_dark(), 32);
    tray_icon::Icon::from_rgba(rgba, w, h).map_err(|e| e.to_string())
}

/// The tray icon. On Windows and macOS it reports clicks and the app draws the menu; it must be
/// created and kept on the main thread with the event loop running.
#[cfg(not(target_os = "linux"))]
pub struct Tray {
    icon: tray_icon::TrayIcon,
}

#[cfg(not(target_os = "linux"))]
impl Tray {
    pub fn start(
        theme: ThemeId,
        on_event: impl Fn(TrayEvent) + Send + Sync + 'static,
    ) -> Result<Self, String> {
        use tray_icon::{MouseButton, MouseButtonState, TrayIconEvent};
        TrayIconEvent::set_event_handler(Some(move |e: TrayIconEvent| {
            if let TrayIconEvent::Click {
                button,
                button_state: MouseButtonState::Up,
                rect,
                ..
            } = e
            {
                let icon = egui::Rect::from_min_size(
                    egui::pos2(rect.position.x as f32, rect.position.y as f32),
                    egui::vec2(rect.size.width as f32, rect.size.height as f32),
                );
                match button {
                    MouseButton::Left => on_event(TrayEvent::LeftClick),
                    MouseButton::Right => on_event(TrayEvent::MenuAt { icon }),
                    MouseButton::Middle => {}
                }
            }
        }));
        let builder = tray_icon::TrayIconBuilder::new().with_tooltip("Takeshot");
        // macOS draws the glyph as a template, so the menu bar tints it light or dark.
        #[cfg(target_os = "macos")]
        let builder = builder.with_icon_templated(tray_icon_image(theme)?);
        #[cfg(not(target_os = "macos"))]
        let builder = builder.with_icon(tray_icon_image(theme)?);
        let icon = builder
            .with_menu_on_left_click(false)
            .with_menu_on_right_click(false)
            .build()
            .map_err(|e| e.to_string())?;
        Ok(Self { icon })
    }

    pub fn set_theme(&self, theme: ThemeId) {
        if let Ok(img) = tray_icon_image(theme) {
            #[cfg(target_os = "macos")]
            let _ = self.icon.set_icon_templated(Some(img));
            #[cfg(not(target_os = "macos"))]
            let _ = self.icon.set_icon(Some(img));
        }
    }

    /// The app draws the menu itself on these systems.
    pub fn refresh_menu(&self, _entries: &[TrayEntry]) {}
}

/// The tray icon on Linux: AppIndicator shows the desktop's own menu and reports no clicks, so the
/// native menu is built from the same entries on a GTK thread.
#[cfg(target_os = "linux")]
pub struct Tray;

#[cfg(target_os = "linux")]
mod gtk_thread {
    use super::*;
    use std::cell::RefCell;
    use std::sync::{Arc, Mutex};
    use tray_icon::menu::{
        CheckMenuItem, IsMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu,
    };

    thread_local! {
        static TRAY: RefCell<Option<tray_icon::TrayIcon>> = const { RefCell::new(None) };
    }

    /// The recents of the menu on screen, so a click maps back to its file.
    pub static RECENTS: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());

    fn shift_held() -> bool {
        gtk::gdk::Display::default()
            .and_then(|d| gtk::gdk::Keymap::for_display(&d))
            .is_some_and(|k| k.modifier_state() & gtk::gdk::ModifierType::SHIFT_MASK.bits() != 0)
    }

    pub fn native_menu(entries: &[TrayEntry]) -> Menu {
        let menu = Menu::new();
        let recents = entries
            .iter()
            .find_map(|e| match e {
                TrayEntry::Recents(r) => Some(r.clone()),
                _ => None,
            })
            .unwrap_or_default();
        for e in entries {
            let _ = match e {
                TrayEntry::Separator => menu.append(&PredefinedMenuItem::separator()),
                TrayEntry::Item {
                    label,
                    keys,
                    action,
                } => {
                    let text = match keys {
                        Some(k) => format!("{label}    {k}"),
                        None => label.clone(),
                    };
                    menu.append(&MenuItem::with_id(
                        id_for(action, &recents),
                        text,
                        true,
                        None,
                    ))
                }
                TrayEntry::Recents(files) => {
                    let sub = Submenu::new(t(Msg::Recent), true);
                    for f in files {
                        let a = TrayAction::CopyRecent(f.clone());
                        let _ = sub.append(&MenuItem::with_id(
                            id_for(&a, files),
                            crate::system::recents::recent_label(f),
                            true,
                            None,
                        ));
                    }
                    let _ = sub.append(&PredefinedMenuItem::separator());
                    let _ = sub.append(&MenuItem::with_id(
                        "folder",
                        t(Msg::OpenCapturesFolder),
                        true,
                        None,
                    ));
                    menu.append(&sub)
                }
                TrayEntry::Mode(current) => {
                    let sub = Submenu::new(t(Msg::Theme), true);
                    for (m, msg) in [
                        (UiMode::Light, Msg::ModeLight),
                        (UiMode::Dark, Msg::ModeDark),
                        (UiMode::System, Msg::ModeSystem),
                    ] {
                        let item = CheckMenuItem::with_id(
                            id_for(&TrayAction::SetMode(m), &[]),
                            t(msg),
                            true,
                            m == *current,
                            None,
                        );
                        let _ = sub.append(&item as &dyn IsMenuItem);
                    }
                    menu.append(&sub)
                }
            };
        }
        if let Ok(mut r) = RECENTS.lock() {
            *r = recents;
        }
        menu
    }

    pub fn start(theme: ThemeId, on_event: Arc<dyn Fn(TrayEvent) + Send + Sync>) {
        MenuEvent::set_event_handler(Some(move |e: MenuEvent| {
            let recents = RECENTS.lock().map(|r| r.clone()).unwrap_or_default();
            if let Some(a) = action_for_id(&e.id.0, &recents, shift_held()) {
                on_event(TrayEvent::Action(a));
            }
        }));
        let _ = std::thread::Builder::new()
            .name("takeshot-tray".into())
            .spawn(move || {
                if let Err(e) = gtk::init() {
                    eprintln!("takeshot: no tray, GTK did not start: {e}");
                    return;
                }
                let icon = match tray_icon_image(theme) {
                    Ok(i) => i,
                    Err(e) => {
                        eprintln!("takeshot: no tray icon: {e}");
                        return;
                    }
                };
                match tray_icon::TrayIconBuilder::new()
                    .with_tooltip("Takeshot")
                    .with_icon(icon)
                    .with_menu(Box::new(Menu::new()))
                    .build()
                {
                    Ok(tray) => TRAY.with(|t| *t.borrow_mut() = Some(tray)),
                    Err(e) => {
                        eprintln!("takeshot: no tray: {e}");
                        return;
                    }
                }
                gtk::main();
            });
    }

    /// Runs `f` with the tray on the GTK thread.
    pub fn with_tray(f: impl FnOnce(&tray_icon::TrayIcon) + Send + 'static) {
        gtk::glib::idle_add_once(move || {
            TRAY.with(|t| {
                if let Some(tray) = t.borrow().as_ref() {
                    f(tray);
                }
            })
        });
    }
}

#[cfg(target_os = "linux")]
impl Tray {
    pub fn start(
        theme: ThemeId,
        on_event: impl Fn(TrayEvent) + Send + Sync + 'static,
    ) -> Result<Self, String> {
        gtk_thread::start(theme, std::sync::Arc::new(on_event));
        Ok(Self)
    }

    pub fn set_theme(&self, theme: ThemeId) {
        gtk_thread::with_tray(move |tray| {
            if let Ok(img) = tray_icon_image(theme) {
                let _ = tray.set_icon(Some(img));
            }
        });
    }

    /// Rebuilds the desktop's menu from the entries (recent captures and the mode change).
    pub fn refresh_menu(&self, entries: &[TrayEntry]) {
        let entries = entries.to_vec();
        gtk_thread::with_tray(move |tray| {
            tray.set_menu(Some(Box::new(gtk_thread::native_menu(&entries))));
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn actions(entries: &[TrayEntry]) -> Vec<String> {
        entries
            .iter()
            .map(|e| match e {
                TrayEntry::Item { action, .. } => format!("{action:?}"),
                TrayEntry::Recents(r) => format!("Recents({})", r.len()),
                TrayEntry::Mode(m) => format!("Mode({m:?})"),
                TrayEntry::Separator => "-".into(),
            })
            .collect()
    }

    #[test]
    fn native_menu_ids_map_back_to_their_actions() {
        let recents = vec![PathBuf::from("a.png"), PathBuf::from("b.png")];
        let mut all = vec![
            TrayAction::Capture(Shot::Area),
            TrayAction::Capture(Shot::Full),
            TrayAction::CaptureIn3s,
            TrayAction::OpenCapturesFolder,
            TrayAction::Quit,
            TrayAction::CopyRecent(recents[1].clone()),
        ];
        all.extend([Page::Start, Page::Style, Page::Settings, Page::About].map(TrayAction::Show));
        all.extend([UiMode::Light, UiMode::Dark, UiMode::System].map(TrayAction::SetMode));
        for a in all {
            let id = id_for(&a, &recents);
            assert_eq!(action_for_id(&id, &recents, false), Some(a.clone()), "{id}");
        }
        assert_eq!(
            action_for_id("recent-1", &recents, true),
            Some(TrayAction::EditRecent(recents[1].clone()))
        );
        assert_eq!(action_for_id("recent-9", &recents, false), None);
        assert_eq!(action_for_id("show-nowhere", &recents, false), None);
    }

    #[test]
    fn tray_entries_follow_the_spec_order() {
        let recents = vec![PathBuf::from("a.png"), PathBuf::from("b.png")];
        let e = tray_entries(&recents, UiMode::Dark, "PrintScreen", "shift+PrintScreen");
        assert_eq!(
            actions(&e),
            vec![
                format!("{:?}", TrayAction::Capture(Shot::Area)),
                format!("{:?}", TrayAction::Capture(Shot::Full)),
                format!("{:?}", TrayAction::CaptureIn3s),
                "-".into(),
                format!("{:?}", TrayAction::Show(Page::Start)),
                "Recents(2)".into(),
                format!("{:?}", TrayAction::Show(Page::Style)),
                format!("{:?}", TrayAction::Show(Page::Settings)),
                "Mode(Dark)".into(),
                "-".into(),
                format!("{:?}", TrayAction::Show(Page::About)),
                format!("{:?}", TrayAction::Quit),
            ]
        );
        // The capture items show their shortcuts the way the keyboard reads them.
        let keys: Vec<Option<String>> = e
            .iter()
            .take(3)
            .map(|x| match x {
                TrayEntry::Item { keys, .. } => keys.clone(),
                _ => None,
            })
            .collect();
        assert_eq!(
            keys,
            vec![Some("PrtSc".into()), Some("Shift+PrtSc".into()), None]
        );
        // With no captures yet the submenu stays, holding only "Open captures folder".
        let empty = tray_entries(&[], UiMode::System, "PrintScreen", "shift+PrintScreen");
        assert!(empty.contains(&TrayEntry::Recents(Vec::new())));
    }
}
