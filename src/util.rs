//! Small helpers shared by every module (wide strings, paths, atomic writes).

use crate::*;

pub(crate) fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain([0]).collect()
}

pub(crate) fn exe_file(name: &str) -> PathBuf {
    let mut p = std::env::current_exe().unwrap_or_default();
    p.set_file_name(name);
    p
}

/// Writes next to the exe via a temp file + rename, so a crash mid-write cannot leave a half file.
pub(crate) fn write_atomic(name: &str, data: &[u8]) {
    let tmp = exe_file(&format!("{name}.tmp"));
    if fs::write(&tmp, data).is_ok() && fs::rename(&tmp, exe_file(name)).is_err() {
        let _ = fs::remove_file(&tmp);
    }
}

pub(crate) fn abs_path(p: &str) -> String {
    std::path::absolute(p).map_or(p.to_string(), |p| p.to_string_lossy().into_owned())
}

