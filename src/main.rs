#![windows_subsystem = "windows"]

use std::{
    ffi::c_void,
    fs, mem,
    path::{Path, PathBuf},
    ptr::{addr_of_mut, null, null_mut},
    slice,
};
mod decode;
mod lang;
use lang::{t as tr, tf as trf};
use decode::{decode, Decoded};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::Gdi::*,
    System::{
        DataExchange::{CloseClipboard, EmptyClipboard, SetClipboardData, GetClipboardData, IsClipboardFormatAvailable, OpenClipboard, RegisterClipboardFormatW, COPYDATASTRUCT},
        LibraryLoader::GetModuleHandleW,
        Memory::{GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock, GMEM_MOVEABLE},
        Registry::*,
        Threading::CreateMutexW,
    },
    UI::{
        Controls::{Dialogs::*, *}, HiDpi::*,
        Input::KeyboardAndMouse::{GetAsyncKeyState, RegisterHotKey, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, VK_CONTROL, 
            ReleaseCapture, SetCapture, SetFocus, TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT, VK_DELETE, VK_DOWN, VK_ESCAPE, VK_SPACE, VK_UP,
        },
        Shell::*,
        WindowsAndMessaging::*,
    },
};

const T_FRAME: usize = 1;
const T_SAVE: usize = 2;
const T_LOCK: usize = 3;

const ID_LOCK: usize = 1;
const ID_OPEN: usize = 2;
const ID_ROT_L: usize = 3;
const ID_ROT_R: usize = 4;
const ID_FLIP_H: usize = 5;
const ID_FLIP_V: usize = 6;
const ID_RESET: usize = 7;
const ID_PAUSE: usize = 8;
const ID_EXIT: usize = 9;
const ID_HISTORY: usize = 10;
const ID_QUIT: usize = 11;
const ID_PASTE: usize = 12;
const ID_COPY: usize = 13;
const ID_FOLDER: usize = 14;
const ID_TOPMOST: usize = 15;
const ID_SCALE: usize = 100; // + index into SCALES
const ID_OPACITY: usize = 200; // + index into OPACITIES
const SCALES: [u32; 7] = [25, 50, 75, 100, 150, 200, 300];
const OPACITIES: [u32; 10] = [10, 20, 30, 40, 50, 60, 70, 80, 90, 100];
const ID_SPEED: usize = 300; // + index into SPEEDS
const SPEEDS: [f32; 6] = [0.25, 0.5, 1.0, 1.5, 2.0, 4.0];
const SPEED_NORMAL: usize = 2; // index of 1x
const HISTORY_MAX: usize = 30;

// ---------- shared helpers ----------

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain([0]).collect()
}

fn exe_file(name: &str) -> PathBuf {
    let mut p = std::env::current_exe().unwrap_or_default();
    p.set_file_name(name);
    p
}

/// Writes next to the exe via a temp file + rename, so a crash mid-write cannot leave a half file.
fn write_atomic(name: &str, data: &[u8]) {
    let tmp = exe_file(&format!("{name}.tmp"));
    if fs::write(&tmp, data).is_ok() && fs::rename(&tmp, exe_file(name)).is_err() {
        let _ = fs::remove_file(&tmp);
    }
}

fn abs_path(p: &str) -> String {
    std::path::absolute(p).map_or(p.to_string(), |p| p.to_string_lossy().into_owned())
}

// ---------- settings (settings.json next to the exe, same keys as the C# version) ----------

fn json_val(s: &str, key: &str) -> Option<String> {
    let i = s.find(&format!("\"{key}\""))? + key.len() + 2;
    let rest = s[i..].trim_start().strip_prefix(':')?.trim_start();
    Some(rest.split([',', '\n', '\r', '}']).next()?.trim().to_string())
}

fn write_settings(opacity: u32) {
    let json = format!(
        "{{
  \"Opacity\": {},
  \"DashSort\": {},
  \"RestoreSession\": {},
  \"Language\": \"{}\"
}}",
        opacity as f64 / 100.0, dash().sort, dash().restore, dash().lang.map_or("auto", |i| lang::CODES[i])
    );
    write_atomic("settings.json", json.as_bytes());
}

fn save_settings(a: &App) {
    write_settings(a.opacity);
    save_session();
}

// ---------- history (history.txt, one path per line, newest first) ----------

/// The saved history, or None when the file exists but cannot be read right now (a virus scanner
/// holding it, for example). Callers that rewrite the file must not treat that as "empty".
fn history_read() -> Option<Vec<String>> {
    let text = match fs::read_to_string(exe_file("history.txt")) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(_) => return None,
    };
    Some(
        text.lines()
            .map(|l| l.trim().trim_start_matches('\u{feff}')) // tolerate editors that add a BOM
            .filter(|l| !l.is_empty())
            .map(String::from)
            .collect(),
    )
}

/// Pasted / downloaded images only live while they are in the history. Files younger than `min_age`
/// are spared: one that was just saved is still loading and is not in the history yet.
fn prune_pasted(dir: &Path, keep: &[String], min_age: std::time::Duration) {
    for e in fs::read_dir(dir).into_iter().flatten().flatten() {
        let listed = keep.iter().any(|p| e.path().to_string_lossy().eq_ignore_ascii_case(p));
        let old = e.metadata().and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).is_some_and(|d| d >= min_age);
        if old && !listed {
            let _ = fs::remove_file(e.path());
        }
    }
}

fn history_save(h: &[String]) {
    write_atomic("history.txt", h.join("\n").as_bytes());
    prune_pasted(&exe_file("pasted"), h, std::time::Duration::from_secs(60));
}

fn history_push(path: &str) {
    let Some(mut h) = history_read() else { return }; // unreadable: keep the file as it is
    h.retain(|p| !p.eq_ignore_ascii_case(path));
    h.insert(0, path.to_string());
    h.truncate(HISTORY_MAX);
    history_save(&h);
}

fn history_remove(path: &str) {
    let Some(mut h) = history_read() else { return };
    h.retain(|p| !p.eq_ignore_ascii_case(path));
    history_save(&h);
}


fn sample(f: &[u32], gw: usize, gh: usize, x: f32, y: f32) -> u32 {
    let x = x.clamp(0.0, (gw - 1) as f32);
    let y = y.clamp(0.0, (gh - 1) as f32);
    let (x0, y0) = (x as usize, y as usize);
    let (x1, y1) = ((x0 + 1).min(gw - 1), (y0 + 1).min(gh - 1));
    let (fx, fy) = (x - x0 as f32, y - y0 as f32);
    let (p00, p10, p01, p11) = (f[y0 * gw + x0], f[y0 * gw + x1], f[y1 * gw + x0], f[y1 * gw + x1]);
    let mut out = 0u32;
    for s in [0, 8, 16, 24] {
        let c = |p: u32| ((p >> s) & 0xFF) as f32;
        let top = c(p00) + (c(p10) - c(p00)) * fx;
        let bot = c(p01) + (c(p11) - c(p01)) * fx;
        out |= ((top + (bot - top) * fy + 0.5) as u32) << s;
    }
    out
}

/// Several files can be picked when `multi` is set (up to 10, like a drop).
fn open_dialog(owner: HWND, multi: bool) -> Vec<String> {
    let filter: Vec<u16> = format!(
        "{}\0*.gif;*.png;*.jpg;*.jpeg;*.bmp;*.tif;*.tiff;*.webp;*.apng;*.avif\0{}\0*.gif\0{}\0*.*\0\0",
        tr("Images"), tr("GIF Files"), tr("All Files")
    )
    .encode_utf16()
    .collect();
    let title = wide(&tr("Select Image"));
    let mut buf = vec![0u16; 16 * 1024];
    unsafe {
        let mut ofn: OPENFILENAMEW = mem::zeroed();
        ofn.lStructSize = mem::size_of::<OPENFILENAMEW>() as u32;
        ofn.hwndOwner = owner;
        ofn.lpstrFilter = filter.as_ptr();
        ofn.lpstrFile = buf.as_mut_ptr();
        ofn.nMaxFile = buf.len() as u32;
        ofn.lpstrTitle = title.as_ptr();
        ofn.Flags = OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST | OFN_NOCHANGEDIR | if multi { OFN_ALLOWMULTISELECT | OFN_EXPLORER } else { 0 };
        if GetOpenFileNameW(&mut ofn) == 0 {
            return Vec::new();
        }
    }
    // one file: "path\0\0"; several: "dir\0name1\0name2\0\0"
    let mut parts = buf.split(|&c| c == 0).take_while(|p| !p.is_empty()).map(String::from_utf16_lossy);
    let first = parts.next();
    let rest: Vec<String> = parts.take(10).collect();
    match first {
        Some(dir) if !rest.is_empty() => rest.iter().map(|n| Path::new(&dir).join(n).to_string_lossy().into_owned()).collect(),
        Some(one) => vec![one],
        None => Vec::new(),
    }
}

/// Files of a WM_DROPFILES message (at most 10, so a careless drop cannot spawn hundreds of decodes).
unsafe fn dropped_files(wp: WPARAM) -> Vec<String> {
    let hdrop = wp as HDROP;
    let count = DragQueryFileW(hdrop, u32::MAX, null_mut(), 0).min(10);
    let out = (0..count).filter_map(|i| drop_path(hdrop, i)).collect();
    DragFinish(hdrop);
    out
}

/// File `i` of a drop / clipboard file list, however long its path is.
unsafe fn drop_path(hdrop: HDROP, i: u32) -> Option<String> {
    let n = DragQueryFileW(hdrop, i, null_mut(), 0) as usize;
    let mut buf = vec![0u16; n + 1];
    let n = DragQueryFileW(hdrop, i, buf.as_mut_ptr(), buf.len() as u32) as usize;
    (n > 0).then(|| String::from_utf16_lossy(&buf[..n]))
}

// =====================================================================
// Overlay window
// =====================================================================

struct App {
    hwnd: HWND,
    frames: Vec<Vec<u32>>, // premultiplied BGRA, gw*gh each
    delays: Vec<u32>,      // ms
    gw: usize,
    gh: usize,
    cur: usize,
    paused: bool,
    rot: u8, // quarter turns clockwise
    flip_h: bool,
    flip_v: bool,
    w: i32,
    h: i32,
    opacity: u32, // percent
    locked: bool,
    hover: bool, // mouse is over the overlay: show the resize handles
    hot: Zone,   // handle under the cursor (drawn highlighted)
    drag: Option<(Zone, i32, i32, i32, i32)>, // zone, start cursor x, y, start width, height
    path: String, // absolute path of the image shown (lets the dashboard mark / reuse open windows)
    restore: Option<(i32, i32, i32, i32)>, // x, y, w, h before "maximize"
    speed: usize, // index into SPEEDS
}

/// Resize handles: the corner keeps the aspect ratio, the edges change one axis only.
#[derive(Clone, Copy, PartialEq)]
enum Zone {
    None,
    Corner,
    Right,
    Bottom,
    Close, // top-right buttons, right to left: close, maximize, minimize
    Max,
    Min,
}

// One `App` per overlay window, owned through the window's GWLP_USERDATA pointer.
// The UI is single threaded; this list only tells us how many overlays are open.
static mut OVERLAYS: Vec<*mut App> = Vec::new();

fn overlays() -> &'static mut Vec<*mut App> {
    unsafe { &mut *addr_of_mut!(OVERLAYS) }
}

fn dpi_px(a: &App, v: i32) -> i32 {
    unsafe { v * GetDpiForWindow(a.hwnd) as i32 / 96 }
}

/// Side of the corner grip triangle in pixels (never more than half the image).
fn btn(a: &App) -> i32 {
    grip(a).min(a.w / 3)
}

fn grip(a: &App) -> i32 {
    dpi_px(a, 22).min(a.w.min(a.h) / 2)
}

/// Thickness of the right/bottom edge handles.
fn edge(a: &App) -> i32 {
    dpi_px(a, 8).max(6).min(a.w.min(a.h) / 4)
}

fn zone_at(a: &App, x: i32, y: i32) -> Zone {
    if x < 0 || y < 0 || x >= a.w || y >= a.h {
        Zone::None
    } else if !a.locked && y < btn(a) && x >= a.w - 3 * btn(a) {
        match (a.w - 1 - x) / btn(a) {
            0 => Zone::Close,
            1 => Zone::Max,
            _ => Zone::Min,
        }
    } else if a.restore.is_some() {
        Zone::None // maximized: no resize handles
    } else if (a.w - 1 - x) + (a.h - 1 - y) < grip(a) {
        Zone::Corner
    } else if x >= a.w - edge(a) {
        Zone::Right
    } else if y >= a.h - edge(a) {
        Zone::Bottom
    } else {
        Zone::None
    }
}

/// Per display column/row: the two source indices to blend and the 8-bit weight of the second.
fn axis_table(n: usize, size: usize, flip: bool, invert: bool) -> Vec<(usize, usize, u32)> {
    (0..n)
        .map(|i| {
            let mut u = (i as f32 + 0.5) / n as f32;
            if flip != invert {
                u = 1.0 - u; // flip and rotation inversion cancel out
            }
            let c = (u * size as f32 - 0.5).clamp(0.0, (size - 1) as f32);
            let i0 = c as usize;
            (i0, (i0 + 1).min(size - 1), ((c - i0 as f32) * 256.0).round() as u32)
        })
        .collect()
}

/// Blends two premultiplied BGRA pixels, `t` in 0..=256 is the weight of `b`
/// (two channels per multiply: exact for t = 0 and for equal pixels).
fn lerp(a: u32, b: u32, t: u32) -> u32 {
    const M: u32 = 0x00FF_00FF;
    let rb = ((a & M) * (256 - t) + (b & M) * t + 0x0080_0080) >> 8 & M; // +128 per lane rounds
    let ag = (((a >> 8) & M) * (256 - t) + ((b >> 8) & M) * t + 0x0080_0080) & !M;
    rb | ag
}

