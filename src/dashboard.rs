//! The dashboard window: history list, menu bar and window procedure.

use crate::*;

pub(crate) const ISSUES_URL: &str = "https://github.com/n3d06/GIF-Overlay/issues";

pub(crate) const EMPTY_TEXT: &str = "No history yet\r\nUse File > Add image or drop an image here.";

pub(crate) const SORT_RECENT: u8 = 0;

pub(crate) const SORT_NAME: u8 = 1;

pub(crate) const SORT_NAME_DESC: u8 = 2;

pub(crate) const SORT_SIZE_DESC: u8 = 3; // largest file first

pub(crate) const SORT_SIZE_ASC: u8 = 4;

/// The "check for updates automatically" choice and what it remembers (settings.json).
pub(crate) struct UpdatePrefs {
    pub(crate) auto: bool, // on unless the user turns it off (Help menu)
    pub(crate) last_check: u64, // day number (days since 1970) of the last successful check
    pub(crate) notified: String, // newest version the user has already been told about
}

pub(crate) struct Dash {
    pub(crate) hwnd: HWND,
    pub(crate) list: HWND,
    pub(crate) sort: u8, // one of the SORT_* values (the View menu order)
    pub(crate) restore: bool, // reopen the last session's windows at start (File menu)
    pub(crate) lang: Option<usize>, // chosen language (index into lang::CODES); None follows the system
    pub(crate) upd: UpdatePrefs,
    pub(crate) empty: HWND,
    pub(crate) scale: f32, // DPI / 96
    pub(crate) tw: usize,  // thumbnail size in pixels
    pub(crate) th: usize,
    pub(crate) items: Vec<Thumb>,
    pub(crate) loading: Vec<String>, // files being decoded on worker threads
}

pub(crate) static DASH: UiCell<Option<Dash>> = UiCell::new(None);

pub(crate) fn dash() -> &'static mut Dash {
    DASH.get().as_mut().unwrap()
}

pub(crate) fn refresh_dash() {
    let d = dash();
    let mut history = history_read().unwrap_or_default();
    // forget entries whose file is gone; only a definite "not found" counts, so an unplugged drive keeps its entries
    let before = history.len();
    history.retain(|p| !matches!(fs::metadata(p), Err(e) if e.kind() == std::io::ErrorKind::NotFound));
    if history.len() != before {
        history_save(&history);
    }
    let mut old = mem::take(&mut d.items);
    let mut missing = Vec::new();
    d.items = history
        .into_iter()
        .filter(|p| fs::metadata(p).is_ok())
        .filter_map(|p| match old.iter().position(|t| t.path == p) {
            Some(i) => Some(old.swap_remove(i)), // keep what is already built (or still being built)
            None => {
                let t = placeholder_thumb(&p, d.tw, d.th)?;
                missing.push(p);
                Some(t)
            }
        })
        .collect();
    if !missing.is_empty() {
        spawn_thumbs(missing);
    }
    match d.sort {
        SORT_NAME => d.items.sort_by_key(|t| t.name.to_lowercase()),
        SORT_NAME_DESC => d.items.sort_by_key(|t| std::cmp::Reverse(t.name.to_lowercase())),
        SORT_SIZE_DESC => d.items.sort_by_key(|t| std::cmp::Reverse(t.size)),
        SORT_SIZE_ASC => d.items.sort_by_key(|t| t.size),
        _ => {} // SORT_RECENT keeps the history order, which is newest first
    }
    let open: Vec<String> = overlays().iter().map(|&p| unsafe { (*p).path.clone() }).collect();
    let is_open = |t: &Thumb| open.iter().any(|p| p.eq_ignore_ascii_case(&t.path));
    unsafe {
        SendMessageW(d.list, LVM_DELETEALLITEMS, 0, 0);
        let himl = ImageList_Create(d.tw as i32, d.th as i32, ILC_COLOR32, d.items.len() as i32, 4);
        for t in &d.items {
            let mut px = t.px.clone(); // the badge is drawn on a copy so the cached thumbnail stays clean
            if is_open(t) {
                draw_open_badge(&mut px, d.tw, d.scale);
            }
            let bmp = CreateBitmap(d.tw as i32, d.th as i32, 1, 32, px.as_ptr() as *const c_void);
            ImageList_Add(himl, bmp, null_mut());
            DeleteObject(bmp);
        }
        let prev = SendMessageW(d.list, LVM_SETIMAGELIST, LVSIL_NORMAL as usize, himl as isize);
        if prev != 0 {
            ImageList_Destroy(prev as HIMAGELIST);
        }
        for (i, t) in d.items.iter().enumerate() {
            let label = if is_open(t) { trf("{} (open)", &[&t.name]) } else { t.name.clone() };
            let mut name = wide(&label);
            let mut it: LVITEMW = mem::zeroed();
            it.mask = LVIF_TEXT | LVIF_IMAGE;
            it.iItem = i as i32;
            it.iImage = i as i32;
            it.pszText = name.as_mut_ptr();
            SendMessageW(d.list, LVM_INSERTITEMW, 0, &it as *const _ as isize);
        }
        ShowWindow(d.empty, if d.items.is_empty() { SW_SHOW } else { SW_HIDE });
        EnableMenuItem(GetMenu(d.hwnd), IDC_CLEAR as u32, MF_BYCOMMAND | if !d.items.is_empty() { MF_ENABLED } else { MF_GRAYED });
    }
}

