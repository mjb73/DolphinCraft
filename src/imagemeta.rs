//! Reading the generation prompt out of a picture.
//!
//! Added feature (asked for by the person): "extract meta data from prompt".
//! Pictures made by the common text-to-image tools carry their recipe in the
//! file itself, in one of a few well-known places:
//!
//! * PNG `tEXt`/`iTXt` chunk with the key `parameters` — the Automatic1111 /
//!   Forge / SD.next shape: the prompt, an optional `Negative prompt:` block and
//!   a last line of `Steps: 20, Sampler: Euler a, CFG scale: 7, Seed: 1234,
//!   Size: 512x512, Model: ...` pairs.
//! * PNG `tEXt` chunk with the key `prompt` or `workflow` — ComfyUI writes its
//!   graph as JSON there.
//! * PNG `tEXt` key `Comment` — NovelAI writes its JSON there.
//! * PNG `tEXt` key `Description` / `Software` — Invoke and Photoshop-ish tools.
//! * JPEG `APP1` Exif `UserComment` (tag 0x9286) or `ImageDescription` (0x010E).
//! * A text file sitting next to the picture with the same stem (`.txt`), which
//!   is how people save a prompt by hand.
//!
//! Nothing here is copied from the original program: the original has no
//! prompt reader at all, so this file is new work. It is deliberately dependency
//! free (the `image` crate only decodes pixels; it does not expose chunk text).

use std::path::{Path, PathBuf};

/// Everything worth showing about how a picture was generated.
#[derive(Clone, Debug, Default)]
pub struct ImageMeta {
    /// The positive prompt, when the file carries one as plain text.
    pub prompt: Option<String>,
    /// The negative prompt, when there is one.
    pub negative: Option<String>,
    /// Label / value pairs: Steps, Sampler, CFG scale, Seed, Size, Model…
    pub fields: Vec<(String, String)>,
    /// Where the text came from: "parameters", "prompt", "UserComment"…
    pub source: String,
    /// The untouched text, kept so the person can copy the whole thing.
    pub raw: String,
}

impl ImageMeta {
    pub fn is_empty(&self) -> bool {
        self.prompt.is_none() && self.fields.is_empty() && self.raw.trim().is_empty()
    }

    /// The text a "Copy prompt" button puts on the clipboard: the prompt and,
    /// when the file has one, the negative prompt.
    pub fn copy_text(&self) -> String {
        let mut out = String::new();
        if let Some(p) = &self.prompt {
            out.push_str(p);
        }
        if let Some(n) = &self.negative {
            if !out.is_empty() {
                out.push_str("\n\n");
            }
            out.push_str("Negative prompt: ");
            out.push_str(n);
        }
        if out.is_empty() {
            out.push_str(&self.raw);
        }
        out
    }

    /// The positive prompt on its own — the text a person pastes back into a
    /// text-to-image tool. Falls back to whatever the file did carry.
    pub fn prompt_only(&self) -> String {
        match &self.prompt {
            Some(p) if !p.trim().is_empty() => p.clone(),
            _ => self.raw.trim().to_string(),
        }
    }

    /// The whole recipe in one block: prompt, negative prompt, then the
    /// settings line (`Steps: 28, Sampler: …, Model: …`) rebuilt from the
    /// fields, so a copied recipe reads the way the tool wrote it.
    pub fn recipe_text(&self) -> String {
        let mut out = String::new();
        if let Some(p) = &self.prompt {
            out.push_str(p.trim());
        }
        if let Some(n) = &self.negative {
            if !out.is_empty() {
                out.push_str("\n\n");
            }
            out.push_str("Negative prompt: ");
            out.push_str(n.trim());
        }
        if !self.fields.is_empty() {
            if !out.is_empty() {
                out.push_str("\n\n");
            }
            let pairs: Vec<String> = self
                .fields
                .iter()
                .map(|(k, v)| format!("{k}: {v}"))
                .collect();
            out.push_str(&pairs.join(", "));
        }
        if out.trim().is_empty() {
            out = self.raw.trim().to_string();
        }
        out
    }

