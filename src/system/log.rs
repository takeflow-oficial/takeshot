//! The log file of each Takeshot process. The Windows release has no console, so what a process
//! says through stderr is lost; here it also goes to `takeshot-<role>.log` in the user's log folder
//! (one file per kind of process: resident, standby, capture, edit, pin), and a panic leaves a line
//! there instead of nothing. A file grows by appending across runs; at the start of a run one past
//! `ROTATE_AT` becomes `takeshot-<role>.previous.log`, so a role keeps about two files' worth. Lines
//! carry the process id, because several editors can append to the same file. Nothing the user
//! captured or typed goes in: a panic is logged by thread and source line, never by its message,
//! and the start line lists only the command line's flags, never a file path. A run stops writing
//! at `RUN_BUDGET`, except for a panic, which still gets in within `URGENT_RESERVE` more bytes.

use std::fmt;
use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::panic::{Location, PanicHookInfo};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

/// A role's log file this big or bigger is set aside as `.previous.log` when a run starts.
pub const ROTATE_AT: u64 = 256 * 1024;

/// What one run writes before it stops: a loop of errors must not fill the disk.
pub const RUN_BUDGET: u64 = 1024 * 1024;

/// What a run may still write past its budget, for the lines that must not be lost (panics).
pub const URGENT_RESERVE: u64 = 16 * 1024;

/// One process's open log file.
pub struct Sink {
    pid: u32,
    state: Mutex<State>,
}

struct State {
    file: File,
    written: u64,
    budget: u64,
    full: bool,
}

impl Sink {
    /// Opens `takeshot-<role>.log` in `dir` (made if missing), setting a big old one aside first,
    /// and writes the start line (`flags` are the command line's `--` flags).
    pub fn open_in(dir: &Path, role: &str, budget: u64, flags: &str) -> io::Result<Self> {
        std::fs::create_dir_all(dir)?;
        let path = dir.join(format!("takeshot-{role}.log"));
        if std::fs::metadata(&path).is_ok_and(|m| m.len() >= ROTATE_AT) {
            // Best effort: a failed rename only lets the file grow a little more.
            let _ = std::fs::rename(&path, dir.join(format!("takeshot-{role}.previous.log")));
        }
        let mut options = OpenOptions::new();
        options.create(true).append(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let sink = Self {
            pid: std::process::id(),
            state: Mutex::new(State {
                file: options.open(path)?,
                written: 0,
                budget,
                full: false,
            }),
        };
        sink.write(format_args!(
            "start takeshot {} {} {} role={role} flags={flags}",
            env!("CARGO_PKG_VERSION"),
            std::env::consts::OS,
            std::env::consts::ARCH,
        ));
        Ok(sink)
    }

    /// Appends one line: time, process id and `args`. Past the run's budget it says so once and
    /// then stays quiet.
    pub fn write(&self, args: fmt::Arguments) {
        self.append(args, false);
    }

    /// Like `write`, for the line that must not be lost (a panic): it still goes in after the
    /// budget is spent, within `URGENT_RESERVE` more bytes.
    pub fn write_urgent(&self, args: fmt::Arguments) {
        self.append(args, true);
    }

    fn append(&self, args: fmt::Arguments, urgent: bool) {
        let at = chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f");
        let line = format!("{at} [{}] {args}\n", self.pid);
        // A panic on another thread poisons the lock; the log is the last thing that should stop.
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if state.full && !urgent {
            return;
        }
        let limit = state.budget + if urgent { URGENT_RESERVE } else { 0 };
        let text = if state.written + line.len() as u64 <= limit {
            line
        } else if urgent {
            return;
        } else {
            state.full = true;
            format!(
                "{at} [{}] log full, no more lines from this run\n",
                self.pid
            )
        };
        if state.file.write_all(text.as_bytes()).is_ok() {
            state.written += text.len() as u64;
        }
    }
}

/// What a panic leaves in the log: the thread and the source line, never the message (it may hold
/// a path or something the user typed).
pub fn panic_text(thread: Option<&str>, at: Option<&Location<'_>>) -> String {
    let thread = thread.unwrap_or("unnamed");
    match at {
        Some(at) => format!(
            "panic in thread '{thread}' at {}:{}:{}",
            at.file(),
            at.line(),
            at.column()
        ),
        None => format!("panic in thread '{thread}'"),
    }
}

type Hook = Box<dyn Fn(&PanicHookInfo<'_>) + Sync + Send + 'static>;

/// A panic hook that writes the panic as a line in `sink` and then runs `previous`.
fn hook_over(sink: Arc<Sink>, previous: Hook) -> Hook {
    Box::new(move |info| {
        let thread = std::thread::current();
        sink.write_urgent(format_args!(
            "{}",
            panic_text(thread.name(), info.location())
        ));
        previous(info);
    })
}

/// Makes every panic of the process a line in `sink`, then lets the previous hook run.
pub fn install_panic_hook(sink: Arc<Sink>) {
    std::panic::set_hook(hook_over(sink, std::panic::take_hook()));
}

static SINK: OnceLock<Arc<Sink>> = OnceLock::new();

/// The user's log folder: `<local data>\takeshot\logs` on Windows, `$XDG_STATE_HOME/takeshot/logs`
/// on Linux and `~/Library/Logs/takeshot` on macOS.
#[cfg(not(target_os = "macos"))]
pub fn dir() -> Option<PathBuf> {
    dirs::state_dir()
        .or_else(dirs::data_local_dir)
        .map(|d| d.join("takeshot").join("logs"))
}

#[cfg(target_os = "macos")]
pub fn dir() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join("Library").join("Logs").join("takeshot"))
}

