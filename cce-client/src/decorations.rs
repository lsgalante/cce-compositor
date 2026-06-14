// Wayland decoration surface drawing and management for cce-client

use std::ffi::CString;
use std::os::fd::RawFd;
use std::ptr;
use wayland_client::protocol::{wl_buffer, wl_shm, wl_shm_pool, wl_surface};
use wayland_client::QueueHandle;

use crate::protocol::river_window_management::client::river_decoration_v1::RiverDecorationV1;
use crate::wayland::AppState;

fn resolve_window_border_font_path() -> Option<String> {
    use std::io::Read;
    let mut child = std::process::Command::new("fc-match")
        .args(&["-f", "%{file}", "window\\-borders"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;

    let mut stdout = child.stdout.take()?;
    let mut output_str = String::new();
    let _ = stdout.read_to_string(&mut output_str);
    let _ = child.wait();

    let path = output_str.trim().to_string();
    if !path.is_empty() && std::path::Path::new(&path).exists() {
        return Some(path);
    }
    None
}

/// Struct tracking the Wayland decoration resources for a window.
pub struct WindowDecoration {
    pub decoration: RiverDecorationV1,
    pub surface: wl_surface::WlSurface,
    pub buffer: Option<wl_buffer::WlBuffer>,
    pub pool: Option<wl_shm_pool::WlShmPool>,
    pub width: i32,
    pub height: i32,
    pub mapped_data: *mut u32,
    pub mapped_size: usize,
}

impl Drop for WindowDecoration {
    fn drop(&mut self) {
        if !self.mapped_data.is_null() {
            unsafe {
                libc::munmap(self.mapped_data as *mut libc::c_void, self.mapped_size);
            }
        }
    }
}

// Standard 8x8 bitmap font (IBM CP437 ASCII printable characters 32..=126)
const FONT_DATA: &[u8; 760] = &[
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, //   (0x20)
    0x18, 0x18, 0x18, 0x18, 0x00, 0x00, 0x18, 0x00, // ! (0x21)
    0x24, 0x24, 0x24, 0x00, 0x00, 0x00, 0x00, 0x00, // " (0x22)
    0x24, 0x24, 0x7e, 0x24, 0x7e, 0x24, 0x24, 0x00, // # (0x23)
    0x08, 0x3e, 0x28, 0x3e, 0x0a, 0x3e, 0x08, 0x00, // $ (0x24)
    0x00, 0x62, 0x64, 0x08, 0x13, 0x23, 0x00, 0x00, // % (0x25)
    0x18, 0x24, 0x18, 0x2a, 0x24, 0x24, 0x1b, 0x00, // & (0x26)
    0x18, 0x18, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // ' (0x27)
    0x08, 0x10, 0x20, 0x20, 0x20, 0x10, 0x08, 0x00, // ( (0x28)
    0x10, 0x08, 0x04, 0x04, 0x04, 0x08, 0x10, 0x00, // ) (0x29)
    0x00, 0x24, 0x18, 0x3c, 0x18, 0x24, 0x00, 0x00, // * (0x2a)
    0x00, 0x08, 0x08, 0x3e, 0x08, 0x08, 0x00, 0x00, // + (0x2b)
    0x00, 0x00, 0x00, 0x00, 0x00, 0x18, 0x18, 0x10, // , (0x2c)
    0x00, 0x00, 0x00, 0x3c, 0x00, 0x00, 0x00, 0x00, // - (0x2d)
    0x00, 0x00, 0x00, 0x00, 0x00, 0x18, 0x18, 0x00, // . (0x2e)
    0x00, 0x02, 0x04, 0x08, 0x10, 0x20, 0x40, 0x00, // / (0x2f)
    0x3c, 0x46, 0x4a, 0x52, 0x62, 0x3c, 0x00, 0x00, // 0 (0x30)
    0x18, 0x28, 0x08, 0x08, 0x08, 0x3e, 0x00, 0x00, // 1 (0x31)
    0x3c, 0x42, 0x02, 0x3c, 0x40, 0x7e, 0x00, 0x00, // 2 (0x32)
    0x3c, 0x42, 0x1c, 0x02, 0x42, 0x3c, 0x00, 0x00, // 3 (0x33)
    0x08, 0x18, 0x28, 0x48, 0x7e, 0x08, 0x00, 0x00, // 4 (0x34)
    0x7e, 0x40, 0x7c, 0x02, 0x42, 0x3c, 0x00, 0x00, // 5 (0x35)
    0x3c, 0x40, 0x7c, 0x42, 0x42, 0x3c, 0x00, 0x00, // 6 (0x36)
    0x7e, 0x02, 0x04, 0x08, 0x10, 0x10, 0x00, 0x00, // 7 (0x37)
    0x3c, 0x42, 0x3c, 0x42, 0x42, 0x3c, 0x00, 0x00, // 8 (0x38)
    0x3c, 0x42, 0x3e, 0x02, 0x02, 0x3c, 0x00, 0x00, // 9 (0x39)
    0x00, 0x18, 0x18, 0x00, 0x18, 0x18, 0x00, 0x00, // : (0x3a)
    0x00, 0x18, 0x18, 0x00, 0x18, 0x18, 0x10, 0x00, // ; (0x3b)
    0x04, 0x08, 0x10, 0x20, 0x10, 0x08, 0x04, 0x00, // < (0x3c)
    0x00, 0x00, 0x3e, 0x00, 0x3e, 0x00, 0x00, 0x00, // = (0x3d)
    0x20, 0x10, 0x08, 0x04, 0x08, 0x10, 0x20, 0x00, // > (0x3e)
    0x3c, 0x42, 0x02, 0x0c, 0x00, 0x08, 0x08, 0x00, // ? (0x3f)
    0x3c, 0x42, 0x5a, 0x5a, 0x5a, 0x3e, 0x00, 0x00, // @ (0x40)
    0x18, 0x24, 0x42, 0x7e, 0x42, 0x42, 0x00, 0x00, // A (0x41)
    0x7c, 0x42, 0x7c, 0x42, 0x42, 0x7c, 0x00, 0x00, // B (0x42)
    0x3c, 0x42, 0x40, 0x40, 0x42, 0x3c, 0x00, 0x00, // C (0x43)
    0x78, 0x44, 0x42, 0x42, 0x44, 0x78, 0x00, 0x00, // D (0x44)
    0x7e, 0x40, 0x7c, 0x40, 0x40, 0x7e, 0x00, 0x00, // E (0x45)
    0x7e, 0x40, 0x7c, 0x40, 0x40, 0x40, 0x00, 0x00, // F (0x46)
    0x3c, 0x42, 0x40, 0x4e, 0x42, 0x3c, 0x00, 0x00, // G (0x47)
    0x42, 0x42, 0x7e, 0x42, 0x42, 0x42, 0x00, 0x00, // H (0x48)
    0x3e, 0x08, 0x08, 0x08, 0x08, 0x3e, 0x00, 0x00, // I (0x49)
    0x02, 0x02, 0x02, 0x02, 0x42, 0x3c, 0x00, 0x00, // J (0x4a)
    0x44, 0x48, 0x70, 0x48, 0x44, 0x42, 0x00, 0x00, // K (0x4b)
    0x40, 0x40, 0x40, 0x40, 0x40, 0x7e, 0x00, 0x00, // L (0x4c)
    0x42, 0x66, 0x5a, 0x42, 0x42, 0x42, 0x00, 0x00, // M (0x4d)
    0x42, 0x62, 0x52, 0x4a, 0x46, 0x42, 0x00, 0x00, // N (0x4e)
    0x3c, 0x42, 0x42, 0x42, 0x42, 0x3c, 0x00, 0x00, // O (0x4f)
    0x7c, 0x42, 0x7c, 0x40, 0x40, 0x40, 0x00, 0x00, // P (0x50)
    0x3c, 0x42, 0x42, 0x42, 0x4a, 0x3c, 0x02, 0x00, // Q (0x51)
    0x7c, 0x42, 0x7c, 0x48, 0x44, 0x42, 0x00, 0x00, // R (0x52)
    0x3c, 0x40, 0x3c, 0x02, 0x02, 0x3c, 0x00, 0x00, // S (0x53)
    0x7e, 0x08, 0x08, 0x08, 0x08, 0x08, 0x00, 0x00, // T (0x54)
    0x42, 0x42, 0x42, 0x42, 0x42, 0x3c, 0x00, 0x00, // U (0x55)
    0x42, 0x42, 0x42, 0x24, 0x24, 0x18, 0x00, 0x00, // V (0x56)
    0x42, 0x42, 0x42, 0x5a, 0x5a, 0x24, 0x00, 0x00, // W (0x57)
    0x42, 0x24, 0x18, 0x18, 0x24, 0x42, 0x00, 0x00, // X (0x58)
    0x42, 0x42, 0x24, 0x18, 0x08, 0x08, 0x00, 0x00, // Y (0x59)
    0x7e, 0x02, 0x04, 0x08, 0x10, 0x7e, 0x00, 0x00, // Z (0x5a)
    0x3c, 0x20, 0x20, 0x20, 0x20, 0x3c, 0x00, 0x00, // [ (0x5b)
    0x00, 0x40, 0x20, 0x10, 0x08, 0x04, 0x02, 0x00, // \ (0x5c)
    0x3c, 0x02, 0x02, 0x02, 0x02, 0x3c, 0x00, 0x00, // ] (0x5d)
    0x08, 0x14, 0x22, 0x00, 0x00, 0x00, 0x00, 0x00, // ^ (0x5e)
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x7e, // _ (0x5f)
    0x08, 0x08, 0x04, 0x00, 0x00, 0x00, 0x00, 0x00, // ` (0x60)
    0x00, 0x3c, 0x02, 0x3e, 0x42, 0x3e, 0x00, 0x00, // a (0x61)
    0x40, 0x40, 0x7c, 0x42, 0x42, 0x7c, 0x00, 0x00, // b (0x62)
    0x00, 0x3c, 0x40, 0x40, 0x42, 0x3c, 0x00, 0x00, // c (0x63)
    0x02, 0x02, 0x3e, 0x42, 0x42, 0x3e, 0x00, 0x00, // d (0x64)
    0x00, 0x3c, 0x42, 0x7e, 0x40, 0x3c, 0x00, 0x00, // e (0x65)
    0x1c, 0x20, 0x78, 0x20, 0x20, 0x20, 0x00, 0x00, // f (0x66)
    0x00, 0x3e, 0x42, 0x3e, 0x02, 0x3c, 0x00, 0x00, // g (0x67)
    0x40, 0x40, 0x7c, 0x42, 0x42, 0x42, 0x00, 0x00, // h (0x68)
    0x08, 0x00, 0x08, 0x08, 0x08, 0x08, 0x00, 0x00, // i (0x69)
    0x02, 0x00, 0x02, 0x02, 0x02, 0x42, 0x3c, 0x00, // j (0x6a)
    0x40, 0x44, 0x48, 0x70, 0x48, 0x44, 0x00, 0x00, // k (0x6b)
    0x18, 0x08, 0x08, 0x08, 0x08, 0x3e, 0x00, 0x00, // l (0x6c)
    0x00, 0x66, 0x5a, 0x42, 0x42, 0x42, 0x00, 0x00, // m (0x6d)
    0x00, 0x7c, 0x42, 0x42, 0x42, 0x42, 0x00, 0x00, // n (0x6e)
    0x00, 0x3c, 0x42, 0x42, 0x42, 0x3c, 0x00, 0x00, // o (0x6f)
    0x00, 0x7c, 0x42, 0x7c, 0x40, 0x40, 0x00, 0x00, // p (0x70)
    0x00, 0x3e, 0x42, 0x3e, 0x02, 0x02, 0x00, 0x00, // q (0x71)
    0x00, 0x7c, 0x40, 0x40, 0x40, 0x40, 0x00, 0x00, // r (0x72)
    0x00, 0x3e, 0x40, 0x3c, 0x02, 0x3c, 0x00, 0x00, // s (0x73)
    0x08, 0x08, 0x3e, 0x08, 0x08, 0x0a, 0x04, 0x00, // t (0x74)
    0x00, 0x42, 0x42, 0x42, 0x42, 0x3c, 0x00, 0x00, // u (0x75)
    0x00, 0x42, 0x42, 0x42, 0x24, 0x18, 0x00, 0x00, // v (0x76)
    0x00, 0x42, 0x42, 0x5a, 0x5a, 0x24, 0x00, 0x00, // w (0x77)
    0x00, 0x42, 0x24, 0x18, 0x24, 0x42, 0x00, 0x00, // x (0x78)
    0x00, 0x42, 0x42, 0x3e, 0x02, 0x3c, 0x00, 0x00, // y (0x79)
    0x00, 0x7e, 0x04, 0x08, 0x10, 0x7e, 0x00, 0x00, // z (0x7a)
    0x0c, 0x10, 0x10, 0x20, 0x10, 0x10, 0x0c, 0x00, // { (0x7b)
    0x08, 0x08, 0x08, 0x08, 0x08, 0x08, 0x08, 0x00, // | (0x7c)
    0x30, 0x08, 0x08, 0x04, 0x08, 0x08, 0x30, 0x00, // } (0x7d)
    0x00, 0x00, 0x00, 0x14, 0x28, 0x00, 0x00, 0x00, // ~ (0x7e)
];

/// Helper to draw a character at (x, y) with a given scale and BGRA color.
fn draw_char(
    buf: &mut [u32],
    buf_w: i32,
    buf_h: i32,
    c: char,
    x: i32,
    y: i32,
    scale: i32,
    color: u32,
) {
    let code = (c as usize).saturating_sub(32);
    if code >= 95 {
        return;
    }
    let glyph = &FONT_DATA[code * 8..(code + 1) * 8];
    for row in 0..8 {
        let row_byte = glyph[row];
        for col in 0..8 {
            let bit = (row_byte >> (7 - col)) & 1;
            if bit != 0 {
                for dy in 0..scale {
                    for dx in 0..scale {
                        let px = x + (col as i32) * scale + dx;
                        let py = y + (row as i32) * scale + dy;
                        if px >= 0 && px < buf_w && py >= 0 && py < buf_h {
                            buf[(py * buf_w + px) as usize] = color;
                        }
                    }
                }
            }
        }
    }
}

/// Create a temporary shared memory file descriptor.
fn create_memfd(size: usize) -> Option<RawFd> {
    let name = CString::new("cce-client-decoration").ok()?;
    let fd = unsafe { libc::memfd_create(name.as_ptr(), libc::MFD_CLOEXEC) };
    if fd < 0 {
        return None;
    }
    if unsafe { libc::ftruncate(fd, size as libc::off_t) } < 0 {
        unsafe { libc::close(fd); }
        return None;
    }
    Some(fd)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CornerType {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

fn round_corner(
    buffer_slice: &mut [u32],
    dec_width: i32,
    dec_height: i32,
    cx: i32,
    cy: i32,
    r: i32,
    corner: CornerType,
) {
    if r <= 0 {
        return;
    }
    let r_f = r as f32;

    let (x_range, y_range) = match corner {
        CornerType::TopLeft => (0..cx, 0..cy),
        CornerType::TopRight => (cx..dec_width, 0..cy),
        CornerType::BottomLeft => (0..cx, cy..dec_height),
        CornerType::BottomRight => (cx..dec_width, cy..dec_height),
    };

    for py in y_range {
        if py < 0 || py >= dec_height {
            continue;
        }
        let row_offset = (py * dec_width) as usize;
        for px in x_range.clone() {
            if px < 0 || px >= dec_width {
                continue;
            }

            let dx = (px - cx) as f32;
            let dy = (py - cy) as f32;
            let dist = (dx * dx + dy * dy).sqrt();

            if dist > r_f + 0.5 {
                buffer_slice[row_offset + px as usize] = 0x00000000;
            } else if dist > r_f - 0.5 {
                let alpha_scale = r_f + 0.5 - dist; // value between 0.0 and 1.0
                let index = row_offset + px as usize;
                let pixel = buffer_slice[index];
                let a = ((pixel >> 24) & 0xFF) as f32 * alpha_scale;
                let r_val = ((pixel >> 16) & 0xFF) as f32 * alpha_scale;
                let g = ((pixel >> 8) & 0xFF) as f32 * alpha_scale;
                let b = (pixel & 0xFF) as f32 * alpha_scale;

                buffer_slice[index] = ((a as u32) << 24)
                    | ((r_val as u32) << 16)
                    | ((g as u32) << 8)
                    | (b as u32);
            }
        }
    }
}

fn update_border_decoration(
    _wid: u64,
    dec_opt: &mut Option<WindowDecoration>,
    compositor: &wayland_client::protocol::wl_compositor::WlCompositor,
    shm: &wayland_client::protocol::wl_shm::WlShm,
    river_window: &crate::protocol::river_window_management::client::river_window_v1::RiverWindowV1,
    qhandle: &QueueHandle<AppState>,
    offset_x: i32,
    offset_y: i32,
    logical_width: i32,
    logical_height: i32,
    scale: i32,
    border_blur: bool,
    bg_color: u32,
    rect_x: i32,
    rect_y: i32,
    rect_w: i32,
    rect_h: i32,
    border_width: i32,
    round_bottom: bool,
    side: &str,
    hover_info: Option<(f64, f64)>,
) {
    let dec_width = logical_width * scale;
    let dec_height = logical_height * scale;

    let needs_new_buffer = match dec_opt {
        Some(dec) => dec.width != dec_width || dec.height != dec_height,
        None => true,
    };

    if dec_opt.is_none() {
        let surface: wl_surface::WlSurface = compositor.create_surface(qhandle, ());
        let decoration: RiverDecorationV1 = river_window.get_decoration_above(&surface, qhandle, ());

        *dec_opt = Some(WindowDecoration {
            decoration,
            surface,
            buffer: None,
            pool: None,
            width: 0,
            height: 0,
            mapped_data: ptr::null_mut(),
            mapped_size: 0,
        });
    }

    let dec = dec_opt.as_mut().unwrap();
    dec.decoration.set_offset(offset_x, offset_y);
    dec.decoration.set_blur(if border_blur { 1 } else { 0 });

    if needs_new_buffer {
        if !dec.mapped_data.is_null() {
            unsafe {
                libc::munmap(dec.mapped_data as *mut libc::c_void, dec.mapped_size);
            }
            dec.mapped_data = ptr::null_mut();
        }

        let stride = dec_width * 4;
        let size = (stride * dec_height) as usize;

        if let Some(fd) = create_memfd(size) {
            let mapped_data = unsafe {
                libc::mmap(
                    ptr::null_mut(),
                    size,
                    libc::PROT_READ | libc::PROT_WRITE,
                    libc::MAP_SHARED,
                    fd,
                    0,
                ) as *mut u32
            };

            if mapped_data != libc::MAP_FAILED as *mut u32 {
                let borrowed_fd = unsafe { std::os::fd::BorrowedFd::borrow_raw(fd) };
                let pool = shm.create_pool(borrowed_fd, size as i32, qhandle, ());
                let buffer = pool.create_buffer(
                    0,
                    dec_width,
                    dec_height,
                    stride,
                    wl_shm::Format::Argb8888,
                    qhandle,
                    (),
                );

                dec.pool = Some(pool);
                dec.buffer = Some(buffer);
                dec.width = dec_width;
                dec.height = dec_height;
                dec.mapped_data = mapped_data;
                dec.mapped_size = size;
                dec.surface.set_buffer_scale(scale);
            } else {
                eprintln!("[decorations] failed to mmap border decoration buffer");
                unsafe { libc::close(fd); }
                return;
            }
            unsafe { libc::close(fd); }
        } else {
            eprintln!("[decorations] failed to create memfd for border decoration");
            return;
        }
    }

    if let Some(ref buffer) = dec.buffer {
        if !dec.mapped_data.is_null() {
            let buffer_slice = unsafe {
                std::slice::from_raw_parts_mut(dec.mapped_data, (dec_width * dec_height) as usize)
            };

            // Clear to fully transparent
            for pixel in buffer_slice.iter_mut() {
                *pixel = 0x00000000;
            }

            let mut highlight_region: Option<HighlightRegion> = None;
            if let Some((hx, hy)) = hover_info {
                const CORNER_THRESHOLD: f64 = 16.0;
                match side {
                    "left" => {
                        let mid_x = (logical_width as f64) / 2.0;
                        if hx < mid_x {
                            if hy < CORNER_THRESHOLD {
                                highlight_region = Some(HighlightRegion::TopLeft);
                            } else if hy > (logical_height as f64 - CORNER_THRESHOLD) {
                                highlight_region = Some(HighlightRegion::BottomLeft);
                            } else {
                                highlight_region = Some(HighlightRegion::Left);
                            }
                        }
                    }
                    "right" => {
                        let mid_x = (logical_width as f64) / 2.0;
                        if hx >= mid_x {
                            if hy < CORNER_THRESHOLD {
                                highlight_region = Some(HighlightRegion::TopRight);
                            } else if hy > (logical_height as f64 - CORNER_THRESHOLD) {
                                highlight_region = Some(HighlightRegion::BottomRight);
                            } else {
                                highlight_region = Some(HighlightRegion::Right);
                            }
                        }
                    }
                    "bottom" => {
                        let mid_y = (logical_height as f64) / 2.0;
                        if hy >= mid_y {
                            if hx < CORNER_THRESHOLD {
                                highlight_region = Some(HighlightRegion::BottomLeft);
                            } else if hx > (logical_width as f64 - CORNER_THRESHOLD) {
                                highlight_region = Some(HighlightRegion::BottomRight);
                            } else {
                                highlight_region = Some(HighlightRegion::Bottom);
                            }
                        }
                    }
                    _ => {}
                }
            }

            // Draw visual border color with 3D cylindrical shading and mitred joints
            let rx_start = rect_x * scale;
            let ry_start = rect_y * scale;
            let rx_end = (rect_x + rect_w) * scale;
            let ry_end = (rect_y + rect_h) * scale;

            let border_w_scaled = border_width * scale;
            let win_w_scaled = (rect_w - 2 * border_width) * scale;
            let grab_w_scaled = logical_height * scale;

            for py in ry_start..ry_end {
                if py >= 0 && py < dec_height {
                    let row_offset = (py * dec_width) as usize;
                    for px in rx_start..rx_end {
                        if px >= 0 && px < dec_width {
                            let s = if rect_w > rect_h {
                                // Horizontal border (bottom)
                                let logical_px = px - grab_w_scaled;
                                let logical_py = py;

                                if logical_px < 0 && logical_py < -logical_px {
                                    // Left border region (outer is at t = 0)
                                    let t_val = ((logical_px + border_w_scaled) as f32) / (border_w_scaled as f32 - 1.0).max(1.0);
                                    if t_val < 0.5 {
                                        1.0 + 0.45 * (1.0 - 2.0 * t_val) * (1.0 - 2.0 * t_val)
                                    } else {
                                        1.0
                                    }
                                } else if logical_px > win_w_scaled && logical_py < (logical_px - win_w_scaled) {
                                    // Right border region (outer is at t = 0)
                                    let t_val = 1.0 - ((logical_px - win_w_scaled) as f32) / (border_w_scaled as f32 - 1.0).max(1.0);
                                    if t_val < 0.5 {
                                        1.0 - 0.50 * (1.0 - 2.0 * t_val) * (1.0 - 2.0 * t_val)
                                    } else {
                                        1.0
                                    }
                                } else {
                                    // Bottom border region (outer is at t = 0)
                                    let t_val = 1.0 - (logical_py as f32) / (border_w_scaled as f32 - 1.0).max(1.0);
                                    if t_val < 0.5 {
                                        1.0 - 0.50 * (1.0 - 2.0 * t_val) * (1.0 - 2.0 * t_val)
                                    } else {
                                        1.0
                                    }
                                }
                            } else {
                                // Vertical border (left/right)
                                let t_val = ((px - rx_start) as f32) / (border_w_scaled as f32 - 1.0).max(1.0);
                                if offset_x < 0 {
                                    // Left border: outer is at t = 0 (highlight)
                                    if t_val < 0.5 {
                                        1.0 + 0.45 * (1.0 - 2.0 * t_val) * (1.0 - 2.0 * t_val)
                                    } else {
                                        1.0
                                    }
                                } else {
                                    // Right border: outer is at t = 1 (shadow)
                                    if t_val > 0.5 {
                                        1.0 - 0.50 * (2.0 * t_val - 1.0) * (2.0 * t_val - 1.0)
                                    } else {
                                        1.0
                                    }
                                }
                            };

                            let a = ((bg_color >> 24) & 0xFF) as f32;
                            let mut r_val = (((bg_color >> 16) & 0xFF) as f32 * s).round().min(255.0) as u32;
                            let mut g = (((bg_color >> 8) & 0xFF) as f32 * s).round().min(255.0) as u32;
                            let mut b = ((bg_color & 0xFF) as f32 * s).round().min(255.0) as u32;

                            if let Some(region) = highlight_region {
                                if is_pixel_in_highlight_region(side, region, px, py, logical_width, logical_height, scale) {
                                    r_val = (r_val as f32 * 0.6 + 255.0 * 0.4).round() as u32;
                                    g = (g as f32 * 0.6 + 255.0 * 0.4).round() as u32;
                                    b = (b as f32 * 0.6 + 255.0 * 0.4).round() as u32;
                                }
                            }

                            buffer_slice[row_offset + px as usize] = ((a as u32) << 24)
                                | (r_val << 16)
                                | (g << 8)
                                | b;
                        }
                    }
                }
            }

            if round_bottom {
                let r_val = (16.min(border_width)) * scale;
                let cy = ry_end - r_val;
                
                // Bottom Left
                let cx_l = rx_start + r_val;
                round_corner(buffer_slice, dec_width, dec_height, cx_l, cy, r_val, CornerType::BottomLeft);

                // Bottom Right
                let cx_r = rx_end - r_val;
                round_corner(buffer_slice, dec_width, dec_height, cx_r, cy, r_val, CornerType::BottomRight);
            }
        }

        dec.surface.attach(Some(buffer), 0, 0);
        dec.surface.damage(0, 0, dec_width, dec_height);
        dec.surface.commit();
    }
}

/// Main entry point to create or update decoration surfaces during rendering.
pub fn update_decorations(state: &mut AppState, qhandle: &QueueHandle<AppState>) {
    // Check if we need to load or reload the font
    let current_path = resolve_window_border_font_path();
    if current_path != state.border_font_path {
        state.border_font_path = current_path.clone();
        state.border_font = current_path
            .and_then(|path| std::fs::read(&path).ok())
            .and_then(|data| fontdue::Font::from_bytes(data, fontdue::FontSettings::default()).ok());
        if state.border_font.is_some() {
            eprintln!("[decorations] loaded border font: {:?}", state.border_font_path);
        } else {
            eprintln!("[decorations] failed to load border font, falling back to built-in bitmap font");
        }
    }

    let active_tags = state.wm.active_tags;

    // We can only create decoration surfaces if the compositor and shm globals are bound
    let (compositor, shm) = match (&state.compositor, &state.shm) {
        (Some(c), Some(s)) => (c, s),
        _ => return,
    };

    // Clean up decorations for windows that should not be decorated,
    // or side borders for windows that do not have them (Popup, Fullscreen).
    for (wid, wp) in &mut state.window_proxies {
        if let Some(w) = state.wm.windows.iter().find(|win| win.id == *wid) {
            let is_minimized = w.minimized;
            let should_not_decorate = w.closed
                || w.app_id.as_deref() == Some("cce-status-interface")
                || w.app_id.as_deref().map_or(false, |aid| aid.contains("noborder"))
                || w.tiling_mode == crate::types::TilingMode::Popup
                || w.tiling_mode == crate::types::TilingMode::Fullscreen
                || w.circular;
            let needs_sides = !should_not_decorate && !is_minimized;
            // eprintln!("[decorations] window {} minimized={} needs_sides={} dec_right_exists={}", w.id, is_minimized, needs_sides, wp.dec_right.is_some());

            if should_not_decorate {
                if let Some(dec) = wp.decoration.take() {
                    dec.decoration.destroy();
                    dec.surface.destroy();
                }
            }

            if !needs_sides {
                if let Some(dec) = wp.dec_left.take() {
                    dec.decoration.destroy();
                    dec.surface.destroy();
                }
                if let Some(dec) = wp.dec_right.take() {
                    dec.decoration.destroy();
                    dec.surface.destroy();
                }
                if let Some(dec) = wp.dec_bottom.take() {
                    dec.decoration.destroy();
                    dec.surface.destroy();
                }
            }
        }
    }

    struct DecorateInfo {
        id: u64,
        win_width: i32,
        win_height: i32,
        border_width: i32,
        title: String,
        bg_color: u32,
        tiling_mode: crate::types::TilingMode,
        is_minimized: bool,
        minimized_idx: Option<usize>,
        win_y: i32,
    }

    // Calculate dynamic border colors
    let border_colors = crate::borders::compute_border_colors(&state.wm);
    let text_color = 0xFFE0E0E0u32;

    // Collect window IDs to modify so we don't violate the borrow checker
    let windows_to_decorate: Vec<DecorateInfo> = state
        .wm
        .windows
        .iter()
        .enumerate()
        .filter(|(_, w)| {
            !w.closed && w.app_id.as_deref() != Some("cce-status-interface") && !w.circular && !w.app_id.as_deref().map_or(false, |aid| aid.contains("noborder")) && (w.minimized || (w.tiling_mode != crate::types::TilingMode::Popup && w.tiling_mode != crate::types::TilingMode::Fullscreen))
        })
        .filter(|(_, w)| (w.tags & active_tags) != 0)
        .map(|(idx, w)| {
            let is_minimized = w.minimized;
            let minimized_idx = if is_minimized {
                state.wm.windows
                    .iter()
                    .filter(|win| !win.closed && win.minimized && (win.tags & active_tags) != 0 && win.app_id.as_deref() != Some("cce-status-interface"))
                    .position(|win| win.id == w.id)
            } else {
                None
            };
            let title = w.title.clone().unwrap_or_else(|| {
                w.app_id.clone().unwrap_or_else(|| "Window".to_string())
            });
            let mode_idx = if state.wm.expose_visual_active && w.tiling_mode != crate::types::TilingMode::Popup {
                let list: Vec<_> = state.wm.windows
                    .iter()
                    .filter(|win| !win.closed && win.app_id.as_deref() != Some("cce-status-interface") && win.tiling_mode != crate::types::TilingMode::Popup && (win.tags & active_tags) != 0)
                    .collect();
                let len = list.len();
                let pos = list.iter().position(|win| win.id == w.id).unwrap_or(0);
                if len > 0 { len - 1 - pos } else { 0 }
            } else {
                let list: Vec<_> = state.wm.windows
                    .iter()
                    .filter(|win| !win.closed && win.app_id.as_deref() != Some("cce-status-interface") && (win.tags & active_tags) != 0 && win.tiling_mode == w.tiling_mode)
                    .collect();
                let len = list.len();
                let pos = list.iter().position(|win| win.id == w.id).unwrap_or(0);
                if len > 0 { len - 1 - pos } else { 0 }
            };
            let indicator = if is_minimized {
                "M"
            } else if state.wm.expose_visual_active && w.tiling_mode != crate::types::TilingMode::Popup {
                "EX"
            } else {
                match w.tiling_mode {
                    crate::types::TilingMode::Floating => "F",
                    crate::types::TilingMode::Cascade => "C",
                    crate::types::TilingMode::Grid => "G",
                    crate::types::TilingMode::Fullscreen => "S",
                    crate::types::TilingMode::Popup => "P",
                    crate::types::TilingMode::SidePanel => "SP",
                }
            };
            let title_with_idx = if is_minimized {
                format!("[{}] {}", indicator, title)
            } else {
                format!("[{}{}] {}", indicator, mode_idx, title)
            };

            // Find matching computed border color for this window
            let bc = border_colors.iter().find(|b| b.window_idx == idx);
            let (r, g, b, a) = match bc {
                Some(b) => (b.r, b.g, b.b, b.a),
                None => (
                    state.wm.layout.border_r,
                    state.wm.layout.border_g,
                    state.wm.layout.border_b,
                    state.wm.layout.border_a,
                ),
            };

            let border_r = (r & 0xFF) as u8;
            let border_g = (g & 0xFF) as u8;
            let border_b = (b & 0xFF) as u8;
            let border_a = (a & 0xFF) as u8;

            let r_premult = ((border_r as u32 * border_a as u32) / 255) as u8;
            let g_premult = ((border_g as u32 * border_a as u32) / 255) as u8;
            let b_premult = ((border_b as u32 * border_a as u32) / 255) as u8;

            let bg_color = ((border_a as u32) << 24)
                | ((r_premult as u32) << 16)
                | ((g_premult as u32) << 8)
                | (b_premult as u32);

            // Border width is mode-specific
            let border_width = if is_minimized {
                state.wm.layout.grid_border_width
            } else if state.wm.expose_visual_active && w.tiling_mode != crate::types::TilingMode::Popup {
                state.wm.layout.grid_border_width
            } else {
                match w.tiling_mode {
                    crate::types::TilingMode::Cascade => state.wm.layout.cascade_border_width,
                    crate::types::TilingMode::Fullscreen => state.wm.layout.fullscreen_border_width,
                    crate::types::TilingMode::Grid => state.wm.layout.grid_border_width,
                    crate::types::TilingMode::Floating => state.wm.layout.floating_border_width,
                    crate::types::TilingMode::Popup => 0,
                    crate::types::TilingMode::SidePanel => state.wm.layout.cascade_border_width,
                }
            };
            DecorateInfo {
                id: w.id,
                win_width: if w.committed_width > 0 { w.committed_width } else { w.width },
                win_height: if w.committed_height > 0 { w.committed_height } else { w.height },
                border_width,
                title: title_with_idx,
                bg_color,
                tiling_mode: w.tiling_mode,
                is_minimized,
                minimized_idx,
                win_y: w.y,
            }
        })
        .collect();

    for info in windows_to_decorate {
        let wid = info.id;
        let win_width = info.win_width;
        let win_height = info.win_height;
        let border_width = info.border_width;
        let title = info.title;
        let bg_color = info.bg_color;
        let tiling_mode = info.tiling_mode;
        let is_minimized = info.is_minimized;
        let minimized_idx = info.minimized_idx;
        let win_y = info.win_y;

        // Find or create decoration proxy
        let wp_idx = match state.window_proxies.iter().position(|(id, _)| *id == wid) {
            Some(idx) => idx,
            None => continue,
        };

        let wp = &mut state.window_proxies[wp_idx].1;

        let scale = (state.wm.output_scale.round() as i32).max(1);

        if tiling_mode != crate::types::TilingMode::Popup && tiling_mode != crate::types::TilingMode::Fullscreen && !is_minimized {
            let grab_w = if tiling_mode == crate::types::TilingMode::Floating {
                border_width.max(10)
            } else {
                border_width.max(1)
            };

            let hover_left = state.pointer_hovered_surface.as_ref()
                .filter(|surf| wp.dec_left.as_ref().map_or(false, |d| &d.surface == *surf))
                .map(|_| (state.last_pointer_surface_x, state.last_pointer_surface_y));

            update_border_decoration(
                wid,
                &mut wp.dec_left,
                compositor,
                shm,
                &wp.river_window,
                qhandle,
                -grab_w,
                0,
                grab_w,
                win_height,
                scale,
                state.wm.layout.border_blur,
                bg_color,
                grab_w - border_width,
                0,
                border_width,
                win_height,
                border_width,
                false,
                "left",
                hover_left,
            );

            let hover_right = state.pointer_hovered_surface.as_ref()
                .filter(|surf| wp.dec_right.as_ref().map_or(false, |d| &d.surface == *surf))
                .map(|_| (state.last_pointer_surface_x, state.last_pointer_surface_y));

            update_border_decoration(
                wid,
                &mut wp.dec_right,
                compositor,
                shm,
                &wp.river_window,
                qhandle,
                win_width,
                0,
                grab_w,
                win_height,
                scale,
                state.wm.layout.border_blur,
                bg_color,
                0,
                0,
                border_width,
                win_height,
                border_width,
                false,
                "right",
                hover_right,
            );

            let hover_bottom = state.pointer_hovered_surface.as_ref()
                .filter(|surf| wp.dec_bottom.as_ref().map_or(false, |d| &d.surface == *surf))
                .map(|_| (state.last_pointer_surface_x, state.last_pointer_surface_y));

            update_border_decoration(
                wid,
                &mut wp.dec_bottom,
                compositor,
                shm,
                &wp.river_window,
                qhandle,
                -grab_w,
                win_height,
                win_width + 2 * grab_w,
                grab_w,
                scale,
                state.wm.layout.border_blur,
                bg_color,
                grab_w - border_width,
                0,
                win_width + 2 * border_width,
                border_width,
                border_width,
                true,
                "bottom",
                hover_bottom,
            );
        }

        // Determine titlebar dimensions:
        // Height equals border_width (or 16 if border_width is too small to display font)
        let logical_height = std::cmp::max(border_width, 16);
        let logical_width = if is_minimized {
            160
        } else {
            win_width + 2 * border_width
        };
        let dec_height = logical_height * scale;
        let dec_width = logical_width * scale;

        let needs_new_buffer = match &wp.decoration {
            Some(dec) => dec.width != dec_width || dec.height != dec_height,
            None => true,
        };

        if wp.decoration.is_none() {
            eprintln!("[decorations] creating decoration for window {}", wid);
            let surface: wl_surface::WlSurface = compositor.create_surface(qhandle, ());
            let decoration: RiverDecorationV1 = wp.river_window.get_decoration_above(&surface, qhandle, ());

            wp.decoration = Some(WindowDecoration {
                decoration,
                surface,
                buffer: None,
                pool: None,
                width: 0,
                height: 0,
                mapped_data: ptr::null_mut(),
                mapped_size: 0,
            });
        }

        let hover_top = state.pointer_hovered_surface.as_ref()
            .filter(|surf| wp.decoration.as_ref().map_or(false, |d| &d.surface == *surf))
            .map(|_| (state.last_pointer_surface_x, state.last_pointer_surface_y));

        let dec = wp.decoration.as_mut().unwrap();

        // Position decoration on top of the window top border
        if is_minimized {
            if let Some(idx) = minimized_idx {
                let bar_height = state.wm.layout.bar_height;
                let gap_top = state.wm.layout.gap_top;
                let bubble_gap = 8;
                let bubble_y = bar_height + gap_top + idx as i32 * (logical_height + bubble_gap);
                dec.decoration.set_offset(0, bubble_y - win_y);
            }
        } else {
            dec.decoration.set_offset(-border_width, -logical_height);
        }

        dec.decoration.set_blur(if state.wm.layout.border_blur { 1 } else { 0 });

        if needs_new_buffer {
            eprintln!(
                "[decorations] allocating buffer {}x{} for window {}",
                dec_width, dec_height, wid
            );

            // Clean up old mapping if it exists
            if !dec.mapped_data.is_null() {
                unsafe {
                    libc::munmap(dec.mapped_data as *mut libc::c_void, dec.mapped_size);
                }
                dec.mapped_data = ptr::null_mut();
            }

            let stride = dec_width * 4;
            let size = (stride * dec_height) as usize;

            if let Some(fd) = create_memfd(size) {
                let mapped_data = unsafe {
                    libc::mmap(
                        ptr::null_mut(),
                        size,
                        libc::PROT_READ | libc::PROT_WRITE,
                        libc::MAP_SHARED,
                        fd,
                        0,
                    ) as *mut u32
                };

                if mapped_data != libc::MAP_FAILED as *mut u32 {
                    let borrowed_fd = unsafe { std::os::fd::BorrowedFd::borrow_raw(fd) };
                    let pool = shm.create_pool(borrowed_fd, size as i32, qhandle, ());
                    let buffer = pool.create_buffer(
                        0,
                        dec_width,
                        dec_height,
                        stride,
                        wl_shm::Format::Argb8888,
                        qhandle,
                        (),
                    );

                    dec.pool = Some(pool);
                    dec.buffer = Some(buffer);
                    dec.width = dec_width;
                    dec.height = dec_height;
                    dec.mapped_data = mapped_data;
                    dec.mapped_size = size;
                    dec.surface.set_buffer_scale(scale);
                } else {
                    eprintln!("[decorations] failed to mmap decoration buffer");
                    unsafe { libc::close(fd); }
                    continue;
                }
                unsafe { libc::close(fd); }
            } else {
                eprintln!("[decorations] failed to create memfd");
                continue;
            }
        }

        // Draw titlebar background and text
        if !dec.mapped_data.is_null() {
            let buffer_slice = unsafe {
                std::slice::from_raw_parts_mut(dec.mapped_data, (dec_width * dec_height) as usize)
            };

            // Clear background with 3D cylindrical shading and mitred corners (outer edge only)
            let border_w_scaled = border_width * scale;
            let titlebar_h_scaled = logical_height * scale;
            let win_w_scaled = (if is_minimized { 160 - 2 * border_width } else { win_width }) * scale;

            let mut highlight_region: Option<HighlightRegion> = None;
            if let Some((hx, hy)) = hover_top {
                let mid_y = (logical_height as f64) / 2.0;
                if hy < mid_y {
                    const CORNER_THRESHOLD: f64 = 16.0;
                    if hx < CORNER_THRESHOLD {
                        highlight_region = Some(HighlightRegion::TopLeft);
                    } else if hx > (logical_width as f64 - CORNER_THRESHOLD) {
                        highlight_region = Some(HighlightRegion::TopRight);
                    } else {
                        highlight_region = Some(HighlightRegion::Top);
                    }
                }
            }

            for py in 0..dec_height {
                let row_offset = (py * dec_width) as usize;
                for px in 0..dec_width {
                    // Convert to coordinates relative to client area (0, 0)
                    let logical_px = px - border_w_scaled;
                    let logical_py = py - titlebar_h_scaled;

                    // Determine which border region this pixel belongs to, using the exact diagonal slope:
                    let slope = (titlebar_h_scaled as f32) / (border_w_scaled as f32).max(1.0);
                    let s = if logical_px < 0 && logical_py > (logical_px as f32 * slope) as i32 {
                        // Left border region (outer is at t = 0)
                        let t_val = ((logical_px + border_w_scaled) as f32) / (border_w_scaled as f32 - 1.0).max(1.0);
                        if t_val < 0.5 {
                            1.0 + 0.45 * (1.0 - 2.0 * t_val) * (1.0 - 2.0 * t_val)
                        } else {
                            1.0
                        }
                    } else if logical_px > win_w_scaled && logical_py > (-((logical_px - win_w_scaled) as f32 * slope)) as i32 {
                        // Right border region (outer is at t = 0)
                        let t_val = 1.0 - ((logical_px - win_w_scaled) as f32) / (border_w_scaled as f32 - 1.0).max(1.0);
                        if t_val < 0.5 {
                            1.0 - 0.50 * (1.0 - 2.0 * t_val) * (1.0 - 2.0 * t_val)
                        } else {
                            1.0
                        }
                    } else {
                        // Top border region (outer is at t = 0)
                        let t_val = ((logical_py + titlebar_h_scaled) as f32) / (titlebar_h_scaled as f32 - 1.0).max(1.0);
                        if t_val < 0.5 {
                            1.0 + 0.45 * (1.0 - 2.0 * t_val) * (1.0 - 2.0 * t_val)
                        } else {
                            1.0
                        }
                    };

                    let a = ((bg_color >> 24) & 0xFF) as f32;
                    let mut r_val = (((bg_color >> 16) & 0xFF) as f32 * s).round().min(255.0) as u32;
                    let mut g = (((bg_color >> 8) & 0xFF) as f32 * s).round().min(255.0) as u32;
                    let mut b = ((bg_color & 0xFF) as f32 * s).round().min(255.0) as u32;

                    if let Some(region) = highlight_region {
                        if is_pixel_in_highlight_region("top", region, px, py, logical_width, logical_height, scale) {
                            r_val = (r_val as f32 * 0.6 + 255.0 * 0.4).round() as u32;
                            g = (g as f32 * 0.6 + 255.0 * 0.4).round() as u32;
                            b = (b as f32 * 0.6 + 255.0 * 0.4).round() as u32;
                        }
                    }

                    let base_pixel_color = ((a as u32) << 24) | (r_val << 16) | (g << 8) | b;
                    let mut final_color = base_pixel_color;

                    let lx = px as f32 / scale as f32;
                    let ly = py as f32 / scale as f32;

                    if !is_minimized && ly >= 0.0 && ly < logical_height as f32 {
                        if lx >= (logical_width as f32 - 48.0) && lx < (logical_width as f32 - 32.0) {
                            // Minimize button
                            let is_hovered = hover_top.map_or(false, |(hx, hy)| {
                                hx >= (logical_width as f64 - 48.0) && hx < (logical_width as f64 - 32.0)
                                    && hy >= 0.0 && hy < logical_height as f64
                            });
                            final_color = if is_hovered {
                                blend_colors(base_pixel_color, 0x30FFFFFF)
                            } else {
                                base_pixel_color
                            };

                            let cx = logical_width as f32 - 40.0;
                            let cy = logical_height as f32 / 2.0;
                            if lx >= (cx - 4.0) && lx <= (cx + 4.0) && ly >= (cy + 2.0) && ly < (cy + 3.0) {
                                final_color = text_color;
                            }
                        } else if lx >= (logical_width as f32 - 32.0) && lx < (logical_width as f32 - 16.0) {
                            // Maximize button
                            let is_hovered = hover_top.map_or(false, |(hx, hy)| {
                                hx >= (logical_width as f64 - 32.0) && hx < (logical_width as f64 - 16.0)
                                    && hy >= 0.0 && hy < logical_height as f64
                            });
                            final_color = if is_hovered {
                                blend_colors(base_pixel_color, 0x30FFFFFF)
                            } else {
                                base_pixel_color
                            };

                            let cx = logical_width as f32 - 24.0;
                            let cy = logical_height as f32 / 2.0;
                            let is_border_x = (lx >= cx - 4.0 && lx < cx - 3.0) || (lx > cx + 3.0 && lx <= cx + 4.0);
                            let is_in_x = lx >= cx - 4.0 && lx <= cx + 4.0;
                            let is_border_y = (ly >= cy - 4.0 && ly < cy - 3.0) || (ly > cy + 3.0 && ly <= cy + 4.0);
                            let is_in_y = ly >= cy - 4.0 && ly <= cy + 4.0;
                            if (is_border_x && is_in_y) || (is_border_y && is_in_x) {
                                final_color = text_color;
                            }
                        } else if lx >= (logical_width as f32 - 16.0) && lx <= logical_width as f32 {
                            // Close button
                            let is_hovered = hover_top.map_or(false, |(hx, hy)| {
                                hx >= (logical_width as f64 - 16.0) && hx <= logical_width as f64
                                    && hy >= 0.0 && hy < logical_height as f64
                            });
                            final_color = if is_hovered {
                                blend_colors(base_pixel_color, 0x90E53935)
                            } else {
                                base_pixel_color
                            };

                            let cx = logical_width as f32 - 8.0;
                            let cy = logical_height as f32 / 2.0;
                            let dx = (lx - cx).abs();
                            let dy = (ly - cy).abs();
                            if dx <= 4.0 && dy <= 4.0 && (dx - dy).abs() < 1.0 {
                                final_color = text_color;
                            }
                        }
                    }

                    buffer_slice[row_offset + px as usize] = final_color;
                }
            }

            // Draw window title text
            if let Some(ref font) = state.border_font {
                let font_size = if state.wm.layout.border_font_size > 0 {
                    state.wm.layout.border_font_size as f32 * scale as f32
                } else {
                    if logical_height >= 32 {
                        22.0 * scale as f32
                    } else if logical_height >= 24 {
                        16.0 * scale as f32
                    } else if logical_height >= 16 {
                        11.0 * scale as f32
                    } else {
                        (logical_height as f32 - 4.0).max(8.0) * scale as f32
                    }
                };

                let line_metrics = font.horizontal_line_metrics(font_size).unwrap_or(fontdue::LineMetrics {
                    ascent: font_size * 0.8,
                    descent: -font_size * 0.2,
                    line_gap: 0.0,
                    new_line_size: font_size,
                });
                let baseline_y = (dec_height as f32 + line_metrics.ascent + line_metrics.descent) / 2.0;

                let mut text_x = 24.0f32 * scale as f32; // Margin from left
                for c in title.chars() {
                    let (metrics, bitmap) = font.rasterize(c, font_size);
                    if text_x + metrics.xmin as f32 + metrics.width as f32 > dec_width as f32 - 48.0 * scale as f32 {
                        break;
                    }

                    let x_start = (text_x + metrics.xmin as f32).round() as i32;
                    let y_start = (baseline_y - metrics.ymin as f32 - metrics.height as f32).round() as i32;

                    for row in 0..metrics.height {
                        for col in 0..metrics.width {
                            let px = x_start + col as i32;
                            let py = y_start + row as i32;

                            if px >= 0 && px < dec_width && py >= 0 && py < dec_height {
                                let alpha_coverage = bitmap[row * metrics.width + col] as u32;
                                if alpha_coverage > 0 {
                                    let index = (py * dec_width + px) as usize;
                                    let dest_pixel = buffer_slice[index];

                                    let src_r = (text_color >> 16) & 0xFF;
                                    let src_g = (text_color >> 8) & 0xFF;
                                    let src_b = text_color & 0xFF;
                                    let src_a = (text_color >> 24) & 0xFF;

                                    let dest_r = (dest_pixel >> 16) & 0xFF;
                                    let dest_g = (dest_pixel >> 8) & 0xFF;
                                    let dest_b = dest_pixel & 0xFF;
                                    let dest_a = (dest_pixel >> 24) & 0xFF;

                                    let alpha = (alpha_coverage * src_a) / 255;

                                    let out_r = ((src_r * alpha) + (dest_r * (255 - alpha))) / 255;
                                    let out_g = ((src_g * alpha) + (dest_g * (255 - alpha))) / 255;
                                    let out_b = ((src_b * alpha) + (dest_b * (255 - alpha))) / 255;
                                    let out_a = dest_a + ((255 - dest_a) * alpha) / 255;

                                    buffer_slice[index] = (out_a << 24) | (out_r << 16) | (out_g << 8) | out_b;
                                }
                            }
                        }
                    }
                    text_x += metrics.advance_width;
                }
            } else {
                // Determine font scale (1x if height < 32, 2x if height >= 32)
                let drawing_scale = if logical_height >= 32 { 2 } else { 1 };
                let font_h = 8 * drawing_scale * scale;
                
                // Vertically center the text inside the titlebar
                let text_y = (dec_height - font_h) / 2;
                let mut text_x = 24 * scale; // Margin from left

                for c in title.chars() {
                    if text_x + 8 * drawing_scale * scale > dec_width - 48 * scale {
                        break; // Out of bounds
                    }
                    draw_char(buffer_slice, dec_width, dec_height, c, text_x, text_y, drawing_scale * scale, text_color);
                    text_x += 8 * drawing_scale * scale;
                }
            }

            // Apply corner rounding to top-left and top-right of titlebar
            let r_top = 16 * scale;
            round_corner(buffer_slice, dec_width, dec_height, r_top, r_top, r_top, CornerType::TopLeft);
            round_corner(buffer_slice, dec_width, dec_height, dec_width - r_top, r_top, r_top, CornerType::TopRight);

            // Commit surface rendering
            if !is_minimized {
                dec.decoration.sync_next_commit();
            }
            if let Some(ref wl_buf) = dec.buffer {
                dec.surface.attach(Some(wl_buf), 0, 0);
            }
            dec.surface.commit();
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HighlightRegion {
    TopLeft,
    Top,
    TopRight,
    Left,
    Right,
    BottomLeft,
    Bottom,
    BottomRight,
}

fn is_pixel_in_highlight_region(
    side: &str,
    region: HighlightRegion,
    px: i32,
    py: i32,
    logical_width: i32,
    logical_height: i32,
    scale: i32,
) -> bool {
    let px_logical = px as f32 / scale as f32;
    let py_logical = py as f32 / scale as f32;
    const CORNER_THRESHOLD: f32 = 16.0;

    match (side, region) {
        ("left", HighlightRegion::TopLeft) => py_logical < CORNER_THRESHOLD,
        ("left", HighlightRegion::BottomLeft) => py_logical > (logical_height as f32 - CORNER_THRESHOLD),
        ("left", HighlightRegion::Left) => py_logical >= CORNER_THRESHOLD && py_logical <= (logical_height as f32 - CORNER_THRESHOLD),

        ("right", HighlightRegion::TopRight) => py_logical < CORNER_THRESHOLD,
        ("right", HighlightRegion::BottomRight) => py_logical > (logical_height as f32 - CORNER_THRESHOLD),
        ("right", HighlightRegion::Right) => py_logical >= CORNER_THRESHOLD && py_logical <= (logical_height as f32 - CORNER_THRESHOLD),

        ("bottom", HighlightRegion::BottomLeft) => px_logical < CORNER_THRESHOLD,
        ("bottom", HighlightRegion::BottomRight) => px_logical > (logical_width as f32 - CORNER_THRESHOLD),
        ("bottom", HighlightRegion::Bottom) => px_logical >= CORNER_THRESHOLD && px_logical <= (logical_width as f32 - CORNER_THRESHOLD),

        ("top", HighlightRegion::TopLeft) => px_logical < CORNER_THRESHOLD,
        ("top", HighlightRegion::TopRight) => px_logical > (logical_width as f32 - CORNER_THRESHOLD),
        ("top", HighlightRegion::Top) => px_logical >= CORNER_THRESHOLD && px_logical <= (logical_width as f32 - CORNER_THRESHOLD),

        _ => false,
    }
}

fn blend_colors(bg: u32, fg: u32) -> u32 {
    let fg_a = (fg >> 24) & 0xFF;
    if fg_a == 0 {
        return bg;
    }
    if fg_a == 255 {
        return fg;
    }
    let bg_a = (bg >> 24) & 0xFF;
    let bg_r = (bg >> 16) & 0xFF;
    let bg_g = (bg >> 8) & 0xFF;
    let bg_b = bg & 0xFF;

    let fg_r = (fg >> 16) & 0xFF;
    let fg_g = (fg >> 8) & 0xFF;
    let fg_b = fg & 0xFF;

    let out_r = ((fg_r * fg_a) + (bg_r * (255 - fg_a))) / 255;
    let out_g = ((fg_g * fg_a) + (bg_g * (255 - fg_a))) / 255;
    let out_b = ((fg_b * fg_a) + (bg_b * (255 - fg_a))) / 255;
    let out_a = bg_a + ((255 - bg_a) * fg_a) / 255;

    (out_a << 24) | (out_r << 16) | (out_g << 8) | out_b
}
