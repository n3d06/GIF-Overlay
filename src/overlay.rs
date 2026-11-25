//! The overlay window: its state, geometry, commands, context menu and window procedure.

use crate::*;

pub(crate) struct App {
    pub(crate) hwnd: HWND,
    pub(crate) frames: Vec<Vec<u32>>, // premultiplied BGRA, gw*gh each
    pub(crate) delays: Vec<u32>,      // ms
    pub(crate) gw: usize,
    pub(crate) gh: usize,
    pub(crate) cur: usize,
    pub(crate) paused: bool,
    pub(crate) rot: u8, // quarter turns clockwise
    pub(crate) flip_h: bool,
    pub(crate) flip_v: bool,
    pub(crate) w: i32,
    pub(crate) h: i32,
    pub(crate) opacity: u32, // percent
    pub(crate) locked: bool,
    pub(crate) hover: bool, // mouse is over the overlay: show the resize handles
    pub(crate) hot: Zone,   // handle under the cursor (drawn highlighted)
    pub(crate) drag: Option<(Zone, i32, i32, i32, i32)>, // zone, start cursor x, y, start width, height
    pub(crate) path: String, // absolute path of the image shown (lets the dashboard mark / reuse open windows)
    pub(crate) restore: Option<(i32, i32, i32, i32)>, // x, y, w, h before "maximize"
    pub(crate) speed: usize, // index into SPEEDS
}

/// Resize handles: the corner keeps the aspect ratio, the edges change one axis only.
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Zone {
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
pub(crate) static OVERLAYS: UiCell<Vec<*mut App>> = UiCell::new(Vec::new());

pub(crate) fn overlays() -> &'static mut Vec<*mut App> {
    OVERLAYS.get()
}

pub(crate) fn dpi_px(a: &App, v: i32) -> i32 {
    unsafe { v * GetDpiForWindow(a.hwnd) as i32 / 96 }
}

/// Side of the corner grip triangle in pixels (never more than half the image).
pub(crate) fn btn(a: &App) -> i32 {
    grip(a).min(a.w / 3)
}

pub(crate) fn grip(a: &App) -> i32 {
    dpi_px(a, 22).min(a.w.min(a.h) / 2)
}

/// Thickness of the right/bottom edge handles.
pub(crate) fn edge(a: &App) -> i32 {
    dpi_px(a, 8).max(6).min(a.w.min(a.h) / 4)
}

