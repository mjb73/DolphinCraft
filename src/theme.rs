//! Colours, spacing and hand-drawn icons (no external image assets).

use eframe::egui::{
    self, epaint::PathStroke, Align2, Color32, CornerRadius, FontData, FontDefinitions, FontFamily,
    FontId, Pos2, Rect, Stroke, TextStyle, Vec2,
};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::fsutil::Kind;

// ---------------------------------------------------------------------------
// Palette — dark and light
// ---------------------------------------------------------------------------

/// Added feature, asked for by the person: dark and light, switched from the app
/// menu's View group.
///
/// The light palette is a second, designed rendering of the same system, not the
/// dark one flipped. Every role is written down as a pair — the dark value and
/// the light value — because a single `1 − L^1.6` derivation squashes the whole
/// near-black surface ladder into five RGB levels of near-white and turns the
/// window's border into its brightest line. The pairs are checked, in
/// `selftest`, against the same contrast targets in both modes: surfaces a
/// visible step apart, ink at body contrast, hairlines that survive, an accent
/// that still reads on paper, and a selection band dark enough to carry white
/// text. `c()` remains for the icon colours and one-off tints, where a
/// lightness flip happens to be the right move.
static LIGHT: AtomicBool = AtomicBool::new(false);

/// Is the light palette in use?
pub fn light() -> bool {
    LIGHT.load(Ordering::Relaxed)
}

/// Choose the palette. The window has to be restyled afterwards
/// (`apply_style`), because egui's own widget colours are set once and kept.
pub fn set_light(on: bool) {
    LIGHT.store(on, Ordering::Relaxed);
}

/// `r, g, b` as the current mode draws it, for the icon colours and one-off
/// tints that have no light value of their own. Light mode keeps the hue and
/// flips the lightness, stretched — `1 − L^1.6` — so the near-black surfaces of
/// the dark palette come out near-white (L 0.15 → 0.95) instead of a muddy grey,
/// the near-white text lands near black (0.94 → 0.09), and a mid-tone icon
/// colour barely moves at all. Saturation gives a little, as it does in any light
/// scheme. The window's own roles do not use this: their light values are written
/// beside their dark ones, because a flip cannot keep a ladder or a hairline.
pub fn c(r: u8, g: u8, b: u8) -> Color32 {
    if !light() {
        return Color32::from_rgba_unmultiplied(r, g, b, 255);
    }
    let (h, s, l) = rgb_to_hsl(r, g, b);
    let l2 = (1.0 - l.powf(1.6)).clamp(0.04, 0.98);
    hsl_to_colour(h, s * 0.92, l2)
}

fn rgb_to_hsl(r: u8, g: u8, b: u8) -> (f32, f32, f32) {
    let (r, g, b) = (r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) / 2.0;
    if (max - min).abs() < f32::EPSILON {
        return (0.0, 0.0, l);
    }
    let d = max - min;
    let s = if l > 0.5 { d / (2.0 - max - min) } else { d / (max + min) };
    let h = if max == r {
        ((g - b) / d + if g < b { 6.0 } else { 0.0 }) / 6.0
    } else if max == g {
        ((b - r) / d + 2.0) / 6.0
    } else {
        ((r - g) / d + 4.0) / 6.0
    };
    (h, s, l)
}

fn hsl_to_colour(h: f32, s: f32, l: f32) -> Color32 {
    let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    if s <= 0.0 {
        let v = byte(l);
        return Color32::from_rgba_unmultiplied(v, v, v, 255);
    }
    let q = if l < 0.5 { l * (1.0 + s) } else { l + s - l * s };
    let p = 2.0 * l - q;
    let channel = |mut t: f32| {
        if t < 0.0 {
            t += 1.0;
        }
        if t > 1.0 {
            t -= 1.0;
        }
        if t < 1.0 / 6.0 {
            p + (q - p) * 6.0 * t
        } else if t < 0.5 {
            q
        } else if t < 2.0 / 3.0 {
            p + (q - p) * (2.0 / 3.0 - t) * 6.0
        } else {
            p
        }
    };
    Color32::from_rgba_unmultiplied(
        byte(channel(h + 1.0 / 3.0)),
        byte(channel(h)),
        byte(channel(h - 1.0 / 3.0)),
        255,
    )
}

// The palette, as pairs: the dark value first, then the light one written for
// paper. The light surfaces climb down from the view (the paper the files sit
// on) through the panel and the toolbar, each a measured step, so the window
// keeps the ladder the dark palette has instead of flattening into one white.
/// The view behind the file rows. Light: paper, not `#fff` — a tinted white.
pub fn view_bg() -> Color32 {
    pair((35, 38, 41), (251, 252, 253))
}
/// The side panel and the find bar's strip. One step under the view.
pub fn panel_bg() -> Color32 {
    pair((43, 47, 51), (238, 242, 245))
}
/// The toolbar and dialog chrome. Two steps under, the deepest surface.
pub fn toolbar_bg() -> Color32 {
    pair((49, 54, 59), (230, 235, 240))
}
/// The details view's column header. A hair under the panel, over the rows.
pub fn header_bg() -> Color32 {
    pair((46, 51, 56), (239, 243, 246))
}
/// A row under the pointer.
pub fn row_hover() -> Color32 {
    pair((57, 63, 69), (225, 231, 237))
}
/// The selected row of the view the person is working in. In light mode the band
/// stays a band — a mid blue the white text on it reads against — rather than
/// turning into a pale wash that could be mistaken for the hover.
pub fn row_sel_active() -> Color32 {
    pair((42, 104, 143), (47, 121, 172))
}
/// The selected row of the view beside it.
pub fn row_sel_inactive() -> Color32 {
    pair((64, 72, 79), (208, 219, 229))
}
/// Text drawn on the selection band, or on a button filled with `accent()`:
/// white in both palettes, because the band and the filled button are the same
/// dark band in both. Only the surfaces around them change mode.
pub fn on_band() -> Color32 {
    Color32::WHITE
}

/// Text drawn on a dark scrim laid over a photograph (the viewer's name strips).
/// The scrim is black in both modes, so its text stays light in both.
pub fn on_scrim() -> Color32 {
    Color32::from_rgba_unmultiplied(226, 232, 238, 255)
}

/// The outline of a document page. In the dark palette the page's own brightness
/// is its edge, so this is the page colour and the stroke is invisible; in the
/// light one an off-white page on off-white paper has no shape at all without a
/// grey line around it.
pub fn paper_edge() -> Color32 {
    pair((226, 230, 233), (170, 178, 185))
}

/// A colour with its own light value. Every role in the window takes one, since
/// the single-formula derivation below cannot hold a surface ladder, a border or
/// an accent at the contrast they need on paper.
fn pair(dark: (u8, u8, u8), light: (u8, u8, u8)) -> Color32 {
    let (r, g, b) = if self::light() { light } else { dark };
    Color32::from_rgba_unmultiplied(r, g, b, 255)
}

