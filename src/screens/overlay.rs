//! Capture overlay: the frozen screen, area selection, drawing tools and quick actions. The state,
//! what the user asks for (`CaptureAction`, `Overlay::apply`), the area and the canvas live here
//! for both themes; `views::capture` draws the bars and the hint.

use crate::engine::annotator::{Annotator, SelectOutcome, View, text_measure};
use crate::engine::cursor::{self, Area, CursorCtx};
use crate::engine::doc::{Doc, Handle as ObjHandle};
use crate::engine::model::*;
use crate::screens::editor::Editor;
use crate::screens::widgets;
use crate::system::capture::Shot;
use crate::system::settings::DrawOpts;
use crate::ui::i18n::{Msg, t as tr, tf};
use crate::ui::style::{Weight, font};
use crate::ui::tokens::Tokens;
use crate::{Shared, Switch};
use egui::{Align2, Color32, Key, Modifiers, Pos2, Rect, Sense, Stroke, Ui, pos2, vec2};
use image::RgbaImage;

/// What the user asked for on the capture's bars; the capture view returns these and `Overlay::apply`
/// carries them out.
#[derive(Clone, PartialEq, Debug)]
pub enum CaptureAction {
    SetTool(Tool),
    Undo,
    Redo,
    Pin,
    Save,
    Copy,
    Edit,
    Close,
    SetColor([u8; 4]),
    SetWidth(f32),
    /// A rectangle's corner radius, in picture px.
    SetRadius(f32),
    SetFill(bool),
    SetFont(usize),
    SetFontSize(f32),
    SetTextBox(bool),
    SetBlurMode(BlurMode),
    Duplicate,
    Delete,
    /// A colour picker grabbed: its changes undo in one step until the button is let go.
    Gesture(bool),
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Handle {
    Move,
    N,
    S,
    E,
    W,
    NE,
    NW,
    SE,
    SW,
}

enum Drag {
    None,
    NewSel(Pos2),
    Adjust {
        handle: Handle,
        start_ptr: Pos2,
        orig: Rect,
    },
}

pub struct Overlay {
    shot: Shot,
    baked: RgbaImage,
    tex: egui::TextureHandle,
    /// Selection in picture pixels.
    sel: Option<Rect>,
    drag: Drag,
    pub(crate) an: Annotator,
    /// The wheel's part of a notch still to come (`wheel_notches`).
    wheel_acc: f32,
    window_ready: u32,
    /// One capture at a time: held while this selection is on screen.
    _slot: Option<crate::system::instance::CaptureSlot>,
}

impl Overlay {
    pub fn new(ctx: &egui::Context, shot: Shot, shared: &Shared) -> Self {
        let tex = load_tex(ctx, "shot", &shot.image, egui::TextureOptions::NEAREST);
        let mut sel = None;
        if shared.settings.keep_selection
            && let Some([x, y, w, h]) = shared.settings.last_selection
        {
            let r = Rect::from_min_size(pos2(x, y), vec2(w, h));
            let full = Rect::from_min_size(
                Pos2::ZERO,
                vec2(shot.image.width() as f32, shot.image.height() as f32),
            );
            if full.contains_rect(r) && w > 2.0 && h > 2.0 {
                sel = Some(r);
            }
        }
        Self {
            baked: shot.image.clone(),
            shot,
            tex,
            sel,
            drag: Drag::None,
            an: Annotator::new(Tool::Select),
            wheel_acc: 0.0,
            window_ready: 0,
            _slot: None,
        }
    }

    /// Sample state for screenshots: an area already chosen and a mark on it.
    /// Keeps the capture slot while the selection is on screen; it goes when the overlay does
    /// (copied, saved, closed, or turned into the editor).
    pub fn hold(mut self, slot: Option<crate::system::instance::CaptureSlot>) -> Self {
        self._slot = slot;
        self
    }

    pub fn demo(self) -> Self {
        let doc = Doc::from_anns(vec![Ann {
            shape: Shape::Ellipse(pos2(700.0, 230.0), pos2(1000.0, 330.0)),
            color: DEFAULT_COLOR,
            width: DEFAULT_WIDTH,
            filled: false,
        }]);
        self.with_area(Rect::from_min_size(pos2(304.0, 153.0), vec2(992.0, 540.0)))
            .with_doc(Tool::Arrow, doc)
    }

    /// The area already chosen, in picture pixels.
    pub fn with_area(mut self, area: Rect) -> Self {
        self.sel = Some(area);
        self
    }

    /// Objects already on the picture, with `tool` in hand.
    pub fn with_doc(mut self, tool: Tool, doc: Doc) -> Self {
        self.an = Annotator::with_doc(tool, doc);
        self
    }

    /// A window on another monitor: it leaves the keyboard focus to the one under the mouse and
    /// starts with no area, since only one monitor holds the area.
    pub fn beside(mut self) -> Self {
        self.window_ready = 3;
        self.sel = None;
        self
    }

    /// Drops the area and what was drawn on it, as when the area moves to another monitor.
    fn clear_area(&mut self) {
        self.sel = None;
        self.drag = Drag::None;
        // Not `Annotator::new`: it takes the picture to be baked already, so the marks would stay
        // on `baked` and the texture. An annotator that has not baked yet draws the plain
        // screenshot again on the next frame.
        self.an = Annotator::with_doc(self.an.tool, Doc::new());
    }

    /// The monitor this window covers.
    pub fn shot(&self) -> &Shot {
        &self.shot
    }

    /// The area on `screen`, which shows the whole picture.
    pub fn area_on(&self, screen: Rect) -> Option<Rect> {
        let view = Self::view_on(screen, self.img_size());
        self.sel
            .map(|s| Rect::from_min_max(view.to_screen(s.min), view.to_screen(s.max)))
    }

    /// What the capture's bars keep off, on `screen`: the selected objects' bounds and the text
    /// being typed with its frame.
    pub fn bars_avoid(&self, screen: Rect, shared: &Shared) -> Option<Rect> {
        let view = Self::view_on(screen, self.img_size());
        let selected = self
            .an
            .selection_bounds(&text_measure(&shared.fonts))
            .map(|b| Rect::from_min_max(view.to_screen(b.min), view.to_screen(b.max)));
        [selected, self.an.draft_on_screen()]
            .into_iter()
            .flatten()
            .reduce(|a, b| a.union(b))
    }

    fn view_on(screen: Rect, img: egui::Vec2) -> View {
        View {
            origin: screen.min,
            k: screen.width() / img.x,
        }
    }

    /// The bars show once an area is chosen, except while it, an object, a new shape or a text's
    /// dashed frame is dragged.
    pub fn bars_shown(&self) -> bool {
        self.sel.is_some()
            && !matches!(self.drag, Drag::NewSel(_) | Drag::Adjust { .. })
            && !self.an.busy()
    }

