//! Every window timer id and menu / control command id in one place. Each group has its own
//! number range, so a new id only has to be unique inside its group.

// ---- overlay window: timers ----
pub(crate) const T_FRAME: usize = 1;
pub(crate) const T_SAVE: usize = 2;
pub(crate) const T_LOCK: usize = 3;

// ---- overlay window: context menu (1..=15), then ranges for the sub-menus ----
pub(crate) const ID_LOCK: usize = 1;
pub(crate) const ID_OPEN: usize = 2;
pub(crate) const ID_ROT_L: usize = 3;
pub(crate) const ID_ROT_R: usize = 4;
pub(crate) const ID_FLIP_H: usize = 5;
pub(crate) const ID_FLIP_V: usize = 6;
pub(crate) const ID_RESET: usize = 7;
pub(crate) const ID_PAUSE: usize = 8;
pub(crate) const ID_EXIT: usize = 9;
pub(crate) const ID_HISTORY: usize = 10;
pub(crate) const ID_QUIT: usize = 11;
pub(crate) const ID_PASTE: usize = 12;
pub(crate) const ID_COPY: usize = 13;
pub(crate) const ID_FOLDER: usize = 14;
pub(crate) const ID_TOPMOST: usize = 15;
pub(crate) const ID_SCALE: usize = 100; // + index into SCALES
pub(crate) const ID_OPACITY: usize = 200; // + index into OPACITIES
pub(crate) const ID_SPEED: usize = 300; // + index into SPEEDS

// ---- dashboard: menu bar and controls (1001..=1023) ----
pub(crate) const IDC_ADD: usize = 1001;
pub(crate) const IDC_LIST: usize = 1002; // the list view
pub(crate) const IDC_DEFAULT: usize = 1003;
pub(crate) const IDC_CLEAR: usize = 1004;
pub(crate) const IDC_HOW_TO: usize = 1006;
pub(crate) const IDC_BUG: usize = 1007;
pub(crate) const IDC_PASTE: usize = 1009;
pub(crate) const IDC_QUIT_APP: usize = 1010;
pub(crate) const IDC_UPDATE: usize = 1012;
pub(crate) const IDC_VERSION: usize = 1013;
pub(crate) const IDC_RESTORE: usize = 1014;
pub(crate) const IDC_AUTOUPD: usize = 1015;
pub(crate) const IDC_WIN_MIN: usize = 1020;
pub(crate) const IDC_WIN_RESTORE: usize = 1021;
pub(crate) const IDC_WIN_CLOSE: usize = 1022;
pub(crate) const IDC_WIN_TOGGLE: usize = 1023;

// ---- dashboard: sort and language items of the View menu ----
pub(crate) const IDM_SORT_RECENT: usize = 1101;
pub(crate) const IDM_SORT_LAST: usize = 1105; // the five sort items are IDM_SORT_RECENT + the SORT_* value
pub(crate) const IDM_LANG_BASE: usize = 1200; // "system default", then + 1 + the language index

// ---- dashboard: the Window menu lists the open images ----
pub(crate) const IDC_WIN_BASE: usize = 2000; // + index of an open image, up to WIN_MAX
pub(crate) const WIN_MAX: usize = 100;

// ---- dashboard: right-click menu of a history item (its own menu, so 1..=6 do not clash) ----
pub(crate) const IDM_OPEN: usize = 1;
pub(crate) const IDM_REMOVE: usize = 2;
pub(crate) const IDM_FOLDER: usize = 3;
pub(crate) const IDM_COPYPATH: usize = 4;
pub(crate) const IDM_ADD: usize = 5;
pub(crate) const IDM_PASTE: usize = 6;
