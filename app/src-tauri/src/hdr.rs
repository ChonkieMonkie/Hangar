// Hangar - a community-made updater and launcher for DCS World.
// Copyright (C) 2026 ChonkieMonkie
// SPDX-License-Identifier: GPL-3.0-or-later
// Free software under the GNU GPL v3 or later, WITHOUT ANY WARRANTY. See LICENSE or <https://www.gnu.org/licenses/>.

//! HDR pictures for the Hangar wall (ROADMAP #16). The app window cannot show HDR, so HDR
//! screenshots are tone-mapped to SDR:
//! - JPEG XR (`.jxr`, Xbox Game Bar / NVIDIA with HDR on) is decoded with Windows Imaging
//!   Component, which ships a JPEG XR codec, into linear scRGB floats (1.0 = 80 nits).
//! - PNG with a `cICP` chunk saying PQ (ST 2084) or HLG is decoded with the image crate and
//!   converted to linear light; BT.2020 primaries are converted to BT.709.
//! The tone curve is extended Reinhard on luminance with the white point at the picture's own
//! 99.5th percentile, so neither dark nor bright pictures clip. Only reads the files.

use std::path::Path;

use image::RgbImage;

/// Linear RGB, Rec.709 primaries, 1.0 = SDR reference white (80 nits).
pub struct Linear {
    pub w: u32,
    pub h: u32,
    pub px: Vec<f32>,
}

pub fn is_jxr(p: &Path) -> bool {
    p.extension().is_some_and(|e| e.eq_ignore_ascii_case("jxr") || e.eq_ignore_ascii_case("wdp"))
}

/// (colour primaries, transfer characteristics) from a PNG's `cICP` chunk.
pub fn png_cicp(path: &Path) -> Option<(u8, u8)> {
    use std::io::Read;
    let mut f = std::fs::File::open(path).ok()?;
    let mut head = vec![0u8; 256 * 1024];
    let n = f.read(&mut head).ok()?;
    let d = &head[..n];
    if d.len() < 8 || &d[..8] != b"\x89PNG\r\n\x1a\n" {
        return None;
    }
    let mut i = 8;
    while i + 8 <= d.len() {
        let len = u32::from_be_bytes([d[i], d[i + 1], d[i + 2], d[i + 3]]) as usize;
        let kind = &d[i + 4..i + 8];
        if kind == b"IDAT" {
            return None;
        }
        if kind == b"cICP" && i + 12 <= d.len() {
            return Some((d[i + 8], d[i + 9]));
        }
        i += 12 + len;
    }
    None
}

/// True if this PNG is HDR (PQ = 16, HLG = 18).
pub fn png_is_hdr(path: &Path) -> bool {
    path.extension().is_some_and(|e| e.eq_ignore_ascii_case("png")) && png_cicp(path).is_some_and(|(_, t)| t == 16 || t == 18)
}

fn pq_to_nits(e: f32) -> f32 {
    const M1: f32 = 2610.0 / 16384.0;
    const M2: f32 = 2523.0 / 4096.0 * 128.0;
    const C1: f32 = 3424.0 / 4096.0;
    const C2: f32 = 2413.0 / 4096.0 * 32.0;
    const C3: f32 = 2392.0 / 4096.0 * 32.0;
    let p = e.clamp(0.0, 1.0).powf(1.0 / M2);
    ((p - C1).max(0.0) / (C2 - C3 * p)).powf(1.0 / M1) * 10_000.0
}

fn hlg_to_nits(e: f32) -> f32 {
    const A: f32 = 0.178_832_77;
    const B: f32 = 1.0 - 4.0 * A;
    let c = 0.5 - A * (4.0 * A).ln();
    let e = e.clamp(0.0, 1.0);
    let scene = if e <= 0.5 { e * e / 3.0 } else { (((e - c) / A).exp() + B) / 12.0 };
    1000.0 * scene.powf(1.2) // simplified OOTF for a 1000-nit display
}

fn bt2020_to_bt709(px: &mut [f32]) {
    for c in px.chunks_exact_mut(3) {
        let (r, g, b) = (c[0], c[1], c[2]);
        c[0] = 1.6605 * r - 0.5876 * g - 0.0728 * b;
        c[1] = -0.1246 * r + 1.1329 * g - 0.0083 * b;
        c[2] = -0.0182 * r - 0.1006 * g + 1.1187 * b;
    }
}

