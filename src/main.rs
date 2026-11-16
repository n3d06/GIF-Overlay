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
        Input::KeyboardAndMouse::{
            EnableWindow, ReleaseCapture, SetCapture, SetFocus, TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT, VK_DELETE,
        },
        Shell::*,
        WindowsAndMessaging::*,
    },
};

const T_FRAME: usize = 1;
const T_SAVE: usize = 2;

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

fn write_settings(opacity: u32, locked: bool) {
    let json = format!(
        "{{\n  \"Opacity\": {},\n  \"IsLocked\": {},\n  \"DashSort\": {}\n}}",
        opacity as f64 / 100.0, locked, dash().sort
    );
    write_atomic("settings.json", json.as_bytes());
}

fn save_settings(a: &App) {
    write_settings(a.opacity, a.locked);
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
}

/// Resize handles: the corner keeps the aspect ratio, the edges change one axis only.
#[derive(Clone, Copy, PartialEq)]
enum Zone {
    None,
    Corner,
    Right,
    Bottom,
    Close, // the X button in the top-right corner
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
    } else if !a.locked && x >= a.w - grip(a) && y < grip(a) {
        Zone::Close
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
            // shrinking: average the covered source area (bilinear alone gives jagged edges)
            let shrink = (w as f64 / nw).min(h as f64 / nh) < 1.0;
            for y in 0..h {
                for x in 0..w {
                    dst[y * w + x] = if shrink {
                        let (u0, v0) = tf(x as f32 / w as f32, y as f32 / h as f32);
                        let (u1, v1) = tf((x + 1) as f32 / w as f32, (y + 1) as f32 / h as f32);
                        box_avg(f, a.gw, a.gh, u0.min(u1) * gwf, v0.min(v1) * ghf, u0.max(u1) * gwf, v0.max(v1) * ghf)
                    } else {
                        let (su, sv) = tf((x as f32 + 0.5) / w as f32, (y as f32 + 0.5) / h as f32);
                        sample(f, a.gw, a.gh, su * gwf - 0.5, sv * ghf - 0.5)
                    };
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
            if a.hover && !a.locked {
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
            // close button: dark square with a white X, red under the cursor (hidden when locked)
            if a.hover && !a.locked {
                let g = grip(a) as usize;
                let bg = if a.hot == Zone::Close { 0xE0C4_2B1B } else { 0x7000_0000 };
                for y in 0..g {
                    for x in w - g..w {
                        dst[y * w + x] = bg;
                    }
                }
                for i in g / 4..g - g / 4 {
                    for (y, x) in [(i, w - g + i), (i, w - 2 - i)] {
                        dst[y * w + x] = 0xFFFF_FFFF;
                        dst[y * w + x + 1] = 0xFFFF_FFFF; // 2px thick
                    }
                }
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

/// Natural size, scaled down to fit 80% of the screen.
fn fit_size(a: &App) -> (i32, i32) {
    let (mut w, mut h) = natural_size(a);
    let (sw, sh) = unsafe { (GetSystemMetrics(SM_CXSCREEN) as f64, GetSystemMetrics(SM_CYSCREEN) as f64) };
    if w > sw * 0.8 || h > sh * 0.8 {
        let r = (sw * 0.8 / w).min(sh * 0.8 / h);
        w *= r;
        h *= r;
    }
    (w.round() as i32, h.round() as i32)
}

fn resize(a: &mut App, w: i32, h: i32) {
    // upper bound: a huge window makes the DIB allocation fail and the render crawl
    let (sw, sh) = unsafe { (GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN)) };
    if w < 50 || h < 50 || w > sw * 3 || h > sh * 3 {
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
        let x = (GetSystemMetrics(SM_CXSCREEN) - a.w) / 2 + offset;
        let y = (GetSystemMetrics(SM_CYSCREEN) - a.h) / 2 + offset;
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
    (a.w, a.h) = fit_size(a);
    render(a);
    schedule_frame(a);
    let name = Path::new(path).file_name().map_or(path.to_string(), |n| n.to_string_lossy().into_owned());
    unsafe { SetWindowTextW(a.hwnd, wide(&name).as_ptr()) }; // tells windows apart in the taskbar
}

/// Replaces the image shown in this window (the window stays where it is).
fn change_image(a: &mut App, path: &str) -> bool {
    let path = abs_path(path);
    match decode(&path, usize::MAX) {
        Ok(d) => {
            apply(a, &path, d);
            history_push(&path);
            true
        }
        Err(e) => {
            show_error(&e);
            false
        }
    }
}

/// (opacity %, locked) remembered from the last session, used for every new window.
fn saved_defaults() -> (u32, bool) {
    let saved = fs::read_to_string(exe_file("settings.json")).unwrap_or_default();
    let opacity = json_val(&saved, "Opacity")
        .and_then(|v| v.parse::<f64>().ok())
        .map_or(100, |o| (o * 100.0).round().clamp(10.0, 100.0) as u32);
    (opacity, json_val(&saved, "IsLocked").is_some_and(|v| v == "true"))
}

/// Opens `path` in a new overlay window.
fn create_overlay(path: &str) -> bool {
    let d = match decode(path, usize::MAX) {
        Ok(d) => d,
        Err(e) => {
            show_error(&e);
            return false;
        }
    };
    let (opacity, locked) = saved_defaults();
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
            locked,
            hover: false,
            hot: Zone::None,
            drag: None,
            path: String::new(),
        }));
        let hwnd = CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_APPWINDOW,
            wide("GifOverlayWnd").as_ptr(), wide("GIF Overlay").as_ptr(), WS_POPUP,
            0, 0, 300, 300,
            null_mut(), null_mut(), GetModuleHandleW(null()), null(),
        );
        (*a).hwnd = hwnd;
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, a as isize);
        let offset = overlays().len() as i32 % 8 * 32;
        overlays().push(a);
        DragAcceptFiles(hwnd, 1);
        apply(&mut *a, path, d);
        center(&*a, offset);
        ShowWindow(hwnd, SW_SHOW);
        SetForegroundWindow(hwnd);
    }
    true
}

