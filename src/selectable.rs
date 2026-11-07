//! Added feature (asked for by the person): "the prompt selection options
//! should allow text to be selected taking scrolling into account".
//!
//! A read-only text block that behaves like a text field — drag to select,
//! double-click for a word, Ctrl+A, Ctrl+C, right-click for Select all and
//! Copy — but which lays the *whole* text out and scrolls it itself. Because
//! the widget owns the scroll offset instead of sitting inside a text field
//! that clips its own content, dragging past the top or bottom edge keeps
//! scrolling and keeps extending the selection: a long generation prompt can
//! be selected from its first word to its last even when it does not fit in
//! the box. The selection stays put while the wheel scrolls, so a range picked
//! at the top and a Shift+click at the bottom still join up.

use std::sync::Arc;

use eframe::egui::{self, Color32, FontId, Id, Pos2, Rect, Sense, Stroke, Vec2};

use crate::theme::{ACCENT, BORDER, TEXT};

/// What the widget remembers between frames, keyed by its id in egui's
/// temporary store: the two ends of the selection and whether a drag is in
/// progress. It survives scrolling and repaints, which is the whole point.
#[derive(Clone, Debug, Default)]
struct SelState {
    /// Where the drag started. `None` means nothing is selected.
    anchor: Option<usize>,
    /// The moving end of the selection (also the caret position).
    cursor: usize,
    dragging: bool,
}

impl SelState {
    fn range(&self) -> Option<(usize, usize)> {
        self.anchor.map(|a| (a.min(self.cursor), a.max(self.cursor)))
    }
}

/// The result of drawing one selectable block.
pub struct TextBlock {
    /// The rectangle the block was given on screen (its full height, which can
    /// be taller than the visible part when the text scrolls).
    pub rect: Rect,
    /// The visible part of that rectangle — what the person actually sees.
    pub view: Rect,
    /// The selection as character indices into the full text, when there is one.
    pub selection: Option<(usize, usize)>,
    /// How far the block is scrolled down, in pixels.
    pub scroll: f32,
    /// Whether the block has keyboard focus (so Ctrl+C is its business).
    pub focused: bool,
}

/// How many pixels one frame of edge-dragging scrolls. 10 px per frame is
/// about 600 px/s at 60 Hz — fast enough to cross a long prompt, slow enough
/// to read where the selection is going.
const AUTO_SCROLL_STEP: f32 = 10.0;

/// How close to the top or bottom edge a drag has to get before the box
/// scrolls itself. 12 px is about one line of text: near enough that the person
/// has to mean it, far enough that they do not have to aim at a single pixel.
const EDGE: f32 = 12.0;

/// Character index nearest to a screen position inside the laid-out text.
fn index_at(galley: &egui::Galley, pos: Pos2) -> usize {
    let local = Vec2::new(pos.x, pos.y);
    galley.cursor_from_pos(local).index
}

/// The word around `index`, for a double-click: letters, digits, underscores,
/// hyphens and apostrophes count as one word; everything else is a boundary.
fn word_at(text: &str, index: usize) -> (usize, usize) {
    let chars: Vec<char> = text.chars().collect();
    let in_word = |c: char| c.is_alphanumeric() || c == '_' || c == '-' || c == '\'' || c == '/';
    let mut start = index.min(chars.len());
    let mut end = start;
    while start > 0 && in_word(chars[start - 1]) {
        start -= 1;
    }
    while end < chars.len() && in_word(chars[end]) {
        end += 1;
    }
    if start == end {
        (start, end)
    } else {
        (start, end)
    }
}

/// The text between two character indices.
pub fn slice(text: &str, a: usize, b: usize) -> String {
    let (lo, hi) = (a.min(b), a.max(b));
    text.chars().skip(lo).take(hi - lo).collect()
}

/// Select the whole of the block's text from the outside — the prompt panel's
/// *Select all* button, so the range can be seen before it is copied. The block
/// picks the state up on its next frame.
pub fn select_all(ctx: &egui::Context, id: Id, text_len: usize) {
    let state = SelState {
        anchor: Some(0),
        cursor: text_len,
        dragging: false,
    };
    ctx.data_mut(|d| d.insert_temp(id, state));
}