/// Decodes a PQ/HLG PNG, scaled to fit `max_w` x `max_h`.
pub fn load_png_hdr(path: &Path, max_w: u32, max_h: u32) -> Result<Linear, String> {
    let (primaries, transfer) = png_cicp(path).ok_or("Not an HDR PNG")?;
    let img = image::ImageReader::open(path).map_err(|e| e.to_string())?.decode().map_err(|e| format!("Could not read the picture: {e}"))?;
    let img = if img.width() > max_w || img.height() > max_h { img.thumbnail(max_w, max_h) } else { img };
    let rgb = img.to_rgb32f();
    let (w, h) = rgb.dimensions();
    let to_nits = if transfer == 18 { hlg_to_nits } else { pq_to_nits };
    let mut px: Vec<f32> = rgb.into_raw().into_iter().map(|v| to_nits(v) / 80.0).collect();
    if primaries == 9 {
        bt2020_to_bt709(&mut px);
    }
    Ok(Linear { w, h, px })
}

/// Decodes a JPEG XR picture with Windows Imaging Component, scaled to fit `max_w` x `max_h`.
#[cfg(windows)]
pub fn load_jxr(path: &Path, max_w: u32, max_h: u32) -> Result<Linear, String> {
    use windows::Win32::Foundation::GENERIC_READ;
    use windows::Win32::Graphics::Imaging::*;
    use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx};
    use windows::core::HSTRING;
    let err = |e: windows::core::Error| format!("Could not read the HDR picture: {}", e.message());
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED); // fine if COM is already set up
        let f: IWICImagingFactory = CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER).map_err(err)?;
        let dec = f.CreateDecoderFromFilename(&HSTRING::from(path.as_os_str()), None, GENERIC_READ, WICDecodeMetadataCacheOnDemand).map_err(err)?;
        let frame = dec.GetFrame(0).map_err(err)?;
        let (mut w, mut h) = (0u32, 0u32);
        frame.GetSize(&mut w, &mut h).map_err(err)?;
        if w == 0 || h == 0 || (w as u64) * (h as u64) > 200_000_000 {
            return Err("Unsupported picture size".into());
        }
        let scale = (max_w as f64 / w as f64).min(max_h as f64 / h as f64).min(1.0);
        let (sw, sh) = (((w as f64 * scale).round() as u32).max(1), ((h as f64 * scale).round() as u32).max(1));
        let source: IWICBitmapSource = if scale < 1.0 {
            let s = f.CreateBitmapScaler().map_err(err)?;
            s.Initialize(&frame, sw, sh, WICBitmapInterpolationModeFant).map_err(err)?;
            windows::core::Interface::cast(&s).map_err(err)?
        } else {
            windows::core::Interface::cast(&frame).map_err(err)?
        };
        let conv = f.CreateFormatConverter().map_err(err)?;
        conv.Initialize(&source, &GUID_WICPixelFormat128bppRGBAFloat, WICBitmapDitherTypeNone, None, 0.0, WICBitmapPaletteTypeCustom).map_err(err)?;
        let stride = sw * 16;
        let mut buf = vec![0u8; stride as usize * sh as usize];
        conv.CopyPixels(std::ptr::null(), stride, &mut buf).map_err(err)?;
        let mut px = Vec::with_capacity(sw as usize * sh as usize * 3);
        for p in buf.chunks_exact(16) {
            for c in 0..3 {
                let v = f32::from_le_bytes([p[c * 4], p[c * 4 + 1], p[c * 4 + 2], p[c * 4 + 3]]);
                px.push(if v.is_finite() { v } else { 0.0 });
            }
        }
        Ok(Linear { w: sw, h: sh, px })
    }
}

#[cfg(not(windows))]
pub fn load_jxr(_path: &Path, _max_w: u32, _max_h: u32) -> Result<Linear, String> {
    Err("JPEG XR needs Windows".into())
}

fn srgb(v: f32) -> u8 {
    let v = v.clamp(0.0, 1.0);
    let e = if v <= 0.003_130_8 { 12.92 * v } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 };
    (e * 255.0 + 0.5) as u8
}