pub(crate) fn zone_at(a: &App, x: i32, y: i32) -> Zone {
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

/// Steps one frame (wraps around) and pauses, so the frames can be inspected one by one.
pub(crate) fn step_frame(a: &mut App, forward: bool) {
    let n = a.frames.len();
    if n < 2 {
        return;
    }
    a.paused = true;
    a.cur = if forward { (a.cur + 1) % n } else { (a.cur + n - 1) % n };
    schedule_frame(a); // paused: just stops the timer
    render(a);
}

pub(crate) fn schedule_save(a: &App) {
    unsafe { SetTimer(a.hwnd, T_SAVE, 500, None) }; // re-arming resets the debounce
}

pub(crate) fn schedule_frame(a: &App) {
    unsafe {
        if a.frames.len() > 1 && !a.paused {
            let ms = (a.delays[a.cur] as f32 / SPEEDS[a.speed]).round().max(10.0) as u32;
            SetTimer(a.hwnd, T_FRAME, ms, None);
        } else {
            KillTimer(a.hwnd, T_FRAME);
        }
    }
}

pub(crate) fn natural_size(a: &App) -> (f64, f64) {
    if a.rot % 2 == 1 { (a.gh as f64, a.gw as f64) } else { (a.gw as f64, a.gh as f64) }
}

/// Bounds of the monitor the window is on (or, before it is shown, the one under the cursor).
pub(crate) fn screen_rect(hwnd: HWND) -> RECT {
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
pub(crate) fn fit_size(a: &App) -> (i32, i32) {
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

pub(crate) fn resize(a: &mut App, w: i32, h: i32) {
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
pub(crate) fn center(a: &App, offset: i32) {
    unsafe {
        let m = screen_rect(a.hwnd);
        let x = m.left + (m.right - m.left - a.w) / 2 + offset;
        let y = m.top + (m.bottom - m.top - a.h) / 2 + offset;
        SetWindowPos(a.hwnd, null_mut(), x, y, 0, 0, SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE);
    }
}

pub(crate) fn show_error(e: &str) {
    unsafe {
        MessageBoxW(null_mut(), wide(e).as_ptr(), wide(&tr("Error")).as_ptr(), MB_OK | MB_ICONERROR);
    }
}

/// Puts a decoded image into the window and fits it to the screen.
pub(crate) fn apply(a: &mut App, path: &str, (gw, gh, frames, delays): Decoded) {
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

pub(crate) static HIDDEN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Hides every image window, or brings them all back (Ctrl+Alt+G anywhere, or Window menu).
pub(crate) fn toggle_all() {
    use std::sync::atomic::Ordering::Relaxed;
    let hide = !HIDDEN.load(Relaxed) && !overlays().is_empty();
    for &p in overlays().iter() {
        unsafe { ShowWindow((*p).hwnd, if hide { SW_HIDE } else { SW_SHOWNA }) };
    }
    HIDDEN.store(hide, Relaxed);
}

/// Opens an already decoded image in a new overlay window; `state` puts it back as it was.
pub(crate) fn create_overlay(path: &str, d: Decoded, state: Option<&Saved>) {
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
pub(crate) fn minimize(a: &mut App) {
    (a.hover, a.hot) = (false, Zone::None);
    unsafe { ShowWindow(a.hwnd, SW_MINIMIZE) };
}

/// Brings the window back from minimized / hidden and to the front.
pub(crate) fn show_overlay(a: &mut App) {
    unsafe {
        ShowWindow(a.hwnd, SW_RESTORE);
        SetForegroundWindow(a.hwnd);
    }
}

/// Stretches the image over the whole monitor it is on (aspect ratio is not kept); a second click restores size and position.
pub(crate) fn toggle_max(a: &mut App) {
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

pub(crate) fn rotate(a: &mut App, dir: u8) {
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
pub(crate) fn apply_lock(a: &App) {
    unsafe {
        if a.locked {
            SetTimer(a.hwnd, T_LOCK, 80, None);
        } else {
            KillTimer(a.hwnd, T_LOCK);
        }
        set_click_through(a.hwnd, a.locked);
    }
}

pub(crate) unsafe fn set_click_through(hwnd: HWND, on: bool) {
    let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
    let new = if on { ex | WS_EX_TRANSPARENT as isize } else { ex & !(WS_EX_TRANSPARENT as isize) };
    if new != ex {
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, new);
    }
}

/// Timer tick of a locked window: interactive only while Ctrl is held with the cursor over it.
pub(crate) unsafe fn lock_tick(a: &App) {
    let (mut pt, mut r) = (POINT { x: 0, y: 0 }, mem::zeroed::<RECT>());
    GetCursorPos(&mut pt);
    GetWindowRect(a.hwnd, &mut r);
    let poke = GetAsyncKeyState(VK_CONTROL as i32) < 0 && PtInRect(&r, pt) != 0;
    set_click_through(a.hwnd, !poke);
}

pub(crate) fn command(a: &mut App, id: usize) {
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

pub(crate) unsafe fn app_of(hwnd: HWND) -> Option<&'static mut App> {
    (GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut App).as_mut()
}

pub(crate) fn context_menu(hwnd: HWND) {
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

pub(crate) unsafe fn track_leave(hwnd: HWND) {
    let mut t = TRACKMOUSEEVENT { cbSize: mem::size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: hwnd, dwHoverTime: 0 };
    TrackMouseEvent(&mut t);
}

pub(crate) unsafe extern "system" fn overlay_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
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

