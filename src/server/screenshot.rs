//! Native screenshots.
//!
//! Two capture paths, both finishing (PNG encode + notification) on a worker
//! thread, and both answering the waiting `ccectl` only once the readback has
//! actually produced pixels:
//!
//! - Full-output / region: `process_ipc_command` parks a [`PendingScreenshot`]
//!   on the window manager and schedules a frame; `Output::render_and_commit`
//!   picks it up right after `wlr_scene_output_build_state` renders the frame
//!   into the output state's buffer, and reads that buffer back
//!   (`wlr_texture_from_buffer` + `wlr_texture_read_pixels`). Regions are
//!   cropped CPU-side in buffer pixels. Because that happens a frame later,
//!   the IPC reply travels with the parked capture (see [`PendingScreenshot`])
//!   instead of being answered optimistically at park time.
//! - Window: the window's committed surface textures are read back directly
//!   (root surface + subsurfaces composited by their offsets), so it works
//!   even when the window is panned outside the visible viewport — the
//!   client's last committed buffers still exist regardless of culling.
//!
//! Completion is announced through the freedesktop notification daemon
//! (`notify-send` with the standard `image-path` hint, which cce-notifier
//! renders as a thumbnail). The `notifications { screenshots }` key in
//! config.kdl disables the announcement (default enabled, but silent when
//! the config itself cannot be read); it is re-read per screenshot on the
//! worker thread, so edits take effect immediately.
//!
//! Destination names carry milliseconds and are uniquified before being
//! handed out — a second is long enough for two captures, and the loser used
//! to overwrite the winner.

use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;

use crate::ffi;

// DRM fourcc codes wlr_texture_preferred_read_format may hand us; all are
// 8-bit-per-channel, little-endian packed. The fourcc name lists channels
// most-significant first, so the memory order is that name *reversed*:
// ARGB8888 is B,G,R,A in memory, and BGR888 — despite the name — is R,G,B.
const DRM_FORMAT_XRGB8888: u32 = 0x34325258;
const DRM_FORMAT_ARGB8888: u32 = 0x34325241;
const DRM_FORMAT_XBGR8888: u32 = 0x34324258;
const DRM_FORMAT_ABGR8888: u32 = 0x34324241;
// The 24-bit pair: 3 bytes/px, no alpha channel at all. NVIDIA's GLES
// renderer hands these back where Intel's offers a 32-bit format, so a
// compositor that only knows the 8888 formats cannot screenshot on it.
const DRM_FORMAT_BGR888: u32 = 0x34324742;
const DRM_FORMAT_RGB888: u32 = 0x34324752;

/// A full-output / region capture waiting for the next composited frame.
pub struct PendingScreenshot {
    /// The output whose next frame is captured.
    pub output: *mut crate::output::Output,
    /// Crop in output-buffer pixels; `None` captures the whole output.
    pub region: Option<ffi::wlr_box>,
    pub path: PathBuf,
    /// The `ccectl` connection still waiting to hear how this went. The
    /// capture only runs on the next composited frame, so replying `ok
    /// <path>` at park time reported success before anything had been read
    /// back — an unsupported read format then left the user holding a path
    /// to a file that never appeared. Answered by [`Self::reply_ok`] /
    /// [`Self::reply_err`], or from `Drop` for the paths that discard a
    /// parked capture (failed commit, WM reset, output teardown).
    reply: Option<Sender<String>>,
}

impl PendingScreenshot {
    pub fn new(
        output: *mut crate::output::Output,
        region: Option<ffi::wlr_box>,
        path: PathBuf,
        reply: Option<Sender<String>>,
    ) -> Self {
        Self { output, region, path, reply }
    }

    /// Report the capture as landed. Sent once the pixels are in hand and the
    /// path is settled — the PNG write itself still happens on the encode
    /// thread and only logs, since holding the reply until a large output is
    /// compressed would push it past the IPC client's timeout.
    pub fn reply_ok(&mut self) {
        let msg = format!("ok {}\n", self.path.display());
        self.answer(msg);
    }

    pub fn reply_err(&mut self, msg: &str) {
        self.answer(format!("error: {msg}\n"));
    }

