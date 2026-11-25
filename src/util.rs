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


/// State that only the UI thread touches: every window procedure runs there, and the worker
/// threads only post messages (they never read these globals). It replaces `static mut`, keeps the
/// one `unsafe` in a single place, and in debug builds panics if a second thread ever calls `get`.
/// Callers must not hold the returned reference across a call that fetches it again.
pub(crate) struct UiCell<T> {
    value: std::cell::UnsafeCell<T>,
    owner: std::sync::OnceLock<std::thread::ThreadId>,
}

unsafe impl<T> Sync for UiCell<T> {}

impl<T> UiCell<T> {
    pub(crate) const fn new(value: T) -> Self {
        UiCell { value: std::cell::UnsafeCell::new(value), owner: std::sync::OnceLock::new() }
    }

    #[allow(clippy::mut_from_ref)]
    pub(crate) fn get(&'static self) -> &'static mut T {
        let id = *self.owner.get_or_init(|| std::thread::current().id());
        debug_assert_eq!(id, std::thread::current().id(), "UI-only state used from another thread");
        unsafe { &mut *self.value.get() }
    }
}
