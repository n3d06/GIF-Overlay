//! Software rendering of an overlay: scaling, flips, rotation, the progress bar.

use crate::*;

pub(crate) fn sample(f: &[u32], gw: usize, gh: usize, x: f32, y: f32) -> u32 {
    let x = x.clamp(0.0, (gw - 1) as f32);
    let y = y.clamp(0.0, (gh - 1) as f32);
    let (x0, y0) = (x as usize, y as usize);
    let (x1, y1) = ((x0 + 1).min(gw - 1), (y0 + 1).min(gh - 1));
    let (fx, fy) = (x - x0 as f32, y - y0 as f32);
    let (p00, p10, p01, p11) = (f[y0 * gw + x0], f[y0 * gw + x1], f[y1 * gw + x0], f[y1 * gw + x1]);
    let mut out = 0u32;
    for s in [0, 8, 16, 24] {
        let c = |p: u32| ((p >> s) & 0xFF) as f32;
        let top = c(p00) + (c(p10) - c(p00)) * fx;
        let bot = c(p01) + (c(p11) - c(p01)) * fx;
        out |= ((top + (bot - top) * fy + 0.5) as u32) << s;
    }
    out
}

/// Per display column/row: the two source indices to blend and the 8-bit weight of the second.
pub(crate) fn axis_table(n: usize, size: usize, flip: bool, invert: bool) -> Vec<(usize, usize, u32)> {
    (0..n)
        .map(|i| {
            let mut u = (i as f32 + 0.5) / n as f32;
            if flip != invert {
                u = 1.0 - u; // flip and rotation inversion cancel out
            }
            let c = (u * size as f32 - 0.5).clamp(0.0, (size - 1) as f32);
            let i0 = c as usize;
            (i0, (i0 + 1).min(size - 1), ((c - i0 as f32) * 256.0).round() as u32)
        })
        .collect()
}

/// Blends two premultiplied BGRA pixels, `t` in 0..=256 is the weight of `b`
/// (two channels per multiply: exact for t = 0 and for equal pixels).
pub(crate) fn lerp(a: u32, b: u32, t: u32) -> u32 {
    const M: u32 = 0x00FF_00FF;
    let rb = ((a & M) * (256 - t) + (b & M) * t + 0x0080_0080) >> 8 & M; // +128 per lane rounds
    let ag = (((a >> 8) & M) * (256 - t) + ((b >> 8) & M) * t + 0x0080_0080) & !M;
    rb | ag
}

/// Bilinear upscale with flips and rotation, in integer math. Because flips and quarter turns
/// keep rows and columns independent, the source positions are computed once per row / column.
pub(crate) fn upscale_into(
    dst: &mut [u32], (w, h): (usize, usize), f: &[u32], (gw, gh): (usize, usize),
    rot: u8, (flip_h, flip_v): (bool, bool),
) {
    let swap = rot % 2 == 1; // display columns then walk the source rows and vice versa
    let tx = axis_table(w, if swap { gh } else { gw }, flip_h, rot == 1 || rot == 2);
    let ty = axis_table(h, if swap { gw } else { gh }, flip_v, rot == 2 || rot == 3);
    for (y, row) in dst.chunks_exact_mut(w).enumerate() {
        let (a0, a1, fa) = ty[y];
        for (px, &(b0, b1, fb)) in row.iter_mut().zip(&tx) {
            let (x0, x1, wx, y0, y1, wy) = if swap { (a0, a1, fa, b0, b1, fb) } else { (b0, b1, fb, a0, a1, fa) };
            let top = lerp(f[y0 * gw + x0], f[y0 * gw + x1], wx);
            let bot = lerp(f[y1 * gw + x0], f[y1 * gw + x1], wx);
            *px = lerp(top, bot, wy);
        }
    }
}