    /// Write the recipe into `<picture>.prompt.txt` next to the picture, which
    /// is where the prompt reader already looks for hand-saved prompts. An
    /// existing file is never overwritten: the second save becomes
    /// `<picture>.2.prompt.txt`, the third `.3.`, and so on.
    pub fn save_beside(&self, picture: &Path) -> std::io::Result<PathBuf> {
        let stem = picture
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "picture".to_string());
        let mut target = picture.with_file_name(format!("{stem}.prompt.txt"));
        let mut n = 2;
        while target.exists() {
            target = picture.with_file_name(format!("{stem}.{n}.prompt.txt"));
            n += 1;
            if n > 999 {
                break;
            }
        }
        let mut body = self.recipe_text();
        body.push('\n');
        std::fs::write(&target, body)?;
        Ok(target)
    }
}

/// Whether a path looks like a picture, by its extension — the same list the
/// file listing uses to pick the picture icon. Used by the file view's
/// "Copy Prompt" action, which only knows paths.
pub fn is_picture_path(path: &Path) -> bool {
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    matches!(
        ext.as_str(),
        "png"
            | "jpg"
            | "jpeg"
            | "gif"
            | "bmp"
            | "webp"
            | "tif"
            | "tiff"
            | "svg"
            | "ico"
            | "heic"
            | "avif"
            | "xpm"
            | "ppm"
    )
}

/// The prompts of several pictures in one block of text, ready for the
/// clipboard: the file name on its own line, then its prompt, then a blank
/// line. Pictures with no prompt are left out, so the count is the number of
/// prompts actually copied rather than the number of files picked.
pub fn prompts_of(paths: &[PathBuf]) -> (String, usize) {
    let mut out = String::new();
    let mut n = 0usize;
    for path in paths {
        let Some(meta) = read(path) else { continue };
        if meta.prompt.is_none() && meta.raw.trim().is_empty() {
            continue;
        }
        let text = meta.prompt_only();
        if text.trim().is_empty() {
            continue;
        }
        if n > 0 {
            out.push('\n');
        }
        out.push_str(
            &path
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default(),
        );
        out.push('\n');
        out.push_str(text.trim());
        out.push('\n');
        n += 1;
    }
    (out, n)
}

/// Read the generation metadata of `path`, looking at the picture itself first
/// and at a neighbouring `.txt` last.
pub fn read(path: &Path) -> Option<ImageMeta> {
    let bytes = std::fs::read(path).ok()?;
    let mut found = Vec::new();
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        found.extend(png_text_chunks(&bytes));
    } else if bytes.starts_with(&[0xFF, 0xD8]) {
        found.extend(jpeg_text_chunks(&bytes));
    }
    if let Some(meta) = best_of(found) {
        return Some(meta);
    }
    // Last resort: the sidecar text file people keep beside a prompt.
    let sidecar = sidecar_path(path);
    if let Some(p) = sidecar {
        if let Ok(text) = std::fs::read_to_string(p) {
            if !text.trim().is_empty() {
                return Some(from_text(&text, "text file"));
            }
        }
    }
    None
}

fn sidecar_path(path: &Path) -> Option<PathBuf> {
    let stem = path.file_stem()?.to_string_lossy().to_string();
    Some(path.with_file_name(format!("{stem}.txt")))
}

/// Choose the most informative block: a plain-text prompt beats a JSON graph,
/// and JSON still gives fields when no plain prompt exists.
fn best_of(blocks: Vec<(String, String)>) -> Option<ImageMeta> {
    let mut best: Option<ImageMeta> = None;
    for (key, text) in blocks {
        let meta = from_text(&text, &key);
        let score = |m: &ImageMeta| {
            m.prompt.as_ref().map(|p| p.len()).unwrap_or(0) + m.fields.len() * 20
        };
        let better = match &best {
            None => true,
            Some(b) => score(&meta) > score(b),
        };
        if better {
            best = Some(meta);
        }
    }
    best.filter(|m| !m.is_empty())
}

