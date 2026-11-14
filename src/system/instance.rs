//! Single instance: the first Takeshot process owns a local socket; a second launch asks it to show a page and exits.
//! Captures are one at a time too: a capture whose selection is on screen holds a file lock.
//! Quit: the editors, pinned pictures and the capture register with the resident and listen on a
//! socket of their own, so the tray's Quit can ask each whether it may close (an editor with
//! changes asks its user first) and then tell all to close, or to stay after a Cancel.

use super::log;
#[cfg(target_os = "macos")]
use interprocess::local_socket::GenericFilePath;
#[cfg(not(target_os = "macos"))]
use interprocess::local_socket::GenericNamespaced;
use interprocess::local_socket::{ListenerOptions, Name, Stream, prelude::*};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::sync::Arc;
use std::time::Duration;

/// The main window pages a second launch or the tray can ask for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Page {
    Start,
    Style,
    Settings,
    About,
}

impl Page {
    pub fn as_str(self) -> &'static str {
        match self {
            Page::Start => "start",
            Page::Style => "style",
            Page::Settings => "settings",
            Page::About => "about",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "start" => Some(Page::Start),
            "style" => Some(Page::Style),
            "settings" => Some(Page::Settings),
            "about" => Some(Page::About),
            _ => None,
        }
    }
}

/// The socket name for this user: one resident per user session.
pub fn endpoint() -> String {
    let user = std::env::var("USERNAME")
        .or_else(|_| std::env::var("USER"))
        .unwrap_or_default();
    let safe: String = user
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .collect();
    format!("takeshot-{}", if safe.is_empty() { "user" } else { &safe })
}

/// A named pipe on Windows and an abstract socket on Linux; on macOS a file in the per-user temp
/// folder, since the shared `/tmp` would let other users take the name.
#[cfg(not(target_os = "macos"))]
fn socket_name(name: &str) -> io::Result<Name<'static>> {
    format!("{name}.sock").to_ns_name::<GenericNamespaced>()
}

#[cfg(target_os = "macos")]
fn socket_name(name: &str) -> io::Result<Name<'static>> {
    std::env::temp_dir()
        .join(format!("{name}.sock"))
        .to_fs_name::<GenericFilePath>()
}

/// The name is taken: Unix says "address in use", Windows says "access denied" for an existing pipe.
fn in_use(e: &io::Error) -> bool {
    matches!(
        e.kind(),
        io::ErrorKind::AddrInUse | io::ErrorKind::PermissionDenied
    )
}

pub(crate) fn connect(name: &str) -> io::Result<Stream> {
    Stream::connect(socket_name(name)?)
}

pub(crate) fn create(
    name: &str,
    overwrite: bool,
) -> io::Result<interprocess::local_socket::Listener> {
    ListenerOptions::new()
        .name(socket_name(name)?)
        .try_overwrite(overwrite)
        .create_sync()
}

/// The first process's end of the socket.
pub struct Listener(interprocess::local_socket::Listener);

pub enum Claim {
    /// No other Takeshot runs: this process is the resident.
    Primary(Listener),
    /// A resident answers: ask it to show itself and exit.
    Secondary,
    /// The socket cannot be used at all: run anyway, without single instance.
    Alone,
}

/// Becomes the resident, or finds the one that runs. A resident that crashed and left its socket
/// file behind (file-path names) does not block the next launch.
pub fn claim(name: &str) -> Claim {
    for _ in 0..5 {
        if connect(name).is_ok() {
            return Claim::Secondary;
        }
        match create(name, false) {
            Ok(l) => return Claim::Primary(Listener(l)),
            // Another launch is starting right now, or a dead resident left its name behind.
            Err(e) if in_use(&e) => std::thread::sleep(Duration::from_millis(60)),
            Err(e) => return alone(e),
        }
    }
    // Still nobody answers: the name is a dead resident's leftover file, which can be reclaimed.
    if connect(name).is_ok() {
        return Claim::Secondary;
    }
    match create(name, true) {
        Ok(l) => Claim::Primary(Listener(l)),
        Err(e) => alone(e),
    }
}