/// Bilinear upscale with flips and rotation, in integer math. Because flips and quarter turns
/// keep rows and columns independent, the source positions are computed once per row / column.
fn upscale_into(
    dst: &mut [u32], (w, h): (usize, usize), f: &[u32], (gw, gh): (usize, usize),
    rot: u8, (flip_h, flip_v): (bool, bool),
) {
    let swap = rot % 2 == 1; // display columns then walk the source rows and vice versa
    let tx = axis_table(w, if swap { gh } else { gw }, flip_h, rot == 1 || rot == 2);
    let ty = axis_table(h, if swap { gw } else { gh }, flip_v, rot == 2 || rot == 3);
    for (y, row) in dst.chunks_exact_mut(w).enumerate() {
        let (a0, a1, fa) = ty[y];
        for (px, &(b0, b1, fb)) in row.iter_mut().zip(&tx) {
            let (x0, x1, wx, y0, y1, wy) = if swap { (a0, a1, fa, b0, b1, fb) } else { (b0, b1, fb, a0, a1, fa) };
            let top = lerp(f[y0 * gw + x0], f[y0 * gw + x1], wx);
            let bot = lerp(f[y1 * gw + x0], f[y1 * gw + x1], wx);
            *px = lerp(top, bot, wy);
        }
    }
}

fn render(a: &App) {
    let (w, h) = (a.w as usize, a.h as usize);
    unsafe {
        let screen = GetDC(null_mut());
        let mem_dc = CreateCompatibleDC(screen);
        let mut bi: BITMAPINFO = mem::zeroed();
        bi.bmiHeader.biSize = mem::size_of::<BITMAPINFOHEADER>() as u32;
        bi.bmiHeader.biWidth = w as i32;
        bi.bmiHeader.biHeight = -(h as i32);
        bi.bmiHeader.biPlanes = 1;
        bi.bmiHeader.biBitCount = 32;
        bi.bmiHeader.biCompression = BI_RGB;
        let mut bits: *mut c_void = null_mut();
        let bmp = CreateDIBSection(mem_dc, &bi, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
        if !bits.is_null() {
            let old = SelectObject(mem_dc, bmp);
            let dst = slice::from_raw_parts_mut(bits as *mut u32, w * h);
            let f = &a.frames[a.cur];
            // display (u,v) in 0..1 -> source (u,v), undoing flips and rotation
            let tf = |mut u: f32, mut v: f32| {
                if a.flip_h { u = 1.0 - u }
                if a.flip_v { v = 1.0 - v }
                match a.rot {
                    0 => (u, v),
                    1 => (v, 1.0 - u),
                    2 => (1.0 - u, 1.0 - v),
                    _ => (1.0 - v, u),
                }
            };
            let (gwf, ghf) = (a.gw as f32, a.gh as f32);
            let (nw, nh) = natural_size(a);
            let scale = (w as f64 / nw).min(h as f64 / nh);
            // shrinking: average the covered source area (bilinear alone gives jagged edges)
            let shrink = scale < 1.0;
            if shrink {
                for y in 0..h {
                    for x in 0..w {
                        let (u0, v0) = tf(x as f32 / w as f32, y as f32 / h as f32);
                        let (u1, v1) = tf((x + 1) as f32 / w as f32, (y + 1) as f32 / h as f32);
                        dst[y * w + x] = box_avg(f, a.gw, a.gh, u0.min(u1) * gwf, v0.min(v1) * ghf, u0.max(u1) * gwf, v0.max(v1) * ghf);
                    }
                }
            } else {
                if (w, h) == (a.gw, a.gh) && a.rot == 0 && !a.flip_h && !a.flip_v {
                    dst.copy_from_slice(f); // shown 1:1: nothing to resample
                } else {
                    upscale_into(dst, (w, h), f, (a.gw, a.gh), a.rot, (a.flip_h, a.flip_v));
                }
            }
            // Fully transparent pixels let the mouse fall through the window, so a PNG with a
            // transparent background could not be grabbed there. Alpha 1 is invisible but hit-testable.
            for p in dst.iter_mut() {
                if *p >> 24 == 0 {
                    *p = 0x0100_0000;
                }
            }
            // resize handles, shown while hovering: edge bars + a striped corner triangle,
            // the one under the cursor is highlighted
            if a.hover && !a.locked && a.restore.is_none() { // no resizing while maximized
                let (g, t) = (grip(a) as usize, (edge(a) / 2).max(3) as usize);
                let color = |z: Zone, normal: u32| if a.hot == z { 0xB000_5394 } else { normal };
                for y in 0..h - g {
                    for x in w - t..w {
                        dst[y * w + x] = color(Zone::Right, 0x6000_0000);
                    }
                }
                for y in h - t..h {
                    for x in 0..w - g {
                        dst[y * w + x] = color(Zone::Bottom, 0x6000_0000);
                    }
                }
                // left and top edges only complete the frame so the four sides look alike
                for y in 0..h {
                    dst[y * w..y * w + t].fill(0x6000_0000);
                }
                dst[..t * w].fill(0x6000_0000);
                for dy in 0..g {
                    for dx in 0..g - dy {
                        let d = dx + dy;
                        dst[(h - 1 - dy) * w + (w - 1 - dx)] =
                            if d < g - 3 && matches!(d % 6, 2 | 3) { 0xC0C0_C0C0 } else { color(Zone::Corner, 0x7000_0000) };
                    }
                }
            }
            if a.hover && a.frames.len() > 1 {
                draw_progress(a, dst);
            }
            // top-right buttons (hidden when locked): minimize | maximize | close, each a dark square
            // with a white icon; the one under the cursor is highlighted (red for close)
            if a.hover && !a.locked {
                let b = btn(a) as usize;
                let m = b / 4;
                let bg = |z: Zone, hot: u32| if a.hot == z { hot } else { 0x7000_0000 };
                let mut rect = |x0: usize, y0: usize, x1: usize, y1: usize, c: u32| {
                    for y in y0..y1 {
                        dst[y * w + x0..y * w + x1].fill(c);
                    }
                };
                const WHITE: u32 = 0xFFFF_FFFF;
                // close: X
                rect(w - b, 0, w, b, bg(Zone::Close, 0xE0C4_2B1B));
                for i in m..b - m {
                    rect(w - b + i, i, w - b + i + 2, i + 1, WHITE);
                    rect(w - 2 - i, i, w - i, i + 1, WHITE);
                }
                // maximize: square outline
                let x0 = w - 2 * b;
                rect(x0, 0, x0 + b, b, bg(Zone::Max, 0xC050_5050));
                rect(x0 + m, m, x0 + b - m, m + 2, WHITE);
                rect(x0 + m, b - m - 2, x0 + b - m, b - m, WHITE);
                rect(x0 + m, m, x0 + m + 2, b - m, WHITE);
                rect(x0 + b - m - 2, m, x0 + b - m, b - m, WHITE);
                // minimize: bar at the bottom
                let x0 = w - 3 * b;
                rect(x0, 0, x0 + b, b, bg(Zone::Min, 0xC050_5050));
                rect(x0 + m, b - m - 2, x0 + b - m, b - m, WHITE);
            }
            let size = SIZE { cx: w as i32, cy: h as i32 };
            let src = POINT { x: 0, y: 0 };
            let bf = BLENDFUNCTION {
                BlendOp: AC_SRC_OVER as u8,
                BlendFlags: 0,
                SourceConstantAlpha: (a.opacity * 255 / 100) as u8,
                AlphaFormat: AC_SRC_ALPHA as u8,
            };
            UpdateLayeredWindow(a.hwnd, screen, null(), &size, mem_dc, &src, 0, &bf, ULW_ALPHA);
            SelectObject(mem_dc, old);
        }
        DeleteObject(bmp);
        DeleteDC(mem_dc);
        ReleaseDC(null_mut(), screen);
    }
}

/// 3x5 pixel glyphs (one row per entry, 3 bits each) for the frame counter.
fn glyph(c: char) -> [u8; 5] {
    match c {
        '0' => [7, 5, 5, 5, 7],
        '1' => [2, 6, 2, 2, 7],
        '2' => [7, 1, 7, 4, 7],
        '3' => [7, 1, 7, 1, 7],
        '4' => [5, 5, 7, 1, 1],
        '5' => [7, 4, 7, 1, 7],
        '6' => [7, 4, 7, 5, 7],
        '7' => [7, 1, 1, 1, 1],
        '8' => [7, 5, 7, 5, 7],
        '9' => [7, 5, 7, 1, 7],
        '/' => [1, 1, 2, 4, 4],
        '.' => [0, 0, 0, 0, 2],
        'x' => [0, 5, 2, 5, 0],
        _ => [0; 5],
    }
}

/// Frame counter ("3/12", plus the speed when it is not 1x) and a progress bar along the top edge.
fn draw_progress(a: &App, dst: &mut [u32]) {
    let (w, h) = (a.w as usize, a.h as usize);
    let k = dpi_px(a, 2).max(2) as usize; // size of one font pixel
    let t = (edge(a) / 2).max(3) as usize;
    let n = a.frames.len();
    let mut text = format!("{}/{}", a.cur + 1, n);
    if a.speed != SPEED_NORMAL {
        text += &format!(" {}x", SPEEDS[a.speed]);
    }
    let (tw, th) = (text.chars().count() * 4 * k - k, 5 * k);
    if w < t + tw + 6 * k + 3 * btn(a) as usize || h < t + 2 * k + th + 4 * k {
        return; // too small to show anything without covering the buttons
    }
    let bar_w = w - 2 * t;
    for y in t..t + k {
        for x in 0..bar_w {
            dst[y * w + t + x] = if x < bar_w * (a.cur + 1) / n { 0xFF3B_82F6 } else { 0x8080_8080 };
        }
    }
    let (x0, y0) = (t + 2 * k, t + 3 * k);
    for y in y0 - k..y0 + th + k {
        dst[y * w + x0 - 2 * k..y * w + x0 + tw + 2 * k].fill(0xB000_0000); // dark chip
    }
    for (i, c) in text.chars().enumerate() {
        for (row, bits) in glyph(c).iter().enumerate() {
            for col in 0..3 {
                if bits >> (2 - col) & 1 != 0 {
                    for dy in 0..k {
                        let start = (y0 + row * k + dy) * w + x0 + (i * 4 + col) * k;
                        dst[start..start + k].fill(0xFFFF_FFFF);
                    }
                }
            }
        }
    }
}

/// Steps one frame (wraps around) and pauses, so the frames can be inspected one by one.
fn step_frame(a: &mut App, forward: bool) {
    let n = a.frames.len();
    if n < 2 {
        return;
    }
    a.paused = true;
    a.cur = if forward { (a.cur + 1) % n } else { (a.cur + n - 1) % n };
    schedule_frame(a); // paused: just stops the timer
    render(a);
}

fn schedule_save(a: &App) {
    unsafe { SetTimer(a.hwnd, T_SAVE, 500, None) }; // re-arming resets the debounce
}

fn schedule_frame(a: &App) {
    unsafe {
        if a.frames.len() > 1 && !a.paused {
            let ms = (a.delays[a.cur] as f32 / SPEEDS[a.speed]).round().max(10.0) as u32;
            SetTimer(a.hwnd, T_FRAME, ms, None);
        } else {
            KillTimer(a.hwnd, T_FRAME);
        }
    }
}

fn natural_size(a: &App) -> (f64, f64) {
    if a.rot % 2 == 1 { (a.gh as f64, a.gw as f64) } else { (a.gw as f64, a.gh as f64) }
}

/// Bounds of the monitor the window is on (or, before it is shown, the one under the cursor).
fn screen_rect(hwnd: HWND) -> RECT {
    unsafe {
        let mon = if IsWindowVisible(hwnd) != 0 {
            MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST)
        } else {
            let mut pt = POINT { x: 0, y: 0 };
            GetCursorPos(&mut pt);
            MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST)
        };
        let mut mi: MONITORINFO = mem::zeroed();
        mi.cbSize = mem::size_of::<MONITORINFO>() as u32;
        if GetMonitorInfoW(mon, &mut mi) != 0 {
            mi.rcMonitor
        } else {
            RECT { left: 0, top: 0, right: GetSystemMetrics(SM_CXSCREEN), bottom: GetSystemMetrics(SM_CYSCREEN) }
        }
    }
}

/// Natural size, scaled down to fit 80% of the screen (and up to a usable minimum for tiny images).
fn fit_size(a: &App) -> (i32, i32) {
    let (mut w, mut h) = natural_size(a);
    // tiny images (icons) would open as a window too small to use: enlarge to at least 128 px
    let min_side = 128.0 * unsafe { GetDpiForWindow(a.hwnd) } as f64 / 96.0;
    if w.max(h) < min_side {
        let k = min_side / w.max(h);
        w *= k;
        h *= k;
    }
    let r = screen_rect(a.hwnd);
    let (sw, sh) = ((r.right - r.left) as f64, (r.bottom - r.top) as f64);
    if w > sw * 0.8 || h > sh * 0.8 {
        let r = (sw * 0.8 / w).min(sh * 0.8 / h);
        w *= r;
        h *= r;
    }
    (w.round() as i32, h.round() as i32)
}

fn resize(a: &mut App, w: i32, h: i32) {
    // upper bound: a huge window makes the DIB allocation fail and the render crawl
    let m = screen_rect(a.hwnd);
    let (sw, sh) = (m.right - m.left, m.bottom - m.top);
    if a.restore.is_some() || w < 50 || h < 50 || w > sw * 3 || h > sh * 3 { // maximized windows keep their size
        return;
    }
    a.w = w;
    a.h = h;
    render(a);
    schedule_save(a);
}

/// Centers the window on the screen, shifted by `offset` px so extra windows cascade.
fn center(a: &App, offset: i32) {
    unsafe {
        let m = screen_rect(a.hwnd);
        let x = m.left + (m.right - m.left - a.w) / 2 + offset;
        let y = m.top + (m.bottom - m.top - a.h) / 2 + offset;
        SetWindowPos(a.hwnd, null_mut(), x, y, 0, 0, SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE);
    }
}

fn show_error(e: &str) {
    unsafe {
        MessageBoxW(null_mut(), wide(e).as_ptr(), wide(&tr("Error")).as_ptr(), MB_OK | MB_ICONERROR);
    }
}

/// Puts a decoded image into the window and fits it to the screen.
fn apply(a: &mut App, path: &str, (gw, gh, frames, delays): Decoded) {
    a.gw = gw;
    a.gh = gh;
    a.frames = frames;
    a.delays = delays;
    a.cur = 0;
    a.rot = 0;
    a.flip_h = false;
    a.flip_v = false;
    a.paused = false;
    a.speed = SPEED_NORMAL;
    a.path = path.to_string();
    a.restore = None;
    (a.w, a.h) = fit_size(a);
    render(a);
    schedule_frame(a);
    set_title(a.hwnd, path);
}