/// The flags of a command line (`--capture`, `--full`) without their values, which are file names
/// (`--flag=value` is cut at the `=`): what the log may say about how a process was started.
pub fn flags<S: AsRef<str>>(args: impl IntoIterator<Item = S>) -> String {
    args.into_iter()
        .filter_map(|a| {
            let a = a.as_ref();
            a.starts_with("--")
                .then(|| a.split('=').next().unwrap_or(a).to_owned())
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Opens this process's log and installs the panic hook. Without a folder it stays on stderr.
pub fn init(role: &str, flags: &str) {
    let Some(dir) = dir() else { return };
    match Sink::open_in(&dir, role, RUN_BUDGET, flags) {
        Ok(sink) => {
            let sink = Arc::new(sink);
            install_panic_hook(sink.clone());
            let _ = SINK.set(sink);
        }
        Err(e) => eprintln!("takeshot: no log file in {}: {e}", dir.display()),
    }
}

/// Says `args` on stderr and in the log file, when this process has one.
pub fn line(args: fmt::Arguments) {
    line_in(SINK.get().map(Arc::as_ref), args);
}

fn line_in(sink: Option<&Sink>, args: fmt::Arguments) {
    eprintln!("takeshot: {args}");
    if let Some(sink) = sink {
        sink.write(args);
    }
}

/// Ends the log of a run with the way the process left: a clean exit in the file only (it is no
/// news on stderr), an error on both.
pub fn finish<E: fmt::Display>(res: &Result<(), E>) {
    finish_in(SINK.get().map(Arc::as_ref), res);
}

fn finish_in<E: fmt::Display>(sink: Option<&Sink>, res: &Result<(), E>) {
    match res {
        Ok(()) => {
            if let Some(sink) = sink {
                sink.write(format_args!("exit"));
            }
        }
        Err(e) => line_in(sink, format_args!("exit with an error: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(test: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("takeshot-test-{}-log-{test}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn read(dir: &Path, name: &str) -> String {
        std::fs::read_to_string(dir.join(name)).unwrap_or_default()
    }

    #[test]
    fn a_run_opens_its_roles_file_and_says_how_it_started() {
        let dir = scratch("start");
        let sink = Sink::open_in(&dir, "edit", RUN_BUDGET, "--edit").unwrap();
        sink.write(format_args!("hello {}", 7));
        let log = read(&dir, "takeshot-edit.log");
        let start = log.lines().next().expect("a start line");
        for part in [
            "start",
            env!("CARGO_PKG_VERSION"),
            std::env::consts::OS,
            std::env::consts::ARCH,
            "role=edit",
            "flags=--edit",
            &format!("[{}]", std::process::id()),
        ] {
            assert!(start.contains(part), "{part:?} in {start:?}");
        }
        assert!(
            log.lines().nth(1).is_some_and(|l| l.ends_with("hello 7")),
            "{log}"
        );
        assert!(!dir.join("takeshot-edit.previous.log").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn runs_of_one_role_append_to_the_same_file() {
        let dir = scratch("append");
        let first = Sink::open_in(&dir, "resident", RUN_BUDGET, "").unwrap();
        first.write(format_args!("from the first run"));
        let second = Sink::open_in(&dir, "resident", RUN_BUDGET, "").unwrap();
        second.write(format_args!("from the second run"));
        let log = read(&dir, "takeshot-resident.log");
        assert!(log.contains("from the first run"), "{log}");
        assert!(log.contains("from the second run"), "{log}");
        assert_eq!(log.matches("start").count(), 2, "{log}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_big_file_is_set_aside_when_a_run_starts() {
        let dir = scratch("rotate");
        std::fs::create_dir_all(&dir).unwrap();
        let mut old = b"OLD RUN\n".to_vec();
        old.resize(ROTATE_AT as usize, b'.');
        std::fs::write(dir.join("takeshot-capture.log"), &old).unwrap();
        std::fs::write(dir.join("takeshot-capture.previous.log"), "OLDER RUN").unwrap();
        let _sink = Sink::open_in(&dir, "capture", RUN_BUDGET, "--capture").unwrap();
        assert!(read(&dir, "takeshot-capture.previous.log").starts_with("OLD RUN"));
        let fresh = read(&dir, "takeshot-capture.log");
        assert!(
            !fresh.contains("OLD RUN") && !fresh.contains("OLDER RUN"),
            "{fresh}"
        );
        assert!(fresh.contains("start"), "{fresh}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn roles_keep_apart_files() {
        let dir = scratch("roles");
        let resident = Sink::open_in(&dir, "resident", RUN_BUDGET, "").unwrap();
        let standby = Sink::open_in(&dir, "standby", RUN_BUDGET, "").unwrap();
        resident.write(format_args!("only the resident"));
        standby.write(format_args!("only the standby"));
        let (r, s) = (
            read(&dir, "takeshot-resident.log"),
            read(&dir, "takeshot-standby.log"),
        );
        assert!(
            r.contains("only the resident") && !r.contains("only the standby"),
            "{r}"
        );
        assert!(
            s.contains("only the standby") && !s.contains("only the resident"),
            "{s}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_run_stops_writing_past_its_budget() {
        let dir = scratch("budget");
        let sink = Sink::open_in(&dir, "pin", 400, "").unwrap();
        for i in 0..200 {
            sink.write(format_args!("line {i} of a loop that never ends"));
        }
        let log = read(&dir, "takeshot-pin.log");
        let lines: Vec<&str> = log.lines().collect();
        let (note, body) = lines.split_last().expect("lines");
        assert!(
            note.ends_with("log full, no more lines from this run"),
            "{log}"
        );
        assert_eq!(log.matches("log full").count(), 1, "{log}");
        // The start line and the lines before the note fit the budget, and the budget was used.
        let body_len: usize = body.iter().map(|l| l.len() + 1).sum();
        // What is left is less than one more line (about 65 bytes, a longer pid adds a few).
        assert!(
            (400 - 80..=400).contains(&body_len),
            "{body_len} bytes in {log}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The panic hook is one per process: the tests that replace it take turns.
    static HOOK: Mutex<()> = Mutex::new(());

    /// Panics on a thread named `thread` with `install` having put the hook in place, then puts the
    /// harness's own hook back.
    fn panic_under(install: impl FnOnce(), thread: &str, message: &'static str) {
        let _turn = HOOK.lock().unwrap_or_else(PoisonError::into_inner);
        let original = std::panic::take_hook();
        install();
        let joined = std::thread::Builder::new()
            .name(thread.into())
            .spawn(move || panic!("{message}"))
            .unwrap()
            .join();
        drop(std::panic::take_hook());
        std::panic::set_hook(original);
        assert!(joined.is_err());
    }

    #[test]
    fn a_panic_leaves_the_thread_and_the_line_but_not_the_message() {
        let dir = scratch("panic");
        let sink = Arc::new(Sink::open_in(&dir, "edit", RUN_BUDGET, "").unwrap());
        panic_under(
            || install_panic_hook(sink),
            "worker",
            "secret C:\\Users\\someone\\picture.png",
        );
        let log = read(&dir, "takeshot-edit.log");
        let line = log
            .lines()
            .find(|l| l.contains("panic in thread 'worker'"))
            .unwrap_or_else(|| panic!("no panic line in {log}"));
        assert!(line.contains("log.rs:"), "{line}");
        assert!(!log.contains("secret") && !log.contains("someone"), "{log}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_panic_is_logged_even_after_the_run_spent_its_budget() {
        let dir = scratch("urgent");
        let sink = Arc::new(Sink::open_in(&dir, "resident", 300, "").unwrap());
        for i in 0..100 {
            sink.write(format_args!("an error of a loop, number {i}"));
        }
        assert!(read(&dir, "takeshot-resident.log").contains("log full"));
        panic_under(|| install_panic_hook(sink), "late", "after the budget");
        let log = read(&dir, "takeshot-resident.log");
        assert!(log.contains("panic in thread 'late'"), "{log}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn what_a_run_may_write_past_its_budget_has_a_limit() {
        let dir = scratch("reserve");
        let sink = Sink::open_in(&dir, "pin", 200, "").unwrap();
        for _ in 0..2000 {
            sink.write_urgent(format_args!("panic in thread 'again' at src/x.rs:1:1"));
        }
        let len = read(&dir, "takeshot-pin.log").len() as u64;
        assert!(len > 200 && len <= 200 + URGENT_RESERVE, "{len} bytes");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_panic_hook_hands_over_to_the_one_before_it() {
        let dir = scratch("chain");
        let sink = Arc::new(Sink::open_in(&dir, "edit", RUN_BUDGET, "").unwrap());
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let seen = calls.clone();
        let previous: Hook = Box::new(move |_| {
            seen.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        });
        panic_under(
            || std::panic::set_hook(hook_over(sink, previous)),
            "chained",
            "once",
        );
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
        let log = read(&dir, "takeshot-edit.log");
        assert!(log.contains("panic in thread 'chained'"), "{log}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_line_and_the_exit_reach_the_file() {
        let dir = scratch("glue");
        let sink = Sink::open_in(&dir, "capture", RUN_BUDGET, "--capture").unwrap();
        line_in(Some(&sink), format_args!("no tray icon: {}", "boom"));
        finish_in(Some(&sink), &Ok::<(), String>(()));
        let log = read(&dir, "takeshot-capture.log");
        let lines: Vec<&str> = log.lines().collect();
        assert_eq!(lines.len(), 3, "{log}");
        assert!(lines[1].ends_with("] no tray icon: boom"), "{log}");
        assert!(lines[2].ends_with("] exit"), "{log}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_exit_with_an_error_says_why_and_a_run_with_no_file_still_works() {
        let dir = scratch("glue-error");
        let sink = Sink::open_in(&dir, "edit", RUN_BUDGET, "").unwrap();
        finish_in(Some(&sink), &Err::<(), _>("no OpenGL"));
        let log = read(&dir, "takeshot-edit.log");
        assert!(
            log.lines()
                .last()
                .is_some_and(|l| l.ends_with("] exit with an error: no OpenGL")),
            "{log}"
        );
        // A throwaway run has no sink: lines go to stderr only, and nothing panics.
        line_in(None, format_args!("only on stderr"));
        finish_in(None, &Err::<(), _>("no OpenGL"));
        finish_in(None, &Ok::<(), String>(()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_panic_text_names_an_unnamed_thread_and_a_missing_place() {
        assert_eq!(panic_text(None, None), "panic in thread 'unnamed'");
        let at = Location::caller();
        let text = panic_text(Some("main"), Some(at));
        assert!(text.starts_with("panic in thread 'main' at "), "{text}");
        assert!(
            text.ends_with(&format!("{}:{}", at.line(), at.column())),
            "{text}"
        );
    }

    #[test]
    fn the_start_line_lists_flags_and_never_a_file() {
        let args = [
            "--edit",
            "C:\\Users\\someone\\Pictures\\a.png",
            "--theme",
            "coral",
            "--full",
        ];
        assert_eq!(flags(args), "--edit --theme --full");
        assert_eq!(flags(args.map(String::from)), "--edit --theme --full");
        assert_eq!(flags(Vec::<String>::new()), "");
        // A value glued with `=` is a value too.
        let glued = [
            "--captures-dir=C:\\Users\\someone\\x",
            "--lang=pt",
            "--full",
        ];
        assert_eq!(flags(glued), "--captures-dir --lang --full");
    }

    #[test]
    fn the_folder_is_takeshots_own_and_per_user() {
        let dir = dir().expect("a log folder");
        assert!(
            dir.ends_with("takeshot/logs") || dir.ends_with("Logs/takeshot"),
            "{dir:?}"
        );
    }
}
