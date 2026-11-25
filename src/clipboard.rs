//! Paste (image, file or image link) and copy to the clipboard.

use crate::*;

pub(crate) const CF_DIB: u32 = 8;

pub(crate) const CF_UNICODETEXT: u32 = 13;

pub(crate) const CF_HDROP: u32 = 15;

pub(crate) const WM_FETCHED: u32 = WM_APP + 3;

/// Pasted and downloaded images live here (next to the exe, like history.txt) so the history can reopen them.
pub(crate) fn pasted_file(name: &str) -> Option<PathBuf> {
    let dir = exe_file("pasted");
    fs::create_dir_all(&dir).ok()?;
    let ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis());
    Some(dir.join(format!("{ms}-{name}")))
}

pub(crate) enum Clip {
    File(String),
    Url(String),
    Image(Vec<u8>, &'static str), // file bytes + extension
}

pub(crate) unsafe fn clip_bytes(h: HANDLE) -> Option<Vec<u8>> {
    let p = GlobalLock(h) as *const u8;
    if p.is_null() {
        return None;
    }
    let v = slice::from_raw_parts(p, GlobalSize(h)).to_vec();
    GlobalUnlock(h);
    Some(v)
}

/// A clipboard DIB is a BMP file minus its 14-byte header; put it back so WIC can decode it.
pub(crate) fn dib_to_bmp(dib: &[u8]) -> Option<Vec<u8>> {
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
pub(crate) unsafe fn read_clipboard() -> Option<Clip> {
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
pub(crate) fn paste() {
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
pub(crate) fn fetch(url: String) {
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

pub(crate) unsafe fn set_clip(format: u32, bytes: &[u8]) {
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

pub(crate) fn copy_text(owner: HWND, s: &str) {
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
pub(crate) fn copy_image(a: &App) {
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