/// File names and other primary text. Light: a tinted near-black at 16:1 on
/// paper, never `#000`.
pub fn text() -> Color32 {
    pair((239, 240, 241), (26, 29, 32))
}
/// Secondary text: sizes, dates, hints. Written out in light mode, where the
/// derivation leaves it too pale to read on white.
pub fn text_dim() -> Color32 {
    pair((160, 168, 174), (86, 94, 100))
}
/// Tertiary text: counts and captions. Likewise written out, and held at 4.5:1
/// or better on the panel and the toolbar as well as on the view — a count in
/// the side panel is still text.
pub fn text_faint() -> Color32 {
    pair((144, 152, 159), (98, 107, 115))
}
/// The one accent: the active sort column, the chosen chip, the focus ring.
/// Light: the same hue, dropped in lightness until it clears 3:1 on paper, so
/// the ring and the sort arrow stay visible on white.
pub fn accent() -> Color32 {
    pair((61, 174, 233), (27, 110, 158))
}
/// A filled primary button. The same fill in both modes, because a filled button
/// is a dark plate whatever the window: white text on it clears 4.5:1.
pub fn action_fill() -> Color32 {
    pair((45, 112, 163), (45, 112, 163))
}
/// A filled destructive button (Empty Trash, Delete Permanently).
pub fn danger_fill() -> Color32 {
    pair((170, 68, 62), (170, 68, 62))
}
/// The window's outer edge. Light: the darkest line in the window, so the frame
/// still reads as a frame instead of glowing brighter than the view it holds.
pub fn border() -> Color32 {
    pair((27, 30, 33), (196, 204, 212))
}
/// The hairline between rows and columns.
pub fn separator() -> Color32 {
    pair((53, 59, 64), (214, 221, 228))
}
/// The folder icon's body.
pub fn folder() -> Color32 {
    pair((76, 148, 201), (58, 134, 187))
}
/// The folder icon's lit top edge. Lighter than the body in both modes: a
/// highlight that flips to a shadow makes the icon read as a cut-out.
pub fn folder_top() -> Color32 {
    pair((104, 172, 219), (91, 161, 208))
}
/// The folder icon's bottom shade. Darker than the body in both modes.
pub fn folder_shade() -> Color32 {
    pair((58, 122, 168), (43, 108, 153))
}
/// The document icon's page. Written out: `c()` would turn a white page into a
/// black one, and a file is drawn on paper in either mode. Light: a warm
/// off-white, not `#fff`, with `paper_edge` giving it its outline.
pub fn paper() -> Color32 {
    pair((226, 230, 233), (255, 253, 248))
}
/// The fold and the page's edge.
pub fn paper_shade() -> Color32 {
    pair((188, 194, 199), (176, 184, 190))
}
/// The glyph of a place in the side panel.
pub fn place() -> Color32 {
    pair((150, 178, 198), (70, 100, 120))
}

/// The original's ceiling: `ZoomLevelInfo::maximumLevel()` @0x2a6370 returns 16,
/// and level 16 is 256 px. Everything up to here is measured, not chosen.
pub const ORIGINAL_MAX_ZOOM: u8 = 16;
/// Added feature: the ladder continues the original's own `level << 4` formula
/// past its ceiling, so a folder of photographs can be browsed at 400 px.
/// Levels 17..=25 are 272, 288, … 400 px; nothing below level 17 changes.
pub const MAX_ZOOM: u8 = 25;
/// Largest thumbnail edge the rebuilt views draw (see `MAX_ZOOM`).
pub const MAX_ICON_PX: f32 = 400.0;

/// A slider that fits inside a one-row strip (the picture strip above the icon
/// grid, the gallery's size bar, the status bar).
///
/// egui takes a slider's knob radius from the rect's height, and that height is
/// the row's interacting height — `interact_size.y`, 24 px here — so a slider
/// dropped into a 22 px strip drew a knob taller than the strip itself, poking
/// out above and below the 4 px track. Inside a strip the knob is a small dot
/// and the row keeps the track's own height instead.
pub fn strip_slider(
    ui: &mut egui::Ui,
    value: &mut f32,
    range: std::ops::RangeInclusive<f32>,
    width: f32,
) -> egui::Response {
    ui.scope(|ui| {
        ui.spacing_mut().interact_size.y = 12.0;
        ui.style_mut().visuals.handle_shape = egui::style::HandleShape::Circle;
        ui.spacing_mut().slider_width = width;
        ui.add(
            egui::Slider::new(value, range)
                .show_value(false)
                .step_by(1.0),
        )
    })
    .inner
}

/// Icon sizes for zoom levels 0..=25. Levels 0..=16 are measured from
/// ZoomLevelInfo; levels 17..=25 are the added continuation.
pub const ZOOM_SIZES: [u16; 26] = [
    16, 22, 32, 48, 64, 80, 96, 112, 128, 144, 160, 176, 192, 208, 224, 240, 256, 272, 288, 304,
    320, 336, 352, 368, 384, 400,
];

/// `ZoomLevelInfo::iconSizeForZoomLevel` @0x2a6380 in libdolphinprivate.so.26.08.3:
/// `size = level << 4`, and for levels 0..4 the table at 0x2cd960 (16, 22, 32,
/// 48, 64) overrides it — a formula that keeps working past the original's
/// own 16-level ceiling.
pub fn icon_size_for_zoom(zoom: u8) -> f32 {
    ZOOM_SIZES[(zoom as usize).min(MAX_ZOOM as usize)] as f32
}

/// `ZoomLevelInfo::zoomLevelForIconSize` @0x2a63a0: 16→0, 22→1, 32→2, 48→3,
/// 64→4, otherwise `4 + (size - 64) / 16`. The original clamps to its
/// 0..16 range (`minimumLevel` 0x2a6360 / `maximumLevel` 0x2a6370); the rebuild
/// clamps to the extended 0..=25 range so 400 px is reachable.
pub fn zoom_for_icon_size(size: f32) -> u8 {
    let px = size.round() as i32;
    let level = match px {
        16 => 0,
        22 => 1,
        32 => 2,
        48 => 3,
        64 => 4,
        _ => 4 + (px - 64).div_euclid(16),
    };
    level.clamp(0, MAX_ZOOM as i32) as u8
}

/// Factory icon size per view mode, read out of the settings constructors:
/// `IconsModeSettings` 0x2b1ac0 → IconSize 32, `CompactModeSettings` 0x2af670
/// → 16, `DetailsModeSettings` 0x2b0f10 → 16. Order is Icons, Compact, Details.
pub const DEFAULT_ICON_SIZE: [u16; 3] = [32, 16, 16];

/// Factory preview size per mode from the same constructors: Icons 64,
/// Compact 48, Details 48.
pub const DEFAULT_PREVIEW_SIZE: [u16; 3] = [64, 48, 48];

/// `DolphinView::updateDefaultZoomLevel` @0x27dd60: the starting zoom is the
/// level whose icon size matches this mode's `PreviewSize` when previews are
/// on, and its `IconSize` when they are off.
pub fn default_zoom(previews: bool) -> [u8; 3] {
    let mut out = [0u8; 3];
    for mode in 0..3 {
        let size = if previews {
            DEFAULT_PREVIEW_SIZE[mode]
        } else {
            DEFAULT_ICON_SIZE[mode]
        };
        out[mode] = zoom_for_icon_size(size as f32);
    }
    out
}

/// `IconsModeSettings::TextWidthIndex` factory value (ctor 0x2b1ac0: the
/// `ItemInt` default is 1).
pub const TEXT_WIDTH_INDEX: f32 = 1.0;

/// Width of one cell in the icons view, straight out of
/// `DolphinItemListView::updateGridSize` @0x26d940:
///
///     d = exp(zoom / 13)
///     width = d * TextWidthIndex * 64 * (averageCharWidth / 9) + 48
///     width = max(width, d * 4 + iconSize)
///
/// The original truncates the result to a whole pixel.
pub fn icon_grid_cell_width(zoom: u8, icon: f32, average_char_width: f32) -> f32 {
    let d = (zoom as f32 / 13.0).exp();
    let text_part = d * (TEXT_WIDTH_INDEX * 64.0) * (average_char_width / 9.0) + 48.0;
    let floor = d * 4.0 + icon;
    text_part.max(floor).floor()
}

/// The original's icons view measured in `checks/orig-zoom/`: a view 592 px
/// wide fits `floor(width / cell)` columns and then *spreads* them evenly, so
/// 160 px cells give 3 columns with centres 197 px apart (592 / 3) and 64 px
/// cells give 4 columns 148 px apart. `icon_grid_cell_width` gives the rebuild
/// the same column count at those widths; the cells themselves are packed to
/// their picture's own width (no empty bands), so its pitch is a little
/// tighter. The selftest holds the rebuild to the original's columns.
pub const ORIGINAL_ICON_PITCH_160: f32 = 197.0;
pub const ORIGINAL_ICON_COLUMNS_160: usize = 3;
pub const ORIGINAL_ICON_VIEW_WIDTH: f32 = 592.0;