/// Draw `text` as a read-only block that can be selected across its whole
/// length, in a box at most `max_height` pixels tall. `id` keys the selection
/// so two blocks (a prompt and its negative prompt) keep their own.
pub fn text_block(ui: &mut egui::Ui, id: Id, text: &str, max_height: f32) -> TextBlock {
    const PAD: f32 = 6.0;
    const GUTTER: f32 = 12.0; // room for the scroll bar

    let font = FontId::proportional(11.5);
    let wrap_width = (ui.available_width() - 2.0 * PAD - GUTTER).max(40.0);
    let galley: Arc<egui::Galley> = ui
        .painter()
        .layout(text.to_owned(), font, TEXT, wrap_width);
    let content_h = galley.size().y + 2.0 * PAD;
    // Cut the box on a line boundary rather than through the middle of a row:
    // a half-drawn line at the bottom looks like a bug and hides the fact that
    // there is more text below.
    let line_h = galley.rows.first().map_or(14.0, |r| r.height()).max(1.0);
    let rows_fit = ((max_height - 2.0 * PAD) / line_h).floor().max(1.0);
    let snapped = rows_fit * line_h + 2.0 * PAD;
    let height = content_h.min(snapped.max(24.0));
    let scrolls = content_h > height + 0.5;

    // The selection lives in egui's per-frame store, keyed by the block's id.
    let mut state: SelState = ui.data(|d| d.get_temp(id).unwrap_or_default());
    let mut copied: Option<String> = None;

    let mut draw = |ui: &mut egui::Ui, state: &mut SelState, block: &mut TextBlock| {
        let size = Vec2::new(ui.available_width(), content_h);
        let (rect, mut resp) = ui.allocate_exact_size(size, Sense::click_and_drag());
        resp = resp.on_hover_cursor(egui::CursorIcon::Text);
        block.rect = rect;
        block.view = rect.intersect(ui.clip_rect());
        let origin = rect.min + Vec2::splat(PAD);

        let (pointer, pressed, down, released, shift) = ui.input(|i| {
            (
                i.pointer.interact_pos(),
                i.pointer.primary_pressed(),
                i.pointer.primary_down(),
                i.pointer.primary_released(),
                i.modifiers.shift,
            )
        });

        // A press inside the text starts a new selection unless Shift is held,
        // in which case it moves the far end of the one already there.
        if let (true, Some(pos)) = (pressed, pointer) {
            if rect.contains(pos) && !resp.context_menu_opened() {
                resp.request_focus();
                let idx = index_at(&galley, pos - origin.to_vec2());
                if !shift || state.anchor.is_none() {
                    state.anchor = Some(idx);
                }
                state.cursor = idx;
                state.dragging = true;
            }
        }

        // Dragging keeps the selection following the pointer, including when
        // the pointer is over the box's border or past it.
        if state.dragging {
            if down {
                if let Some(pos) = pointer {
                    let idx = index_at(&galley, pos - origin.to_vec2());
                    state.cursor = idx;
                    // Past the top or bottom of the visible part, keep scrolling
                    // in that direction; the galley moves under the pointer, so
                    // the next frame selects text the person could not see yet.
                    let view = block.view;
                    if scrolls && pos.y > view.bottom() - EDGE {
                        ui.scroll_with_delta(Vec2::new(0.0, -AUTO_SCROLL_STEP));
                        ui.ctx().request_repaint();
                    } else if scrolls && pos.y < view.top() + EDGE {
                        ui.scroll_with_delta(Vec2::new(0.0, AUTO_SCROLL_STEP));
                        ui.ctx().request_repaint();
                    }
                }
            } else if released || !down {
                state.dragging = false;
            }
        }

        // A double-click picks the word under the pointer, as a text field
        // would. A shifted double-click is a Shift+click, so it is left alone.
        if resp.double_clicked() && !shift {
            if let Some(pos) = pointer {
                let idx = index_at(&galley, pos - origin.to_vec2());
                let (a, b) = word_at(text, idx);
                state.anchor = Some(a);
                state.cursor = b;
            }
        }

        // Keyboard, only while this block has the focus so the file list keeps
        // its own Ctrl+A.
        if resp.has_focus() {
            let select_all = ui.input(|i| {
                i.modifiers.command && i.key_pressed(egui::Key::A) && !i.modifiers.shift
            });
            if select_all {
                state.anchor = Some(0);
                state.cursor = text.chars().count();
            }
            let escape = ui.input(|i| i.key_pressed(egui::Key::Escape));
            if escape {
                state.anchor = None;
                state.cursor = 0;
            }
            let wants_copy = ui.input(|i| {
                i.events
                    .iter()
                    .any(|e| matches!(e, egui::Event::Copy))
            });
            if wants_copy {
                if let Some((a, b)) = state.range() {
                    copied = Some(slice(text, a, b));
                }
            }
        }

        // Right-click gives the selection options a menu rather than asking
        // the person to remember the keys.
        let mut ask_all = false;
        let mut ask_copy = false;
        resp.context_menu(|ui| {
            ui.set_min_width(150.0);
            if ui.button("Select all").clicked() {
                ask_all = true;
                ui.close();
            }
            let has_selection = state.range().is_some_and(|(a, b)| b > a);
            if ui
                .add_enabled(has_selection, egui::Button::new("Copy selection"))
                .clicked()
            {
                ask_copy = true;
                ui.close();
            }
        });
        if ask_all {
            state.anchor = Some(0);
            state.cursor = text.chars().count();
        }
        if ask_copy {
            if let Some((a, b)) = state.range() {
                copied = Some(slice(text, a, b));
            }
        }

        // Box around the text, then the selection band, then the glyphs. The
        // text is clipped one padding step above the bottom edge so a row that
        // does not fit is not left half-drawn: a sliver of the next line reads
        // as a rendering bug rather than as "there is more below".
        let view = block.view;
        let clip = Rect::from_min_max(
            view.min,
            Pos2::new(view.right(), (view.bottom() - PAD).max(view.top() + 1.0)),
        );
        let painter = ui.painter().with_clip_rect(clip);
        painter.rect_filled(rect, 3.0, Color32::from_rgb(33, 37, 40));
        painter.rect_stroke(
            rect,
            3.0,
            Stroke::new(
                1.0,
                if resp.has_focus() { ACCENT } else { BORDER },
            ),
            egui::StrokeKind::Inside,
        );
        if let Some((a, b)) = state.range() {
            if b > a {
                let band = Color32::from_rgba_unmultiplied(61, 174, 233, 96);
                let mut index = 0usize;
                for row in &galley.rows {
                    let start = index;
                    let end = index + row.char_count_excluding_newline();
                    let lo = a.max(start).min(end);
                    let hi = b.max(start).min(end);
                    if hi > lo {
                        let x0 = row.x_offset(lo - start) + row.pos.x;
                        let x1 = row.x_offset(hi - start) + row.pos.x;
                        let r = Rect::from_min_max(
                            Pos2::new(origin.x + x0, origin.y + row.min_y()),
                            Pos2::new(origin.x + x1, origin.y + row.max_y()),
                        );
                        painter.rect_filled(r, 1.0, band);
                    }
                    index += row.char_count_including_newline();
                }
            }
        }
        painter.galley(origin, galley.clone(), TEXT);

        // A hairline down the right edge shows how much of the text is in view.
        if scrolls {
            let track = Rect::from_min_max(
                Pos2::new(rect.right() - 5.0, rect.top() + 3.0),
                Pos2::new(rect.right() - 2.0, rect.bottom() - 3.0),
            );
            painter.rect_filled(track, 2.0, Color32::from_rgb(48, 53, 57));
            let frac = (block.view.height() / rect.height()).clamp(0.06, 1.0);
            let top = rect.top()
                + (rect.height() - track.height() * frac)
                    * if rect.height() > block.view.height() {
                        ((block.view.top() - rect.top()) / (rect.height() - block.view.height()))
                            .clamp(0.0, 1.0)
                    } else {
                        0.0
                    };
            let thumb = Rect::from_min_size(
                Pos2::new(track.left(), top),
                Vec2::new(track.width(), track.height() * frac),
            );
            painter.rect_filled(thumb, 2.0, Color32::from_rgb(96, 105, 112));
        }

        if state.range().is_some_and(|(a, b)| b > a) {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Text);
        }
        block.selection = state.range().filter(|(a, b)| b > a);
        block.focused = resp.has_focus();
    };

    let mut block = TextBlock {
        rect: Rect::NOTHING,
        view: Rect::NOTHING,
        selection: None,
        scroll: 0.0,
        focused: false,
    };

    if scrolls {
        let out = egui::ScrollArea::vertical()
            .id_salt(id.with("scroll"))
            .max_height(height)
            .auto_shrink([false, false])
            .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
            .show(ui, |ui| {
                draw(ui, &mut state, &mut block);
            });
        block.scroll = out.state.offset.y;
        // The viewport, not the whole content, is what counts as visible.
        block.view = block.rect.intersect(out.inner_rect);
    } else {
        draw(ui, &mut state, &mut block);
    }

    ui.data_mut(|d| d.insert_temp(id, state.clone()));
    if let Some(text) = copied {
        ui.ctx().copy_text(text);
    }
    block
}