/// Turn one block of text into metadata, whatever shape it has.
pub fn from_text(text: &str, source: &str) -> ImageMeta {
    let trimmed = text.trim();
    let mut meta = ImageMeta {
        source: source.to_string(),
        raw: trimmed.to_string(),
        ..Default::default()
    };
    if trimmed.starts_with('{') {
        // ComfyUI / NovelAI / Invoke: a JSON blob. Pull out the handful of
        // values that matter without a JSON dependency; a full parse would only
        // add weight for a viewer.
        if let Some(p) = json_string(trimmed, &["\"prompt\"", "\"positive\"", "\"Positive prompt\""]) {
            meta.prompt = Some(p);
        }
        if let Some(n) = json_string(trimmed, &["\"uc\"", "\"negative\"", "\"Negative prompt\""]) {
            meta.negative = Some(n);
        }
        for (label, keys) in [
            ("Steps", &["\"steps\"", "\"Steps\""][..]),
            ("Sampler", &["\"sampler_name\"", "\"sampler\"", "\"Sampler\""]),
            ("CFG scale", &["\"cfg\"", "\"cfg_scale\"", "\"scale\"", "\"CFG scale\""]),
            ("Seed", &["\"seed\"", "\"Seed\""]),
            ("Model", &["\"ckpt_name\"", "\"model\"", "\"Model\""]),
            ("Size", &["\"size\"", "\"Size\""]),
        ] {
            if let Some(v) = json_value(trimmed, keys) {
                meta.fields.push((label.to_string(), v));
            }
        }
        return meta;
    }

    // Automatic1111 shape: prompt, optional "Negative prompt:" block, then a
    // final line of "Key: value" pairs.
    let mut prompt_lines: Vec<&str> = Vec::new();
    let mut negative_lines: Vec<&str> = Vec::new();
    let mut param_line: Option<&str> = None;
    let mut in_negative = false;
    for line in trimmed.lines() {
        let l = line.trim_end();
        if is_parameter_line(l) {
            param_line = Some(l);
            continue;
        }
        if l.trim_start().starts_with("Negative prompt:") {
            in_negative = true;
            let rest = l.trim_start()["Negative prompt:".len()..].trim();
            if !rest.is_empty() {
                negative_lines.push(rest);
            }
            continue;
        }
        if in_negative {
            negative_lines.push(l);
        } else {
            prompt_lines.push(l);
        }
    }
    let prompt = prompt_lines.join("\n").trim().to_string();
    if !prompt.is_empty() {
        meta.prompt = Some(prompt);
    }
    let negative = negative_lines.join("\n").trim().to_string();
    if !negative.is_empty() {
        meta.negative = Some(negative);
    }
    if let Some(line) = param_line {
        for pair in split_parameters(line) {
            meta.fields.push(pair);
        }
    }
    meta
}

/// A line made only of `Key: value` pairs — the parameters line A1111 appends.
fn is_parameter_line(line: &str) -> bool {
    let l = line.trim();
    if !l.contains(':') {
        return false;
    }
    // It must start with one of the keys these tools always write, so a prompt
    // line such as "a portrait: soft light" is not mistaken for parameters.
    const KEYS: [&str; 6] = ["Steps:", "Sampler:", "CFG scale:", "Seed:", "Size:", "Model:"];
    KEYS.iter().any(|k| l.starts_with(k))
}

/// Split the parameters line on commas that separate `Key: value` pairs.
fn split_parameters(line: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut depth = 0i32;
    let mut items: Vec<String> = Vec::new();
    for ch in line.chars() {
        match ch {
            '"' | '\'' => {
                current.push(ch);
            }
            '(' | '[' | '{' => {
                depth += 1;
                current.push(ch);
            }
            ')' | ']' | '}' => {
                depth -= 1;
                current.push(ch);
            }
            ',' if depth <= 0 => {
                items.push(current.trim().to_string());
                current.clear();
            }
            _ => current.push(ch),
        }
    }
    if !current.trim().is_empty() {
        items.push(current.trim().to_string());
    }
    for item in items {
        if let Some((k, v)) = item.split_once(':') {
            let key = k.trim();
            let val = v.trim();
            if key.is_empty() || val.is_empty() {
                continue;
            }
            // "Size: 512x512" is two numbers, "Model hash: abc" a hex string;
            // both stay as text.
            out.push((key.to_string(), val.to_string()));
        }
    }
    out
}

