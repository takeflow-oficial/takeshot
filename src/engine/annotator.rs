//! Drawing on a picture: the same logic runs on the capture overlay and in the editor.
//! Annotations live in a [`Doc`] of objects that can be selected, moved, resized and restyled.

use crate::engine::doc::{Doc, Handle, ObjId, Restyle, TextMeasure, ann_bounds};
use crate::engine::model::*;
use crate::engine::render::text_box_padding;
use crate::system::fonts::{self, FontLib};
use crate::system::settings::DrawOpts;
use crate::ui::kit::selection_marker;
use crate::ui::tokens::Tokens;
use egui::{Color32, Painter, Pos2, Response, Shape as EShape, Stroke, Ui, Vec2, vec2};

/// How far from a handle, in screen points, a press still grabs it.
const HANDLE_REACH: f32 = 7.0;
/// How far from a stroke, in screen points, a click still picks its object.
const HIT_REACH: f32 = 4.0;

/// Maps picture pixels to screen points: screen = origin + px * k.
#[derive(Clone, Copy, Debug)]
pub struct View {
    pub origin: Pos2,
    pub k: f32,
}

impl View {
    pub fn to_screen(self, p: Pos2) -> Pos2 {
        self.origin + p.to_vec2() * self.k
    }
    pub fn to_img(self, s: Pos2) -> Pos2 {
        ((s - self.origin) / self.k).to_pos2()
    }
}

pub struct TextDraft {
    pub pos: Pos2,
    pub text: String,
    /// Names the draft's text field: each draft opened gets its own number, so the field keeps its
    /// caret while the draft moves and a reopened text never takes another's state.
    pub started: f64,
    /// Where the caret goes when the field opens (a character index); `None` leaves it to the
    /// field (the end).
    pub caret: Option<usize>,
}

/// A number for each draft opened in this process (`TextDraft::started`): a capture's text and
/// the editor's next one never share a text field, nor its undo.
fn next_draft() -> f64 {
    static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed) as f64
}

/// The character index nearest `rel` (picture px from the text's top-left) in `text`: the line
/// under it, then the boundary between letters nearest it.
fn caret_at(text: &str, font: usize, size: f32, rel: Vec2, m: TextMeasure) -> usize {
    // One line's step, line gap included: two lines' height less one's.
    let line_h = (m("A\nA", font, size, false).y - m("A", font, size, false).y).max(1.0);
    let line = (rel.y / line_h).floor().max(0.0) as usize;
    let mut before = 0;
    let lines: Vec<&str> = text.split('\n').collect();
    let last = lines.len().saturating_sub(1);
    for (i, l) in lines.iter().enumerate() {
        if i < line.min(last) {
            before += l.chars().count() + 1;
            continue;
        }
        let chars: Vec<char> = l.chars().collect();
        let nearest = (0..=chars.len())
            .min_by(|a, b| {
                let w = |k: usize| m(&chars[..k].iter().collect::<String>(), font, size, false).x;
                (w(*a) - rel.x).abs().total_cmp(&(w(*b) - rel.x).abs())
            })
            .unwrap_or(0);
        return before + nearest;
    }
    text.chars().count()
}

/// An object drag in progress: the whole selection, or one handle of one object.
#[derive(Clone, Copy, Debug)]
pub enum Drag {
    Move { last: Pos2 },
    Resize { id: ObjId, handle: Handle },
}

/// Whether the selection took this pointer interaction, or left it to drawing or the capture area.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SelectOutcome {
    Consumed,
    Missed,
}

pub struct Annotator {
    pub doc: Doc,
    pub tool: Tool,
    pub live: Option<Ann>,
    start: Option<Pos2>,
    pub draft: Option<TextDraft>,
    /// The text being typed is moved by its dashed frame this frame (set by `handle_typing`).
    moving_draft: bool,
    pub dragging: Option<Drag>,
    /// Revision and left-out objects of the last baked picture.
    baked: Option<(u64, Vec<ObjId>)>,
}

impl Annotator {
    /// An empty picture: the untouched screenshot is already the baked picture.
    pub fn new(tool: Tool) -> Self {
        let mut an = Self::with_doc(tool, Doc::new());
        an.baked = Some((an.doc.revision(), Vec::new()));
        an
    }

    /// A picture that arrives with objects; the first `bake` renders them.
    pub fn with_doc(tool: Tool, doc: Doc) -> Self {
        Self {
            doc,
            tool,
            live: None,
            start: None,
            draft: None,
            moving_draft: false,
            dragging: None,
            baked: None,
        }
    }

    /// A shape is being drawn, an object dragged or a text moved by its frame: the floating bars
    /// step aside until it is let go. Typing alone is not busy.
    pub fn busy(&self) -> bool {
        self.live.is_some() || self.dragging.is_some() || self.moving_draft
    }

    pub fn next_counter(&self) -> u32 {
        self.doc.next_counter()
    }

    pub fn commit(&mut self, a: Ann) {
        self.doc.add(a);
    }

    pub fn undo(&mut self) {
        if self.draft.take().is_some() {
            return;
        }
        self.doc.undo();
    }

    pub fn redo(&mut self) {
        self.doc.redo();
    }

    /// Objects left out of the baked picture: the selection while it is dragged, painted live instead.
    pub fn excluded_from_bake(&self) -> Vec<ObjId> {
        if self.dragging.is_some() {
            self.doc.selected().to_vec()
        } else {
            Vec::new()
        }
    }

    /// The annotations to render when the baked picture is out of date, once per change.
    /// During a drag only its start re-bakes, to take the dragged objects out; moves are painted live.
    /// `Some` marks the picture as up to date, so the caller must render what it returns.
    pub fn bake(&mut self) -> Option<Vec<Ann>> {
        let excluded = self.excluded_from_bake();
        let rev = self.doc.revision();
        let fresh = self
            .baked
            .as_ref()
            .is_some_and(|(r, ex)| *ex == excluded && (self.dragging.is_some() || *r == rev));
        if fresh {
            return None;
        }
        let anns = self.doc.visible_anns(&excluded);
        self.baked = Some((rev, excluded));
        Some(anns)
    }

    /// The objects the last baked picture left out, which `paint_selection` paints live. It follows the
    /// bake, not the drag, so an object is never shown twice or missing on the frames a drag starts or ends.
    pub fn painted_live(&self) -> &[ObjId] {
        self.baked
            .as_ref()
            .map_or(&[], |(_, left_out)| left_out.as_slice())
    }

    pub fn commit_draft(&mut self, opts: &DrawOpts) {
        if let Some(d) = self.draft.take() {
            self.commit(Ann {
                shape: Shape::Text {
                    pos: d.pos,
                    text: d.text,
                    font: opts.font,
                    size: opts.font_size,
                    boxed: opts.text_box,
                },
                color: opts.color,
                width: opts.width,
                filled: false,
            });
        }
    }

    pub fn set_tool(&mut self, t: Tool, opts: &mut DrawOpts) {
        self.commit_draft(opts);
        self.tool = t;
        if t != Tool::Select {
            opts.last_tool = t;
        }
    }

    fn new_shape(&self, p: Pos2, opts: &DrawOpts) -> Shape {
        match self.tool {
            Tool::Pen => Shape::Pen(vec![p]),
            Tool::Marker => Shape::Marker(vec![p]),
            Tool::Line => Shape::Line(p, p),
            Tool::Arrow => Shape::Arrow(p, p),
            Tool::Rect => Shape::Rect(p, p, opts.radius),
            Tool::Ellipse => Shape::Ellipse(p, p),
            Tool::Blur => Shape::Blur(p, p, opts.blur_mode),
            _ => Shape::Line(p, p),
        }
    }

    /// Pointer handling for drawing tools on a canvas response.
    pub fn handle_pointer(&mut self, ui: &Ui, resp: &Response, view: &View, opts: &DrawOpts) {
        let mods = ui.input(|i| i.modifiers);
        let (shift, alt) = (mods.shift, mods.alt);
        let pointer = ui.input(|i| i.pointer.interact_pos());
        match self.tool {
            Tool::Select => {}
            Tool::Text => {
                if (resp.clicked() || resp.drag_started())
                    && let Some(p) = pointer
                {
                    self.commit_draft(opts);
                    // The text being typed takes the options: nothing else may follow them.
                    self.doc.select(None, false);
                    self.draft = Some(TextDraft {
                        pos: view.to_img(p),
                        text: String::new(),
                        started: next_draft(),
                        caret: None,
                    });
                }
            }
            Tool::Counter => {
                if resp.clicked()
                    && let Some(p) = pointer
                {
                    self.commit_draft(opts);
                    let n = self.next_counter();
                    self.commit(Ann {
                        shape: Shape::Counter {
                            center: view.to_img(p),
                            n,
                        },
                        color: opts.color,
                        width: opts.width,
                        filled: true,
                    });
                }
            }
            _ => {
                if resp.drag_started() {
                    let origin = ui.input(|i| i.pointer.press_origin()).or(pointer);
                    if let Some(o) = origin {
                        self.commit_draft(opts);
                        let p = view.to_img(o);
                        self.start = Some(p);
                        self.live = Some(Ann {
                            shape: self.new_shape(p, opts),
                            color: opts.color,
                            width: opts.width,
                            filled: opts.filled,
                        });
                    }
                }
                if resp.dragged()
                    && let (Some(start), Some(cur), Some(live)) =
                        (self.start, pointer, self.live.as_mut())
                {
                    let cur = view.to_img(cur);
                    let (a, b) = constrain(self.tool, start, cur, shift, alt);
                    match &mut live.shape {
                        Shape::Pen(pts) | Shape::Marker(pts) => {
                            if shift {
                                *pts = vec![a, b];
                            } else {
                                let last = *pts.last().unwrap();
                                if last.distance(cur) * view.k >= 1.5 {
                                    pts.push(cur);
                                }
                            }
                        }
                        Shape::Line(p0, p1)
                        | Shape::Arrow(p0, p1)
                        | Shape::Rect(p0, p1, _)
                        | Shape::Ellipse(p0, p1)
                        | Shape::Blur(p0, p1, _) => {
                            *p0 = a;
                            *p1 = b;
                        }
                        _ => {}
                    }
                }
                if resp.drag_stopped() {
                    if let Some(a) = self.live.take() {
                        self.commit(a);
                    }
                    self.start = None;
                }
            }
        }
    }

