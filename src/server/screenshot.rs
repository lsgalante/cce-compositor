//! Native screenshots.
//!
//! Two capture paths, both replying over IPC with the destination path and
//! finishing (PNG encode + notification) on a worker thread:
//!
//! - Full-output / region: `process_ipc_command` parks a [`PendingScreenshot`]
//!   on the window manager and schedules a frame; `Output::render_and_commit`
//!   picks it up right after `wlr_scene_output_build_state` renders the frame
//!   into the output state's buffer, and reads that buffer back
//!   (`wlr_texture_from_buffer` + `wlr_texture_read_pixels`). Regions are
//!   cropped CPU-side in buffer pixels.
//! - Window: the window's committed surface textures are read back directly
//!   (root surface + subsurfaces composited by their offsets), so it works
//!   even when the window is panned outside the visible viewport — the
//!   client's last committed buffers still exist regardless of culling.
//!
//! Completion is announced through the freedesktop notification daemon
//! (`notify-send` with the standard `image-path` hint, which cce-notifier
//! renders as a thumbnail). The `notifications { screenshots }` key in
//! config.kdl disables the announcement (default enabled); it is re-read per
//! screenshot on the worker thread, so edits take effect immediately.

use std::path::PathBuf;

use crate::ffi;

// DRM fourcc codes wlr_texture_preferred_read_format may hand us; all are
// 8-bit-per-channel, little-endian packed (so ARGB8888 is B,G,R,A in memory).
const DRM_FORMAT_XRGB8888: u32 = 0x34325258;
const DRM_FORMAT_ARGB8888: u32 = 0x34325241;
const DRM_FORMAT_XBGR8888: u32 = 0x34324258;
const DRM_FORMAT_ABGR8888: u32 = 0x34324241;

/// A full-output / region capture waiting for the next composited frame.
pub struct PendingScreenshot {
    /// The output whose next frame is captured.
    pub output: *mut crate::output::Output,
    /// Crop in output-buffer pixels; `None` captures the whole output.
    pub region: Option<ffi::wlr_box>,
    pub path: PathBuf,
}

/// `~/Pictures/screenshots/screenshot-YYYYMMDD-HHMMSS.png` (the directory is
/// created by the encode thread).
pub fn default_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    let now = unsafe { libc::time(std::ptr::null_mut()) };
    unsafe { libc::localtime_r(&now, &mut tm) };
    let name = format!(
        "screenshot-{:04}{:02}{:02}-{:02}{:02}{:02}.png",
        tm.tm_year + 1900,
        tm.tm_mon + 1,
        tm.tm_mday,
        tm.tm_hour,
        tm.tm_min,
        tm.tm_sec
    );
    PathBuf::from(home).join("Pictures").join("screenshots").join(name)
}

/// Read a texture's full contents into a tightly packed `w*h*4` byte buffer.
/// Returns the bytes plus the DRM format they are in.
unsafe fn read_texture(texture: *mut ffi::wlr_texture, w: i32, h: i32) -> Option<(Vec<u8>, u32)> {
    if texture.is_null() || w <= 0 || h <= 0 {
        return None;
    }
    let format = ffi::wlr_texture_preferred_read_format(texture);
    let mut data = vec![0u8; (w as usize) * (h as usize) * 4];
    let options = ffi::wlr_texture_read_pixels_options {
        data: data.as_mut_ptr() as *mut std::ffi::c_void,
        format,
        stride: (w as u32) * 4,
        dst_x: 0,
        dst_y: 0,
        src_box: std::mem::zeroed(), // empty = full texture
    };
    if !ffi::wlr_texture_read_pixels(texture, &options) {
        return None;
    }
    Some((data, format))
}

/// Convert read-back pixels to RGBA in place. Alpha is forced opaque — the
/// X-variants carry garbage alpha, and screenshots should not be translucent.
fn to_rgba(mut pixels: Vec<u8>, format: u32) -> Option<Vec<u8>> {
    match format {
        DRM_FORMAT_XRGB8888 | DRM_FORMAT_ARGB8888 => {
            for px in pixels.chunks_exact_mut(4) {
                px.swap(0, 2);
                px[3] = 255;
            }
            Some(pixels)
        }
        DRM_FORMAT_XBGR8888 | DRM_FORMAT_ABGR8888 => {
            for px in pixels.chunks_exact_mut(4) {
                px[3] = 255;
            }
            Some(pixels)
        }
        _ => {
            log::warn!("screenshot: unsupported read format {format:#x}");
            None
        }
    }
}