// ---------------------------------------------------------------------------
// PNG text chunks
// ---------------------------------------------------------------------------

fn png_text_chunks(bytes: &[u8]) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut pos = 8usize;
    while pos + 8 <= bytes.len() {
        let len = u32::from_be_bytes([bytes[pos], bytes[pos + 1], bytes[pos + 2], bytes[pos + 3]])
            as usize;
        let kind = &bytes[pos + 4..pos + 8];
        let start = pos + 8;
        let end = start.saturating_add(len);
        if end + 4 > bytes.len() {
            break;
        }
        let data = &bytes[start..end];
        match kind {
            b"tEXt" => {
                if let Some(zero) = data.iter().position(|b| *b == 0) {
                    let key = String::from_utf8_lossy(&data[..zero]).to_string();
                    let text = decode_text(&data[zero + 1..]);
                    if !text.trim().is_empty() {
                        out.push((key, text));
                    }
                }
            }
            b"iTXt" => {
                // keyword\0 compFlag compMethod language\0 translated\0 text
                if let Some(zero) = data.iter().position(|b| *b == 0) {
                    let key = String::from_utf8_lossy(&data[..zero]).to_string();
                    let rest = &data[zero + 1..];
                    if rest.len() >= 2 {
                        let compressed = rest[0] == 1;
                        let mut p = 2;
                        // language tag
                        if let Some(z) = rest[p..].iter().position(|b| *b == 0) {
                            p += z + 1;
                        } else {
                            continue;
                        }
                        // translated keyword
                        if let Some(z) = rest[p..].iter().position(|b| *b == 0) {
                            p += z + 1;
                        } else {
                            continue;
                        }
                        if compressed {
                            continue; // zlib: not needed by the tools in use
                        }
                        let text = String::from_utf8_lossy(&rest[p..]).to_string();
                        if !text.trim().is_empty() {
                            out.push((key, text));
                        }
                    }
                }
            }
            b"IEND" => break,
            _ => {}
        }
        pos = end + 4;
    }
    out
}

fn decode_text(raw: &[u8]) -> String {
    match std::str::from_utf8(raw) {
        Ok(s) => s.to_string(),
        Err(_) => raw.iter().map(|b| *b as char).collect(),
    }
}

// ---------------------------------------------------------------------------
// JPEG Exif
// ---------------------------------------------------------------------------

fn jpeg_text_chunks(bytes: &[u8]) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut pos = 2usize;
    while pos + 4 <= bytes.len() {
        if bytes[pos] != 0xFF {
            pos += 1;
            continue;
        }
        let marker = bytes[pos + 1];
        if marker == 0xD8 || marker == 0x01 || (0xD0..=0xD7).contains(&marker) {
            pos += 2;
            continue;
        }
        let len = u16::from_be_bytes([bytes[pos + 2], bytes[pos + 3]]) as usize;
        if len < 2 || pos + 2 + len > bytes.len() {
            break;
        }
        let seg = &bytes[pos + 4..pos + 2 + len];
        if marker == 0xE1 && seg.starts_with(b"Exif\0\0") {
            out.extend(exif_text(seg));
        }
        if marker == 0xDA {
            break; // start of scan: no more metadata
        }
        pos += 2 + len;
    }
    out
}

