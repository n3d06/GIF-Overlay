//! gifoverlay_codec.dll: the image decoders, kept out of the exe. The exe loads it at runtime
//! (see src/decode.rs) and talks to it through a small C ABI. Everything the DLL allocates is freed
//! by the DLL (`go_free`), and the exe copies the pixels out frame by frame.

mod decode;

pub use decode::{decode, Decoded};

use std::{ffi::c_void, panic::catch_unwind, slice};

type Res = Result<Decoded, String>;

/// Decodes up to `max` frames of the file (UTF-16, `len` units). Returns a handle for the other
/// calls; never null, a failure is a handle whose `go_error` is set.
#[no_mangle]
pub unsafe extern "C" fn go_decode(path: *const u16, len: usize, max: usize) -> *mut c_void {
    let path = String::from_utf16_lossy(slice::from_raw_parts(path, len));
    let res: Res = catch_unwind(|| decode(&path, max)).unwrap_or_else(|_| Err("the decoder crashed on this file".into()));
    Box::into_raw(Box::new(res)) as *mut c_void
}

unsafe fn res<'a>(h: *mut c_void) -> &'a mut Res {
    &mut *(h as *mut Res)
}

/// The DLL's version (NUL-terminated), so the exe can refuse a DLL from another release.
#[no_mangle]
pub extern "C" fn go_version() -> *const u8 {
    concat!(env!("CARGO_PKG_VERSION"), "\0").as_ptr()
}

/// 1 when the decode worked.
#[no_mangle]
pub unsafe extern "C" fn go_ok(h: *mut c_void) -> i32 {
    res(h).is_ok() as i32
}

/// The error text (UTF-8, valid until `go_free`).
#[no_mangle]
pub unsafe extern "C" fn go_error(h: *mut c_void, len: *mut usize) -> *const u8 {
    match res(h) {
        Err(e) => {
            *len = e.len();
            e.as_ptr()
        }
        Ok(_) => {
            *len = 0;
            std::ptr::null()
        }
    }
}

/// Canvas size and frame count of a successful decode.
#[no_mangle]
pub unsafe extern "C" fn go_info(h: *mut c_void, w: *mut usize, hh: *mut usize, frames: *mut usize) {
    if let Ok((gw, gh, f, _)) = res(h) {
        (*w, *hh, *frames) = (*gw, *gh, f.len());
    }
}

/// Frame `i` as premultiplied 0xAARRGGBB (w*h pixels).
#[no_mangle]
pub unsafe extern "C" fn go_frame(h: *mut c_void, i: usize) -> *const u32 {
    res(h).as_ref().ok().and_then(|d| d.2.get(i)).map_or(std::ptr::null(), |f| f.as_ptr())
}

#[no_mangle]
pub unsafe extern "C" fn go_delay(h: *mut c_void, i: usize) -> u32 {
    res(h).as_ref().ok().and_then(|d| d.3.get(i).copied()).unwrap_or(100)
}

/// Drops frame `i` early, so the exe's copy and the DLL's copy never both hold the whole animation.
#[no_mangle]
pub unsafe extern "C" fn go_release_frame(h: *mut c_void, i: usize) {
    if let Ok(d) = res(h) {
        if let Some(f) = d.2.get_mut(i) {
            *f = Vec::new();
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn go_free(h: *mut c_void) {
    if !h.is_null() {
        drop(Box::from_raw(h as *mut Res));
    }
}