    /// Carries out what the user asked for on the bars. `frame` parents the save dialog.
    pub fn apply(
        &mut self,
        a: CaptureAction,
        ctx: &egui::Context,
        frame: Option<&eframe::Frame>,
        shared: &mut Shared,
    ) -> Switch {
        let restyle = |ov: &mut Self, shared: &mut Shared, f: &dyn Fn(&mut DrawOpts)| {
            let before = shared.settings.draw.clone();
            f(&mut shared.settings.draw);
            ov.an.restyle_from_opts(&before, &shared.settings.draw);
            shared.settings_dirty = true;
        };
        match a {
            CaptureAction::SetTool(tool) => {
                self.an.set_tool(tool, &mut shared.settings.draw);
                shared.settings_dirty = true;
            }
            CaptureAction::Undo => {
                self.an.undo();
                self.an.sync_options(&mut shared.settings.draw);
            }
            CaptureAction::Redo => {
                self.an.redo();
                self.an.sync_options(&mut shared.settings.draw);
            }
            CaptureAction::Pin => return self.act_pin(shared),
            CaptureAction::Save => return self.act_save(frame, shared),
            CaptureAction::Copy => return self.act_copy(shared),
            CaptureAction::Edit => return self.open_editor(ctx, shared),
            CaptureAction::Close => return Switch::Close,
            CaptureAction::SetColor(c) => restyle(self, shared, &|o| {
                o.color = [c[0], c[1], c[2], o.color[3]];
            }),
            CaptureAction::SetWidth(w) => restyle(self, shared, &|o| o.width = w),
            CaptureAction::SetRadius(r) => restyle(self, shared, &|o| o.radius = r),
            CaptureAction::SetFill(on) => restyle(self, shared, &|o| o.filled = on),
            CaptureAction::SetFont(f) => restyle(self, shared, &|o| o.font = f),
            CaptureAction::SetFontSize(s) => restyle(self, shared, &|o| o.font_size = s),
            CaptureAction::SetTextBox(on) => restyle(self, shared, &|o| o.text_box = on),
            CaptureAction::SetBlurMode(m) => restyle(self, shared, &|o| o.blur_mode = m),
            CaptureAction::Duplicate => {
                self.an.doc.duplicate_selected(vec2(12.0, 12.0));
            }
            CaptureAction::Delete => self.an.doc.delete_selected(),
            CaptureAction::Gesture(true) => self.an.doc.begin_gesture(),
            CaptureAction::Gesture(false) => self.an.doc.end_gesture(),
        }
        Switch::None
    }

    fn img_size(&self) -> egui::Vec2 {
        vec2(
            self.shot.image.width() as f32,
            self.shot.image.height() as f32,
        )
    }

    fn crop(&self) -> Option<RgbaImage> {
        let s = self.sel?;
        let x = s.min.x.round().max(0.0) as u32;
        let y = s.min.y.round().max(0.0) as u32;
        let w = (s.width().round() as u32).min(self.baked.width().saturating_sub(x));
        let h = (s.height().round() as u32).min(self.baked.height().saturating_sub(y));
        (w > 0 && h > 0).then(|| image::imageops::crop_imm(&self.baked, x, y, w, h).to_image())
    }

    fn remember_selection(&self, shared: &mut Shared) {
        if let Some(s) = self.sel {
            shared.settings.last_selection = Some([s.min.x, s.min.y, s.width(), s.height()]);
            shared.settings.save();
        }
    }

    fn output(&self, shared: &mut Shared) -> Option<RgbaImage> {
        let crop = self.crop()?;
        let logo = shared.logo.image.clone();
        Some(crate::engine::render::finish(
            &crop,
            &shared.settings,
            logo.as_ref(),
            &shared.fonts,
            shared.settings.quick_style,
        ))
    }

    fn open_editor(&mut self, ctx: &egui::Context, shared: &mut Shared) -> Switch {
        self.an.commit_draft(&shared.settings.draw);
        let Some((base, doc)) = self.editor_handoff() else {
            return Switch::None;
        };
        self.remember_selection(shared);
        Switch::ToEditor(Box::new(Editor::new(ctx, base, doc, shared)))
    }

    /// One frame of the capture. `frame` parents the save dialog when there is one.
    pub fn ui(
        &mut self,
        ui: &mut Ui,
        frame: Option<&eframe::Frame>,
        t: &Tokens,
        shared: &mut Shared,
    ) -> Switch {
        let ctx = ui.ctx().clone();
        if self.window_ready < 3 {
            self.window_ready += 1;
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        }
        let screen = ui.max_rect();
        let view = Self::view_on(screen, self.img_size());
        // A colour picker's gesture ends with the button.
        if !ui.input(|i| i.pointer.any_down()) {
            self.an.doc.end_gesture();
        }
        let opts_snapshot = shared.settings.draw.clone();

        // ---------------- baked picture
        if let Some(anns) = self.an.bake() {
            self.baked =
                crate::engine::render::render_annotations(&self.shot.image, &anns, &shared.fonts);
            self.tex
                .set(color_image(&self.baked), egui::TextureOptions::NEAREST);
        }

        // ---------------- canvas interaction (first, so the floating bars sit on top)
        let canvas = ui.interact(
            screen,
            ui.scope_id().with("canvas"),
            Sense::click_and_drag(),
        );
        let pointer = ui.input(|i| i.pointer.hover_pos());
        // The text field is placed after the picture is painted, so it shows over it.
        let typing = self.an.draft.is_some();
        // A menu open at the start of the frame takes Esc (it closes while the view draws).
        let menu_open = egui::Popup::is_any_open(&ctx);

        let drawing_tool = self.sel.is_some() && self.an.tool != Tool::Select;
        self.canvas_input(ui, &canvas, &view, pointer, shared);
        if drawing_tool && !menu_open && canvas.hovered() {
            // The wheel changes the stroke width (or the text size), a notch a step; over a menu
            // it scrolls the menu.
            let events = ui.input(|i| i.events.clone());
            let n = crate::engine::annotator::wheel_notches(&mut self.wheel_acc, &events);
            if n != 0 {
                let before = shared.settings.draw.clone();
                crate::engine::annotator::wheel_restyle(self.an.tool, &mut shared.settings.draw, n);
                self.an.restyle_from_opts(&before, &shared.settings.draw);
            }
        }

        // ---------------- painting
        let painter = ui.painter_at(screen);
        painter.image(
            self.tex.id(),
            screen,
            Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
            Color32::WHITE,
        );
        let dim = Color32::from_black_alpha((DIM_ALPHA * 255.0) as u8);
        match self.sel {
            None => {
                painter.rect_filled(screen, 0.0, dim);
            }
            Some(s) => {
                let r = Rect::from_min_max(view.to_screen(s.min), view.to_screen(s.max));
                for part in [
                    Rect::from_min_max(screen.min, pos2(screen.max.x, r.min.y)),
                    Rect::from_min_max(pos2(screen.min.x, r.max.y), screen.max),
                    Rect::from_min_max(pos2(screen.min.x, r.min.y), pos2(r.min.x, r.max.y)),
                    Rect::from_min_max(pos2(r.max.x, r.min.y), pos2(screen.max.x, r.max.y)),
                ] {
                    painter.rect_filled(part, 0.0, dim);
                }
                // The area in the theme's colour: a 1.5 px line, and with Select its handles,
                // white circles ringed in the same colour.
                painter.rect_stroke(
                    r,
                    0.0,
                    Stroke::new(1.5, t.primary),
                    egui::StrokeKind::Inside,
                );
                if self.an.tool == Tool::Select {
                    for (_, hp) in handles(r) {
                        painter.circle(hp, 5.0, Color32::WHITE, Stroke::new(2.0, t.primary));
                    }
                }
                size_label(&painter, t, r, s, screen);
            }
        }
        self.an.paint_selection(
            &painter,
            &view,
            t,
            &shared.fonts,
            &text_measure(&shared.fonts),
        );
        self.an.paint_live(&painter, &view, &shared.fonts);
        self.an.handle_typing(
            ui,
            &view,
            &shared.fonts,
            &shared.settings.draw,
            tr(Msg::TypeHere),
        );

        // magnifier while choosing the area
        let choosing = self.sel.is_none() || matches!(self.drag, Drag::NewSel(_));
        if choosing && let Some(p) = pointer {
            self.magnifier(&painter, t, &view, p, screen);
            let (cx, cy) = (p.x, p.y);
            let guide = Stroke::new(1.0, Color32::from_white_alpha(70));
            painter.line_segment([pos2(screen.min.x, cy), pos2(screen.max.x, cy)], guide);
            painter.line_segment([pos2(cx, screen.min.y), pos2(cx, screen.max.y)], guide);
        }

        // ---------------- bars and hint, drawn by the view
        let mut switch = Switch::None;
        for a in crate::views::capture(ui, t, self, shared, screen) {
            let s = self.apply(a, &ctx, frame, shared);
            if !matches!(s, Switch::None) {
                switch = s;
            }
        }

        // ---------------- keyboard
        // Keys wait while a text is typed, a menu is open or a text field (a colour's numbers)
        // has the keyboard.
        if !typing
            && !menu_open
            && !ctx.text_edit_focused()
            && let Some(sw) = self.keys(ui, &ctx, frame, shared, pointer, &view)
        {
            switch = sw;
        }

        if shared.settings.draw.color != opts_snapshot.color
            || shared.settings.draw.width != opts_snapshot.width
        {
            shared.settings_dirty = true;
        }
        ctx.request_repaint();
        switch
    }