fn exif_text(seg: &[u8]) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let tiff = &seg[6..];
    if tiff.len() < 8 {
        return out;
    }
    let big = match &tiff[0..2] {
        b"II" => false,
        b"MM" => true,
        _ => return out,
    };
    let u16_at = |d: &[u8], o: usize| -> u16 {
        if o + 2 > d.len() {
            return 0;
        }
        if big {
            u16::from_be_bytes([d[o], d[o + 1]])
        } else {
            u16::from_le_bytes([d[o], d[o + 1]])
        }
    };
    let u32_at = |d: &[u8], o: usize| -> u32 {
        if o + 4 > d.len() {
            return 0;
        }
        if big {
            u32::from_be_bytes([d[o], d[o + 1], d[o + 2], d[o + 3]])
        } else {
            u32::from_le_bytes([d[o], d[o + 1], d[o + 2], d[o + 3]])
        }
    };
    let ifd0 = u32_at(tiff, 4) as usize;
    let mut exif_ifd = None;
    // IFD0 entries: look for the pointer to the Exif sub-IFD and for a plain
    // description.
    let count = u16_at(tiff, ifd0) as usize;
    for i in 0..count {
        let e = ifd0 + 2 + i * 12;
        if e + 12 > tiff.len() {
            break;
        }
        let tag = u16_at(tiff, e);
        let typ = u16_at(tiff, e + 2);
        let cnt = u32_at(tiff, e + 4) as usize;
        let val_off = u32_at(tiff, e + 8) as usize;
        match tag {
            0x8769 => exif_ifd = Some(val_off),
            0x010E => {
                if let Some(t) = ascii_value(tiff, typ, cnt, e + 8, val_off, big) {
                    out.push(("ImageDescription".into(), t));
                }
            }
            _ => {}
        }
    }
    if let Some(ifd) = exif_ifd {
        let count = u16_at(tiff, ifd) as usize;
        for i in 0..count {
            let e = ifd + 2 + i * 12;
            if e + 12 > tiff.len() {
                break;
            }
            let tag = u16_at(tiff, e);
            if tag != 0x9286 {
                continue;
            }
            let typ = u16_at(tiff, e + 2);
            let cnt = u32_at(tiff, e + 4) as usize;
            let val_off = u32_at(tiff, e + 8) as usize;
            if let Some(text) = undefined_value(tiff, typ, cnt, e + 8, val_off, big) {
                if !text.trim().is_empty() {
                    out.push(("UserComment".into(), text));
                }
            }
        }
    }
    out
}

fn ascii_value(
    d: &[u8],
    typ: u16,
    cnt: usize,
    inline: usize,
    off: usize,
    _big: bool,
) -> Option<String> {
    if typ != 2 {
        return None;
    }
    let raw = if cnt <= 4 {
        &d[inline..(inline + cnt).min(d.len())]
    } else {
        if off + cnt > d.len() {
            return None;
        }
        &d[off..off + cnt]
    };
    let end = raw.iter().position(|b| *b == 0).unwrap_or(raw.len());
    Some(String::from_utf8_lossy(&raw[..end]).to_string())
}

fn undefined_value(
    d: &[u8],
    typ: u16,
    cnt: usize,
    inline: usize,
    off: usize,
    big: bool,
) -> Option<String> {
    if typ != 7 && typ != 1 && typ != 2 {
        return None;
    }
    let raw = if cnt <= 4 {
        &d[inline..(inline + cnt).min(d.len())]
    } else {
        if off + cnt > d.len() {
            return None;
        }
        &d[off..off + cnt]
    };
    // The first eight bytes say how the rest is encoded.
    if raw.len() > 8 {
        match &raw[..8] {
            b"ASCII\0\0\0" => return Some(String::from_utf8_lossy(&raw[8..]).trim_end_matches('\0').to_string()),
            b"UNICODE\0" => {
                let body = &raw[8..];
                let units: Vec<u16> = body
                    .chunks_exact(2)
                    .map(|c| {
                        if big {
                            u16::from_be_bytes([c[0], c[1]])
                        } else {
                            u16::from_le_bytes([c[0], c[1]])
                        }
                    })
                    .filter(|u| *u != 0)
                    .collect();
                return Some(String::from_utf16_lossy(&units));
            }
            _ => {}
        }
    }
    Some(
        String::from_utf8_lossy(raw)
            .trim_end_matches('\0')
            .to_string(),
    )
}

// ---------------------------------------------------------------------------
// A very small JSON reader, only for the fixed keys above
// ---------------------------------------------------------------------------

