//! settings.json next to the exe.

use crate::*;

/// Value of `key` in a flat JSON object (settings.json, or GitHub's release answer). A string comes
/// back unescaped and without its quotes; a number or `true`/`false` comes back as written. This is
/// not a general JSON parser: it finds the first `"key"` and does not look into nested objects.
pub(crate) fn json_val(s: &str, key: &str) -> Option<String> {
    let i = s.find(&format!("\"{key}\""))? + key.len() + 2;
    let rest = s[i..].trim_start().strip_prefix(':')?.trim_start();
    if let Some(body) = rest.strip_prefix('"') {
        return json_string(body);
    }
    Some(rest.split([',', '\n', '\r', '}']).next()?.trim().to_string())
}

/// The text of a JSON string up to its closing quote (`body` starts after the opening one).
fn json_string(body: &str) -> Option<String> {
    let mut out = String::new();
    let mut chars = body.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => return Some(out),
            '\\' => match chars.next()? {
                'n' => out.push('\n'),
                'r' => out.push('\r'),
                't' => out.push('\t'),
                'u' => {
                    let hex: String = chars.by_ref().take(4).collect();
                    out.push(u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32).unwrap_or('\u{FFFD}'));
                }
                other => out.push(other), // \" \ \/
            },
            c => out.push(c),
        }
    }
    None // no closing quote
}

/// `s` as the inside of a JSON string.
fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
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
        opacity as f64 / 100.0, dash().sort, dash().restore, json_escape(dash().lang.map_or("auto", |i| lang::CODES[i])),
        dash().upd.auto, dash().upd.last_check, json_escape(&dash().upd.notified)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_numbers_bools_and_strings() {
        let s = "{\n  \"Opacity\": 0.8,\n  \"RestoreSession\": false,\n  \"Language\": \"vi\"\n}";
        assert_eq!(json_val(s, "Opacity").as_deref(), Some("0.8"));
        assert_eq!(json_val(s, "RestoreSession").as_deref(), Some("false"));
        assert_eq!(json_val(s, "Language").as_deref(), Some("vi"));
        assert_eq!(json_val(s, "Missing"), None);
    }

    #[test]
    fn strings_keep_commas_braces_and_escapes() {
        let s = r#"{"UpdateNotified": "v1,2}\"x\\yA", "Next": 3}"#;
        assert_eq!(json_val(s, "UpdateNotified").as_deref(), Some("v1,2}\"x\\yA"));
        assert_eq!(json_val(s, "Next").as_deref(), Some("3"));
        assert_eq!(json_val(r#"{"A": "no end"#, "A"), None);
    }

    #[test]
    fn escape_round_trips() {
        for text in ["plain", "a\"b", "back\\slash", "line\nbreak\ttab", "ctrl\u{1}", "tiếng Việt"] {
            let doc = format!("{{\"K\": \"{}\"}}", json_escape(text));
            assert_eq!(json_val(&doc, "K").as_deref(), Some(text), "{doc}");
        }
    }
}