fn alone(e: io::Error) -> Claim {
    log::line(format_args!("running without single instance: {e}"));
    Claim::Alone
}

/// Held by the capture whose selection is on screen; dropping it lets the next capture start.
pub struct CaptureSlot(#[allow(dead_code)] Option<std::fs::File>);

/// Takes the capture slot, or `None` when another capture holds it (that PrtSc is ignored). The
/// lock goes with the process, so a crashed capture never blocks the next. Where the lock file
/// cannot be used, captures run unguarded.
pub fn capture_slot(name: &str) -> Option<CaptureSlot> {
    let path = std::env::temp_dir().join(format!("{name}.capture.lock"));
    let file = match std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&path)
    {
        Ok(f) => f,
        Err(e) => {
            log::line(format_args!("captures not guarded: {e}"));
            return Some(CaptureSlot(None));
        }
    };
    match file.try_lock() {
        Ok(()) => Some(CaptureSlot(Some(file))),
        Err(std::fs::TryLockError::WouldBlock) => None,
        Err(std::fs::TryLockError::Error(e)) => {
            log::line(format_args!("captures not guarded: {e}"));
            Some(CaptureSlot(None))
        }
    }
}

/// What a second launch asks the running resident for: Home, or nothing when it was started
/// minimized (start with the system must not pop the window up).
pub fn second_launch_page(minimized: bool) -> Option<Page> {
    (!minimized).then_some(Page::Start)
}

/// Connects to the resident, retrying briefly: it may be between connections.
fn connect_retrying(name: &str) -> io::Result<Stream> {
    let mut last = io::Error::other("no attempt");
    for _ in 0..10 {
        match connect(name) {
            Ok(s) => return Ok(s),
            Err(e) => {
                last = e;
                std::thread::sleep(Duration::from_millis(50));
            }
        }
    }
    Err(last)
}

/// Asks the resident to show `page`.
pub fn send_show(name: &str, page: Page) -> io::Result<()> {
    let mut s = connect_retrying(name)?;
    s.write_all(format!("show {}\n", page.as_str()).as_bytes())?;
    s.flush()
}

/// A line a client sends the resident.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Request {
    /// A second launch: show this page.
    Show(Page),
    /// A window of its own (editor, pinned picture, capture) started: its process id.
    Child(u32),
    /// `takeshot --quit`: quit the way the tray's Quit does, and say how it went.
    Quit,
}

/// Reads one request line.
pub fn parse_request(line: &str) -> Option<Request> {
    let line = line.trim();
    if let Some(p) = line.strip_prefix("show ") {
        return Page::parse(p).map(Request::Show);
    }
    if line == "quit" {
        return Some(Request::Quit);
    }
    line.strip_prefix("child ")
        .and_then(|pid| pid.parse().ok())
        .map(Request::Child)
}

/// How a `takeshot --quit` ended.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum QuitReply {
    /// No resident answers: nothing to quit.
    NotRunning,
    /// The resident quit (every window agreed).
    Quit,
    /// A window's user chose Cancel: everything stays open.
    Stay,
    /// The resident did not say in time (a window may still be asking its user).
    NoAnswer,
}

impl QuitReply {
    /// The process exit code: 0 when nothing is left running, 1 after a Cancel, 2 on no answer.
    pub fn exit_code(self) -> i32 {
        match self {
            QuitReply::NotRunning | QuitReply::Quit => 0,
            QuitReply::Stay => 1,
            QuitReply::NoAnswer => 2,
        }
    }
}

/// Asks the resident to quit and waits up to `wait` for how it went. The resident answers only a
/// Cancel (`stay`); when it quits, its end of the connection just closes, which is also what a
/// resident that died meanwhile looks like.
pub fn send_quit(name: &str, wait: Duration) -> QuitReply {
    let Ok(mut s) = connect_retrying(name) else {
        return QuitReply::NotRunning;
    };
    if s.write_all(b"quit\n").and_then(|_| s.flush()).is_err() {
        return QuitReply::Quit;
    }
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut line = String::new();
        let _ = BufReader::new(s).read_line(&mut line);
        let _ = tx.send(line);
    });
    match rx.recv_timeout(wait) {
        Ok(line) if line.trim() == "stay" => QuitReply::Stay,
        Ok(_) => QuitReply::Quit,
        Err(_) => QuitReply::NoAnswer,
    }
}