/// The details view's side padding: `DetailsModeSettings::DetailsModeSettings`
/// @0x2b0f10 registers `LeftPadding` and `RightPadding` with the factory value
/// 0x14 = **20 px** each (`ItemUInt::ItemUInt(..., 0x14)` at +0x50 and +0x54),
/// and `KItemListView::setSidePadding(left, right)` is what consumes them — it
/// insets the header and every row from the two edges of the view. The rebuild
/// draws its details columns inside the same 20 px.
pub const DETAILS_SIDE_PAD: f32 = 20.0;

/// Width of one cell in the compact view, from the same function:
/// `iconSize + 8 + 5 * fontHeight`.
pub fn compact_cell_width(icon: f32, font_height: f32) -> f32 {
    icon + 8.0 + font_height * 5.0
}


pub fn install(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    let regular = "/usr/share/fonts/noto/NotoSans-Regular.ttf";
    let bold = "/usr/share/fonts/noto/NotoSans-Bold.ttf";
    if let Ok(data) = std::fs::read(regular) {
        fonts
            .font_data
            .insert("noto".to_owned(), Arc::new(FontData::from_owned(data)));
        if let Some(list) = fonts.families.get_mut(&FontFamily::Proportional) {
            list.insert(0, "noto".to_owned());
        }
    }
    if let Ok(data) = std::fs::read(bold) {
        fonts
            .font_data
            .insert("noto-bold".to_owned(), Arc::new(FontData::from_owned(data)));
        if let Some(list) = fonts.families.get_mut(&FontFamily::Proportional) {
            list.push("noto-bold".to_owned());
        }
    }
    if let Ok(data) = std::fs::read("/usr/share/fonts/TTF/JetBrainsMono-Regular.ttf") {
        fonts.font_data.insert(
            "jetbrains".to_owned(),
            Arc::new(FontData::from_owned(data)),
        );
        if let Some(list) = fonts.families.get_mut(&FontFamily::Monospace) {
            list.insert(0, "jetbrains".to_owned());
        }
    }
    ctx.set_fonts(fonts);

    apply_style(ctx);
}

/// The window's colours and spacing, from the palette in force. Called once at
/// startup and again whenever the person switches between dark and light, which
/// is why the fonts are set in `install` and only the style is here.
pub fn apply_style(ctx: &egui::Context) {
    let mut style = (*ctx.style()).clone();
    let mut v = egui::Visuals::dark();
    v.panel_fill = panel_bg();
    v.window_fill = toolbar_bg();
    v.window_stroke = Stroke::new(1.0, border());
    v.extreme_bg_color = pair((30, 33, 36), (243, 246, 249));
    v.faint_bg_color = pair((48, 53, 58), (236, 241, 246));
    v.selection.bg_fill = row_sel_active();
    v.selection.stroke = Stroke::new(1.0, on_band());
    v.hyperlink_color = accent();
    v.slider_trailing_fill = true;
    v.widgets.noninteractive.bg_fill = Color32::TRANSPARENT;
    v.widgets.noninteractive.weak_bg_fill = Color32::TRANSPARENT;
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, separator());
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, text_dim());
    v.widgets.inactive.bg_fill = pair((56, 62, 68), (232, 237, 242));
    v.widgets.inactive.weak_bg_fill = pair((56, 62, 68), (232, 237, 242));
    v.widgets.inactive.bg_stroke = Stroke::NONE;
    v.widgets.inactive.fg_stroke = Stroke::new(1.0, text());
    v.widgets.hovered.bg_fill = pair((70, 78, 85), (219, 226, 233));
    v.widgets.hovered.weak_bg_fill = pair((70, 78, 85), (219, 226, 233));
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, pair((84, 92, 99), (190, 199, 208)));
    v.widgets.hovered.fg_stroke = Stroke::new(1.0, pair((255, 255, 255), (26, 29, 32)));
    v.widgets.active.bg_fill = c(61, 174, 233);
    v.widgets.active.weak_bg_fill = c(61, 174, 233);
    // `RichText::strong()` takes its colour from the active widget visuals, so
    // this is the colour of every emphasised name in the window (the location
    // bar's current folder, a file's name in its properties). It has to follow
    // the palette: left as a dark literal it turned near-white in light mode and
    // those names vanished.
    v.widgets.active.fg_stroke = Stroke::new(1.0, text());
    v.widgets.open.bg_fill = pair((70, 78, 85), (219, 226, 233));
    v.widgets.open.weak_bg_fill = pair((70, 78, 85), (219, 226, 233));
    v.widgets.open.fg_stroke = Stroke::new(1.0, accent());
    v.window_corner_radius = CornerRadius::same(6);
    v.menu_corner_radius = CornerRadius::same(6);
    style.visuals = v;

    style.spacing.item_spacing = Vec2::new(6.0, 4.0);
    style.spacing.button_padding = Vec2::new(7.0, 3.0);
    style.spacing.interact_size = Vec2::new(24.0, 24.0);
    style.spacing.scroll = egui::style::ScrollStyle::solid();
    style.spacing.scroll.bar_width = 9.0;
    style.text_styles.insert(TextStyle::Body, FontId::proportional(13.0));
    style.text_styles.insert(TextStyle::Button, FontId::proportional(13.0));
    style.text_styles.insert(TextStyle::Small, FontId::proportional(11.0));
    style.text_styles.insert(TextStyle::Heading, FontId::proportional(15.5));
    style.text_styles.insert(TextStyle::Monospace, FontId::monospace(12.0));
    ctx.set_style(style);
}

/// Cut a string to fit `max_w`, appending an ellipsis.
pub fn elide(ui: &egui::Ui, text: &str, max_w: f32, font: FontId) -> String {
    let galley = ui.fonts_mut(|f| f.layout_no_wrap(text.to_owned(), font.clone(), self::text()));
    if galley.size().x <= max_w {
        return text.to_owned();
    }
    let ell = "…";
    let mut lo = 0usize;
    let mut hi = text.chars().count();
    while lo < hi {
        let mid = (lo + hi + 1) / 2;
        let candidate: String = text.chars().take(mid).collect::<String>() + ell;
        let w = ui
            .fonts_mut(|f| f.layout_no_wrap(candidate, font.clone(), self::text()))
            .size()
            .x;
        if w <= max_w {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    let mut s: String = text.chars().take(lo).collect();
    s.push_str(ell);
    s
}

/// File names are cut in the **middle**, not at the end. The original's own
/// switch for this is `[ContentDisplay] ElidingMode`, registered by
/// `ContentDisplaySettings::ContentDisplaySettings` @0x2b29b0 in
/// `libdolphinprivate.so.6` with exactly two choices, `Right` = 0 and
/// `Middle` = 1, and the default passed to `ItemEnum` is **1** — so a fresh
/// profile elides in the middle and keeps both the start of the name and the
/// extension visible. Only the file names follow this; menu rows and the
/// location bar keep the right-hand cut Qt gives them.
pub fn elide_name(ui: &egui::Ui, text: &str, max_w: f32, font: FontId) -> String {
    let width = |s: &str, ui: &egui::Ui| {
        ui.fonts_mut(|f| f.layout_no_wrap(s.to_owned(), font.clone(), self::text()))
            .size()
            .x
    };
    if width(text, ui) <= max_w {
        return text.to_owned();
    }
    let chars: Vec<char> = text.chars().collect();
    // Keep 5/9 of the room for the head and 4/9 for the tail: the extension at
    // the end is short, the folder-ish start of a name is what tells files
    // apart.
    let mut best = "…".to_string();
    let mut lo = 2usize;
    let mut hi = chars.len().saturating_sub(1);
    while lo <= hi {
        let keep = (lo + hi) / 2;
        let head = ((keep * 5 + 4) / 9).max(1);
        let tail = keep.saturating_sub(head);
        let mut s: String = chars[..head].iter().collect();
        s.push('…');
        if tail > 0 {
            s.extend(chars[chars.len() - tail..].iter());
        }
        if width(&s, ui) <= max_w {
            best = s;
            lo = keep + 1;
        } else {
            hi = keep - 1;
        }
    }
    best
}

// ---------------------------------------------------------------------------
// File icons
// ---------------------------------------------------------------------------

pub fn draw_kind_icon(painter: &egui::Painter, rect: Rect, kind: Kind, dim: bool) {
    draw_kind_icon_tinted(painter, rect, kind, dim, None)
}

/// The same icon, but with a folder drawn in the person's colour. Files keep
/// their own look: only an entry that is a folder takes a tint.
pub fn draw_kind_icon_tinted(
    painter: &egui::Painter,
    rect: Rect,
    kind: Kind,
    dim: bool,
    tint: Option<Color32>,
) {
    let alpha = if dim { 120 } else { 255 };
    match kind {
        Kind::Folder => draw_folder(painter, rect, alpha, tint),
        _ => draw_document(painter, rect, kind, alpha),
    }
}

fn fade(c: Color32, alpha: u8) -> Color32 {
    Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), alpha)
}

