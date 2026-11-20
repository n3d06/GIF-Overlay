//! Image decoding. GIF, APNG and animated WebP are sniffed by content and decoded by pure-Rust
//! crates; everything else goes through the Windows Imaging Component.

use std::{fs, ptr::null, slice};

const GIF_BUDGET: usize = 512 << 20; // bytes of decoded frames kept per image

pub type Decoded = (usize, usize, Vec<Vec<u32>>, Vec<u32>);

/// Animated formats are sniffed by content (GIF / APNG / animated WebP) and decoded by pure-Rust
/// crates; everything else, and any of those that turn out to be a still image, goes through the
/// Windows Imaging Component (PNG, JPEG, BMP, TIFF, WebP, AVIF if the codec is installed...).
pub fn decode(path: &str, max: usize) -> Result<Decoded, String> {
    let mut head = [0u8; 12];
    let _ = fs::File::open(path).and_then(|mut f| std::io::Read::read(&mut f, &mut head));
    let anim = if head.starts_with(b"GIF8") {
        decode_gif(path, max)
    } else if head.starts_with(b"\x89PNG") {
        decode_apng(path, max)
    } else if &head[..4] == b"RIFF" && &head[8..] == b"WEBP" {
        decode_webp(path, max)
    } else {
        Err(String::new())
    };
    anim.or_else(|_| decode_wic(path))
}

/// How many full-canvas frames fit the memory budget (running out of RAM aborts the app).
fn frame_cap(w: usize, h: usize, max: usize) -> Result<usize, String> {
    if w == 0 || h == 0 {
        return Err("empty image".into());
    }
    let bytes = w.checked_mul(h).and_then(|n| n.checked_mul(4)).filter(|&b| b <= GIF_BUDGET);
    Ok(max.min(GIF_BUDGET / bytes.ok_or("image too large")?)) // longer animations are cut to the frames that fit
}

/// Straight RGBA to the premultiplied 0xAARRGGBB the renderer expects.
fn premul(r: u8, g: u8, b: u8, a: u8) -> u32 {
    let m = |c: u8| (c as u32 * a as u32 + 127) / 255;
    (a as u32) << 24 | m(r) << 16 | m(g) << 8 | m(b)
}

/// Animated WebP (frames come out already composited). A still WebP returns Err and goes to WIC.
fn decode_webp(path: &str, max: usize) -> Result<Decoded, String> {
    let e = |e: image_webp::DecodingError| e.to_string();
    let file = fs::File::open(path).map_err(|e| e.to_string())?;
    let mut dec = image_webp::WebPDecoder::new(std::io::BufReader::new(file)).map_err(e)?;
    if !dec.is_animated() {
        return Err("still image".into());
    }
    let (w, h) = dec.dimensions();
    let (w, h) = (w as usize, h as usize);
    let max = frame_cap(w, h, max)?;
    let ch = if dec.has_alpha() { 4 } else { 3 };
    let mut buf = vec![0u8; dec.output_buffer_size().ok_or("image too large")?];
    let (mut frames, mut delays) = (Vec::new(), Vec::new());
    while frames.len() < max {
        match dec.read_frame(&mut buf) {
            Ok(ms) => {
                frames.push(buf.chunks_exact(ch).map(|p| premul(p[0], p[1], p[2], if ch == 4 { p[3] } else { 255 })).collect());
                delays.push(if ms <= 10 { 100 } else { ms }); // browsers treat tiny delays as 100ms
            }
            Err(_) if !frames.is_empty() => break, // keep what decoded before a corrupt frame
            Err(err) => return Err(e(err)),
        }
    }
    Ok((w, h, frames, delays))
}