    fn answer(&mut self, msg: String) {
        if let Some(tx) = self.reply.take() {
            let _ = tx.send(msg);
        }
    }
}

impl Drop for PendingScreenshot {
    fn drop(&mut self) {
        // Anything that drops a parked capture without capturing is still an
        // outcome someone is blocked on; answer rather than let ccectl sit
        // out its timeout.
        self.answer("error: screenshot: capture dropped before a frame rendered\n".to_string());
    }
}

/// `~/Pictures/screenshots/screenshot-YYYYMMDD-HHMMSS-mmm.png` (the directory
/// is created by the encode thread).
pub fn default_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    let dir = PathBuf::from(home).join("Pictures").join("screenshots");
    unique_path(&dir, &timestamp_stem())
}

/// Millisecond-resolution stem. Seconds were not enough: `ccectl screenshot`
/// followed by `ccectl screenshot window` lands inside one second, and the
/// second capture silently overwrote the first.
fn timestamp_stem() -> String {
    let mut ts: libc::timespec = unsafe { std::mem::zeroed() };
    unsafe { libc::clock_gettime(libc::CLOCK_REALTIME, &mut ts) };
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    unsafe { libc::localtime_r(&ts.tv_sec, &mut tm) };
    format!(
        "screenshot-{:04}{:02}{:02}-{:02}{:02}{:02}-{:03}",
        tm.tm_year + 1900,
        tm.tm_mon + 1,
        tm.tm_mday,
        tm.tm_hour,
        tm.tm_min,
        tm.tm_sec,
        ts.tv_nsec / 1_000_000,
    )
}

/// `<stem>.png`, then `<stem>-2.png`, … until the name is free.
///
/// Checking the filesystem alone is not enough: the file is created by the
/// encode thread well after the path is handed out, so two captures in the
/// same millisecond would both see an empty directory. Hence the set of names
/// already issued this run (one `PathBuf` per capture, never reclaimed —
/// bounded by how many screenshots a session takes).
fn unique_path(dir: &Path, stem: &str) -> PathBuf {
    static ISSUED: std::sync::LazyLock<std::sync::Mutex<std::collections::HashSet<PathBuf>>> =
        std::sync::LazyLock::new(|| std::sync::Mutex::new(std::collections::HashSet::new()));
    let mut issued = ISSUED.lock().unwrap_or_else(|e| e.into_inner());
    let mut n = 1u32;
    loop {
        let path = dir.join(match n {
            1 => format!("{stem}.png"),
            n => format!("{stem}-{n}.png"),
        });
        if !issued.contains(&path) && !path.exists() {
            issued.insert(path.clone());
            return path;
        }
        n += 1;
    }
}

/// Bytes per pixel of the formats [`to_rgba`] can convert; `None` for
/// anything else, which is the one place that knowledge is written down.
fn bytes_per_pixel(format: u32) -> Option<usize> {
    match format {
        DRM_FORMAT_XRGB8888 | DRM_FORMAT_ARGB8888 | DRM_FORMAT_XBGR8888 | DRM_FORMAT_ABGR8888 => {
            Some(4)
        }
        DRM_FORMAT_BGR888 | DRM_FORMAT_RGB888 => Some(3),
        _ => None,
    }
}

/// Read a texture's full contents into a tightly packed `w*h*bpp` byte buffer.
/// Returns the bytes plus the DRM format they are in.
unsafe fn read_texture(texture: *mut ffi::wlr_texture, w: i32, h: i32) -> Option<(Vec<u8>, u32)> {
    if texture.is_null() || w <= 0 || h <= 0 {
        return None;
    }
    let format = ffi::wlr_texture_preferred_read_format(texture);
    // Both the buffer and the stride must be sized for the format the
    // renderer is about to write: a 24-bit format read into a 4-byte-strided
    // buffer would leave every row short and shifted.
    let Some(bpp) = bytes_per_pixel(format) else {
        log::warn!("screenshot: unsupported read format {format:#x}");
        return None;
    };
    let mut data = vec![0u8; (w as usize) * (h as usize) * bpp];
    let options = ffi::wlr_texture_read_pixels_options {
        data: data.as_mut_ptr() as *mut std::ffi::c_void,
        format,
        stride: (w as u32) * bpp as u32,
        dst_x: 0,
        dst_y: 0,
        src_box: std::mem::zeroed(), // empty = full texture
    };
    if !ffi::wlr_texture_read_pixels(texture, &options) {
        return None;
    }
    Some((data, format))
}

