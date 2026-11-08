//! Small notice shown after copying or saving, in the theme's colours: a card with the action's
//! icon, what happened and what to do next. Clicking it opens the folder of a saved file; it
//! closes on its own.

use crate::Switch;
use crate::system::notify::Spec;
use crate::ui::i18n::{Msg, t as tr, tf};
use crate::ui::kit::Ico;
use crate::ui::style::{Weight, font};
use crate::ui::tokens::Tokens;
use egui::{Sense, Stroke, Ui, pos2, vec2};
use std::path::PathBuf;

pub const NOTICE_SIZE: egui::Vec2 = egui::vec2(380.0, 72.0);
/// The gap to the right edge and to the taskbar (the edge of the work area).
pub const NOTICE_MARGIN: f32 = 16.0;
const NOTICE_SECONDS: f64 = 3.5;

pub struct Notice {
    title: String,
    hint: String,
    icon: Ico,
    path: Option<PathBuf>,
    started: Option<f64>,
}

/// Where the notice opens: the bottom-right corner of the work area (`area`, in points).
pub fn notice_pos(area: egui::Rect) -> egui::Pos2 {
    area.max - NOTICE_SIZE - vec2(NOTICE_MARGIN, NOTICE_MARGIN)
}

impl Notice {
    pub fn copied() -> Self {
        let paste = if cfg!(target_os = "macos") {
            "Cmd+V"
        } else {
            "Ctrl+V"
        };
        Self {
            title: tr(Msg::NoticeCopied).to_string(),
            hint: tf(Msg::NoticeCopiedHint, &[paste]),
            icon: Ico::Check,
            path: None,
            started: None,
        }
    }

    pub fn saved(path: PathBuf) -> Self {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        Self {
            title: tr(Msg::NoticeSaved).to_string(),
            hint: tf(Msg::NoticeSavedHint, &[&name]),
            icon: Ico::Download,
            path: Some(path),
            started: None,
        }
    }

    /// The same words as a desktop notification, with the folder a click opens.
    pub fn for_desktop(&self) -> Spec {
        Spec {
            title: self.title.clone(),
            body: self.hint.clone(),
            folder: self
                .path
                .as_ref()
                .and_then(|p| p.parent())
                .map(PathBuf::from),
        }
    }

    /// The card fills the window: the surface with a 1 px border, the icon in a tinted circle
    /// (accent), the title (13 medium) and the hint (12, muted), cut with "…" to fit.
    pub fn ui(&mut self, ui: &mut Ui, t: &Tokens) -> Switch {
        let now = ui.input(|i| i.time);
        let started = *self.started.get_or_insert(now);
        let rect = ui.max_rect();
        let resp = ui.interact(rect, ui.scope_id().with("notice"), Sense::click());
        let p = ui.painter();
        let bg = if resp.hovered() { t.raised } else { t.surf };
        p.rect_filled(rect, 0.0, bg);
        p.rect_stroke(
            rect,
            0.0,
            Stroke::new(1.0, t.border),
            egui::StrokeKind::Inside,
        );
        let icon_at = pos2(rect.left() + 18.0 + 16.0, rect.center().y);
        p.circle_filled(icon_at, 16.0, t.tint);
        p.text(
            icon_at,
            egui::Align2::CENTER_CENTER,
            crate::ui::icon::glyph(self.icon),
            crate::ui::icon::font(16.0),
            t.accent,
        );
        let left = icon_at.x + 16.0 + 12.0;
        let room = (rect.right() - 18.0 - left).max(1.0);
        let line = |text: &str, size: f32, weight: Weight, color| {
            let mut job = egui::text::LayoutJob::simple_singleline(
                text.to_owned(),
                font(size, weight),
                color,
            );
            job.wrap = egui::text::TextWrapping::truncate_at_width(room);
            p.layout_job(job)
        };
        let title = line(&self.title, 13.0, Weight::Medium, t.text);
        let hint = line(&self.hint, 12.0, Weight::Regular, t.muted);
        let gap = 4.0;
        let top = rect.center().y - (title.size().y + gap + hint.size().y) / 2.0;
        let title_h = title.size().y;
        p.galley(pos2(left, top), title, t.text);
        p.galley(pos2(left, top + title_h + gap), hint, t.muted);
        if resp.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        if resp.clicked() {
            if let Some(dir) = self.path.as_ref().and_then(|p| p.parent()) {
                crate::system::actions::open_folder(dir);
            }
            return Switch::Close;
        }
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(200));
        if now - started > NOTICE_SECONDS && !resp.hovered() {
            return Switch::Close;
        }
        Switch::None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::Rect;

