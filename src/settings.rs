//! settings.json next to the exe.

use crate::*;

pub(crate) fn json_val(s: &str, key: &str) -> Option<String> {
    let i = s.find(&format!("\"{key}\""))? + key.len() + 2;
    let rest = s[i..].trim_start().strip_prefix(':')?.trim_start();
    Some(rest.split([',', '\n', '\r', '}']).next()?.trim().to_string())
}

pub(crate) fn write_settings(opacity: u32) {
    let json = format!(
        "{{
  \"Opacity\": {},
  \"DashSort\": {},
  \"RestoreSession\": {},
  \"Language\": \"{}\",
  \"AutoUpdate\": {},
  \"LastUpdateCheck\": {},
  \"UpdateNotified\": \"{}\"
}}",
        opacity as f64 / 100.0, dash().sort, dash().restore, dash().lang.map_or("auto", |i| lang::CODES[i]),
        dash().upd.auto, dash().upd.last_check, dash().upd.notified
    );
    write_atomic("settings.json", json.as_bytes());
}

pub(crate) fn save_settings(a: &App) {
    write_settings(a.opacity);
    save_session();
}

/// Opacity (%) remembered from the last session, used for every new window. The lock is not
/// remembered: a new window always starts unlocked so it can be moved right away.
pub(crate) fn saved_opacity() -> u32 {
    let saved = fs::read_to_string(exe_file("settings.json")).unwrap_or_default();
    json_val(&saved, "Opacity")
        .and_then(|v| v.parse::<f64>().ok())
        .map_or(100, |o| (o * 100.0).round().clamp(10.0, 100.0) as u32)
}

