#![windows_subsystem = "windows"]

use std::{
    ffi::c_void,
    fs, mem,
    path::{Path, PathBuf},
    ptr::{addr_of_mut, null, null_mut},
    slice,
};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::Gdi::*,
    System::{DataExchange::COPYDATASTRUCT, LibraryLoader::GetModuleHandleW, Registry::*, Threading::CreateMutexW},
    UI::{
        Controls::{Dialogs::*, *}, HiDpi::*,
        Input::KeyboardAndMouse::{GetAsyncKeyState, VK_CONTROL, 
            EnableWindow, ReleaseCapture, SetCapture, SetFocus, TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT, VK_DELETE,
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
const ID_SCALE: usize = 100; // + index into SCALES
const ID_OPACITY: usize = 200; // + index into OPACITIES
const SCALES: [u32; 7] = [25, 50, 75, 100, 150, 200, 300];
const OPACITIES: [u32; 10] = [10, 20, 30, 40, 50, 60, 70, 80, 90, 100];
const HISTORY_MAX: usize = 30;
const GIF_BUDGET: usize = 512 << 20; // bytes of decoded GIF frames kept per image

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
  \"DashSort\": {}
}}",
        opacity as f64 / 100.0, dash().sort
    );
    write_atomic("settings.json", json.as_bytes());
}

fn save_settings(a: &App) {
    write_settings(a.opacity);
}

// ---------- history (history.txt, one path per line, newest first) ----------

fn history_load() -> Vec<String> {
    fs::read_to_string(exe_file("history.txt"))
        .unwrap_or_default()
        .lines()
        .map(|l| l.trim().trim_start_matches('\u{feff}')) // tolerate editors that add a BOM
        .filter(|l| !l.is_empty())
        .map(String::from)
        .collect()
}

fn history_save(h: &[String]) {
    write_atomic("history.txt", h.join("\n").as_bytes());
}

fn history_push(path: &str) {
    let mut h = history_load();
    h.retain(|p| !p.eq_ignore_ascii_case(path));
    h.insert(0, path.to_string());
    h.truncate(HISTORY_MAX);
    history_save(&h);
}

fn history_remove(path: &str) {
    let mut h = history_load();
    h.retain(|p| p != path);
    history_save(&h);
}

// ---------- GIF decoding ----------

type Decoded = (usize, usize, Vec<Vec<u32>>, Vec<u32>);

/// Animated GIFs go through the `gif` crate; every other format (PNG, JPEG, BMP, TIFF, WebP...)
/// is decoded by the Windows Imaging Component, which adds almost nothing to the exe size.
fn decode(path: &str, max: usize) -> Result<Decoded, String> {
    let is_gif = Path::new(path).extension().is_some_and(|e| e.eq_ignore_ascii_case("gif"));
    if is_gif {
        if let Ok(d) = decode_gif(path, max) {
            return Ok(d);
        }
    }
    decode_wic(path)
}

