//! The resident process: single instance, hotkeys, PrtSc hook and tray feed one event queue the app drains each frame.

use crate::system::hotkeys::{Hotkeys, Shot};
use crate::system::instance::{Listener, Page};
use crate::system::settings::{OnClose, Settings, TrayClick};
use crate::system::tray::{Tray, TrayEvent};
use std::sync::mpsc;

/// Something the resident must react to on the UI thread.
#[derive(Clone, PartialEq, Debug)]
pub enum ResidentEvent {
    Show(Page),
    Shot(Shot),
    Tray(TrayEvent),
}

/// What the X of the main window does right now.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum XAction {
    Ask,
    Hide,
    Quit,
}

/// winit uses Wayland when the session has one.
pub fn on_wayland() -> bool {
    cfg!(target_os = "linux") && std::env::var_os("WAYLAND_DISPLAY").is_some()
}

/// What the close prompt stores: the answer (Hide or Quit), only with "Remember my choice"
/// checked. `Ask` is the prompt cancelled.
pub fn remembered(answer: XAction, remember: bool) -> Option<OnClose> {
    match answer {
        XAction::Hide if remember => Some(OnClose::Minimize),
        XAction::Quit if remember => Some(OnClose::Quit),
        _ => None,
    }
}

pub fn on_x(setting: OnClose) -> XAction {
    match setting {
        OnClose::Ask => XAction::Ask,
        OnClose::Minimize => XAction::Hide,
        OnClose::Quit => XAction::Quit,
    }
}

/// A left click on the tray icon, as the setting wants it.
pub fn left_click(setting: TrayClick) -> ResidentEvent {
    match setting {
        TrayClick::OpenHome => ResidentEvent::Show(Page::Start),
        TrayClick::CaptureArea => ResidentEvent::Shot(Shot::Area),
    }
}

/// The UI-thread end of the event queue.
pub struct Queue {
    rx: mpsc::Receiver<ResidentEvent>,
}

/// The end the hotkey, hook, tray and instance threads hold: sending also wakes the app. It wakes
/// the context it points at, which `retarget` moves when the window changes (Linux destroys the
/// main window to hide it and makes another to show it).
#[derive(Clone)]
pub struct Sender {
    tx: mpsc::Sender<ResidentEvent>,
    ctx: std::sync::Arc<std::sync::Mutex<egui::Context>>,
}

impl Queue {
    pub fn new(ctx: &egui::Context) -> (Self, Sender) {
        let (tx, rx) = mpsc::channel();
        (
            Self { rx },
            Sender {
                tx,
                ctx: std::sync::Arc::new(std::sync::Mutex::new(ctx.clone())),
            },
        )
    }

    pub fn drain(&self) -> Vec<ResidentEvent> {
        self.rx.try_iter().collect()
    }
}

impl Sender {
    /// Queues the event and asks egui for a frame; cheap enough for a keyboard hook callback.
    pub fn send(&self, e: ResidentEvent) {
        let _ = self.tx.send(e);
        self.wake();
    }

    /// Asks for a frame (or, with no window, a pass of the resident's loop) without an event.
    pub fn wake(&self) {
        let ctx = self.ctx.lock().ok().map(|c| c.clone());
        if let Some(ctx) = ctx {
            ctx.request_repaint();
        }
    }

    /// From now on wakes `ctx`, for every clone of this sender.
    pub fn retarget(&self, ctx: &egui::Context) {
        if let Ok(mut c) = self.ctx.lock() {
            *c = ctx.clone();
        }
    }
}

/// What the resident owns: the hotkeys, the PrtSc hook (Windows), the tray icon and the queue
/// their threads feed. It lives on the main thread (the tray icon must).
pub struct Resident {
    queue: Queue,
    sender: Sender,
    pub hotkeys: Hotkeys,
    #[cfg(windows)]
    hook: Option<crate::system::prtsc_hook::PrtScHook>,
    pub tray: Option<Tray>,
    /// The process ids of the windows of their own (editors, pinned pictures, the capture) that
    /// registered, for Quit.
    children: std::sync::Arc<std::sync::Mutex<Vec<u32>>>,
}

impl Resident {
    /// The children registered so far.
    pub fn children(&self) -> Vec<u32> {
        self.children.lock().map(|c| c.clone()).unwrap_or_default()
    }

    /// Starts everything; `listener` answers second launches when this process won the name.
    pub fn start(ctx: &egui::Context, settings: &Settings, listener: Option<Listener>) -> Self {
        let (queue, sender) = Queue::new(ctx);
        let children = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        if let Some(l) = listener {
            let s = sender.clone();
            let list = children.clone();
            l.serve(
                move |page| s.send(ResidentEvent::Show(page)),
                move |pid| {
                    if let Ok(mut c) = list.lock() {
                        c.push(pid);
                    }
                },
            );
        }
        let s = sender.clone();
        let hotkeys = Hotkeys::start(settings, move |shot| s.send(ResidentEvent::Shot(shot)));
        let s = sender.clone();
        let tray = match Tray::start(settings.ui_theme, move |e| s.send(ResidentEvent::Tray(e))) {
            Ok(t) => Some(t),
            Err(e) => {
                eprintln!("takeshot: no tray icon: {e}");
                None
            }
        };
        let mut r = Self {
            queue,
            sender,
            hotkeys,
            #[cfg(windows)]
            hook: None,
            tray,
            children,
        };
        r.set_priority(wants_hook(settings));
        r
    }

    pub fn poll(&self) -> Vec<ResidentEvent> {
        self.queue.drain()
    }