/// Opacity (%) remembered from the last session, used for every new window. The lock is not
/// remembered: a new window always starts unlocked so it can be moved right away.
fn saved_opacity() -> u32 {
    let saved = fs::read_to_string(exe_file("settings.json")).unwrap_or_default();
    json_val(&saved, "Opacity")
        .and_then(|v| v.parse::<f64>().ok())
        .map_or(100, |o| (o * 100.0).round().clamp(10.0, 100.0) as u32)
}

// ---------- session (session.txt): the open windows, reopened at the next start ----------

/// What is remembered about one window.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Saved {
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    opacity: u32,
    locked: bool,
    rot: u8,
    flip_h: bool,
    flip_v: bool,
}

/// One line per window: x, y, w, h, opacity, locked, rotation, flip h, flip v, then the path (last, so it may hold tabs).
fn session_line(path: &str, s: &Saved) -> String {
    format!("{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}", s.x, s.y, s.w, s.h, s.opacity, s.locked as u8, s.rot, s.flip_h as u8, s.flip_v as u8, path)
}

/// Lines that do not parse are skipped, so a damaged file loses only that window.
fn parse_session(text: &str) -> Vec<(String, Saved)> {
    text.lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.trim_start_matches('\u{feff}').splitn(10, '\t').collect();
            let n = |i: usize| f.get(i)?.parse::<i32>().ok();
            let s = Saved {
                x: n(0)?,
                y: n(1)?,
                w: n(2)?,
                h: n(3)?,
                opacity: n(4)?.clamp(10, 100) as u32,
                locked: n(5)? != 0,
                rot: (n(6)?.rem_euclid(4)) as u8,
                flip_h: n(7)? != 0,
                flip_v: n(8)? != 0,
            };
            Some((f.get(9).filter(|p| !p.is_empty())?.to_string(), s))
        })
        .collect()
}

/// Writes the open windows. Skipped while images are still loading (at startup the other windows
/// of the last session are on their way and would be forgotten).
fn save_session() {
    if dash().restore && dash().loading.is_empty() {
        let lines: Vec<String> = overlays().iter().map(|&p| unsafe { window_state(&*p) }).collect();
        write_atomic("session.txt", lines.join("\n").as_bytes());
    }
}

unsafe fn window_state(a: &App) -> String {
    let mut r: RECT = mem::zeroed();
    GetWindowRect(a.hwnd, &mut r);
    if IsIconic(a.hwnd) != 0 {
        // a minimized window sits off screen: its restored place is in the window placement
        let mut wp: WINDOWPLACEMENT = mem::zeroed();
        wp.length = mem::size_of::<WINDOWPLACEMENT>() as u32;
        GetWindowPlacement(a.hwnd, &mut wp);
        r = wp.rcNormalPosition;
    }
    // maximized: remember the size and place the window had before
    let (x, y, w, h) = a.restore.unwrap_or((r.left, r.top, a.w, a.h));
    let s = Saved { x, y, w, h, opacity: a.opacity, locked: a.locked, rot: a.rot, flip_h: a.flip_h, flip_v: a.flip_v };
    session_line(&a.path, &s)
}

static HIDDEN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Hides every image window, or brings them all back (Ctrl+Alt+G anywhere, or Window menu).
fn toggle_all() {
    use std::sync::atomic::Ordering::Relaxed;
    let hide = !HIDDEN.load(Relaxed) && !overlays().is_empty();
    for &p in overlays().iter() {
        unsafe { ShowWindow((*p).hwnd, if hide { SW_HIDE } else { SW_SHOWNA }) };
    }
    HIDDEN.store(hide, Relaxed);
}

/// Opens an already decoded image in a new overlay window; `state` puts it back as it was.
fn create_overlay(path: &str, d: Decoded, state: Option<&Saved>) {
    if HIDDEN.load(std::sync::atomic::Ordering::Relaxed) {
        toggle_all(); // a new window brings the hidden ones back
    }
    let opacity = saved_opacity();
    unsafe {
        let a = Box::into_raw(Box::new(App {
            hwnd: null_mut(),
            frames: Vec::new(),
            delays: Vec::new(),
            gw: 0,
            gh: 0,
            cur: 0,
            paused: false,
            rot: 0,
            flip_h: false,
            flip_v: false,
            w: 300,
            h: 300,
            opacity,
            locked: false,
            hover: false,
            hot: Zone::None,
            drag: None,
            path: String::new(),
            restore: None,
            speed: SPEED_NORMAL,
        }));
        let hwnd = CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_APPWINDOW,
            wide("GifOverlayWnd").as_ptr(), wide("GIF Overlay").as_ptr(), WS_POPUP | WS_MINIMIZEBOX | WS_SYSMENU,
            0, 0, 300, 300,
            null_mut(), null_mut(), GetModuleHandleW(null()), null(),
        );
        (*a).hwnd = hwnd;
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, a as isize);
        let offset = overlays().len() as i32 % 8 * 32;
        overlays().push(a);
        DragAcceptFiles(hwnd, 1);
        apply(&mut *a, path, d);
        let mut placed = false;
        if let Some(s) = state {
            let m = screen_rect(hwnd);
            if s.w >= 50 && s.h >= 50 && s.w <= (m.right - m.left) * 3 && s.h <= (m.bottom - m.top) * 3 {
                let app = &mut *a;
                (app.opacity, app.locked, app.rot, app.flip_h, app.flip_v, app.w, app.h) = (s.opacity, s.locked, s.rot, s.flip_h, s.flip_v, s.w, s.h);
                // only trust the position if its middle is still on a monitor (it may have been unplugged)
                let mid = POINT { x: s.x + s.w / 2, y: s.y + s.h / 2 };
                if !MonitorFromPoint(mid, MONITOR_DEFAULTTONULL).is_null() {
                    SetWindowPos(hwnd, null_mut(), s.x, s.y, 0, 0, SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE);
                    placed = true;
                }
                render(app);
            }
        }
        apply_lock(&*a);
        if !placed {
            center(&*a, offset);
        }
        ShowWindow(hwnd, SW_SHOW);
        if state.is_none() {
            SetForegroundWindow(hwnd); // a restored window must not steal focus from what the user is doing
        }
    }
}

/// Minimizes to the taskbar (the window has a taskbar button via WS_EX_APPWINDOW).
fn minimize(a: &mut App) {
    (a.hover, a.hot) = (false, Zone::None);
    unsafe { ShowWindow(a.hwnd, SW_MINIMIZE) };
}

/// Brings the window back from minimized / hidden and to the front.
fn show_overlay(a: &mut App) {
    unsafe {
        ShowWindow(a.hwnd, SW_RESTORE);
        SetForegroundWindow(a.hwnd);
    }
}

/// Stretches the image over the whole monitor it is on (aspect ratio is not kept); a second click restores size and position.
fn toggle_max(a: &mut App) {
    if let Some((x, y, w, h)) = a.restore.take() {
        (a.w, a.h) = (w, h);
        unsafe { SetWindowPos(a.hwnd, null_mut(), x, y, 0, 0, SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE) };
        render(a);
        schedule_save(a);
        return;
    }
    let mut r: RECT = unsafe { mem::zeroed() };
    unsafe { GetWindowRect(a.hwnd, &mut r) };
    let m = screen_rect(a.hwnd);
    a.restore = Some((r.left, r.top, a.w, a.h));
    (a.w, a.h) = (m.right - m.left, m.bottom - m.top);
    unsafe { SetWindowPos(a.hwnd, null_mut(), m.left, m.top, 0, 0, SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE) };
    render(a);
}

const WM_LOADED: u32 = WM_APP + 2;

/// Result of a background decode, posted to the dashboard window.
struct Loaded {
    path: String,
    target: usize, // overlay window to put the image in (0 = open a new window)
    res: Result<Decoded, String>,
    state: Option<Saved>, // set when the window is being restored from the last session
}

fn set_title(hwnd: HWND, path: &str) {
    let name = Path::new(path).file_name().map_or(path.to_string(), |n| n.to_string_lossy().into_owned());
    unsafe { SetWindowTextW(hwnd, wide(&name).as_ptr()) }; // tells windows apart in the taskbar
}

/// Decodes on a worker thread so a big GIF cannot freeze the UI; `finish_load` runs when it is done.
fn start_load(path: &str, target: HWND) {
    start_load_with(path, target, None);
}

fn start_load_with(path: &str, target: HWND, state: Option<Saved>) {
    let path = abs_path(path);
    let d = dash();
    if d.loading.iter().any(|p| p.eq_ignore_ascii_case(&path)) {
        return; // the same file is already on its way
    }
    d.loading.push(path.clone());
    unsafe {
        SetWindowTextW(d.hwnd, wide(&tr("GIF Overlay - loading...")).as_ptr());
        if !target.is_null() {
            SetWindowTextW(target, wide(&tr("Loading...")).as_ptr());
        }
    }
    let (dash_hwnd, target) = (d.hwnd as usize, target as usize);
    std::thread::spawn(move || {
        use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_MULTITHREADED};
        let com = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.is_ok(); // WIC needs COM on this thread
        // a decoder bug on a bad file must not take every window down with it
        let res = std::panic::catch_unwind(|| decode(&path, usize::MAX))
            .unwrap_or_else(|_| Err(tr("the decoder crashed on this file")));
        if com {
            unsafe { CoUninitialize() };
        }
        let msg = Box::into_raw(Box::new(Loaded { path, target, res, state }));
        if unsafe { PostMessageW(dash_hwnd as HWND, WM_LOADED, 0, msg as isize) } == 0 {
            drop(unsafe { Box::from_raw(msg) }); // dashboard already gone: the app is quitting
        }
    });
}

/// Back on the UI thread: shows the decoded image, or reports the error.
unsafe fn finish_load(l: Loaded) {
    let d = dash();
    d.loading.retain(|p| !p.eq_ignore_ascii_case(&l.path));
    if d.loading.is_empty() {
        SetWindowTextW(d.hwnd, wide("GIF Overlay").as_ptr());
    }
    let target = overlays().iter().copied().find(|&p| l.target != 0 && (*p).hwnd as usize == l.target);
    match l.res {
        Ok(img) => {
            if l.state.is_none() {
                history_push(&l.path); // restoring a session must not reshuffle the history
            }
            if l.target != 0 {
                if let Some(p) = target {
                    apply(&mut *p, &l.path, img);
                } // else: that window was closed meanwhile, drop the image
                if IsWindowVisible(d.hwnd) != 0 {
                    refresh_dash(); // the "open" marks changed
                }
            } else {
                if let Some(p) = overlays().iter().copied().find(|&p| (*p).path.eq_ignore_ascii_case(&l.path)) {
                    show_overlay(&mut *p);
                } else {
                    create_overlay(&l.path, img, l.state.as_ref());
                }
                if IsWindowVisible(d.hwnd) != 0 {
                    refresh_dash(); // the dashboard stays open: update the "open" marks and the order
                }
            }
        }
        Err(e) => {
            if let Some(p) = target {
                set_title((*p).hwnd, &(*p).path); // undo "Loading..."
            }
            if l.state.is_none() {
                show_error(&trf("Failed to load image: {}", &[&e])); // a file of the last session that no longer opens is skipped quietly
                history_remove(&l.path);
            }
            if overlays().is_empty() && d.loading.is_empty() {
                show_dash(); // never leave the app without a window
            } else if IsWindowVisible(d.hwnd) != 0 {
                refresh_dash();
            }
        }
    }
    save_session();
}

/// Opens `path` in a new window (or brings the window that already shows it to the front).
/// New images are decoded in the background; `finish_load` records them; a visible dashboard stays open.
fn open_gif(path: &str) {
    let path = abs_path(path);
    let existing = overlays().iter().copied().find(|&p| unsafe { (*p).path.eq_ignore_ascii_case(&path) });
    if let Some(p) = existing {
        show_overlay(unsafe { &mut *p });
        history_push(&path);
        if unsafe { IsWindowVisible(dash().hwnd) } != 0 {
            refresh_dash();
        }
    } else {
        start_load(&path, null_mut());
    }
}

// ---------- paste: image, file or image link from the clipboard ----------

const CF_DIB: u32 = 8;
const CF_UNICODETEXT: u32 = 13;
const CF_HDROP: u32 = 15;
const WM_FETCHED: u32 = WM_APP + 3;

/// Pasted and downloaded images live here (next to the exe, like history.txt) so the history can reopen them.
fn pasted_file(name: &str) -> Option<PathBuf> {
    let dir = exe_file("pasted");
    fs::create_dir_all(&dir).ok()?;
    let ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis());
    Some(dir.join(format!("{ms}-{name}")))
}

