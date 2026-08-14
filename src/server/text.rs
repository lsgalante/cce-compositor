//! Minimal CPU text rendering, for the desktop-grid square labels.
//!
//! The compositor has no toolkit — clients own their own text (cce-ui does
//! Vulkan + cosmic-text). The one thing the compositor itself has to letter is the
//! desktop grid, so this is deliberately the smallest thing that works:
//! fontdue rasterizes a short ASCII label into an ARGB8888 buffer, which
//! `river_data_buffer_create` wraps as a `wlr_buffer` for a scene node.
//!
//! Labels are short and repeat across frames, so rasterized buffers are cached
//! by (text, size); the cache is swept whenever the label set changes size
//! enough to matter (see `Output::draw_cell_labels`).

use std::collections::HashMap;
use std::sync::OnceLock;

use crate::ffi;

/// Where to look for a font file, in order of preference. The DE's own font
/// wins; the rest are the usual monospace suspects so a machine without it
/// still gets labels. `CCE_GRID_LABEL_FONT` overrides everything.
const FONT_HINTS: &[&str] = &[
    "berkeleymono",
    "jetbrainsmono",
    "dejavusansmono",
    "liberationmono",
    "notosansmono",
    "firacode",
    "hack",
];

fn font_dirs() -> Vec<std::path::PathBuf> {
    let mut dirs = Vec::new();
    if let Ok(home) = std::env::var("HOME") {
        let data = std::env::var("XDG_DATA_HOME")
            .unwrap_or_else(|_| format!("{home}/.local/share"));
        dirs.push(std::path::PathBuf::from(format!("{data}/fonts")));
        dirs.push(std::path::PathBuf::from(format!("{home}/.fonts")));
        // The DE keeps its own fonts in Dropbox on this machine; harmless
        // elsewhere since a missing dir is simply skipped.
        dirs.push(std::path::PathBuf::from(format!("{home}/Dropbox/Fonts")));
    }
    dirs.push(std::path::PathBuf::from("/usr/local/share/fonts"));
    dirs.push(std::path::PathBuf::from("/usr/share/fonts"));
    dirs
}

/// Recursively collect font files, cheaply bounded so a pathological font tree
/// can't stall startup.
fn collect_fonts(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>, depth: u32) {
    if depth > 4 || out.len() > 4000 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_fonts(&path, out, depth + 1);
        } else if matches!(
            path.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()).as_deref(),
            Some("ttf") | Some("otf")
        ) {
            out.push(path);
        }
    }
}

fn normalized_stem(path: &std::path::Path) -> String {
    path.file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect::<String>()
        .to_ascii_lowercase()
}

fn load_font() -> Option<fontdue::Font> {
    let try_file = |path: &std::path::Path| -> Option<fontdue::Font> {
        let bytes = std::fs::read(path).ok()?;
        // fontdue rejects fonts it cannot parse; keep looking rather than
        // giving up on labels entirely.
        fontdue::Font::from_bytes(bytes, fontdue::FontSettings::default()).ok()
    };

    if let Ok(explicit) = std::env::var("CCE_GRID_LABEL_FONT") {
        if let Some(font) = try_file(std::path::Path::new(&explicit)) {
            log::info!("grid labels: using font {explicit}");
            return Some(font);
        }
        log::warn!("grid labels: CCE_GRID_LABEL_FONT={explicit} could not be loaded");
    }

    let mut candidates = Vec::new();
    for dir in font_dirs() {
        collect_fonts(&dir, &mut candidates, 0);
    }
    // Preferred families first, then a regular-weight fallback.
    for hint in FONT_HINTS {
        for path in &candidates {
            let stem = normalized_stem(path);
            if stem.contains(hint) && (stem.contains("regular") || !stem.contains("italic")) {
                if let Some(font) = try_file(path) {
                    log::info!("grid labels: using font {}", path.display());
                    return Some(font);
                }
            }
        }
    }
    for path in &candidates {
        if let Some(font) = try_file(path) {
            log::info!("grid labels: falling back to font {}", path.display());
            return Some(font);
        }
    }
    log::warn!("grid labels: no usable font found, labels disabled");
    None
}

fn font() -> Option<&'static fontdue::Font> {
    static FONT: OnceLock<Option<fontdue::Font>> = OnceLock::new();
    FONT.get_or_init(load_font).as_ref()
}

/// One rasterized label, owning the scene-side buffer.
pub struct Label {
    pub buffer: *mut ffi::wlr_buffer,
    pub width: i32,
    pub height: i32,
}