/// Tone-maps linear light to an 8-bit sRGB picture.
pub fn tonemap(img: &Linear) -> RgbImage {
    let lum = |c: &[f32]| (0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]).max(0.0);
    // white point: 99.5th percentile of luminance (sampled), never below SDR white
    let n = img.px.len() / 3;
    let step = (n / 200_000).max(1);
    let mut sample: Vec<f32> = img.px.chunks_exact(3).step_by(step).map(lum).collect();
    sample.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let white = sample.get(((sample.len() as f64) * 0.995) as usize).copied().unwrap_or(1.0).max(1.0);
    let w2 = white * white;
    let mut out = Vec::with_capacity(n * 3);
    for c in img.px.chunks_exact(3) {
        let l = lum(c);
        let (r, g, b) = if l > 0.0 {
            let ld = l * (1.0 + l / w2) / (1.0 + l);
            let k = ld / l;
            let (mut r, mut g, mut b) = (c[0].max(0.0) * k, c[1].max(0.0) * k, c[2].max(0.0) * k);
            // soft clip: pull over-range colours towards the grey of the same brightness
            let m = r.max(g).max(b);
            if m > 1.0 {
                let t = (m - 1.0) / (m - ld).max(1e-6);
                r += (ld - r) * t;
                g += (ld - g) * t;
                b += (ld - b) * t;
            }
            (r, g, b)
        } else {
            (0.0, 0.0, 0.0)
        };
        out.extend_from_slice(&[srgb(r), srgb(g), srgb(b)]);
    }
    RgbImage::from_raw(img.w, img.h, out).unwrap_or_else(|| RgbImage::new(1, 1))
}