    /// Whether the tray icon is made and a panel shows it: hiding the window to the tray is
    /// only a way to leave it when there is a way back.
    pub fn tray_shown(&self) -> bool {
        self.tray.is_some() && crate::system::tray::shown()
    }

    /// The end that wakes whichever context the resident draws on now.
    pub fn sender(&self) -> Sender {
        self.sender.clone()
    }

    /// Wake the context of the window that exists now (or of the resident's loop with none).
    pub fn retarget(&self, ctx: &egui::Context) {
        self.sender.retarget(ctx);
    }

    /// Turns the PrtSc priority hook on or off (Windows; elsewhere there is none).
    pub fn set_priority(&mut self, on: bool) {
        #[cfg(windows)]
        {
            if on && self.hook.is_none() {
                let s = self.sender.clone();
                self.hook = match crate::system::prtsc_hook::PrtScHook::install(move |shot| {
                    s.send(ResidentEvent::Shot(shot))
                }) {
                    Ok(h) => Some(h),
                    Err(e) => {
                        eprintln!("takeshot: PrtSc priority unavailable: {e}");
                        None
                    }
                };
            } else if !on {
                self.hook = None;
            }
        }
        #[cfg(not(windows))]
        let _ = (on, &self.sender);
    }

    pub fn priority_on(&self) -> bool {
        #[cfg(windows)]
        return self.hook.is_some();
        #[cfg(not(windows))]
        false
    }
}

/// Whether the PrtSc priority hook should run: asked for, and the global shortcuts switched on.
pub fn wants_hook(settings: &Settings) -> bool {
    settings.prtsc_priority && settings.hotkeys
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::system::settings::{OnClose, TrayClick};

    #[test]
    fn the_prtsc_hook_needs_the_shortcuts_on() {
        // Turning the global shortcuts off frees PrtSc, priority or not.
        let mut s = Settings {
            prtsc_priority: true,
            hotkeys: true,
            ..Settings::default()
        };
        assert!(wants_hook(&s));
        s.hotkeys = false;
        assert!(!wants_hook(&s));
        s.hotkeys = true;
        s.prtsc_priority = false;
        assert!(!wants_hook(&s));
    }

    #[test]
    fn x_follows_the_remembered_choice() {
        assert_eq!(on_x(OnClose::Ask), XAction::Ask);
        assert_eq!(on_x(OnClose::Minimize), XAction::Hide);
        assert_eq!(on_x(OnClose::Quit), XAction::Quit);
    }

    #[test]
    fn left_click_on_the_icon_follows_the_setting() {
        assert_eq!(
            left_click(TrayClick::OpenHome),
            ResidentEvent::Show(Page::Start)
        );
        assert_eq!(
            left_click(TrayClick::CaptureArea),
            ResidentEvent::Shot(Shot::Area)
        );
    }

    #[test]
    fn events_reach_the_queue_and_wake_the_app() {
        let ctx = egui::Context::default();
        let woken = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let w = woken.clone();
        ctx.set_request_repaint_callback(move |_| {
            w.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        });
        let (q, sender) = Queue::new(&ctx);
        sender.send(ResidentEvent::Show(Page::Settings));
        // egui folds wake-ups until the next pass, which drains the whole queue: one is enough.
        assert_eq!(
            woken.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "an event wakes the app, window shown or not"
        );
        sender.send(ResidentEvent::Shot(Shot::Full));
        assert_eq!(
            q.drain(),
            vec![
                ResidentEvent::Show(Page::Settings),
                ResidentEvent::Shot(Shot::Full)
            ]
        );
        assert!(q.drain().is_empty());
    }

    #[test]
    fn events_wake_the_context_the_resident_points_at() {
        // On Linux the window is destroyed and made again: the threads that hold a sender must
        // wake the context that exists now, not the one of the window that closed.
        fn counted(ctx: &egui::Context) -> std::sync::Arc<std::sync::atomic::AtomicUsize> {
            let n = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let n2 = n.clone();
            ctx.set_request_repaint_callback(move |_| {
                n2.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            });
            n
        }
        let woken =
            |n: &std::sync::atomic::AtomicUsize| n.load(std::sync::atomic::Ordering::SeqCst);
        let (first, second) = (egui::Context::default(), egui::Context::default());
        let (a, b) = (counted(&first), counted(&second));
        let (_queue, sender) = Queue::new(&first);
        let elsewhere = sender.clone();
        sender.send(ResidentEvent::Show(Page::Start));
        assert_eq!((woken(&a), woken(&b)), (1, 0));
        sender.retarget(&second);
        elsewhere.send(ResidentEvent::Show(Page::Start));
        assert_eq!(
            (woken(&a), woken(&b)),
            (1, 1),
            "a clone follows the retarget"
        );
        let third = egui::Context::default();
        let c = counted(&third);
        elsewhere.retarget(&third);
        sender.wake();
        assert_eq!(woken(&c), 1, "wake() asks for a frame without an event");
    }

    #[test]
    fn the_close_prompt_stores_a_choice_only_when_asked() {
        assert_eq!(remembered(XAction::Hide, true), Some(OnClose::Minimize));
        assert_eq!(remembered(XAction::Quit, true), Some(OnClose::Quit));
        assert_eq!(remembered(XAction::Hide, false), None, "box unchecked");
        assert_eq!(remembered(XAction::Quit, false), None, "box unchecked");
        assert_eq!(remembered(XAction::Ask, true), None, "cancelled");
    }
}
