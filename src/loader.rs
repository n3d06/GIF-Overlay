//! Loading an image on a worker thread and turning it into an overlay.

use crate::*;

pub(crate) const WM_LOADED: u32 = WM_APP + 2;

/// Result of a background decode, posted to the dashboard window.
pub(crate) struct Loaded {
    pub(crate) path: String,
    pub(crate) target: usize, // overlay window to put the image in (0 = open a new window)
    pub(crate) res: Result<Decoded, String>,
    pub(crate) state: Option<Saved>, // set when the window is being restored from the last session
}

pub(crate) fn set_title(hwnd: HWND, path: &str) {
    let name = Path::new(path).file_name().map_or(path.to_string(), |n| n.to_string_lossy().into_owned());
    unsafe { SetWindowTextW(hwnd, wide(&name).as_ptr()) }; // tells windows apart in the taskbar
}

/// Decodes on a worker thread so a big GIF cannot freeze the UI; `finish_load` runs when it is done.
pub(crate) fn start_load(path: &str, target: HWND) {
    start_load_with(path, target, None);
}

pub(crate) fn start_load_with(path: &str, target: HWND, state: Option<Saved>) {
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
pub(crate) unsafe fn finish_load(l: Loaded) {
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
pub(crate) fn open_gif(path: &str) {
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

