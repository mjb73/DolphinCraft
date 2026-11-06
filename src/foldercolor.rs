//! Added feature: a colour of the person's choosing for any folder.
//!
//! The original has no colour picker. What it does have is a place for each
//! folder to keep its own settings, described in its own words (string at
//! 0x136e48 of /usr/bin/dolphin): "Dolphin will add file system metadata to
//! folders you change view properties for. If that is not possible, a hidden
//! .directory file is created instead." ViewProperties::save (0x1a8c90 in
//! libdolphinprivate.so.6) writes that file: it creates the folder's
//! .directory when the file system has no metadata support, stores the
//! settings under a group, skips any key whose value equals the default, and
//! ignores the reserved Timestamp and Version keys. The keys it stores there
//! are visible in the same library: IconSize, ShowViewMode, SortRole,
//! SortFoldersFirst, SortHiddenLast, PreviewsShown, ShowHiddenFiles,
//! GroupedSorting, GroupRole, HeaderColumnWidths, VisibleRoles.
//!
//! The colour rides in the same file, in the same group, under FolderColor,
//! so a folder that already carries view settings keeps them.

use std::fs;
use std::path::{Path, PathBuf};

pub struct Swatch {
    pub name: &'static str,
    pub rgb: [u8; 3],
}

/// The colours offered. The first one matches the folder colour the rebuild
/// already draws, so choosing it is the same as leaving the folder alone.
pub const PALETTE: [Swatch; 8] = [
    Swatch { name: "Blue", rgb: [61, 145, 200] },
    Swatch { name: "Teal", rgb: [45, 146, 138] },
    Swatch { name: "Green", rgb: [86, 148, 78] },
    Swatch { name: "Yellow", rgb: [201, 166, 60] },
    Swatch { name: "Orange", rgb: [207, 118, 49] },
    Swatch { name: "Red", rgb: [196, 72, 66] },
    Swatch { name: "Purple", rgb: [124, 92, 168] },
    Swatch { name: "Grey", rgb: [124, 131, 140] },
];

pub const FILE: &str = ".directory";
pub const KEY: &str = "FolderColor";
const GROUP: &str = "[Settings]";

pub fn file_for(folder: &Path) -> PathBuf {
    folder.join(FILE)
}

/// The colour stored for a folder, if any.
pub fn read(folder: &Path) -> Option<[u8; 3]> {
    let text = fs::read_to_string(file_for(folder)).ok()?;
    parse_colour(&text)
}

/// The palette entry a colour belongs to, for naming it in a message.
pub fn name_of(rgb: [u8; 3]) -> Option<&'static str> {
    PALETTE
        .iter()
        .find(|s| s.rgb == rgb)
        .map(|s| s.name)
}

/// Store (or clear, with `None`) a folder's colour.
pub fn write(folder: &Path, rgb: Option<[u8; 3]>) -> Result<(), String> {
    let path = file_for(folder);
    if std::env::var_os("FC_DEBUG").is_some() {
        eprintln!("foldercolor::write {:?} -> {:?}", path, rgb);
    }
    let original = fs::read_to_string(&path).unwrap_or_default();
    match rewrite(&original, rgb) {
        Some(text) => {
            let tmp = folder.join(".directory.tmp");
            fs::write(&tmp, text.as_bytes())
                .map_err(|e| format!("The colour could not be saved: {e}"))?;
            fs::rename(&tmp, &path).map_err(|e| {
                let _ = fs::remove_file(&tmp);
                format!("The colour could not be saved: {e}")
            })?;
        }
        None => {
            // Nothing else is kept in the file, so it goes away again; the
            // original does the same ("cleaning .directory", 0x1bffe0).
            if path.exists() {
                fs::remove_file(&path)
                    .map_err(|e| format!("The colour could not be cleared: {e}"))?;
            }
        }
    }
    Ok(())
}

fn parse_colour(text: &str) -> Option<[u8; 3]> {
    let mut in_group = false;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') {
            in_group = line.eq_ignore_ascii_case(GROUP);
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            if in_group && k.trim().eq_ignore_ascii_case(KEY) {
                return parse_hex(v.trim());
            }
        }
    }
    None
}

