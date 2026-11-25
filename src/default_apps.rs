//! "Open with" / default-apps registration.

use crate::*;

pub(crate) fn reg_set(subkey: &str, name: Option<&str>, value: &str) {
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
pub(crate) fn open_default_apps() {
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