fn crop_rgba(pixels: &[u8], w: i32, h: i32, region: ffi::wlr_box) -> Option<(Vec<u8>, i32, i32)> {
    let x0 = region.x.clamp(0, w);
    let y0 = region.y.clamp(0, h);
    let x1 = (region.x + region.width).clamp(0, w);
    let y1 = (region.y + region.height).clamp(0, h);
    let (cw, ch) = (x1 - x0, y1 - y0);
    if cw <= 0 || ch <= 0 {
        return None;
    }
    let mut out = Vec::with_capacity((cw as usize) * (ch as usize) * 4);
    for row in y0..y1 {
        let start = ((row * w + x0) * 4) as usize;
        out.extend_from_slice(&pixels[start..start + (cw as usize) * 4]);
    }
    Some((out, cw, ch))
}

/// Full-output / region capture: called from `Output::render_and_commit`
/// after a successful commit, while the output state's buffer is still alive.
pub unsafe fn capture_state_buffer(
    renderer: *mut ffi::wlr_renderer,
    buffer: *mut ffi::wlr_buffer,
    buf_w: i32,
    buf_h: i32,
    shot: PendingScreenshot,
) {
    let texture = ffi::wlr_texture_from_buffer(renderer, buffer);
    if texture.is_null() {
        log::warn!("screenshot: wlr_texture_from_buffer failed");
        return;
    }
    let read = read_texture(texture, buf_w, buf_h);
    ffi::wlr_texture_destroy(texture);
    let Some((pixels, format)) = read else {
        log::warn!("screenshot: pixel readback failed");
        return;
    };
    let Some(rgba) = to_rgba(pixels, format) else { return };
    let (rgba, out_w, out_h) = match shot.region {
        Some(region) => match crop_rgba(&rgba, buf_w, buf_h, region) {
            Some(cropped) => cropped,
            None => {
                log::warn!("screenshot: region outside the output");
                return;
            }
        },
        None => (rgba, buf_w, buf_h),
    };
    spawn_encode(rgba, out_w as u32, out_h as u32, shot.path);
}

/// Window capture straight from the committed surface textures: the root
/// surface's buffer is the canvas, subsurfaces composite at their offsets.
/// Works for windows outside the visible viewport (their last committed
/// buffers persist), but needs the client to have committed at least once.
pub unsafe fn capture_window(window: *mut crate::window::Window, path: PathBuf) -> Result<String, String> {
    let (canvas, bw, bh) = capture_window_rgba(window)?;
    let reply = path.display().to_string();
    spawn_encode(canvas, bw as u32, bh as u32, path);
    Ok(reply)
}

/// The readback+composite half of [`capture_window`], PNG-free: returns the
/// tightly packed RGBA canvas and its pixel dimensions. Also the frame source
/// for the window-stream server.
pub unsafe fn capture_window_rgba(window: *mut crate::window::Window) -> Result<(Vec<u8>, i32, i32), String> {
    let root = (*window).root_surface();
    if root.is_null() {
        return Err("window has no surface".to_string());
    }
    let (mut bw, mut bh) = (0i32, 0i32);
    ffi::river_wlr_surface_get_buffer_size(root, &mut bw, &mut bh);
    if bw <= 0 || bh <= 0 {
        return Err("window has no committed buffer".to_string());
    }
    // Subsurface offsets are surface-logical; buffers are physical pixels.
    let logical_w = ffi::river_wlr_surface_get_width(root).max(1);
    let scale = bw as f64 / logical_w as f64;

    struct Collect {
        list: Vec<(*mut ffi::wlr_surface, i32, i32)>,
    }
    unsafe extern "C" fn collect_cb(
        surface: *mut ffi::wlr_surface,
        sx: std::os::raw::c_int,
        sy: std::os::raw::c_int,
        data: *mut std::ffi::c_void,
    ) {
        let collect = &mut *(data as *mut Collect);
        collect.list.push((surface, sx, sy));
    }
    let mut collect = Collect { list: Vec::new() };
    ffi::wlr_surface_for_each_surface(
        root,
        Some(collect_cb),
        &mut collect as *mut Collect as *mut std::ffi::c_void,
    );

    let mut canvas = vec![0u8; (bw as usize) * (bh as usize) * 4];
    let mut composited = 0usize;
    for (surface, sx, sy) in collect.list {
        let texture = ffi::wlr_surface_get_texture(surface);
        if texture.is_null() {
            continue;
        }
        let (mut sw, mut sh) = (0i32, 0i32);
        ffi::river_wlr_surface_get_buffer_size(surface, &mut sw, &mut sh);
        let Some((pixels, format)) = read_texture(texture, sw, sh) else { continue };
        let Some(rgba) = to_rgba(pixels, format) else { continue };
        let dst_x = (sx as f64 * scale).round() as i32;
        let dst_y = (sy as f64 * scale).round() as i32;
        blit(&mut canvas, bw, bh, &rgba, sw, sh, dst_x, dst_y);
        composited += 1;
    }
    if composited == 0 {
        return Err("no readable surface content".to_string());
    }
    Ok((canvas, bw, bh))
}