enum Clip {
    File(String),
    Url(String),
    Image(Vec<u8>, &'static str), // file bytes + extension
}

unsafe fn clip_bytes(h: HANDLE) -> Option<Vec<u8>> {
    let p = GlobalLock(h) as *const u8;
    if p.is_null() {
        return None;
    }
    let v = slice::from_raw_parts(p, GlobalSize(h)).to_vec();
    GlobalUnlock(h);
    Some(v)
}

/// A clipboard DIB is a BMP file minus its 14-byte header; put it back so WIC can decode it.
fn dib_to_bmp(dib: &[u8]) -> Option<Vec<u8>> {
    let u32_at = |i: usize| Some(u32::from_le_bytes(dib.get(i..i + 4)?.try_into().ok()?) as usize);
    let (size, comp, used) = (u32_at(0)?, u32_at(16)?, u32_at(32)?);
    let bits = u16::from_le_bytes(dib.get(14..16)?.try_into().ok()?) as usize;
    let masks = if comp == 3 && size == 40 { 12 } else { 0 }; // BI_BITFIELDS stores 3 masks after the header
    let palette = (if used > 0 { used } else if bits <= 8 { 1 << bits } else { 0 }) * 4;
    let mut bmp = b"BM".to_vec();
    bmp.extend_from_slice(&(14 + dib.len() as u32).to_le_bytes());
    bmp.extend_from_slice(&[0; 4]);
    bmp.extend_from_slice(&((14 + size + masks + palette) as u32).to_le_bytes());
    bmp.extend_from_slice(dib);
    Some(bmp)
}

/// What the clipboard holds, best first: a copied file, a PNG (keeps transparency), a bitmap, or text
/// that is an http(s) link / an existing file path.
unsafe fn read_clipboard() -> Option<Clip> {
    if OpenClipboard(null_mut()) == 0 {
        return None;
    }
    let r = (|| {
        if IsClipboardFormatAvailable(CF_HDROP) != 0 {
            if let Some(p) = drop_path(GetClipboardData(CF_HDROP) as HDROP, 0) {
                return Some(Clip::File(p));
            }
        }
        let png = RegisterClipboardFormatW(wide("PNG").as_ptr());
        if IsClipboardFormatAvailable(png) != 0 {
            return clip_bytes(GetClipboardData(png)).map(|b| Clip::Image(b, "png"));
        }
        if IsClipboardFormatAvailable(CF_DIB) != 0 {
            return clip_bytes(GetClipboardData(CF_DIB)).and_then(|b| dib_to_bmp(&b)).map(|b| Clip::Image(b, "bmp"));
        }
        if IsClipboardFormatAvailable(CF_UNICODETEXT) != 0 {
            let b = clip_bytes(GetClipboardData(CF_UNICODETEXT))?;
            let u: Vec<u16> = b.chunks(2).filter(|c| c.len() == 2).map(|c| u16::from_le_bytes([c[0], c[1]])).take_while(|&c| c != 0).collect();
            let t = String::from_utf16_lossy(&u).trim().trim_matches('"').to_string();
            if (t.starts_with("http://") || t.starts_with("https://")) && !t.contains(char::is_whitespace) {
                return Some(Clip::Url(t));
            }
            return Path::new(&t).is_file().then_some(Clip::File(t));
        }
        None
    })();
    CloseClipboard();
    r
}

/// Ctrl+V / menu: opens whatever image the clipboard offers in a new window.
fn paste() {
    match unsafe { read_clipboard() } {
        Some(Clip::File(p)) => open_gif(&p),
        Some(Clip::Url(u)) => fetch(u),
        Some(Clip::Image(bytes, ext)) => match pasted_file(&format!("paste.{ext}")).filter(|p| fs::write(p, &bytes).is_ok()) {
            Some(p) => open_gif(&p.to_string_lossy()),
            None => show_error(&tr("Cannot save the pasted image next to the app.")),
        },
        None => show_error(&tr("The clipboard has no image, image file or image link.")),
    }
}

/// Downloads `url` on a worker thread (shortcut: no size limit or cancel, add if big links hang the title).
fn fetch(url: String) {
    unsafe { SetWindowTextW(dash().hwnd, wide(&tr("GIF Overlay - downloading...")).as_ptr()) };
    let dash_hwnd = dash().hwnd as usize;
    std::thread::spawn(move || {
        use windows::{core::HSTRING, Win32::System::Com::Urlmon::URLDownloadToFileW};
        let name: String = url.split(['?', '#']).next().unwrap_or("").rsplit('/').next().unwrap_or("")
            .chars().filter(|c| c.is_ascii_alphanumeric() || ".-_".contains(*c)).take(60).collect();
        let res: Result<String, String> = pasted_file(if name.is_empty() { "image" } else { &name })
            .ok_or(tr("Cannot create the pasted folder next to the app."))
            .and_then(|p| {
                unsafe { URLDownloadToFileW(None, &HSTRING::from(url.as_str()), &HSTRING::from(p.as_os_str()), 0, None) }
                    .map(|_| p.to_string_lossy().into_owned())
                    .map_err(|e| trf("Download failed: {}", &[&e.message()]))
            });
        let msg = Box::into_raw(Box::new(res));
        if unsafe { PostMessageW(dash_hwnd as HWND, WM_FETCHED, 0, msg as isize) } == 0 {
            drop(unsafe { Box::from_raw(msg) }); // dashboard already gone: the app is quitting
        }
    });
}

// ---------- copy to the clipboard ----------

unsafe fn set_clip(format: u32, bytes: &[u8]) {
    let h = GlobalAlloc(GMEM_MOVEABLE, bytes.len());
    if h.is_null() {
        return;
    }
    let p = GlobalLock(h) as *mut u8;
    if p.is_null() {
        GlobalFree(h);
        return;
    }
    std::ptr::copy_nonoverlapping(bytes.as_ptr(), p, bytes.len());
    GlobalUnlock(h);
    if SetClipboardData(format, h).is_null() {
        GlobalFree(h); // the clipboard owns the memory only when this succeeds
    }
}

fn copy_text(owner: HWND, s: &str) {
    let bytes: Vec<u8> = wide(s).iter().flat_map(|c| c.to_le_bytes()).collect();
    unsafe {
        if OpenClipboard(owner) != 0 {
            EmptyClipboard();
            set_clip(CF_UNICODETEXT, &bytes);
            CloseClipboard();
        }
    }
}

/// Copies the frame on screen at its original size (rotation / flip are not applied) as PNG, which
/// keeps transparency, plus a plain bitmap on white for apps that only read CF_DIB.
fn copy_image(a: &App) {
    let Some(f) = a.frames.get(a.cur) else { return };
    let (w, h) = (a.gw, a.gh);
    let mut rgba = Vec::with_capacity(f.len() * 4);
    let mut dib = Vec::with_capacity(40 + f.len() * 4);
    for v in [40u32, w as u32, h as u32, 1 | 32 << 16, 0, 0, 0, 0, 0, 0] {
        dib.extend_from_slice(&v.to_le_bytes()); // BITMAPINFOHEADER: 32 bpp, height > 0 = bottom-up
    }
    for p in f {
        let al = p >> 24;
        let c = |s: u32| (p >> s) & 0xFF;
        let un = |s: u32| (c(s) * 255 + al / 2).checked_div(al).map_or(0, |v| v.min(255) as u8); // premultiplied -> straight
        rgba.extend_from_slice(&[un(16), un(8), un(0), al as u8]);
    }
    for row in f.chunks(w).rev() {
        for p in row {
            let al = p >> 24;
            for s in [0, 8, 16] {
                dib.push((((p >> s) & 0xFF) + (255 - al)).min(255) as u8); // over white
            }
            dib.push(255);
        }
    }
    let mut png_bytes = Vec::new();
    let ok = (|| {
        let mut enc = png::Encoder::new(&mut png_bytes, w as u32, h as u32);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        enc.write_header().ok()?.write_image_data(&rgba).ok()
    })()
    .is_some();
    unsafe {
        if OpenClipboard(a.hwnd) != 0 {
            EmptyClipboard();
            if ok {
                set_clip(RegisterClipboardFormatW(wide("PNG").as_ptr()), &png_bytes);
            }
            set_clip(CF_DIB, &dib);
            CloseClipboard();
        }
    }
}

fn show_in_folder(path: &str) {
    let args = wide(&format!("/select,\"{path}\""));
    unsafe { ShellExecuteW(null_mut(), wide("open").as_ptr(), wide("explorer.exe").as_ptr(), args.as_ptr(), null(), SW_SHOWNORMAL) };
}
// ---------- version / update check ----------

const VERSION: &str = env!("CARGO_PKG_VERSION"); // stamped from the release tag by the pipeline
const RELEASES_URL: &str = "https://github.com/n3d06/GIF-Overlay/releases/latest";
const WM_UPDATE: u32 = WM_APP + 4;
static CHECKING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn version_parts(v: &str) -> Vec<u32> {
    v.trim_start_matches('v').split('.').map(|p| p.parse().unwrap_or(0)).collect()
}

/// Asks GitHub for the latest release on a worker thread (only when the user picks Help > Check for
/// updates); the tag comes back to the dashboard as WM_UPDATE.
fn check_update() {
    if CHECKING.swap(true, std::sync::atomic::Ordering::SeqCst) {
        return; // already checking
    }
    let dash_hwnd = dash().hwnd as usize;
    std::thread::spawn(move || {
        use windows::{core::HSTRING, Win32::System::Com::Urlmon::URLDownloadToFileW};
        let res: Result<String, String> = (|| {
            let ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis());
            let tmp = std::env::temp_dir().join(format!("gif-overlay-latest-{ms}.json"));
            // the query string only defeats the download cache
            let url = format!("https://api.github.com/repos/n3d06/GIF-Overlay/releases/latest?t={ms}");
            unsafe { URLDownloadToFileW(None, &HSTRING::from(url.as_str()), &HSTRING::from(tmp.as_os_str()), 0, None) }
                .map_err(|e| e.message())?;
            let json = fs::read_to_string(&tmp).map_err(|e| e.to_string());
            let _ = fs::remove_file(&tmp);
            json_val(&json?, "tag_name").map(|t| t.trim_matches('"').to_string()).ok_or(tr("no release found"))
        })();
        let msg = Box::into_raw(Box::new(res));
        if unsafe { PostMessageW(dash_hwnd as HWND, WM_UPDATE, 0, msg as isize) } == 0 {
            drop(unsafe { Box::from_raw(msg) });
        }
    });
}

unsafe fn show_update_result(hwnd: HWND, res: Result<String, String>) {
    CHECKING.store(false, std::sync::atomic::Ordering::SeqCst);
    let title = wide(&tr("Check for updates"));
    match res {
        Ok(tag) if version_parts(&tag) > version_parts(VERSION) => {
            let q = wide(&trf("Version {} is available (you have {}).\nOpen the download page?", &[tag.trim_start_matches('v'), VERSION]));
            if MessageBoxW(hwnd, q.as_ptr(), title.as_ptr(), MB_YESNO | MB_ICONINFORMATION) == IDYES {
                ShellExecuteW(null_mut(), wide("open").as_ptr(), wide(RELEASES_URL).as_ptr(), null(), null(), SW_SHOWNORMAL);
            }
        }
        Ok(_) => {
            let m = wide(&trf("You are up to date (version {}).", &[VERSION]));
            MessageBoxW(hwnd, m.as_ptr(), title.as_ptr(), MB_OK | MB_ICONINFORMATION);
        }
        Err(e) => {
            let m = wide(&trf("Could not check for updates: {}", &[&e]));
            MessageBoxW(hwnd, m.as_ptr(), title.as_ptr(), MB_OK | MB_ICONERROR);
        }
    }
}


fn rotate(a: &mut App, dir: u8) {
    a.rot = (a.rot + dir) % 4;
    if a.restore.is_none() {
        mem::swap(&mut a.w, &mut a.h); // a maximized window keeps filling the screen
    }
    render(a);
    schedule_save(a);
}

/// A locked window is click-through (WS_EX_TRANSPARENT), so it never gets in the way. Since it then
/// receives no mouse input, a timer lets the user poke through the lock by holding Ctrl over it,
/// and click to unlock (the right-click menu works too).
fn apply_lock(a: &App) {
    unsafe {
        if a.locked {
            SetTimer(a.hwnd, T_LOCK, 80, None);
        } else {
            KillTimer(a.hwnd, T_LOCK);
        }
        set_click_through(a.hwnd, a.locked);
    }
}

unsafe fn set_click_through(hwnd: HWND, on: bool) {
    let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
    let new = if on { ex | WS_EX_TRANSPARENT as isize } else { ex & !(WS_EX_TRANSPARENT as isize) };
    if new != ex {
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, new);
    }
}

/// Timer tick of a locked window: interactive only while Ctrl is held with the cursor over it.
unsafe fn lock_tick(a: &App) {
    let (mut pt, mut r) = (POINT { x: 0, y: 0 }, mem::zeroed::<RECT>());
    GetCursorPos(&mut pt);
    GetWindowRect(a.hwnd, &mut r);
    let poke = GetAsyncKeyState(VK_CONTROL as i32) < 0 && PtInRect(&r, pt) != 0;
    set_click_through(a.hwnd, !poke);
}

fn command(a: &mut App, id: usize) {
    match id {
        ID_LOCK => {
            a.locked = !a.locked;
            apply_lock(a);
            render(a); // show / hide the handles and the X right away
            schedule_save(a);
        }
        ID_OPEN => {
            if let Some(p) = open_dialog(a.hwnd, false).first() {
                start_load(p, a.hwnd);
            }
        }
        ID_HISTORY => show_dash(),
        ID_PASTE => paste(),
        ID_COPY => copy_image(a),
        ID_FOLDER => show_in_folder(&a.path),
        ID_TOPMOST => unsafe {
            let top = GetWindowLongPtrW(a.hwnd, GWL_EXSTYLE) & WS_EX_TOPMOST as isize != 0;
            SetWindowPos(a.hwnd, if top { HWND_NOTOPMOST } else { HWND_TOPMOST }, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
        },
        ID_QUIT => unsafe {
            save_settings(a);
            PostQuitMessage(0); // process exit tears down every overlay
        },
        ID_ROT_L => rotate(a, 3),
        ID_ROT_R => rotate(a, 1),
        ID_FLIP_H => {
            a.flip_h = !a.flip_h;
            render(a);
        }
        ID_FLIP_V => {
            a.flip_v = !a.flip_v;
            render(a);
        }
        ID_RESET => {
            a.rot = 0;
            a.flip_h = false;
            a.flip_v = false;
            a.opacity = 100;
            a.restore = None;
            (a.w, a.h) = fit_size(a);
            render(a);
            schedule_save(a);
        }
        ID_PAUSE => {
            a.paused = !a.paused;
            schedule_frame(a);
        }
        // Closes this image only (the app quits when the last one goes and no dashboard is showing).
        // WM_DESTROY frees `a`, so nothing may touch it after DestroyWindow.
        ID_EXIT => unsafe {
            save_settings(a);
            DestroyWindow(a.hwnd);
        },
        i if (ID_SCALE..ID_SCALE + SCALES.len()).contains(&i) => {
            let (nw, nh) = natural_size(a);
            let s = SCALES[i - ID_SCALE] as f64 / 100.0;
            resize(a, (nw * s).round() as i32, (nh * s).round() as i32);
        }
        i if (ID_SPEED..ID_SPEED + SPEEDS.len()).contains(&i) => {
            a.speed = i - ID_SPEED;
            schedule_frame(a);
            render(a);
        }
        i if (ID_OPACITY..ID_OPACITY + OPACITIES.len()).contains(&i) => {
            a.opacity = OPACITIES[i - ID_OPACITY];
            render(a);
            schedule_save(a);
        }
        _ => {}
    }
}

unsafe fn app_of(hwnd: HWND) -> Option<&'static mut App> {
    (GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut App).as_mut()
}

