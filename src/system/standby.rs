//! The warm capture: a `--capture-standby` process keeps a window and a GL context ready, invisible and
//! off screen, so that PrtSc skips the ~200 ms they cost. The resident asks it over a local socket; it
//! serves one capture (it turns into the capture window, then the editor or the notice, like any
//! capture process) and the resident starts the next one. Every failure falls back to a `--capture` process.

use crate::system::hotkeys::Shot;
use crate::system::instance::{self, CaptureSlot};
use interprocess::local_socket::Stream;
use interprocess::local_socket::prelude::*;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

/// How long after one standby is used up (or missing) the resident starts the next: past the first
/// frames of the capture it serves, so the two do not compete for the CPU.
pub const RESPAWN_AFTER: Duration = Duration::from_millis(1500);
/// A standby started less than this ago is still booting: no second one is started meanwhile.
pub const WARM_UP: Duration = Duration::from_secs(10);
/// The wait for a window that was just minimized to leave the picture (`--wait`).
pub const WAIT_MS: u32 = 300;
/// The resident does not wait longer than this for the standby's answer.
const ANSWER_WITHIN: Duration = Duration::from_millis(250);
/// How often an idle standby checks that its resident still runs.
const WATCH_EVERY: Duration = Duration::from_secs(5);

/// Where the standby is kept: Windows for now (Wayland cannot place a window, and the other
/// systems are not measured yet).
pub fn supported() -> bool {
    cfg!(windows)
}

/// Whether this run keeps a standby: supported, and `TAKESHOT_NO_STANDBY` not set.
pub fn enabled() -> bool {
    supported() && std::env::var_os("TAKESHOT_NO_STANDBY").is_none()
}

/// Whether the standby may serve a capture of `shot`: the area capture of PrtSc. A full-screen
/// capture saves at once or opens the editor, which need windows of another shape.
pub fn serves(shot: Shot, enabled: bool) -> bool {
    enabled && shot == Shot::Area
}

/// The socket the standby of the resident `base` listens on.
pub fn socket(base: &str) -> String {
    format!("{base}-standby")
}

/// What the resident asks of the standby.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Request {
    /// Time to wait before grabbing the screen.
    pub delay_ms: u32,
}

pub fn format_request(r: Request) -> String {
    format!("capture area {}\n", r.delay_ms)
}

pub fn parse_request(line: &str) -> Option<Request> {
    let delay_ms = line.trim().strip_prefix("capture area ")?.parse().ok()?;
    Some(Request { delay_ms })
}

/// What the standby answered.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Outcome {
    /// It took the capture.
    Taken,
    /// A capture is on screen: this request is ignored, as a `--capture` process would exit.
    Busy,
    /// Nobody answered in time: start a `--capture` process.
    Absent,
}

fn parse_reply(line: &str) -> Outcome {
    match line.trim() {
        "ok" => Outcome::Taken,
        "busy" => Outcome::Busy,
        _ => Outcome::Absent,
    }
}

/// Asks the standby of the resident `base` to capture. A standby that does not answer in time
/// counts as missing, so a stuck one never holds up PrtSc.
pub fn ask(base: &str, r: Request) -> Outcome {
    allow_foreground();
    let name = socket(base);
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(exchange(&name, r));
    });
    rx.recv_timeout(ANSWER_WITHIN).unwrap_or(Outcome::Absent)
}

/// The standby did not get the last key, so Windows could keep it from taking the foreground and
/// the capture would not hear Esc. The resident, which sees the key first, lets it take it.
#[cfg(windows)]
fn allow_foreground() {
    // ASFW_ANY: the standby's process id is not known here.
    unsafe {
        windows_sys::Win32::UI::WindowsAndMessaging::AllowSetForegroundWindow(u32::MAX);
    }
}

#[cfg(not(windows))]
fn allow_foreground() {}

fn exchange(name: &str, r: Request) -> Outcome {
    let Ok(mut s) = instance::connect(name) else {
        return Outcome::Absent;
    };
    if s.write_all(format_request(r).as_bytes())
        .and_then(|()| s.flush())
        .is_err()
    {
        return Outcome::Absent;
    }
    let mut line = String::new();
    match BufReader::new(s).take(16).read_line(&mut line) {
        Ok(_) => parse_reply(&line),
        Err(_) => Outcome::Absent,
    }
}

/// A request the standby accepted, with the capture slot it took.
pub struct Incoming {
    pub req: Request,
    /// When the request came in, for the timing log (`mark`).
    pub at: Instant,
    pub slot: CaptureSlot,
}

/// The standby's end: it listens for the resident's request.
pub struct Warm {
    rx: mpsc::Receiver<Incoming>,
    used: Arc<AtomicBool>,
}