pub(crate) fn render(a: &App) {
    let (w, h) = (a.w as usize, a.h as usize);
    unsafe {
        let screen = GetDC(null_mut());
        let mem_dc = CreateCompatibleDC(screen);
        let mut bi: BITMAPINFO = mem::zeroed();
        bi.bmiHeader.biSize = mem::size_of::<BITMAPINFOHEADER>() as u32;
        bi.bmiHeader.biWidth = w as i32;
        bi.bmiHeader.biHeight = -(h as i32);
        bi.bmiHeader.biPlanes = 1;
        bi.bmiHeader.biBitCount = 32;
        bi.bmiHeader.biCompression = BI_RGB;
        let mut bits: *mut c_void = null_mut();
        let bmp = CreateDIBSection(mem_dc, &bi, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
        if !bits.is_null() {
            let old = SelectObject(mem_dc, bmp);
            let dst = slice::from_raw_parts_mut(bits as *mut u32, w * h);
            let f = &a.frames[a.cur];
            // display (u,v) in 0..1 -> source (u,v), undoing flips and rotation
            let tf = |mut u: f32, mut v: f32| {
                if a.flip_h { u = 1.0 - u }
                if a.flip_v { v = 1.0 - v }
                match a.rot {
                    0 => (u, v),
                    1 => (v, 1.0 - u),
                    2 => (1.0 - u, 1.0 - v),
                    _ => (1.0 - v, u),
                }
            };
            let (gwf, ghf) = (a.gw as f32, a.gh as f32);
            let (nw, nh) = natural_size(a);
            let scale = (w as f64 / nw).min(h as f64 / nh);
            // shrinking: average the covered source area (bilinear alone gives jagged edges)
            let shrink = scale < 1.0;
            if shrink {
                for y in 0..h {
                    for x in 0..w {
                        let (u0, v0) = tf(x as f32 / w as f32, y as f32 / h as f32);
                        let (u1, v1) = tf((x + 1) as f32 / w as f32, (y + 1) as f32 / h as f32);
                        dst[y * w + x] = box_avg(f, a.gw, a.gh, u0.min(u1) * gwf, v0.min(v1) * ghf, u0.max(u1) * gwf, v0.max(v1) * ghf);
                    }
                }
            } else {
                if (w, h) == (a.gw, a.gh) && a.rot == 0 && !a.flip_h && !a.flip_v {
                    dst.copy_from_slice(f); // shown 1:1: nothing to resample
                } else {
                    upscale_into(dst, (w, h), f, (a.gw, a.gh), a.rot, (a.flip_h, a.flip_v));
                }
            }
            // Fully transparent pixels let the mouse fall through the window, so a PNG with a
            // transparent background could not be grabbed there. Alpha 1 is invisible but hit-testable.
            for p in dst.iter_mut() {
                if *p >> 24 == 0 {
                    *p = 0x0100_0000;
                }
            }
            // resize handles, shown while hovering: edge bars + a striped corner triangle,
            // the one under the cursor is highlighted
            if a.hover && !a.locked && a.restore.is_none() { // no resizing while maximized
                let (g, t) = (grip(a) as usize, (edge(a) / 2).max(3) as usize);
                let color = |z: Zone, normal: u32| if a.hot == z { 0xB000_5394 } else { normal };
                for y in 0..h - g {
                    for x in w - t..w {
                        dst[y * w + x] = color(Zone::Right, 0x6000_0000);
                    }
                }
                for y in h - t..h {
                    for x in 0..w - g {
                        dst[y * w + x] = color(Zone::Bottom, 0x6000_0000);
                    }
                }
                // left and top edges only complete the frame so the four sides look alike
                for y in 0..h {
                    dst[y * w..y * w + t].fill(0x6000_0000);
                }
                dst[..t * w].fill(0x6000_0000);
                for dy in 0..g {
                    for dx in 0..g - dy {
                        let d = dx + dy;
                        dst[(h - 1 - dy) * w + (w - 1 - dx)] =
                            if d < g - 3 && matches!(d % 6, 2 | 3) { 0xC0C0_C0C0 } else { color(Zone::Corner, 0x7000_0000) };
                    }
                }
            }
            if a.hover && a.frames.len() > 1 {
                draw_progress(a, dst);
            }
            // top-right buttons (hidden when locked): minimize | maximize | close, each a dark square
            // with a white icon; the one under the cursor is highlighted (red for close)
            if a.hover && !a.locked {
                let b = btn(a) as usize;
                let m = b / 4;
                let bg = |z: Zone, hot: u32| if a.hot == z { hot } else { 0x7000_0000 };
                let mut rect = |x0: usize, y0: usize, x1: usize, y1: usize, c: u32| {
                    for y in y0..y1 {
                        dst[y * w + x0..y * w + x1].fill(c);
                    }
                };
                const WHITE: u32 = 0xFFFF_FFFF;
                // close: X
                rect(w - b, 0, w, b, bg(Zone::Close, 0xE0C4_2B1B));
                for i in m..b - m {
                    rect(w - b + i, i, w - b + i + 2, i + 1, WHITE);
                    rect(w - 2 - i, i, w - i, i + 1, WHITE);
                }
                // maximize: square outline
                let x0 = w - 2 * b;
                rect(x0, 0, x0 + b, b, bg(Zone::Max, 0xC050_5050));
                rect(x0 + m, m, x0 + b - m, m + 2, WHITE);
                rect(x0 + m, b - m - 2, x0 + b - m, b - m, WHITE);
                rect(x0 + m, m, x0 + m + 2, b - m, WHITE);
                rect(x0 + b - m - 2, m, x0 + b - m, b - m, WHITE);
                // minimize: bar at the bottom
                let x0 = w - 3 * b;
                rect(x0, 0, x0 + b, b, bg(Zone::Min, 0xC050_5050));
                rect(x0 + m, b - m - 2, x0 + b - m, b - m, WHITE);
            }
            let size = SIZE { cx: w as i32, cy: h as i32 };
            let src = POINT { x: 0, y: 0 };
            let bf = BLENDFUNCTION {
                BlendOp: AC_SRC_OVER as u8,
                BlendFlags: 0,
                SourceConstantAlpha: (a.opacity * 255 / 100) as u8,
                AlphaFormat: AC_SRC_ALPHA as u8,
            };
            UpdateLayeredWindow(a.hwnd, screen, null(), &size, mem_dc, &src, 0, &bf, ULW_ALPHA);
            SelectObject(mem_dc, old);
        }
        DeleteObject(bmp);
        DeleteDC(mem_dc);
        ReleaseDC(null_mut(), screen);
    }
}

/// 3x5 pixel glyphs (one row per entry, 3 bits each) for the frame counter.
pub(crate) fn glyph(c: char) -> [u8; 5] {
    match c {
        '0' => [7, 5, 5, 5, 7],
        '1' => [2, 6, 2, 2, 7],
        '2' => [7, 1, 7, 4, 7],
        '3' => [7, 1, 7, 1, 7],
        '4' => [5, 5, 7, 1, 1],
        '5' => [7, 4, 7, 1, 7],
        '6' => [7, 4, 7, 5, 7],
        '7' => [7, 1, 1, 1, 1],
        '8' => [7, 5, 7, 5, 7],
        '9' => [7, 5, 7, 1, 7],
        '/' => [1, 1, 2, 4, 4],
        '.' => [0, 0, 0, 0, 2],
        'x' => [0, 5, 2, 5, 0],
        _ => [0; 5],
    }
}

/// Frame counter ("3/12", plus the speed when it is not 1x) and a progress bar along the top edge.
pub(crate) fn draw_progress(a: &App, dst: &mut [u32]) {
    let (w, h) = (a.w as usize, a.h as usize);
    let k = dpi_px(a, 2).max(2) as usize; // size of one font pixel
    let t = (edge(a) / 2).max(3) as usize;
    let n = a.frames.len();
    let mut text = format!("{}/{}", a.cur + 1, n);
    if a.speed != SPEED_NORMAL {
        text += &format!(" {}x", SPEEDS[a.speed]);
    }
    let (tw, th) = (text.chars().count() * 4 * k - k, 5 * k);
    if w < t + tw + 6 * k + 3 * btn(a) as usize || h < t + 2 * k + th + 4 * k {
        return; // too small to show anything without covering the buttons
    }
    let bar_w = w - 2 * t;
    for y in t..t + k {
        for x in 0..bar_w {
            dst[y * w + t + x] = if x < bar_w * (a.cur + 1) / n { 0xFF3B_82F6 } else { 0x8080_8080 };
        }
    }
    let (x0, y0) = (t + 2 * k, t + 3 * k);
    for y in y0 - k..y0 + th + k {
        dst[y * w + x0 - 2 * k..y * w + x0 + tw + 2 * k].fill(0xB000_0000); // dark chip
    }
    for (i, c) in text.chars().enumerate() {
        for (row, bits) in glyph(c).iter().enumerate() {
            for col in 0..3 {
                if bits >> (2 - col) & 1 != 0 {
                    for dy in 0..k {
                        let start = (y0 + row * k + dy) * w + x0 + (i * 4 + col) * k;
                        dst[start..start + k].fill(0xFFFF_FFFF);
                    }
                }
            }
        }
    }
}

