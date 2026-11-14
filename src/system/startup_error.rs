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

/// The first line of `reason`, cut to `REASON_MAX` characters.
fn short(reason: &str) -> String {
    let line = reason.lines().next().unwrap_or_default();
    if line.chars().count() > REASON_MAX {
        let cut: String = line.chars().take(REASON_MAX).collect();
        format!("{cut}...")
    } else {
        line.to_owned()
    }
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
        body: parts.join(
            "

",
        ),
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
    log::line(format_args!("could not start: {}", short(&reason)));
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
        assert!(cut.ends_with("..."));
        let n = notice(Cause::Other, &reason, None);
        assert!(!n.body.contains("xxx"), "{}", n.body);
    }

    #[test]
    fn only_a_graphics_error_is_told_apart() {
        assert_eq!(cause(&gl_failure()), Cause::Graphics);
        let other = eframe::Error::AppCreation("setup failed".into());
        assert_eq!(cause(&other), Cause::Other);
    }
}