/// The socket name a child process listens on for the resident's Quit.
pub fn child_endpoint(base: &str, pid: u32) -> String {
    format!("{base}-child-{pid}")
}

/// Tells the resident (when one runs) that this process is a window of its own, so its Quit asks
/// it to close too.
pub fn register_child(base: &str, pid: u32) {
    if let Ok(mut s) = connect_retrying(base) {
        let _ = s.write_all(format!("child {pid}\n").as_bytes());
        let _ = s.flush();
    }
}

/// How a child answered the resident's Quit.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum QuitAnswer {
    /// It may close (or was gone already).
    Agreed,
    /// Its user chose Cancel: everything stays open.
    Cancelled,
}

/// Listens on `name` for the resident's Quit, on a background thread. A Quit comes in two steps:
/// the question, where `on_ask` gets a sender for the answer (`true` agrees, `false` cancels) that
/// goes back once the user has chosen; then, once every window has answered, the outcome, where
/// `on_close` runs only if all agreed. Nothing closes on the question alone, so one Cancel keeps
/// every window open.
pub fn listen_for_quit(
    name: &str,
    on_ask: impl Fn(std::sync::mpsc::Sender<bool>) + Send + Sync + 'static,
    on_close: impl Fn() + Send + Sync + 'static,
) -> io::Result<()> {
    let listener = create(name, true)?;
    let on_ask = Arc::new(on_ask);
    let on_close = Arc::new(on_close);
    std::thread::Builder::new()
        .name("takeshot-quit".into())
        .spawn(move || {
            for conn in listener.incoming() {
                let Ok(conn) = conn else {
                    std::thread::sleep(Duration::from_millis(50));
                    continue;
                };
                let (on_ask, on_close) = (on_ask.clone(), on_close.clone());
                std::thread::spawn(move || {
                    let mut reader = BufReader::new(conn);
                    let mut line = String::new();
                    if (&mut reader).take(16).read_line(&mut line).is_err() || line.trim() != "quit"
                    {
                        return;
                    }
                    let (tx, rx) = std::sync::mpsc::channel();
                    on_ask(tx);
                    // An answer that never comes keeps the window open.
                    let agrees = rx.recv().unwrap_or(false);
                    let conn = reader.get_mut();
                    let said = conn
                        .write_all(if agrees { b"yes\n" } else { b"no\n" })
                        .and_then(|_| conn.flush());
                    if said.is_err() || !agrees {
                        return;
                    }
                    line.clear();
                    if (&mut reader).take(16).read_line(&mut line).is_ok() && line.trim() == "close"
                    {
                        on_close();
                    }
                });
            }
        })?;
    Ok(())
}

/// The resident's side of one child's Quit: its answer, and the line to tell it the outcome.
pub struct QuitAsk {
    pub answer: QuitAnswer,
    conn: Option<Stream>,
}

impl QuitAsk {
    /// Tells the child to close (`true`) or to stay.
    pub fn finish(self, close: bool) {
        if let Some(mut c) = self.conn {
            let _ = c.write_all(if close { b"close\n" } else { b"stay\n" });
            let _ = c.flush();
        }
    }
}

/// Asks the child listening on `name` whether it may close and waits for its answer (its user may
/// be choosing); a child that is gone, or goes meanwhile, counts as agreeing.
pub fn ask_to_quit(name: &str) -> QuitAsk {
    let gone = QuitAsk {
        answer: QuitAnswer::Agreed,
        conn: None,
    };
    let Ok(mut s) = connect(name) else {
        return gone;
    };
    if s.write_all(b"quit\n").and_then(|_| s.flush()).is_err() {
        return gone;
    }
    let mut reader = BufReader::new(s);
    let mut line = String::new();
    let answer = match reader.read_line(&mut line) {
        Ok(_) if line.trim() == "yes" => QuitAnswer::Agreed,
        Ok(_) if line.trim() == "no" => QuitAnswer::Cancelled,
        _ => return gone,
    };
    QuitAsk {
        answer,
        conn: Some(reader.into_inner()),
    }
}

