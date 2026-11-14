//! What the user is told when Takeshot cannot start. The Windows release has no console, so a start
//! that fails (no OpenGL on a remote desktop or a virtual machine, an old graphics driver) used to
//! end with nothing on screen: the process just left. A native message box now says why, hints at
//! the graphics driver when that is the cause, and names the log folder; the failure goes to the log
//! first, so it is there even if the box stays open for hours.

use crate::system::log;
use crate::ui::i18n::{Msg, t, tf};
use std::path::Path;

/// How long a reason may be in the box: a glutin error can carry a whole config dump.
const REASON_MAX: usize = 240;

/// What went wrong, as far as the user can act on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cause {
    /// No usable OpenGL: glutin or egui_glow refused.
    Graphics,
    /// The window system, the app's own setup or anything else.
    Other,
}

pub fn cause(e: &eframe::Error) -> Cause {
    use eframe::Error;
    match e {
        Error::Glutin(_) | Error::NoGlutinConfigs(..) | Error::OpenGL(_) => Cause::Graphics,
        _ => Cause::Other,
    }
}

/// The text of the message box.
pub struct Notice {
    pub title: String,
    pub body: String,
}

/// `line` without the " at <file>.rs:<line>" that winit and others put in their errors: the path is
/// the build machine's and the number means nothing to the user.
fn without_location(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    while let Some(i) = rest.find(" at ") {
        let after = &rest[i + 4..];
        let token = after
            .find(char::is_whitespace)
            .map_or(after, |n| &after[..n]);
        let place = token.trim_end_matches(':');
        let is_location = place.rfind(".rs:").is_some_and(|p| {
            let n = &place[p + 4..];
            !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit())
        });
        out.push_str(&rest[..i]);
        if is_location {
            rest = &rest[i + 4 + place.len()..];
        } else {
            out.push_str(" at ");
            rest = after;
        }
    }
    out.push_str(rest);
    out
}

/// The first line of `reason` without its source location: what the log keeps whole.
fn first_line(reason: &str) -> String {
    without_location(reason.lines().next().unwrap_or_default())
}

/// `first_line` for the box: past `REASON_MAX` characters the middle goes and both ends stay, since
/// eframe puts a long dump of the glutin config template first and the cause last.
fn short(reason: &str) -> String {
    let line = first_line(reason);
    let n = line.chars().count();
    if n <= REASON_MAX {
        return line;
    }
    let half = REASON_MAX / 2;
    let head: String = line.chars().take(half).collect();
    let tail: String = line.chars().skip(n - half).collect();
    format!("{head}...{tail}")
}

/// The log line for a start that failed: the whole reason, since the box points here for details.
fn failure_line(reason: &str) -> String {
    format!("could not start: {}", first_line(reason))
}

/// The box for a failure of `cause` with `reason`; `log_dir` is where the log is, when there is one.
pub fn notice(cause: Cause, reason: &str, log_dir: Option<&Path>) -> Notice {
    let mut parts = vec![tf(Msg::StartFailedBody, &[&short(reason)])];
    if cause == Cause::Graphics {
        parts.push(t(Msg::StartFailedGraphics).to_owned());
    }
    if let Some(dir) = log_dir {
        parts.push(tf(Msg::StartFailedLog, &[&dir.display().to_string()]));
    }
    Notice {
        title: t(Msg::StartFailedTitle).to_owned(),
        body: parts.join("\n\n"),
    }
}

/// Shows the box and waits for its button.
fn show(notice: &Notice) {
    rfd::MessageDialog::new()
        .set_level(rfd::MessageLevel::Error)
        .set_title(notice.title.as_str())
        .set_description(notice.body.as_str())
        .set_buttons(rfd::MessageButtons::Ok)
        .show();
}

