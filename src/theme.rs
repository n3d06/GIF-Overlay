//! Light / dark theme: the choice (follow Windows, always light, always dark), the colors, and the
//! pieces Windows does not theme by itself (title bar, popup menus, the menu bar).
//!
//! Popup menus and list scrollbars go dark through two undocumented but long-stable `uxtheme.dll`
//! entry points (ordinals 135 and 136, what Explorer's dark mode uses). If they are missing the
//! menus simply stay light. The menu bar is drawn by hand (the UAH messages), as there is no
//! documented way to color it.

use crate::*;
use windows_sys::Win32::{
    Graphics::Dwm::DwmSetWindowAttribute,
    System::LibraryLoader::{GetProcAddress, LoadLibraryExW, LOAD_LIBRARY_SEARCH_SYSTEM32},
};

pub(crate) const AUTO: u8 = 0;
pub(crate) const LIGHT: u8 = 1;
pub(crate) const DARK: u8 = 2;

/// The saved choice (settings.json "Theme").
static PREF: UiCell<u8> = UiCell::new(AUTO);
static BRUSH: UiCell<HBRUSH> = UiCell::new(null_mut());

pub(crate) fn pref() -> u8 {
    *PREF.get()
}

pub(crate) fn set_pref(v: u8) {
    *PREF.get() = v;
}

pub(crate) fn name(v: u8) -> &'static str {
    match v {
        LIGHT => "light",
        DARK => "dark",
        _ => "auto",
    }
}

pub(crate) fn from_name(s: &str) -> u8 {
    match s {
        "light" => LIGHT,
        "dark" => DARK,
        _ => AUTO,
    }
}

/// True when Windows is set to dark mode for apps (read every time, so a change applies at once).
pub(crate) fn system_dark() -> bool {
    unsafe {
        let (mut data, mut size) = (1u32, 4u32);
        let key = wide("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize");
        let value = wide("AppsUseLightTheme");
        let ok = RegGetValueW(HKEY_CURRENT_USER, key.as_ptr(), value.as_ptr(), RRF_RT_REG_DWORD, null_mut(), &mut data as *mut u32 as *mut c_void, &mut size);
        ok == 0 && data == 0
    }
}

pub(crate) fn dark() -> bool {
    match pref() {
        DARK => true,
        LIGHT => false,
        _ => system_dark(),
    }
}

/// COLORREF (0x00BBGGRR).
pub(crate) const fn rgb(r: u32, g: u32, b: u32) -> u32 {
    b << 16 | g << 8 | r
}

pub(crate) fn window_bg() -> u32 {
    if dark() { rgb(32, 32, 32) } else { rgb(255, 255, 255) }
}

pub(crate) fn text() -> u32 {
    if dark() { rgb(255, 255, 255) } else { rgb(0, 0, 0) }
}

pub(crate) fn text_dim() -> u32 {
    if dark() { rgb(140, 140, 140) } else { rgb(109, 109, 109) }
}

pub(crate) fn hot() -> u32 {
    if dark() { rgb(62, 62, 62) } else { rgb(229, 243, 255) }
}

/// Opaque 0xAARRGGBB pixel of a COLORREF (what the thumbnails are blended onto).
pub(crate) fn pixel(c: u32) -> u32 {
    0xFF00_0000 | (c & 0xFF) << 16 | ((c >> 8) & 0xFF) << 8 | (c >> 16) & 0xFF
}

/// A brush in the window color, kept until the theme changes.
pub(crate) fn bg_brush() -> HBRUSH {
    let b = BRUSH.get();
    if b.is_null() {
        *b = unsafe { CreateSolidBrush(window_bg()) };
    }
    *b
}

/// Tells Windows which mode the app is in, so popup menus and scrollbars match, and drops the
/// cached brush. Call at start and whenever the theme changes.
pub(crate) unsafe fn apply_app() {
    let b = BRUSH.get();
    if !b.is_null() {
        DeleteObject(*b);
        *b = null_mut();
    }
    let lib = LoadLibraryExW(wide("uxtheme.dll").as_ptr(), null_mut(), LOAD_LIBRARY_SEARCH_SYSTEM32);
    if lib.is_null() {
        return;
    }
    // 135 = SetPreferredAppMode (0 default, 1 allow dark, 2 force dark, 3 force light), 136 = FlushMenuThemes
    let set: Option<unsafe extern "system" fn(i32) -> i32> = mem::transmute(GetProcAddress(lib, 135 as *const u8));
    let flush: Option<unsafe extern "system" fn()> = mem::transmute(GetProcAddress(lib, 136 as *const u8));
    if let Some(set) = set {
        set(if dark() { 2 } else { 3 });
    }
    if let Some(flush) = flush {
        flush();
    }
}