impl Warm {
    /// Listens on the socket of `base`; `wake` runs when a request comes in. After one request the
    /// socket is closed, so the next PrtSc never reaches a process that is busy being the editor.
    pub fn start(base: &str, wake: impl Fn() + Send + Sync + 'static) -> io::Result<Self> {
        let name = socket(base);
        let listener = instance::create(&name, true)?;
        let (tx, rx) = mpsc::channel();
        let used = Arc::new(AtomicBool::new(false));
        let wake = Arc::new(wake);
        let (base, flag) = (base.to_owned(), used.clone());
        std::thread::Builder::new()
            .name("takeshot-standby".into())
            .spawn(move || {
                for conn in listener.incoming() {
                    if flag.load(Ordering::SeqCst) {
                        break;
                    }
                    let Ok(conn) = conn else {
                        std::thread::sleep(Duration::from_millis(50));
                        continue;
                    };
                    let job = Job {
                        base: base.clone(),
                        name: name.clone(),
                        used: flag.clone(),
                        tx: tx.clone(),
                        wake: wake.clone(),
                    };
                    std::thread::spawn(move || job.answer(conn));
                }
            })?;
        Ok(Self { rx, used })
    }

    /// The request that came in, if one did.
    pub fn take(&self) -> Option<Incoming> {
        self.rx.try_recv().ok()
    }

    /// Ends this process when the resident `base` is gone while it still waits: a resident that
    /// crashed or was killed cannot ask it to close.
    pub fn leave_with_resident(&self, base: &str) {
        let (base, used) = (base.to_owned(), self.used.clone());
        std::thread::spawn(move || {
            loop {
                std::thread::sleep(WATCH_EVERY);
                if used.load(Ordering::SeqCst) {
                    return;
                }
                if instance::connect(&base).is_err() {
                    std::process::exit(0);
                }
            }
        });
    }
}

/// What one connection to the standby needs to be answered.
struct Job {
    base: String,
    name: String,
    used: Arc<AtomicBool>,
    tx: mpsc::Sender<Incoming>,
    wake: Arc<dyn Fn() + Send + Sync>,
}

impl Job {
    /// Reads the request; answers `ok` and hands it to the app when the capture slot is free, `busy`
    /// when a capture is on screen. Each connection has its own thread and at most 64 bytes, so a
    /// client that never writes cannot hold up the next one.
    fn answer(self, conn: Stream) {
        let mut reader = BufReader::new(conn);
        let mut line = String::new();
        if (&mut reader).take(64).read_line(&mut line).is_err() {
            return;
        }
        let Some(req) = parse_request(&line) else {
            return;
        };
        let taken = instance::capture_slot(&self.base).map(|slot| Incoming {
            req,
            at: Instant::now(),
            slot,
        });
        let reply: &[u8] = if taken.is_some() { b"ok\n" } else { b"busy\n" };
        let served = taken.is_some();
        if let Some(incoming) = taken {
            self.used.store(true, Ordering::SeqCst);
            let _ = self.tx.send(incoming);
            (self.wake)();
        }
        let conn = reader.get_mut();
        let _ = conn.write_all(reply).and_then(|()| conn.flush());
        if served {
            // The listener is blocked in `accept`: one more connection lets it see the flag and end.
            let _ = instance::connect(&self.name);
        }
    }
}

/// Decides when a standby is started: one at a time. The next waits for the last one to boot
/// (`WARM_UP`), unless that one was used up by a capture, which is when the next is wanted.
pub struct Keeper {
    last: Option<Instant>,
}

impl Keeper {
    pub const fn new() -> Self {
        Self { last: None }
    }

    /// The standby that was started has served a capture.
    pub fn used_up(&mut self) {
        self.last = None;
    }

    /// Whether to start a standby `now`.
    pub fn start_now(&mut self, now: Instant) -> bool {
        if self.last.is_none_or(|t| now.duration_since(t) >= WARM_UP) {
            self.last = Some(now);
            return true;
        }
        false
    }
}

static KEEPER: Mutex<Keeper> = Mutex::new(Keeper::new());

/// Starts a standby in `after`, unless one started lately and is still booting.
pub fn keep_warm(after: Duration) {
    if !enabled() {
        return;
    }
    std::thread::spawn(move || {
        std::thread::sleep(after);
        let Ok(mut keeper) = KEEPER.lock() else {
            return;
        };
        if keeper.start_now(Instant::now())
            && crate::system::actions::spawn_self(&["--capture-standby"]).is_err()
        {
            keeper.used_up();
        }
    });
}

/// The standby served a capture: the next starts in `RESPAWN_AFTER`, however lately the last did.
pub fn replace_used() {
    if let Ok(mut keeper) = KEEPER.lock() {
        keeper.used_up();
    }
    keep_warm(RESPAWN_AFTER);
}