/// Logs why the app could not start and, when `tell` (a run the user started, not a background
/// process or a picture rendering), shows the box.
pub fn report(e: &eframe::Error, tell: bool) {
    let reason = e.to_string();
    log::line(format_args!("{}", failure_line(&reason)));
    if tell {
        show(&notice(cause(e), &reason, log::dir().as_deref()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::i18n::{Lang, set_lang};
    use std::path::PathBuf;

    fn gl_failure() -> eframe::Error {
        eframe::Error::OpenGL(eframe::egui_glow::PainterError::from(
            "egui_glow requires opengl 2.0+. ".to_owned(),
        ))
    }

    #[test]
    fn a_graphics_failure_says_why_hints_at_the_driver_and_names_the_log() {
        set_lang(Lang::En);
        let dir = PathBuf::from("logs-folder").join("takeshot");
        let n = notice(Cause::Graphics, "OpenGL: too old", Some(&dir));
        assert_eq!(n.title, "Takeshot could not start");
        assert!(
            n.body
                .starts_with("Takeshot could not start: OpenGL: too old"),
            "{}",
            n.body
        );
        assert!(n.body.contains("OpenGL 2.0 or newer"), "{}", n.body);
        assert!(n.body.contains("graphics driver"), "{}", n.body);
        assert!(n.body.contains(&dir.display().to_string()), "{}", n.body);
    }

    #[test]
    fn the_box_speaks_portuguese_too() {
        set_lang(Lang::Pt);
        let dir = PathBuf::from("pasta-de-logs");
        let n = notice(Cause::Graphics, "OpenGL: antigo", Some(&dir));
        set_lang(Lang::En);
        assert_eq!(n.title, "O Takeshot não conseguiu iniciar");
        assert!(n.body.contains("driver de vídeo"), "{}", n.body);
        assert!(n.body.contains("pasta de logs"), "{}", n.body);
        assert!(n.body.contains("pasta-de-logs"), "{}", n.body);
    }

    #[test]
    fn another_failure_has_no_graphics_hint_and_no_log_line_without_a_folder() {
        set_lang(Lang::En);
        let n = notice(Cause::Other, "winit EventLoopError: no display", None);
        assert!(n.body.contains("no display"), "{}", n.body);
        assert!(!n.body.contains("graphics driver"), "{}", n.body);
        assert!(!n.body.contains("log folder"), "{}", n.body);
    }

    #[test]
    fn a_long_reason_is_cut_to_its_first_line() {
        set_lang(Lang::En);
        let reason = format!("first line\n{}", "x".repeat(2000));
        assert_eq!(short(&reason), "first line");
        let long = "y".repeat(1000);
        let cut = short(&long);
        assert_eq!(cut.chars().count(), REASON_MAX + 3, "{cut}");
        assert!(cut.contains("..."));
        let n = notice(Cause::Other, &reason, None);
        assert!(!n.body.contains("xxx"), "{}", n.body);
    }

    #[test]
    fn a_long_reason_keeps_both_ends_in_the_box_and_all_of_it_in_the_log() {
        // What eframe says when glutin finds no config: a dump of the template, then the cause.
        let dump = "d".repeat(450);
        let reason =
            format!("Found no glutin configs matching the template: {dump}. Error: BadMatch");
        let boxed = short(&reason);
        assert!(boxed.starts_with("Found no glutin configs"), "{boxed}");
        assert!(boxed.ends_with("Error: BadMatch"), "{boxed}");
        assert_eq!(boxed.chars().count(), REASON_MAX + 3, "{boxed}");
        // The box points to the log for details, so the log holds the whole line.
        assert_eq!(failure_line(&reason), format!("could not start: {reason}"));
        assert!(!failure_line("first\nsecond").contains("second"));
        assert_eq!(failure_line("x at /p/q.rs:9: y"), "could not start: x: y");
    }

    #[test]
    fn a_source_location_is_left_out_of_the_reason() {
        // What winit says with no display (seen in a clean Ubuntu container): the path is the
        // build machine's and the line number means nothing to the user.
        let raw = "winit EventLoopError: os error at /usr/local/cargo/registry/src/index.crates.io-1/winit-0.30.13/src/platform_impl/linux/mod.rs:765: neither WAYLAND_DISPLAY nor WAYLAND_SOCKET nor DISPLAY is set.";
        assert_eq!(
            short(raw),
            "winit EventLoopError: os error: neither WAYLAND_DISPLAY nor WAYLAND_SOCKET nor DISPLAY is set."
        );
        // Only a source location goes: other " at " text stays.
        assert_eq!(short("stopped at the door"), "stopped at the door");
        assert_eq!(
            short("failed at file.rs: no number"),
            "failed at file.rs: no number"
        );
        let n = notice(Cause::Other, raw, None);
        assert!(!n.body.contains("registry"), "{}", n.body);
    }

    #[test]
    fn only_a_graphics_error_is_told_apart() {
        assert_eq!(cause(&gl_failure()), Cause::Graphics);
        let other = eframe::Error::AppCreation("setup failed".into());
        assert_eq!(cause(&other), Cause::Other);
    }
}
