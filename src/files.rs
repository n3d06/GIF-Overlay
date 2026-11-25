//! Open dialog, drag and drop, and Explorer helpers.

use crate::*;

/// Several files can be picked when `multi` is set (up to 10, like a drop).
pub(crate) fn open_dialog(owner: HWND, multi: bool) -> Vec<String> {
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
pub(crate) unsafe fn dropped_files(wp: WPARAM) -> Vec<String> {
    let hdrop = wp as HDROP;
    let count = DragQueryFileW(hdrop, u32::MAX, null_mut(), 0).min(10);
    let out = (0..count).filter_map(|i| drop_path(hdrop, i)).collect();
    DragFinish(hdrop);
    out
}

/// File `i` of a drop / clipboard file list, however long its path is.
pub(crate) unsafe fn drop_path(hdrop: HDROP, i: u32) -> Option<String> {
    let n = DragQueryFileW(hdrop, i, null_mut(), 0) as usize;
    let mut buf = vec![0u16; n + 1];
    let n = DragQueryFileW(hdrop, i, buf.as_mut_ptr(), buf.len() as u32) as usize;
    (n > 0).then(|| String::from_utf16_lossy(&buf[..n]))
}

pub(crate) fn show_in_folder(path: &str) {
    let args = wide(&format!("/select,\"{path}\""));
    unsafe { ShellExecuteW(null_mut(), wide("open").as_ptr(), wide("explorer.exe").as_ptr(), args.as_ptr(), null(), SW_SHOWNORMAL) };
}