/// Appends `label` and the milliseconds since `since` to the file `TAKESHOT_STANDBY_LOG` names:
/// the breakdown of a warm capture for timing runs. Without the variable it does nothing.
pub fn mark(since: Instant, label: &str) {
    if let Some(path) = std::env::var_os("TAKESHOT_STANDBY_LOG")
        && let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
    {
        let ms = since.elapsed().as_secs_f64() * 1000.0;
        let _ = writeln!(f, "{ms:>7.1} ms  {label}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unique(test: &str) -> String {
        format!("takeshot-test-{}-standby-{test}", std::process::id())
    }

    #[test]
    fn only_the_area_capture_goes_to_the_standby() {
        assert!(serves(Shot::Area, true));
        assert!(!serves(Shot::Full, true), "full screen needs other windows");
        assert!(!serves(Shot::Area, false), "switched off");
    }

    #[test]
    fn a_request_round_trips_through_the_wire_format() {
        for ms in [0, WAIT_MS, 3000] {
            let r = Request { delay_ms: ms };
            assert_eq!(parse_request(&format_request(r)), Some(r));
        }
        assert_eq!(parse_request("capture full 0\n"), None);
        assert_eq!(parse_request("capture area soon\n"), None);
        assert_eq!(parse_request(""), None);
    }

    #[test]
    fn nobody_listening_is_absent() {
        let name = unique("absent");
        assert_eq!(ask(&name, Request { delay_ms: 0 }), Outcome::Absent);
    }

    #[test]
    fn a_request_reaches_the_standby_and_takes_the_slot() {
        let base = unique("taken");
        let woken = Arc::new(AtomicBool::new(false));
        let w = woken.clone();
        let warm = Warm::start(&base, move || w.store(true, Ordering::SeqCst)).expect("listens");
        assert_eq!(ask(&base, Request { delay_ms: 300 }), Outcome::Taken);
        let deadline = Instant::now() + Duration::from_secs(2);
        let got = loop {
            if let Some(i) = warm.take() {
                break i;
            }
            assert!(Instant::now() < deadline, "the request never arrived");
            std::thread::sleep(Duration::from_millis(5));
        };
        assert_eq!(got.req, Request { delay_ms: 300 });
        assert!(woken.load(Ordering::SeqCst), "the app is woken");
        assert!(
            instance::capture_slot(&base).is_none(),
            "the slot is held while the capture lives"
        );
        drop(got);
        assert!(
            instance::capture_slot(&base).is_some(),
            "and free once it goes"
        );
    }

    #[test]
    fn a_standby_serves_one_capture_and_then_leaves() {
        let base = unique("once");
        let warm = Warm::start(&base, || {}).expect("listens");
        assert_eq!(ask(&base, Request { delay_ms: 0 }), Outcome::Taken);
        let deadline = Instant::now() + Duration::from_secs(2);
        // The used-up standby stops answering, so the next PrtSc reaches the one that replaces it
        // (or starts a capture process), never a process that has become the editor.
        while ask(&base, Request { delay_ms: 0 }) != Outcome::Absent {
            assert!(
                Instant::now() < deadline,
                "still listening after its capture"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
        drop(warm);
    }

    #[test]
    fn a_capture_on_screen_makes_the_standby_answer_busy_and_keep_waiting() {
        let base = unique("busy");
        let warm = Warm::start(&base, || {}).expect("listens");
        let on_screen = instance::capture_slot(&base).expect("slot free");
        assert_eq!(ask(&base, Request { delay_ms: 0 }), Outcome::Busy);
        assert!(warm.take().is_none(), "a busy request is not delivered");
        drop(on_screen);
        assert_eq!(
            ask(&base, Request { delay_ms: 0 }),
            Outcome::Taken,
            "still there once the other capture lets go"
        );
    }

    #[test]
    fn a_standby_is_not_started_twice_while_the_first_boots() {
        let t0 = Instant::now();
        let mut keeper = Keeper::new();
        assert!(keeper.start_now(t0), "none yet");
        assert!(
            !keeper.start_now(t0 + Duration::from_secs(1)),
            "the first is still booting"
        );
        assert!(
            keeper.start_now(t0 + WARM_UP),
            "it never answered: lost for good, replaced"
        );
    }

    #[test]
    fn a_standby_that_served_a_capture_is_replaced_without_waiting() {
        // PrtSc twice within the warm-up time: the standby of the first was used up long ago.
        let t0 = Instant::now();
        let mut keeper = Keeper::new();
        assert!(keeper.start_now(t0));
        keeper.used_up();
        assert!(
            keeper.start_now(t0 + Duration::from_secs(4)),
            "the next one starts at once"
        );
        assert!(
            !keeper.start_now(t0 + Duration::from_secs(5)),
            "but only one"
        );
    }
}