/// APNG: composites each frame (dispose / blend ops) onto a full canvas. A plain PNG returns Err.
fn decode_apng(path: &str, max: usize) -> Result<Decoded, String> {
    use png::{BlendOp, ColorType, DisposeOp};
    let e = |e: png::DecodingError| e.to_string();
    let file = fs::File::open(path).map_err(|e| e.to_string())?;
    let mut dec = png::Decoder::new(std::io::BufReader::new(file));
    dec.set_transformations(png::Transformations::normalize_to_color8());
    let mut r = dec.read_info().map_err(e)?;
    let n = r.info().animation_control().ok_or("still image")?.num_frames as usize;
    let (gw, gh) = (r.info().width as usize, r.info().height as usize);
    let max = frame_cap(gw, gh, max)?.min(n);
    let mut buf = vec![0u8; gw * gh * 4];
    if r.info().frame_control().is_none() {
        r.next_frame(&mut buf).map_err(e)?; // the default image is not part of the animation
    }
    let mut canvas = vec![0u32; gw * gh];
    let (mut frames, mut delays) = (Vec::new(), Vec::new());
    while frames.len() < max {
        let out = match r.next_frame(&mut buf) {
            Ok(o) => o,
            Err(_) if !frames.is_empty() => break,
            Err(err) => return Err(e(err)),
        };
        let Some(fc) = r.info().frame_control().cloned() else { break };
        let (l, t, w, h) = (fc.x_offset as usize, fc.y_offset as usize, fc.width as usize, fc.height as usize);
        if l + w > gw || t + h > gh {
            break; // invalid frame rectangle
        }
        let saved = (fc.dispose_op == DisposeOp::Previous).then(|| canvas.clone());
        let bpp = out.color_type.samples();
        for y in 0..h {
            for x in 0..w {
                let p = &buf[(y * w + x) * bpp..][..bpp];
                let (rgb, a) = match out.color_type {
                    ColorType::Grayscale => ([p[0]; 3], 255),
                    ColorType::GrayscaleAlpha => ([p[0]; 3], p[1]),
                    ColorType::Rgb => ([p[0], p[1], p[2]], 255),
                    _ => ([p[0], p[1], p[2]], p[3]),
                };
                let src = premul(rgb[0], rgb[1], rgb[2], a);
                let dst = &mut canvas[(t + y) * gw + l + x];
                *dst = if fc.blend_op == BlendOp::Source || a == 255 {
                    src
                } else {
                    // premultiplied "over": src + dst * (1 - src alpha)
                    let k = 255 - a as u32;
                    (0..4).fold(0, |o, i| o | (((src >> (i * 8)) & 0xFF) + ((*dst >> (i * 8)) & 0xFF) * k / 255).min(255) << (i * 8))
                };
            }
        }
        frames.push(canvas.clone());
        let den = if fc.delay_den == 0 { 100 } else { fc.delay_den as u32 };
        let ms = fc.delay_num as u32 * 1000 / den;
        delays.push(if ms <= 10 { 100 } else { ms });
        match fc.dispose_op {
            DisposeOp::Background => {
                for y in t..t + h {
                    canvas[y * gw + l..y * gw + l + w].fill(0);
                }
            }
            DisposeOp::Previous => canvas = saved.unwrap(),
            DisposeOp::None => {}
        }
    }
    if frames.is_empty() {
        return Err("no frames".into());
    }
    Ok((gw, gh, frames, delays))
}

/// First frame as premultiplied BGRA, which is exactly what WIC's 32bppPBGRA gives.
fn decode_wic(path: &str) -> Result<Decoded, String> {
    use windows::{
        core::HSTRING,
        Win32::{
            Foundation::GENERIC_READ,
            Graphics::Imaging::*,
            System::Com::{CoCreateInstance, CLSCTX_INPROC_SERVER},
        },
    };
    let e = |e: windows::core::Error| e.message();
    unsafe {
        let factory: IWICImagingFactory = CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER).map_err(e)?;
        let decoder = factory
            .CreateDecoderFromFilename(&HSTRING::from(path), None, GENERIC_READ, WICDecodeMetadataCacheOnDemand)
            .map_err(e)?;
        let frame = decoder.GetFrame(0).map_err(e)?;
        // EXIF orientation (phone photos): WIC applies flip first, then rotate
        let opts = exif_orientation(&frame).map_or(0, |o| match o {
            2 => 8,
            3 => 2,
            4 => 16,
            5 => 3 | 8,
            6 => 1,
            7 => 1 | 8,
            8 => 3,
            _ => 0,
        });
        let rotator = factory.CreateBitmapFlipRotator().map_err(e)?;
        rotator.Initialize(&frame, WICBitmapTransformOptions(opts)).map_err(e)?;
        let conv = factory.CreateFormatConverter().map_err(e)?;
        conv.Initialize(&rotator, &GUID_WICPixelFormat32bppPBGRA, WICBitmapDitherTypeNone, None, 0.0, WICBitmapPaletteTypeCustom)
            .map_err(e)?;
        let (mut w, mut h) = (0u32, 0u32);
        conv.GetSize(&mut w, &mut h).map_err(e)?;
        if w == 0 || h == 0 {
            return Err("empty image".into());
        }
        if w as usize * h as usize * 4 > GIF_BUDGET {
            return Err("image too large".into()); // a failed allocation would abort the whole app
        }
        let mut px = vec![0u32; w as usize * h as usize];
        conv.CopyPixels(null(), w * 4, slice::from_raw_parts_mut(px.as_mut_ptr() as *mut u8, px.len() * 4))
            .map_err(e)?;
        Ok((w as usize, h as usize, vec![px], vec![100]))
    }
}

