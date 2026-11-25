#![windows_subsystem = "windows"]

use std::{
    ffi::c_void,
    fs, mem,
    path::{Path, PathBuf},
    ptr::{null, null_mut},
    slice,
};
mod clipboard;
mod dashboard;
mod decode;
mod darkmenu;
mod default_apps;
mod files;
mod history;
mod ids;
mod lang;
mod loader;
mod overlay;
mod render;
mod session;
mod settings;
mod thumbs;
mod update;
mod util;

// Every module does `use crate::*`, so the items below are one shared namespace (the app is a
// single Win32 message loop); the modules only split the code by topic.
pub(crate) use clipboard::*;
pub(crate) use dashboard::*;
pub(crate) use default_apps::*;
pub(crate) use files::*;
pub(crate) use history::*;
pub(crate) use ids::*;
pub(crate) use loader::*;
pub(crate) use overlay::*;
pub(crate) use render::*;
pub(crate) use session::*;
pub(crate) use settings::*;
pub(crate) use thumbs::*;
pub(crate) use update::*;
pub(crate) use util::*;

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

const SCALES: [u32; 7] = [25, 50, 75, 100, 150, 200, 300];
const OPACITIES: [u32; 10] = [10, 20, 30, 40, 50, 60, 70, 80, 90, 100];
const SPEEDS: [f32; 6] = [0.25, 0.5, 1.0, 1.5, 2.0, 4.0];
const SPEED_NORMAL: usize = 2; // index of 1x
const HISTORY_MAX: usize = 100;

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
        *DASH.get() = Some(Dash {
            hwnd: null_mut(),
            list: null_mut(),
            sort: SORT_RECENT,
            restore: true,
            lang: None,
            upd: UpdatePrefs { auto: true, last_check: 0, notified: String::new() },
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

        auto_check_update();

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
