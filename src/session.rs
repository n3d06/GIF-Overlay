//! session.txt: the open windows, reopened at the next start.

use crate::*;

/// What is remembered about one window.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Saved {
    pub(crate) x: i32,
    pub(crate) y: i32,
    pub(crate) w: i32,
    pub(crate) h: i32,
    pub(crate) opacity: u32,
    pub(crate) locked: bool,
    pub(crate) rot: u8,
    pub(crate) flip_h: bool,
    pub(crate) flip_v: bool,
}

/// One line per window: x, y, w, h, opacity, locked, rotation, flip h, flip v, then the path (last, so it may hold tabs).
pub(crate) fn session_line(path: &str, s: &Saved) -> String {
    format!("{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}", s.x, s.y, s.w, s.h, s.opacity, s.locked as u8, s.rot, s.flip_h as u8, s.flip_v as u8, path)
}

/// Lines that do not parse are skipped, so a damaged file loses only that window.
pub(crate) fn parse_session(text: &str) -> Vec<(String, Saved)> {
    text.lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.trim_start_matches('\u{feff}').splitn(10, '\t').collect();
            let n = |i: usize| f.get(i)?.parse::<i32>().ok();
            let s = Saved {
                x: n(0)?,
                y: n(1)?,
                w: n(2)?,
                h: n(3)?,
                opacity: n(4)?.clamp(10, 100) as u32,
                locked: n(5)? != 0,
                rot: (n(6)?.rem_euclid(4)) as u8,
                flip_h: n(7)? != 0,
                flip_v: n(8)? != 0,
            };
            Some((f.get(9).filter(|p| !p.is_empty())?.to_string(), s))
        })
        .collect()
}

/// Writes the open windows. Skipped while images are still loading (at startup the other windows
/// of the last session are on their way and would be forgotten).
pub(crate) fn save_session() {
    if dash().restore && dash().loading.is_empty() {
        let lines: Vec<String> = overlays().iter().map(|&p| unsafe { window_state(&*p) }).collect();
        write_atomic("session.txt", lines.join("\n").as_bytes());
    }
}

pub(crate) unsafe fn window_state(a: &App) -> String {
    let mut r: RECT = mem::zeroed();
    GetWindowRect(a.hwnd, &mut r);
    if IsIconic(a.hwnd) != 0 {
        // a minimized window sits off screen: its restored place is in the window placement
        let mut wp: WINDOWPLACEMENT = mem::zeroed();
        wp.length = mem::size_of::<WINDOWPLACEMENT>() as u32;
        GetWindowPlacement(a.hwnd, &mut wp);
        r = wp.rcNormalPosition;
    }
    // maximized: remember the size and place the window had before
    let (x, y, w, h) = a.restore.unwrap_or((r.left, r.top, a.w, a.h));
    let s = Saved { x, y, w, h, opacity: a.opacity, locked: a.locked, rot: a.rot, flip_h: a.flip_h, flip_v: a.flip_v };
    session_line(&a.path, &s)
}