/// Read back only `src` (in buffer px) of a texture, into a `w`×`h` buffer.
///
/// The full-texture [`read_texture`] is fine for a screenshot, which wants
/// every pixel anyway; it is not fine for the backdrop sampler, which wants a
/// bar-height strip out of a window that may be 4K — 33MB copied per sample to
/// look at 0.3% of it.
pub(crate) unsafe fn read_texture_region(
    texture: *mut ffi::wlr_texture,
    src: ffi::wlr_box,
    w: i32,
    h: i32,
) -> Option<(Vec<u8>, u32)> {
    if texture.is_null() || w <= 0 || h <= 0 || src.width <= 0 || src.height <= 0 {
        return None;
    }
    let format = ffi::wlr_texture_preferred_read_format(texture);
    let bpp = bytes_per_pixel(format)?;
    let mut data = vec![0u8; (w as usize) * (h as usize) * bpp];
    let options = ffi::wlr_texture_read_pixels_options {
        data: data.as_mut_ptr() as *mut std::ffi::c_void,
        format,
        stride: (w as u32) * bpp as u32,
        dst_x: 0,
        dst_y: 0,
        src_box: src,
    };
    if !ffi::wlr_texture_read_pixels(texture, &options) {
        return None;
    }
    Some((data, format))
}

/// Convert read-back pixels to RGBA. Alpha is forced opaque — the X-variants
/// carry garbage alpha, the 24-bit formats carry none at all, and screenshots
/// should not be translucent.
pub(crate) fn to_rgba(mut pixels: Vec<u8>, format: u32) -> Option<Vec<u8>> {
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
        // 24-bit: widen rather than swizzle in place.
        DRM_FORMAT_BGR888 => Some(widen_24(&pixels, false)),
        DRM_FORMAT_RGB888 => Some(widen_24(&pixels, true)),
        _ => {
            log::warn!("screenshot: unsupported read format {format:#x}");
            None
        }
    }
}

