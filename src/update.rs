//! Version check against the latest GitHub release.

use crate::*;

pub(crate) const VERSION: &str = env!("CARGO_PKG_VERSION"); // stamped from the release tag by the pipeline

pub(crate) const RELEASES_URL: &str = "https://github.com/n3d06/GIF-Overlay/releases/latest";

pub(crate) const WM_UPDATE: u32 = WM_APP + 4;

pub(crate) static CHECKING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub(crate) fn version_parts(v: &str) -> Vec<u32> {
    v.trim_start_matches('v').split('.').map(|p| p.parse().unwrap_or(0)).collect()
}

/// Asks GitHub for the latest release on a worker thread (only when the user picks Help > Check for
/// updates); the tag comes back to the dashboard as WM_UPDATE.
pub(crate) fn check_update(silent: bool) {
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
        if unsafe { PostMessageW(dash_hwnd as HWND, WM_UPDATE, silent as usize, msg as isize) } == 0 {
            drop(unsafe { Box::from_raw(msg) });
        }
    });
}

pub(crate) fn today() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() / 86400)
}

/// Startup check, only when the user turned it on (Help menu) and at most once a day. It is quiet:
/// see `show_update_result`.
pub(crate) fn auto_check_update() {
    if dash().upd.auto && today() > dash().upd.last_check {
        check_update(true);
    }
}

/// `silent` is the daily check: no message unless a version the user has not been told about yet is out.
pub(crate) unsafe fn show_update_result(hwnd: HWND, mut res: Result<String, String>, silent: bool) {
    CHECKING.store(false, std::sync::atomic::Ordering::SeqCst);
    if silent {
        let d = dash();
        if res.is_ok() {
            d.upd.last_check = today(); // a failed check (offline) is retried at the next start
        }
        match res {
            Ok(tag) if version_parts(&tag) > version_parts(VERSION) && tag != d.upd.notified => d.upd.notified = tag,
            _ => {
                write_settings(saved_opacity());
                return;
            }
        }
        write_settings(saved_opacity());
        res = Ok(d.upd.notified.clone()); // fall through to the same message as the manual check
    }
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