/// Copy `src` (sw×sh RGBA) into `dst` (dw×dh RGBA) at (dx, dy), clipped.
fn blit(dst: &mut [u8], dw: i32, dh: i32, src: &[u8], sw: i32, sh: i32, dx: i32, dy: i32) {
    for sy in 0..sh {
        let ty = dy + sy;
        if ty < 0 || ty >= dh {
            continue;
        }
        let sx0 = (-dx).clamp(0, sw);
        let sx1 = (dw - dx).clamp(0, sw);
        if sx0 >= sx1 {
            continue;
        }
        let src_start = ((sy * sw + sx0) * 4) as usize;
        let dst_start = ((ty * dw + dx + sx0) * 4) as usize;
        let len = ((sx1 - sx0) * 4) as usize;
        dst[dst_start..dst_start + len].copy_from_slice(&src[src_start..src_start + len]);
    }
}

/// PNG-encode and save off the main thread, then announce via the
/// notification daemon (unless disabled in config).
fn spawn_encode(rgba: Vec<u8>, w: u32, h: u32, path: PathBuf) {
    std::thread::spawn(move || {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let file = match std::fs::File::create(&path) {
            Ok(f) => f,
            Err(e) => {
                log::warn!("screenshot: failed to create {}: {e}", path.display());
                return;
            }
        };
        let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), w, h);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let write = encoder
            .write_header()
            .and_then(|mut writer| writer.write_image_data(&rgba));
        if let Err(e) = write {
            log::warn!("screenshot: failed to encode {}: {e}", path.display());
            return;
        }
        log::info!("screenshot saved to {}", path.display());

        if notifications_enabled() {
            let path_str = path.display().to_string();
            let _ = std::process::Command::new("notify-send")
                .arg("-a")
                .arg("cce")
                .arg("-h")
                .arg(format!("string:image-path:{path_str}"))
                .arg("Screenshot saved")
                .arg(&path_str)
                .spawn();
        }
    });
}

/// `notifications { screenshots <bool> }` in the shared config.kdl; absent
/// means enabled.
fn notifications_enabled() -> bool {
    let Ok(content) = std::fs::read_to_string(cce_ui::config::get_config_path()) else {
        return true;
    };
    cce_ui::config::parse_kdl_to_json(&content)
        .pointer("/notifications/screenshots")
        .and_then(|v| v.as_bool())
        .unwrap_or(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn to_rgba_swizzles_bgra_and_forces_opaque() {
        // One BGRA pixel: B=1 G=2 R=3 A=4 → RGBA 3,2,1,255.
        let out = to_rgba(vec![1, 2, 3, 4], DRM_FORMAT_ARGB8888).unwrap();
        assert_eq!(out, vec![3, 2, 1, 255]);
        // RGBA passthrough, alpha forced opaque.
        let out = to_rgba(vec![1, 2, 3, 4], DRM_FORMAT_ABGR8888).unwrap();
        assert_eq!(out, vec![1, 2, 3, 255]);
        assert!(to_rgba(vec![0; 4], 0x1234).is_none());
    }

    #[test]
    fn crop_clamps_to_bounds() {
        // 2x2 image, pixels numbered 0..4 in the red channel.
        let px: Vec<u8> = (0..4u8).flat_map(|i| [i, 0, 0, 255]).collect();
        let region = ffi::wlr_box { x: 1, y: 0, width: 5, height: 5 };
        let (out, w, h) = crop_rgba(&px, 2, 2, region).unwrap();
        assert_eq!((w, h), (1, 2));
        assert_eq!(out[0], 1);
        assert_eq!(out[4], 3);
        let empty = ffi::wlr_box { x: 5, y: 5, width: 1, height: 1 };
        assert!(crop_rgba(&px, 2, 2, empty).is_none());
    }

    #[test]
    fn blit_clips_at_edges() {
        let mut dst = vec![0u8; 2 * 2 * 4];
        let src: Vec<u8> = vec![9; 2 * 2 * 4];
        blit(&mut dst, 2, 2, &src, 2, 2, 1, 1); // only dst (1,1) covered
        assert_eq!(dst[(1 * 2 + 1) * 4], 9);
        assert_eq!(dst[0], 0);
        blit(&mut dst, 2, 2, &src, 2, 2, -5, -5); // fully clipped: no panic
    }
}