    /// Pointer on the capture: objects answer first (with Select a miss on empty space falls through
    /// to moving or resizing the area), drawing tools draw, and the pointer shows what a click will do.
    fn canvas_input(
        &mut self,
        ui: &Ui,
        canvas: &egui::Response,
        view: &View,
        pointer: Option<Pos2>,
        shared: &mut Shared,
    ) {
        let drawing_tool = self.sel.is_some() && self.an.tool != Tool::Select;
        let sel_screen = self
            .sel
            .map(|s| Rect::from_min_max(view.to_screen(s.min), view.to_screen(s.max)));
        let measure = text_measure(&shared.fonts);
        // Objects answer first; with Select a miss on empty space falls through to the area.
        // A drawing tool only starts inside the area, where the pointer shows the crosshair.
        // egui clears the press origin on the release frame, where clicks fire, so a click is judged
        // by where it happened.
        let pressed_at = ui.input(|i| i.pointer.press_origin()).or_else(|| {
            (canvas.clicked() || canvas.double_clicked())
                .then(|| canvas.interact_pointer_pos())
                .flatten()
        });
        let press_inside = pressed_at
            .zip(sel_screen)
            .is_none_or(|(o, r)| r.contains(o));
        let objects_answer = self.sel.is_some()
            && matches!(self.drag, Drag::None)
            && (self.an.tool == Tool::Select || press_inside);
        let outcome = if objects_answer {
            self.an
                .handle_canvas(ui, canvas, view, &measure, &mut shared.settings.draw)
        } else {
            SelectOutcome::Missed
        };
        if !drawing_tool && outcome == SelectOutcome::Missed {
            self.selection_input(ui, canvas, view, pointer);
        }
        if let Some(p) = pointer
            && (canvas.hovered() || self.an.dragging.is_some() || !matches!(self.drag, Drag::None))
        {
            // A drawing tool reaches objects only inside the area, like its clicks.
            let reachable = !drawing_tool || sel_screen.is_some_and(|r| r.contains(p));
            let (handle, over_object, over_text) = if self.an.live.is_some()
                || self.sel.is_none()
                || !matches!(self.drag, Drag::None)
                || !reachable
            {
                (None, false, false)
            } else {
                self.an.hover(view.to_img(p), view, &measure)
            };
            ui.ctx().set_cursor_icon(cursor::pick(&CursorCtx {
                tool: self.an.tool,
                over_image: sel_screen.is_some_and(|r| r.contains(p)),
                over_chrome: false,
                handle,
                over_object,
                over_text,
                typing: self.an.draft.is_some(),
                dragging: self.an.dragging.is_some(),
                space_pan: false,
                area: pointer_area(self.sel, &self.drag, p, view, !drawing_tool),
            }));
        }
    }

    /// With Select and no object selected, arrows move the area and Shift+arrows resize it.
    /// Object shortcuts run first and consume the arrows when an object is selected.
    fn nudge_area(&mut self, ui: &Ui) {
        if self.an.tool != Tool::Select {
            return;
        }
        let mut d = vec2(0.0, 0.0);
        let shift = ui.input(|i| i.modifiers.shift);
        for (k, v) in [
            (Key::ArrowLeft, vec2(-1.0, 0.0)),
            (Key::ArrowRight, vec2(1.0, 0.0)),
            (Key::ArrowUp, vec2(0.0, -1.0)),
            (Key::ArrowDown, vec2(0.0, 1.0)),
        ] {
            if ui.input(|i| i.key_pressed(k)) {
                d += v;
            }
        }
        if d != vec2(0.0, 0.0) {
            let bounds = Rect::from_min_size(Pos2::ZERO, self.img_size());
            if let Some(s) = &mut self.sel {
                if shift {
                    s.max += d;
                } else {
                    *s = s.translate(d);
                }
                *s = Rect::from_two_pos(s.min, s.max).intersect(bounds);
            }
        }
    }

    /// The cropped picture and the whole document (objects, ids, hidden ones and history) moved
    /// into the crop's coordinates, for the editor.
    fn editor_handoff(&mut self) -> Option<(RgbaImage, Doc)> {
        let sel = self.sel?;
        let x = sel.min.x.round().max(0.0) as u32;
        let y = sel.min.y.round().max(0.0) as u32;
        let w = (sel.width().round() as u32)
            .min(self.shot.image.width().saturating_sub(x))
            .max(1);
        let h = (sel.height().round() as u32)
            .min(self.shot.image.height().saturating_sub(y))
            .max(1);
        let base = image::imageops::crop_imm(&self.shot.image, x, y, w, h).to_image();
        let doc = std::mem::take(&mut self.an.doc).translated(-vec2(x as f32, y as f32));
        Some((base, doc))
    }

    fn selection_input(
        &mut self,
        ui: &Ui,
        canvas: &egui::Response,
        view: &View,
        pointer: Option<Pos2>,
    ) {
        let mods = ui.input(|i| i.modifiers);
        let bounds = Rect::from_min_size(Pos2::ZERO, self.img_size());
        if canvas.drag_started() {
            let origin = ui
                .input(|i| i.pointer.press_origin())
                .or(pointer)
                .unwrap_or_default();
            self.drag = match area_hit(self.sel, view, origin) {
                Some(h) => Drag::Adjust {
                    handle: h,
                    start_ptr: view.to_img(origin),
                    orig: self.sel.unwrap(),
                },
                None => Drag::NewSel(view.to_img(origin).clamp(bounds.min, bounds.max)),
            };
        }
        if canvas.dragged()
            && let Some(p) = pointer
        {
            let cur = view.to_img(p).clamp(bounds.min, bounds.max);
            match &self.drag {
                Drag::NewSel(start) => {
                    let (a, b) = constrain(Tool::Select, *start, cur, mods.shift, mods.alt);
                    self.sel = Some(Rect::from_two_pos(a, b).intersect(bounds));
                }
                Drag::Adjust {
                    handle,
                    start_ptr,
                    orig,
                } => {
                    let d = view.to_img(p) - *start_ptr;
                    let mut r = *orig;
                    match handle {
                        Handle::Move => {
                            r = r.translate(d);
                            let off = vec2(
                                (bounds.min.x - r.min.x).max(0.0)
                                    + (bounds.max.x - r.max.x).min(0.0),
                                (bounds.min.y - r.min.y).max(0.0)
                                    + (bounds.max.y - r.max.y).min(0.0),
                            );
                            r = r.translate(off);
                        }
                        h => {
                            if matches!(h, Handle::N | Handle::NE | Handle::NW) {
                                r.min.y += d.y;
                            }
                            if matches!(h, Handle::S | Handle::SE | Handle::SW) {
                                r.max.y += d.y;
                            }
                            if matches!(h, Handle::W | Handle::NW | Handle::SW) {
                                r.min.x += d.x;
                            }
                            if matches!(h, Handle::E | Handle::NE | Handle::SE) {
                                r.max.x += d.x;
                            }
                            r = Rect::from_two_pos(r.min, r.max).intersect(bounds);
                        }
                    }
                    self.sel = Some(r);
                }
                Drag::None => {}
            }
        }
        if canvas.drag_stopped() {
            if let Some(s) = self.sel
                && (s.width() < 3.0 || s.height() < 3.0)
            {
                self.sel = None;
            }
            self.drag = Drag::None;
        }
        // a click without a selection grabs the whole screen (like a window-less fallback)
        if canvas.double_clicked() && self.sel.is_none() {
            self.sel = Some(bounds);
        }
    }