/// Opens `path` in a new window (or brings the window that already shows it to the front),
/// records it in the history and hides the dashboard.
fn open_gif(path: &str) -> bool {
    let path = abs_path(path);
    let existing = overlays().iter().copied().find(|&p| unsafe { (*p).path.eq_ignore_ascii_case(&path) });
    if let Some(p) = existing {
        unsafe {
            let hwnd = (*p).hwnd;
            ShowWindow(hwnd, SW_SHOW);
            SetForegroundWindow(hwnd);
        }
    } else if !create_overlay(&path) {
        return false;
    }
    history_push(&path);
    unsafe { ShowWindow(dash().hwnd, SW_HIDE) };
    true
}

fn rotate(a: &mut App, dir: u8) {
    a.rot = (a.rot + dir) % 4;
    mem::swap(&mut a.w, &mut a.h);
    render(a);
    schedule_save(a);
}

fn command(a: &mut App, id: usize) {
    match id {
        ID_LOCK => {
            a.locked = !a.locked;
            render(a); // show / hide the handles and the X right away
            schedule_save(a);
        }
        ID_OPEN => {
            if let Some(p) = open_dialog(a.hwnd) {
                change_image(a, &p);
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

        add(menu, MF_STRING | check(locked), ID_LOCK, "Lock Window");
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
            let zone = zone_at(a, lp as i16 as i32, (lp >> 16) as i16 as i32);
            if !a.locked {
                if zone != Zone::None {
                    let mut pt = POINT { x: 0, y: 0 };
                    GetCursorPos(&mut pt);
                    a.drag = Some((zone, pt.x, pt.y, a.w, a.h));
                    SetCapture(hwnd);
                } else {
                    ReleaseCapture();
                    SendMessageW(hwnd, WM_NCLBUTTONDOWN, HTCAPTION as usize, 0);
                }
            }
            0
        }
        WM_MOUSEMOVE => {
            if let Some((Zone::Close, ..)) = a.drag {
                // pressed on X: it only closes if the button is released still on it
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
                if zone == Zone::Close && zone_at(a, lp as i16 as i32, (lp >> 16) as i16 as i32) == Zone::Close {
                    command(a, ID_EXIT); // destroys the window: `a` is gone after this
                    return 0;
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
                Zone::Close => IDC_HAND,
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
                change_image(a, &p);
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
                    render(a);
                    schedule_frame(a);
                }
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
    sort_box: HWND,
    sort: u8, // SORT_RECENT (most recently opened first) or SORT_NAME
    empty: HWND,
    scale: f32, // DPI / 96
    tw: usize,  // thumbnail size in pixels
    th: usize,
    items: Vec<Thumb>,
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

/// Green dot with a white ring in the top-right corner of a thumbnail.
fn draw_open_badge(px: &mut [u32], tw: usize, scale: f32) {
    let r = 9.0 * scale;
    let (cx, cy) = (tw as f32 - r - 6.0 * scale, r + 6.0 * scale);
    let th = px.len() / tw;
    for y in 0..th {
        for x in 0..tw {
            let dist = ((x as f32 + 0.5 - cx).powi(2) + (y as f32 + 0.5 - cy).powi(2)).sqrt();
            if dist <= r + 2.0 * scale {
                px[y * tw + x] = if dist <= r { 0xFF16_A34A } else { 0xFFFF_FFFF };
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
    if !open_gif(&p) {
        history_remove(&p); // unreadable file: drop it from the list
        refresh_dash();
    }
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
        MoveWindow(d.add, s(12.0), s(10.0), s(120.0), s(32.0), 1);
        MoveWindow(d.default_btn, s(140.0), s(10.0), s(190.0), s(32.0), 1);
        MoveWindow(d.clear_btn, s(338.0), s(10.0), s(120.0), s(32.0), 1);
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
            (*(lp as *mut MINMAXINFO)).ptMinTrackSize.x = (700.0 * dash().scale) as i32;
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
            let (opacity, locked) = saved_defaults();
            write_settings(opacity, locked); // remember the choice
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
        CW_USEDEFAULT, CW_USEDEFAULT, s(860.0), s(560.0),
        null_mut(), null_mut(), hinst, null(),
    );
    let child = |class: &str, text: &str, style: u32, id: usize| {
        CreateWindowExW(0, wide(class).as_ptr(), wide(text).as_ptr(), WS_CHILD | style, 0, 0, 0, 0, hwnd, id as HMENU, hinst, null())
    };
    let add = child("BUTTON", "Add image\u{2026}", WS_VISIBLE | WS_TABSTOP | BS_PUSHBUTTON as u32, IDC_ADD);
    let default_btn = child("BUTTON", "Set as default viewer\u{2026}", WS_VISIBLE | WS_TABSTOP | BS_PUSHBUTTON as u32, IDC_DEFAULT);
    let clear_btn = child("BUTTON", "Clear history", WS_VISIBLE | WS_TABSTOP | BS_PUSHBUTTON as u32, IDC_CLEAR);
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
    for c in [add, default_btn, clear_btn, sort_box, list, empty] {
        SendMessageW(c, WM_SETFONT, font as usize, 1);
    }
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
    *dash() = Dash { hwnd, list, add, default_btn, clear_btn, sort_box, sort, empty, scale, tw, th, items: Vec::new() };
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
            sort_box: null_mut(),
            sort: SORT_RECENT,
            empty: null_mut(),
            scale: 1.0,
            tw: 0,
            th: 0,
            items: Vec::new(),
        });

        let hinst = GetModuleHandleW(null());
        let (oc, dc) = (wide("GifOverlayWnd"), wide("GifOverlayDash"));
        register(&oc, Some(overlay_proc), hinst);
        register(&dc, Some(dash_proc), hinst);
        create_dash(hinst, &dc);

        // "Open With" / command line goes straight to an overlay; a bare launch shows the history.
        let opened = std::env::args()
            .nth(1)
            .filter(|p| fs::metadata(p).is_ok())
            .is_some_and(|p| open_gif(&p));
        if !opened {
            show_dash();
        }

        let mut msg: MSG = mem::zeroed();
        while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}