    /// Everything a press, drag or click on the picture does: a double click re-opens a text, then
    /// handles and objects take the pointer, then the active drawing tool draws. `Missed` only with
    /// Select on empty space, so the capture can move its area instead.
    pub fn handle_canvas(
        &mut self,
        ui: &Ui,
        resp: &Response,
        view: &View,
        m: TextMeasure,
        opts: &mut DrawOpts,
    ) -> SelectOutcome {
        // A double click edits a text with any tool; the Text tool edits one with a single click
        // (or a press), so it never starts a new text on top of it.
        let press = if resp.drag_started() {
            ui.input(|i| i.pointer.press_origin())
        } else if resp.clicked() || resp.double_clicked() {
            ui.input(|i| i.pointer.interact_pos())
        } else {
            None
        };
        let text_tool_press = self.tool == Tool::Text && (resp.clicked() || resp.drag_started());
        if (resp.double_clicked() || text_tool_press)
            && let Some(p) = press
            && self.text_at(view.to_img(p), m)
            && self.edit_text_at(view.to_img(p), m, opts)
        {
            return SelectOutcome::Consumed;
        }
        if self.handle_select_pointer(ui, resp, view, m, opts) == SelectOutcome::Consumed {
            return SelectOutcome::Consumed;
        }
        if self.tool == Tool::Select {
            return SelectOutcome::Missed;
        }
        self.handle_pointer(ui, resp, view, opts);
        SelectOutcome::Consumed
    }

    /// Ctrl+Z undoes; Ctrl+Y or Ctrl+Shift+Z redo. Redo is matched first because egui also matches
    /// Ctrl+Shift+Z against the plain Ctrl+Z pattern. Returns whether a key was used.
    pub fn handle_history_keys(&mut self, ui: &Ui) -> bool {
        use egui::{Key, Modifiers as M};
        let (redo, undo) = ui.input_mut(|i| {
            let redo =
                i.consume_key(M::COMMAND, Key::Y) || i.consume_key(M::COMMAND | M::SHIFT, Key::Z);
            let undo = !redo && i.consume_key(M::COMMAND, Key::Z);
            (redo, undo)
        });
        if redo {
            self.redo();
        }
        if undo {
            self.undo();
        }
        redo || undo
    }

    /// Object shortcuts, only with a selection: Delete or Backspace deletes, Ctrl+D copies 12 px
    /// away, arrows nudge 1 px (Shift 10 px, one undo step per press), Esc clears the selection.
    /// Returns whether a key was used, so the screen can give the rest its own meaning.
    pub fn handle_object_keys(&mut self, ui: &Ui) -> bool {
        use egui::{Key, Modifiers as M};
        if self.doc.selected().is_empty() {
            return false;
        }
        let mut used = false;
        ui.input_mut(|i| {
            if i.consume_key(M::COMMAND, Key::D) {
                self.doc.duplicate_selected(vec2(12.0, 12.0));
                used = true;
            }
            if i.consume_key(M::NONE, Key::Delete) || i.consume_key(M::NONE, Key::Backspace) {
                self.doc.delete_selected();
                used = true;
            }
            for (k, dir) in [
                (Key::ArrowLeft, vec2(-1.0, 0.0)),
                (Key::ArrowRight, vec2(1.0, 0.0)),
                (Key::ArrowUp, vec2(0.0, -1.0)),
                (Key::ArrowDown, vec2(0.0, 1.0)),
            ] {
                // A held key repeats: only its first press opens an undo step.
                let repeat = i.events.iter().any(|e| {
                    matches!(e, egui::Event::Key { key, pressed: true, repeat: true, .. } if *key == k)
                });
                // Shift first: a plain pattern also matches Shift+key in egui.
                let step = if i.consume_key(M::SHIFT, k) {
                    10.0
                } else if i.consume_key(M::NONE, k) {
                    1.0
                } else {
                    continue;
                };
                if !self.doc.selected().is_empty() {
                    if !repeat {
                        self.doc.begin_change();
                    }
                    self.doc.translate_selected(dir * step);
                    used = true;
                }
            }
            if !self.doc.selected().is_empty() && i.consume_key(M::NONE, Key::Escape) {
                self.doc.select(None, false);
                used = true;
            }
        });
        used
    }

    /// Whether a visible text object is at `p_img`.
    fn text_at(&self, p_img: Pos2, m: TextMeasure) -> bool {
        self.doc.hit(p_img, 0.0, m).is_some_and(|id| {
            self.doc
                .objs()
                .iter()
                .any(|o| o.id == id && matches!(o.ann.shape, Shape::Text { .. }))
        })
    }

    /// What is under the pointer: a handle of the selection, whether an object itself is (any
    /// tool moves it), and whether that object is a text (the Text tool edits it).
    pub fn hover(&self, p_img: Pos2, view: &View, m: TextMeasure) -> (Option<Handle>, bool, bool) {
        let handle = match self.dragging {
            Some(Drag::Resize { handle, .. }) => Some(handle),
            _ => self
                .doc
                .handle_at(p_img, HANDLE_REACH / view.k, m)
                .map(|(_, h)| h),
        };
        let over = matches!(self.dragging, Some(Drag::Move { .. }))
            || self.doc.hit(p_img, HIT_REACH / view.k, m).is_some();
        (handle, over, self.text_at(p_img, m))
    }

    /// Selection, moving and resizing on a canvas response, with any tool: the selection's handles
    /// resize, and pressing on an object itself (its line, its fill, its text; never its box) picks
    /// it and moves it. `Missed` leaves the interaction to drawing (`handle_pointer`) or, with
    /// Select, to the capture area.
    pub fn handle_select_pointer(
        &mut self,
        ui: &Ui,
        resp: &Response,
        view: &View,
        m: TextMeasure,
        opts: &mut DrawOpts,
    ) -> SelectOutcome {
        let (shift, pointer, origin) = ui.input(|i| {
            (
                i.modifiers.shift,
                i.pointer.interact_pos(),
                i.pointer.press_origin(),
            )
        });
        if let Some(drag) = self.dragging {
            if resp.dragged()
                && let Some(p) = pointer
            {
                self.drag_to(drag, view.to_img(p), shift, m);
            } else {
                self.dragging = None;
            }
            return SelectOutcome::Consumed;
        }
        let started = resp.drag_started();
        let at = if started {
            origin.or(pointer)
        } else if resp.clicked() {
            pointer
        } else {
            None
        };
        let Some(at) = at else {
            return SelectOutcome::Missed;
        };
        let p = view.to_img(at);
        let drag = if let Some((id, handle)) = self.doc.handle_at(p, HANDLE_REACH / view.k, m) {
            Drag::Resize { id, handle }
        } else if let Some(id) = self.doc.hit(p, HIT_REACH / view.k, m) {
            // The text being typed is kept first, in its own style.
            self.commit_draft(opts);
            // Shift adds to the selection with Select; with a drawing tool it constrains drawing.
            if shift && self.tool == Tool::Select {
                self.doc.select(Some(id), true);
            } else if !started || !self.doc.selected().contains(&id) {
                // Dragging an object of a multi-selection moves them all; a click picks just it.
                self.doc.select(Some(id), false);
            }
            self.copy_style(id, opts);
            Drag::Move { last: p }
        } else {
            // Off every object: Select clears the selection; a drawing tool draws, and its new
            // shape takes the selection.
            if !shift && self.tool == Tool::Select {
                self.doc.select(None, false);
            }
            return SelectOutcome::Missed;
        };
        if started {
            self.doc.begin_change();
            self.dragging = Some(drag);
            if let Some(cur) = pointer {
                self.drag_to(drag, view.to_img(cur), shift, m);
            }
        }
        SelectOutcome::Consumed
    }

    fn drag_to(&mut self, drag: Drag, p: Pos2, keep_ratio: bool, m: TextMeasure) {
        match drag {
            Drag::Move { last } => {
                self.doc.translate_selected(p - last);
                self.dragging = Some(Drag::Move { last: p });
            }
            Drag::Resize { id, handle } => self.doc.resize(id, handle, p, keep_ratio, m),
        }
    }

    /// The options bar shows the style of what was just picked, so the next change applies to it.
    fn copy_style(&self, id: ObjId, opts: &mut DrawOpts) {
        let Some(o) = self.doc.objs().iter().find(|o| o.id == id) else {
            return;
        };
        opts.color = o.ann.color;
        match o.ann.shape {
            Shape::Text {
                font, size, boxed, ..
            } => {
                opts.font = font;
                opts.font_size = size;
                opts.text_box = boxed;
            }
            Shape::Rect(_, _, radius) => {
                opts.width = o.ann.width;
                opts.filled = o.ann.filled;
                opts.radius = radius;
            }
            Shape::Ellipse(..) => {
                opts.width = o.ann.width;
                opts.filled = o.ann.filled;
            }
            Shape::Blur(_, _, mode) => opts.blur_mode = mode,
            _ => opts.width = o.ann.width,
        }
    }

    /// Shows the first selected object's style in the options, as after undo or redo, which can
    /// bring back another style.
    pub fn sync_options(&self, opts: &mut DrawOpts) {
        if let Some(id) = self.doc.selected().first() {
            self.copy_style(*id, opts);
        }
    }

    /// Picks `id` from the object list: switches to Select, selects it (`additive` adds it) and
    /// shows its style in the options.
    pub fn pick(&mut self, id: ObjId, additive: bool, opts: &mut DrawOpts) {
        self.set_tool(Tool::Select, opts);
        self.doc.select(Some(id), additive);
        if self.doc.selected().contains(&id) {
            self.copy_style(id, opts);
        }
    }

    /// The bounds of the visible selected objects, in picture pixels.
    pub fn selection_bounds(&self, m: TextMeasure) -> Option<egui::Rect> {
        self.doc
            .objs()
            .iter()
            .filter(|o| !o.hidden && self.doc.selected().contains(&o.id))
            .map(|o| ann_bounds(&o.ann, m))
            .reduce(|a, b| a.union(b))
    }

    /// The kind whose options the bar and the panel show: the selected object's, whatever the
    /// tool (any tool picks objects); with nothing selected, the drawing tool's; with Select and
    /// nothing selected, none.
    pub fn bar_tool(&self) -> Option<Tool> {
        let picked = self.doc.selected().first().and_then(|id| {
            self.doc
                .objs()
                .iter()
                .find(|o| o.id == *id)
                .map(|o| o.ann.shape.tool())
        });
        picked.or((self.tool != Tool::Select).then_some(self.tool))
    }