fn context_menu(hwnd: HWND) {
    unsafe {
        // Copy what the menu needs: it runs a nested message loop in which timers keep
        // firing (and the window could even be closed), so no reference may be held across it.
        let Some(a) = app_of(hwnd) else { return };
        let (opacity, locked, flip_h, flip_v, paused) = (a.opacity, a.locked, a.flip_h, a.flip_v, a.paused);
        let animated = a.frames.len() > 1;
        let speed = a.speed;
        let topmost = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) & WS_EX_TOPMOST as isize != 0;
        let add =|m: HMENU, flags: u32, id: usize, text: &str| {
            AppendMenuW(m, flags, id, wide(&tr(text)).as_ptr());
        };
        let check = |on: bool| if on { MF_CHECKED } else { 0 };
        let menu = CreatePopupMenu();
        let (scale, opac, xform, settings) = (CreatePopupMenu(), CreatePopupMenu(), CreatePopupMenu(), CreatePopupMenu());
        for (i, s) in SCALES.iter().enumerate() {
            add(scale, MF_STRING, ID_SCALE + i, &format!("{s}%"));
        }
        for (i, o) in OPACITIES.iter().enumerate() {
            add(opac, MF_STRING | check(opacity == *o), ID_OPACITY + i, &format!("{o}%"));
        }
        add(xform, MF_STRING, ID_ROT_L, "Rotate Left");
        add(xform, MF_STRING, ID_ROT_R, "Rotate Right");
        add(xform, MF_STRING | check(flip_h), ID_FLIP_H, "Flip Horizontal");
        add(xform, MF_STRING | check(flip_v), ID_FLIP_V, "Flip Vertical");
        add(settings, MF_POPUP, scale as usize, "Scale");
        add(settings, MF_POPUP, opac as usize, "Opacity");
        add(settings, MF_POPUP, xform as usize, "Transform");
        add(settings, MF_SEPARATOR, 0, "");
        add(settings, MF_STRING, ID_RESET, "Reset to Default");

        add(menu, MF_STRING | check(locked), ID_LOCK, "Lock Window (Ctrl+click to unlock)");
        add(menu, MF_SEPARATOR, 0, "");
        add(menu, MF_STRING, ID_HISTORY, "Dashboard");
        add(menu, MF_STRING, ID_OPEN, "Replace this image...");
        add(menu, MF_STRING, ID_PASTE, "Paste image or link\tCtrl+V");
        add(menu, MF_STRING, ID_COPY, "Copy image\tCtrl+C");
        add(menu, MF_STRING, ID_FOLDER, "Show in folder");
        add(menu, MF_SEPARATOR, 0, "");
        add(menu, MF_POPUP, settings as usize, "Settings");
        add(menu, MF_STRING | check(topmost), ID_TOPMOST, "Always on Top");
        if animated {
            let spd = CreatePopupMenu();
            for (i, s) in SPEEDS.iter().enumerate() {
                add(spd, MF_STRING | check(speed == i), ID_SPEED + i, &format!("{s}x"));
            }
            add(menu, MF_STRING | check(paused), ID_PAUSE, "Pause / Play\tSpace");
            add(menu, MF_POPUP, spd as usize, "Speed\t- / +");
        }
        add(menu, MF_SEPARATOR, 0, "");
        add(menu, MF_STRING, ID_EXIT, "Close window\tEsc");
        add(menu, MF_STRING, ID_QUIT, "Quit GIF Overlay");

        let mut pt = POINT { x: 0, y: 0 };
        GetCursorPos(&mut pt);
        SetForegroundWindow(hwnd);
        let id = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_RIGHTBUTTON, pt.x, pt.y, 0, hwnd, null());
        DestroyMenu(menu); // also destroys the submenus
        if id > 0 {
            if let Some(a) = app_of(hwnd) {
                command(a, id as usize);
            }
        }
    }
}

unsafe fn track_leave(hwnd: HWND) {
    let mut t = TRACKMOUSEEVENT { cbSize: mem::size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: hwnd, dwHoverTime: 0 };
    TrackMouseEvent(&mut t);
}

unsafe extern "system" fn overlay_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    // messages sent while the window is being created / after it is destroyed have no App yet
    let Some(a) = app_of(hwnd) else { return DefWindowProcW(hwnd, msg, wp, lp) };
    match msg {
        WM_LBUTTONDOWN => {
            if a.locked {
                // only reachable while Ctrl is held over a locked window (see lock_tick)
                command(a, ID_LOCK);
                return 0;
            }
            let zone = zone_at(a, lp as i16 as i32, (lp >> 16) as i16 as i32);
            if !a.locked {
                if zone != Zone::None {
                    let mut pt = POINT { x: 0, y: 0 };
                    GetCursorPos(&mut pt);
                    a.drag = Some((zone, pt.x, pt.y, a.w, a.h));
                    SetCapture(hwnd);
                } else if a.restore.is_none() { // a maximized window stays put
                    ReleaseCapture();
                    SendMessageW(hwnd, WM_NCLBUTTONDOWN, HTCAPTION as usize, 0);
                }
            }
            0
        }
        WM_MOUSEMOVE => {
            if let Some((Zone::Close | Zone::Max | Zone::Min, ..)) = a.drag {
                // pressed on a button: it only fires if released still on it
                let zone = zone_at(a, lp as i16 as i32, (lp >> 16) as i16 as i32);
                if zone != a.hot {
                    a.hot = zone;
                    render(a);
                }
            } else if let Some((zone, mx, my, w0, h0)) = a.drag {
                let mut pt = POINT { x: 0, y: 0 };
                GetCursorPos(&mut pt);
                let (nw, nh) = match zone {
                    Zone::Right => (w0 + pt.x - mx, h0),
                    Zone::Bottom => (w0, h0 + pt.y - my),
                    _ => {
                        // corner keeps the aspect ratio: follow whichever axis moved further
                        let k = ((w0 + pt.x - mx) as f64 / w0 as f64).max((h0 + pt.y - my) as f64 / h0 as f64);
                        ((w0 as f64 * k).round() as i32, (h0 as f64 * k).round() as i32)
                    }
                };
                resize(a, nw, nh);
            } else if !a.frames.is_empty() {
                let zone = zone_at(a, lp as i16 as i32, (lp >> 16) as i16 as i32);
                if !a.hover || zone != a.hot {
                    if !a.hover {
                        track_leave(hwnd);
                    }
                    a.hover = true;
                    a.hot = zone;
                    render(a);
                }
            }
            0
        }
        WM_MOUSELEAVE => {
            a.hover = false;
            a.hot = Zone::None;
            if a.drag.is_none() && !a.frames.is_empty() {
                render(a);
            }
            0
        }
        WM_LBUTTONUP => {
            if let Some((zone, ..)) = a.drag.take() {
                ReleaseCapture();
                if zone_at(a, lp as i16 as i32, (lp >> 16) as i16 as i32) == zone {
                    match zone {
                        Zone::Close => {
                            command(a, ID_EXIT); // destroys the window: `a` is gone after this
                            return 0;
                        }
                        Zone::Max => toggle_max(a),
                        Zone::Min => minimize(a),
                        _ => {}
                    }
                }
                track_leave(hwnd); // the cursor may have ended up outside the window
            }
            0
        }
        WM_EXITSIZEMOVE => {
            schedule_save(a); // the window was dragged to a new place
            0
        }
        WM_CAPTURECHANGED => {
            a.drag = None;
            0
        }
        WM_SETCURSOR => {
            let mut pt = POINT { x: 0, y: 0 };
            GetCursorPos(&mut pt);
            ScreenToClient(hwnd, &mut pt);
            let zone = a.drag.map_or_else(|| zone_at(a, pt.x, pt.y), |d| d.0);
            let cursor = match zone {
                Zone::Corner => IDC_SIZENWSE,
                Zone::Right => IDC_SIZEWE,
                Zone::Bottom => IDC_SIZENS,
                Zone::Close | Zone::Max | Zone::Min => IDC_HAND,
                Zone::None => null(),
            };
            if !a.locked && !cursor.is_null() {
                SetCursor(LoadCursorW(null_mut(), cursor));
                1
            } else {
                DefWindowProcW(hwnd, msg, wp, lp)
            }
        }
        WM_RBUTTONUP => {
            context_menu(hwnd);
            0
        }
        WM_KEYDOWN => {
            let first = lp & (1 << 30) == 0; // ignore auto-repeat for the one-shot keys
            match wp as u16 {
                VK_ESCAPE if first => {
                    command(a, ID_EXIT); // destroys the window: `a` is gone after this
                    return 0;
                }
                VK_SPACE if first && a.frames.len() > 1 => command(a, ID_PAUSE),
                0x25 | 0x27 => step_frame(a, wp == 0x27), // Left / Right: previous / next frame
                0xBD | 0xBB | 0x6D | 0x6B if a.frames.len() > 1 => {
                    // - / + (main row or numpad): slower / faster
                    let up = matches!(wp, 0xBB | 0x6B);
                    command(a, ID_SPEED + if up { (a.speed + 1).min(SPEEDS.len() - 1) } else { a.speed.saturating_sub(1) });
                }
                VK_UP | VK_DOWN => {
                    let step = if wp as u16 == VK_UP { 10 } else { -10 };
                    a.opacity = (a.opacity as i32 + step).clamp(10, 100) as u32;
                    render(a);
                    schedule_save(a);
                }
                0x43 if first && GetAsyncKeyState(VK_CONTROL as i32) < 0 => command(a, ID_COPY), // Ctrl+C
                _ => {}
            }
            0
        }
        WM_DROPFILES => {
            // dropping on an overlay swaps its image; the dashboard is where new windows are opened
            if let Some(p) = dropped_files(wp).first() {
                start_load(p, a.hwnd);
            }
            0
        }
        WM_MOUSEWHEEL => {
            if wp & 0x4 != 0 { // MK_SHIFT: opacity
                let step = if (wp >> 16) as i16 > 0 { 10 } else { -10 };
                a.opacity = (a.opacity as i32 + step).clamp(10, 100) as u32;
                render(a);
                schedule_save(a);
            }
            if wp & 0x8 != 0 { // MK_CONTROL
                let k = if (wp >> 16) as i16 > 0 { 1.1 } else { 0.9 };
                resize(a, (a.w as f64 * k).round() as i32, (a.h as f64 * k).round() as i32);
            }
            0
        }
        WM_TIMER => {
            match wp {
                T_FRAME if !a.frames.is_empty() => {
                    a.cur = (a.cur + 1) % a.frames.len();
                    if IsIconic(hwnd) == 0 {
                        render(a); // a minimized window needs no redraw, just keep the animation clock running
                    }
                    schedule_frame(a);
                }
                T_LOCK => lock_tick(a),
                T_SAVE => {
                    KillTimer(hwnd, T_SAVE);
                    save_settings(a);
                }
                _ => {}
            }
            0
        }
        WM_DESTROY => {
            let p = a as *mut App;
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            overlays().retain(|&q| q != p);
            save_session();
            drop(Box::from_raw(p));
            // closing the last image quits, unless the dashboard was opened on purpose and is still showing
            let dash_visible = IsWindowVisible(dash().hwnd) != 0;
            if overlays().is_empty() && !dash_visible {
                PostQuitMessage(0);
            } else if dash_visible {
                refresh_dash(); // the "open" marks changed
            }
            0
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

// =====================================================================
// Dashboard window (history of opened GIFs) - built from native controls
// =====================================================================

const IDC_ADD: usize = 1001;
const IDC_DEFAULT: usize = 1003;

fn reg_set(subkey: &str, name: Option<&str>, value: &str) {
    let (sub, name, v) = (wide(subkey), name.map(wide), wide(value));
    unsafe {
        RegSetKeyValueW(
            HKEY_CURRENT_USER,
            sub.as_ptr(),
            name.as_ref().map_or(null(), |n| n.as_ptr()),
            REG_SZ,
            v.as_ptr() as *const c_void,
            (v.len() * 2) as u32,
        );
    }
}

/// Windows won't let an app make itself the default, so register as a candidate for .gif
/// (per-user, same as the installer does) and open Settings on this app's Default apps page.
fn open_default_apps() {
    let exe = std::env::current_exe().unwrap_or_default().to_string_lossy().into_owned();
    reg_set(r"Software\Classes\GIFOverlay.gif", None, "GIF Image");
    reg_set(r"Software\Classes\GIFOverlay.gif\DefaultIcon", None, &format!("{exe},0"));
    reg_set(r"Software\Classes\GIFOverlay.gif\shell\open\command", None, &format!("\"{exe}\" \"%1\""));
    reg_set(r"Software\Classes\.gif\OpenWithProgids", Some("GIFOverlay.gif"), "");
    reg_set(r"Software\GIFOverlay\Capabilities", Some("ApplicationName"), "GIF Overlay");
    reg_set(r"Software\GIFOverlay\Capabilities", Some("ApplicationDescription"), "Minimalist GIF & Image Viewer");
    reg_set(r"Software\GIFOverlay\Capabilities\FileAssociations", Some(".gif"), "GIFOverlay.gif");
    reg_set(r"Software\RegisteredApplications", Some("GIF Overlay"), r"Software\GIFOverlay\Capabilities");
    unsafe {
        ShellExecuteW(
            null_mut(),
            wide("open").as_ptr(),
            wide("ms-settings:defaultapps?registeredAppUser=GIF%20Overlay").as_ptr(),
            null(),
            null(),
            SW_SHOWNORMAL,
        );
    }
}
const IDC_LIST: usize = 1002;
const IDC_CLEAR: usize = 1004;
const IDC_HELP: usize = 1006;
const IDC_BUG: usize = 1007;
const ISSUES_URL: &str = "https://github.com/n3d06/GIF-Overlay/issues";
const IDC_PASTE: usize = 1009;
const IDC_QUIT_APP: usize = 1010;
const IDC_UPDATE: usize = 1012;
const IDC_RESTORE: usize = 1014;
const IDC_WIN_MIN: usize = 1020;
const IDC_WIN_RESTORE: usize = 1021;
const IDC_WIN_CLOSE: usize = 1022;
const IDC_WIN_TOGGLE: usize = 1023;
const IDC_WIN_BASE: usize = 2000; // + index of an open image, up to WIN_MAX
const WIN_MAX: usize = 100;
const IDC_VERSION: usize = 1013;
const IDM_SORT_RECENT: usize = 1101;
const IDM_SORT_LAST: usize = 1105; // the five sort items are IDM_SORT_RECENT + the SORT_* value
const IDM_LANG_BASE: usize = 1200; // "system default", then + 1 + the language index
const EMPTY_TEXT: &str = "No history yet\r\nUse File > Add image or drop an image here.";
const SORT_RECENT: u8 = 0;
const SORT_NAME: u8 = 1;
const SORT_NAME_DESC: u8 = 2;
const SORT_SIZE_DESC: u8 = 3; // largest file first
const SORT_SIZE_ASC: u8 = 4;
const IDM_OPEN: usize = 1;
const IDM_REMOVE: usize = 2;
const IDM_FOLDER: usize = 3;
const IDM_COPYPATH: usize = 4;
const IDM_ADD: usize = 5;
const IDM_PASTE: usize = 6;

struct Thumb {
    path: String,
    name: String,
    size: u64, // file size in bytes
    ready: bool, // false while the thumbnail is still being built on a worker thread (px is a blank)
    px: Vec<u32>, // opaque BGRA, tw*th
}

struct Dash {
    hwnd: HWND,
    list: HWND,
    sort: u8, // one of the SORT_* values (the View menu order)
    restore: bool, // reopen the last session's windows at start (File menu)
    lang: Option<usize>, // chosen language (index into lang::CODES); None follows the system
    empty: HWND,
    scale: f32, // DPI / 96
    tw: usize,  // thumbnail size in pixels
    th: usize,
    items: Vec<Thumb>,
    loading: Vec<String>, // files being decoded on worker threads
}

static mut DASH: Option<Dash> = None;

fn dash() -> &'static mut Dash {
    unsafe { (*addr_of_mut!(DASH)).as_mut().unwrap() }
}

/// Mean of the source pixels in [x0,x1) x [y0,y1) (premultiplied, so averaging is correct).
fn box_avg(f: &[u32], gw: usize, gh: usize, x0: f32, y0: f32, x1: f32, y1: f32) -> u32 {
    let (xs, ys) = ((x0 as usize).min(gw - 1), (y0 as usize).min(gh - 1));
    let (xe, ye) = ((x1.ceil() as usize).clamp(xs + 1, gw), (y1.ceil() as usize).clamp(ys + 1, gh));
    let (mut acc, mut n) = ([0u32; 4], 0u32);
    for y in ys..ye {
        for x in xs..xe {
            for (i, c) in acc.iter_mut().enumerate() {
                *c += (f[y * gw + x] >> (i * 8)) & 0xFF;
            }
            n += 1;
        }
    }
    acc.iter().enumerate().fold(0, |o, (i, c)| o | ((c + n / 2) / n) << (i * 8))
}

fn make_thumb(path: &str, tw: usize, th: usize) -> Option<Thumb> {
    let (gw, gh, frames, _) = decode(path, 1).ok()?;
    let s = (tw as f32 / gw as f32).min(th as f32 / gh as f32).min(2.0);
    let (dw, dh) = (((gw as f32 * s) as usize).clamp(1, tw), ((gh as f32 * s) as usize).clamp(1, th));
    let (ox, oy) = ((tw - dw) / 2, (th - dh) / 2);
    // blend onto the list's own background so transparent GIFs look right
    let c = unsafe { GetSysColor(COLOR_WINDOW) };
    let bg = (c & 0xFF) << 16 | ((c >> 8) & 0xFF) << 8 | (c >> 16) & 0xFF;
    let mut px = vec![0xFF00_0000 | bg; tw * th];
    for y in 0..dh {
        for x in 0..dw {
            // averaging the covered source area avoids the jagged edges bilinear gives when shrinking
            let p = if s < 1.0 {
                box_avg(&frames[0], gw, gh, x as f32 / s, y as f32 / s, (x + 1) as f32 / s, (y + 1) as f32 / s)
            } else {
                sample(&frames[0], gw, gh, (x as f32 + 0.5) / s - 0.5, (y as f32 + 0.5) / s - 0.5)
            };
            let a = p >> 24;
            let mut o = 0xFF00_0000;
            for sh in [0, 8, 16] {
                o |= (((p >> sh) & 0xFF) + ((bg >> sh) & 0xFF) * (255 - a) / 255).min(255) << sh;
            }
            px[(oy + y) * tw + ox + x] = o;
        }
    }
    Some(Thumb {
        path: path.to_string(),
        name: Path::new(path).file_name()?.to_string_lossy().into_owned(),
        size: fs::metadata(path).map_or(0, |m| m.len()),
        ready: true,
        px,
    })
}

/// A blank thumbnail shown at once, so the dashboard opens without decoding any image.
fn placeholder_thumb(path: &str, tw: usize, th: usize) -> Option<Thumb> {
    let c = unsafe { GetSysColor(COLOR_WINDOW) };
    let bg = (c & 0xFF) << 16 | ((c >> 8) & 0xFF) << 8 | (c >> 16) & 0xFF;
    Some(Thumb {
        path: path.to_string(),
        name: Path::new(path).file_name()?.to_string_lossy().into_owned(),
        size: fs::metadata(path).map_or(0, |m| m.len()),
        ready: false,
        px: vec![0xFF00_0000 | bg; tw * th],
    })
}

const WM_THUMB: u32 = WM_APP + 5;

/// Builds the thumbnails on one worker thread and posts each to the dashboard as it is done, so a
/// history full of big photos never freezes the window.
fn spawn_thumbs(paths: Vec<String>) {
    let d = dash();
    let (hwnd, tw, th) = (d.hwnd as usize, d.tw, d.th);
    std::thread::spawn(move || {
        use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_MULTITHREADED};
        let com = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.is_ok(); // WIC needs COM on this thread
        for p in paths {
            let thumb = std::panic::catch_unwind(|| make_thumb(&p, tw, th)).ok().flatten();
            let msg = Box::into_raw(Box::new((p, thumb)));
            if unsafe { PostMessageW(hwnd as HWND, WM_THUMB, 0, msg as isize) } == 0 {
                drop(unsafe { Box::from_raw(msg) }); // dashboard already gone: the app is quitting
                break;
            }
        }
        if com {
            unsafe { CoUninitialize() };
        }
    });
}