/// An HDR picture as an SDR image, or None if `path` is not an HDR picture.
pub fn load_tonemapped(path: &Path, max_w: u32, max_h: u32) -> Option<Result<RgbImage, String>> {
    if is_jxr(path) {
        Some(load_jxr(path, max_w, max_h).map(|l| tonemap(&l)))
    } else if png_is_hdr(path) {
        Some(load_png_hdr(path, max_w, max_h).map(|l| tonemap(&l)))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gradient(w: u32, h: u32, peak: f32) -> Linear {
        let mut px = Vec::new();
        for _y in 0..h {
            for x in 0..w {
                let v = peak * x as f32 / (w - 1) as f32;
                px.extend_from_slice(&[v, v * 0.8, v * 0.6]);
            }
        }
        Linear { w, h, px }
    }

    #[test]
    fn tonemap_is_monotonic_and_not_clipped() {
        let img = tonemap(&gradient(256, 4, 12.5)); // up to 1000 nits
        // green carries most of the luminance; red (the warmest channel) is allowed to saturate
        let row: Vec<u8> = (0..256).map(|x| img.get_pixel(x, 0)[1]).collect();
        assert!(row.windows(2).all(|w| w[1] >= w[0]), "monotonic");
        assert_eq!(row[0], 0);
        assert!(row[255] >= 200, "the brightest part is bright: {}", row[255]);
        let clipped = row.iter().filter(|&&v| v == 255).count();
        assert!(clipped < 10, "only the very top clips, got {clipped}");
        assert!(row[64] > 90 && row[64] < 230, "midtones stay visible: {}", row[64]);
    }

    #[test]
    fn pq_and_hlg_curves() {
        assert!((pq_to_nits(0.0)).abs() < 0.01);
        assert!((pq_to_nits(1.0) - 10_000.0).abs() < 1.0);
        assert!((pq_to_nits(0.5081) - 100.0).abs() < 3.0, "{}", pq_to_nits(0.5081));
        assert!((hlg_to_nits(1.0) - 1000.0).abs() < 1.0);
    }

    fn write_pq_png(path: &Path) {
        // 16-bit PNG with cICP (BT.2020, PQ): a gradient of PQ code values
        let (w, h) = (64u32, 2u32);
        let mut img = image::ImageBuffer::<image::Rgb<u16>, Vec<u16>>::new(w, h);
        for (x, _y, p) in img.enumerate_pixels_mut() {
            let v = (x as f32 / (w - 1) as f32 * 0.75 * 65535.0) as u16;
            *p = image::Rgb([v, v, v]);
        }
        let mut bytes = Vec::new();
        image::DynamicImage::ImageRgb16(img).write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png).unwrap();
        // insert cICP right after IHDR (8 + 25 bytes)
        let mut chunk = Vec::new();
        chunk.extend_from_slice(&4u32.to_be_bytes());
        chunk.extend_from_slice(b"cICP");
        chunk.extend_from_slice(&[9, 16, 0, 1]);
        let crc = crc32(&chunk[4..]);
        chunk.extend_from_slice(&crc.to_be_bytes());
        let mut out = bytes[..33].to_vec();
        out.extend_from_slice(&chunk);
        out.extend_from_slice(&bytes[33..]);
        std::fs::write(path, out).unwrap();
    }

    fn crc32(data: &[u8]) -> u32 {
        let mut c = 0xffff_ffffu32;
        for &b in data {
            c ^= b as u32;
            for _ in 0..8 {
                c = if c & 1 != 0 { 0xedb8_8320 ^ (c >> 1) } else { c >> 1 };
            }
        }
        !c
    }

    #[test]
    fn pq_png_is_detected_and_tonemapped() {
        let d = std::env::temp_dir().join(format!("hangar-hdr-png-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let p = d.join("hdr.png");
        write_pq_png(&p);
        assert_eq!(png_cicp(&p), Some((9, 16)));
        assert!(png_is_hdr(&p));
        let img = load_tonemapped(&p, 800, 600).unwrap().unwrap();
        let row: Vec<u8> = (0..64).map(|x| img.get_pixel(x, 0)[1]).collect();
        assert!(row.windows(2).all(|w| w[1] >= w[0]));
        assert!(row[63] > 240 && row[10] < 120, "{row:?}");
        std::fs::remove_dir_all(&d).unwrap();
    }

    /// Encodes a float HDR picture as JPEG XR with WIC, then reads it back through the same path
    /// as the Hangar wall. Writes only to %TEMP%.
    #[cfg(windows)]
    #[test]
    fn jxr_round_trip_with_wic() {
        use windows::Win32::Graphics::Imaging::*;
        use windows::Win32::System::Com::StructuredStorage::IPropertyBag2;
        use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx};
        use windows::core::HSTRING;
        let d = std::env::temp_dir().join(format!("hangar-hdr-jxr-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let p = d.join("capture.jxr");
        let (w, h) = (128u32, 8u32);
        let src = gradient(w, h, 12.5);
        let mut bytes = Vec::with_capacity((w * h * 16) as usize);
        for c in src.px.chunks_exact(3) {
            for v in [c[0], c[1], c[2], 1.0f32] {
                bytes.extend_from_slice(&v.to_le_bytes());
            }
        }
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            let f: IWICImagingFactory = CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER).unwrap();
            let stream = f.CreateStream().unwrap();
            stream.InitializeFromFilename(&HSTRING::from(p.as_os_str()), 0x4000_0000 /* GENERIC_WRITE */).unwrap();
            let enc = f.CreateEncoder(&GUID_ContainerFormatWmp, std::ptr::null()).unwrap();
            enc.Initialize(&stream, WICBitmapEncoderNoCache).unwrap();
            let mut frame: Option<IWICBitmapFrameEncode> = None;
            let mut opts: Option<IPropertyBag2> = None;
            enc.CreateNewFrame(&mut frame, &mut opts).unwrap();
            let frame = frame.unwrap();
            frame.Initialize(opts.as_ref()).unwrap();
            frame.SetSize(w, h).unwrap();
            let mut fmt = GUID_WICPixelFormat128bppRGBAFloat;
            frame.SetPixelFormat(&mut fmt).unwrap();
            assert_eq!(fmt, GUID_WICPixelFormat128bppRGBAFloat, "the JPEG XR encoder accepts float pixels");
            frame.WritePixels(h, w * 16, &bytes).unwrap();
            frame.Commit().unwrap();
            enc.Commit().unwrap();
        }
        let back = load_jxr(&p, 2200, 1400).unwrap();
        assert_eq!((back.w, back.h), (w, h));
        let peak = back.px.chunks_exact(3).map(|c| c[0]).fold(0.0f32, f32::max);
        assert!(peak > 10.0 && peak < 15.0, "HDR values survive: peak {peak}");
        let img = load_tonemapped(&p, 64, 64).unwrap().unwrap();
        assert_eq!(img.width(), 64, "scaled while decoding");
        let row: Vec<u8> = (0..64).map(|x| img.get_pixel(x, 0)[1]).collect();
        assert!(row.windows(2).all(|w| w[1] as i16 + 2 >= w[0] as i16), "{row:?}");
        assert!(row[63] > 200 && row[4] < 200 && row[0] < 10, "{row:?}");
        std::fs::remove_dir_all(&d).unwrap();
    }
}
