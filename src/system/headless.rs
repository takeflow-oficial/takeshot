//! The resident with no window (Linux): sleeps until something wakes it.
//!
//! A Wayland client cannot hide its window, so closing it to the tray destroys it and the resident
//! carries on here: one egui pass per wake-up on a context nobody sees, which is how the events of
//! the tray, the hotkeys and a second launch are handled, until one asks for a window again. The
//! idea (closing the window to hide it, a loop between windows) is the one ZapFast's
//! `fastframe-shell` (MIT) uses; the code here is Takeshot's own.

use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

/// The sleep between wake-ups: ends when woken, from any thread, or with its time.
#[derive(Clone, Default)]
pub struct Idle {
    woken: Arc<(Mutex<bool>, Condvar)>,
}

impl Idle {
    pub fn wake(&self) {
        let (flag, wake) = &*self.woken;
        if let Ok(mut f) = flag.lock() {
            *f = true;
            wake.notify_all();
        }
    }

    /// Returns when woken (a wake that came earlier counts) or after `at_most`; one wake ends one
    /// sleep.
    pub fn sleep(&self, at_most: Duration) {
        let (flag, wake) = &*self.woken;
        let Ok(woken) = flag.lock() else {
            return;
        };
        if let Ok((mut woken, _)) = wake.wait_timeout_while(woken, at_most, |w| !*w) {
            *woken = false;
        }
    }
}

/// The context the resident's passes run on while no window exists. A repaint asked for now (by a
/// thread that holds the context) wakes `idle`; one asked for later does not, because the loop
/// wakes on its own often enough.
pub fn context(idle: &Idle) -> egui::Context {
    let ctx = egui::Context::default();
    let idle = idle.clone();
    ctx.set_request_repaint_callback(move |info| {
        if info.delay.is_zero() {
            idle.wake();
        }
    });
    ctx
}

/// One egui pass on `ctx` at `now` seconds, with nothing to draw: what `f` asks of the context
/// (repaints, viewport commands, textures) is let go when it ends.
pub fn pass<T>(ctx: &egui::Context, now: f64, f: impl FnOnce(&egui::Context) -> T) -> T {
    ctx.begin_pass(egui::RawInput {
        time: Some(now),
        ..Default::default()
    });
    let out = f(ctx);
    // Nothing paints these: egui panics on textures dropped without being applied.
    ctx.end_pass().textures_delta.clear();
    out
}

/// How long a hidden start waits for a panel that shows the tray icon before it shows the window
/// (at login the panel may come up after the app).
pub const TRAY_WAIT: Duration = Duration::from_secs(30);

/// How often a hidden start looks for the tray while it waits.
pub const TRAY_POLL: Duration = Duration::from_secs(2);

/// What a hidden start does about the tray.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TrayWait {
    /// The tray is shown: stay hidden.
    Done,
    /// Not yet: look again.
    Keep,
    /// None came: show the window, or the app stays unseen.
    GiveUp,
}

pub fn tray_wait(waited: Duration, shown: bool) -> TrayWait {
    if shown {
        TrayWait::Done
    } else if waited >= TRAY_WAIT {
        TrayWait::GiveUp
    } else {
        TrayWait::Keep
    }
}

/// What the resident does after a window closed or a pass ran.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Next {
    /// Open the main window.
    Window,
    /// Sleep until something wakes the resident.
    Wait,
    /// Leave the process.
    Quit,
}

pub fn next(wants_window: bool, quitting: bool) -> Next {
    if quitting {
        Next::Quit
    } else if wants_window {
        Next::Window
    } else {
        Next::Wait
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn quick(f: impl FnOnce()) -> Duration {
        let start = Instant::now();
        f();
        start.elapsed()
    }

    #[test]
    fn a_wake_before_the_sleep_is_not_lost() {
        let idle = Idle::default();
        idle.wake();
        assert!(quick(|| idle.sleep(Duration::from_secs(30))) < Duration::from_secs(5));
    }

    #[test]
    fn the_sleep_ends_with_its_time_when_nothing_wakes_it() {
        let idle = Idle::default();
        let took = quick(|| idle.sleep(Duration::from_millis(80)));
        assert!(took >= Duration::from_millis(80), "{took:?}");
        assert!(took < Duration::from_secs(5), "{took:?}");
    }

    #[test]
    fn one_wake_ends_one_sleep() {
        let idle = Idle::default();
        idle.wake();
        idle.sleep(Duration::from_secs(30));
        let took = quick(|| idle.sleep(Duration::from_millis(80)));
        assert!(
            took >= Duration::from_millis(80),
            "the wake was used up: {took:?}"
        );
    }

    #[test]
    fn a_thread_can_wake_the_sleep() {
        let idle = Idle::default();
        let other = idle.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(30));
            other.wake();
        });
        assert!(quick(|| idle.sleep(Duration::from_secs(30))) < Duration::from_secs(5));
    }

    #[test]
    fn a_repaint_request_on_the_context_wakes_the_loop() {
        // What a thread asks of egui while no window exists (the recent captures, a Quit answer).
        let idle = Idle::default();
        let ctx = context(&idle);
        let other = ctx.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(30));
            other.request_repaint();
        });
        assert!(quick(|| idle.sleep(Duration::from_secs(30))) < Duration::from_secs(5));
    }

    #[test]
    fn a_repaint_later_does_not_wake_the_loop_now() {
        // The loop wakes on its own often enough; "in 10 s" must not spin it.
        let idle = Idle::default();
        let ctx = context(&idle);
        ctx.request_repaint_after(Duration::from_secs(10));
        let took = quick(|| idle.sleep(Duration::from_millis(80)));
        assert!(took >= Duration::from_millis(80), "{took:?}");
    }

    #[test]
    fn a_pass_runs_with_the_time_it_is_given() {
        let ctx = context(&Idle::default());
        assert_eq!(pass(&ctx, 12.5, |c| c.input(|i| i.time)), 12.5);
        assert_eq!(pass(&ctx, 40.0, |c| c.input(|i| i.time)), 40.0);
    }

    #[test]
    fn a_hidden_start_waits_for_a_tray_then_shows_the_window() {
        // At login the panel may come up after the app; with none, the app must not stay unseen.
        let at = |secs| Duration::from_secs(secs);
        assert_eq!(tray_wait(at(0), true), TrayWait::Done);
        assert_eq!(tray_wait(at(5), false), TrayWait::Keep);
        assert_eq!(tray_wait(TRAY_WAIT - at(1), false), TrayWait::Keep);
        assert_eq!(tray_wait(TRAY_WAIT, false), TrayWait::GiveUp);
        assert_eq!(
            tray_wait(TRAY_WAIT + at(9), true),
            TrayWait::Done,
            "a late tray still wins"
        );
    }

    #[test]
    fn the_next_step_follows_what_the_resident_wants() {
        assert_eq!(next(false, false), Next::Wait, "no window wanted: sleep");
        assert_eq!(next(true, false), Next::Window, "a window wanted: open it");
        assert_eq!(next(false, true), Next::Quit);
        assert_eq!(next(true, true), Next::Quit, "quitting wins over a window");
    }
}
