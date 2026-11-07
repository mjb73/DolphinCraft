//! Small JSON config: bookmarks and the last window state.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Bookmark {
    pub name: String,
    pub path: PathBuf,
}

/// Added feature: the colour the person gave one row of the Shortcuts panel.
/// The original has no place colours at all, so this is stored next to the
/// shortcuts in the rebuild's own config, keyed by the folder the row stands
/// for. Folder colours go into the folder's `.directory` file instead, because
/// there they can travel with the folder; a panel row has no such file.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ShortcutColor {
    pub path: PathBuf,
    pub rgb: [u8; 3],
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Config {
    #[serde(default)]
    pub bookmarks: Vec<Bookmark>,
    #[serde(default)]
    pub last_dir: String,
    #[serde(default)]
    pub split: bool,
    #[serde(default = "yes")]
    pub places_visible: bool,
    #[serde(default)]
    pub info_panel: bool,
    #[serde(default)]
    pub show_hidden: bool,
    #[serde(default = "yes")]
    pub previews: bool,
    #[serde(default = "details")]
    pub view: usize,
    #[serde(default = "default_zoom")]
    pub zoom: [u8; 3],
    #[serde(default = "yes")]
    pub folders_first: bool,
    /// Which name comparison the Sort menu uses, as the index of the three rows
    /// the original's Sort page offers (`FUN_001e5750` @0x1e5750):
    /// 0 Natural, 1 Alphabetical case insensitive, 2 Alphabetical case sensitive.
    #[serde(default)]
    pub name_sort: usize,
    /// The Shortcuts section of the navigation panel can be folded away.
    #[serde(default = "yes")]
    pub shortcuts_expanded: bool,
    /// Show the keyboard hint next to each shortcut row.
    #[serde(default = "yes")]
    pub shortcut_keys: bool,
    /// Colours the person gave to rows of the Shortcuts panel, keyed by folder.
    #[serde(default)]
    pub shortcut_colors: Vec<ShortcutColor>,
    /// Added feature: the picture gallery was the last presentation used, and
    /// how big its thumbnails were, whether names and dimension captions were
    /// shown. The gallery's own size range is 50..400 px.
    #[serde(default)]
    pub pictures: bool,
    #[serde(default = "gallery_thumb")]
    pub thumb_px: f32,
    #[serde(default = "yes")]
    pub gallery_names: bool,
    #[serde(default)]
    pub gallery_dims: bool,
    /// Rounded corners on the gallery's thumbnails.
    #[serde(default = "yes")]
    pub gallery_round: bool,
    /// Slideshow interval in seconds and whether pictures cross-fade. Kept
    /// between runs so the show starts the way the person left it.
    #[serde(default = "slide_secs")]
    pub slide_secs: f32,
    #[serde(default = "yes")]
    pub slide_fade: bool,
}

fn slide_secs() -> f32 {
    crate::gallery::SLIDE_DEFAULT
}

fn gallery_thumb() -> f32 {
    crate::gallery::THUMB_DEFAULT
}

fn yes() -> bool {
    true
}

fn details() -> usize {
    2
}

fn default_zoom() -> [u8; 3] {
    // A config file that predates the `zoom` key must land on the same default
    // as a fresh profile: `DolphinView::updateDefaultZoomLevel` @0x27df20 asks
    // each mode's settings object for `PreviewSize` while previews are shown
    // (Icons 64, Compact 48, Details 48) and for `IconSize` when they are off
    // (Icons 32, Compact 16, Details 16), then turns that size into a zoom level
    // with `ZoomLevelInfo::zoomLevelForIconSize` @0x2a6560 → levels 4, 3, 3.
    crate::theme::default_zoom(true)
}

impl Default for Config {
    fn default() -> Self {
        Self {
            bookmarks: Vec::new(),
            last_dir: String::new(),
            split: false,
            places_visible: true,
            info_panel: false,
            show_hidden: false,
            previews: true,
            // The original opens a fresh window in the icons view, at the zoom
            // whose size matches the mode's PreviewSize (DolphinView::
            // updateDefaultZoomLevel @0x27df20). Measured on a fresh profile:
            // icons view, 64 px tiles.
            view: 0,
            zoom: crate::theme::default_zoom(true),
            folders_first: true,
            name_sort: 0,
            shortcuts_expanded: true,
            shortcut_keys: true,
            shortcut_colors: Vec::new(),
            pictures: false,
            thumb_px: crate::gallery::THUMB_DEFAULT,
            gallery_names: true,
            gallery_dims: false,
            gallery_round: true,
            slide_secs: crate::gallery::SLIDE_DEFAULT,
            slide_fade: true,
        }
    }
}

pub fn config_path() -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| crate::fsutil::home_dir().join(".config"));
    base.join("dolphin-remake").join("config.json")
}

impl Config {
    pub fn load() -> Self {
        let path = config_path();
        let Ok(text) = std::fs::read_to_string(path) else {
            return Self::default();
        };
        serde_json::from_str(&text).unwrap_or_default()
    }

    pub fn save(&self) {
        let path = config_path();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(text) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(path, text);
        }
    }

    /// The colour of one shortcut row, if the person chose one.
    pub fn shortcut_color(&self, path: &Path) -> Option<[u8; 3]> {
        self.shortcut_colors
            .iter()
            .find(|c| c.path == path)
            .map(|c| c.rgb)
    }

    /// Store a colour for one shortcut row; `None` puts the row back to the
    /// normal place colour. The entry is removed rather than kept as a
    /// "no colour" record, so the file stays as small as the choices made.
    pub fn set_shortcut_color(&mut self, path: &Path, rgb: Option<[u8; 3]>) {
        self.shortcut_colors.retain(|c| c.path != path);
        if let Some(rgb) = rgb {
            self.shortcut_colors.push(ShortcutColor {
                path: path.to_path_buf(),
                rgb,
            });
        }
    }
}

/// Move one shortcut to an insertion slot in the list. `to` counts the gaps
/// between rows: 0 is above the first row, `list.len()` is below the last, and
/// `to` between two rows puts the dragged one in that gap.
///
/// This is the rule `KFilePlacesModel::movePlace(int from, int to)` @0x96220 in
/// libKF6KIOFileWidgets.so.6 follows: a slot equal to `from` or `from + 1` means
/// "it is already there" and the function returns without touching anything;
/// otherwise the dragged row lands at `to - 1` when it moved down and at `to`
/// when it moved up. Returns whether the order really changed.
pub fn move_shortcut(list: &mut Vec<Bookmark>, from: usize, to: usize) -> bool {
    if from >= list.len() {
        return false;
    }
    let to = to.min(list.len());
    if to == from || to == from + 1 {
        return false;
    }
    let item = list.remove(from);
    let target = if to > from { to - 1 } else { to };
    list.insert(target.min(list.len()), item);
    true
}