pub(crate) fn show_dash() {
    refresh_dash();
    unsafe {
        let d = dash();
        ShowWindow(d.hwnd, SW_SHOW);
        ShowWindow(d.hwnd, SW_RESTORE);
        SetForegroundWindow(d.hwnd);
        SetFocus(d.list);
    }
}

pub(crate) fn selected() -> Option<usize> {
    let i = unsafe { SendMessageW(dash().list, LVM_GETNEXTITEM, usize::MAX, LVNI_SELECTED as isize) };
    (i >= 0).then_some(i as usize)
}

pub(crate) fn item_path(i: usize) -> Option<String> {
    dash().items.get(i).map(|t| t.path.clone())
}

pub(crate) fn remove_item(path: &str) {
    history_remove(path);
    refresh_dash();
}

pub(crate) fn layout() {
    let d = dash();
    if d.list.is_null() {
        return; // WM_SIZE can arrive before the children exist
    }
    let s = |v: f32| (v * d.scale) as i32;
    let (w, h) = {
        let mut r: RECT = unsafe { mem::zeroed() };
        unsafe { GetClientRect(d.hwnd, &mut r) };
        (r.right, r.bottom)
    };
    let list_h = h;
    unsafe {
        MoveWindow(d.list, 0, 0, w, list_h, 1);
        MoveWindow(d.empty, 0, list_h / 2 - s(24.0), w, s(48.0), 1);
    }
}