/// Where a Quit stands.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum QuitOutcome {
    /// A child has not answered yet.
    Waiting,
    /// Every child agreed: they close and the resident quits.
    Quit,
    /// A child's user chose Cancel: the resident stays.
    Stay,
}

/// The answers to one Quit, child by child.
#[derive(Clone, Debug)]
pub struct QuitRound {
    left: usize,
    cancelled: bool,
}

impl QuitRound {
    pub fn new(children: usize) -> Self {
        Self {
            left: children,
            cancelled: false,
        }
    }

    pub fn answer(&mut self, a: QuitAnswer) {
        self.left = self.left.saturating_sub(1);
        self.cancelled |= a == QuitAnswer::Cancelled;
    }

    pub fn outcome(&self) -> QuitOutcome {
        match (self.left, self.cancelled) {
            (0, false) => QuitOutcome::Quit,
            (0, true) => QuitOutcome::Stay,
            _ => QuitOutcome::Waiting,
        }
    }
}

/// A Quit under way: every child is asked at once, and only once all have answered is each told
/// to close (all agreed) or to stay (one cancelled).
pub struct QuitRun {
    round: QuitRound,
    answers: std::sync::mpsc::Receiver<QuitAsk>,
    asks: Vec<QuitAsk>,
}

impl QuitRun {
    /// Asks the children listening on `names`; `wake` runs as each answer comes in.
    pub fn start(names: Vec<String>, wake: impl Fn() + Send + Sync + 'static) -> Self {
        let (tx, answers) = std::sync::mpsc::channel();
        let wake = Arc::new(wake);
        for name in &names {
            let (tx, name, wake) = (tx.clone(), name.clone(), wake.clone());
            std::thread::spawn(move || {
                let _ = tx.send(ask_to_quit(&name));
                wake();
            });
        }
        Self {
            round: QuitRound::new(names.len()),
            answers,
            asks: Vec::new(),
        }
    }

    /// Takes the answers that came in; once the outcome is known, tells every child.
    pub fn poll(&mut self) -> QuitOutcome {
        while let Ok(a) = self.answers.try_recv() {
            self.round.answer(a.answer);
            self.asks.push(a);
        }
        let out = self.round.outcome();
        if out != QuitOutcome::Waiting {
            for a in self.asks.drain(..) {
                a.finish(out == QuitOutcome::Quit);
            }
        }
        out
    }
}

impl Listener {
    /// Answers second launches on a background thread for the rest of the process.
    #[cfg(test)]
    pub fn spawn(self, on_show: impl Fn(Page) + Send + Sync + 'static) {
        self.serve(on_show, |_| {}, |_| {});
    }