    /// Applies to the selection what the options bar changed between `before` and `after`.
    pub fn restyle_from_opts(&mut self, before: &DrawOpts, after: &DrawOpts) {
        fn changed<T: PartialEq + Copy>(a: T, b: T) -> Option<T> {
            (a != b).then_some(b)
        }
        let rgb = |c: [u8; 4]| [c[0], c[1], c[2]];
        let r = Restyle {
            rgb: changed(rgb(before.color), rgb(after.color)),
            alpha: changed(before.color[3], after.color[3]),
            width: changed(before.width, after.width),
            filled: changed(before.filled, after.filled),
            font: changed(before.font, after.font),
            font_size: changed(before.font_size, after.font_size),
            text_box: changed(before.text_box, after.text_box),
            blur_mode: changed(before.blur_mode, after.blur_mode),
            radius: changed(before.radius, after.radius),
        };
        if r != Restyle::default() {
            self.doc.restyle_selected(&r);
        }
    }

    /// A text object at `p_img` (a double click, or the Text tool's click): takes it out of the
    /// picture and reopens it for typing with the caret where it was clicked, and its font,
    /// size, background and colour back in the options.
    pub fn edit_text_at(&mut self, p_img: Pos2, m: TextMeasure, opts: &mut DrawOpts) -> bool {
        self.commit_draft(opts);
        let Some(id) = self.doc.hit(p_img, 0.0, m) else {
            return false;
        };
        let is_text = self
            .doc
            .objs()
            .iter()
            .any(|o| o.id == id && matches!(o.ann.shape, Shape::Text { .. }));
        if !is_text {
            return false;
        }
        let Some(Ann {
            shape:
                Shape::Text {
                    pos,
                    text,
                    font,
                    size,
                    boxed,
                },
            color,
            ..
        }) = self.doc.take_for_edit(id)
        else {
            return false;
        };
        opts.font = font;
        opts.font_size = size;
        opts.text_box = boxed;
        opts.color = color;
        self.doc.select(None, false);
        let caret = caret_at(&text, font, size, p_img - pos, m);
        self.draft = Some(TextDraft {
            pos,
            text,
            started: next_draft(),
            caret: Some(caret),
        });
        true
    }

    /// The text being typed, as a text field over the picture in the text's own font, size and
    /// colour: caret, arrows, Shift+arrows, mouse selection, Ctrl+A, copy and paste. Enter keeps
    /// the text, Shift+Enter starts a line, Esc keeps it and leaves (an empty text goes). `hint`
    /// shows in an empty field. Returns true while a text is typed, so the caller skips its
    /// shortcuts.
    pub fn handle_typing(
        &mut self,
        ui: &mut Ui,
        view: &View,
        fonts: &FontLib,
        opts: &DrawOpts,
        hint: &str,
    ) -> bool {
        if self.draft.is_none() {
            self.moving_draft = false;
            return false;
        }
        // Plain Enter or Esc only: egui's consume_key would also take Shift+Enter.
        let mut end = false;
        ui.input_mut(|i| {
            i.events.retain(|e| match e {
                egui::Event::Key {
                    key: egui::Key::Enter | egui::Key::Escape,
                    pressed: true,
                    modifiers,
                    ..
                } if modifiers.matches_exact(egui::Modifiers::NONE) => {
                    end = true;
                    false
                }
                _ => true,
            })
        });
        if end {
            self.moving_draft = false;
            if self
                .draft
                .as_ref()
                .is_some_and(|d| d.text.trim().is_empty())
            {
                self.draft = None;
            } else {
                self.commit_draft(opts);
            }
            return true;
        }
        let Some(d) = self.draft.as_mut() else {
            return true;
        };
        let size = (opts.font_size * view.k).max(4.0);
        let font = egui::FontId::new(size, fonts.get(opts.font).family.clone());
        let ink = text_ink(opts.color, opts.text_box);
        let at = view.to_screen(d.pos);
        let shown = if d.text.is_empty() { hint } else { &d.text };
        let ink_size = ui
            .painter()
            .layout_no_wrap(shown.to_owned(), font.clone(), ink)
            .size();
        let text_rect = egui::Rect::from_min_size(at, ink_size);
        if opts.text_box {
            ui.painter().rect_filled(
                text_rect.expand2(text_box_padding(size)),
                size * 0.25,
                c32(opts.color),
            );
        }
        ui.painter().rect_stroke(
            text_rect.expand(4.0),
            3.0,
            Stroke::new(1.0, Color32::from_white_alpha(140)),
            egui::StrokeKind::Outside,
        );
        // Room after the last letter for the caret.
        let field = egui::Rect::from_min_size(at, ink_size + vec2(size * 0.6, 0.0));
        let mut layouter = |ui: &Ui, buf: &dyn egui::TextBuffer, _wrap: f32| {
            let job = egui::text::LayoutJob::simple(
                buf.as_str().to_owned(),
                font.clone(),
                ink,
                f32::INFINITY,
            );
            ui.painter().layout_job(job)
        };
        let id = egui::Id::unique(("takeshot-text-draft", d.started.to_bits()));
        // The dashed frame is a handle: dragging it moves the text, and typing goes on. It is laid
        // before the field, so over the letters the field wins.
        let grab = ui.interact(field.expand(8.0), id.with("frame"), egui::Sense::drag());
        if grab.dragged() {
            d.pos += grab.drag_delta() / view.k;
        }
        self.moving_draft = grab.dragged();
        if grab.hovered() || grab.dragged() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Move);
        }
        if let Some(i) = d.caret.take() {
            let mut state =
                egui::widgets::text_edit::TextEditState::load(ui.ctx(), id).unwrap_or_default();
            let at = egui::text::CCursor::new(i.min(d.text.chars().count()));
            state
                .cursor
                .set_char_range(Some(egui::text::CCursorRange::one(at)));
            state.store(ui.ctx(), id);
        }
        let resp = ui.place(
            field,
            egui::TextEdit::multiline(&mut d.text)
                .id(id)
                .frame(egui::Frame::NONE)
                .margin(egui::Margin::ZERO)
                .font(font.clone())
                .hint_text(
                    egui::RichText::new(hint)
                        .font(font.clone())
                        .color(ink.gamma_multiply(0.5)),
                )
                .desired_rows(1)
                .desired_width(field.width())
                .return_key(egui::KeyboardShortcut::new(
                    egui::Modifiers::SHIFT,
                    egui::Key::Enter,
                ))
                .layouter(&mut layouter),
        );
        // The field keeps the keyboard unless another widget took it (a font picked in a panel
        // gives it back).
        if ui.memory(|m| m.focused().is_none()) {
            resp.request_focus();
        }
        true
    }

    /// Paint the shape being drawn (the text being typed is `handle_typing`'s field).
    pub fn paint_live(&self, painter: &Painter, view: &View, fonts: &FontLib) {
        if let Some(a) = &self.live {
            paint_ann(painter, view, a, fonts);
        }
    }

    /// Paints the dragged objects live (they are out of the baked picture) and, on every selected
    /// object, the theme's selection marker and its handles: white squares with a 2 px primary
    /// border, round on line and arrow ends.
    pub fn paint_selection(
        &self,
        painter: &Painter,
        view: &View,
        t: &Tokens,
        fonts: &FontLib,
        m: TextMeasure,
    ) {
        let live = self.painted_live();
        for o in self.doc.objs().iter().filter(|o| live.contains(&o.id)) {
            paint_ann(painter, view, &o.ann, fonts);
        }
        let ring = Stroke::new(2.0, t.primary);
        for o in self
            .doc
            .objs()
            .iter()
            .filter(|o| !o.hidden && self.doc.selected().contains(&o.id))
        {
            let b = ann_bounds(&o.ann, m);
            let screen = egui::Rect::from_min_max(view.to_screen(b.min), view.to_screen(b.max));
            selection_marker(painter, t, screen);
            for (h, at) in self.doc.handles(o.id, m) {
                let c = view.to_screen(at);
                if matches!(h, Handle::Start | Handle::End) {
                    painter.circle(c, 5.0, Color32::WHITE, ring);
                } else {
                    painter.rect(
                        egui::Rect::from_center_size(c, vec2(8.0, 8.0)),
                        1.0,
                        Color32::WHITE,
                        ring,
                        egui::StrokeKind::Inside,
                    );
                }
            }
        }
    }
}

/// Measures text as the final render draws it, background spacing included on both sides.
pub fn text_measure(fonts: &FontLib) -> impl Fn(&str, usize, f32, bool) -> egui::Vec2 + '_ {
    move |text, font, size, boxed| {
        let (w, h) = fonts::measure(&fonts.get(font).font, size, text);
        let pad = if boxed {
            text_box_padding(size) * 2.0
        } else {
            egui::Vec2::ZERO
        };
        vec2(w, h) + pad
    }
}

/// Draws text at `pos` (screen) with its optional background, as the editor shows it; returns the text rect.
#[allow(clippy::too_many_arguments)]
fn paint_text(
    p: &Painter,
    pos: Pos2,
    text: &str,
    family: egui::FontFamily,
    size: f32,
    color: [u8; 4],
    boxed: bool,
    fade: f32,
) -> egui::Rect {
    let size = size.max(4.0);
    let ink = text_ink(color, boxed).gamma_multiply(fade);
    let galley = p.layout(
        text.to_owned(),
        egui::FontId::new(size, family),
        ink,
        f32::INFINITY,
    );
    let rect = egui::Rect::from_min_size(pos, galley.size());
    if boxed {
        p.rect_filled(
            rect.expand2(text_box_padding(size)),
            size * 0.25,
            c32(color),
        );
    }
    p.galley(pos, galley, ink);
    rect
}

/// Consumes Cmd/Ctrl + `key`. egui-winit turns Cmd/Ctrl+C and +X into [`egui::Event::Copy`] and
/// [`egui::Event::Cut`] without their key, so those events count (and are consumed) too, but only
/// while Cmd/Ctrl is down: Shift+Delete on Windows is a cut as well. Callers check first that no
/// text field has the keyboard, since those events are its own.
pub fn consume_command(i: &mut egui::InputState, key: egui::Key) -> bool {
    let event = match key {
        egui::Key::C => Some(egui::Event::Copy),
        egui::Key::X => Some(egui::Event::Cut),
        _ => None,
    };
    let mut hit = i.consume_key(egui::Modifiers::COMMAND, key);
    if let Some(ev) = event
        && i.modifiers.command
    {
        let before = i.events.len();
        i.events.retain(|e| *e != ev);
        hit |= i.events.len() != before;
    }
    hit
}

