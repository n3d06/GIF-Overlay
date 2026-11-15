#![windows_subsystem = "windows"]

use std::{
    ffi::c_void,
    fs, mem,
    path::PathBuf,
    ptr::{addr_of_mut, null, null_mut},
    slice,
};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::Gdi::*,
    System::LibraryLoader::GetModuleHandleW,
    UI::{Controls::Dialogs::*, Input::KeyboardAndMouse::ReleaseCapture, WindowsAndMessaging::*},
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
const ID_SCALE: usize = 100; // + index into SCALES
const ID_OPACITY: usize = 200; // + index into OPACITIES
const SCALES: [u32; 7] = [25, 50, 75, 100, 150, 200, 300];
const OPACITIES: [u32; 10] = [10, 20, 30, 40, 50, 60, 70, 80, 90, 100];

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
    last: String,
}

static mut APP: Option<App> = None;

// Single UI thread; the window procedure is the only caller.
fn app() -> &'static mut App {
    unsafe { (*addr_of_mut!(APP)).as_mut().unwrap() }
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain([0]).collect()
}

// ---------- settings (settings.json next to the exe, same keys as the C# version) ----------

fn settings_path() -> PathBuf {
    let mut p = std::env::current_exe().unwrap_or_default();
    p.set_file_name("settings.json");
    p
}

fn json_val(s: &str, key: &str) -> Option<String> {
    let i = s.find(&format!("\"{key}\""))? + key.len() + 2;
    let rest = s[i..].trim_start().strip_prefix(':')?.trim_start();
    if let Some(body) = rest.strip_prefix('"') {
        let (mut out, mut esc) = (String::new(), false);
        for c in body.chars() {
            match (esc, c) {
                (true, c) => { out.push(c); esc = false }
                (false, '\\') => esc = true,
                (false, '"') => return Some(out),
                (false, c) => out.push(c),
            }
        }
        None
    } else {
        Some(rest.split([',', '\n', '\r', '}']).next()?.trim().to_string())
    }
}

fn save_settings(a: &App) {
    let esc = a.last.replace('\\', "\\\\").replace('"', "\\\"");
    let json = format!(
        "{{\n  \"Width\": {},\n  \"Height\": {},\n  \"Opacity\": {},\n  \"IsLocked\": {},\n  \"LastGifPath\": \"{}\"\n}}",
        a.w, a.h, a.opacity as f64 / 100.0, a.locked, esc
    );
    let _ = fs::write(settings_path(), json);
}

// ---------- GIF decoding ----------

type Decoded = (usize, usize, Vec<Vec<u32>>, Vec<u32>);

