//! Dark popup menu. The native popup menu cannot be themed, so when the system's app theme is dark
//! the items are owner-drawn (MF_OWNERDRAW + WM_MEASUREITEM / WM_DRAWITEM, answered by the window
//! that opened the menu). In light mode the menu stays native.

use crate::*;

const BG: u32 = 0x002B_2B2B; // COLORREF is 0x00BBGGRR; all of these are grays
const BG_HOT: u32 = 0x003D_3D3D;
const TEXT: u32 = 0x00FF_FFFF;
const TEXT_DIM: u32 = 0x0080_8080;
const TEXT_HINT: u32 = 0x00A0_A0A0;
const LINE: u32 = 0x004D_4D4D;

struct Entry {
    text: String,
    separator: bool,
    bold: bool,
}

/// The items of the menu being shown; an item's `itemData` is its index + 1.
static ENTRIES: UiCell<Vec<Entry>> = UiCell::new(Vec::new());

/// True when Windows is set to dark mode for apps (re-read at every menu, so a theme change applies).
pub(crate) fn system_dark() -> bool {
    unsafe {
        let (mut data, mut size) = (1u32, 4u32);
        let key = wide("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize");
        let value = wide("AppsUseLightTheme");
        let ok = RegGetValueW(HKEY_CURRENT_USER, key.as_ptr(), value.as_ptr(), RRF_RT_REG_DWORD, null_mut(), &mut data as *mut u32 as *mut c_void, &mut size);
        ok == 0 && data == 0
    }
}

/// Call before building a menu that goes through `append`.
pub(crate) fn begin() {
    ENTRIES.get().clear();
}

/// Adds an item (or a separator when `flags` has MF_SEPARATOR); owner-drawn in dark mode.
pub(crate) unsafe fn append(menu: HMENU, flags: u32, id: usize, text: &str, bold: bool) {
    if !system_dark() {
        let w = wide(text);
        AppendMenuW(menu, flags, id, if flags & MF_SEPARATOR != 0 { null() } else { w.as_ptr() });
        return;
    }
    let entries = ENTRIES.get();
    entries.push(Entry { text: text.to_string(), separator: flags & MF_SEPARATOR != 0, bold });
    // a separator is a disabled owner-drawn item that draws a line
    let flags = (flags & !MF_SEPARATOR) | MF_OWNERDRAW | if flags & MF_SEPARATOR != 0 { MF_GRAYED } else { 0 };
    AppendMenuW(menu, flags, id, entries.len() as *const u16);
}

unsafe fn entry(item_data: usize) -> Option<&'static Entry> {
    ENTRIES.get().get(item_data.checked_sub(1)?)
}

/// The system's menu font at the window's DPI.
unsafe fn menu_font(hwnd: HWND, bold: bool) -> HFONT {
    let mut ncm: NONCLIENTMETRICSW = mem::zeroed();
    ncm.cbSize = mem::size_of::<NONCLIENTMETRICSW>() as u32;
    SystemParametersInfoForDpi(SPI_GETNONCLIENTMETRICS, ncm.cbSize, &mut ncm as *mut _ as *mut c_void, 0, GetDpiForWindow(hwnd));
    if bold {
        ncm.lfMenuFont.lfWeight = 700;
    }
    CreateFontIndirectW(&ncm.lfMenuFont)
}

unsafe fn text_width(hdc: HDC, s: &str) -> i32 {
    let w: Vec<u16> = s.encode_utf16().collect();
    let mut size = SIZE { cx: 0, cy: 0 };
    GetTextExtentPoint32W(hdc, w.as_ptr(), w.len() as i32, &mut size);
    size.cx
}

/// `label\tshortcut` as two parts.
fn split_tab(s: &str) -> (&str, &str) {
    s.split_once('\t').unwrap_or((s, ""))
}

/// WM_MEASUREITEM for a menu item. False when the message is not about one of our menu items.
pub(crate) unsafe fn measure(hwnd: HWND, lp: LPARAM) -> bool {
    let mis = &mut *(lp as *mut MEASUREITEMSTRUCT);
    let Some(e) = (if mis.CtlType == ODT_MENU { entry(mis.itemData) } else { None }) else { return false };
    let scale = dash().scale;
    if e.separator {
        mis.itemHeight = (9.0 * scale) as u32;
        mis.itemWidth = 0;
        return true;
    }
    let hdc = GetDC(hwnd);
    let font = menu_font(hwnd, e.bold);
    let old = SelectObject(hdc, font);
    let (label, hint) = split_tab(&e.text);
    let mut w = text_width(hdc, label);
    if !hint.is_empty() {
        w += (24.0 * scale) as i32 + text_width(hdc, hint);
    }
    let mut tm: TEXTMETRICW = mem::zeroed();
    GetTextMetricsW(hdc, &mut tm);
    SelectObject(hdc, old);
    DeleteObject(font);
    ReleaseDC(hwnd, hdc);
    mis.itemWidth = (w + (28.0 * scale) as i32) as u32;
    mis.itemHeight = (tm.tmHeight as f32 + 10.0 * scale) as u32;
    true
}

/// WM_DRAWITEM for a menu item. False when the message is not about one of our menu items.
pub(crate) unsafe fn draw(hwnd: HWND, lp: LPARAM) -> bool {
    let di = &*(lp as *const DRAWITEMSTRUCT);
    let Some(e) = (if di.CtlType == ODT_MENU { entry(di.itemData) } else { None }) else { return false };
    let scale = dash().scale;
    let (hdc, mut rc) = (di.hDC, di.rcItem);
    let disabled = di.itemState & (ODS_GRAYED | ODS_DISABLED) != 0;
    let hot = di.itemState & ODS_SELECTED != 0 && !disabled;

    let brush = CreateSolidBrush(if hot { BG_HOT } else { BG });
    FillRect(hdc, &rc, brush);
    DeleteObject(brush);

    if e.separator {
        let y = rc.top + (rc.bottom - rc.top) / 2;
        let pen = CreateSolidBrush(LINE);
        let line = RECT { left: rc.left + (8.0 * scale) as i32, top: y, right: rc.right - (8.0 * scale) as i32, bottom: y + 1 };
        FillRect(hdc, &line, pen);
        DeleteObject(pen);
        return true;
    }

    let font = menu_font(hwnd, e.bold);
    let old = SelectObject(hdc, font);
    SetBkMode(hdc, TRANSPARENT as i32);
    let pad = (14.0 * scale) as i32;
    rc.left += pad;
    rc.right -= pad;
    let (label, hint) = split_tab(&e.text);
    let flags = DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX;
    SetTextColor(hdc, if disabled { TEXT_DIM } else { TEXT });
    DrawTextW(hdc, wide(label).as_ptr(), -1, &mut rc, flags | DT_LEFT);
    if !hint.is_empty() {
        SetTextColor(hdc, if disabled { TEXT_DIM } else { TEXT_HINT });
        DrawTextW(hdc, wide(hint).as_ptr(), -1, &mut rc, flags | DT_RIGHT);
    }
    SelectObject(hdc, old);
    DeleteObject(font);
    true
}