fn refresh_dash() {
    let d = dash();
    let mut history = history_read().unwrap_or_default();
    // forget entries whose file is gone; only a definite "not found" counts, so an unplugged drive keeps its entries
    let before = history.len();
    history.retain(|p| !matches!(fs::metadata(p), Err(e) if e.kind() == std::io::ErrorKind::NotFound));
    if history.len() != before {
        history_save(&history);
    }
    let mut old = mem::take(&mut d.items);
    let mut missing = Vec::new();
    d.items = history
        .into_iter()
        .filter(|p| fs::metadata(p).is_ok())
        .filter_map(|p| match old.iter().position(|t| t.path == p) {
            Some(i) => Some(old.swap_remove(i)), // keep what is already built (or still being built)
            None => {
                let t = placeholder_thumb(&p, d.tw, d.th)?;
                missing.push(p);
                Some(t)
            }
        })
        .collect();
    if !missing.is_empty() {
        spawn_thumbs(missing);
    }
    match d.sort {
        SORT_NAME => d.items.sort_by_key(|t| t.name.to_lowercase()),
        SORT_NAME_DESC => d.items.sort_by_key(|t| std::cmp::Reverse(t.name.to_lowercase())),
        SORT_SIZE_DESC => d.items.sort_by_key(|t| std::cmp::Reverse(t.size)),
        SORT_SIZE_ASC => d.items.sort_by_key(|t| t.size),
        _ => {} // SORT_RECENT keeps the history order, which is newest first
    }
    let open: Vec<String> = overlays().iter().map(|&p| unsafe { (*p).path.clone() }).collect();
    let is_open = |t: &Thumb| open.iter().any(|p| p.eq_ignore_ascii_case(&t.path));
    unsafe {
        SendMessageW(d.list, LVM_DELETEALLITEMS, 0, 0);
        let himl = ImageList_Create(d.tw as i32, d.th as i32, ILC_COLOR32, d.items.len() as i32, 4);
        for t in &d.items {
            let mut px = t.px.clone(); // the badge is drawn on a copy so the cached thumbnail stays clean
            if is_open(t) {
                draw_open_badge(&mut px, d.tw, d.scale);
            }
            let bmp = CreateBitmap(d.tw as i32, d.th as i32, 1, 32, px.as_ptr() as *const c_void);
            ImageList_Add(himl, bmp, null_mut());
            DeleteObject(bmp);
        }
        let prev = SendMessageW(d.list, LVM_SETIMAGELIST, LVSIL_NORMAL as usize, himl as isize);
        if prev != 0 {
            ImageList_Destroy(prev as HIMAGELIST);
        }
        for (i, t) in d.items.iter().enumerate() {
            let label = if is_open(t) { trf("{} (open)", &[&t.name]) } else { t.name.clone() };
            let mut name = wide(&label);
            let mut it: LVITEMW = mem::zeroed();
            it.mask = LVIF_TEXT | LVIF_IMAGE;
            it.iItem = i as i32;
            it.iImage = i as i32;
            it.pszText = name.as_mut_ptr();
            SendMessageW(d.list, LVM_INSERTITEMW, 0, &it as *const _ as isize);
        }
        ShowWindow(d.empty, if d.items.is_empty() { SW_SHOW } else { SW_HIDE });
        EnableMenuItem(GetMenu(d.hwnd), IDC_CLEAR as u32, MF_BYCOMMAND | if !d.items.is_empty() { MF_ENABLED } else { MF_GRAYED });
    }
}

/// Mixes `src` over the opaque pixel `dst`; `a` is the coverage 0..1.
fn mix(dst: u32, src: u32, a: f32) -> u32 {
    (0..3).fold(0xFF00_0000, |o, i| {
        let (d, s) = (((dst >> (i * 8)) & 0xFF) as f32, ((src >> (i * 8)) & 0xFF) as f32);
        o | ((d + (s - d) * a + 0.5) as u32) << (i * 8)
    })
}

/// Small green dot with a thin white ring in the top-right corner of a thumbnail.
/// Edges are anti-aliased from the distance to the circle, so they stay smooth at any DPI.
fn draw_open_badge(px: &mut [u32], tw: usize, scale: f32) {
    let (r, ring) = (5.0 * scale, 1.5 * scale);
    let (cx, cy) = (tw as f32 - r - ring - 6.0 * scale, r + ring + 6.0 * scale);
    let th = px.len() / tw;
    for y in 0..th {
        for x in 0..tw {
            let dist = ((x as f32 + 0.5 - cx).powi(2) + (y as f32 + 0.5 - cy).powi(2)).sqrt();
            let outer = (r + ring + 0.5 - dist).clamp(0.0, 1.0);
            if outer > 0.0 {
                let p = &mut px[y * tw + x];
                *p = mix(*p, 0xFFFF_FFFF, outer);
                *p = mix(*p, 0xFF22_C55E, (r + 0.5 - dist).clamp(0.0, 1.0));
            }
        }
    }
}

fn show_dash() {
    refresh_dash();
    unsafe {
        let d = dash();
        ShowWindow(d.hwnd, SW_SHOW);
        ShowWindow(d.hwnd, SW_RESTORE);
        SetForegroundWindow(d.hwnd);
        SetFocus(d.list);
    }
}

fn selected() -> Option<usize> {
    let i = unsafe { SendMessageW(dash().list, LVM_GETNEXTITEM, usize::MAX, LVNI_SELECTED as isize) };
    (i >= 0).then_some(i as usize)
}

fn item_path(i: usize) -> Option<String> {
    dash().items.get(i).map(|t| t.path.clone())
}

fn remove_item(path: &str) {
    history_remove(path);
    refresh_dash();
}

fn layout() {
    let d = dash();
    if d.list.is_null() {
        return; // WM_SIZE can arrive before the children exist
    }
    let s = |v: f32| (v * d.scale) as i32;
    let (w, h) = {
        let mut r: RECT = unsafe { mem::zeroed() };
        unsafe { GetClientRect(d.hwnd, &mut r) };
        (r.right, r.bottom)
    };
    let list_h = h;
    unsafe {
        MoveWindow(d.list, 0, 0, w, list_h, 1);
        MoveWindow(d.empty, 0, list_h / 2 - s(24.0), w, s(48.0), 1);
    }
}

/// Right-click menu of the list: item actions when the click lands on a thumbnail, otherwise
/// Add / Paste. `lp` is the screen position, or -1 when opened from the keyboard (Menu / Shift+F10).
fn context_menu_dash(hwnd: HWND, lp: LPARAM) {
    unsafe {
        let list = dash().list;
        let (mut pt, item) = if lp == -1 {
            let i = selected();
            let mut r: RECT = mem::zeroed();
            if let Some(i) = i {
                SendMessageW(list, LVM_GETITEMRECT, i, &mut r as *mut RECT as isize); // r.left = 0 = LVIR_BOUNDS
            }
            (POINT { x: r.left + 8, y: r.top + 8 }, i)
        } else {
            let mut pt = POINT { x: lp as i16 as i32, y: (lp >> 16) as i16 as i32 };
            ScreenToClient(list, &mut pt);
            let mut hit: LVHITTESTINFO = mem::zeroed();
            hit.pt = pt;
            SendMessageW(list, LVM_HITTEST, 0, &mut hit as *mut LVHITTESTINFO as isize);
            (pt, (hit.iItem >= 0 && hit.flags & (LVHT_ONITEMICON | LVHT_ONITEMLABEL) != 0).then_some(hit.iItem as usize))
        };
        ClientToScreen(list, &mut pt);
        let menu = CreatePopupMenu();
        let add = |flags: u32, id: usize, text: &str| {
            AppendMenuW(menu, flags, id, wide(&tr(text)).as_ptr());
        };
        if let Some(i) = item {
            let exists = Path::new(&dash().items[i].path).exists();
            add(MF_STRING, IDM_OPEN, "Open");
            add(MF_STRING | if exists { 0 } else { MF_GRAYED }, IDM_FOLDER, "Show in folder");
            add(MF_STRING, IDM_COPYPATH, "Copy path");
            add(MF_SEPARATOR, 0, "");
            add(MF_STRING, IDM_REMOVE, "Remove from history");
            SetMenuDefaultItem(menu, IDM_OPEN as u32, 0); // double-click action in bold
        } else {
            add(MF_STRING, IDM_ADD, "Add image");
            add(MF_STRING, IDM_PASTE, "Paste image or link\tCtrl+V");
        }
        // the list can be refreshed while the menu is open, so remember the path now, not the index
        let path = item.and_then(item_path);
        let id = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_RIGHTBUTTON, pt.x, pt.y, 0, hwnd, null());
        DestroyMenu(menu);
        match (id as usize, path) {
            (IDM_OPEN, Some(p)) => open_gif(&p),
            (IDM_FOLDER, Some(p)) => show_in_folder(&p),
            (IDM_COPYPATH, Some(p)) => copy_text(hwnd, &p),
            (IDM_REMOVE, Some(p)) => remove_item(&p),
            (IDM_ADD, _) => {
                for p in open_dialog(hwnd, true) {
                    open_gif(&p);
                }
            }
            (IDM_PASTE, _) => paste(),
            _ => {}
        }
    }
}