fn decode(path: &str) -> Result<Decoded, String> {
    let file = fs::File::open(path).map_err(|e| e.to_string())?;
    let mut opts = gif::DecodeOptions::new();
    opts.set_color_output(gif::ColorOutput::RGBA);
    let mut dec = opts.read_info(file).map_err(|e| e.to_string())?;
    let (gw, gh) = (dec.width() as usize, dec.height() as usize);
    if gw == 0 || gh == 0 {
        return Err("empty image".into());
    }
    let mut canvas = vec![0u32; gw * gh];
    let (mut frames, mut delays) = (Vec::new(), Vec::new());
    while let Some(f) = dec.read_next_frame().map_err(|e| e.to_string())? {
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

// ---------- rendering ----------

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
            for y in 0..h {
                for x in 0..w {
                    let (mut u, mut v) = ((x as f32 + 0.5) / w as f32, (y as f32 + 0.5) / h as f32);
                    if a.flip_h { u = 1.0 - u }
                    if a.flip_v { v = 1.0 - v }
                    let (su, sv) = match a.rot {
                        0 => (u, v),
                        1 => (v, 1.0 - u),
                        2 => (1.0 - u, 1.0 - v),
                        _ => (1.0 - v, u),
                    };
                    dst[y * w + x] = sample(f, a.gw, a.gh, su * a.gw as f32 - 0.5, sv * a.gh as f32 - 0.5);
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

// ---------- behaviour ----------

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
    if w < 50 || h < 50 {
        return;
    }
    a.w = w;
    a.h = h;
    render(a);
    schedule_save(a);
}

fn center(a: &App) {
    unsafe {
        let x = (GetSystemMetrics(SM_CXSCREEN) - a.w) / 2;
        let y = (GetSystemMetrics(SM_CYSCREEN) - a.h) / 2;
        SetWindowPos(a.hwnd, null_mut(), x, y, 0, 0, SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE);
    }
}

fn load(a: &mut App, path: &str) {
    match decode(path) {
        Ok((gw, gh, frames, delays)) => {
            a.gw = gw;
            a.gh = gh;
            a.frames = frames;
            a.delays = delays;
            a.cur = 0;
            a.rot = 0;
            a.flip_h = false;
            a.flip_v = false;
            (a.w, a.h) = fit_size(a);
            // relative paths (e.g. from the command line) would break once the cwd changes
            a.last = std::path::absolute(path).map_or(path.to_string(), |p| p.to_string_lossy().into_owned());
            render(a);
            center(a);
            schedule_frame(a);
            schedule_save(a);
        }
        Err(e) => unsafe {
            MessageBoxW(a.hwnd, wide(&format!("Failed to load GIF: {e}")).as_ptr(), wide("Error").as_ptr(), MB_OK | MB_ICONERROR);
        },
    }
}

fn open_dialog(owner: HWND) -> Option<String> {
    let filter: Vec<u16> = "GIF Files\0*.gif\0\0".encode_utf16().collect();
    let title = wide("Select GIF File");
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
            schedule_save(a);
        }
        ID_OPEN => {
            if let Some(p) = open_dialog(a.hwnd) {
                load(a, &p);
            }
        }
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

fn context_menu(a: &mut App) {
    unsafe {
        let add = |m: HMENU, flags: u32, id: usize, text: &str| {
            AppendMenuW(m, flags, id, wide(text).as_ptr());
        };
        let check = |on: bool| if on { MF_CHECKED } else { 0 };
        let menu = CreatePopupMenu();
        let (scale, opac, xform, settings) = (CreatePopupMenu(), CreatePopupMenu(), CreatePopupMenu(), CreatePopupMenu());
        for (i, s) in SCALES.iter().enumerate() {
            add(scale, MF_STRING, ID_SCALE + i, &format!("{s}%"));
        }
        for (i, o) in OPACITIES.iter().enumerate() {
            add(opac, MF_STRING | check(a.opacity == *o), ID_OPACITY + i, &format!("{o}%"));
        }
        add(xform, MF_STRING, ID_ROT_L, "Rotate Left");
        add(xform, MF_STRING, ID_ROT_R, "Rotate Right");
        add(xform, MF_STRING | check(a.flip_h), ID_FLIP_H, "Flip Horizontal");
        add(xform, MF_STRING | check(a.flip_v), ID_FLIP_V, "Flip Vertical");
        add(settings, MF_POPUP, scale as usize, "Scale");
        add(settings, MF_POPUP, opac as usize, "Opacity");
        add(settings, MF_POPUP, xform as usize, "Transform");
        add(settings, MF_SEPARATOR, 0, "");
        add(settings, MF_STRING, ID_RESET, "Reset to Default");

        add(menu, MF_STRING | check(a.locked), ID_LOCK, "Lock Window");
        add(menu, MF_SEPARATOR, 0, "");
        add(menu, MF_STRING, ID_OPEN, "Change GIF...");
        add(menu, MF_SEPARATOR, 0, "");
        add(menu, MF_POPUP, settings as usize, "Settings");
        add(menu, MF_STRING | check(a.paused), ID_PAUSE, "Pause / Play");
        add(menu, MF_SEPARATOR, 0, "");
        add(menu, MF_STRING, ID_EXIT, "Exit");

        let mut pt = POINT { x: 0, y: 0 };
        GetCursorPos(&mut pt);
        SetForegroundWindow(a.hwnd);
        let hwnd = a.hwnd;
        // The menu runs a nested message loop; timers keep firing, so don't hold `a` across it.
        let id = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_RIGHTBUTTON, pt.x, pt.y, 0, hwnd, null());
        DestroyMenu(menu); // also destroys the submenus
        if id > 0 {
            command(app(), id as usize);
        }
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_LBUTTONDOWN => {
            if !app().locked {
                ReleaseCapture();
                SendMessageW(hwnd, WM_NCLBUTTONDOWN, HTCAPTION as usize, 0);
            }
            0
        }
        WM_RBUTTONUP => {
            context_menu(app());
            0
        }
        WM_MOUSEWHEEL => {
            if wp & 0x8 != 0 { // MK_CONTROL
                let a = app();
                let k = if (wp >> 16) as i16 > 0 { 1.1 } else { 0.9 };
                resize(a, (a.w as f64 * k) as i32, (a.h as f64 * k) as i32);
            }
            0
        }
        WM_TIMER => {
            let a = app();
            match wp {
                T_FRAME => {
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
            PostQuitMessage(0);
            0
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

fn main() {
    let saved = fs::read_to_string(settings_path()).unwrap_or_default();
    let opacity = json_val(&saved, "Opacity")
        .and_then(|v| v.parse::<f64>().ok())
        .map_or(100, |o| (o * 100.0).round().clamp(10.0, 100.0) as u32);
    let locked = json_val(&saved, "IsLocked").is_some_and(|v| v == "true");
    let last = json_val(&saved, "LastGifPath").unwrap_or_default();

    unsafe {
        APP = Some(App {
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
            last: String::new(),
        });

        let hinst = GetModuleHandleW(null());
        let class = wide("GifOverlayWnd");
        let wc = WNDCLASSEXW {
            cbSize: mem::size_of::<WNDCLASSEXW>() as u32,
            style: 0,
            lpfnWndProc: Some(wndproc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: hinst,
            hIcon: LoadIconW(hinst, 1 as *const u16), // resource id 1 = app icon (build.rs)
            hCursor: LoadCursorW(null_mut(), IDC_ARROW),
            hbrBackground: null_mut(),
            lpszMenuName: null(),
            lpszClassName: class.as_ptr(),
            hIconSm: null_mut(),
        };
        RegisterClassExW(&wc);
        let hwnd = CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_APPWINDOW,
            class.as_ptr(),
            wide("GIF Overlay").as_ptr(),
            WS_POPUP,
            0, 0, 300, 300,
            null_mut(), null_mut(), hinst, null(),
        );
        app().hwnd = hwnd;

        // Priority: command line (Open With) > last opened > bundled demo
        let demo = std::env::current_exe()
            .map(|mut p| { p.set_file_name("assets"); p.join("demo1.gif") })
            .unwrap_or_default();
        let start = std::env::args()
            .nth(1)
            .filter(|p| fs::metadata(p).is_ok())
            .or_else(|| fs::metadata(&last).is_ok().then_some(last))
            .or_else(|| demo.to_str().filter(|p| fs::metadata(p).is_ok()).map(String::from));
        if let Some(p) = start {
            load(app(), &p);
        }
        // Nothing to show (first run, or the GIF failed to load): ask for one, quit if declined.
        if app().frames.is_empty() {
            if let Some(p) = open_dialog(null_mut()) {
                load(app(), &p);
            }
            if app().frames.is_empty() {
                return;
            }
        }
        ShowWindow(hwnd, SW_SHOW);

        let mut msg: MSG = mem::zeroed();
        while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}