/// First frame as premultiplied BGRA, which is exactly what WIC's 32bppPBGRA gives.
fn decode_wic(path: &str) -> Result<Decoded, String> {
    use windows::{
        core::HSTRING,
        Win32::{
            Foundation::GENERIC_READ,
            Graphics::Imaging::*,
            System::Com::{CoCreateInstance, CLSCTX_INPROC_SERVER},
        },
    };
    let e = |e: windows::core::Error| e.message();
    unsafe {
        let factory: IWICImagingFactory = CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER).map_err(e)?;
        let decoder = factory
            .CreateDecoderFromFilename(&HSTRING::from(path), None, GENERIC_READ, WICDecodeMetadataCacheOnDemand)
            .map_err(e)?;
        let frame = decoder.GetFrame(0).map_err(e)?;
        // EXIF orientation (phone photos): WIC applies flip first, then rotate
        let opts = exif_orientation(&frame).map_or(0, |o| match o {
            2 => 8,
            3 => 2,
            4 => 16,
            5 => 3 | 8,
            6 => 1,
            7 => 1 | 8,
            8 => 3,
            _ => 0,
        });
        let rotator = factory.CreateBitmapFlipRotator().map_err(e)?;
        rotator.Initialize(&frame, WICBitmapTransformOptions(opts)).map_err(e)?;
        let conv = factory.CreateFormatConverter().map_err(e)?;
        conv.Initialize(&rotator, &GUID_WICPixelFormat32bppPBGRA, WICBitmapDitherTypeNone, None, 0.0, WICBitmapPaletteTypeCustom)
            .map_err(e)?;
        let (mut w, mut h) = (0u32, 0u32);
        conv.GetSize(&mut w, &mut h).map_err(e)?;
        if w == 0 || h == 0 {
            return Err("empty image".into());
        }
        if w as usize * h as usize * 4 > GIF_BUDGET {
            return Err("image too large".into()); // a failed allocation would abort the whole app
        }
        let mut px = vec![0u32; w as usize * h as usize];
        conv.CopyPixels(null(), w * 4, slice::from_raw_parts_mut(px.as_mut_ptr() as *mut u8, px.len() * 4))
            .map_err(e)?;
        Ok((w as usize, h as usize, vec![px], vec![100]))
    }
}

fn exif_orientation(frame: &windows::Win32::Graphics::Imaging::IWICBitmapFrameDecode) -> Option<u16> {
    use windows::core::w;
    let mut pv = windows::Win32::System::Com::StructuredStorage::PROPVARIANT::default();
    unsafe { frame.GetMetadataQueryReader().ok()?.GetMetadataByName(w!("/app1/ifd/{ushort=274}"), &mut pv).ok()? };
    u16::try_from(&pv).ok()
}