/// Plain mouse-wheel notches in this frame's `events` (Ctrl+wheel zooms instead): a line is a
/// notch, and on a touchpad 50 points make one; `acc` keeps the part of a notch still to come.
/// egui smooths the wheel over frames, so its scroll delta would count one notch many times.
pub fn wheel_notches(acc: &mut f32, events: &[egui::Event]) -> i32 {
    for e in events {
        if let egui::Event::MouseWheel {
            unit,
            delta,
            modifiers,
            ..
        } = e
            && !(modifiers.ctrl || modifiers.command)
        {
            *acc += match unit {
                egui::MouseWheelUnit::Line => delta.y,
                egui::MouseWheelUnit::Point => delta.y / 50.0,
                egui::MouseWheelUnit::Page => delta.y * 3.0,
            };
        }
    }
    let n = acc.trunc();
    *acc -= n;
    n as i32
}

/// The wheel's `notches` on the options: the stroke width by 1 px (1 to 40), or for text the
/// size by 2 px (8 to 240).
pub fn wheel_restyle(tool: Tool, opts: &mut DrawOpts, notches: i32) {
    if notches == 0 {
        return;
    }
    let n = notches as f32;
    if tool == Tool::Text {
        opts.font_size = (opts.font_size + n * 2.0).clamp(8.0, 240.0);
    } else {
        opts.width = (opts.width + n).clamp(1.0, 40.0);
    }
}

/// The colour of a text's letters: its colour, or on a background in its colour, black or white.
fn text_ink(color: [u8; 4], boxed: bool) -> Color32 {
    if !boxed {
        c32(color)
    } else if luminance(color) > 0.6 {
        Color32::from_rgb(17, 17, 17)
    } else {
        Color32::WHITE
    }
}

/// Approximate on-screen drawing of one annotation (while it is drawn or dragged).
pub fn paint_ann(p: &Painter, v: &View, a: &Ann, fonts: &FontLib) {
    let col = c32(a.color);
    let w = (a.width * v.k).max(1.0);
    let st = Stroke::new(w, col);
    match &a.shape {
        Shape::Pen(pts) => {
            let pts: Vec<Pos2> = pts.iter().map(|q| v.to_screen(*q)).collect();
            if pts.len() > 1 {
                p.add(EShape::line(pts.clone(), st));
            }
            for q in [pts.first(), pts.last()].into_iter().flatten() {
                p.circle_filled(*q, w / 2.0, col);
            }
        }
        Shape::Marker(pts) => {
            let c = Color32::from_rgba_unmultiplied(
                a.color[0],
                a.color[1],
                a.color[2],
                marker_alpha(a.color[3]),
            );
            let pts: Vec<Pos2> = pts.iter().map(|q| v.to_screen(*q)).collect();
            if pts.len() > 1 {
                p.add(EShape::line(
                    pts,
                    Stroke::new(a.width * MARKER_WIDTH_FACTOR * v.k, c),
                ));
            }
        }
        Shape::Line(a0, b0) => {
            p.line_segment([v.to_screen(*a0), v.to_screen(*b0)], st);
        }
        Shape::Arrow(a0, b0) => {
            let (end, head) = arrow_geom(*a0, *b0, a.width);
            p.line_segment([v.to_screen(*a0), v.to_screen(end)], st);
            p.add(EShape::convex_polygon(
                head.iter().map(|q| v.to_screen(*q)).collect(),
                col,
                Stroke::NONE,
            ));
        }
        Shape::Rect(a0, b0, radius) => {
            let r = egui::Rect::from_two_pos(v.to_screen(*a0), v.to_screen(*b0));
            let rad = (radius * v.k).clamp(0.0, r.width().min(r.height()) / 2.0);
            let corners = egui::CornerRadius::same(rad.round().min(255.0) as u8);
            if a.filled {
                p.rect_filled(r, corners, col);
            } else {
                p.rect_stroke(r, corners, st, egui::StrokeKind::Middle);
            }
        }
        Shape::Ellipse(a0, b0) => {
            let r = egui::Rect::from_two_pos(v.to_screen(*a0), v.to_screen(*b0));
            p.add(EShape::Ellipse(egui::epaint::EllipseShape {
                center: r.center(),
                radius: r.size() / 2.0,
                fill: if a.filled { col } else { Color32::TRANSPARENT },
                stroke: if a.filled { Stroke::NONE } else { st },
                angle: 0.0,
            }));
        }
        Shape::Blur(a0, b0, _) => {
            let r = egui::Rect::from_two_pos(v.to_screen(*a0), v.to_screen(*b0));
            p.rect_filled(r, 2.0, Color32::from_white_alpha(40));
            dashed_rect(p, r, Color32::WHITE);
        }
        Shape::Counter { center, n } => {
            let c = v.to_screen(*center);
            let r = counter_radius(a.width) * v.k;
            p.circle_filled(c, r, col);
            // The picture's number: a line as tall as 1.25 radii.
            let em = fonts::em_for_height(&fonts.get(fonts.bold).font, r * 1.25);
            p.text(
                c,
                egui::Align2::CENTER_CENTER,
                n.to_string(),
                egui::FontId::new(em, fonts.get(fonts.bold).family.clone()),
                Color32::WHITE,
            );
        }
        Shape::Text {
            pos,
            text,
            font,
            size,
            boxed,
        } => {
            paint_text(
                p,
                v.to_screen(*pos),
                text,
                fonts.get(*font).family.clone(),
                size * v.k,
                a.color,
                *boxed,
                1.0,
            );
        }
    }
}

