//! The desktop's own notifications, for where a window cannot be the copy and save notice.
//!
//! On Wayland a client chooses neither its place nor "always on top", and the capture window is
//! full screen, so its notice came out as a full-screen window. There the notice goes to
//! `org.freedesktop.Notifications` instead (KDE, GNOME and the rest draw it in their corner), and
//! the process stays a few seconds to hear a click on a saved capture, which opens its folder.

use crate::system::resident::on_wayland;
use std::path::PathBuf;
use std::sync::{Mutex, mpsc};
use std::time::{Duration, Instant};

/// What a notification says: the notice's words, and the folder a click on it opens.
#[derive(Clone, Debug, PartialEq)]
pub struct Spec {
    pub title: String,
    pub body: String,
    pub folder: Option<PathBuf>,
}

/// How long the desktop keeps the notification on screen.
#[cfg(target_os = "linux")]
const SHOWN_FOR: Duration = Duration::from_secs(6);
/// How long the process waits for a click before it leaves, a little longer than it is shown.
const STAY: Duration = Duration::from_secs(8);

/// The notifications on their way, each ending when its thread is done (shown, and for a saved
/// capture clicked, closed or timed out).
static PENDING: Mutex<Vec<mpsc::Receiver<()>>> = Mutex::new(Vec::new());

/// Whether the notice is a desktop notification here instead of a window.
pub fn native_notices() -> bool {
    on_wayland()
}

/// Shows the notification on a thread of its own (the desktop answers over D-Bus).
pub fn show(spec: Spec) {
    let (done, finished) = mpsc::channel();
    if let Ok(mut pending) = PENDING.lock() {
        pending.push(finished);
    }
    std::thread::spawn(move || {
        deliver(&spec);
        let _ = done.send(());
    });
}

/// Keeps the process alive for the click on a notification still on screen, for `STAY` at most:
/// a desktop that never reports it closed must not hold a faceless process forever.
pub fn wait_pending() {
    let waiting: Vec<_> = PENDING
        .lock()
        .map(|mut p| p.drain(..).collect())
        .unwrap_or_default();
    wait_all(waiting, STAY);
}

/// Waits for every channel to answer, `budget` in all; how many were still silent.
fn wait_all(waiting: Vec<mpsc::Receiver<()>>, budget: Duration) -> usize {
    let end = Instant::now() + budget;
    waiting
        .iter()
        .filter(|w| {
            w.recv_timeout(end.saturating_duration_since(Instant::now()))
                .is_err()
        })
        .count()
}

#[cfg(target_os = "linux")]
fn deliver(spec: &Spec) {
    use notify_rust::{Hint, Notification, Timeout};
    let mut n = Notification::new();
    n.appname("Takeshot")
        .summary(&spec.title)
        .body(&spec.body)
        .icon("takeshot")
        .hint(Hint::DesktopEntry("takeshot".into()))
        .timeout(Timeout::Milliseconds(SHOWN_FOR.as_millis() as u32));
    if spec.folder.is_some() {
        // The click on the notification itself: the desktops send it as "default".
        n.action("default", "Open");
    }
    match n.show() {
        Ok(shown) => {
            if let Some(dir) = &spec.folder {
                shown.wait_for_action(|action| {
                    if action == "default" {
                        crate::system::actions::open_folder(dir);
                    }
                });
            }
        }
        Err(e) => eprintln!("takeshot: no notification: {e}"),
    }
}

#[cfg(not(target_os = "linux"))]
fn deliver(_: &Spec) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_wait_ends_with_its_budget_when_nobody_answers() {
        let (_silent, never) = mpsc::channel::<()>();
        let start = Instant::now();
        assert_eq!(wait_all(vec![never], Duration::from_millis(60)), 1);
        let took = start.elapsed();
        assert!(took >= Duration::from_millis(60), "{took:?}");
        assert!(took < Duration::from_secs(5), "{took:?}");
    }

    #[test]
    fn the_wait_does_not_hold_the_process_once_all_answered() {
        let (a, first) = mpsc::channel();
        let (b, second) = mpsc::channel();
        a.send(()).unwrap();
        b.send(()).unwrap();
        let start = Instant::now();
        assert_eq!(wait_all(vec![first, second], Duration::from_secs(30)), 0);
        assert!(start.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn a_thread_that_died_does_not_hold_the_process_either() {
        let (gone, lost) = mpsc::channel::<()>();
        drop(gone);
        let start = Instant::now();
        wait_all(vec![lost], Duration::from_secs(30));
        assert!(start.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn only_a_wayland_session_gets_the_desktop_notification() {
        // Windows, macOS and X11 keep the notice window in the corner.
        assert_eq!(native_notices(), on_wayland());
        if cfg!(not(target_os = "linux")) {
            assert!(!native_notices());
        }
    }
}
