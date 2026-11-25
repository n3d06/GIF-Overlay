//! Image decoding is done by gifoverlay_codec.dll, which sits next to the exe and is loaded the
//! first time an image is opened (full path, so nothing is picked up from the current folder).
//! The DLL allocates and frees its own memory; this side copies the frames out one at a time.

use std::{ffi::c_void, sync::OnceLock};
use windows_sys::Win32::{
    Foundation::{HMODULE, MAX_PATH},
    System::LibraryLoader::{GetModuleFileNameW, GetProcAddress, LoadLibraryW},
};

pub type Decoded = (usize, usize, Vec<Vec<u32>>, Vec<u32>);

const DLL: &str = "gifoverlay_codec.dll";

struct Codec {
    decode: unsafe extern "C" fn(*const u16, usize, usize) -> *mut c_void,
    ok: unsafe extern "C" fn(*mut c_void) -> i32,
    error: unsafe extern "C" fn(*mut c_void, *mut usize) -> *const u8,
    info: unsafe extern "C" fn(*mut c_void, *mut usize, *mut usize, *mut usize),
    frame: unsafe extern "C" fn(*mut c_void, usize) -> *const u32,
    delay: unsafe extern "C" fn(*mut c_void, usize) -> u32,
    release: unsafe extern "C" fn(*mut c_void, usize),
    free: unsafe extern "C" fn(*mut c_void),
}

fn load() -> Result<Codec, String> {
    unsafe {
        let mut buf = [0u16; MAX_PATH as usize * 2];
        let n = GetModuleFileNameW(std::ptr::null_mut(), buf.as_mut_ptr(), buf.len() as u32) as usize;
        let exe = String::from_utf16_lossy(&buf[..n]);
        let dir = exe.rsplit_once('\\').map_or("", |(d, _)| d);
        let full: Vec<u16> = format!("{dir}\\{DLL}").encode_utf16().chain([0]).collect();
        let lib: HMODULE = LoadLibraryW(full.as_ptr());
        if lib.is_null() {
            return Err(format!("{DLL} not found next to the exe"));
        }
        macro_rules! sym {
            ($name:literal) => {
                std::mem::transmute(GetProcAddress(lib, concat!($name, "\0").as_ptr()).ok_or(concat!($name, " missing in the codec DLL"))?)
            };
        }
        let version: unsafe extern "C" fn() -> *const u8 = sym!("go_version");
        let theirs = std::ffi::CStr::from_ptr(version() as *const _).to_string_lossy();
        if theirs != env!("CARGO_PKG_VERSION") {
            return Err(format!("{DLL} is version {theirs}, the exe is {}: reinstall so both match", env!("CARGO_PKG_VERSION")));
        }
        Ok(Codec {
            decode: sym!("go_decode"),
            ok: sym!("go_ok"),
            error: sym!("go_error"),
            info: sym!("go_info"),
            frame: sym!("go_frame"),
            delay: sym!("go_delay"),
            release: sym!("go_release_frame"),
            free: sym!("go_free"),
        })
    }
}

fn codec() -> Result<&'static Codec, String> {
    static CODEC: OnceLock<Result<Codec, String>> = OnceLock::new();
    CODEC.get_or_init(load).as_ref().map_err(|e| e.clone())
}

/// Decodes up to `max` frames into full-canvas premultiplied BGRA (formats: see the DLL).
pub fn decode(path: &str, max: usize) -> Result<Decoded, String> {
    let c = codec()?;
    let wpath: Vec<u16> = path.encode_utf16().collect();
    unsafe {
        let h = (c.decode)(wpath.as_ptr(), wpath.len(), max);
        let out = if (c.ok)(h) == 0 {
            let mut len = 0;
            let p = (c.error)(h, &mut len);
            Err(String::from_utf8_lossy(std::slice::from_raw_parts(p, len)).into_owned())
        } else {
            let (mut w, mut hh, mut n) = (0, 0, 0);
            (c.info)(h, &mut w, &mut hh, &mut n);
            let (mut frames, mut delays) = (Vec::with_capacity(n), Vec::with_capacity(n));
            for i in 0..n {
                frames.push(std::slice::from_raw_parts((c.frame)(h, i), w * hh).to_vec());
                (c.release)(h, i);
                delays.push((c.delay)(h, i));
            }
            Ok((w, hh, frames, delays))
        };
        (c.free)(h);
        out
    }
}