    fn magnifier(&self, painter: &egui::Painter, t: &Tokens, view: &View, p: Pos2, screen: Rect) {
        let img = view.to_img(p);
        let (iw, ih) = (
            self.shot.image.width() as i32,
            self.shot.image.height() as i32,
        );
        let (px, py) = (img.x.floor() as i32, img.y.floor() as i32);
        if px < 0 || py < 0 || px >= iw || py >= ih {
            return;
        }
        let cells = 15.0;
        let size = 135.0;
        let mut at = p + vec2(22.0, 22.0);
        if at.x + size > screen.max.x {
            at.x = p.x - 22.0 - size;
        }
        if at.y + size + 44.0 > screen.max.y {
            at.y = p.y - 22.0 - size - 44.0;
        }
        let r = Rect::from_min_size(at, vec2(size, size));
        let half = (cells - 1.0) / 2.0;
        let uv = Rect::from_min_max(
            pos2(
                (px as f32 - half) / iw as f32,
                (py as f32 - half) / ih as f32,
            ),
            pos2(
                (px as f32 + half + 1.0) / iw as f32,
                (py as f32 + half + 1.0) / ih as f32,
            ),
        );
        painter.rect_filled(r.expand(3.0), 10.0, t.surf);
        painter.image(self.tex.id(), r, uv, Color32::WHITE);
        let cell = size / cells;
        let c = Rect::from_min_size(r.min + vec2(half * cell, half * cell), vec2(cell, cell));
        painter.rect_stroke(
            c,
            0.0,
            Stroke::new(1.5, t.primary),
            egui::StrokeKind::Outside,
        );
        painter.rect_stroke(
            r.expand(3.0),
            10.0,
            Stroke::new(1.0, t.border),
            egui::StrokeKind::Inside,
        );
        let px_col = self.shot.image.get_pixel(px as u32, py as u32).0;
        let info = Rect::from_min_size(pos2(r.min.x - 3.0, r.max.y + 8.0), vec2(size + 6.0, 38.0));
        painter.rect_filled(info, 8.0, t.surf);
        painter.rect_filled(
            Rect::from_min_size(info.min + vec2(9.0, 11.0), vec2(16.0, 16.0)),
            4.0,
            c32(px_col),
        );
        painter.text(
            info.min + vec2(32.0, 9.0),
            Align2::LEFT_TOP,
            widgets::hex(px_col),
            font(12.0, Weight::Mono),
            t.text,
        );
        painter.text(
            info.min + vec2(32.0, 24.0),
            Align2::LEFT_TOP,
            tf(Msg::CapPixel, &[&px.to_string(), &py.to_string()]),
            font(10.5, Weight::Regular),
            t.muted,
        );
    }

    fn keys(
        &mut self,
        ui: &mut Ui,
        ctx: &egui::Context,
        frame: Option<&eframe::Frame>,
        shared: &mut Shared,
        pointer: Option<Pos2>,
        view: &View,
    ) -> Option<Switch> {
        let cmd = Modifiers::COMMAND;
        // One Esc closes the capture at any stage; a text being typed takes it first, to end
        // the text.
        if ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
            return Some(Switch::Close);
        }
        if self.an.handle_history_keys(ui) {
            self.an.sync_options(&mut shared.settings.draw);
        }
        // With objects selected, Delete, Ctrl+D, arrows and Esc act on them; otherwise on the area.
        self.an.handle_object_keys(ui);
        if ui.input_mut(|i| i.consume_key(cmd, Key::A)) {
            self.sel = Some(Rect::from_min_size(Pos2::ZERO, self.img_size()));
        }
        if self.sel.is_some() {
            if ui.input_mut(|i| crate::engine::annotator::consume_command(i, Key::C)) {
                return Some(self.act_copy(shared));
            }
            if ui.input_mut(|i| i.consume_key(cmd, Key::S)) {
                return Some(self.act_save(frame, shared));
            }
            if ui.input_mut(|i| i.consume_key(cmd, Key::T)) {
                return Some(self.act_pin(shared));
            }
            if ui.input_mut(|i| {
                i.consume_key(cmd, Key::E) || i.consume_key(Modifiers::NONE, Key::Enter)
            }) {
                return Some(self.open_editor(ctx, shared));
            }
        }
        if ui.input_mut(|i| crate::engine::annotator::consume_command(i, Key::X)) {
            return Some(Switch::Close);
        }
        if self.sel.is_none() && ui.input(|i| i.key_pressed(Key::C) && !i.modifiers.any()) {
            if let Some(p) = pointer {
                let ip = view.to_img(p);
                if ip.x >= 0.0
                    && ip.y >= 0.0
                    && (ip.x as u32) < self.shot.image.width()
                    && (ip.y as u32) < self.shot.image.height()
                {
                    let c = self.shot.image.get_pixel(ip.x as u32, ip.y as u32).0;
                    crate::system::actions::copy_text(&widgets::hex(c));
                    shared.toast(tf(Msg::ColorCopied, &[&widgets::hex(c)]));
                }
            }
            return None;
        }
        if self.sel.is_some() {
            widgets::tool_shortcuts(ui, &mut self.an, &mut shared.settings.draw, true);
            self.nudge_area(ui);
        }
        None
    }

    fn act_copy(&mut self, shared: &mut Shared) -> Switch {
        self.an.commit_draft(&shared.settings.draw);
        self.flush(shared);
        if let Some(img) = self.output(shared) {
            match crate::system::actions::copy_image(&img) {
                Ok(()) => {
                    self.remember_selection(shared);
                    return if shared.settings.show_bubbles {
                        Switch::ToNotice(Box::new(crate::screens::notice::Notice::copied()))
                    } else {
                        Switch::Close
                    };
                }
                Err(e) => shared.toast(tf(Msg::CouldNotCopy, &[&e.to_string()])),
            }
        }
        Switch::None
    }

    fn act_save(&mut self, frame: Option<&eframe::Frame>, shared: &mut Shared) -> Switch {
        self.an.commit_draft(&shared.settings.draw);
        self.flush(shared);
        let Some(img) = self.output(shared) else {
            return Switch::None;
        };
        let path = if shared.settings.auto_save {
            Some(crate::system::actions::auto_path(&shared.settings.save_dir))
        } else {
            crate::system::actions::ask_save_path(&shared.settings.save_dir, frame)
        };
        let Some(path) = path else {
            return Switch::None;
        };
        match crate::system::actions::save_png(&img, &path) {
            Ok(()) => {
                if let Some(dir) = path.parent() {
                    shared.settings.save_dir = dir.to_string_lossy().into_owned();
                }
                self.remember_selection(shared);
                if shared.settings.show_bubbles {
                    Switch::ToNotice(Box::new(crate::screens::notice::Notice::saved(path)))
                } else {
                    Switch::Close
                }
            }
            Err(e) => {
                shared.toast(tf(Msg::CouldNotSave, &[&e.to_string()]));
                Switch::None
            }
        }
    }

    fn act_pin(&mut self, shared: &mut Shared) -> Switch {
        self.an.commit_draft(&shared.settings.draw);
        self.flush(shared);
        let Some(img) = self.crop() else {
            return Switch::None;
        };
        if let Some(p) = crate::system::actions::temp_png(&img) {
            let _ = crate::system::actions::spawn_self(&["--pin", &p.to_string_lossy()]);
            self.remember_selection(shared);
            return Switch::Close;
        }
        Switch::None
    }