/// 3-byte pixels to RGBA with opaque alpha. `swap_rb` covers RGB888, whose
/// memory order is B,G,R; BGR888 is already R,G,B. (Reversed from how the
/// names read — verified against a known-colour desktop on NVIDIA, which
/// hands back BGR888: assuming the intuitive order swapped every capture's
/// red and blue.)
fn widen_24(pixels: &[u8], swap_rb: bool) -> Vec<u8> {
    let mut out = Vec::with_capacity(pixels.len() / 3 * 4);
    for px in pixels.chunks_exact(3) {
        if swap_rb {
            out.extend_from_slice(&[px[2], px[1], px[0], 255]);
        } else {
            out.extend_from_slice(&[px[0], px[1], px[2], 255]);
        }
    }
    out
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
    mut shot: PendingScreenshot,
) {
    let texture = ffi::wlr_texture_from_buffer(renderer, buffer);
    if texture.is_null() {
        log::warn!("screenshot: wlr_texture_from_buffer failed");
        shot.reply_err("screenshot: wlr_texture_from_buffer failed");
        return;
    }
    let read = read_texture(texture, buf_w, buf_h);
    ffi::wlr_texture_destroy(texture);
    let Some((pixels, format)) = read else {
        log::warn!("screenshot: pixel readback failed");
        shot.reply_err("screenshot: pixel readback failed");
        return;
    };
    let Some(rgba) = to_rgba(pixels, format) else {
        shot.reply_err(&format!("screenshot: unsupported read format {format:#x}"));
        return;
    };
    let (rgba, out_w, out_h) = match shot.region {
        Some(region) => match crop_rgba(&rgba, buf_w, buf_h, region) {
            Some(cropped) => cropped,
            None => {
                log::warn!("screenshot: region outside the output");
                shot.reply_err("screenshot: region outside the output");
                return;
            }
        },
        None => (rgba, buf_w, buf_h),
    };
    shot.reply_ok();
    spawn_encode(rgba, out_w as u32, out_h as u32, shot.path.clone());
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
pub(crate) fn blit(dst: &mut [u8], dw: i32, dh: i32, src: &[u8], sw: i32, sh: i32, dx: i32, dy: i32) {
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

/// `notifications { screenshots <bool> }` in the shared config.kdl. Absent
/// *key* in a readable config means enabled — that is the documented default.
/// An unreadable config is a different thing: we know nothing about the
/// user's wishes, and a session running against a config we cannot read is
/// typically an isolated one (a headless shadow session, say) whose toasts
/// would land on someone else's screen. Stay quiet there.
fn notifications_enabled() -> bool {
    let Ok(content) = std::fs::read_to_string(cce_ui::config::get_config_path()) else {
        log::debug!("screenshot: config unreadable, staying quiet about the capture");
        return false;
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
        // 24-bit, two pixels. The names read backwards from the memory
        // order: BGR888 arrives R,G,B and widens as-is, RGB888 arrives
        // B,G,R and needs the swap. Both gain an alpha they never carried.
        let out = to_rgba(vec![1, 2, 3, 4, 5, 6], DRM_FORMAT_BGR888).unwrap();
        assert_eq!(out, vec![1, 2, 3, 255, 4, 5, 6, 255]);
        let out = to_rgba(vec![1, 2, 3, 4, 5, 6], DRM_FORMAT_RGB888).unwrap();
        assert_eq!(out, vec![3, 2, 1, 255, 6, 5, 4, 255]);
        assert!(to_rgba(vec![0; 4], 0x1234).is_none());
    }

    #[test]
    fn bytes_per_pixel_matches_what_to_rgba_accepts() {
        // The readback allocates and strides by this, so a format to_rgba
        // handles must have a size here and vice versa.
        for (format, bpp) in [
            (DRM_FORMAT_XRGB8888, 4usize),
            (DRM_FORMAT_ARGB8888, 4),
            (DRM_FORMAT_XBGR8888, 4),
            (DRM_FORMAT_ABGR8888, 4),
            (DRM_FORMAT_BGR888, 3),
            (DRM_FORMAT_RGB888, 3),
        ] {
            assert_eq!(bytes_per_pixel(format), Some(bpp), "{format:#x}");
            // One pixel's worth of bytes converts to exactly one RGBA pixel.
            assert_eq!(to_rgba(vec![0; bpp], format).map(|p| p.len()), Some(4));
        }
        assert_eq!(bytes_per_pixel(0x1234), None);
    }

    #[test]
    fn unique_path_never_reuses_a_name() {
        let dir = std::env::temp_dir().join(format!("cce-shot-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // Same stem twice: the second capture must not be handed the first
        // one's path, even though the encode thread has created no file yet.
        let a = unique_path(&dir, "screenshot-20260815-120000-000");
        let b = unique_path(&dir, "screenshot-20260815-120000-000");
        assert_ne!(a, b);
        assert!(a.ends_with("screenshot-20260815-120000-000.png"));
        assert!(b.ends_with("screenshot-20260815-120000-000-2.png"));
        // A name already on disk is skipped too (a stem reused across runs).
        std::fs::write(dir.join("screenshot-20260815-130000-000.png"), b"").unwrap();
        let c = unique_path(&dir, "screenshot-20260815-130000-000");
        assert!(c.ends_with("screenshot-20260815-130000-000-2.png"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn pending_screenshot_answers_exactly_once() {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut shot = PendingScreenshot::new(
            std::ptr::null_mut(),
            None,
            PathBuf::from("/tmp/shot.png"),
            Some(tx),
        );
        shot.reply_ok();
        assert_eq!(rx.recv().unwrap(), "ok /tmp/shot.png\n");
        drop(shot); // already answered: Drop must not send a second verdict
        assert!(rx.recv().is_err());

        // A capture discarded before it ran answers from Drop, so ccectl
        // hears an error instead of sitting out its timeout.
        let (tx, rx) = std::sync::mpsc::channel();
        drop(PendingScreenshot::new(
            std::ptr::null_mut(),
            None,
            PathBuf::from("/tmp/shot.png"),
            Some(tx),
        ));
        assert!(rx.recv().unwrap().starts_with("error: "));
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