/// Rasterize `text` at `px` and wrap it in a wlr_buffer. White glyphs with a
/// soft dark halo so the label stays legible over both the light grid gaps and
/// the dark cells; ARGB8888 premultiplied, as the renderer expects.
fn rasterize(text: &str, px: f32) -> Option<Label> {
    let font = font()?;
    if text.is_empty() || !(4.0..=200.0).contains(&px) {
        return None;
    }

    // Lay the glyphs out on a common baseline.
    let mut glyphs = Vec::new();
    let mut pen_x = 0i32;
    let (mut top, mut bottom) = (i32::MAX, i32::MIN);
    for ch in text.chars() {
        let (metrics, bitmap) = font.rasterize(ch, px);
        let x = pen_x + metrics.xmin;
        // fontdue's ymin is the offset of the bitmap's BOTTOM from the
        // baseline, y-up; the buffer is y-down.
        let y = -(metrics.height as i32 + metrics.ymin);
        top = top.min(y);
        bottom = bottom.max(y + metrics.height as i32);
        glyphs.push((x, y, metrics.width as i32, metrics.height as i32, bitmap));
        pen_x += metrics.advance_width.round() as i32;
    }
    if glyphs.is_empty() || pen_x <= 0 || top >= bottom {
        return None;
    }

    // One pixel of padding all round so the halo has somewhere to land.
    const PAD: i32 = 2;
    let width = pen_x + 2 * PAD;
    let height = (bottom - top) + 2 * PAD;
    if width <= 0 || height <= 0 || width > 4096 || height > 4096 {
        return None;
    }

    // Coverage first, then two passes: halo from blurred coverage, glyph on
    // top. Keeping coverage separate avoids the halo eating the glyph.
    let (w, h) = (width as usize, height as usize);
    let mut cov = vec![0u8; w * h];
    for (gx, gy, gw, gh, bitmap) in &glyphs {
        for row in 0..*gh {
            for col in 0..*gw {
                let a = bitmap[(row * gw + col) as usize];
                if a == 0 {
                    continue;
                }
                let px_x = gx + col + PAD;
                let px_y = gy - top + row + PAD;
                if px_x < 0 || px_y < 0 || px_x >= width || px_y >= height {
                    continue;
                }
                let idx = px_y as usize * w + px_x as usize;
                cov[idx] = cov[idx].max(a);
            }
        }
    }

    let mut data = vec![0u8; w * h * 4];
    for y in 0..h {
        for x in 0..w {
            // Halo = max coverage of the 8 neighbours, dimmed.
            let mut halo = 0u32;
            for dy in -1i32..=1 {
                for dx in -1i32..=1 {
                    let (nx, ny) = (x as i32 + dx, y as i32 + dy);
                    if nx < 0 || ny < 0 || nx >= w as i32 || ny >= h as i32 {
                        continue;
                    }
                    halo = halo.max(cov[ny as usize * w + nx as usize] as u32);
                }
            }
            let glyph = cov[y * w + x] as u32;
            // Composite: black halo under white glyph, both premultiplied.
            let halo_a = (halo * 180) / 255;
            let out_a = (glyph + halo_a * (255 - glyph) / 255).min(255);
            let out_rgb = glyph; // white premultiplied by its own alpha
            let idx = (y * w + x) * 4;
            // ARGB8888 little-endian byte order: B, G, R, A.
            data[idx] = out_rgb as u8;
            data[idx + 1] = out_rgb as u8;
            data[idx + 2] = out_rgb as u8;
            data[idx + 3] = out_a as u8;
        }
    }

    let stride = w * 4;
    let buffer = unsafe {
        ffi::river_data_buffer_create(
            width,
            height,
            stride,
            data.as_ptr() as *const std::ffi::c_void,
        )
    };
    if buffer.is_null() {
        return None;
    }
    Some(Label { buffer, width, height })
}

/// Rasterized-label cache. Labels repeat every frame and change only as the
/// camera moves, so this keeps the per-frame cost to a hash lookup.
#[derive(Default)]
pub struct LabelCache {
    entries: HashMap<(String, u32), Option<Label>>,
}

impl LabelCache {
    /// Look up (or rasterize) a label. `None` means "cannot draw this" — no
    /// font, or an unrasterizable string — and is cached too, so a missing
    /// font costs one lookup per label rather than a filesystem scan.
    pub fn get(&mut self, text: &str, px: f32) -> Option<&Label> {
        let key = (text.to_string(), px.round() as u32);
        self.entries
            .entry(key)
            .or_insert_with(|| rasterize(text, px))
            .as_ref()
    }

    /// Drop everything (font size changed, or the cache grew unreasonably).
    pub fn clear(&mut self) {
        for (_, label) in self.entries.drain() {
            if let Some(label) = label {
                unsafe { ffi::wlr_buffer_drop(label.buffer) };
            }
        }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }
}

impl Drop for LabelCache {
    fn drop(&mut self) {
        self.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Rasterization needs a font on the machine; skip rather than fail on a
    // bare build host.
    #[test]
    fn glyph_layout_produces_sane_extents() {
        let Some(font) = font() else {
            eprintln!("no font available, skipping");
            return;
        };
        // A label the desktop actually uses.
        let (metrics, bitmap) = font.rasterize('C', 24.0);
        assert!(metrics.width > 0 && metrics.height > 0);
        assert_eq!(bitmap.len(), metrics.width * metrics.height);
        assert!(bitmap.iter().any(|&a| a > 0), "glyph rasterized blank");
    }
}