    /// Bake pending changes right away (actions can't wait for the next frame).
    fn flush(&mut self, shared: &Shared) {
        if let Some(anns) = self.an.bake() {
            self.baked =
                crate::engine::render::render_annotations(&self.shot.image, &anns, &shared.fonts);
        }
    }
}

/// One capture window per monitor and a single area: the window that gains one becomes the
/// active one and the others drop theirs. Returns the new active index.
pub fn settle(overlays: &mut [&mut Overlay], active: usize) -> usize {
    let Some(now) = (0..overlays.len()).find(|&i| i != active && overlays[i].sel.is_some()) else {
        return active;
    };
    for (i, o) in overlays.iter_mut().enumerate() {
        if i != now {
            o.clear_area();
        }
    }
    now
}

/// The area handle (or `Move` inside the area) under screen point `p`.
fn area_hit(sel: Option<Rect>, view: &View, p: Pos2) -> Option<Handle> {
    let s = sel?;
    let r = Rect::from_min_max(view.to_screen(s.min), view.to_screen(s.max));
    for (h, hp) in handles(r) {
        if hp.distance(p) <= 9.0 {
            return Some(h);
        }
    }
    r.contains(p).then_some(Handle::Move)
}

/// Where the pointer is relative to the capture area, for `cursor::pick`. The area handles count
/// only where they work, with Select (`handles_work`); otherwise their spot is inside or outside.
fn pointer_area(sel: Option<Rect>, drag: &Drag, p: Pos2, view: &View, handles_work: bool) -> Area {
    if sel.is_none() || matches!(drag, Drag::NewSel(_)) {
        return Area::NotStarted;
    }
    let under = match drag {
        Drag::Adjust { handle, .. } => Some(*handle),
        _ => area_hit(sel, view, p),
    };
    let inside = sel.is_some_and(|s| {
        Rect::from_min_max(view.to_screen(s.min), view.to_screen(s.max)).contains(p)
    });
    match under {
        Some(Handle::Move) => Area::Inside,
        Some(h) if handles_work => Area::Handle(as_obj_handle(h)),
        _ if inside => Area::Inside,
        _ => Area::Outside,
    }
}

fn as_obj_handle(h: Handle) -> ObjHandle {
    match h {
        Handle::N => ObjHandle::N,
        Handle::S => ObjHandle::S,
        Handle::E => ObjHandle::E,
        Handle::W => ObjHandle::W,
        Handle::NE => ObjHandle::NE,
        Handle::NW => ObjHandle::NW,
        Handle::SE => ObjHandle::SE,
        Handle::SW | Handle::Move => ObjHandle::SW,
    }
}

fn handles(r: Rect) -> [(Handle, Pos2); 8] {
    let c = r.center();
    [
        (Handle::NW, r.left_top()),
        (Handle::N, pos2(c.x, r.top())),
        (Handle::NE, r.right_top()),
        (Handle::E, pos2(r.right(), c.y)),
        (Handle::SE, r.right_bottom()),
        (Handle::S, pos2(c.x, r.bottom())),
        (Handle::SW, r.left_bottom()),
        (Handle::W, pos2(r.left(), c.y)),
    ]
}

/// The area's size ("760 × 430", 12 mono) on the shell colour, 6 px over its top-left corner, or
/// inside it when the area touches the top of the screen.
fn size_label(painter: &egui::Painter, t: &Tokens, r: Rect, s: Rect, screen: Rect) {
    let text = format!("{} × {}", s.width().round(), s.height().round());
    let galley = painter.layout_no_wrap(text, font(12.0, Weight::Mono), t.text);
    let size = galley.size() + vec2(16.0, 8.0);
    let mut at = pos2(r.min.x, r.min.y - size.y - 6.0);
    if at.y < screen.min.y + 2.0 {
        at = pos2(r.min.x + 6.0, r.min.y + 6.0);
    }
    painter.rect_filled(
        Rect::from_min_size(at, size),
        6.0,
        t.shell.gamma_multiply(0.9),
    );
    painter.galley(at + vec2(8.0, 4.0), galley, t.text);
}

pub fn color_image(img: &RgbaImage) -> egui::ColorImage {
    egui::ColorImage::from_rgba_unmultiplied(
        [img.width() as usize, img.height() as usize],
        img.as_raw(),
    )
}

pub fn load_tex(
    ctx: &egui::Context,
    name: &str,
    img: &RgbaImage,
    opts: egui::TextureOptions,
) -> egui::TextureHandle {
    ctx.load_texture(name, color_image(img), opts)
}

#[cfg(test)]
mod tests {
    use super::*;

    const VIEW: View = View {
        origin: Pos2::ZERO,
        k: 1.0,
    };

    fn sel() -> Option<Rect> {
        Some(Rect::from_min_max(pos2(100.0, 100.0), pos2(300.0, 200.0)))
    }

    fn shared() -> Shared {
        Shared {
            settings: Default::default(),
            settings_dirty: false,
            fonts: std::sync::Arc::new(crate::system::fonts::FontLib::load()),
            logo: widgets::LogoCache::new(),
            toast: None,
            overrides: Default::default(),
        }
    }

    const AREA: Rect = Rect {
        min: pos2(100.0, 100.0),
        max: pos2(900.0, 700.0),
    };

    /// The 1600x900 sample with the area (100,100)-(900,700) and a filled rectangle at (300,300)-(400,400).
    fn capture(ctx: &egui::Context, sh: &Shared, tool: Tool) -> Overlay {
        let mut o = Overlay::new(ctx, crate::system::capture::sample_shot(), sh);
        o.sel = Some(AREA);
        o.an = Annotator::with_doc(
            tool,
            Doc::from_anns(vec![Ann {
                shape: Shape::Rect(pos2(300.0, 300.0), pos2(400.0, 400.0), 0.0),
                color: [255, 0, 0, 255],
                width: 4.0,
                filled: true,
            }]),
        );
        o
    }

    #[test]
    fn a_new_area_on_another_monitor_clears_the_first() {
        let ctx = egui::Context::default();
        let sh = shared();
        let area = Rect::from_min_size(pos2(10.0, 10.0), vec2(50.0, 50.0));
        let mut a = capture(&ctx, &sh, Tool::Select).with_area(area);
        let mut b = Overlay::new(&ctx, crate::system::capture::sample_shot(), &sh).beside();
        assert!(b.sel.is_none());
        // Nothing new: the first keeps its area.
        assert_eq!(settle(&mut [&mut a, &mut b], 0), 0);
        assert!(a.sel.is_some());
        // An area drawn on the second monitor takes over.
        b.sel = Some(area);
        assert_eq!(settle(&mut [&mut a, &mut b], 0), 1);
        assert!(a.sel.is_none() && b.sel.is_some());
        assert!(
            a.an.doc.objs().is_empty(),
            "what was drawn goes with the area"
        );
    }

    #[test]
    fn the_marks_dropped_with_an_area_leave_the_picture_too() {
        let ctx = egui::Context::default();
        let mut sh = shared();
        let mut a = capture(&ctx, &sh, Tool::Select);
        ui_frames(&mut a, &mut sh, vec![]);
        let plain = *a.shot.image.get_pixel(350, 350);
        assert_ne!(*a.baked.get_pixel(350, 350), plain, "the mark is baked in");
        let mut b = Overlay::new(&ctx, crate::system::capture::sample_shot(), &sh).beside();
        b.sel = Some(AREA);
        assert_eq!(settle(&mut [&mut a, &mut b], 0), 1);
        ui_frames(&mut a, &mut sh, vec![]);
        assert_eq!(
            *a.baked.get_pixel(350, 350),
            plain,
            "a later copy from this monitor would carry the old mark"
        );
    }