fn parse_hex(s: &str) -> Option<[u8; 3]> {
    let s = s.trim().trim_start_matches('#');
    if s.len() != 6 {
        return None;
    }
    let r = u8::from_str_radix(&s[0..2], 16).ok()?;
    let g = u8::from_str_radix(&s[2..4], 16).ok()?;
    let b = u8::from_str_radix(&s[4..6], 16).ok()?;
    Some([r, g, b])
}

fn hex(rgb: [u8; 3]) -> String {
    format!("#{:02x}{:02x}{:02x}", rgb[0], rgb[1], rgb[2])
}

/// Rewrite the file's text with the colour set or removed. `None` means the
/// file would hold nothing else, so it should not exist at all.
fn rewrite(original: &str, rgb: Option<[u8; 3]>) -> Option<String> {
    let mut lines: Vec<String> = original.lines().map(|l| l.to_string()).collect();

    let mut group_at: Option<usize> = None;
    let mut key_at: Option<usize> = None;
    let mut in_group = false;
    for (i, line) in lines.iter().enumerate() {
        let t = line.trim();
        if t.starts_with('[') {
            in_group = t.eq_ignore_ascii_case(GROUP);
            if in_group {
                group_at = Some(i);
            }
            continue;
        }
        if in_group {
            if let Some((k, _)) = t.split_once('=') {
                if k.trim().eq_ignore_ascii_case(KEY) {
                    key_at = Some(i);
                }
            }
        }
    }

    match rgb {
        Some(rgb) => {
            let entry = format!("{KEY}={}", hex(rgb));
            match key_at {
                Some(i) => lines[i] = entry,
                None => match group_at {
                    Some(h) => lines.insert(h + 1, entry),
                    None => {
                        if lines.iter().any(|l| !l.trim().is_empty()) && !ends_clean(&lines) {
                            lines.push(String::new());
                        }
                        lines.push(GROUP.to_string());
                        lines.push(entry);
                    }
                },
            }
            Some(finish(lines))
        }
        None => {
            if let Some(i) = key_at {
                lines.remove(i);
            }
            if holds_nothing(&lines) {
                None
            } else {
                Some(finish(lines))
            }
        }
    }
}

fn ends_clean(lines: &[String]) -> bool {
    lines
        .last()
        .map(|l| l.trim().is_empty())
        .unwrap_or(true)
}

/// True when the text keeps no settings at all: only blank lines, comments
/// and section headers are left.
fn holds_nothing(lines: &[String]) -> bool {
    !lines.iter().any(|l| {
        let t = l.trim();
        !t.is_empty() && !t.starts_with('[') && !t.starts_with(';')
    })
}

fn finish(mut lines: Vec<String>) -> String {
    while lines.last().map(|l| l.trim().is_empty()).unwrap_or(false) {
        lines.pop();
    }
    let mut out = lines.join("\n");
    out.push('\n');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_into_a_new_file_and_reads_back() {
        let text = rewrite("", Some([196, 72, 66])).unwrap();
        assert_eq!(text, "[Settings]\nFolderColor=#c44842\n");
        assert_eq!(parse_colour(&text), Some([196, 72, 66]));
    }

    #[test]
    fn keeps_the_other_view_settings() {
        let original = "[Settings]\nViewMode=1\nIconSize=48\n";
        let text = rewrite(original, Some([86, 148, 78])).unwrap();
        assert!(text.contains("ViewMode=1"));
        assert!(text.contains("FolderColor=#56944e"));
        assert_eq!(parse_colour(&text), Some([86, 148, 78]));
    }

    #[test]
    fn removing_the_last_setting_removes_the_file() {
        let text = rewrite("[Settings]\nFolderColor=#c44842\n", None);
        assert!(text.is_none());
        let kept = rewrite("[Settings]\nViewMode=1\nFolderColor=#c44842\n", None).unwrap();
        assert_eq!(kept, "[Settings]\nViewMode=1\n");
        assert_eq!(parse_colour(&kept), None);
    }

    #[test]
    fn ignores_a_colour_outside_the_settings_group() {
        assert_eq!(parse_colour("[Desktop Entry]\nFolderColor=#c44842\n"), None);
    }
}
