//! Dashboard thumbnails.

use crate::*;

pub(crate) struct Thumb {
    pub(crate) path: String,
    pub(crate) name: String,
    pub(crate) size: u64, // file size in bytes
    pub(crate) ready: bool, // false while the thumbnail is still being built on a worker thread (px is a blank)
    pub(crate) px: Vec<u32>, // opaque BGRA, tw*th
}

/// Mean of the source pixels in [x0,x1) x [y0,y1) (premultiplied, so averaging is correct).
pub(crate) fn box_avg(f: &[u32], gw: usize, gh: usize, x0: f32, y0: f32, x1: f32, y1: f32) -> u32 {
    let (xs, ys) = ((x0 as usize).min(gw - 1), (y0 as usize).min(gh - 1));
    let (xe, ye) = ((x1.ceil() as usize).clamp(xs + 1, gw), (y1.ceil() as usize).clamp(ys + 1, gh));
    let (mut acc, mut n) = ([0u32; 4], 0u32);
    for y in ys..ye {
        for x in xs..xe {
            for (i, c) in acc.iter_mut().enumerate() {
                *c += (f[y * gw + x] >> (i * 8)) & 0xFF;
            }
            n += 1;
        }
    }
    acc.iter().enumerate().fold(0, |o, (i, c)| o | ((c + n / 2) / n) << (i * 8))
}

pub(crate) fn make_thumb(path: &str, tw: usize, th: usize) -> Option<Thumb> {
    let (gw, gh, frames, _) = decode(path, 1).ok()?;
    let s = (tw as f32 / gw as f32).min(th as f32 / gh as f32).min(2.0);
    let (dw, dh) = (((gw as f32 * s) as usize).clamp(1, tw), ((gh as f32 * s) as usize).clamp(1, th));
    let (ox, oy) = ((tw - dw) / 2, (th - dh) / 2);
    // blend onto the list's own background so transparent GIFs look right
    let c = unsafe { GetSysColor(COLOR_WINDOW) };
    let bg = (c & 0xFF) << 16 | ((c >> 8) & 0xFF) << 8 | (c >> 16) & 0xFF;
    let mut px = vec![0xFF00_0000 | bg; tw * th];
    for y in 0..dh {
        for x in 0..dw {
            // averaging the covered source area avoids the jagged edges bilinear gives when shrinking
            let p = if s < 1.0 {
                box_avg(&frames[0], gw, gh, x as f32 / s, y as f32 / s, (x + 1) as f32 / s, (y + 1) as f32 / s)
            } else {
                sample(&frames[0], gw, gh, (x as f32 + 0.5) / s - 0.5, (y as f32 + 0.5) / s - 0.5)
            };
            let a = p >> 24;
            let mut o = 0xFF00_0000;
            for sh in [0, 8, 16] {
                o |= (((p >> sh) & 0xFF) + ((bg >> sh) & 0xFF) * (255 - a) / 255).min(255) << sh;
            }
            px[(oy + y) * tw + ox + x] = o;
        }
    }
    Some(Thumb {
        path: path.to_string(),
        name: Path::new(path).file_name()?.to_string_lossy().into_owned(),
        size: fs::metadata(path).map_or(0, |m| m.len()),
        ready: true,
        px,
    })
}

/// A blank thumbnail shown at once, so the dashboard opens without decoding any image.
pub(crate) fn placeholder_thumb(path: &str, tw: usize, th: usize) -> Option<Thumb> {
    let c = unsafe { GetSysColor(COLOR_WINDOW) };
    let bg = (c & 0xFF) << 16 | ((c >> 8) & 0xFF) << 8 | (c >> 16) & 0xFF;
    Some(Thumb {
        path: path.to_string(),
        name: Path::new(path).file_name()?.to_string_lossy().into_owned(),
        size: fs::metadata(path).map_or(0, |m| m.len()),
        ready: false,
        px: vec![0xFF00_0000 | bg; tw * th],
    })
}

pub(crate) const WM_THUMB: u32 = WM_APP + 5;

/// Builds the thumbnails on one worker thread and posts each to the dashboard as it is done, so a
/// history full of big photos never freezes the window.
pub(crate) fn spawn_thumbs(paths: Vec<String>) {
    let d = dash();
    let (hwnd, tw, th) = (d.hwnd as usize, d.tw, d.th);
    std::thread::spawn(move || {
        use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_MULTITHREADED};
        let com = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.is_ok(); // WIC needs COM on this thread
        for p in paths {
            let thumb = std::panic::catch_unwind(|| make_thumb(&p, tw, th)).ok().flatten();
            let msg = Box::into_raw(Box::new((p, thumb)));
            if unsafe { PostMessageW(hwnd as HWND, WM_THUMB, 0, msg as isize) } == 0 {
                drop(unsafe { Box::from_raw(msg) }); // dashboard already gone: the app is quitting
                break;
            }
        }
        if com {
            unsafe { CoUninitialize() };
        }
    });
}

/// Mixes `src` over the opaque pixel `dst`; `a` is the coverage 0..1.
pub(crate) fn mix(dst: u32, src: u32, a: f32) -> u32 {
    (0..3).fold(0xFF00_0000, |o, i| {
        let (d, s) = (((dst >> (i * 8)) & 0xFF) as f32, ((src >> (i * 8)) & 0xFF) as f32);
        o | ((d + (s - d) * a + 0.5) as u32) << (i * 8)
    })
}

/// Small green dot with a thin white ring in the top-right corner of a thumbnail.
/// Edges are anti-aliased from the distance to the circle, so they stay smooth at any DPI.
pub(crate) fn draw_open_badge(px: &mut [u32], tw: usize, scale: f32) {
    let (r, ring) = (5.0 * scale, 1.5 * scale);
    let (cx, cy) = (tw as f32 - r - ring - 6.0 * scale, r + ring + 6.0 * scale);
    let th = px.len() / tw;
    for y in 0..th {
        for x in 0..tw {
            let dist = ((x as f32 + 0.5 - cx).powi(2) + (y as f32 + 0.5 - cy).powi(2)).sqrt();
            let outer = (r + ring + 0.5 - dist).clamp(0.0, 1.0);
            if outer > 0.0 {
                let p = &mut px[y * tw + x];
                *p = mix(*p, 0xFFFF_FFFF, outer);
                *p = mix(*p, 0xFF22_C55E, (r + 0.5 - dist).clamp(0.0, 1.0));
            }
        }
    }
}