    #[test]
    fn the_area_follows_the_monitor_it_was_last_drawn_on() {
        let ctx = egui::Context::default();
        let sh = shared();
        let area = Rect::from_min_size(pos2(10.0, 10.0), vec2(50.0, 50.0));
        let mut a = capture(&ctx, &sh, Tool::Select).with_area(area);
        let mut b = Overlay::new(&ctx, crate::system::capture::sample_shot(), &sh).beside();
        let mut c = Overlay::new(&ctx, crate::system::capture::sample_shot(), &sh).beside();
        b.sel = Some(area);
        assert_eq!(settle(&mut [&mut a, &mut b, &mut c], 0), 1);
        assert!(a.sel.is_none() && b.sel.is_some() && c.sel.is_none());
        // Back to the first monitor: the area leaves the second one.
        a.sel = Some(area);
        assert_eq!(settle(&mut [&mut a, &mut b, &mut c], 1), 0);
        assert!(a.sel.is_some() && b.sel.is_none() && c.sel.is_none());
        c.sel = Some(area);
        assert_eq!(settle(&mut [&mut a, &mut b, &mut c], 0), 2);
        assert!(a.sel.is_none() && b.sel.is_none() && c.sel.is_some());
    }

    #[test]
    fn a_window_beside_does_not_take_the_remembered_area() {
        let ctx = egui::Context::default();
        let mut sh = shared();
        sh.settings.keep_selection = true;
        sh.settings.last_selection = Some([10.0, 10.0, 50.0, 50.0]);
        let first = Overlay::new(&ctx, crate::system::capture::sample_shot(), &sh);
        assert!(first.sel.is_some(), "the first monitor keeps it");
        let beside = Overlay::new(&ctx, crate::system::capture::sample_shot(), &sh).beside();
        assert!(beside.sel.is_none(), "only one monitor holds the area");
    }

    #[test]
    fn the_capture_slot_lives_and_goes_with_the_overlay() {
        // Becoming the editor or a notice drops the overlay: the next PrtSc may capture again.
        let ctx = egui::Context::default();
        let sh = shared();
        let name = format!("takeshot-test-{}-overlay-slot", std::process::id());
        let slot = crate::system::instance::capture_slot(&name);
        let o = Overlay::new(&ctx, crate::system::capture::sample_shot(), &sh).hold(slot);
        assert!(
            crate::system::instance::capture_slot(&name).is_none(),
            "held while open"
        );
        drop(o);
        assert!(
            crate::system::instance::capture_slot(&name).is_some(),
            "free once it is gone"
        );
    }

    fn rect_min(o: &Overlay) -> Pos2 {
        match o.an.doc.objs()[0].ann.shape {
            Shape::Rect(a, _, _) => a,
            _ => unreachable!(),
        }
    }

    /// One egui frame per event list over a full-screen canvas, after a warm-up frame.
    fn frames(
        ctx: &egui::Context,
        events: Vec<Vec<egui::Event>>,
        mut f: impl FnMut(&mut Ui, &egui::Response),
    ) {
        let screen = Rect::from_min_size(Pos2::ZERO, vec2(1600.0, 900.0));
        let canvas = |ui: &mut Ui| {
            ui.interact(
                screen,
                ui.scope_id().with("canvas"),
                Sense::click_and_drag(),
            )
        };
        let warm = egui::RawInput {
            screen_rect: Some(screen),
            ..Default::default()
        };
        ctx.run_ui(warm, |ui| {
            canvas(ui);
        })
        .textures_delta
        .clear();
        for (i, ev) in events.into_iter().enumerate() {
            let input = egui::RawInput {
                screen_rect: Some(screen),
                time: Some(0.016 * (i + 1) as f64),
                events: ev,
                ..Default::default()
            };
            ctx.run_ui(input, |ui| {
                let r = canvas(ui);
                f(ui, &r);
            })
            .textures_delta
            .clear();
        }
    }

    fn button(p: Pos2, down: bool) -> egui::Event {
        egui::Event::PointerButton {
            pos: p,
            button: egui::PointerButton::Primary,
            pressed: down,
            modifiers: Modifiers::NONE,
        }
    }

    fn click(p: Pos2) -> Vec<Vec<egui::Event>> {
        vec![
            vec![egui::Event::PointerMoved(p), button(p, true)],
            vec![button(p, false)],
            vec![],
        ]
    }

    fn drag(from: Pos2, path: &[Pos2]) -> Vec<Vec<egui::Event>> {
        let mut f = vec![vec![egui::Event::PointerMoved(from), button(from, true)]];
        f.extend(path.iter().map(|p| vec![egui::Event::PointerMoved(*p)]));
        f.push(vec![button(*path.last().unwrap(), false)]);
        f.push(vec![]);
        f
    }

    fn canvas_frames(
        o: &mut Overlay,
        sh: &mut Shared,
        ctx: &egui::Context,
        ev: Vec<Vec<egui::Event>>,
    ) {
        frames(ctx, ev, |ui, r| {
            let p = ui.input(|i| i.pointer.hover_pos());
            o.canvas_input(ui, r, &VIEW, p, sh);
        });
    }

    #[test]
    fn select_drags_an_object_and_leaves_the_area() {
        let ctx = egui::Context::default();
        let mut sh = shared();
        let mut o = capture(&ctx, &sh, Tool::Select);
        let path = [pos2(360.0, 350.0), pos2(450.0, 350.0)];
        canvas_frames(&mut o, &mut sh, &ctx, drag(pos2(350.0, 350.0), &path));
        assert_eq!(o.sel, Some(AREA));
        assert!(rect_min(&o).distance(pos2(400.0, 300.0)) < 0.01);
    }

    #[test]
    fn select_on_empty_space_moves_the_area_and_leaves_the_objects() {
        let ctx = egui::Context::default();
        let mut sh = shared();
        let mut o = capture(&ctx, &sh, Tool::Select);
        let path = [pos2(610.0, 600.0), pos2(650.0, 600.0)];
        canvas_frames(&mut o, &mut sh, &ctx, drag(pos2(600.0, 600.0), &path));
        assert_eq!(o.sel, Some(AREA.translate(vec2(50.0, 0.0))));
        assert!(rect_min(&o).distance(pos2(300.0, 300.0)) < 0.01);
    }

    #[test]
    fn click_tools_only_work_inside_the_area() {
        let ctx = egui::Context::default();
        let mut sh = shared();
        let mut o = capture(&ctx, &sh, Tool::Text);
        canvas_frames(&mut o, &mut sh, &ctx, click(pos2(50.0, 50.0)));
        assert!(o.an.draft.is_none(), "text started outside the area");
        canvas_frames(&mut o, &mut sh, &ctx, click(pos2(500.0, 500.0)));
        assert!(o.an.draft.is_some());

        let mut o = capture(&ctx, &sh, Tool::Counter);
        canvas_frames(&mut o, &mut sh, &ctx, click(pos2(50.0, 50.0)));
        assert_eq!(o.an.doc.objs().len(), 1, "step placed outside the area");
        canvas_frames(&mut o, &mut sh, &ctx, click(pos2(500.0, 500.0)));
        assert_eq!(o.an.doc.objs().len(), 2);
    }