/// How far a place's colour is lifted towards white when its row is marked.
///
/// A marked row sits on the selection band, so its icon has to read against
/// that band. The original lifts selected symbolic icons all the way to the
/// highlight text colour (white), which is why a shortcut that was given a
/// colour looked plain the moment it was clicked. The rebuild keeps the hue
/// and only lightens it: 0.32 is enough for a dark purple to stand out on the
/// band while a red still reads as red.
pub const SELECTION_LIFT: f32 = 0.32;

/// The colour an icon keeps on the marked row: the same hue, lifted towards
/// white so it is legible on the selection band.
pub fn selection_tint(colour: Color32) -> Color32 {
    blend(colour, Color32::WHITE, SELECTION_LIFT)
}

/// Blend `a` towards `b` by `t` (0..=1).
fn blend(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let m = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(m(a.r(), b.r()), m(a.g(), b.g()), m(a.b(), b.b()))
}

fn draw_folder(painter: &egui::Painter, rect: Rect, alpha: u8, tint: Option<Color32>) {
    let (body, top, shade) = match tint {
        Some(base) => (
            base,
            blend(base, Color32::WHITE, 0.22),
            blend(base, Color32::BLACK, 0.20),
        ),
        None => (
            folder(),
            folder_top(),
            folder_shade(),
        ),
    };
    let (w, h) = (rect.width(), rect.height());
    let x0 = rect.left() + w * 0.05;
    let x1 = rect.right() - w * 0.05;
    let y0 = rect.top() + h * 0.22;
    let y1 = rect.bottom() - h * 0.08;
    let r = CornerRadius::same((h * 0.07).clamp(1.0, 4.0) as u8);
    // Tab behind the body.
    let tab = Rect::from_min_max(
        Pos2::new(x0 + w * 0.02, y0 - h * 0.11),
        Pos2::new(x0 + w * 0.44, y0 + h * 0.04),
    );
    painter.rect_filled(tab, r, fade(top, alpha));
    // Body.
    let body_rect = Rect::from_min_max(Pos2::new(x0, y0), Pos2::new(x1, y1));
    painter.rect_filled(body_rect, r, fade(body, alpha));
    // Top edge highlight.
    let hi = Rect::from_min_max(
        Pos2::new(x0 + w * 0.02, y0 + h * 0.02),
        Pos2::new(x1 - w * 0.02, y0 + h * 0.09),
    );
    painter.rect_filled(hi, r, fade(top, alpha));
    // Slight inner shading at the bottom.
    let shade_rect = Rect::from_min_max(
        Pos2::new(x0 + w * 0.03, y1 - h * 0.10),
        Pos2::new(x1 - w * 0.03, y1 - h * 0.02),
    );
    painter.rect_filled(shade_rect, r, fade(shade, alpha));
}