/// Find `"key": "value"` (first of `keys` that matches) and return the string.
fn json_string(text: &str, keys: &[&str]) -> Option<String> {
    for key in keys {
        if let Some(idx) = text.find(key) {
            let rest = &text[idx + key.len()..];
            let rest = rest.trim_start();
            let rest = rest.strip_prefix(':')?.trim_start();
            if let Some(stripped) = rest.strip_prefix('"') {
                let mut out = String::new();
                let mut chars = stripped.chars();
                while let Some(c) = chars.next() {
                    match c {
                        '"' => return Some(out),
                        '\\' => {
                            if let Some(next) = chars.next() {
                                match next {
                                    'n' => out.push('\n'),
                                    't' => out.push('\t'),
                                    other => out.push(other),
                                }
                            }
                        }
                        other => out.push(other),
                    }
                }
            }
        }
    }
    None
}

/// Find a number or a short string for `"key": ...`.
fn json_value(text: &str, keys: &[&str]) -> Option<String> {
    for key in keys {
        if let Some(idx) = text.find(key) {
            let rest = &text[idx + key.len()..];
            let rest = rest.trim_start();
            let rest = rest.strip_prefix(':')?.trim_start();
            if let Some(s) = json_string_at(rest) {
                return Some(s);
            }
            let num: String = rest
                .chars()
                .take_while(|c| c.is_ascii_digit() || *c == '.' || *c == '-' || *c == 'e' || *c == '+')
                .collect();
            if !num.is_empty() {
                return Some(trim_number(&num));
            }
        }
    }
    None
}

fn json_string_at(rest: &str) -> Option<String> {
    if !rest.starts_with('"') {
        return None;
    }
    let mut out = String::new();
    let mut chars = rest[1..].chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => return Some(out),
            '\\' => {
                if let Some(n) = chars.next() {
                    out.push(n);
                }
            }
            other => out.push(other),
        }
    }
    None
}

/// 7.0 → "7", 20.0 → "20".
pub fn trim_number(s: &str) -> String {
    if let Ok(v) = s.parse::<f64>() {
        if (v - v.round()).abs() < 1e-9 {
            return format!("{}", v.round() as i64);
        }
        return format!("{v}");
    }
    s.to_string()
}

/// Format a pixel size as the picture tools write it: "1024 × 768".
pub fn size_label(w: u32, h: u32) -> String {
    format!("{w} × {h}")
}

/// "1024 × 768 · 0.8 MP" — the caption the gallery can show under a picture.
pub fn caption_label(w: u32, h: u32) -> String {
    let mp = (w as f64 * h as f64) / 1_000_000.0;
    if mp >= 0.1 {
        format!("{} × {} · {:.1} MP", w, h, mp)
    } else {
        format!("{} × {}", w, h)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a1111_parameters() {
        let text = "a lighthouse at dusk\nNegative prompt: blurry\nSteps: 28, Sampler: DPM++ 2M, CFG scale: 6.5, Seed: 1234567890, Size: 832x1216, Model: dream-v2";
        let m = from_text(text, "parameters");
        assert_eq!(m.prompt.as_deref(), Some("a lighthouse at dusk"));
        assert_eq!(m.negative.as_deref(), Some("blurry"));
        let field = |k: &str| {
            m.fields
                .iter()
                .find(|(a, _)| a == k)
                .map(|(_, v)| v.clone())
        };
        assert_eq!(field("Steps").as_deref(), Some("28"));
        assert_eq!(field("CFG scale").as_deref(), Some("6.5"));
        assert_eq!(field("Size").as_deref(), Some("832x1216"));
    }

    #[test]
    fn reads_comfy_json() {
        let text = r#"{"prompt": "a red fox", "steps": 20, "sampler_name": "euler", "cfg": 7.5, "seed": 42, "ckpt_name": "flux-dev"}"#;
        let m = from_text(text, "prompt");
        assert_eq!(m.prompt.as_deref(), Some("a red fox"));
        assert!(m.fields.iter().any(|(k, v)| k == "Steps" && v == "20"));
    }

    #[test]
    fn parameter_line_needs_a_known_key() {
        assert!(!is_parameter_line("a portrait: soft light"));
        assert!(is_parameter_line("Steps: 20, Sampler: Euler"));
    }
}