    #[test]
    fn arrows_move_the_selected_object_or_else_the_area() {
        let ctx = egui::Context::default();
        let sh = shared();
        let mut o = capture(&ctx, &sh, Tool::Select);
        let id = o.an.doc.objs()[0].id;
        o.an.doc.select(Some(id), false);
        let right = || {
            vec![vec![egui::Event::Key {
                key: Key::ArrowRight,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Modifiers::NONE,
            }]]
        };
        let keys = |o: &mut Overlay, ui: &mut Ui| {
            o.an.handle_object_keys(ui);
            o.nudge_area(ui);
        };
        frames(&ctx, right(), |ui, _| keys(&mut o, ui));
        assert_eq!(o.sel, Some(AREA), "the area stayed while the object moved");
        assert!(rect_min(&o).distance(pos2(301.0, 300.0)) < 0.01);
        o.an.doc.select(None, false);
        frames(&ctx, right(), |ui, _| keys(&mut o, ui));
        assert_eq!(o.sel, Some(AREA.translate(vec2(1.0, 0.0))));
        assert!(rect_min(&o).distance(pos2(301.0, 300.0)) < 0.01);
    }

    #[test]
    fn apply_restyles_the_selection_and_commits_the_text() {
        let ctx = egui::Context::default();
        let mut sh = shared();
        let mut o = capture(&ctx, &sh, Tool::Select);
        let id = o.an.doc.objs()[0].id;
        o.an.pick(id, false, &mut sh.settings.draw);
        o.apply(CaptureAction::SetFill(false), &ctx, None, &mut sh);
        assert!(!o.an.doc.objs()[0].ann.filled);
        // A colour keeps the object's opacity.
        o.apply(CaptureAction::Undo, &ctx, None, &mut sh);
        o.an.doc.select(Some(id), false);
        sh.settings.draw.color[3] = 128;
        o.an.doc.begin_change();
        o.an.restyle_from_opts(
            &DrawOpts {
                color: [255, 0, 0, 255],
                ..sh.settings.draw.clone()
            },
            &sh.settings.draw,
        );
        o.apply(
            CaptureAction::SetColor([0, 200, 0, 255]),
            &ctx,
            None,
            &mut sh,
        );
        assert_eq!(o.an.doc.objs()[0].ann.color, [0, 200, 0, 128]);
        // Undo shows the style it brings back.
        o.apply(CaptureAction::Undo, &ctx, None, &mut sh);
        assert_eq!(sh.settings.draw.color, o.an.doc.objs()[0].ann.color);
        // Changing the tool keeps the text being typed.
        o.an.tool = Tool::Text;
        o.an.draft = Some(crate::engine::annotator::TextDraft {
            pos: pos2(500.0, 500.0),
            text: "Pico".into(),
            started: 0.0,
            caret: None,
        });
        let n = o.an.doc.objs().len();
        o.apply(CaptureAction::SetTool(Tool::Rect), &ctx, None, &mut sh);
        assert_eq!(o.an.doc.objs().len(), n + 1);
        assert!(o.an.draft.is_none());
    }

    #[test]
    fn the_bars_hide_while_the_area_is_resized() {
        let ctx = egui::Context::default();
        let sh = shared();
        let mut o = capture(&ctx, &sh, Tool::Select);
        assert!(o.bars_shown());
        o.drag = Drag::Adjust {
            handle: Handle::SE,
            start_ptr: pos2(900.0, 700.0),
            orig: AREA,
        };
        assert!(!o.bars_shown());
        o.drag = Drag::NewSel(pos2(10.0, 10.0));
        assert!(!o.bars_shown());
    }

    #[test]
    fn the_bars_hide_while_the_text_frame_is_dragged_and_not_while_typing() {
        let ctx = egui::Context::default();
        let typing = |sh: &Shared| {
            let mut o = capture(&ctx, sh, Tool::Text);
            o.an.draft = Some(crate::engine::annotator::TextDraft {
                pos: pos2(500.0, 500.0),
                text: "Pico".into(),
                started: 1.0,
                caret: None,
            });
            o
        };
        let mut sh = shared();
        let mut o = typing(&sh);
        ui_frames(&mut o, &mut sh, vec![vec![egui::Event::Text("X".into())]]);
        assert_eq!(o.an.draft.as_ref().map(|d| d.text.as_str()), Some("PicoX"));
        assert!(o.bars_shown(), "typing alone leaves the bars up");

        // Just left of the text: on its dashed frame, outside the letters.
        let at = pos2(494.0, 506.0);
        let grab = |release: bool| {
            let mut f = vec![
                vec![egui::Event::PointerMoved(at)],
                vec![],
                vec![button(at, true)],
                vec![egui::Event::PointerMoved(at + vec2(20.0, 15.0))],
                vec![egui::Event::PointerMoved(at + vec2(40.0, 30.0))],
            ];
            if release {
                f.push(vec![button(at + vec2(40.0, 30.0), false)]);
                f.push(vec![]);
            }
            f
        };
        let mut o = typing(&sh);
        ui_frames(&mut o, &mut sh, grab(false));
        let moved = o.an.draft.as_ref().expect("still typing").pos - pos2(500.0, 500.0);
        assert!(moved.length() > 10.0, "the frame took the drag: {moved:?}");
        assert!(!o.bars_shown(), "hidden while the frame is dragged");

        let mut o = typing(&sh);
        ui_frames(&mut o, &mut sh, grab(true));
        assert!(o.an.draft.is_some(), "still typing after the drop");
        assert!(o.bars_shown(), "back once the frame is let go");
    }

    /// Full frames of the capture (its own canvas, bars and keys) on the 1600 x 900 sample;
    /// returns the last switch that was not `None`.
    fn ui_frames(o: &mut Overlay, sh: &mut Shared, events: Vec<Vec<egui::Event>>) -> Switch {
        ui_frames_with(o, sh, events, None)
    }

    /// `ui_frames` with the popup `menu` open before every frame, as a view's menu is.
    fn ui_frames_with(
        o: &mut Overlay,
        sh: &mut Shared,
        events: Vec<Vec<egui::Event>>,
        menu: Option<egui::Id>,
    ) -> Switch {
        let ctx = egui::Context::default();
        sh.fonts.install(&ctx);
        let t = crate::ui::tokens::tokens(crate::ui::tokens::ThemeId::TakeUi, true);
        let screen = Rect::from_min_size(Pos2::ZERO, vec2(1600.0, 900.0));
        let mut last = Switch::None;
        for (i, ev) in [vec![], vec![]].into_iter().chain(events).enumerate() {
            let input = egui::RawInput {
                screen_rect: Some(screen),
                time: Some(0.05 * i as f64),
                events: ev,
                ..Default::default()
            };
            if let Some(id) = menu {
                egui::Popup::open_id(&ctx, id);
            }
            ctx.run_ui(input, |ui| {
                let s = o.ui(ui, None, &t, sh);
                if !matches!(s, Switch::None) {
                    last = s;
                }
            })
            .textures_delta
            .clear();
        }
        last
    }

    fn esc() -> egui::Event {
        egui::Event::Key {
            key: Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        }
    }

    #[test]
    fn one_esc_closes_the_capture_at_any_stage() {
        let ctx = egui::Context::default();
        // A drawing tool in hand.
        let mut sh = shared();
        let mut o = capture(&ctx, &sh, Tool::Rect);
        assert!(matches!(
            ui_frames(&mut o, &mut sh, vec![vec![esc()]]),
            Switch::Close
        ));
        // An object selected with Select.
        let mut sh = shared();
        let mut o = capture(&ctx, &sh, Tool::Select);
        let id = o.an.doc.objs()[0].id;
        o.an.doc.select(Some(id), false);
        assert!(matches!(
            ui_frames(&mut o, &mut sh, vec![vec![esc()]]),
            Switch::Close
        ));
        // No area yet.
        let mut sh = shared();
        let mut o = Overlay::new(&ctx, crate::system::capture::sample_shot(), &sh);
        assert!(matches!(
            ui_frames(&mut o, &mut sh, vec![vec![esc()]]),
            Switch::Close
        ));
    }