unsafe extern "system" fn dash_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_SIZE => {
            layout();
            0
        }
        WM_SETFOCUS => {
            SetFocus(dash().list);
            0
        }
                WM_ERASEBKGND => {
            let mut r: RECT = mem::zeroed();
            GetClientRect(hwnd, &mut r);
            FillRect(wp as HDC, &r, GetSysColorBrush(COLOR_WINDOW));
            1
        }
        WM_GETMINMAXINFO => {
            // keep the menu bar from wrapping
            (*(lp as *mut MINMAXINFO)).ptMinTrackSize.x = (560.0 * dash().scale) as i32;
            0
        }
        WM_COMMAND if wp & 0xFFFF == IDC_BUG => {
            ShellExecuteW(null_mut(), wide("open").as_ptr(), wide(ISSUES_URL).as_ptr(), null(), null(), SW_SHOWNORMAL);
            0
        }
        WM_COMMAND if wp & 0xFFFF == IDC_HELP => {
            // one translated line per entry; "" is a blank line between the sections
            const LINES: [&str; 20] = [
                "OPEN AN IMAGE",
                "\u{2022} Use File > Add image, drop a file here, or double-click a thumbnail.",
                "\u{2022} Ctrl+V pastes a screenshot, a copied file, or an image link (http / https).",
                "\u{2022} Right-click a thumbnail: Open, Show in folder, Copy path (Ctrl+C), Remove (Delete).",
                "\u{2022} Right-click an empty spot for Add image and Paste.",
                "",
                "ON AN IMAGE",
                "\u{2022} Drag the image to move it. Hover it to show the buttons at the top right: minimize, maximize and close.",
                "\u{2022} Drag the right edge, bottom edge or the corner to resize. Ctrl + mouse wheel zooms.",
                "\u{2022} Maximize stretches the image over the whole screen; click it again to restore.",
                "\u{2022} Drop another file on the image to replace it.",
                "\u{2022} Right-click for the menu: scale, opacity, rotate / flip, pause, lock, always on top, copy image, show in folder, replace image, dashboard, close and quit.",
                "\u{2022} Keys: Esc closes, Space pauses, Left / Right step one frame, - / + change the speed, Up / Down change the opacity, Ctrl+C copies the image.",
                "",
                "LOCK WINDOW",
                "\u{2022} A locked image is click-through, so it never gets in the way.",
                "\u{2022} To unlock it, hold Ctrl and click the image.",
                "",
                "FOUND A PROBLEM?",
                "\u{2022} Use Help > Report a bug to open the issue page on GitHub.",
            ];
            let text = LINES.iter().map(|l| if l.is_empty() { String::new() } else { tr(l) }).collect::<Vec<_>>().join("\n");
            MessageBoxW(hwnd, wide(&text).as_ptr(), wide(&tr("How to use GIF Overlay")).as_ptr(), MB_OK | MB_ICONINFORMATION);
            0
        }
        WM_COMMAND if wp & 0xFFFF == IDC_CLEAR => {
            let q = wide("Clear the history list?\nThe image files themselves are not deleted.");
            if MessageBoxW(hwnd, q.as_ptr(), wide(&tr("Clear history")).as_ptr(), MB_YESNO | MB_ICONQUESTION | MB_DEFBUTTON2) == IDYES {
                history_save(&[]);
                refresh_dash();
            }
            0
        }
        WM_COMMAND if (IDM_SORT_RECENT..=IDM_SORT_LAST).contains(&(wp & 0xFFFF)) => {
            let id = wp & 0xFFFF;
            dash().sort = (id - IDM_SORT_RECENT) as u8;
            CheckMenuRadioItem(GetSubMenu(GetMenu(hwnd), 1), IDM_SORT_RECENT as u32, IDM_SORT_LAST as u32, id as u32, MF_BYCOMMAND);
            write_settings(saved_opacity()); // remember the choice
            refresh_dash();
            0
        }
        WM_COMMAND if (IDM_LANG_BASE..=IDM_LANG_BASE + lang::NAMES.len()).contains(&(wp & 0xFFFF)) => {
            set_language(hwnd, (wp & 0xFFFF).checked_sub(IDM_LANG_BASE + 1)); // first entry = system default
            0
        }
        WM_COMMAND if wp & 0xFFFF == IDC_RESTORE => {
            let d = dash();
            d.restore = !d.restore;
            CheckMenuItem(GetMenu(hwnd), IDC_RESTORE as u32, MF_BYCOMMAND | if d.restore { MF_CHECKED } else { MF_UNCHECKED });
            write_settings(saved_opacity()); // remember the choice
            if d.restore {
                save_session(); // the windows open right now are the ones to bring back
            } else {
                let _ = fs::remove_file(exe_file("session.txt")); // no stale session if it is turned on later
            }
            0
        }
        WM_COMMAND if wp & 0xFFFF == IDC_PASTE => {
            paste();
            0
        }
        WM_COMMAND if wp & 0xFFFF == IDC_UPDATE => {
            check_update();
            0
        }
        WM_UPDATE => {
            show_update_result(hwnd, *Box::from_raw(lp as *mut Result<String, String>));
            0
        }
        // the Window menu (3rd of the bar) lists the open images each time it opens
        WM_INITMENUPOPUP if lp & 0xFFFF == 2 && (lp >> 16) & 0xFFFF == 0 => {
            let m = wp as HMENU;
            while DeleteMenu(m, 0, MF_BYPOSITION) != 0 {}
            let names: Vec<String> = overlays().iter().take(WIN_MAX).map(|&p| (*p).path.clone()).collect();
            if names.is_empty() {
                AppendMenuW(m, MF_STRING | MF_GRAYED, 0, wide(&tr("No open images")).as_ptr());
            }
            for (i, p) in names.iter().enumerate() {
                let name = Path::new(p).file_name().map_or(p.clone(), |n| n.to_string_lossy().into_owned());
                AppendMenuW(m, MF_STRING, IDC_WIN_BASE + i, wide(&name.replace('&', "&&")).as_ptr()); // && = a literal &
            }
            AppendMenuW(m, MF_SEPARATOR, 0, null());
            let on = if names.is_empty() { MF_GRAYED } else { 0 };
            AppendMenuW(m, MF_STRING | on, IDC_WIN_TOGGLE, wide(&tr("Hide / show all windows	Ctrl+Alt+G")).as_ptr());
            AppendMenuW(m, MF_STRING | on, IDC_WIN_MIN, wide(&tr("Minimize all")).as_ptr());
            AppendMenuW(m, MF_STRING | on, IDC_WIN_RESTORE, wide(&tr("Restore all")).as_ptr());
            AppendMenuW(m, MF_STRING | on, IDC_WIN_CLOSE, wide(&tr("Close all windows")).as_ptr());
            0
        }
        WM_COMMAND if (IDC_WIN_BASE..IDC_WIN_BASE + WIN_MAX).contains(&(wp & 0xFFFF)) => {
            if let Some(&p) = overlays().get((wp & 0xFFFF) - IDC_WIN_BASE) {
                show_overlay(&mut *p);
            }
            0
        }
        WM_COMMAND if wp & 0xFFFF == IDC_WIN_TOGGLE => {
            toggle_all();
            0
        }
        WM_HOTKEY => {
            toggle_all();
            0
        }
        WM_COMMAND if wp & 0xFFFF == IDC_WIN_MIN => {
            for p in overlays().clone() {
                minimize(&mut *p);
            }
            0
        }
        WM_COMMAND if wp & 0xFFFF == IDC_WIN_RESTORE => {
            for p in overlays().clone() {
                show_overlay(&mut *p);
            }
            0
        }
        WM_COMMAND if wp & 0xFFFF == IDC_WIN_CLOSE => {
            // destroying a window edits `overlays()`, so collect the handles first
            let all: Vec<usize> = overlays().iter().map(|&p| (*p).hwnd as usize).collect();
            for h in all {
                DestroyWindow(h as HWND);
            }
            0
        }
        WM_COMMAND if wp & 0xFFFF == IDC_QUIT_APP => {
            save_session();
            PostQuitMessage(0);
            0
        }
        WM_COMMAND if wp & 0xFFFF == IDC_DEFAULT => {
            open_default_apps();
            0
        }
        WM_COMMAND if wp & 0xFFFF == IDC_ADD => {
            for p in open_dialog(hwnd, true) {
                open_gif(&p);
            }
            0
        }
        WM_NOTIFY => {
            let nm = &*(lp as *const NMHDR);
            if nm.hwndFrom == dash().list {
                if nm.code == LVN_ITEMACTIVATE {
                    if let Some(p) = item_path((*(lp as *const NMITEMACTIVATE)).iItem as usize) {
                        open_gif(&p); // an unreadable file is dropped from the list once its decode fails
                    }
                } else if nm.code == LVN_KEYDOWN {
                    let key = (*(lp as *const NMLVKEYDOWN)).wVKey;
                    if key == VK_DELETE {
                        if let Some(p) = selected().and_then(item_path) {
                            remove_item(&p);
                        }
                    } else if key == 0x43 && GetAsyncKeyState(VK_CONTROL as i32) < 0 {
                        if let Some(p) = selected().and_then(item_path) {
                            copy_text(hwnd, &p);
                        }
                    }
                }
            }
            0
        }
        WM_CONTEXTMENU => {
            if wp as HWND == dash().list {
                context_menu_dash(hwnd, lp);
            }
            0
        }
        // keep the empty-state label on the list's background
        WM_CTLCOLORSTATIC => {
            SetBkColor(wp as HDC, GetSysColor(COLOR_WINDOW));
            SetTextColor(wp as HDC, GetSysColor(COLOR_GRAYTEXT));
            GetSysColorBrush(COLOR_WINDOW) as LRESULT
        }
        WM_DROPFILES => {
            for p in dropped_files(wp) {
                open_gif(&p);
            }
            0
        }
        WM_LOADED => {
            finish_load(*Box::from_raw(lp as *mut Loaded));
            0
        }
        WM_THUMB => {
            let (path, thumb) = *Box::from_raw(lp as *mut (String, Option<Thumb>));
            if let Some(item) = dash().items.iter_mut().find(|t| t.path == path) {
                match thumb {
                    Some(t) => *item = t,
                    None => item.ready = true, // undecodable: keep the blank, do not retry
                }
                refresh_dash();
            }
            0
        }
        WM_FETCHED => {
            if dash().loading.is_empty() {
                SetWindowTextW(hwnd, wide("GIF Overlay").as_ptr());
            }
            match *Box::from_raw(lp as *mut Result<String, String>) {
                Ok(p) => open_gif(&p),
                Err(e) => show_error(&e),
            }
            0
        }
        // a second launch hands over its file (or nothing) instead of starting another process
        WM_COPYDATA => {
            let cds = &*(lp as *const COPYDATASTRUCT);
            if cds.cbData >= 2 {
                let s = slice::from_raw_parts(cds.lpData as *const u16, cds.cbData as usize / 2);
                open_gif(String::from_utf16_lossy(s).trim_end_matches('\0'));
            } else {
                show_dash();
            }
            1
        }
        WM_CLOSE => {
            // With overlays on screen the dashboard just hides; otherwise closing it quits.
            if !overlays().is_empty() {
                ShowWindow(hwnd, SW_HIDE);
            } else {
                DestroyWindow(hwnd);
            }
            0
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            0
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

/// Index into `lang::CODES` for the system's display language.
fn system_language() -> usize {
    lang::detect(unsafe { windows_sys::Win32::Globalization::GetUserDefaultUILanguage() })
}

/// Switches the language on the fly: the menu bar is rebuilt, the other texts follow when they next appear.
unsafe fn set_language(hwnd: HWND, pref: Option<usize>) {
    let d = dash();
    d.lang = pref;
    lang::set(pref.unwrap_or_else(system_language));
    let old = GetMenu(hwnd);
    SetMenu(hwnd, null_mut());
    DestroyMenu(old);
    build_menu(hwnd, d.sort, d.restore, pref);
    SetWindowTextW(d.empty, wide(&tr(EMPTY_TEXT)).as_ptr());
    write_settings(saved_opacity()); // remember the choice
    refresh_dash(); // "(open)" labels
}

/// Menu bar of the dashboard: File / View / Help. Commands reuse the old button ids.
unsafe fn build_menu(hwnd: HWND, sort: u8, restore: bool, pref: Option<usize>) {
    let bar = CreateMenu();
    let (file, view, help) = (CreatePopupMenu(), CreatePopupMenu(), CreatePopupMenu());
    let add = |m: HMENU, flags: u32, id: usize, text: &str| {
        AppendMenuW(m, flags, id, wide(&tr(text)).as_ptr());
    };
    add(file, MF_STRING, IDC_ADD, "Add image\tCtrl+O");
    add(file, MF_STRING, IDC_PASTE, "Paste image or link\tCtrl+V");
    add(file, MF_SEPARATOR, 0, "");
    add(file, MF_STRING, IDC_CLEAR, "Clear history");
    add(file, MF_STRING | if restore { MF_CHECKED } else { 0 }, IDC_RESTORE, "Reopen windows on start");
    add(file, MF_SEPARATOR, 0, "");
    add(file, MF_STRING, IDC_QUIT_APP, "Quit GIF Overlay");
    add(view, MF_STRING, IDM_SORT_RECENT, "Sort by recently opened");
    add(view, MF_STRING, IDM_SORT_RECENT + SORT_NAME as usize, "Sort by name (A\u{2013}Z)");
    add(view, MF_STRING, IDM_SORT_RECENT + SORT_NAME_DESC as usize, "Sort by name (Z\u{2013}A)");
    add(view, MF_STRING, IDM_SORT_RECENT + SORT_SIZE_DESC as usize, "Sort by size (largest first)");
    add(view, MF_STRING, IDM_SORT_RECENT + SORT_SIZE_ASC as usize, "Sort by size (smallest first)");
    add(view, MF_SEPARATOR, 0, "");
    let langs = CreatePopupMenu();
    add(langs, MF_STRING, IDM_LANG_BASE, "System default");
    for (i, name) in lang::NAMES.iter().enumerate() {
        AppendMenuW(langs, MF_STRING, IDM_LANG_BASE + 1 + i, wide(name).as_ptr()); // each language in its own name
    }
    CheckMenuRadioItem(langs, IDM_LANG_BASE as u32, (IDM_LANG_BASE + lang::NAMES.len()) as u32, (IDM_LANG_BASE + pref.map_or(0, |i| i + 1)) as u32, MF_BYCOMMAND);
    add(view, MF_POPUP, langs as usize, "Language");
    add(help, MF_STRING, IDC_HELP, "How to use");
    add(help, MF_STRING, IDC_DEFAULT, "Set as default viewer");
    add(help, MF_SEPARATOR, 0, "");
    add(help, MF_STRING, IDC_BUG, "Report a bug");
    add(help, MF_SEPARATOR, 0, "");
    add(help, MF_STRING, IDC_UPDATE, "Check for updates");
    add(help, MF_STRING | MF_GRAYED, IDC_VERSION, &trf("Version {}", &[VERSION]));
    add(bar, MF_POPUP, file as usize, "File");
    add(bar, MF_POPUP, view as usize, "View");
    add(bar, MF_POPUP, CreatePopupMenu() as usize, "Window"); // filled in by WM_INITMENUPOPUP
    add(bar, MF_POPUP, help as usize, "Help");
    CheckMenuRadioItem(view, IDM_SORT_RECENT as u32, IDM_SORT_LAST as u32, (IDM_SORT_RECENT + sort as usize) as u32, MF_BYCOMMAND);
    SetMenu(hwnd, bar);
}

/// Creates the dashboard window and its native child controls.
unsafe fn create_dash(hinst: HINSTANCE, class: &[u16]) {
    let icc = INITCOMMONCONTROLSEX { dwSize: mem::size_of::<INITCOMMONCONTROLSEX>() as u32, dwICC: ICC_LISTVIEW_CLASSES };
    InitCommonControlsEx(&icc);
    let settings = fs::read_to_string(exe_file("settings.json")).unwrap_or_default();
    // the texts below are created in the saved language (no saved choice: the system's display language)
    let pref = json_val(&settings, "Language").and_then(|v| lang::CODES.iter().position(|c| *c == v.trim_matches('"')));
    lang::set(pref.unwrap_or_else(system_language));
    let dpi = GetDpiForSystem();
    let scale = dpi as f32 / 96.0;
    let s = |v: f32| (v * scale) as i32;
    let title = wide("GIF Overlay");
    let hwnd = CreateWindowExW(
        0, class.as_ptr(), title.as_ptr(), WS_OVERLAPPEDWINDOW,
        CW_USEDEFAULT, CW_USEDEFAULT, s(960.0), s(560.0),
        null_mut(), null_mut(), hinst, null(),
    );
    let child = |class: &str, text: &str, style: u32, id: usize| {
        CreateWindowExW(0, wide(class).as_ptr(), wide(text).as_ptr(), WS_CHILD | style, 0, 0, 0, 0, hwnd, id as HMENU, hinst, null())
    };
    let list = child(
        "SysListView32", "",
        WS_VISIBLE | WS_TABSTOP | LVS_ICON | LVS_AUTOARRANGE | LVS_SINGLESEL | LVS_SHOWSELALWAYS,
        IDC_LIST,
    );
    let empty = child("STATIC", &tr(EMPTY_TEXT), 0x1 /* SS_CENTER */, 0);

    // Explorer-style selection and system UI font, like any native app
    SetWindowTheme(list, wide("Explorer").as_ptr(), null());
    SendMessageW(list, LVM_SETEXTENDEDLISTVIEWSTYLE, 0, LVS_EX_DOUBLEBUFFER as isize);
    let mut ncm: NONCLIENTMETRICSW = mem::zeroed();
    ncm.cbSize = mem::size_of::<NONCLIENTMETRICSW>() as u32;
    SystemParametersInfoForDpi(SPI_GETNONCLIENTMETRICS, ncm.cbSize, &mut ncm as *mut _ as *mut c_void, 0, dpi);
    let font = CreateFontIndirectW(&ncm.lfMessageFont);
    for c in [list, empty] {
        SendMessageW(c, WM_SETFONT, font as usize, 1);
    }
    let sort = json_val(&fs::read_to_string(exe_file("settings.json")).unwrap_or_default(), "DashSort")
        .and_then(|v| v.parse::<u8>().ok())
        .filter(|&v| v <= SORT_SIZE_ASC)
        .unwrap_or(SORT_RECENT);
    let restore = json_val(&settings, "RestoreSession").is_none_or(|v| v != "false"); // on unless turned off
    build_menu(hwnd, sort, restore, pref);

    let (tw, th) = (s(144.0) as usize, s(112.0) as usize);
    SendMessageW(list, LVM_SETICONSPACING, 0, ((th as isize + s(60.0) as isize) << 16) | (tw as isize + s(28.0) as isize));
    *dash() = Dash { hwnd, list, sort, restore, lang: pref, empty, scale, tw, th, items: Vec::new(), loading: Vec::new() };
    DragAcceptFiles(hwnd, 1);
    RegisterHotKey(hwnd, 1, MOD_CONTROL | MOD_ALT | MOD_NOREPEAT, 0x47); // Ctrl+Alt+G; if another app owns it the Window menu item still works
    layout();
}

// =====================================================================

unsafe fn register(name: &[u16], proc: WNDPROC, hinst: HINSTANCE) {
    let wc = WNDCLASSEXW {
        cbSize: mem::size_of::<WNDCLASSEXW>() as u32,
        style: 0,
        lpfnWndProc: proc,
        cbClsExtra: 0,
        cbWndExtra: 0,
        hInstance: hinst,
        hIcon: LoadIconW(hinst, std::ptr::without_provenance::<u16>(1)), // resource id 1 = app icon (build.rs)
        hCursor: LoadCursorW(null_mut(), IDC_ARROW),
        hbrBackground: (COLOR_WINDOW + 1) as HBRUSH, // system window color, follows the theme
        lpszMenuName: null(),
        lpszClassName: name.as_ptr(),
        hIconSm: null_mut(),
    };
    RegisterClassExW(&wc);
}

/// Single instance: the first process owns a named mutex. A later launch sends its file
/// (if any) to the running dashboard window, which opens it or comes to the front, and exits.
unsafe fn forward_to_running_instance() -> bool {
    // the handle is deliberately never closed: it lives as long as this process
    CreateMutexW(null(), 0, wide("Local\\GifOverlaySingleInstance").as_ptr());
    if GetLastError() != ERROR_ALREADY_EXISTS {
        return false;
    }
    let class = wide("GifOverlayDash");
    let mut target = null_mut();
    for _ in 0..40 {
        // the first instance may still be creating its window
        target = FindWindowW(class.as_ptr(), null());
        if !target.is_null() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    if target.is_null() {
        return false;
    }
    AllowSetForegroundWindow(ASFW_ANY);
    let path: Vec<u16> = std::env::args()
        .nth(1)
        .filter(|p| fs::metadata(p).is_ok())
        .map(|p| wide(&abs_path(&p)))
        .unwrap_or_default();
    let cds = COPYDATASTRUCT { dwData: 1, cbData: (path.len() * 2) as u32, lpData: path.as_ptr() as *mut c_void };
    let mut result = 0;
    SendMessageTimeoutW(target, WM_COPYDATA, 0, &cds as *const _ as isize, SMTO_ABORTIFHUNG, 5000, &mut result);
    true
}

fn main() {
    if unsafe { forward_to_running_instance() } {
        return;
    }
    unsafe {
        // WIC (static image decoding) needs COM on this thread
        let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_APARTMENTTHREADED);
        DASH = Some(Dash {
            hwnd: null_mut(),
            list: null_mut(),
            sort: SORT_RECENT,
            restore: true,
            lang: None,
            empty: null_mut(),
            scale: 1.0,
            tw: 0,
            th: 0,
            items: Vec::new(),
            loading: Vec::new(),
        });

        let hinst = GetModuleHandleW(null());
        let (oc, dc) = (wide("GifOverlayWnd"), wide("GifOverlayDash"));
        register(&oc, Some(overlay_proc), hinst);
        register(&dc, Some(dash_proc), hinst);
        create_dash(hinst, &dc);

        // "Open With" / command line goes straight to an overlay; a bare launch shows the history.
        // The windows that were open when the app last quit come back first.
        let last = if dash().restore { parse_session(&fs::read_to_string(exe_file("session.txt")).unwrap_or_default()) } else { Vec::new() };
        let last: Vec<_> = last.into_iter().filter(|(p, _)| fs::metadata(p).is_ok()).take(WIN_MAX).collect();
        for (p, s) in &last {
            start_load_with(p, null_mut(), Some(*s));
        }
        match std::env::args().nth(1).filter(|p| fs::metadata(p).is_ok()) {
            Some(p) => open_gif(&p),
            None if last.is_empty() => show_dash(),
            None => {} // the restored windows are the app's windows; the dashboard is one click away
        }

        let mut msg: MSG = mem::zeroed();
        while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
            // Ctrl+V works in every window of the app (not on key repeat),
            if msg.message == WM_KEYDOWN && msg.wParam == 0x56 && msg.lParam & (1 << 30) == 0 && GetAsyncKeyState(VK_CONTROL as i32) < 0 {
                paste();
                continue;
            }
            // Ctrl+O: add images (like File > Add image)
            if msg.message == WM_KEYDOWN && msg.wParam == 0x4F && msg.lParam & (1 << 30) == 0 && GetAsyncKeyState(VK_CONTROL as i32) < 0 {
                for p in open_dialog(msg.hwnd, true) {
                    open_gif(&p);
                }
                continue;
            }
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The integer fast path must match the old float `sample` path (within rounding) for every
    /// rotation / flip combination, and be clearly faster.
    #[test]
    fn upscale_matches_float_reference() {
        let (gw, gh) = (37, 23);
        let f: Vec<u32> = (0..gw * gh).map(|i| (i as u32).wrapping_mul(2654435761) | 0x8000_0000).collect();
        for rot in 0..4u8 {
            for (fh, fv) in [(false, false), (true, false), (false, true), (true, true)] {
                let (w, h) = if rot % 2 == 1 { (50, 90) } else { (90, 50) };
                let mut fast = vec![0u32; w * h];
                upscale_into(&mut fast, (w, h), &f, (gw, gh), rot, (fh, fv));
                for y in 0..h {
                    for x in 0..w {
                        let (mut u, mut v) = ((x as f32 + 0.5) / w as f32, (y as f32 + 0.5) / h as f32);
                        if fh { u = 1.0 - u }
                        if fv { v = 1.0 - v }
                        let (su, sv) = match rot {
                            0 => (u, v),
                            1 => (v, 1.0 - u),
                            2 => (1.0 - u, 1.0 - v),
                            _ => (1.0 - v, u),
                        };
                        let want = sample(&f, gw, gh, su * gw as f32 - 0.5, sv * gh as f32 - 0.5);
                        for s in [0, 8, 16, 24] {
                            let d = ((fast[y * w + x] >> s) & 0xFF) as i32 - ((want >> s) & 0xFF) as i32;
                            assert!(d.abs() <= 2, "rot {rot} flip {fh}/{fv} at ({x},{y}) channel {s}: {d}");
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn session_round_trip() {
        let s = Saved { x: -1200, y: 40, w: 640, h: 360, opacity: 70, locked: true, rot: 3, flip_h: false, flip_v: true };
        let text = format!("{}\nbroken line\n{}\n", session_line("C:\\a\tb\\x.gif", &s), session_line("D:\\y.png", &Saved { rot: 0, ..s }));
        let got = parse_session(&text);
        assert_eq!(got.len(), 2, "the damaged line is skipped");
        assert_eq!(got[0], ("C:\\a\tb\\x.gif".to_string(), s)); // a path with a tab survives
        assert_eq!(got[1].1.rot, 0);
    }

    #[test]
    fn version_compare() {
        assert!(version_parts("v1.4.3") > version_parts("1.3.5"));
        assert!(version_parts("1.10.0") > version_parts("1.9.9")); // numeric, not text, order
        assert!(version_parts("1.3.5") == version_parts("v1.3.5"));
    }

    /// The cleanup may only delete files that are not in the history AND are old enough.
    #[test]
    fn prune_pasted_keeps_listed_and_recent_files() {
        use std::time::Duration;
        let dir = std::env::temp_dir().join(format!("gif_overlay_prune_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let (listed, unlisted) = (dir.join("a.png"), dir.join("b.png"));
        fs::write(&listed, b"x").unwrap();
        fs::write(&unlisted, b"x").unwrap();
        let keep = vec![listed.to_string_lossy().to_uppercase()]; // the history compare ignores case

        prune_pasted(&dir, &keep, Duration::from_secs(3600)); // everything is younger than an hour
        assert!(listed.exists() && unlisted.exists(), "recent files must be spared");

        prune_pasted(&dir, &keep, Duration::ZERO);
        assert!(listed.exists(), "a listed file must stay");
        assert!(!unlisted.exists(), "an old unlisted file must go");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn dib_gets_a_bmp_header() {
        let mut dib = vec![0u8; 40 + 4]; // BITMAPINFOHEADER + a pixel stub
        dib[0] = 40;
        dib[14] = 24;
        let bmp = dib_to_bmp(&dib).unwrap();
        assert_eq!(&bmp[..2], b"BM");
        assert_eq!(u32::from_le_bytes(bmp[10..14].try_into().unwrap()), 54); // pixels start right after the header
        assert_eq!(bmp.len(), 14 + dib.len());
    }

    /// Run with `cargo test --release -- --ignored --nocapture` to compare speed at full-screen size.
    #[test]
    #[ignore]
    fn upscale_speed() {
        let (gw, gh, w, h) = (480, 270, 1920, 1080);
        let f: Vec<u32> = (0..gw * gh).map(|i| (i as u32).wrapping_mul(2654435761) | 0x8000_0000).collect();
        let mut dst = vec![0u32; w * h];
        let t = std::time::Instant::now();
        for _ in 0..20 {
            upscale_into(&mut dst, (w, h), &f, (gw, gh), 0, (false, false));
        }
        eprintln!("integer: {:?}/frame", t.elapsed() / 20);
        let t = std::time::Instant::now();
        for _ in 0..5 {
            for y in 0..h {
                for x in 0..w {
                    dst[y * w + x] = sample(&f, gw, gh, (x as f32 + 0.5) / w as f32 * gw as f32 - 0.5, (y as f32 + 0.5) / h as f32 * gh as f32 - 0.5);
                }
            }
        }
        eprintln!("float:   {:?}/frame", t.elapsed() / 5);
    }
}