/// Right-click menu of the list: item actions when the click lands on a thumbnail, otherwise
/// Add / Paste. `lp` is the screen position, or -1 when opened from the keyboard (Menu / Shift+F10).
pub(crate) fn context_menu_dash(hwnd: HWND, lp: LPARAM) {
    unsafe {
        let list = dash().list;
        let (mut pt, item) = if lp == -1 {
            let i = selected();
            let mut r: RECT = mem::zeroed();
            if let Some(i) = i {
                SendMessageW(list, LVM_GETITEMRECT, i, &mut r as *mut RECT as isize); // r.left = 0 = LVIR_BOUNDS
            }
            (POINT { x: r.left + 8, y: r.top + 8 }, i)
        } else {
            let mut pt = POINT { x: lp as i16 as i32, y: (lp >> 16) as i16 as i32 };
            ScreenToClient(list, &mut pt);
            let mut hit: LVHITTESTINFO = mem::zeroed();
            hit.pt = pt;
            SendMessageW(list, LVM_HITTEST, 0, &mut hit as *mut LVHITTESTINFO as isize);
            (pt, (hit.iItem >= 0 && hit.flags & (LVHT_ONITEMICON | LVHT_ONITEMLABEL) != 0).then_some(hit.iItem as usize))
        };
        ClientToScreen(list, &mut pt);
        let menu = CreatePopupMenu();
        darkmenu::begin();
        let add = |flags: u32, id: usize, text: &str| {
            darkmenu::append(menu, flags, id, &tr(text), id == IDM_OPEN);
        };
        if let Some(i) = item {
            let exists = Path::new(&dash().items[i].path).exists();
            add(MF_STRING, IDM_OPEN, "Open");
            add(MF_STRING | if exists { 0 } else { MF_GRAYED }, IDM_FOLDER, "Show in folder");
            add(MF_STRING, IDM_COPYPATH, "Copy path");
            add(MF_SEPARATOR, 0, "");
            add(MF_STRING, IDM_REMOVE, "Remove from history");
            SetMenuDefaultItem(menu, IDM_OPEN as u32, 0); // double-click action in bold
        } else {
            add(MF_STRING, IDM_ADD, "Add image");
            add(MF_STRING, IDM_PASTE, "Paste image or link\tCtrl+V");
        }
        // the list can be refreshed while the menu is open, so remember the path now, not the index
        let path = item.and_then(item_path);
        let id = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_RIGHTBUTTON, pt.x, pt.y, 0, hwnd, null());
        DestroyMenu(menu);
        match (id as usize, path) {
            (IDM_OPEN, Some(p)) => open_gif(&p),
            (IDM_FOLDER, Some(p)) => show_in_folder(&p),
            (IDM_COPYPATH, Some(p)) => copy_text(hwnd, &p),
            (IDM_REMOVE, Some(p)) => remove_item(&p),
            (IDM_ADD, _) => {
                for p in open_dialog(hwnd, true) {
                    open_gif(&p);
                }
            }
            (IDM_PASTE, _) => paste(),
            _ => {}
        }
    }
}