    #[test]
    fn esc_on_an_open_menu_keeps_the_capture() {
        let ctx = egui::Context::default();
        let mut sh = shared();
        let mut o = capture(&ctx, &sh, Tool::Text);
        let s = ui_frames_with(
            &mut o,
            &mut sh,
            vec![vec![esc()]],
            Some(egui::Id::unique("a-menu")),
        );
        assert!(
            matches!(s, Switch::None),
            "Esc closes the menu, not the capture"
        );
    }

    #[test]
    fn letters_do_not_switch_tools_while_typing_in_the_capture() {
        let ctx = egui::Context::default();
        let mut sh = shared();
        let mut o = capture(&ctx, &sh, Tool::Text);
        o.an.draft = Some(crate::engine::annotator::TextDraft {
            pos: pos2(500.0, 500.0),
            text: "Pico".into(),
            started: 0.0,
            caret: None,
        });
        let key = |pressed| egui::Event::Key {
            key: Key::R,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers: Modifiers::NONE,
        };
        ui_frames(
            &mut o,
            &mut sh,
            vec![vec![key(true), egui::Event::Text("r".into()), key(false)]],
        );
        assert_eq!(o.an.tool, Tool::Text);
        assert_eq!(o.an.draft.as_ref().map(|d| d.text.as_str()), Some("Picor"));
    }

    fn cut_with(held: Modifiers) -> Vec<Vec<egui::Event>> {
        vec![vec![egui::Event::ModifiersChanged(held), egui::Event::Cut]]
    }

    #[test]
    fn ctrl_x_closes_the_capture() {
        let ctx = egui::Context::default();
        let mut sh = shared();
        let mut o = capture(&ctx, &sh, Tool::Rect);
        let s = ui_frames(&mut o, &mut sh, cut_with(Modifiers::COMMAND));
        assert!(matches!(s, Switch::Close));
    }

    #[test]
    fn shift_delete_keeps_the_capture() {
        let ctx = egui::Context::default();
        let mut sh = shared();
        let mut o = capture(&ctx, &sh, Tool::Rect);
        let s = ui_frames(&mut o, &mut sh, cut_with(Modifiers::SHIFT));
        assert!(matches!(s, Switch::None));
    }

    #[test]
    fn ctrl_x_in_a_text_being_typed_keeps_the_capture() {
        let ctx = egui::Context::default();
        let mut sh = shared();
        let mut o = capture(&ctx, &sh, Tool::Text);
        o.an.draft = Some(crate::engine::annotator::TextDraft {
            pos: pos2(500.0, 500.0),
            text: "Pico".into(),
            started: 0.0,
            caret: None,
        });
        let s = ui_frames(&mut o, &mut sh, cut_with(Modifiers::COMMAND));
        assert!(matches!(s, Switch::None));
        assert_eq!(o.an.draft.as_ref().map(|d| d.text.as_str()), Some("Pico"));
    }

    #[test]
    fn a_wheel_notch_changes_the_width_by_one() {
        let ctx = egui::Context::default();
        let mut sh = shared();
        sh.settings.draw.width = 3.0;
        let mut o = capture(&ctx, &sh, Tool::Rect);
        let at = pos2(600.0, 600.0);
        let wheel = egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Line,
            delta: vec2(0.0, 1.0),
            modifiers: Modifiers::NONE,
            phase: egui::TouchPhase::Move,
            source: egui::MouseWheelSource::Unknown,
        };
        ui_frames(
            &mut o,
            &mut sh,
            vec![
                vec![egui::Event::PointerMoved(at)],
                vec![wheel],
                vec![],
                vec![],
                vec![],
            ],
        );
        assert_eq!(sh.settings.draw.width, 4.0);
    }

    #[test]
    fn the_wheel_over_an_open_menu_leaves_the_width() {
        let ctx = egui::Context::default();
        let mut sh = shared();
        sh.settings.draw.width = 3.0;
        let mut o = capture(&ctx, &sh, Tool::Rect);
        let at = pos2(600.0, 600.0);
        let wheel = egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Line,
            delta: vec2(0.0, 1.0),
            modifiers: Modifiers::NONE,
            phase: egui::TouchPhase::Move,
            source: egui::MouseWheelSource::Unknown,
        };
        ui_frames_with(
            &mut o,
            &mut sh,
            vec![vec![egui::Event::PointerMoved(at)], vec![wheel], vec![]],
            Some(egui::Id::unique("a-menu")),
        );
        assert_eq!(
            sh.settings.draw.width, 3.0,
            "the menu scrolls, the width stays"
        );
    }

    #[test]
    fn esc_while_typing_only_ends_the_text() {
        let ctx = egui::Context::default();
        let mut sh = shared();
        let mut o = capture(&ctx, &sh, Tool::Text);
        o.an.draft = Some(crate::engine::annotator::TextDraft {
            pos: pos2(500.0, 500.0),
            text: "Pico".into(),
            started: 0.0,
            caret: None,
        });
        let n = o.an.doc.objs().len();
        let s = ui_frames(&mut o, &mut sh, vec![vec![esc()]]);
        assert!(matches!(s, Switch::None), "the capture stays");
        assert!(o.an.draft.is_none());
        assert_eq!(o.an.doc.objs().len(), n + 1, "the text is kept");
    }

    #[test]
    fn edit_hands_the_whole_document_to_the_editor() {
        let ctx = egui::Context::default();
        let sh = shared();
        let mut o = capture(&ctx, &sh, Tool::Select);
        let hidden =
            o.an.doc
                .add(Ann {
                    shape: Shape::Ellipse(pos2(500.0, 500.0), pos2(600.0, 600.0)),
                    color: [0, 0, 255, 255],
                    width: 3.0,
                    filled: false,
                })
                .unwrap();
        o.an.doc.set_hidden(hidden, true);
        let (base, mut doc) = o.editor_handoff().unwrap();
        assert_eq!((base.width(), base.height()), (800, 600));
        assert_eq!(doc.objs().len(), 2);
        assert!(doc.objs().iter().any(|x| x.id == hidden && x.hidden));
        match doc.objs()[0].ann.shape {
            Shape::Rect(a, _, _) => assert!(a.distance(pos2(200.0, 200.0)) < 0.01),
            _ => unreachable!(),
        }
        assert!(doc.undo(), "the history came along");
    }

    #[test]
    fn pointer_area_follows_the_capture_state() {
        let idle = Drag::None;
        let at = |p: Pos2, select: bool| pointer_area(sel(), &idle, p, &VIEW, select);
        assert_eq!(
            pointer_area(None, &idle, pos2(5.0, 5.0), &VIEW, true),
            Area::NotStarted
        );
        assert_eq!(
            pointer_area(
                sel(),
                &Drag::NewSel(Pos2::ZERO),
                pos2(150.0, 150.0),
                &VIEW,
                true
            ),
            Area::NotStarted
        );
        assert_eq!(at(pos2(200.0, 150.0), true), Area::Inside);
        assert_eq!(at(pos2(300.0, 200.0), true), Area::Handle(ObjHandle::SE));
        assert_eq!(at(pos2(200.0, 100.0), true), Area::Handle(ObjHandle::N));
        // Area handles only work with Select; with a drawing tool their spot is inside or outside.
        assert_eq!(at(pos2(300.0, 200.0), false), Area::Inside);
        assert_eq!(at(pos2(305.0, 203.0), false), Area::Outside);
        assert_eq!(at(pos2(500.0, 500.0), true), Area::Outside);
        let adjusting = Drag::Adjust {
            handle: Handle::E,
            start_ptr: Pos2::ZERO,
            orig: sel().unwrap(),
        };
        assert_eq!(
            pointer_area(sel(), &adjusting, pos2(500.0, 500.0), &VIEW, true),
            Area::Handle(ObjHandle::E)
        );
    }
}