    /// Answers second launches (`on_show`), children registering (`on_child`) and `--quit`
    /// (`on_quit`, which gets a sender: a `()` through it says the resident stays) on a background
    /// thread for the rest of the process. Each connection gets its own short thread and at most
    /// 64 bytes, so a client that never writes cannot hold up the next launch.
    pub fn serve(
        self,
        on_show: impl Fn(Page) + Send + Sync + 'static,
        on_child: impl Fn(u32) + Send + Sync + 'static,
        on_quit: impl Fn(std::sync::mpsc::Sender<()>) + Send + Sync + 'static,
    ) {
        let listener = self.0;
        let on_show = Arc::new(on_show);
        let on_child = Arc::new(on_child);
        let on_quit = Arc::new(on_quit);
        let spawned = std::thread::Builder::new()
            .name("takeshot-instance".into())
            .spawn(move || {
                for conn in listener.incoming() {
                    let Ok(conn) = conn else {
                        std::thread::sleep(Duration::from_millis(50));
                        continue;
                    };
                    let (on_show, on_child, on_quit) =
                        (on_show.clone(), on_child.clone(), on_quit.clone());
                    std::thread::spawn(move || {
                        let mut line = String::new();
                        let mut reader = BufReader::new(conn.take(64));
                        if reader.read_line(&mut line).is_ok() {
                            match parse_request(&line) {
                                Some(Request::Show(page)) => on_show(page),
                                Some(Request::Child(pid)) => on_child(pid),
                                Some(Request::Quit) => {
                                    let (tx, rx) = std::sync::mpsc::channel();
                                    on_quit(tx);
                                    // Only a Cancel is answered: a resident that quits closes the
                                    // connection with its process.
                                    if rx.recv().is_ok() {
                                        let mut conn = reader.into_inner().into_inner();
                                        let _ =
                                            conn.write_all(b"stay\n").and_then(|_| conn.flush());
                                    }
                                }
                                None => {}
                            }
                        }
                    });
                }
            });
        if let Err(e) = spawned {
            log::line(format_args!("cannot answer second launches: {e}"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::channel;
    use std::time::Duration;

    fn unique(test: &str) -> String {
        format!("takeshot-test-{}-{test}", std::process::id())
    }

    #[test]
    fn second_claim_is_secondary_and_its_show_reaches_the_first() {
        let name = unique("second");
        let Claim::Primary(listener) = claim(&name) else {
            panic!("the first claim must be primary");
        };
        let (tx, rx) = channel();
        listener.spawn(move |page| {
            let _ = tx.send(page);
        });
        assert!(matches!(claim(&name), Claim::Secondary));
        send_show(&name, Page::Settings).unwrap();
        assert_eq!(rx.recv_timeout(Duration::from_secs(2)), Ok(Page::Settings));
    }

    #[test]
    fn a_dead_primary_does_not_block_the_next_claim() {
        let name = unique("dead");
        let Claim::Primary(listener) = claim(&name) else {
            panic!("the first claim must be primary");
        };
        drop(listener);
        assert!(matches!(claim(&name), Claim::Primary(_)));
    }

    #[test]
    fn launches_racing_for_the_name_leave_exactly_one_primary() {
        let name = unique("race");
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(4));
        let threads: Vec<_> = (0..4)
            .map(|_| {
                let (name, barrier) = (name.clone(), barrier.clone());
                std::thread::spawn(move || {
                    barrier.wait();
                    match claim(&name) {
                        Claim::Primary(l) => {
                            l.spawn(|_| {});
                            "primary"
                        }
                        Claim::Secondary => "secondary",
                        Claim::Alone => "alone",
                    }
                })
            })
            // Collect first: joining inside the same lazy chain would wait at the barrier forever.
            .collect::<Vec<_>>()
            .into_iter()
            .map(|t| t.join().unwrap())
            .collect::<Vec<_>>();
        let count = |k: &str| threads.iter().filter(|r| **r == k).count();
        assert_eq!(count("primary"), 1, "{threads:?}");
        assert_eq!(count("alone"), 0, "{threads:?}");
    }

    #[test]
    fn a_silent_client_does_not_block_the_next_show() {
        let name = unique("silent");
        let Claim::Primary(listener) = claim(&name) else {
            panic!("the first claim must be primary");
        };
        let (tx, rx) = channel();
        listener.spawn(move |page| {
            let _ = tx.send(page);
        });
        // A client that connects and never writes a line.
        let _mute = connect(&name).unwrap();
        send_show(&name, Page::Style).unwrap();
        assert_eq!(rx.recv_timeout(Duration::from_secs(2)), Ok(Page::Style));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn a_stale_socket_file_is_reclaimed() {
        let name = unique("stale");
        let path = std::env::temp_dir().join(format!("{name}.sock"));
        // std leaves the file behind when its listener drops, like a crashed resident.
        drop(std::os::unix::net::UnixListener::bind(&path).unwrap());
        assert!(path.exists());
        assert!(matches!(claim(&name), Claim::Primary(_)));
    }

    #[test]
    fn requests_read_show_child_and_quit() {
        assert_eq!(
            parse_request("show settings\n"),
            Some(Request::Show(Page::Settings))
        );
        assert_eq!(parse_request("child 4242\n"), Some(Request::Child(4242)));
        assert_eq!(parse_request("child x"), None);
        assert_eq!(parse_request("quit\n"), Some(Request::Quit));
        assert_eq!(parse_request("quit now"), None);
    }

    #[test]
    fn a_child_registers_with_the_resident() {
        let name = unique("register");
        let Claim::Primary(listener) = claim(&name) else {
            panic!("the first claim must be primary");
        };
        let (tx, rx) = channel();
        listener.serve(
            |_| {},
            move |pid| {
                let _ = tx.send(pid);
            },
            |_| {},
        );
        register_child(&name, 4242);
        assert_eq!(rx.recv_timeout(Duration::from_secs(2)), Ok(4242));
    }

    /// A resident whose `--quit` handler runs `on_quit` with the sender the connection waits on.
    fn quit_server(
        name: &str,
        on_quit: impl Fn(std::sync::mpsc::Sender<()>) + Send + Sync + 'static,
    ) {
        let Claim::Primary(listener) = claim(name) else {
            panic!("the first claim must be primary");
        };
        listener.serve(|_| {}, |_| {}, on_quit);
    }

    #[test]
    fn a_quit_from_outside_says_how_it_ended() {
        let soon = Duration::from_secs(5);
        // No resident: nothing to quit.
        assert_eq!(
            send_quit(&unique("quit-nobody"), Duration::from_secs(1)),
            QuitReply::NotRunning
        );
        // A window's user cancelled: the resident says it stays.
        let stays = unique("quit-stay");
        quit_server(&stays, |stay| {
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(100));
                let _ = stay.send(());
            });
        });
        assert_eq!(send_quit(&stays, soon), QuitReply::Stay);
        // The resident goes: nobody answers, the connection just closes.
        let goes = unique("quit-go");
        quit_server(&goes, drop);
        assert_eq!(send_quit(&goes, soon), QuitReply::Quit);
        // Still asking when the wait is over.
        let hung = unique("quit-hung");
        let held = Arc::new(std::sync::Mutex::new(Vec::new()));
        quit_server(&hung, move |stay| held.lock().unwrap().push(stay));
        assert_eq!(
            send_quit(&hung, Duration::from_millis(300)),
            QuitReply::NoAnswer
        );
    }

    #[test]
    fn the_exit_code_tells_the_installer_how_the_quit_ended() {
        assert_eq!(QuitReply::NotRunning.exit_code(), 0);
        assert_eq!(QuitReply::Quit.exit_code(), 0);
        assert_eq!(QuitReply::Stay.exit_code(), 1);
        assert_eq!(QuitReply::NoAnswer.exit_code(), 2);
    }

    #[test]
    fn a_child_answers_then_closes_only_when_told() {
        for (n, (agrees, close, closes)) in [
            (true, true, true),
            (true, false, false),
            (false, false, false),
        ]
        .into_iter()
        .enumerate()
        {
            let name = unique(&format!("child-{n}"));
            let (closed, was_closed) = channel();
            listen_for_quit(
                &name,
                move |reply| {
                    let _ = reply.send(agrees);
                },
                move || {
                    let _ = closed.send(());
                },
            )
            .unwrap();
            let ask = ask_to_quit(&name);
            let want = if agrees {
                QuitAnswer::Agreed
            } else {
                QuitAnswer::Cancelled
            };
            assert_eq!(ask.answer, want);
            assert!(
                was_closed.recv_timeout(Duration::from_millis(100)).is_err(),
                "{n}: nothing closes on the question alone"
            );
            ask.finish(close);
            assert_eq!(
                was_closed.recv_timeout(Duration::from_millis(500)).is_ok(),
                closes,
                "{n}"
            );
        }
        // A child that is gone counts as agreeing.
        assert_eq!(ask_to_quit(&unique("nobody")).answer, QuitAnswer::Agreed);
    }

    /// A child listening as `name` that answers `agrees` after `wait`, and reports a close.
    fn child(name: &str, agrees: bool, wait: Duration) -> std::sync::mpsc::Receiver<()> {
        let (closed, was_closed) = channel();
        listen_for_quit(
            name,
            move |reply| {
                std::thread::spawn(move || {
                    std::thread::sleep(wait);
                    let _ = reply.send(agrees);
                });
            },
            move || {
                let _ = closed.send(());
            },
        )
        .unwrap();
        was_closed
    }

    fn settle(run: &mut QuitRun) -> QuitOutcome {
        for _ in 0..200 {
            match run.poll() {
                QuitOutcome::Waiting => std::thread::sleep(Duration::from_millis(10)),
                done => return done,
            }
        }
        panic!("the quit never settled")
    }

    #[test]
    fn a_cancel_keeps_every_window_open() {
        // A clean child agrees at once; another asks its user, who cancels.
        let (clean, asking) = (unique("run-clean"), unique("run-asking"));
        let clean_closed = child(&clean, true, Duration::ZERO);
        let asking_closed = child(&asking, false, Duration::from_millis(150));
        let mut run = QuitRun::start(vec![clean, asking], || {});
        assert_eq!(settle(&mut run), QuitOutcome::Stay);
        assert!(
            clean_closed
                .recv_timeout(Duration::from_millis(300))
                .is_err(),
            "the clean window stays open too"
        );
        assert!(asking_closed.try_recv().is_err());
    }

    #[test]
    fn when_all_agree_every_window_closes() {
        let (a, b) = (unique("run-a"), unique("run-b"));
        let a_closed = child(&a, true, Duration::ZERO);
        let b_closed = child(&b, true, Duration::from_millis(100));
        let mut run = QuitRun::start(vec![a, b, unique("run-gone")], || {});
        assert_eq!(settle(&mut run), QuitOutcome::Quit);
        assert!(a_closed.recv_timeout(Duration::from_millis(500)).is_ok());
        assert!(b_closed.recv_timeout(Duration::from_millis(500)).is_ok());
    }

    #[test]
    fn a_quit_round_waits_for_every_child_and_any_cancel_keeps_all() {
        let mut round = QuitRound::new(3);
        assert_eq!(round.outcome(), QuitOutcome::Waiting);
        round.answer(QuitAnswer::Agreed);
        round.answer(QuitAnswer::Cancelled);
        assert_eq!(
            round.outcome(),
            QuitOutcome::Waiting,
            "one still asking its user"
        );
        round.answer(QuitAnswer::Agreed);
        assert_eq!(round.outcome(), QuitOutcome::Stay);
        let mut round = QuitRound::new(2);
        round.answer(QuitAnswer::Agreed);
        round.answer(QuitAnswer::Agreed);
        assert_eq!(round.outcome(), QuitOutcome::Quit);
        assert_eq!(
            QuitRound::new(0).outcome(),
            QuitOutcome::Quit,
            "no child: quit at once"
        );
    }

    #[test]
    fn a_second_launch_shows_home_unless_it_starts_minimized() {
        assert_eq!(second_launch_page(false), Some(Page::Start));
        assert_eq!(
            second_launch_page(true),
            None,
            "start with the system must not pop the window up"
        );
    }

    #[test]
    fn one_capture_at_a_time_until_it_lets_go() {
        let name = unique("capture");
        let first = capture_slot(&name).expect("no capture open yet");
        assert!(
            capture_slot(&name).is_none(),
            "a second capture finds the slot taken"
        );
        drop(first);
        assert!(
            capture_slot(&name).is_some(),
            "free again once the first lets go"
        );
    }

    #[test]
    fn page_round_trips_through_the_wire_format() {
        for page in [Page::Start, Page::Style, Page::Settings, Page::About] {
            assert_eq!(Page::parse(page.as_str()), Some(page));
        }
        assert_eq!(Page::parse("nope"), None);
    }

    #[test]
    fn the_endpoint_is_per_user_and_safe() {
        let e = endpoint();
        assert!(e.starts_with("takeshot-"), "{e}");
        assert!(
            e.chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'),
            "{e}"
        );
    }
}