fn draw_document(painter: &egui::Painter, rect: Rect, kind: Kind, alpha: u8) {
    let (w, h) = (rect.width(), rect.height());
    let x0 = rect.left() + w * 0.18;
    let x1 = rect.right() - w * 0.18;
    let y0 = rect.top() + h * 0.07;
    let y1 = rect.bottom() - h * 0.07;
    let fold = (w * 0.26).min(h * 0.24);

    let page = vec![
        Pos2::new(x0, y0),
        Pos2::new(x1 - fold, y0),
        Pos2::new(x1, y0 + fold),
        Pos2::new(x1, y1),
        Pos2::new(x0, y1),
    ];
    painter.add(egui::Shape::convex_polygon(
        page,
        fade(paper(), alpha),
        PathStroke::new(1.0, fade(paper_edge(), alpha)),
    ));
    // Folded corner.
    let corner = vec![
        Pos2::new(x1 - fold, y0),
        Pos2::new(x1, y0 + fold),
        Pos2::new(x1 - fold, y0 + fold),
    ];
    painter.add(egui::Shape::convex_polygon(
        corner,
        fade(paper_shade(), alpha),
        PathStroke::NONE,
    ));

    let cx = (x0 + x1) * 0.5;
    let cy = (y0 + y1) * 0.5;
    let glyph = (w * 0.44).min(h * 0.40);
    let g = Rect::from_center_size(Pos2::new(cx, cy + h * 0.08), Vec2::splat(glyph));

    match kind {
        Kind::Image => {
            let green = fade(c(103, 168, 92), alpha);
            let hill = vec![
                Pos2::new(g.left(), g.bottom()),
                Pos2::new(g.left() + g.width() * 0.34, g.top() + g.height() * 0.42),
                Pos2::new(g.left() + g.width() * 0.58, g.bottom()),
            ];
            painter.add(egui::Shape::convex_polygon(hill, green, PathStroke::NONE));
            let hill2 = vec![
                Pos2::new(g.left() + g.width() * 0.38, g.bottom()),
                Pos2::new(g.left() + g.width() * 0.68, g.top() + g.height() * 0.58),
                Pos2::new(g.right(), g.bottom()),
            ];
            painter.add(egui::Shape::convex_polygon(
                hill2,
                fade(c(76, 140, 70), alpha),
                PathStroke::NONE,
            ));
            painter.circle_filled(
                Pos2::new(g.left() + g.width() * 0.24, g.top() + g.height() * 0.22),
                g.width() * 0.11,
                fade(c(240, 194, 84), alpha),
            );
        }
        Kind::Audio => {
            let orange = fade(c(219, 139, 61), alpha);
            painter.circle_filled(
                Pos2::new(g.left() + g.width() * 0.30, g.bottom() - g.height() * 0.12),
                g.width() * 0.15,
                orange,
            );
            painter.circle_filled(
                Pos2::new(g.right() - g.width() * 0.18, g.bottom() - g.height() * 0.30),
                g.width() * 0.15,
                orange,
            );
            let st = Stroke::new((g.width() * 0.11).max(1.0), orange);
            painter.line_segment(
                [
                    Pos2::new(g.left() + g.width() * 0.43, g.bottom() - g.height() * 0.14),
                    Pos2::new(g.left() + g.width() * 0.43, g.top() + g.height() * 0.22),
                ],
                st,
            );
            painter.line_segment(
                [
                    Pos2::new(g.right() - g.width() * 0.05, g.bottom() - g.height() * 0.32),
                    Pos2::new(g.right() - g.width() * 0.05, g.top() + g.height() * 0.06),
                ],
                st,
            );
            painter.line_segment(
                [
                    Pos2::new(g.left() + g.width() * 0.43, g.top() + g.height() * 0.22),
                    Pos2::new(g.right() - g.width() * 0.05, g.top() + g.height() * 0.06),
                ],
                Stroke::new((g.width() * 0.09).max(1.0), orange),
            );
        }
        Kind::Video => {
            let purple = fade(c(151, 116, 206), alpha);
            let tri = vec![
                Pos2::new(g.left() + g.width() * 0.18, g.top()),
                Pos2::new(g.right() - g.width() * 0.10, cy),
                Pos2::new(g.left() + g.width() * 0.18, g.bottom()),
            ];
            painter.add(egui::Shape::convex_polygon(tri, purple, PathStroke::NONE));
        }
        Kind::Archive => {
            let amber = fade(c(198, 158, 80), alpha);
            painter.rect_filled(
                Rect::from_min_max(
                    Pos2::new(g.left(), g.top() + g.height() * 0.18),
                    Pos2::new(g.right(), g.top() + g.height() * 0.42),
                ),
                CornerRadius::same(1),
                amber,
            );
            painter.rect_filled(
                Rect::from_min_max(
                    Pos2::new(cx - g.width() * 0.10, g.top() + g.height() * 0.50),
                    Pos2::new(cx + g.width() * 0.10, g.bottom()),
                ),
                CornerRadius::same(1),
                amber,
            );
        }
        Kind::Text => {
            let grey = fade(c(140, 148, 154), alpha);
            for i in 0..3 {
                let y = g.top() + g.height() * (0.12 + 0.26 * i as f32);
                painter.line_segment(
                    [
                        Pos2::new(g.left() + g.width() * 0.06, y),
                        Pos2::new(g.right() - g.width() * (0.06 + 0.22 * (i % 2) as f32), y),
                    ],
                    Stroke::new((h * 0.035).clamp(1.0, 2.0), grey),
                );
            }
        }
        Kind::Code => {
            let teal = fade(c(86, 170, 175), alpha);
            let st = Stroke::new((h * 0.045).clamp(1.2, 2.4), teal);
            painter.line_segment(
                [
                    Pos2::new(g.left() + g.width() * 0.34, g.top() + g.height() * 0.12),
                    Pos2::new(g.left(), g.top() + g.height() * 0.5),
                ],
                st,
            );
            painter.line_segment(
                [
                    Pos2::new(g.left(), g.top() + g.height() * 0.5),
                    Pos2::new(g.left() + g.width() * 0.34, g.bottom() - g.height() * 0.12),
                ],
                st,
            );
            painter.line_segment(
                [
                    Pos2::new(g.right() - g.width() * 0.34, g.top() + g.height() * 0.12),
                    Pos2::new(g.right(), g.top() + g.height() * 0.5),
                ],
                st,
            );
            painter.line_segment(
                [
                    Pos2::new(g.right(), g.top() + g.height() * 0.5),
                    Pos2::new(g.right() - g.width() * 0.34, g.bottom() - g.height() * 0.12),
                ],
                st,
            );
        }
        Kind::Pdf => {
            let red = fade(c(203, 84, 74), alpha);
            painter.rect_filled(
                Rect::from_min_max(
                    Pos2::new(g.left(), g.top() + g.height() * 0.40),
                    Pos2::new(g.right(), g.bottom()),
                ),
                CornerRadius::same(2),
                red,
            );
            painter.text(
                Pos2::new(cx, g.top() + g.height() * 0.70),
                Align2::CENTER_CENTER,
                "PDF",
                FontId::proportional((g.height() * 0.30).max(6.0)),
                fade(c(255, 255, 255), alpha),
            );
        }
        Kind::Exec => {
            let green = fade(c(96, 168, 104), alpha);
            painter.circle_filled(Pos2::new(cx, cy + h * 0.08), g.width() * 0.16, green);
            for i in 0..6 {
                let a = i as f32 * std::f32::consts::TAU / 6.0;
                let c = Pos2::new(cx, cy + h * 0.08);
                painter.line_segment(
                    [c, Pos2::new(c.x + a.cos() * g.width() * 0.34, c.y + a.sin() * g.width() * 0.34)],
                    Stroke::new((g.width() * 0.10).max(1.0), green),
                );
            }
        }
        Kind::Config => {
            let blue = fade(c(104, 156, 200), alpha);
            for i in 0..3 {
                let y = g.top() + g.height() * (0.22 + 0.28 * i as f32);
                painter.line_segment(
                    [
                        Pos2::new(g.left(), y),
                        Pos2::new(g.right(), y),
                    ],
                    Stroke::new((h * 0.032).max(1.0), blue),
                );
                let x = g.left() + g.width() * (0.30 + 0.22 * i as f32);
                painter.circle_filled(Pos2::new(x, y), (g.height() * 0.07).max(1.5), blue);
            }
        }
        Kind::Link => {
            let accent = fade(accent(), alpha);
            let stw = (h * 0.045).clamp(1.2, 2.4);
            painter.circle_stroke(
                Pos2::new(cx - g.width() * 0.16, cy + h * 0.06),
                g.width() * 0.22,
                Stroke::new(stw, accent),
            );
            painter.circle_stroke(
                Pos2::new(cx + g.width() * 0.16, cy + h * 0.06),
                g.width() * 0.22,
                Stroke::new(stw, accent),
            );
        }
        _ => {
            let grey = fade(c(150, 157, 163), alpha);
            for i in 0..3 {
                let y = g.top() + g.height() * (0.20 + 0.28 * i as f32);
                painter.line_segment(
                    [Pos2::new(g.left(), y), Pos2::new(g.right(), y)],
                    Stroke::new((h * 0.032).max(1.0), grey),
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Places icons (line art, monochrome like Breeze)
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PlaceIcon {
    Home,
    Desktop,
    Documents,
    Downloads,
    Music,
    Pictures,
    Videos,
    Trash,
    Disk,
    Bookmark,
    Recent,
    /// Added feature: a single file, for the rows of the Recent Files group.
    File,
    Network,
    Search,
    Tools,
}

pub fn draw_place_icon(painter: &egui::Painter, rect: Rect, icon: PlaceIcon, tint: Color32) {
    let r = rect.shrink(rect.width() * 0.14);
    let st = Stroke::new((r.width() * 0.085).max(1.1), tint);
    let (l, t, rr, b) = (r.left(), r.top(), r.right(), r.bottom());
    let cx = r.center().x;
    match icon {
        PlaceIcon::Home => {
            painter.add(egui::Shape::convex_polygon(
                vec![Pos2::new(cx, t), Pos2::new(rr, t + r.height() * 0.42), Pos2::new(l, t + r.height() * 0.42)],
                tint,
                PathStroke::NONE,
            ));
            painter.rect_stroke(
                Rect::from_min_max(
                    Pos2::new(l + r.width() * 0.16, t + r.height() * 0.40),
                    Pos2::new(rr - r.width() * 0.16, b),
                ),
                CornerRadius::same(1),
                st,
                egui::StrokeKind::Inside,
            );
        }
        PlaceIcon::Desktop => {
            painter.rect_stroke(
                Rect::from_min_max(Pos2::new(l, t + r.height() * 0.10), Pos2::new(rr, t + r.height() * 0.70)),
                CornerRadius::same(1),
                st,
                egui::StrokeKind::Inside,
            );
            painter.line_segment(
                [Pos2::new(cx, t + r.height() * 0.70), Pos2::new(cx, b)],
                st,
            );
            painter.line_segment(
                [Pos2::new(cx - r.width() * 0.22, b), Pos2::new(cx + r.width() * 0.22, b)],
                st,
            );
        }
        PlaceIcon::Documents => {
            painter.rect_stroke(
                Rect::from_min_max(Pos2::new(l + r.width() * 0.12, t), Pos2::new(rr - r.width() * 0.12, b)),
                CornerRadius::same(1),
                st,
                egui::StrokeKind::Inside,
            );
            for i in 0..3 {
                let y = t + r.height() * (0.28 + 0.20 * i as f32);
                painter.line_segment(
                    [Pos2::new(l + r.width() * 0.24, y), Pos2::new(rr - r.width() * 0.24, y)],
                    Stroke::new(st.width * 0.8, tint),
                );
            }
        }
        PlaceIcon::Downloads => {
            painter.line_segment([Pos2::new(cx, t), Pos2::new(cx, t + r.height() * 0.55)], st);
            painter.line_segment(
                [
                    Pos2::new(cx - r.width() * 0.24, t + r.height() * 0.34),
                    Pos2::new(cx, t + r.height() * 0.58),
                ],
                st,
            );
            painter.line_segment(
                [
                    Pos2::new(cx + r.width() * 0.24, t + r.height() * 0.34),
                    Pos2::new(cx, t + r.height() * 0.58),
                ],
                st,
            );
            painter.line_segment(
                [Pos2::new(l, b - r.height() * 0.10), Pos2::new(rr, b - r.height() * 0.10)],
                st,
            );
        }
        PlaceIcon::Music => {
            painter.circle_filled(Pos2::new(l + r.width() * 0.26, b - r.height() * 0.16), r.width() * 0.14, tint);
            painter.circle_filled(Pos2::new(rr - r.width() * 0.16, b - r.height() * 0.30), r.width() * 0.14, tint);
            painter.line_segment(
                [
                    Pos2::new(l + r.width() * 0.38, b - r.height() * 0.18),
                    Pos2::new(l + r.width() * 0.38, t + r.height() * 0.16),
                ],
                st,
            );
            painter.line_segment(
                [
                    Pos2::new(rr - r.width() * 0.04, b - r.height() * 0.32),
                    Pos2::new(rr - r.width() * 0.04, t),
                ],
                st,
            );
            painter.line_segment(
                [
                    Pos2::new(l + r.width() * 0.38, t + r.height() * 0.16),
                    Pos2::new(rr - r.width() * 0.04, t),
                ],
                st,
            );
        }
        PlaceIcon::Pictures => {
            painter.rect_stroke(
                Rect::from_min_max(Pos2::new(l, t + r.height() * 0.08), Pos2::new(rr, b)),
                CornerRadius::same(1),
                st,
                egui::StrokeKind::Inside,
            );
            painter.add(egui::Shape::convex_polygon(
                vec![
                    Pos2::new(l + r.width() * 0.10, b - r.height() * 0.10),
                    Pos2::new(l + r.width() * 0.38, t + r.height() * 0.42),
                    Pos2::new(l + r.width() * 0.62, b - r.height() * 0.10),
                ],
                tint,
                PathStroke::NONE,
            ));
            painter.circle_filled(
                Pos2::new(rr - r.width() * 0.20, t + r.height() * 0.30),
                r.width() * 0.08,
                tint,
            );
        }
        PlaceIcon::Videos => {
            painter.rect_stroke(
                Rect::from_min_max(Pos2::new(l, t + r.height() * 0.12), Pos2::new(rr, b - r.height() * 0.12)),
                CornerRadius::same(1),
                st,
                egui::StrokeKind::Inside,
            );
            painter.add(egui::Shape::convex_polygon(
                vec![
                    Pos2::new(cx - r.width() * 0.14, t + r.height() * 0.34),
                    Pos2::new(cx + r.width() * 0.20, t + r.height() * 0.50),
                    Pos2::new(cx - r.width() * 0.14, t + r.height() * 0.66),
                ],
                tint,
                PathStroke::NONE,
            ));
        }
        PlaceIcon::Trash => {
            painter.rect_stroke(
                Rect::from_min_max(Pos2::new(l + r.width() * 0.14, t + r.height() * 0.22), Pos2::new(rr - r.width() * 0.14, b)),
                CornerRadius::same(1),
                st,
                egui::StrokeKind::Inside,
            );
            painter.line_segment(
                [Pos2::new(l, t + r.height() * 0.20), Pos2::new(rr, t + r.height() * 0.20)],
                st,
            );
            painter.line_segment(
                [Pos2::new(cx - r.width() * 0.14, t + r.height() * 0.20), Pos2::new(cx - r.width() * 0.14, t + r.height() * 0.06)],
                st,
            );
            painter.line_segment(
                [Pos2::new(cx + r.width() * 0.14, t + r.height() * 0.20), Pos2::new(cx + r.width() * 0.14, t + r.height() * 0.06)],
                st,
            );
            painter.line_segment(
                [Pos2::new(cx - r.width() * 0.14, t + r.height() * 0.06), Pos2::new(cx + r.width() * 0.14, t + r.height() * 0.06)],
                st,
            );
        }
        PlaceIcon::Disk => {
            painter.rect_stroke(
                Rect::from_min_max(Pos2::new(l, t + r.height() * 0.24), Pos2::new(rr, b - r.height() * 0.22)),
                CornerRadius::same(2),
                st,
                egui::StrokeKind::Inside,
            );
            painter.circle_filled(Pos2::new(rr - r.width() * 0.18, b - r.height() * 0.34), r.width() * 0.05, tint);
            painter.line_segment(
                [Pos2::new(l + r.width() * 0.12, t + r.height() * 0.34), Pos2::new(rr - r.width() * 0.36, t + r.height() * 0.34)],
                Stroke::new(st.width * 0.8, tint),
            );
        }
        PlaceIcon::Bookmark => {
            painter.add(egui::Shape::convex_polygon(
                vec![
                    Pos2::new(l + r.width() * 0.20, t),
                    Pos2::new(rr - r.width() * 0.20, t),
                    Pos2::new(rr - r.width() * 0.20, b),
                    Pos2::new(cx, b - r.height() * 0.24),
                    Pos2::new(l + r.width() * 0.20, b),
                ],
                tint,
                PathStroke::NONE,
            ));
        }
        PlaceIcon::Recent => {
            painter.circle_stroke(r.center(), r.width() * 0.42, st);
            painter.line_segment([r.center(), Pos2::new(cx, t + r.height() * 0.16)], st);
            painter.line_segment([r.center(), Pos2::new(cx + r.width() * 0.26, r.center().y)], st);
        }
        PlaceIcon::File => {
            // A page with its top-right corner turned down, drawn in outline.
            let fold = r.width() * 0.26;
            let page = vec![
                Pos2::new(l + r.width() * 0.16, t),
                Pos2::new(rr - fold, t),
                Pos2::new(rr - r.width() * 0.10, t + fold),
                Pos2::new(rr - r.width() * 0.10, b),
                Pos2::new(l + r.width() * 0.16, b),
            ];
            painter.add(egui::Shape::closed_line(page, st));
            painter.line_segment(
                [
                    Pos2::new(rr - fold, t),
                    Pos2::new(rr - fold, t + fold),
                ],
                Stroke::new(st.width * 0.8, tint),
            );
            painter.line_segment(
                [
                    Pos2::new(rr - fold, t + fold),
                    Pos2::new(rr - r.width() * 0.10, t + fold),
                ],
                Stroke::new(st.width * 0.8, tint),
            );
        }
        PlaceIcon::Network => {
            painter.circle_stroke(r.center(), r.width() * 0.42, st);
            painter.line_segment([Pos2::new(l, r.center().y), Pos2::new(rr, r.center().y)], Stroke::new(st.width * 0.8, tint));
            painter.line_segment(
                [
                    Pos2::new(cx - r.width() * 0.16, t + r.height() * 0.10),
                    Pos2::new(cx - r.width() * 0.16, b - r.height() * 0.10),
                ],
                Stroke::new(st.width * 0.8, tint),
            );
            painter.line_segment(
                [
                    Pos2::new(cx + r.width() * 0.16, t + r.height() * 0.10),
                    Pos2::new(cx + r.width() * 0.16, b - r.height() * 0.10),
                ],
                Stroke::new(st.width * 0.8, tint),
            );
        }
        PlaceIcon::Tools => {
            // Small wrench: a bar with a ring at one end.
            painter.line_segment(
                [Pos2::new(l + r.width() * 0.15, b - r.height() * 0.15), Pos2::new(rr - r.width() * 0.20, t + r.height() * 0.22)],
                st,
            );
            painter.circle_stroke(
                Pos2::new(l + r.width() * 0.20, b - r.height() * 0.20),
                r.width() * 0.18,
                st,
            );
            painter.circle_filled(
                Pos2::new(rr - r.width() * 0.16, t + r.height() * 0.18),
                r.width() * 0.12,
                tint,
            );
        }
        PlaceIcon::Search => {
            painter.circle_stroke(
                Pos2::new(cx - r.width() * 0.08, r.center().y - r.height() * 0.08),
                r.width() * 0.30,
                st,
            );
            painter.line_segment(
                [
                    Pos2::new(cx + r.width() * 0.14, r.center().y + r.height() * 0.14),
                    Pos2::new(rr, b),
                ],
                st,
            );
        }
    }
}

/// What the person picked in a colour menu.
pub enum ColourPick {
    Set([u8; 3]),
    Clear,
}

/// The eight colours the folder menu offers, drawn as swatches, plus "Normal
/// Colour" to go back to the usual look. Shared by the folder menu in the file
/// view and the shortcut rows in the panel, so both offer the same choices.
///
/// The palette itself is `foldercolor::PALETTE`; the folder menu's swatch size
/// (26 px, 6 px apart, a 2 px white ring on the colour in use) is kept here so
/// the two menus cannot drift apart.
pub fn colour_menu(ui: &mut egui::Ui, current: Option<[u8; 3]>) -> Option<ColourPick> {
    let mut picked = None;
    for row in crate::foldercolor::PALETTE.chunks(4) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            for sw in row {
                let rgb = c(sw.rgb[0], sw.rgb[1], sw.rgb[2]);
                let (rect, resp) = ui.allocate_exact_size(Vec2::splat(26.0), egui::Sense::click());
                let painter = ui.painter();
                painter.rect_filled(rect.shrink(2.0), CornerRadius::same(4), rgb);
                let chosen = current == Some(sw.rgb);
                painter.rect_stroke(
                    rect.shrink(1.0),
                    CornerRadius::same(5),
                    Stroke::new(
                        if chosen { 2.0 } else { 1.0 },
                        if chosen {
                            c(255, 255, 255)
                        } else {
                            c(24, 27, 30)
                        },
                    ),
                    egui::StrokeKind::Inside,
                );
                if resp.on_hover_text(sw.name).clicked() {
                    picked = Some(ColourPick::Set(sw.rgb));
                }
            }
        });
    }
    ui.separator();
    if ui.button("Normal Colour").clicked() {
        picked = Some(ColourPick::Clear);
    }
    picked
}

/// Small toolbar glyphs drawn with lines: back, forward, up, refresh, etc.
pub fn draw_glyph(painter: &egui::Painter, rect: Rect, glyph: &str, tint: Color32) {
    let c = rect.center();
    let s = rect.width().min(rect.height()) * 0.26;
    let st = Stroke::new(1.7, tint);
    match glyph {
        "back" => {
            painter.line_segment([Pos2::new(c.x + s * 0.5, c.y - s), Pos2::new(c.x - s * 0.5, c.y)], st);
            painter.line_segment([Pos2::new(c.x - s * 0.5, c.y), Pos2::new(c.x + s * 0.5, c.y + s)], st);
        }
        "forward" => {
            painter.line_segment([Pos2::new(c.x - s * 0.5, c.y - s), Pos2::new(c.x + s * 0.5, c.y)], st);
            painter.line_segment([Pos2::new(c.x + s * 0.5, c.y), Pos2::new(c.x - s * 0.5, c.y + s)], st);
        }
        "up" => {
            painter.line_segment([Pos2::new(c.x - s, c.y + s * 0.5), Pos2::new(c.x, c.y - s * 0.5)], st);
            painter.line_segment([Pos2::new(c.x, c.y - s * 0.5), Pos2::new(c.x + s, c.y + s * 0.5)], st);
        }
        "refresh" => {
            let r = s * 1.1;
            let mut pts = Vec::new();
            for i in 0..=24 {
                let a = -0.6 + i as f32 * (std::f32::consts::TAU * 0.82) / 24.0;
                pts.push(Pos2::new(c.x + a.cos() * r, c.y + a.sin() * r));
            }
            painter.add(egui::Shape::line(pts, st));
            let tip = Pos2::new(c.x + 0.36 * r, c.y - 0.95 * r);
            painter.add(egui::Shape::convex_polygon(
                vec![
                    tip,
                    Pos2::new(tip.x - s * 0.55, tip.y + s * 0.15),
                    Pos2::new(tip.x + s * 0.15, tip.y + s * 0.6),
                ],
                tint,
                PathStroke::NONE,
            ));
        }
        "close" => {
            painter.line_segment([Pos2::new(c.x - s * 0.6, c.y - s * 0.6), Pos2::new(c.x + s * 0.6, c.y + s * 0.6)], st);
            painter.line_segment([Pos2::new(c.x + s * 0.6, c.y - s * 0.6), Pos2::new(c.x - s * 0.6, c.y + s * 0.6)], st);
        }
        "plus" => {
            painter.line_segment([Pos2::new(c.x - s * 0.7, c.y), Pos2::new(c.x + s * 0.7, c.y)], st);
            painter.line_segment([Pos2::new(c.x, c.y - s * 0.7), Pos2::new(c.x, c.y + s * 0.7)], st);
        }
        "menu" => {
            for i in -1..=1 {
                let y = c.y + i as f32 * s * 0.55;
                painter.line_segment([Pos2::new(c.x - s, y), Pos2::new(c.x + s, y)], st);
            }
        }
        "dropdown" => {
            painter.add(egui::Shape::convex_polygon(
                vec![
                    Pos2::new(c.x - s * 0.6, c.y - s * 0.28),
                    Pos2::new(c.x + s * 0.6, c.y - s * 0.28),
                    Pos2::new(c.x, c.y + s * 0.45),
                ],
                tint,
                PathStroke::NONE,
            ));
        }
        // Sort: three lines of decreasing length with an arrow pointing down.
        "sort" => {
            for (i, w) in [1.0f32, 0.7, 0.4].iter().enumerate() {
                let y = c.y - s * 0.55 + i as f32 * s * 0.55;
                painter.line_segment(
                    [Pos2::new(c.x - s, y), Pos2::new(c.x - s + s * 2.0 * w, y)],
                    st,
                );
            }
            painter.line_segment(
                [Pos2::new(c.x + s * 0.55, c.y - s * 0.55), Pos2::new(c.x + s * 0.55, c.y + s * 0.5)],
                st,
            );
            painter.add(egui::Shape::convex_polygon(
                vec![
                    Pos2::new(c.x + s * 0.15, c.y + s * 0.1),
                    Pos2::new(c.x + s * 0.95, c.y + s * 0.1),
                    Pos2::new(c.x + s * 0.55, c.y + s * 0.6),
                ],
                tint,
                PathStroke::NONE,
            ));
        }
        // Open in: a box with an arrow leaving its top-right corner.
        "openin" => {
            let h = s * 0.95;
            let box_ = [
                Pos2::new(c.x - h, c.y - h * 0.2),
                Pos2::new(c.x - h, c.y + h),
                Pos2::new(c.x + h * 0.4, c.y + h),
                Pos2::new(c.x + h * 0.4, c.y + h * 0.15),
            ];
            for i in 0..3 {
                painter.line_segment([box_[i], box_[i + 1]], st);
            }
            painter.line_segment(
                [Pos2::new(c.x - h * 0.1, c.y + h * 0.1), Pos2::new(c.x + h * 0.95, c.y - h * 0.95)],
                st,
            );
            painter.line_segment(
                [Pos2::new(c.x + h * 0.3, c.y - h * 0.95), Pos2::new(c.x + h * 0.95, c.y - h * 0.95)],
                st,
            );
            painter.line_segment(
                [Pos2::new(c.x + h * 0.95, c.y - h * 0.95), Pos2::new(c.x + h * 0.95, c.y - h * 0.3)],
                st,
            );
        }
        "split" => {
            painter.rect_stroke(
                Rect::from_center_size(c, Vec2::splat(s * 2.2)),
                CornerRadius::same(2),
                st,
                egui::StrokeKind::Inside,
            );
            painter.line_segment([Pos2::new(c.x, c.y - s * 1.1), Pos2::new(c.x, c.y + s * 1.1)], st);
        }
        "search" => {
            painter.circle_stroke(Pos2::new(c.x - s * 0.2, c.y - s * 0.2), s * 0.75, st);
            painter.line_segment(
                [Pos2::new(c.x + s * 0.38, c.y + s * 0.38), Pos2::new(c.x + s, c.y + s)],
                st,
            );
        }
        "filter" => {
            painter.add(egui::Shape::convex_polygon(
                vec![
                    Pos2::new(c.x - s, c.y - s * 0.7),
                    Pos2::new(c.x + s, c.y - s * 0.7),
                    Pos2::new(c.x + s * 0.22, c.y + s * 0.1),
                    Pos2::new(c.x + s * 0.22, c.y + s * 0.9),
                    Pos2::new(c.x - s * 0.22, c.y + s * 0.6),
                    Pos2::new(c.x - s * 0.22, c.y + s * 0.1),
                ],
                tint,
                PathStroke::NONE,
            ));
        }
        "info" => {
            painter.circle_stroke(c, s * 0.95, st);
            painter.line_segment([Pos2::new(c.x, c.y - s * 0.1), Pos2::new(c.x, c.y + s * 0.55)], st);
            painter.circle_filled(Pos2::new(c.x, c.y - s * 0.5), st.width * 0.55, tint);
        }
        "grid" => {
            for dx in [-1.0f32, 1.0] {
                for dy in [-1.0f32, 1.0] {
                    painter.rect_filled(
                        Rect::from_center_size(
                            Pos2::new(c.x + dx * s * 0.5, c.y + dy * s * 0.5),
                            Vec2::splat(s * 0.7),
                        ),
                        CornerRadius::same(1),
                        tint,
                    );
                }
            }
        }
        "list" => {
            for i in -1..=1 {
                let y = c.y + i as f32 * s * 0.55;
                painter.circle_filled(Pos2::new(c.x - s * 0.7, y), s * 0.13, tint);
                painter.line_segment([Pos2::new(c.x - s * 0.35, y), Pos2::new(c.x + s, y)], st);
            }
        }
        "details" => {
            painter.line_segment([Pos2::new(c.x - s, c.y - s * 0.6), Pos2::new(c.x + s, c.y - s * 0.6)], st);
            painter.line_segment([Pos2::new(c.x - s, c.y), Pos2::new(c.x + s, c.y)], Stroke::new(1.2, tint));
            painter.line_segment([Pos2::new(c.x - s, c.y + s * 0.6), Pos2::new(c.x + s, c.y + s * 0.6)], Stroke::new(1.2, tint));
        }
        "chevron_down" => {
            painter.line_segment(
                [Pos2::new(c.x - s * 0.6, c.y - s * 0.3), Pos2::new(c.x, c.y + s * 0.4)],
                st,
            );
            painter.line_segment(
                [Pos2::new(c.x, c.y + s * 0.4), Pos2::new(c.x + s * 0.6, c.y - s * 0.3)],
                st,
            );
        }
        "chevron_right" => {
            painter.line_segment(
                [Pos2::new(c.x - s * 0.3, c.y - s * 0.6), Pos2::new(c.x + s * 0.4, c.y)],
                st,
            );
            painter.line_segment(
                [Pos2::new(c.x + s * 0.4, c.y), Pos2::new(c.x - s * 0.3, c.y + s * 0.6)],
                st,
            );
        }
        "bookmark_add" => {
            painter.add(egui::Shape::convex_polygon(
                vec![
                    Pos2::new(c.x - s * 0.6, c.y - s),
                    Pos2::new(c.x + s * 0.6, c.y - s),
                    Pos2::new(c.x + s * 0.6, c.y + s),
                    Pos2::new(c.x, c.y + s * 0.5),
                    Pos2::new(c.x - s * 0.6, c.y + s),
                ],
                tint,
                PathStroke::NONE,
            ));
        }
        // Added feature — the gallery's own small marks: a picture frame, a
        // minus sign, fit-to-window corners, play/pause, and the compare mark.
        "pictures" => {
            let frame = Rect::from_center_size(c, Vec2::splat(s * 1.9));
            painter.rect_stroke(frame, CornerRadius::same(2), Stroke::new(1.5, tint), egui::StrokeKind::Inside);
            // A little horizon and a sun, so the frame reads as a photograph.
            painter.add(egui::Shape::convex_polygon(
                vec![
                    Pos2::new(frame.left() + s * 0.25, frame.bottom() - s * 0.25),
                    Pos2::new(frame.left() + s * 0.85, frame.center().y + s * 0.1),
                    Pos2::new(frame.right() - s * 0.25, frame.bottom() - s * 0.25),
                ],
                tint,
                PathStroke::NONE,
            ));
            painter.circle_filled(
                Pos2::new(frame.right() - s * 0.55, frame.top() + s * 0.55),
                s * 0.2,
                tint,
            );
        }
        "minus" => {
            painter.line_segment(
                [Pos2::new(c.x - s * 0.7, c.y), Pos2::new(c.x + s * 0.7, c.y)],
                st,
            );
        }
        "fit" => {
            let b = Rect::from_center_size(c, Vec2::splat(s * 1.7));
            let arm = s * 0.55;
            for (corner, dx, dy) in [
                (Pos2::new(b.left(), b.top()), 1.0f32, 1.0f32),
                (Pos2::new(b.right(), b.top()), -1.0, 1.0),
                (Pos2::new(b.left(), b.bottom()), 1.0, -1.0),
                (Pos2::new(b.right(), b.bottom()), -1.0, -1.0),
            ] {
                painter.line_segment([corner, Pos2::new(corner.x + arm * dx, corner.y)], st);
                painter.line_segment([corner, Pos2::new(corner.x, corner.y + arm * dy)], st);
            }
        }
        "play" => {
            painter.add(egui::Shape::convex_polygon(
                vec![
                    Pos2::new(c.x - s * 0.5, c.y - s * 0.85),
                    Pos2::new(c.x + s * 0.85, c.y),
                    Pos2::new(c.x - s * 0.5, c.y + s * 0.85),
                ],
                tint,
                PathStroke::NONE,
            ));
        }
        "pause" => {
            for dx in [-0.35f32, 0.35] {
                painter.rect_filled(
                    Rect::from_center_size(
                        Pos2::new(c.x + s * dx, c.y),
                        Vec2::new(s * 0.4, s * 1.7),
                    ),
                    CornerRadius::same(1),
                    tint,
                );
            }
        }
        "compare" => {
            for (dx, dy) in [(-0.25f32, -0.25f32), (0.25, 0.25)] {
                painter.rect_stroke(
                    Rect::from_center_size(
                        Pos2::new(c.x + s * dx, c.y + s * dy),
                        Vec2::splat(s * 1.25),
                    ),
                    CornerRadius::same(1),
                    Stroke::new(1.4, tint),
                    egui::StrokeKind::Inside,
                );
            }
        }
        "chevron_left" => {
            painter.line_segment(
                [Pos2::new(c.x + s * 0.3, c.y - s * 0.6), Pos2::new(c.x - s * 0.4, c.y)],
                st,
            );
            painter.line_segment(
                [Pos2::new(c.x - s * 0.4, c.y), Pos2::new(c.x + s * 0.3, c.y + s * 0.6)],
                st,
            );
        }
        "swap" => {
            painter.line_segment(
                [Pos2::new(c.x - s, c.y - s * 0.35), Pos2::new(c.x + s * 0.8, c.y - s * 0.35)],
                st,
            );
            painter.line_segment(
                [Pos2::new(c.x - s, c.y + s * 0.35), Pos2::new(c.x + s * 0.8, c.y + s * 0.35)],
                st,
            );
            painter.line_segment(
                [Pos2::new(c.x + s * 0.8, c.y - s * 0.35), Pos2::new(c.x + s * 0.4, c.y - s * 0.7)],
                st,
            );
            painter.line_segment(
                [Pos2::new(c.x - s * 0.8, c.y + s * 0.35), Pos2::new(c.x - s * 0.4, c.y + s * 0.7)],
                st,
            );
        }
        _ => {}
    }
}

/// Put the whole text of a single-line editor in the selection, the way
/// Dolphin preselects the suggested name of a new folder or a rename.
pub fn select_all_text(ctx: &egui::Context, id: egui::Id, len_chars: usize) {
    if let Some(mut state) = egui::TextEdit::load_state(ctx, id) {
        state.cursor.set_char_range(Some(egui::text::CCursorRange::two(
            egui::text::CCursor::new(0),
            egui::text::CCursor::new(len_chars),
        )));
        state.store(ctx, id);
    }
}