fn exif_orientation(frame: &windows::Win32::Graphics::Imaging::IWICBitmapFrameDecode) -> Option<u16> {
    use windows::core::w;
    let mut pv = windows::Win32::System::Com::StructuredStorage::PROPVARIANT::default();
    unsafe { frame.GetMetadataQueryReader().ok()?.GetMetadataByName(w!("/app1/ifd/{ushort=274}"), &mut pv).ok()? };
    u16::try_from(&pv).ok()
}

/// Decodes up to `max` frames into full-canvas premultiplied BGRA.
fn decode_gif(path: &str, max: usize) -> Result<Decoded, String> {
    let file = fs::File::open(path).map_err(|e| e.to_string())?;
    let mut opts = gif::DecodeOptions::new();
    opts.set_color_output(gif::ColorOutput::RGBA);
    let mut dec = opts.read_info(file).map_err(|e| e.to_string())?;
    let (gw, gh) = (dec.width() as usize, dec.height() as usize);
    let max = frame_cap(gw, gh, max)?;
    let mut canvas = vec![0u32; gw * gh];
    let (mut frames, mut delays) = (Vec::new(), Vec::new());
    while frames.len() < max {
        // a corrupt frame mid-file keeps the frames decoded so far instead of losing the animation
        let f = match dec.read_next_frame() {
            Ok(Some(f)) => f,
            Ok(None) => break,
            Err(e) if frames.is_empty() => return Err(e.to_string()),
            Err(_) => break,
        };
        let saved = (f.dispose == gif::DisposalMethod::Previous).then(|| canvas.clone());
        let (l, t, w, h) = (f.left as usize, f.top as usize, f.width as usize, f.height as usize);
        for y in 0..h {
            for x in 0..w {
                let (cx, cy) = (l + x, t + y);
                let p = &f.buffer[(y * w + x) * 4..][..4];
                // GIF alpha is binary: transparent pixels leave the canvas untouched
                if cx < gw && cy < gh && p[3] != 0 {
                    canvas[cy * gw + cx] = 0xFF00_0000 | (p[0] as u32) << 16 | (p[1] as u32) << 8 | p[2] as u32;
                }
            }
        }
        frames.push(canvas.clone());
        delays.push(if f.delay <= 1 { 100 } else { f.delay as u32 * 10 }); // browsers treat 0/1 as 100ms
        match f.dispose {
            gif::DisposalMethod::Background => {
                for y in t..(t + h).min(gh) {
                    for x in l..(l + w).min(gw) {
                        canvas[y * gw + x] = 0;
                    }
                }
            }
            gif::DisposalMethod::Previous => canvas = saved.unwrap(),
            _ => {}
        }
    }
    if frames.is_empty() {
        return Err("no frames".into());
    }
    Ok((gw, gh, frames, delays))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two-frame APNG: red 2x2 canvas, then a half-transparent blue pixel blended over (0,0).
    #[test]
    fn apng_composites_frames() {
        let path = std::env::temp_dir().join("gif_overlay_test.apng");
        {
            let mut enc = png::Encoder::new(fs::File::create(&path).unwrap(), 2, 2);
            enc.set_color(png::ColorType::Rgba);
            enc.set_depth(png::BitDepth::Eight);
            enc.set_animated(2, 0).unwrap();
            let mut w = enc.write_header().unwrap();
            w.set_frame_delay(5, 100).unwrap(); // 50ms
            w.write_image_data(&[255, 0, 0, 255].repeat(4)).unwrap();
            w.set_frame_dimension(1, 1).unwrap();
            w.set_frame_position(0, 0).unwrap();
            w.set_blend_op(png::BlendOp::Over).unwrap();
            w.write_image_data(&[0, 0, 255, 128]).unwrap();
        }
        let (gw, gh, frames, delays) = decode(path.to_str().unwrap(), usize::MAX).unwrap();
        assert_eq!((gw, gh, frames.len(), delays[0]), (2, 2, 2, 50));
        assert_eq!(frames[0][0], 0xFFFF_0000);
        assert_eq!(frames[1][3], 0xFFFF_0000); // untouched pixel
        let p = frames[1][0]; // blue (a=128) over red: opaque, red fades to ~127, blue ~128
        assert_eq!(p >> 24, 255);
        assert!(((p >> 16) & 0xFF).abs_diff(127) <= 1 && (p & 0xFF).abs_diff(128) <= 1, "{p:08x}");
        let _ = fs::remove_file(path);
    }
}