/// Dark or light title bar (DWMWA_USE_IMMERSIVE_DARK_MODE).
pub(crate) unsafe fn apply_titlebar(hwnd: HWND) {
    let on: i32 = dark() as i32;
    DwmSetWindowAttribute(hwnd, 20, &on as *const i32 as *const c_void, 4);
}

// ---- the menu bar (undocumented UAH messages, the same ones Explorer's dark mode uses) ----

pub(crate) const WM_UAHDRAWMENU: u32 = 0x91;
pub(crate) const WM_UAHDRAWMENUITEM: u32 = 0x92;

#[repr(C)]
struct UahMenu {
    hmenu: HMENU,
    hdc: HDC,
    flags: u32,
}

#[repr(C)]
struct UahDrawMenuItem {
    dis: DRAWITEMSTRUCT,
    um: UahMenu,
    position: i32, // UAHMENUITEM.iPosition; its metrics follow and are not needed
}

/// WM_UAHDRAWMENU: paints the menu bar strip.
pub(crate) unsafe fn draw_menubar(hwnd: HWND, lp: LPARAM) {
    let um = &*(lp as *const UahMenu);
    let mut mbi: MENUBARINFO = mem::zeroed();
    mbi.cbSize = mem::size_of::<MENUBARINFO>() as u32;
    if GetMenuBarInfo(hwnd, OBJID_MENU, 0, &mut mbi) == 0 {
        return;
    }
    let mut win: RECT = mem::zeroed();
    GetWindowRect(hwnd, &mut win);
    let mut rc = mbi.rcBar;
    OffsetRect(&mut rc, -win.left, -win.top); // from screen to window coordinates
    FillRect(um.hdc, &rc, bg_brush());
}

/// WM_UAHDRAWMENUITEM: paints one top-level item (File, View, ...).
pub(crate) unsafe fn draw_menu_item(lp: LPARAM) {
    let item = &*(lp as *const UahDrawMenuItem);
    let mut buf = [0u16; 128];
    let mut mii: MENUITEMINFOW = mem::zeroed();
    mii.cbSize = mem::size_of::<MENUITEMINFOW>() as u32;
    mii.fMask = MIIM_STRING;
    mii.dwTypeData = buf.as_mut_ptr();
    mii.cch = buf.len() as u32 - 1;
    GetMenuItemInfoW(item.um.hmenu, item.position as u32, 1, &mut mii);

    let state = item.dis.itemState;
    let disabled = state & (ODS_GRAYED | ODS_DISABLED | ODS_INACTIVE) != 0;
    let hot = state & (ODS_HOTLIGHT | ODS_SELECTED) != 0;
    let hdc = item.dis.hDC;
    let brush = CreateSolidBrush(if hot { self::hot() } else { window_bg() });
    FillRect(hdc, &item.dis.rcItem, brush);
    DeleteObject(brush);
    SetBkMode(hdc, TRANSPARENT as i32);
    SetTextColor(hdc, if disabled { text_dim() } else { text() });
    let mut rc = item.dis.rcItem;
    let flags = DT_CENTER | DT_SINGLELINE | DT_VCENTER | if state & ODS_NOACCEL != 0 { DT_HIDEPREFIX } else { 0 };
    DrawTextW(hdc, buf.as_ptr(), mii.cch as i32, &mut rc, flags);
}

/// The one-pixel line Windows draws under the menu bar stays light; paint over it (after
/// WM_NCPAINT / WM_NCACTIVATE).
pub(crate) unsafe fn paint_menubar_line(hwnd: HWND) {
    let mut mbi: MENUBARINFO = mem::zeroed();
    mbi.cbSize = mem::size_of::<MENUBARINFO>() as u32;
    if GetMenuBarInfo(hwnd, OBJID_MENU, 0, &mut mbi) == 0 {
        return;
    }
    let mut client: RECT = mem::zeroed();
    GetClientRect(hwnd, &mut client);
    MapWindowPoints(hwnd, null_mut(), &mut client as *mut RECT as *mut POINT, 2);
    let mut win: RECT = mem::zeroed();
    GetWindowRect(hwnd, &mut win);
    OffsetRect(&mut client, -win.left, -win.top);
    let line = RECT { left: client.left, top: client.top - 1, right: client.right, bottom: client.top };
    let hdc = GetWindowDC(hwnd);
    FillRect(hdc, &line, bg_brush());
    ReleaseDC(hwnd, hdc);
}
