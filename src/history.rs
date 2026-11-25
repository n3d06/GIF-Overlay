//! history.txt: one path per line, newest first, and the pasted-images folder.

use crate::*;

/// The saved history, or None when the file exists but cannot be read right now (a virus scanner
/// holding it, for example). Callers that rewrite the file must not treat that as "empty".
pub(crate) fn history_read() -> Option<Vec<String>> {
    let text = match fs::read_to_string(exe_file("history.txt")) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(_) => return None,
    };
    Some(
        text.lines()
            .map(|l| l.trim().trim_start_matches('\u{feff}')) // tolerate editors that add a BOM
            .filter(|l| !l.is_empty())
            .map(String::from)
            .collect(),
    )
}

/// Pasted / downloaded images only live while they are in the history. Files younger than `min_age`
/// are spared: one that was just saved is still loading and is not in the history yet.
pub(crate) fn prune_pasted(dir: &Path, keep: &[String], min_age: std::time::Duration) {
    for e in fs::read_dir(dir).into_iter().flatten().flatten() {
        let listed = keep.iter().any(|p| e.path().to_string_lossy().eq_ignore_ascii_case(p));
        let old = e.metadata().and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).is_some_and(|d| d >= min_age);
        if old && !listed {
            let _ = fs::remove_file(e.path());
        }
    }
}

pub(crate) fn history_save(h: &[String]) {
    write_atomic("history.txt", h.join("\n").as_bytes());
    prune_pasted(&exe_file("pasted"), h, std::time::Duration::from_secs(60));
}

pub(crate) fn history_push(path: &str) {
    let Some(mut h) = history_read() else { return }; // unreadable: keep the file as it is
    h.retain(|p| !p.eq_ignore_ascii_case(path));
    h.insert(0, path.to_string());
    h.truncate(HISTORY_MAX);
    history_save(&h);
}

pub(crate) fn history_remove(path: &str) {
    let Some(mut h) = history_read() else { return };
    h.retain(|p| !p.eq_ignore_ascii_case(path));
    history_save(&h);
}