pub(crate) unsafe extern "system" fn dash_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_SIZE => {
            layout();
            0
        }
        WM_SETFOCUS => {
            SetFocus(dash().list);
            0
        }
        WM_ERASEBKGND => {
            let mut r: RECT = mem::zeroed();
            GetClientRect(hwnd, &mut r);
            FillRect(wp as HDC, &r, GetSysColorBrush(COLOR_WINDOW));
            1
        }
        WM_GETMINMAXINFO => {
            // keep the menu bar from wrapping
            (*(lp as *mut MINMAXINFO)).ptMinTrackSize.x = (560.0 * dash().scale) as i32;
            0
        }
        WM_MEASUREITEM if darkmenu::measure(hwnd, lp) => 1,
        WM_DRAWITEM if darkmenu::draw(hwnd, lp) => 1,
        WM_COMMAND if wp & 0xFFFF == IDC_BUG => {
            ShellExecuteW(null_mut(), wide("open").as_ptr(), wide(ISSUES_URL).as_ptr(), null(), null(), SW_SHOWNORMAL);
            0
        }
        WM_COMMAND if wp & 0xFFFF == IDC_HOW_TO => {
            show_how_to(hwnd);
            0
        }
        WM_COMMAND if wp & 0xFFFF == IDC_CLEAR => {
            clear_history(hwnd);
            0
        }
        WM_COMMAND if (IDM_SORT_RECENT..=IDM_SORT_LAST).contains(&(wp & 0xFFFF)) => {
            set_sort(hwnd, wp);
            0
        }
        WM_COMMAND if (IDM_LANG_BASE..=IDM_LANG_BASE + lang::NAMES.len()).contains(&(wp & 0xFFFF)) => {
            set_language(hwnd, (wp & 0xFFFF).checked_sub(IDM_LANG_BASE + 1)); // first entry = system default
            0
        }
        WM_COMMAND if wp & 0xFFFF == IDC_AUTOUPD => {
            toggle_auto_update(hwnd);
            0
        }
        WM_COMMAND if wp & 0xFFFF == IDC_RESTORE => {
            toggle_restore(hwnd);
            0
        }
        WM_COMMAND if wp & 0xFFFF == IDC_PASTE => {
            paste();
            0
        }
        WM_COMMAND if wp & 0xFFFF == IDC_UPDATE => {
            check_update(false);
            0
        }
        WM_UPDATE => {
            show_update_result(hwnd, *Box::from_raw(lp as *mut Result<String, String>), wp != 0);
            0
        }
        // the Window menu (3rd of the bar) lists the open images each time it opens
        WM_INITMENUPOPUP if lp & 0xFFFF == 2 && (lp >> 16) & 0xFFFF == 0 => {
            fill_window_menu(wp);
            0
        }
        WM_COMMAND if (IDC_WIN_BASE..IDC_WIN_BASE + WIN_MAX).contains(&(wp & 0xFFFF)) => {
            if let Some(&p) = overlays().get((wp & 0xFFFF) - IDC_WIN_BASE) {
                show_overlay(&mut *p);
            }
            0
        }
        WM_COMMAND if wp & 0xFFFF == IDC_WIN_TOGGLE => {
            toggle_all();
            0
        }
        WM_HOTKEY => {
            toggle_all();
            0
        }
        WM_COMMAND if wp & 0xFFFF == IDC_WIN_MIN => {
            for p in overlays().clone() {
                minimize(&mut *p);
            }
            0
        }
        WM_COMMAND if wp & 0xFFFF == IDC_WIN_RESTORE => {
            for p in overlays().clone() {
                show_overlay(&mut *p);
            }
            0
        }
        WM_COMMAND if wp & 0xFFFF == IDC_WIN_CLOSE => {
            // destroying a window edits `overlays()`, so collect the handles first
            let all: Vec<usize> = overlays().iter().map(|&p| (*p).hwnd as usize).collect();
            for h in all {
                DestroyWindow(h as HWND);
            }
            0
        }
        WM_COMMAND if wp & 0xFFFF == IDC_QUIT_APP => {
            save_session();
            PostQuitMessage(0);
            0
        }
        WM_COMMAND if wp & 0xFFFF == IDC_DEFAULT => {
            open_default_apps();
            0
        }
        WM_COMMAND if wp & 0xFFFF == IDC_ADD => {
            for p in open_dialog(hwnd, true) {
                open_gif(&p);
            }
            0
        }
        WM_NOTIFY => {
            on_list_notify(hwnd, lp);
            0
        }
        WM_CONTEXTMENU => {
            if wp as HWND == dash().list {
                context_menu_dash(hwnd, lp);
            }
            0
        }
        // keep the empty-state label on the list's background
        WM_CTLCOLORSTATIC => {
            SetBkColor(wp as HDC, GetSysColor(COLOR_WINDOW));
            SetTextColor(wp as HDC, GetSysColor(COLOR_GRAYTEXT));
            GetSysColorBrush(COLOR_WINDOW) as LRESULT
        }
        WM_DROPFILES => {
            for p in dropped_files(wp) {
                open_gif(&p);
            }
            0
        }
        WM_LOADED => {
            finish_load(*Box::from_raw(lp as *mut Loaded));
            0
        }
        WM_THUMB => {
            on_thumb(lp);
            0
        }
        WM_FETCHED => {
            if dash().loading.is_empty() {
                SetWindowTextW(hwnd, wide("GIF Overlay").as_ptr());
            }
            match *Box::from_raw(lp as *mut Result<String, String>) {
                Ok(p) => open_gif(&p),
                Err(e) => show_error(&e),
            }
            0
        }
        // a second launch hands over its file (or nothing) instead of starting another process
        WM_COPYDATA => {
            on_copydata(lp);
            1
        }
        WM_CLOSE => {
            // With overlays on screen the dashboard just hides; otherwise closing it quits.
            if !overlays().is_empty() {
                ShowWindow(hwnd, SW_HIDE);
            } else {
                DestroyWindow(hwnd);
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

unsafe fn show_how_to(hwnd: HWND) {
    // one translated line per entry; "" is a blank line between the sections
    const LINES: [&str; 20] = [
        "OPEN AN IMAGE",
        "\u{2022} Use File > Add image, drop a file here, or double-click a thumbnail.",
        "\u{2022} Ctrl+V pastes a screenshot, a copied file, or an image link (http / https).",
        "\u{2022} Right-click a thumbnail: Open, Show in folder, Copy path (Ctrl+C), Remove (Delete).",
        "\u{2022} Right-click an empty spot for Add image and Paste.",
        "",
        "ON AN IMAGE",
        "\u{2022} Drag the image to move it. Hover it to show the buttons at the top right: minimize, maximize and close.",
        "\u{2022} Drag the right edge, bottom edge or the corner to resize. Ctrl + mouse wheel zooms.",
        "\u{2022} Maximize stretches the image over the whole screen; click it again to restore.",
        "\u{2022} Drop another file on the image to replace it.",
        "\u{2022} Right-click for the menu: scale, opacity, rotate / flip, pause, lock, always on top, copy image, show in folder, replace image, dashboard, close and quit.",
        "\u{2022} Keys: Esc closes, Space pauses, Left / Right step one frame, - / + change the speed, Up / Down change the opacity, Ctrl+C copies the image.",
        "",
        "LOCK WINDOW",
        "\u{2022} A locked image is click-through, so it never gets in the way.",
        "\u{2022} To unlock it, hold Ctrl and click the image.",
        "",
        "FOUND A PROBLEM?",
        "\u{2022} Use Help > Report a bug to open the issue page on GitHub.",
    ];
    let text = LINES.iter().map(|l| if l.is_empty() { String::new() } else { tr(l) }).collect::<Vec<_>>().join("\n");
    MessageBoxW(hwnd, wide(&text).as_ptr(), wide(&tr("How to use GIF Overlay")).as_ptr(), MB_OK | MB_ICONINFORMATION);
}

unsafe fn clear_history(hwnd: HWND) {
    let q = wide("Clear the history list?\nThe image files themselves are not deleted.");
    if MessageBoxW(hwnd, q.as_ptr(), wide(&tr("Clear history")).as_ptr(), MB_YESNO | MB_ICONQUESTION | MB_DEFBUTTON2) == IDYES {
        history_save(&[]);
        refresh_dash();
    }
}

unsafe fn set_sort(hwnd: HWND, wp: WPARAM) {
    let id = wp & 0xFFFF;
    dash().sort = (id - IDM_SORT_RECENT) as u8;
    CheckMenuRadioItem(GetSubMenu(GetMenu(hwnd), 1), IDM_SORT_RECENT as u32, IDM_SORT_LAST as u32, id as u32, MF_BYCOMMAND);
    write_settings(saved_opacity()); // remember the choice
    refresh_dash();
}

unsafe fn toggle_auto_update(hwnd: HWND) {
    let d = dash();
    d.upd.auto = !d.upd.auto;
    CheckMenuItem(GetMenu(hwnd), IDC_AUTOUPD as u32, MF_BYCOMMAND | if d.upd.auto { MF_CHECKED } else { MF_UNCHECKED });
    write_settings(saved_opacity());
}

unsafe fn toggle_restore(hwnd: HWND) {
    let d = dash();
    d.restore = !d.restore;
    CheckMenuItem(GetMenu(hwnd), IDC_RESTORE as u32, MF_BYCOMMAND | if d.restore { MF_CHECKED } else { MF_UNCHECKED });
    write_settings(saved_opacity()); // remember the choice
    if d.restore {
        save_session(); // the windows open right now are the ones to bring back
    } else {
        let _ = fs::remove_file(exe_file("session.txt")); // no stale session if it is turned on later
    }
}

unsafe fn fill_window_menu(wp: WPARAM) {
    let m = wp as HMENU;
    while DeleteMenu(m, 0, MF_BYPOSITION) != 0 {}
    let names: Vec<String> = overlays().iter().take(WIN_MAX).map(|&p| (*p).path.clone()).collect();
    if names.is_empty() {
        AppendMenuW(m, MF_STRING | MF_GRAYED, 0, wide(&tr("No open images")).as_ptr());
    }
    for (i, p) in names.iter().enumerate() {
        let name = Path::new(p).file_name().map_or(p.clone(), |n| n.to_string_lossy().into_owned());
        AppendMenuW(m, MF_STRING, IDC_WIN_BASE + i, wide(&name.replace('&', "&&")).as_ptr()); // && = a literal &
    }
    AppendMenuW(m, MF_SEPARATOR, 0, null());
    let on = if names.is_empty() { MF_GRAYED } else { 0 };
    AppendMenuW(m, MF_STRING | on, IDC_WIN_TOGGLE, wide(&tr("Hide / show all windows	Ctrl+Alt+G")).as_ptr());
    AppendMenuW(m, MF_STRING | on, IDC_WIN_MIN, wide(&tr("Minimize all")).as_ptr());
    AppendMenuW(m, MF_STRING | on, IDC_WIN_RESTORE, wide(&tr("Restore all")).as_ptr());
    AppendMenuW(m, MF_STRING | on, IDC_WIN_CLOSE, wide(&tr("Close all windows")).as_ptr());
}

unsafe fn on_list_notify(hwnd: HWND, lp: LPARAM) {
    let nm = &*(lp as *const NMHDR);
    if nm.hwndFrom == dash().list {
        if nm.code == LVN_ITEMACTIVATE {
            if let Some(p) = item_path((*(lp as *const NMITEMACTIVATE)).iItem as usize) {
                open_gif(&p); // an unreadable file is dropped from the list once its decode fails
            }
        } else if nm.code == LVN_KEYDOWN {
            let key = (*(lp as *const NMLVKEYDOWN)).wVKey;
            if key == VK_DELETE {
                if let Some(p) = selected().and_then(item_path) {
                    remove_item(&p);
                }
            } else if key == 0x43 && GetAsyncKeyState(VK_CONTROL as i32) < 0 {
                if let Some(p) = selected().and_then(item_path) {
                    copy_text(hwnd, &p);
                }
            }
        }
    }
}

unsafe fn on_thumb(lp: LPARAM) {
    let (path, thumb) = *Box::from_raw(lp as *mut (String, Option<Thumb>));
    if let Some(item) = dash().items.iter_mut().find(|t| t.path == path) {
        match thumb {
            Some(t) => *item = t,
            None => item.ready = true, // undecodable: keep the blank, do not retry
        }
        refresh_dash();
    }
}

unsafe fn on_copydata(lp: LPARAM) {
    let cds = &*(lp as *const COPYDATASTRUCT);
    if cds.cbData >= 2 {
        let s = slice::from_raw_parts(cds.lpData as *const u16, cds.cbData as usize / 2);
        open_gif(String::from_utf16_lossy(s).trim_end_matches('\0'));
    } else {
        show_dash();
    }
}

/// Index into `lang::CODES` for the system's display language.
pub(crate) fn system_language() -> usize {
    lang::detect(unsafe { windows_sys::Win32::Globalization::GetUserDefaultUILanguage() })
}

/// Switches the language on the fly: the menu bar is rebuilt, the other texts follow when they next appear.
pub(crate) unsafe fn set_language(hwnd: HWND, pref: Option<usize>) {
    let d = dash();
    d.lang = pref;
    lang::set(pref.unwrap_or_else(system_language));
    let old = GetMenu(hwnd);
    SetMenu(hwnd, null_mut());
    DestroyMenu(old);
    build_menu(hwnd, &MenuState { sort: d.sort, restore: d.restore, lang: pref, auto_update: d.upd.auto });
    SetWindowTextW(d.empty, wide(&tr(EMPTY_TEXT)).as_ptr());
    write_settings(saved_opacity()); // remember the choice
    refresh_dash(); // "(open)" labels
}

/// What the menu bar shows checked: the View > Sort and Language choices and the two File / Help toggles.
#[derive(Clone, Copy)]
pub(crate) struct MenuState {
    pub(crate) sort: u8,
    pub(crate) restore: bool,
    pub(crate) lang: Option<usize>, // None = follow the system
    pub(crate) auto_update: bool,
}

/// Menu bar of the dashboard: File / View / Help. Commands reuse the old button ids.
pub(crate) unsafe fn build_menu(hwnd: HWND, m: &MenuState) {
    let MenuState { sort, restore, lang: pref, auto_update } = *m;
    let bar = CreateMenu();
    let (file, view, help) = (CreatePopupMenu(), CreatePopupMenu(), CreatePopupMenu());
    let add = |m: HMENU, flags: u32, id: usize, text: &str| {
        AppendMenuW(m, flags, id, wide(&tr(text)).as_ptr());
    };
    add(file, MF_STRING, IDC_ADD, "Add image\tCtrl+O");
    add(file, MF_STRING, IDC_PASTE, "Paste image or link\tCtrl+V");
    add(file, MF_SEPARATOR, 0, "");
    add(file, MF_STRING, IDC_CLEAR, "Clear history");
    add(file, MF_STRING | if restore { MF_CHECKED } else { 0 }, IDC_RESTORE, "Reopen windows on start");
    add(file, MF_SEPARATOR, 0, "");
    add(file, MF_STRING, IDC_QUIT_APP, "Quit GIF Overlay");
    add(view, MF_STRING, IDM_SORT_RECENT, "Sort by recently opened");
    add(view, MF_STRING, IDM_SORT_RECENT + SORT_NAME as usize, "Sort by name (A\u{2013}Z)");
    add(view, MF_STRING, IDM_SORT_RECENT + SORT_NAME_DESC as usize, "Sort by name (Z\u{2013}A)");
    add(view, MF_STRING, IDM_SORT_RECENT + SORT_SIZE_DESC as usize, "Sort by size (largest first)");
    add(view, MF_STRING, IDM_SORT_RECENT + SORT_SIZE_ASC as usize, "Sort by size (smallest first)");
    add(view, MF_SEPARATOR, 0, "");
    let langs = CreatePopupMenu();
    add(langs, MF_STRING, IDM_LANG_BASE, "System default");
    for (i, name) in lang::NAMES.iter().enumerate() {
        AppendMenuW(langs, MF_STRING, IDM_LANG_BASE + 1 + i, wide(name).as_ptr()); // each language in its own name
    }
    CheckMenuRadioItem(langs, IDM_LANG_BASE as u32, (IDM_LANG_BASE + lang::NAMES.len()) as u32, (IDM_LANG_BASE + pref.map_or(0, |i| i + 1)) as u32, MF_BYCOMMAND);
    add(view, MF_POPUP, langs as usize, "Language");
    add(help, MF_STRING, IDC_HOW_TO, "How to use");
    add(help, MF_STRING, IDC_DEFAULT, "Set as default viewer");
    add(help, MF_SEPARATOR, 0, "");
    add(help, MF_STRING, IDC_BUG, "Report a bug");
    add(help, MF_SEPARATOR, 0, "");
    add(help, MF_STRING, IDC_UPDATE, "Check for updates");
    add(help, MF_STRING | if auto_update { MF_CHECKED } else { 0 }, IDC_AUTOUPD, "Check for updates automatically");
    add(help, MF_STRING | MF_GRAYED, IDC_VERSION, &trf("Version {}", &[VERSION]));
    add(bar, MF_POPUP, file as usize, "File");
    add(bar, MF_POPUP, view as usize, "View");
    add(bar, MF_POPUP, CreatePopupMenu() as usize, "Window"); // filled in by WM_INITMENUPOPUP
    add(bar, MF_POPUP, help as usize, "Help");
    CheckMenuRadioItem(view, IDM_SORT_RECENT as u32, IDM_SORT_LAST as u32, (IDM_SORT_RECENT + sort as usize) as u32, MF_BYCOMMAND);
    SetMenu(hwnd, bar);
}

/// Creates the dashboard window and its native child controls.
pub(crate) unsafe fn create_dash(hinst: HINSTANCE, class: &[u16]) {
    let icc = INITCOMMONCONTROLSEX { dwSize: mem::size_of::<INITCOMMONCONTROLSEX>() as u32, dwICC: ICC_LISTVIEW_CLASSES };
    InitCommonControlsEx(&icc);
    let settings = fs::read_to_string(exe_file("settings.json")).unwrap_or_default();
    // the texts below are created in the saved language (no saved choice: the system's display language)
    let pref = json_val(&settings, "Language").and_then(|v| lang::CODES.iter().position(|c| *c == v.trim_matches('"')));
    lang::set(pref.unwrap_or_else(system_language));
    let dpi = GetDpiForSystem();
    let scale = dpi as f32 / 96.0;
    let s = |v: f32| (v * scale) as i32;
    let title = wide("GIF Overlay");
    let hwnd = CreateWindowExW(
        0, class.as_ptr(), title.as_ptr(), WS_OVERLAPPEDWINDOW,
        CW_USEDEFAULT, CW_USEDEFAULT, s(960.0), s(560.0),
        null_mut(), null_mut(), hinst, null(),
    );
    let child = |class: &str, text: &str, style: u32, id: usize| {
        CreateWindowExW(0, wide(class).as_ptr(), wide(text).as_ptr(), WS_CHILD | style, 0, 0, 0, 0, hwnd, id as HMENU, hinst, null())
    };
    let list = child(
        "SysListView32", "",
        WS_VISIBLE | WS_TABSTOP | LVS_ICON | LVS_AUTOARRANGE | LVS_SINGLESEL | LVS_SHOWSELALWAYS,
        IDC_LIST,
    );
    let empty = child("STATIC", &tr(EMPTY_TEXT), 0x1 /* SS_CENTER */, 0);

    // Explorer-style selection and system UI font, like any native app
    SetWindowTheme(list, wide("Explorer").as_ptr(), null());
    SendMessageW(list, LVM_SETEXTENDEDLISTVIEWSTYLE, 0, LVS_EX_DOUBLEBUFFER as isize);
    let mut ncm: NONCLIENTMETRICSW = mem::zeroed();
    ncm.cbSize = mem::size_of::<NONCLIENTMETRICSW>() as u32;
    SystemParametersInfoForDpi(SPI_GETNONCLIENTMETRICS, ncm.cbSize, &mut ncm as *mut _ as *mut c_void, 0, dpi);
    let font = CreateFontIndirectW(&ncm.lfMessageFont);
    for c in [list, empty] {
        SendMessageW(c, WM_SETFONT, font as usize, 1);
    }
    let sort = json_val(&fs::read_to_string(exe_file("settings.json")).unwrap_or_default(), "DashSort")
        .and_then(|v| v.parse::<u8>().ok())
        .filter(|&v| v <= SORT_SIZE_ASC)
        .unwrap_or(SORT_RECENT);
    let restore = json_val(&settings, "RestoreSession").is_none_or(|v| v != "false"); // on unless turned off
    let upd = UpdatePrefs {
        auto: json_val(&settings, "AutoUpdate").is_none_or(|v| v != "false"),
        last_check: json_val(&settings, "LastUpdateCheck").and_then(|v| v.parse().ok()).unwrap_or(0),
        notified: json_val(&settings, "UpdateNotified").map(|v| v.trim_matches('"').to_string()).unwrap_or_default(),
    };
    build_menu(hwnd, &MenuState { sort, restore, lang: pref, auto_update: upd.auto });

    let (tw, th) = (s(144.0) as usize, s(112.0) as usize);
    SendMessageW(list, LVM_SETICONSPACING, 0, ((th as isize + s(60.0) as isize) << 16) | (tw as isize + s(28.0) as isize));
    *dash() = Dash { hwnd, list, sort, restore, lang: pref, upd, empty, scale, tw, th, items: Vec::new(), loading: Vec::new() };
    DragAcceptFiles(hwnd, 1);
    RegisterHotKey(hwnd, 1, MOD_CONTROL | MOD_ALT | MOD_NOREPEAT, 0x47); // Ctrl+Alt+G; if another app owns it the Window menu item still works
    layout();
}