pub fn dashed_rect(p: &Painter, r: egui::Rect, col: Color32) {
    let pts = vec![
        r.left_top(),
        r.right_top(),
        r.right_bottom(),
        r.left_bottom(),
        r.left_top(),
    ];
    p.extend(EShape::dashed_line(&pts, Stroke::new(1.0, col), 5.0, 4.0));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::doc::{Doc, Handle, ObjId, ann_bounds};
    use crate::ui::tokens::{ThemeId, tokens};
    use egui::{Event, Modifiers, PointerButton, Rect, Sense, Vec2, pos2};

    fn m(t: &str, _f: usize, size: f32, boxed: bool) -> Vec2 {
        let w = t.lines().map(|l| l.chars().count()).max().unwrap_or(0) as f32 * size * 0.5;
        let h = t.lines().count() as f32 * size * 1.2;
        let pad = if boxed { 8.0 } else { 0.0 };
        vec2(w + pad, h + pad)
    }

    const VIEW: View = View {
        origin: Pos2::ZERO,
        k: 1.0,
    };

    fn rect_ann(a: (f32, f32), b: (f32, f32), filled: bool) -> Ann {
        Ann {
            shape: Shape::Rect(pos2(a.0, a.1), pos2(b.0, b.1), 0.0),
            color: [255, 0, 0, 255],
            width: 4.0,
            filled,
        }
    }

    fn press(p: Pos2) -> Vec<Event> {
        vec![
            Event::PointerMoved(p),
            Event::PointerButton {
                pos: p,
                button: PointerButton::Primary,
                pressed: true,
                modifiers: Modifiers::NONE,
            },
        ]
    }

    fn release(p: Pos2) -> Vec<Event> {
        vec![Event::PointerButton {
            pos: p,
            button: PointerButton::Primary,
            pressed: false,
            modifiers: Modifiers::NONE,
        }]
    }

    fn to(p: Pos2) -> Vec<Event> {
        vec![Event::PointerMoved(p)]
    }

    fn click(p: Pos2) -> Vec<Vec<Event>> {
        vec![press(p), release(p), vec![]]
    }

    fn drag(from: Pos2, path: &[Pos2]) -> Vec<Vec<Event>> {
        let mut f = vec![press(from)];
        f.extend(path.iter().map(|p| to(*p)));
        f.push(release(*path.last().unwrap()));
        f.push(vec![]);
        f
    }

    /// What one frame of `handle_select_pointer` did, as the screens call it.
    struct Frame {
        outcome: SelectOutcome,
        excluded: Vec<ObjId>,
        baked: bool,
        live: Vec<ObjId>,
    }

    fn drive(
        an: &mut Annotator,
        opts: &mut DrawOpts,
        frames: Vec<Vec<Event>>,
        shift: bool,
    ) -> Vec<Frame> {
        let ctx = egui::Context::default();
        let screen = Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0));
        let canvas = |ui: &mut Ui| {
            ui.interact(
                ui.max_rect(),
                ui.scope_id().with("canvas"),
                Sense::click_and_drag(),
            )
        };
        // egui hit-tests against the previous frame's widgets, so the canvas is laid out once first.
        let warm = egui::RawInput {
            screen_rect: Some(screen),
            ..Default::default()
        };
        ctx.run_ui(warm, |ui| {
            canvas(ui);
        })
        .textures_delta
        .clear();
        let mut out = Vec::new();
        let held = if shift {
            Modifiers::SHIFT
        } else {
            Modifiers::NONE
        };
        for (i, events) in frames.into_iter().enumerate() {
            let mut all = vec![Event::ModifiersChanged(held)];
            all.extend(events);
            let input = egui::RawInput {
                screen_rect: Some(screen),
                time: Some(0.016 + i as f64 * 0.016),
                events: all,
                ..Default::default()
            };
            let mut full = ctx.run_ui(input, |ui| {
                // The screens bake first, then handle the pointer, then paint.
                let baked = an.bake().is_some();
                let resp = canvas(ui);
                let outcome = an.handle_select_pointer(ui, &resp, &VIEW, &m, opts);
                out.push(Frame {
                    outcome,
                    excluded: an.excluded_from_bake(),
                    baked,
                    live: an.painted_live().to_vec(),
                });
            });
            full.textures_delta.clear();
        }
        out
    }

    fn with(tool: Tool, anns: Vec<Ann>) -> (Annotator, Vec<ObjId>) {
        let an = Annotator::with_doc(tool, Doc::from_anns(anns));
        let ids = an.doc.objs().iter().map(|o| o.id).collect();
        (an, ids)
    }

    fn text_ann(font: usize, size: f32, boxed: bool) -> Ann {
        Ann {
            shape: Shape::Text {
                pos: pos2(100.0, 100.0),
                text: "hi".into(),
                font,
                size,
                boxed,
            },
            color: [0, 0, 255, 200],
            width: 3.0,
            filled: false,
        }
    }

    #[test]
    fn picking_from_the_list_selects_and_copies_the_style() {
        let (mut an, ids) = with(Tool::Rect, vec![text_ann(1, 28.0, true)]);
        let mut opts = DrawOpts::default();
        an.pick(ids[0], false, &mut opts);
        assert_eq!(an.tool, Tool::Select);
        assert_eq!(an.doc.selected(), &[ids[0]]);
        assert_eq!(
            (opts.font, opts.font_size, opts.text_box, opts.color),
            (1, 28.0, true, [0, 0, 255, 200])
        );
        assert_eq!(an.bar_tool(), Some(Tool::Text));
    }

    #[test]
    fn the_options_restyle_a_selected_text() {
        let (mut an, ids) = with(Tool::Select, vec![text_ann(0, 20.0, false)]);
        an.doc.select(Some(ids[0]), false);
        let before = DrawOpts::default();
        let after = DrawOpts {
            font: 2,
            font_size: 40.0,
            text_box: true,
            ..before.clone()
        };
        an.restyle_from_opts(&before, &after);
        match &an.doc.objs()[0].ann.shape {
            Shape::Text {
                font, size, boxed, ..
            } => assert_eq!((*font, *size, *boxed), (2, 40.0, true)),
            s => panic!("{s:?}"),
        }
    }

    fn bounds(an: &Annotator, id: ObjId) -> Rect {
        let o = an.doc.objs().iter().find(|o| o.id == id).unwrap();
        ann_bounds(&o.ann, &m)
    }

    #[test]
    fn dragging_an_object_moves_it_in_one_undo_step_and_bakes_only_at_the_ends() {
        let (mut an, ids) = with(
            Tool::Select,
            vec![rect_ann((100.0, 100.0), (200.0, 200.0), true)],
        );
        let mut opts = DrawOpts::default();
        let before = bounds(&an, ids[0]);
        let path = [
            pos2(160.0, 150.0),
            pos2(250.0, 150.0),
            pos2(350.0, 150.0),
            pos2(450.0, 150.0),
        ];
        let frames = drive(&mut an, &mut opts, drag(pos2(150.0, 150.0), &path), false);
        // Frames: press, drag start, three moves, release, idle.
        assert!(
            frames[1..=5]
                .iter()
                .all(|f| f.outcome == SelectOutcome::Consumed)
        );
        assert_eq!(frames[3].excluded, ids);
        assert!(frames[6].excluded.is_empty());
        let baked: Vec<bool> = frames.iter().map(|f| f.baked).collect();
        assert_eq!(baked, vec![true, false, true, false, false, false, true]);
        // Painted live exactly while the baked picture leaves it out: never twice, never missing.
        let live: Vec<bool> = frames.iter().map(|f| f.live == ids).collect();
        assert_eq!(live, vec![false, false, true, true, true, true, false]);
        assert!(frames[6].live.is_empty());
        assert!((bounds(&an, ids[0]).min.x - (before.min.x + 300.0)).abs() < 0.01);
        assert!(an.dragging.is_none());
        an.undo();
        assert!((bounds(&an, ids[0]).min.x - before.min.x).abs() < 0.01);
    }

    #[test]
    fn dragging_one_of_a_multi_selection_moves_all_and_a_click_picks_one() {
        let (mut an, ids) = with(
            Tool::Select,
            vec![
                rect_ann((0.0, 0.0), (100.0, 100.0), true),
                rect_ann((200.0, 0.0), (300.0, 100.0), true),
            ],
        );
        an.doc.select(Some(ids[0]), false);
        an.doc.select(Some(ids[1]), true);
        let (a, b) = (bounds(&an, ids[0]).min, bounds(&an, ids[1]).min);
        let mut opts = DrawOpts::default();
        drive(
            &mut an,
            &mut opts,
            drag(pos2(250.0, 50.0), &[pos2(260.0, 50.0), pos2(250.0, 90.0)]),
            false,
        );
        assert_eq!(an.doc.selected(), &[ids[0], ids[1]]);
        assert!(bounds(&an, ids[0]).min.distance(a + vec2(0.0, 40.0)) < 0.01);
        assert!(bounds(&an, ids[1]).min.distance(b + vec2(0.0, 40.0)) < 0.01);
        drive(&mut an, &mut opts, click(pos2(250.0, 90.0)), false);
        assert_eq!(an.doc.selected(), &[ids[1]]);
    }

    #[test]
    fn a_new_empty_annotator_needs_no_bake_but_a_loaded_one_does() {
        assert!(Annotator::new(Tool::Select).bake().is_none());
        let doc = Doc::from_anns(vec![rect_ann((0.0, 0.0), (10.0, 10.0), true)]);
        let mut an = Annotator::with_doc(Tool::Select, doc);
        assert_eq!(an.bake().map(|a| a.len()), Some(1));
        assert!(an.bake().is_none());
    }

    #[test]
    fn handles_resize_with_any_tool_and_drawing_goes_on_elsewhere() {
        for tool in [Tool::Select, Tool::Rect] {
            let (mut an, ids) = with(tool, vec![rect_ann((100.0, 100.0), (200.0, 200.0), false)]);
            an.doc.select(Some(ids[0]), false);
            let mut opts = DrawOpts::default();
            let path = [pos2(220.0, 220.0), pos2(300.0, 300.0)];
            let frames = drive(&mut an, &mut opts, drag(pos2(200.0, 200.0), &path), false);
            assert!(
                frames[1..=3]
                    .iter()
                    .all(|f| f.outcome == SelectOutcome::Consumed),
                "{tool:?}"
            );
            match an.doc.objs()[0].ann.shape {
                Shape::Rect(a, b, _) => {
                    assert!(a.distance(pos2(100.0, 100.0)) < 0.01, "{tool:?}");
                    assert!(b.distance(pos2(300.0, 300.0)) < 0.01, "{tool:?}");
                }
                _ => unreachable!(),
            }
            assert_eq!(an.doc.objs().len(), 1);
        }
        // Away from the handles a drawing tool is left to draw.
        let (mut an, ids) = with(
            Tool::Rect,
            vec![rect_ann((100.0, 100.0), (200.0, 200.0), false)],
        );
        an.doc.select(Some(ids[0]), false);
        let mut opts = DrawOpts::default();
        let frames = drive(
            &mut an,
            &mut opts,
            drag(pos2(400.0, 400.0), &[pos2(500.0, 450.0)]),
            false,
        );
        assert!(frames.iter().all(|f| f.outcome == SelectOutcome::Missed));
        assert!(an.dragging.is_none());
    }

    #[test]
    fn click_selects_shift_click_adds_and_empty_click_clears() {
        let (mut an, ids) = with(
            Tool::Select,
            vec![
                rect_ann((0.0, 0.0), (100.0, 100.0), true),
                rect_ann((200.0, 0.0), (300.0, 100.0), true),
            ],
        );
        let mut opts = DrawOpts::default();
        let f = drive(&mut an, &mut opts, click(pos2(50.0, 50.0)), false);
        assert!(f.iter().any(|f| f.outcome == SelectOutcome::Consumed));
        assert_eq!(an.doc.selected(), &[ids[0]]);
        drive(&mut an, &mut opts, click(pos2(250.0, 50.0)), true);
        assert_eq!(an.doc.selected(), &[ids[0], ids[1]]);
        let f = drive(&mut an, &mut opts, click(pos2(500.0, 500.0)), false);
        assert!(f.iter().all(|f| f.outcome == SelectOutcome::Missed));
        assert!(an.doc.selected().is_empty());
    }

    #[test]
    fn selecting_copies_the_object_style_into_the_options() {
        let mut blue = rect_ann((0.0, 0.0), (100.0, 100.0), true);
        blue.color = [0, 0, 255, 255];
        blue.width = 8.0;
        let (mut an, _) = with(Tool::Select, vec![blue]);
        let mut opts = DrawOpts::default();
        drive(&mut an, &mut opts, click(pos2(50.0, 50.0)), false);
        assert_eq!(opts.color, [0, 0, 255, 255]);
        assert_eq!(opts.width, 8.0);
        assert!(opts.filled);
    }

    #[test]
    fn option_bar_changes_restyle_the_selection() {
        let (mut an, ids) = with(
            Tool::Select,
            vec![rect_ann((0.0, 0.0), (100.0, 100.0), false)],
        );
        let before = DrawOpts::default();
        let mut after = before.clone();
        after.color = [0, 200, 0, 255];
        let rev = an.doc.revision();
        an.restyle_from_opts(&before, &after);
        assert_eq!(an.doc.revision(), rev, "nothing selected, nothing restyled");
        an.doc.select(Some(ids[0]), false);
        an.restyle_from_opts(&before, &after);
        assert_eq!(an.doc.objs()[0].ann.color, [0, 200, 0, 255]);
        assert_eq!(an.doc.objs()[0].ann.width, 4.0, "only what the bar changed");
        an.undo();
        assert_eq!(an.doc.objs()[0].ann.color, [255, 0, 0, 255]);
    }

    #[test]
    fn double_click_on_text_reopens_it_for_typing() {
        let text = Ann {
            shape: Shape::Text {
                pos: pos2(100.0, 100.0),
                text: "hi".into(),
                font: 0,
                size: 20.0,
                boxed: true,
            },
            color: [0, 0, 255, 255],
            width: 3.0,
            filled: false,
        };
        let (mut an, _) = with(Tool::Select, vec![text]);
        let mut opts = DrawOpts::default();
        assert!(!an.edit_text_at(pos2(500.0, 500.0), &m, &mut opts));
        assert!(an.edit_text_at(pos2(105.0, 105.0), &m, &mut opts));
        let d = an.draft.as_ref().unwrap();
        assert_eq!((d.pos, d.text.as_str()), (pos2(100.0, 100.0), "hi"));
        assert!(an.doc.objs().is_empty());
        assert_eq!(
            (opts.font_size, opts.text_box, opts.color),
            (20.0, true, [0, 0, 255, 255])
        );
        an.commit_draft(&opts);
        assert_eq!(an.doc.objs().len(), 1);
    }

    #[test]
    fn text_measure_adds_the_background_on_both_sides() {
        let fonts = FontLib::load();
        let f = text_measure(&fonts);
        let plain = f("Ab\nc", 0, 20.0, false);
        assert!(plain.x > 0.0 && plain.y > 20.0, "{plain:?}");
        let boxed = f("Ab\nc", 0, 20.0, true);
        let pad = crate::engine::render::text_box_padding(20.0) * 2.0;
        assert!(
            (boxed - plain - pad).length() < 0.01,
            "{boxed:?} vs {plain:?}"
        );
    }

    #[test]
    fn selection_paints_handles_and_the_dragged_text_live() {
        let line = Ann {
            shape: Shape::Line(pos2(10.0, 10.0), pos2(100.0, 10.0)),
            color: [255, 0, 0, 255],
            width: 4.0,
            filled: false,
        };
        let text = Ann {
            shape: Shape::Text {
                pos: pos2(100.0, 100.0),
                text: "hi".into(),
                font: 0,
                size: 20.0,
                boxed: false,
            },
            color: [0, 0, 255, 255],
            width: 3.0,
            filled: false,
        };
        let (mut an, ids) = with(Tool::Select, vec![line, text]);
        let fonts = FontLib::load();
        let t = tokens(ThemeId::TakeUi, true);
        let ctx = egui::Context::default();
        fonts.install(&ctx);
        let paint = |an: &Annotator| {
            let mut full = ctx.run_ui(egui::RawInput::default(), |ui| {
                an.paint_selection(ui.painter(), &VIEW, &t, &fonts, &m);
            });
            full.textures_delta.clear();
            let mut circles = 0;
            let mut texts = Vec::new();
            for c in &full.shapes {
                match &c.shape {
                    egui::Shape::Circle(_) => circles += 1,
                    egui::Shape::Text(tx) => texts.push(tx.galley.text().to_owned()),
                    _ => {}
                }
            }
            (circles, texts)
        };
        an.doc.select(Some(ids[0]), false);
        let (circles, texts) = paint(&an);
        assert_eq!(circles, 2, "line ends get round handles");
        assert!(
            texts.is_empty(),
            "nothing is dragged, so nothing is painted live"
        );
        an.doc.select(Some(ids[1]), false);
        an.dragging = Some(Drag::Move { last: Pos2::ZERO });
        // Until the bake takes the dragged text out, the baked picture still shows it.
        assert!(paint(&an).1.is_empty(), "painted twice before the bake");
        assert!(an.bake().is_some());
        let (_, texts) = paint(&an);
        assert_eq!(texts, vec!["hi".to_owned()]);
    }

    /// Runs `f` once per frame of events over a full-screen canvas, after a warm-up frame.
    fn run<T>(
        held: Modifiers,
        frames: Vec<Vec<Event>>,
        mut f: impl FnMut(&Ui, &egui::Response) -> T,
    ) -> Vec<T> {
        let ctx = egui::Context::default();
        let screen = Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0));
        let canvas = |ui: &mut Ui| {
            ui.interact(
                ui.max_rect(),
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
        let mut out = Vec::new();
        for (i, events) in frames.into_iter().enumerate() {
            let mut all = vec![Event::ModifiersChanged(held)];
            all.extend(events);
            let input = egui::RawInput {
                screen_rect: Some(screen),
                time: Some(0.016 + i as f64 * 0.016),
                events: all,
                ..Default::default()
            };
            ctx.run_ui(input, |ui| {
                let resp = canvas(ui);
                out.push(f(ui, &resp));
            })
            .textures_delta
            .clear();
        }
        out
    }

    fn key(k: egui::Key, modifiers: Modifiers) -> Vec<Event> {
        vec![Event::Key {
            key: k,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        }]
    }

    #[test]
    fn the_bar_shows_the_picked_object_or_the_drawing_tool() {
        // With something selected, its own controls whatever the tool; else the tool's.
        let (mut an, ids) = with(
            Tool::Select,
            vec![rect_ann((0.0, 0.0), (100.0, 100.0), false)],
        );
        assert_eq!(an.bar_tool(), None);
        an.doc.select(Some(ids[0]), false);
        assert_eq!(an.bar_tool(), Some(Tool::Rect));
        an.tool = Tool::Arrow;
        assert_eq!(
            an.bar_tool(),
            Some(Tool::Rect),
            "a rectangle picked with the Arrow"
        );
        an.doc.select(None, false);
        assert_eq!(an.bar_tool(), Some(Tool::Arrow));
    }

    #[test]
    fn a_drawing_tool_moves_the_selected_shape() {
        let (mut an, ids) = with(
            Tool::Arrow,
            vec![rect_ann((100.0, 100.0), (200.0, 200.0), false)],
        );
        an.doc.select(Some(ids[0]), false);
        let mut opts = DrawOpts::default();
        // The left edge, away from its handles.
        let frames = drag(
            pos2(100.0, 120.0),
            &[pos2(130.0, 150.0), pos2(160.0, 180.0)],
        );
        run(Modifiers::NONE, frames, |ui, resp| {
            an.handle_canvas(ui, resp, &VIEW, &m, &mut opts)
        });
        assert_eq!(an.doc.objs().len(), 1, "nothing new was drawn");
        assert!(
            bounds(&an, ids[0])
                .min
                .distance(pos2(160.0 - 2.0, 160.0 - 2.0))
                < 0.01
        );
        an.undo();
        assert!(
            bounds(&an, ids[0])
                .min
                .distance(pos2(100.0 - 2.0, 100.0 - 2.0))
                < 0.01
        );
        assert!(!an.doc.undo(), "one step for the whole move");
    }

    fn text_obj(text: &str) -> Ann {
        Ann {
            shape: Shape::Text {
                pos: pos2(100.0, 100.0),
                text: text.into(),
                font: 0,
                size: 20.0,
                boxed: false,
            },
            color: [0, 0, 255, 255],
            width: 3.0,
            filled: false,
        }
    }

    fn double_click(p: Pos2) -> Vec<Vec<Event>> {
        vec![press(p), release(p), press(p), release(p), vec![]]
    }

    #[test]
    fn the_text_tool_edits_a_text_with_one_click() {
        // The test measure: 10 px a character at size 20. A click after "Pico" puts the caret
        // there.
        let (mut an, _) = with(Tool::Text, vec![text_obj("Pico em outubro")]);
        let mut opts = DrawOpts::default();
        run(Modifiers::NONE, click(pos2(141.0, 108.0)), |ui, resp| {
            an.handle_canvas(ui, resp, &VIEW, &m, &mut opts)
        });
        let d = an.draft.as_ref().expect("editing the text");
        assert_eq!(d.text, "Pico em outubro");
        assert_eq!(d.caret, Some(4));
        assert!(an.doc.objs().is_empty(), "taken out while it is typed");
        // Off every text, the Text tool starts a new one.
        let (mut an, _) = with(Tool::Text, vec![text_obj("Pico")]);
        run(Modifiers::NONE, click(pos2(400.0, 400.0)), |ui, resp| {
            an.handle_canvas(ui, resp, &VIEW, &m, &mut opts)
        });
        assert_eq!(an.draft.as_ref().map(|d| d.text.as_str()), Some(""));
        assert_eq!(an.doc.objs().len(), 1);
    }

    #[test]
    fn a_double_click_edits_any_text_with_any_tool() {
        for tool in [Tool::Select, Tool::Arrow, Tool::Rect] {
            let (mut an, _) = with(tool, vec![text_obj("Pico")]);
            let mut opts = DrawOpts::default();
            run(
                Modifiers::NONE,
                double_click(pos2(121.0, 108.0)),
                |ui, resp| an.handle_canvas(ui, resp, &VIEW, &m, &mut opts),
            );
            let d = an
                .draft
                .as_ref()
                .unwrap_or_else(|| panic!("{tool:?} edits"));
            assert_eq!((d.text.as_str(), d.caret), ("Pico", Some(2)), "{tool:?}");
        }
    }

    #[test]
    fn the_text_tool_shows_the_text_cursor_over_a_text() {
        let (an, _) = with(Tool::Text, vec![text_obj("Pico")]);
        let (_, over, over_text) = an.hover(pos2(121.0, 108.0), &VIEW, &m);
        assert!(over && over_text);
        let ctx = |tool, over_object, over_text| crate::engine::cursor::CursorCtx {
            tool,
            over_image: true,
            over_chrome: false,
            handle: None,
            over_object,
            over_text,
            typing: false,
            dragging: false,
            space_pan: false,
            area: crate::engine::cursor::Area::NotApplicable,
        };
        use crate::engine::cursor::pick;
        assert_eq!(pick(&ctx(Tool::Text, true, true)), egui::CursorIcon::Text);
        assert_eq!(pick(&ctx(Tool::Arrow, true, true)), egui::CursorIcon::Move);
        assert_eq!(pick(&ctx(Tool::Text, true, false)), egui::CursorIcon::Move);
    }

    fn typing(an: &mut Annotator, text: &str) {
        an.draft = Some(TextDraft {
            pos: pos2(400.0, 400.0),
            text: text.into(),
            started: 0.0,
            caret: None,
        });
    }

    fn text_colour(an: &Annotator) -> Option<[u8; 4]> {
        an.doc
            .objs()
            .iter()
            .find(|o| matches!(o.ann.shape, Shape::Text { .. }))
            .map(|o| o.ann.color)
    }

    #[test]
    fn picking_a_shape_ends_the_text_being_typed_in_its_own_style() {
        let (mut an, ids) = with(
            Tool::Text,
            vec![rect_ann((100.0, 100.0), (200.0, 200.0), false)],
        );
        typing(&mut an, "Pico");
        let mut opts = DrawOpts {
            color: [0, 0, 255, 255],
            ..Default::default()
        };
        run(Modifiers::NONE, click(pos2(100.0, 150.0)), |ui, resp| {
            an.handle_canvas(ui, resp, &VIEW, &m, &mut opts)
        });
        assert!(an.draft.is_none(), "the text is kept first");
        assert_eq!(text_colour(&an), Some([0, 0, 255, 255]));
        assert_eq!(an.doc.objs()[0].ann.color, [255, 0, 0, 255]);
        assert_eq!(an.doc.selected(), &[ids[0]]);
    }

    #[test]
    fn a_new_shape_ends_the_text_being_typed() {
        let (mut an, _) = with(Tool::Arrow, vec![]);
        typing(&mut an, "Pico");
        let mut opts = DrawOpts::default();
        run(
            Modifiers::NONE,
            drag(
                pos2(100.0, 100.0),
                &[pos2(150.0, 150.0), pos2(200.0, 200.0)],
            ),
            |ui, resp| an.handle_canvas(ui, resp, &VIEW, &m, &mut opts),
        );
        assert!(an.draft.is_none());
        assert_eq!(an.doc.objs().len(), 2, "the text and the arrow");
    }

    #[test]
    fn shift_with_a_drawing_tool_moves_only_the_shape_under_it() {
        let (mut an, ids) = with(
            Tool::Arrow,
            vec![
                rect_ann((100.0, 100.0), (200.0, 200.0), false),
                rect_ann((400.0, 400.0), (500.0, 500.0), false),
            ],
        );
        an.doc.select(Some(ids[0]), false);
        let first = bounds(&an, ids[0]);
        let second = bounds(&an, ids[1]);
        let mut opts = DrawOpts::default();
        run(
            Modifiers::SHIFT,
            drag(
                pos2(400.0, 420.0),
                &[pos2(430.0, 450.0), pos2(460.0, 480.0)],
            ),
            |ui, resp| an.handle_canvas(ui, resp, &VIEW, &m, &mut opts),
        );
        assert_eq!(bounds(&an, ids[0]), first, "the last shape drawn stays");
        assert_ne!(bounds(&an, ids[1]), second);
        assert_eq!(an.doc.selected(), &[ids[1]]);
    }

    #[test]
    fn the_caret_finds_the_line_with_the_font_s_line_gap() {
        // 20 px lines 10 px apart: the click at y 45 is on the second line.
        let gap = |t: &str, _f: usize, _s: f32, _b: bool| {
            let lines = t.lines().count().max(1) as f32;
            let w = t.lines().map(|l| l.chars().count()).max().unwrap_or(0) as f32 * 10.0;
            vec2(w, lines * 20.0 + (lines - 1.0) * 10.0)
        };
        assert_eq!(caret_at("ab\ncd\nef", 0, 20.0, vec2(11.0, 45.0), &gap), 4);
    }

    #[test]
    fn every_draft_gets_its_own_field() {
        // A capture's text and the editor's first text never share a field (or its undo).
        let mut a = Annotator::new(Tool::Text);
        let mut b = Annotator::new(Tool::Text);
        let mut opts = DrawOpts::default();
        for an in [&mut a, &mut b] {
            run(Modifiers::NONE, click(pos2(300.0, 300.0)), |ui, resp| {
                an.handle_canvas(ui, resp, &VIEW, &m, &mut opts)
            });
        }
        let (sa, sb) = (a.draft.unwrap().started, b.draft.unwrap().started);
        assert_ne!(sa, sb);
    }

    #[test]
    fn a_live_marker_takes_its_opacity() {
        for (opacity, want) in [(255u8, MARKER_ALPHA), (128, 63)] {
            let ann = Ann {
                shape: Shape::Marker(vec![pos2(0.0, 0.0), pos2(100.0, 0.0)]),
                color: [255, 214, 10, opacity],
                width: 4.0,
                filled: false,
            };
            let ctx = egui::Context::default();
            let fonts = FontLib::load();
            let mut full = ctx.run_ui(egui::RawInput::default(), |ui| {
                let view = View {
                    origin: pos2(0.0, 0.0),
                    k: 1.0,
                };
                paint_ann(ui.painter(), &view, &ann, &fonts);
            });
            full.textures_delta.clear();
            let alpha = full.shapes.iter().find_map(|c| match &c.shape {
                egui::Shape::Path(p) => match p.stroke.color {
                    egui::epaint::ColorMode::Solid(c) => Some(c.a()),
                    _ => None,
                },
                _ => None,
            });
            let alpha = alpha.expect("the marker's line");
            assert!(
                alpha.abs_diff(want) <= 1,
                "opacity {opacity}: alpha {alpha}, want {want}"
            );
        }
    }

    #[test]
    fn live_drawing_rounds_the_corners_by_the_view_scale() {
        // The picture shown at half size: a 30 px corner shows as 15.
        let ann = Ann {
            shape: Shape::Rect(pos2(0.0, 0.0), pos2(100.0, 80.0), 30.0),
            color: [255, 0, 0, 255],
            width: 4.0,
            filled: false,
        };
        let ctx = egui::Context::default();
        let fonts = FontLib::load();
        let mut full = ctx.run_ui(egui::RawInput::default(), |ui| {
            let view = View {
                origin: pos2(0.0, 0.0),
                k: 0.5,
            };
            paint_ann(ui.painter(), &view, &ann, &fonts);
        });
        full.textures_delta.clear();
        let radius = full.shapes.iter().find_map(|c| match &c.shape {
            egui::Shape::Rect(r) => Some(r.corner_radius),
            _ => None,
        });
        assert_eq!(radius, Some(egui::CornerRadius::same(15)));
    }

    #[test]
    fn picking_a_rect_shows_its_corners_in_the_options() {
        let (mut an, ids) = with(
            Tool::Select,
            vec![Ann {
                shape: Shape::Rect(pos2(100.0, 100.0), pos2(200.0, 200.0), 18.0),
                color: [255, 0, 0, 255],
                width: 4.0,
                filled: false,
            }],
        );
        let mut opts = DrawOpts::default();
        an.pick(ids[0], false, &mut opts);
        assert_eq!(opts.radius, 18.0);
    }

    #[test]
    fn a_drawing_tool_moves_any_shape_under_it() {
        // Not selected: pressing on its line picks it, shows its style and moves it.
        let (mut an, ids) = with(
            Tool::Arrow,
            vec![
                rect_ann((100.0, 100.0), (200.0, 200.0), false),
                Ann {
                    color: [0, 0, 255, 255],
                    ..rect_ann((400.0, 400.0), (500.0, 500.0), false)
                },
            ],
        );
        an.doc.select(Some(ids[0]), false);
        let mut opts = DrawOpts::default();
        let before = bounds(&an, ids[1]);
        let frames = drag(
            pos2(400.0, 420.0),
            &[pos2(430.0, 450.0), pos2(460.0, 480.0)],
        );
        run(Modifiers::NONE, frames, |ui, resp| {
            an.handle_canvas(ui, resp, &VIEW, &m, &mut opts)
        });
        assert_eq!(an.doc.objs().len(), 2, "nothing new was drawn");
        assert_eq!(an.doc.selected(), &[ids[1]]);
        assert_eq!(
            opts.color,
            [0, 0, 255, 255],
            "its style shows in the options"
        );
        assert!(
            bounds(&an, ids[1])
                .min
                .distance(before.min + vec2(60.0, 60.0))
                < 0.01
        );
        assert_eq!(an.tool, Tool::Arrow, "the tool stays");
    }

    #[test]
    fn a_drawing_tool_draws_beside_shapes() {
        // Off every shape's line (inside an outline rectangle too), the tool draws.
        let (mut an, ids) = with(
            Tool::Arrow,
            vec![rect_ann((100.0, 100.0), (300.0, 300.0), false)],
        );
        let mut opts = DrawOpts::default();
        let before = bounds(&an, ids[0]);
        let frames = drag(
            pos2(200.0, 200.0),
            &[pos2(250.0, 230.0), pos2(400.0, 400.0)],
        );
        run(Modifiers::NONE, frames, |ui, resp| {
            an.handle_canvas(ui, resp, &VIEW, &m, &mut opts)
        });
        assert_eq!(an.doc.objs().len(), 2, "an arrow was drawn");
        assert_eq!(bounds(&an, ids[0]), before);
    }

    #[test]
    fn the_move_cross_shows_on_the_shape_not_its_box() {
        let (an, _) = with(
            Tool::Pen,
            vec![rect_ann((100.0, 100.0), (300.0, 300.0), false)],
        );
        // On the outline rectangle's line, and inside its box away from the line.
        let (_, on_line, _) = an.hover(pos2(100.0, 200.0), &VIEW, &m);
        let (_, in_box, _) = an.hover(pos2(200.0, 200.0), &VIEW, &m);
        assert!(on_line && !in_box);
        let ctx = |over_object| crate::engine::cursor::CursorCtx {
            tool: Tool::Pen,
            over_image: true,
            over_chrome: false,
            handle: None,
            over_object,
            over_text: false,
            typing: false,
            dragging: false,
            space_pan: false,
            area: crate::engine::cursor::Area::NotApplicable,
        };
        assert_eq!(
            crate::engine::cursor::pick(&ctx(on_line)),
            egui::CursorIcon::Move
        );
        assert_eq!(
            crate::engine::cursor::pick(&ctx(in_box)),
            egui::CursorIcon::Crosshair
        );
    }

    #[test]
    fn a_new_text_leaves_the_previous_selection_alone() {
        // A label typed after an arrow: changing the text's colour must not recolour the arrow.
        let (mut an, ids) = with(
            Tool::Text,
            vec![rect_ann((100.0, 100.0), (200.0, 200.0), false)],
        );
        an.doc.select(Some(ids[0]), false);
        let mut opts = DrawOpts::default();
        run(Modifiers::NONE, click(pos2(400.0, 400.0)), |ui, resp| {
            an.handle_canvas(ui, resp, &VIEW, &m, &mut opts)
        });
        assert!(an.draft.is_some());
        assert!(an.doc.selected().is_empty());
        let before = opts.clone();
        opts.color = [0, 200, 0, 255];
        an.restyle_from_opts(&before, &opts);
        assert_eq!(an.doc.objs()[0].ann.color, [255, 0, 0, 255]);
    }

    #[test]
    fn cmd_c_and_cmd_x_arrive_as_copy_and_cut() {
        use egui::{Key, Modifiers as M};
        let consume = |modifiers, event: Event, key| {
            let mut i = egui::InputState::default();
            i.modifiers = modifiers;
            i.events = vec![event];
            (consume_command(&mut i, key), i.events.len())
        };
        assert_eq!(consume(M::COMMAND, Event::Copy, Key::C), (true, 0));
        assert_eq!(consume(M::COMMAND, Event::Cut, Key::X), (true, 0));
        assert_eq!(consume(M::COMMAND, Event::Copy, Key::X), (false, 1));
        assert_eq!(consume(M::COMMAND, Event::Cut, Key::C), (false, 1));
        assert_eq!(
            consume(M::SHIFT, Event::Cut, Key::X),
            (false, 1),
            "Shift+Delete"
        );
    }

    #[test]
    fn a_wheel_notch_is_one_width_step() {
        let wheel = |unit, y: f32, modifiers| Event::MouseWheel {
            unit,
            delta: vec2(0.0, y),
            modifiers,
            phase: egui::TouchPhase::Move,
            source: egui::MouseWheelSource::Unknown,
        };
        let mut acc = 0.0;
        assert_eq!(
            wheel_notches(
                &mut acc,
                &[wheel(egui::MouseWheelUnit::Line, 1.0, Modifiers::NONE)]
            ),
            1
        );
        assert_eq!(
            wheel_notches(
                &mut acc,
                &[wheel(egui::MouseWheelUnit::Line, -2.0, Modifiers::NONE)]
            ),
            -2
        );
        // Ctrl+wheel zooms; it is no width step.
        assert_eq!(
            wheel_notches(
                &mut acc,
                &[wheel(egui::MouseWheelUnit::Line, 1.0, Modifiers::CTRL)]
            ),
            0
        );
        // A touchpad: 50 points make a notch, the rest waits.
        let pad: Vec<Event> = (0..7)
            .map(|_| wheel(egui::MouseWheelUnit::Point, 10.0, Modifiers::NONE))
            .collect();
        assert_eq!(wheel_notches(&mut acc, &pad), 1);
        assert_eq!(
            wheel_notches(
                &mut acc,
                &[wheel(egui::MouseWheelUnit::Point, 30.0, Modifiers::NONE)]
            ),
            1
        );
        let mut opts = DrawOpts {
            width: 3.0,
            font_size: 20.0,
            ..Default::default()
        };
        wheel_restyle(Tool::Rect, &mut opts, 1);
        assert_eq!(opts.width, 4.0);
        wheel_restyle(Tool::Text, &mut opts, -1);
        assert_eq!(opts.font_size, 18.0);
    }

    #[test]
    fn a_drawing_tool_on_a_selected_handle_resizes_instead_of_drawing() {
        let (mut an, ids) = with(
            Tool::Rect,
            vec![rect_ann((100.0, 100.0), (200.0, 200.0), false)],
        );
        an.doc.select(Some(ids[0]), false);
        let mut opts = DrawOpts::default();
        let frames = drag(
            pos2(200.0, 200.0),
            &[pos2(250.0, 250.0), pos2(300.0, 300.0)],
        );
        run(Modifiers::NONE, frames, |ui, resp| {
            an.handle_canvas(ui, resp, &VIEW, &m, &mut opts)
        });
        assert_eq!(an.doc.objs().len(), 1, "nothing new was drawn");
        assert!(an.live.is_none());
        match an.doc.objs()[0].ann.shape {
            Shape::Rect(_, b, _) => assert!(b.distance(pos2(300.0, 300.0)) < 0.01),
            _ => unreachable!(),
        }
    }

    #[test]
    fn history_keys_undo_and_redo_including_ctrl_shift_z() {
        use egui::Key;
        let (mut an, _) = with(Tool::Select, vec![]);
        an.commit(rect_ann((0.0, 0.0), (50.0, 50.0), true));
        an.commit(rect_ann((100.0, 0.0), (150.0, 50.0), true));
        let used = run(
            Modifiers::NONE,
            vec![
                key(Key::Z, Modifiers::COMMAND),
                key(Key::Z, Modifiers::COMMAND),
                key(Key::Z, Modifiers::COMMAND | Modifiers::SHIFT),
                key(Key::Y, Modifiers::COMMAND),
                key(Key::A, Modifiers::NONE),
            ],
            |ui, _| {
                let used = an.handle_history_keys(ui);
                (used, an.doc.objs().len())
            },
        );
        assert_eq!(
            used,
            vec![(true, 1), (true, 0), (true, 1), (true, 2), (false, 2)]
        );
    }

    #[test]
    fn holding_an_arrow_is_one_undo_step() {
        use egui::Key;
        let (mut an, ids) = with(
            Tool::Select,
            vec![rect_ann((100.0, 100.0), (200.0, 200.0), true)],
        );
        an.doc.select(Some(ids[0]), false);
        let start = bounds(&an, ids[0]).min;
        let held = |repeat: bool| {
            vec![Event::Key {
                key: Key::ArrowRight,
                physical_key: None,
                pressed: true,
                repeat,
                modifiers: Modifiers::NONE,
            }]
        };
        run(
            Modifiers::NONE,
            vec![held(false), held(true), held(true), held(true)],
            |ui, _| an.handle_object_keys(ui),
        );
        assert!(bounds(&an, ids[0]).min.distance(start + vec2(4.0, 0.0)) < 0.01);
        an.undo();
        assert!(
            bounds(&an, ids[0]).min.distance(start) < 0.01,
            "one step for the whole hold"
        );
    }

    #[test]
    fn a_drawn_object_ends_selected_and_the_tool_stays() {
        let (mut an, _) = with(Tool::Rect, vec![]);
        let mut opts = DrawOpts::default();
        let frames = drag(
            pos2(100.0, 100.0),
            &[pos2(150.0, 150.0), pos2(200.0, 200.0)],
        );
        run(Modifiers::NONE, frames, |ui, resp| {
            an.handle_canvas(ui, resp, &VIEW, &m, &mut opts)
        });
        assert_eq!(an.doc.objs().len(), 1);
        assert_eq!(an.doc.selected(), &[an.doc.objs()[0].id]);
        assert_eq!(an.tool, Tool::Rect);
    }

    #[test]
    fn select_misses_on_empty_space_so_the_capture_can_move_its_area() {
        let (mut an, _) = with(
            Tool::Select,
            vec![rect_ann((0.0, 0.0), (100.0, 100.0), true)],
        );
        let mut opts = DrawOpts::default();
        let out = run(
            Modifiers::NONE,
            drag(pos2(400.0, 400.0), &[pos2(450.0, 450.0)]),
            |ui, resp| an.handle_canvas(ui, resp, &VIEW, &m, &mut opts),
        );
        assert!(out.iter().all(|o| *o == SelectOutcome::Missed));
        let out = run(
            Modifiers::NONE,
            drag(pos2(50.0, 50.0), &[pos2(80.0, 50.0)]),
            |ui, resp| an.handle_canvas(ui, resp, &VIEW, &m, &mut opts),
        );
        assert!(out.contains(&SelectOutcome::Consumed));
    }

    #[test]
    fn double_click_on_a_text_reopens_it_from_the_canvas() {
        let text = Ann {
            shape: Shape::Text {
                pos: pos2(100.0, 100.0),
                text: "hi".into(),
                font: 0,
                size: 20.0,
                boxed: false,
            },
            color: [0, 0, 255, 255],
            width: 3.0,
            filled: false,
        };
        let (mut an, _) = with(Tool::Select, vec![text]);
        let mut opts = DrawOpts::default();
        let p = pos2(105.0, 105.0);
        let mut frames = click(p);
        frames.extend(click(p));
        run(Modifiers::NONE, frames, |ui, resp| {
            an.handle_canvas(ui, resp, &VIEW, &m, &mut opts)
        });
        assert_eq!(an.draft.as_ref().map(|d| d.text.as_str()), Some("hi"));
        assert!(an.doc.objs().is_empty());
    }

    #[test]
    fn object_keys_delete_duplicate_nudge_and_escape() {
        use egui::Key;
        let (mut an, ids) = with(
            Tool::Select,
            vec![rect_ann((100.0, 100.0), (200.0, 200.0), true)],
        );
        let min = |an: &Annotator| bounds(an, an.doc.selected()[0]).min;
        an.doc.select(Some(ids[0]), false);
        let start = min(&an);
        let used = run(
            Modifiers::NONE,
            vec![
                key(Key::ArrowRight, Modifiers::NONE),
                key(Key::ArrowDown, Modifiers::SHIFT),
            ],
            |ui, _| an.handle_object_keys(ui),
        );
        assert_eq!(used, vec![true, true]);
        assert!(min(&an).distance(start + vec2(1.0, 10.0)) < 0.01);
        an.undo();
        assert!(
            min(&an).distance(start + vec2(1.0, 0.0)) < 0.01,
            "one step per press"
        );

        run(
            Modifiers::NONE,
            vec![key(Key::D, Modifiers::COMMAND)],
            |ui, _| an.handle_object_keys(ui),
        );
        assert_eq!(an.doc.objs().len(), 2);
        assert!(
            min(&an).distance(start + vec2(13.0, 12.0)) < 0.01,
            "the copy is 12 px off"
        );

        for k in [Key::Delete, Key::Backspace] {
            let before = an.doc.objs().len();
            an.doc.select(Some(an.doc.objs()[0].id), false);
            run(Modifiers::NONE, vec![key(k, Modifiers::NONE)], |ui, _| {
                an.handle_object_keys(ui)
            });
            assert_eq!(an.doc.objs().len(), before - 1, "{k:?}");
            an.undo();
        }

        an.doc.select(Some(ids[0]), false);
        let esc = run(
            Modifiers::NONE,
            vec![
                key(Key::Escape, Modifiers::NONE),
                key(Key::Escape, Modifiers::NONE),
            ],
            |ui, _| an.handle_object_keys(ui),
        );
        assert_eq!(
            esc,
            vec![true, false],
            "Esc clears first, then is left to the screen"
        );
        assert!(an.doc.selected().is_empty());
        let arrows = run(
            Modifiers::NONE,
            vec![key(Key::ArrowLeft, Modifiers::NONE)],
            |ui, _| an.handle_object_keys(ui),
        );
        assert_eq!(
            arrows,
            vec![false],
            "without a selection the arrows move the capture area"
        );
    }

    #[test]
    fn hover_reports_the_handle_and_the_object_under_the_pointer() {
        let (mut an, ids) = with(
            Tool::Select,
            vec![rect_ann((100.0, 100.0), (200.0, 200.0), true)],
        );
        assert_eq!(an.hover(pos2(150.0, 150.0), &VIEW, &m), (None, true, false));
        assert_eq!(
            an.hover(pos2(200.0, 200.0), &VIEW, &m).0,
            None,
            "not selected yet"
        );
        an.doc.select(Some(ids[0]), false);
        assert_eq!(
            an.hover(pos2(201.0, 199.0), &VIEW, &m),
            (Some(Handle::SE), true, false)
        );
        assert_eq!(
            an.hover(pos2(400.0, 400.0), &VIEW, &m),
            (None, false, false)
        );
        an.dragging = Some(Drag::Resize {
            id: ids[0],
            handle: Handle::N,
        });
        assert_eq!(an.hover(pos2(400.0, 400.0), &VIEW, &m).0, Some(Handle::N));
    }

    #[test]
    fn each_shape_maps_to_the_tool_that_draws_it() {
        let p = pos2(0.0, 0.0);
        let cases = [
            (Shape::Pen(vec![p]), Tool::Pen),
            (Shape::Marker(vec![p]), Tool::Marker),
            (Shape::Line(p, p), Tool::Line),
            (Shape::Arrow(p, p), Tool::Arrow),
            (Shape::Rect(p, p, 0.0), Tool::Rect),
            (Shape::Ellipse(p, p), Tool::Ellipse),
            (Shape::Blur(p, p, BlurMode::Blur), Tool::Blur),
            (
                Shape::Text {
                    pos: p,
                    text: String::new(),
                    font: 0,
                    size: 1.0,
                    boxed: false,
                },
                Tool::Text,
            ),
            (Shape::Counter { center: p, n: 1 }, Tool::Counter),
        ];
        for (s, t) in cases {
            assert_eq!(s.tool(), t, "{s:?}");
        }
    }
}