/// Decodes up to `max` frames into full-canvas premultiplied BGRA.
fn decode_gif(path: &str, max: usize) -> Result<Decoded, String> {
    let file = fs::File::open(path).map_err(|e| e.to_string())?;
    let mut opts = gif::DecodeOptions::new();
    opts.set_color_output(gif::ColorOutput::RGBA);
    let mut dec = opts.read_info(file).map_err(|e| e.to_string())?;
    let (gw, gh) = (dec.width() as usize, dec.height() as usize);
    if gw == 0 || gh == 0 {
        return Err("empty image".into());
    }
    // memory budget: every frame is a full canvas, and running out of RAM aborts the app
    let frame_bytes = gw * gh * 4;
    if frame_bytes > GIF_BUDGET {
        return Err("image too large".into());
    }
    let max = max.min(GIF_BUDGET / frame_bytes); // longer GIFs are cut to the frames that fit
    let mut canvas = vec![0u32; gw * gh];
    let (mut frames, mut delays) = (Vec::new(), Vec::new());
    while frames.len() < max {
        // a corrupt frame mid-file keeps the frames decoded so far instead of losing the animation
        let f = match dec.read_next_frame() {
            Ok(Some(f)) => f,
            Ok(None) => break,
            Err(e) if frames.is_empty() => return Err(e.to_string()),
            Err(_) => break,
        };
        let saved = (f.dispose == gif::DisposalMethod::Previous).then(|| canvas.clone());
        let (l, t, w, h) = (f.left as usize, f.top as usize, f.width as usize, f.height as usize);
        for y in 0..h {
            for x in 0..w {
                let (cx, cy) = (l + x, t + y);
                let p = &f.buffer[(y * w + x) * 4..][..4];
                // GIF alpha is binary: transparent pixels leave the canvas untouched
                if cx < gw && cy < gh && p[3] != 0 {
                    canvas[cy * gw + cx] = 0xFF00_0000 | (p[0] as u32) << 16 | (p[1] as u32) << 8 | p[2] as u32;
                }
            }
        }
        frames.push(canvas.clone());
        delays.push(if f.delay <= 1 { 100 } else { f.delay as u32 * 10 }); // browsers treat 0/1 as 100ms
        match f.dispose {
            gif::DisposalMethod::Background => {
                for y in t..(t + h).min(gh) {
                    for x in l..(l + w).min(gw) {
                        canvas[y * gw + x] = 0;
                    }
                }
            }
            gif::DisposalMethod::Previous => canvas = saved.unwrap(),
            _ => {}
        }
    }
    if frames.is_empty() {
        return Err("no frames".into());
    }
    Ok((gw, gh, frames, delays))
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

fn open_dialog(owner: HWND) -> Option<String> {
    let filter: Vec<u16> = "Images\0*.gif;*.png;*.jpg;*.jpeg;*.bmp;*.tif;*.tiff;*.webp\0GIF Files\0*.gif\0All Files\0*.*\0\0"
        .encode_utf16()
        .collect();
    let title = wide("Select Image");
    let mut buf = [0u16; 1024];
    unsafe {
        let mut ofn: OPENFILENAMEW = mem::zeroed();
        ofn.lStructSize = mem::size_of::<OPENFILENAMEW>() as u32;
        ofn.hwndOwner = owner;
        ofn.lpstrFilter = filter.as_ptr();
        ofn.lpstrFile = buf.as_mut_ptr();
        ofn.nMaxFile = buf.len() as u32;
        ofn.lpstrTitle = title.as_ptr();
        ofn.Flags = OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST | OFN_NOCHANGEDIR;
        if GetOpenFileNameW(&mut ofn) == 0 {
            return None;
        }
    }
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    Some(String::from_utf16_lossy(&buf[..len]))
}

/// First file of a WM_DROPFILES message.
unsafe fn dropped_file(wp: WPARAM) -> Option<String> {
    let hdrop = wp as HDROP;
    let mut buf = [0u16; 1024];
    let n = DragQueryFileW(hdrop, 0, buf.as_mut_ptr(), buf.len() as u32) as usize;
    DragFinish(hdrop);
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
                for dy in 0..g {
                    for dx in 0..g - dy {
                        let d = dx + dy;
                        dst[(h - 1 - dy) * w + (w - 1 - dx)] =
                            if d < g - 3 && matches!(d % 6, 2 | 3) { 0xC0C0_C0C0 } else { color(Zone::Corner, 0x7000_0000) };
                    }
                }
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

fn schedule_save(a: &App) {
    unsafe { SetTimer(a.hwnd, T_SAVE, 500, None) }; // re-arming resets the debounce
}

fn schedule_frame(a: &App) {
    unsafe {
        if a.frames.len() > 1 && !a.paused {
            SetTimer(a.hwnd, T_FRAME, a.delays[a.cur], None);
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
        MessageBoxW(null_mut(), wide(&format!("Failed to load image: {e}")).as_ptr(), wide("Error").as_ptr(), MB_OK | MB_ICONERROR);
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

/// Opens an already decoded image in a new overlay window.
fn create_overlay(path: &str, d: Decoded) {
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
        apply_lock(&*a);
        center(&*a, offset);
        ShowWindow(hwnd, SW_SHOW);
        SetForegroundWindow(hwnd);
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
}

fn set_title(hwnd: HWND, path: &str) {
    let name = Path::new(path).file_name().map_or(path.to_string(), |n| n.to_string_lossy().into_owned());
    unsafe { SetWindowTextW(hwnd, wide(&name).as_ptr()) }; // tells windows apart in the taskbar
}

/// Decodes on a worker thread so a big GIF cannot freeze the UI; `finish_load` runs when it is done.
fn start_load(path: &str, target: HWND) {
    let path = abs_path(path);
    let d = dash();
    if d.loading.iter().any(|p| p.eq_ignore_ascii_case(&path)) {
        return; // the same file is already on its way
    }
    d.loading.push(path.clone());
    unsafe {
        SetWindowTextW(d.hwnd, wide("GIF Overlay - loading...").as_ptr());
        if !target.is_null() {
            SetWindowTextW(target, wide("Loading...").as_ptr());
        }
    }
    let (dash_hwnd, target) = (d.hwnd as usize, target as usize);
    std::thread::spawn(move || {
        use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_MULTITHREADED};
        let com = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.is_ok(); // WIC needs COM on this thread
        let res = decode(&path, usize::MAX);
        if com {
            unsafe { CoUninitialize() };
        }
        let msg = Box::into_raw(Box::new(Loaded { path, target, res }));
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
            history_push(&l.path);
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
                    create_overlay(&l.path, img);
                }
                ShowWindow(d.hwnd, SW_HIDE);
            }
        }
        Err(e) => {
            if let Some(p) = target {
                set_title((*p).hwnd, &(*p).path); // undo "Loading..."
            }
            show_error(&e);
            history_remove(&l.path);
            if overlays().is_empty() {
                show_dash(); // never leave the app without a window
            } else if IsWindowVisible(d.hwnd) != 0 {
                refresh_dash();
            }
        }
    }
}

/// Opens `path` in a new window (or brings the window that already shows it to the front).
/// New images are decoded in the background; `finish_load` records them and hides the dashboard.
fn open_gif(path: &str) {
    let path = abs_path(path);
    let existing = overlays().iter().copied().find(|&p| unsafe { (*p).path.eq_ignore_ascii_case(&path) });
    if let Some(p) = existing {
        show_overlay(unsafe { &mut *p });
        history_push(&path);
        unsafe { ShowWindow(dash().hwnd, SW_HIDE) };
    } else {
        start_load(&path, null_mut());
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
        }
        ID_OPEN => {
            if let Some(p) = open_dialog(a.hwnd) {
                start_load(&p, a.hwnd);
            }
        }
        ID_HISTORY => show_dash(),
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
        let add =|m: HMENU, flags: u32, id: usize, text: &str| {
            AppendMenuW(m, flags, id, wide(text).as_ptr());
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
        add(menu, MF_SEPARATOR, 0, "");
        add(menu, MF_POPUP, settings as usize, "Settings");
        add(menu, MF_STRING | check(paused), ID_PAUSE, "Pause / Play");
        add(menu, MF_SEPARATOR, 0, "");
        add(menu, MF_STRING, ID_QUIT, "Exit app");

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
        WM_DROPFILES => {
            // dropping on an overlay swaps its image; the dashboard is where new windows are opened
            if let Some(p) = dropped_file(wp) {
                start_load(&p, a.hwnd);
            }
            0
        }
        WM_MOUSEWHEEL => {
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
const IDC_SORT: usize = 1005;
const SORT_RECENT: u8 = 0;
const SORT_NAME: u8 = 1;
const IDM_OPEN: usize = 1;
const IDM_REMOVE: usize = 2;

struct Thumb {
    path: String,
    name: String,
    px: Vec<u32>, // opaque BGRA, tw*th
}

struct Dash {
    hwnd: HWND,
    list: HWND,
    add: HWND,
    default_btn: HWND,
    clear_btn: HWND,
    help_btn: HWND,
    sort_box: HWND,
    sort: u8, // SORT_RECENT (most recently opened first) or SORT_NAME
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
        px,
    })
}

fn refresh_dash() {
    let d = dash();
    let mut old = mem::take(&mut d.items);
    d.items = history_load()
        .into_iter()
        .filter(|p| fs::metadata(p).is_ok())
        .filter_map(|p| match old.iter().position(|t| t.path == p) {
            Some(i) => Some(old.swap_remove(i)), // keep already built thumbnails
            None => make_thumb(&p, d.tw, d.th),
        })
        .collect();
    if d.sort == SORT_NAME {
        d.items.sort_by_key(|t| t.name.to_lowercase());
    } // SORT_RECENT keeps the history order, which is newest first
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
            let label = if is_open(t) { format!("{} (open)", t.name) } else { t.name.clone() };
            let mut name = wide(&label);
            let mut it: LVITEMW = mem::zeroed();
            it.mask = LVIF_TEXT | LVIF_IMAGE;
            it.iItem = i as i32;
            it.iImage = i as i32;
            it.pszText = name.as_mut_ptr();
            SendMessageW(d.list, LVM_INSERTITEMW, 0, &it as *const _ as isize);
        }
        ShowWindow(d.empty, if d.items.is_empty() { SW_SHOW } else { SW_HIDE });
        EnableWindow(d.clear_btn, (!d.items.is_empty()) as i32);
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

fn open_item(i: usize) {
    let p = dash().items[i].path.clone();
    open_gif(&p); // an unreadable file is dropped from the list once its decode fails
}

fn remove_item(i: usize) {
    history_remove(&dash().items[i].path.clone());
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
    let bar = s(52.0);
    unsafe {
        MoveWindow(d.add, s(12.0), s(10.0), s(130.0), s(32.0), 1);
        MoveWindow(d.default_btn, s(150.0), s(10.0), s(210.0), s(32.0), 1);
        MoveWindow(d.clear_btn, s(368.0), s(10.0), s(140.0), s(32.0), 1);
        MoveWindow(d.help_btn, s(516.0), s(10.0), s(120.0), s(32.0), 1);
        // a combo box's height includes its drop-down list
        MoveWindow(d.sort_box, w - s(12.0 + 190.0), s(10.0), s(190.0), s(160.0), 1);
        MoveWindow(d.list, 0, bar, w, (h - bar).max(0), 1);
        MoveWindow(d.empty, 0, bar + (h - bar) / 2 - s(24.0), w, s(48.0), 1);
    }
}

fn context_menu_dash(hwnd: HWND) {
    let Some(i) = selected() else { return };
    unsafe {
        let menu = CreatePopupMenu();
        AppendMenuW(menu, MF_STRING, IDM_OPEN, wide("Open").as_ptr());
        AppendMenuW(menu, MF_STRING, IDM_REMOVE, wide("Remove from history").as_ptr());
        let mut pt = POINT { x: 0, y: 0 };
        GetCursorPos(&mut pt);
        let id = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_RIGHTBUTTON, pt.x, pt.y, 0, hwnd, null());
        DestroyMenu(menu);
        match id as usize {
            IDM_OPEN => open_item(i),
            IDM_REMOVE => remove_item(i),
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
        WM_GETMINMAXINFO => {
            // keep the toolbar from overlapping itself
            (*(lp as *mut MINMAXINFO)).ptMinTrackSize.x = (860.0 * dash().scale) as i32;
            0
        }
        WM_COMMAND if wp & 0xFFFF == IDC_HELP => {
            let text = "OPEN AN IMAGE\n\
                \u{2022} Click \"Add image...\", drop a file here, or double-click a thumbnail.\n\
                \u{2022} Right-click a thumbnail (or press Delete) to remove it from the history.\n\n\
                ON AN IMAGE\n\
                \u{2022} Drag the image to move it. Hover it to show the buttons at the top right: minimize, maximize and close.\n\
                \u{2022} Drag the right edge, bottom edge or the corner to resize. Ctrl + mouse wheel zooms.\n\
                \u{2022} Maximize stretches the image over the whole screen; click it again to restore.\n\
                \u{2022} Drop another file on the image to replace it.\n\
                \u{2022} Right-click for the menu: scale, opacity, rotate / flip, pause, lock, replace image, dashboard and exit.\n\n\
                LOCK WINDOW\n\
                \u{2022} A locked image is click-through, so it never gets in the way.\n\
                \u{2022} To unlock it, hold Ctrl and click the image.";
            MessageBoxW(hwnd, wide(text).as_ptr(), wide("How to use GIF Overlay").as_ptr(), MB_OK | MB_ICONINFORMATION);
            0
        }
        WM_COMMAND if wp & 0xFFFF == IDC_CLEAR => {
            let q = wide("Clear the history list?\nThe image files themselves are not deleted.");
            if MessageBoxW(hwnd, q.as_ptr(), wide("Clear history").as_ptr(), MB_YESNO | MB_ICONQUESTION | MB_DEFBUTTON2) == IDYES {
                history_save(&[]);
                refresh_dash();
            }
            0
        }
        // CBN_SELCHANGE = 1
        WM_COMMAND if wp & 0xFFFF == IDC_SORT && (wp >> 16) & 0xFFFF == 1 => {
            let d = dash();
            d.sort = SendMessageW(d.sort_box, CB_GETCURSEL, 0, 0).max(0) as u8;
            write_settings(saved_opacity()); // remember the choice
            refresh_dash();
            SetFocus(d.list);
            0
        }
        WM_COMMAND if wp & 0xFFFF == IDC_DEFAULT => {
            open_default_apps();
            0
        }
        WM_COMMAND if wp & 0xFFFF == IDC_ADD => {
            if let Some(p) = open_dialog(hwnd) {
                open_gif(&p);
            }
            0
        }
        WM_NOTIFY => {
            let nm = &*(lp as *const NMHDR);
            if nm.hwndFrom == dash().list {
                if nm.code == LVN_ITEMACTIVATE {
                    open_item((*(lp as *const NMITEMACTIVATE)).iItem as usize);
                } else if nm.code == LVN_KEYDOWN {
                    let key = (*(lp as *const NMLVKEYDOWN)).wVKey;
                    if key == VK_DELETE {
                        if let Some(i) = selected() {
                            remove_item(i);
                        }
                    }
                }
            }
            0
        }
        WM_CONTEXTMENU => {
            if wp as HWND == dash().list {
                context_menu_dash(hwnd);
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
            if let Some(p) = dropped_file(wp) {
                open_gif(&p);
            }
            0
        }
        WM_LOADED => {
            finish_load(*Box::from_raw(lp as *mut Loaded));
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

unsafe fn dib(dc: HDC, size: i32, bits: &mut *mut c_void) -> HBITMAP {
    let mut bi: BITMAPINFO = mem::zeroed();
    bi.bmiHeader.biSize = mem::size_of::<BITMAPINFOHEADER>() as u32;
    (bi.bmiHeader.biWidth, bi.bmiHeader.biHeight) = (size, -size);
    (bi.bmiHeader.biPlanes, bi.bmiHeader.biBitCount, bi.bmiHeader.biCompression) = (1, 32, BI_RGB);
    CreateDIBSection(dc, &bi, DIB_RGB_COLORS, bits, null_mut(), 0)
}

/// Draws one glyph of the Segoe icon font into a premultiplied-alpha bitmap.
/// The glyph is drawn 4x larger and averaged down, which gives smooth edges at 16px
/// (GDI's own anti-aliasing of hinted small glyphs looks jagged).
/// Returns None when no icon font is installed (GDI would substitute a text font).
unsafe fn glyph_bitmap(glyph: u16, size: i32, color: u32) -> Option<HBITMAP> {
    const SS: i32 = 4;
    let dc = CreateCompatibleDC(null_mut());
    let big = size * SS;
    let mut big_bits: *mut c_void = null_mut();
    let big_bmp = dib(dc, big, &mut big_bits);
    let mut bits: *mut c_void = null_mut();
    let bmp = dib(dc, size, &mut bits);
    let mut result = None;
    if !big_bits.is_null() && !bits.is_null() {
        let old_bmp = SelectObject(dc, big_bmp);
        // Windows 11 ships "Segoe Fluent Icons"; Windows 10 only has "Segoe MDL2 Assets" (same code points)
        for name in ["Segoe Fluent Icons", "Segoe MDL2 Assets"] {
            let face = wide(name);
            let font = CreateFontW(-(big * 9 / 10), 0, 0, 0, 400, 0, 0, 0, DEFAULT_CHARSET as u32, 0, 0, ANTIALIASED_QUALITY as u32, 0, face.as_ptr());
            let old_font = SelectObject(dc, font);
            let mut got = [0u16; 32];
            GetTextFaceW(dc, got.len() as i32, got.as_mut_ptr());
            if got[..name.len()] == face[..name.len()] {
                // white on black, so the coverage of every pixel can be read back from its brightness
                SetTextColor(dc, 0x00FF_FFFF);
                SetBkColor(dc, 0);
                let mut r = RECT { left: 0, top: 0, right: big, bottom: big };
                DrawTextW(dc, &glyph, 1, &mut r, DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOCLIP);
                let src = slice::from_raw_parts(big_bits as *const u32, (big * big) as usize);
                let dst = slice::from_raw_parts_mut(bits as *mut u32, (size * size) as usize);
                let (cr, cg, cb) = (color & 0xFF, (color >> 8) & 0xFF, (color >> 16) & 0xFF);
                for y in 0..size as usize {
                    for x in 0..size as usize {
                        let mut sum = 0;
                        for sy in 0..SS as usize {
                            for sx in 0..SS as usize {
                                sum += (src[(y * SS as usize + sy) * big as usize + x * SS as usize + sx] >> 8) & 0xFF;
                            }
                        }
                        let a = sum / (SS * SS) as u32;
                        dst[y * size as usize + x] = a << 24 | (cr * a / 255) << 16 | (cg * a / 255) << 8 | (cb * a / 255);
                    }
                }
                result = Some(bmp);
            }
            SelectObject(dc, old_font);
            DeleteObject(font);
            if result.is_some() {
                break; // GDI substituted another font otherwise: the loop tries the next one
            }
        }
        SelectObject(dc, old_bmp);
    }
    DeleteDC(dc);
    DeleteObject(big_bmp);
    if result.is_none() {
        DeleteObject(bmp);
    }
    result
}

/// Puts an icon to the left of the text of each (button, glyph) pair.
unsafe fn set_button_icons(buttons: &[(HWND, u16)], scale: f32) {
    let size = (16.0 * scale).round() as i32;
    let color = GetSysColor(COLOR_BTNTEXT);
    for &(btn, glyph) in buttons {
        let Some(bmp) = glyph_bitmap(glyph, size, color) else { return };
        let himl = ImageList_Create(size, size, ILC_COLOR32, 1, 0);
        ImageList_Add(himl, bmp, null_mut());
        DeleteObject(bmp);
        let bil = BUTTON_IMAGELIST {
            himl,
            margin: RECT { left: (8.0 * scale) as i32, top: 0, right: 0, bottom: 0 },
            uAlign: BUTTON_IMAGELIST_ALIGN_LEFT,
        };
        SendMessageW(btn, BCM_SETIMAGELIST, 0, &bil as *const _ as isize);
    }
}

/// Creates the dashboard window and its native child controls.
unsafe fn create_dash(hinst: HINSTANCE, class: &[u16]) {
    let icc = INITCOMMONCONTROLSEX { dwSize: mem::size_of::<INITCOMMONCONTROLSEX>() as u32, dwICC: ICC_LISTVIEW_CLASSES };
    InitCommonControlsEx(&icc);
    let dpi = GetDpiForSystem();
    let scale = dpi as f32 / 96.0;
    let s = |v: f32| (v * scale) as i32;
    let title = wide("GIF Overlay");
    let hwnd = CreateWindowExW(
        0, class.as_ptr(), title.as_ptr(), WS_OVERLAPPEDWINDOW,
        CW_USEDEFAULT, CW_USEDEFAULT, s(900.0), s(560.0),
        null_mut(), null_mut(), hinst, null(),
    );
    let child = |class: &str, text: &str, style: u32, id: usize| {
        CreateWindowExW(0, wide(class).as_ptr(), wide(text).as_ptr(), WS_CHILD | style, 0, 0, 0, 0, hwnd, id as HMENU, hinst, null())
    };
    let add = child("BUTTON", "Add image\u{2026}", WS_VISIBLE | WS_TABSTOP | BS_PUSHBUTTON as u32, IDC_ADD);
    let default_btn = child("BUTTON", "Set as default viewer\u{2026}", WS_VISIBLE | WS_TABSTOP | BS_PUSHBUTTON as u32, IDC_DEFAULT);
    let clear_btn = child("BUTTON", "Clear history", WS_VISIBLE | WS_TABSTOP | BS_PUSHBUTTON as u32, IDC_CLEAR);
    let help_btn = child("BUTTON", "How to use", WS_VISIBLE | WS_TABSTOP | BS_PUSHBUTTON as u32, IDC_HELP);
    let sort_box = child("COMBOBOX", "", WS_VISIBLE | WS_TABSTOP | WS_VSCROLL | 0x3 /* CBS_DROPDOWNLIST */, IDC_SORT);
    let list = child(
        "SysListView32", "",
        WS_VISIBLE | WS_TABSTOP | LVS_ICON | LVS_AUTOARRANGE | LVS_SINGLESEL | LVS_SHOWSELALWAYS,
        IDC_LIST,
    );
    let empty = child("STATIC", "No history yet\r\nClick \"Add image\u{2026}\" or drop an image here.", 0x1 /* SS_CENTER */, 0);

    // Explorer-style selection and system UI font, like any native app
    SetWindowTheme(list, wide("Explorer").as_ptr(), null());
    SendMessageW(list, LVM_SETEXTENDEDLISTVIEWSTYLE, 0, LVS_EX_DOUBLEBUFFER as isize);
    let mut ncm: NONCLIENTMETRICSW = mem::zeroed();
    ncm.cbSize = mem::size_of::<NONCLIENTMETRICSW>() as u32;
    SystemParametersInfoForDpi(SPI_GETNONCLIENTMETRICS, ncm.cbSize, &mut ncm as *mut _ as *mut c_void, 0, dpi);
    let font = CreateFontIndirectW(&ncm.lfMessageFont);
    for c in [add, default_btn, clear_btn, help_btn, sort_box, list, empty] {
        SendMessageW(c, WM_SETFONT, font as usize, 1);
    }
    // icons: add, settings, trash, help (Segoe Fluent Icons / MDL2 Assets code points)
    set_button_icons(&[(add, 0xE710), (default_btn, 0xE713), (clear_btn, 0xE74D), (help_btn, 0xE897)], scale);
    let sort = json_val(&fs::read_to_string(exe_file("settings.json")).unwrap_or_default(), "DashSort")
        .and_then(|v| v.parse::<u8>().ok())
        .filter(|&v| v == SORT_RECENT || v == SORT_NAME)
        .unwrap_or(SORT_RECENT);
    for label in ["Sort: Recently opened", "Sort: Name (A\u{2013}Z)"] {
        SendMessageW(sort_box, CB_ADDSTRING, 0, wide(label).as_ptr() as isize);
    }
    SendMessageW(sort_box, CB_SETCURSEL, sort as usize, 0);

    let (tw, th) = (s(144.0) as usize, s(112.0) as usize);
    SendMessageW(list, LVM_SETICONSPACING, 0, ((th as isize + s(60.0) as isize) << 16) | (tw as isize + s(28.0) as isize));
    *dash() = Dash { hwnd, list, add, default_btn, clear_btn, help_btn, sort_box, sort, empty, scale, tw, th, items: Vec::new(), loading: Vec::new() };
    DragAcceptFiles(hwnd, 1);
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
            add: null_mut(),
            default_btn: null_mut(),
            clear_btn: null_mut(),
            help_btn: null_mut(),
            sort_box: null_mut(),
            sort: SORT_RECENT,
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
        match std::env::args().nth(1).filter(|p| fs::metadata(p).is_ok()) {
            Some(p) => open_gif(&p),
            None => show_dash(),
        }

        let mut msg: MSG = mem::zeroed();
        while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
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