    /// What one notice frame painted: rectangle fills and texts.
    fn paint(n: &mut Notice, t: &Tokens) -> (Vec<egui::Color32>, Vec<String>) {
        fn walk(s: &egui::Shape, fills: &mut Vec<egui::Color32>, texts: &mut Vec<String>) {
            match s {
                egui::Shape::Rect(r) => fills.push(r.fill),
                egui::Shape::Circle(c) => fills.push(c.fill),
                egui::Shape::Text(t) => texts.push(t.galley.text().to_owned()),
                egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, fills, texts)),
                _ => {}
            }
        }
        let ctx = egui::Context::default();
        crate::system::fonts::FontLib::load().install(&ctx);
        let mut out = (Vec::new(), Vec::new());
        for i in 0..2 {
            let input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), NOTICE_SIZE)),
                time: Some(i as f64 * 0.05),
                ..Default::default()
            };
            let mut full = ctx.run_ui(input, |ui| {
                n.ui(ui, t);
            });
            full.textures_delta.clear();
            out = (Vec::new(), Vec::new());
            for c in &full.shapes {
                walk(&c.shape, &mut out.0, &mut out.1);
            }
        }
        out
    }

    #[test]
    fn the_notice_uses_the_theme_and_the_language() {
        use crate::ui::i18n::{Lang, Msg, lang, set_lang, t as tr};
        use crate::ui::tokens::{ThemeId, tokens};
        let before = lang();
        for l in [Lang::Pt, Lang::En] {
            set_lang(l);
            for (theme, dark) in [
                (ThemeId::TakeUi, true),
                (ThemeId::TakeUi, false),
                (ThemeId::Coral, true),
            ] {
                let t = tokens(theme, dark);
                let (fills, texts) = paint(&mut Notice::copied(), &t);
                assert!(fills.contains(&t.surf), "{theme:?} {dark}: the card");
                assert!(
                    fills.contains(&t.tint),
                    "{theme:?} {dark}: the icon's circle"
                );
                assert!(
                    !fills.contains(&egui::Color32::from_rgb(255, 90, 54)),
                    "the old orange bar"
                );
                assert!(
                    texts.iter().any(|s| s == tr(Msg::NoticeCopied)),
                    "{l:?}: {texts:?}"
                );
            }
        }
        set_lang(before);
    }

    #[test]
    fn the_desktop_notification_says_what_the_notice_says() {
        use crate::ui::i18n::{Lang, Msg, lang, set_lang, t as tr, tf};
        let before = lang();
        for l in [Lang::Pt, Lang::En] {
            set_lang(l);
            let copied = Notice::copied().for_desktop();
            assert_eq!(copied.title, tr(Msg::NoticeCopied));
            assert!(
                copied.body.contains("V"),
                "{l:?}: the paste keys in {copied:?}"
            );
            assert_eq!(copied.folder, None, "a copy has no folder to open");

            let path = PathBuf::from("pics").join("a.png");
            let saved = Notice::saved(path).for_desktop();
            assert_eq!(saved.title, tr(Msg::NoticeSaved));
            assert_eq!(saved.body, tf(Msg::NoticeSavedHint, &["a.png"]));
            assert_eq!(
                saved.folder,
                Some(PathBuf::from("pics")),
                "a click opens the folder of the file"
            );
        }
        set_lang(before);
    }

    #[test]
    fn the_notice_sits_in_the_corner_of_the_work_area() {
        // A 40 px taskbar at the bottom: the notice keeps off it by the same gap as off the right edge.
        let area = Rect::from_min_max(pos2(0.0, 0.0), pos2(1920.0, 1040.0));
        let r = Rect::from_min_size(notice_pos(area), NOTICE_SIZE);
        assert!(area.contains_rect(r), "{r:?}");
        assert_eq!(area.max.x - r.max.x, NOTICE_MARGIN);
        assert_eq!(area.max.y - r.max.y, NOTICE_MARGIN);
        // A second monitor to the left, taskbar on top: still its own corner.
        let area = Rect::from_min_max(pos2(-1280.0, 48.0), pos2(0.0, 1024.0));
        let r = Rect::from_min_size(notice_pos(area), NOTICE_SIZE);
        assert!(area.contains_rect(r), "{r:?}");
    }
}
