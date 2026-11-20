//! Draws Ripple's app icons for Windows and Linux (the three circles on a transparent
//! background), their green versions, and the green macOS icon used when "Green app icon"
//! is on. The default macOS icon is the layered one in `icons/macos` and is not touched.
//!
//! Run from the repository root:
//!     cargo run -p ripple-core --no-default-features --example app_icons

use std::path::Path;

use image::{ImageFormat, Rgba, RgbaImage};

/// Neutral enough to read on light and dark taskbars alike.
const GREY: [u8; 3] = [0x78, 0x7e, 0x86];
/// The green people know from WhatsApp's icon.
const GREEN: [u8; 3] = [0x25, 0xd3, 0x66];

/// Radii as fractions of the outer ring's outer edge, from the macOS icon's layers
/// (outer ring 345–400, inner ring 208–265, dot 104, on a 1024 canvas).
struct Rings {
    dot: f32,
    inner: (f32, f32),
    outer: (f32, f32),
}

const RINGS: Rings = Rings { dot: 104.0 / 400.0, inner: (208.0 / 400.0, 265.0 / 400.0), outer: (345.0 / 400.0, 1.0) };

/// Coverage of the band `lo..hi` at distance `d`, with one pixel of anti-aliasing.
fn band(d: f32, lo: f32, hi: f32) -> f32 {
    ((d - lo + 0.5).clamp(0.0, 1.0)).min((hi - d + 0.5).clamp(0.0, 1.0))
}

/// The three circles in one colour. Tiny sizes get rings at least `min_line` px wide (the
/// gaps shrink instead), so they stay three circles rather than a grey smudge.
fn circles(size: u32, colour: [u8; 3]) -> RgbaImage {
    let c = size as f32 / 2.0;
    let r = c - (size as f32 * 0.03).max(0.5);
    let min_line = 1.35;
    let widen = |(lo, hi): (f32, f32)| {
        let (lo, hi) = (lo * r, hi * r);
        if hi - lo >= min_line { (lo, hi) } else { (hi - min_line, hi) }
    };
    let (inner, outer) = (widen(RINGS.inner), widen(RINGS.outer));
    let dot = (RINGS.dot * r).max(1.6);
    RgbaImage::from_fn(size, size, |x, y| {
        let d = ((x as f32 + 0.5 - c).powi(2) + (y as f32 + 0.5 - c).powi(2)).sqrt();
        let a = band(d, -1.0, dot) + band(d, inner.0, inner.1) + band(d, outer.0, outer.1);
        Rgba([colour[0], colour[1], colour[2], (a.min(1.0) * 255.0).round() as u8])
    })
}

/// A macOS-style icon: a green rounded square with the white circles, on the standard
/// 1024 grid (an 824 px body with a soft shadow).
fn mac_green() -> RgbaImage {
    const S: u32 = 1024;
    let (body, half, n) = (824.0f32, 412.0f32, 5.0f32);
    let c = S as f32 / 2.0;
    // Superellipse, a close match for Apple's continuous corners; 4×4 samples per pixel.
    let inside = |x: f32, y: f32| ((x - c).abs() / half).powf(n) + ((y - c).abs() / half).powf(n) <= 1.0;
    let mask = RgbaImage::from_fn(S, S, |x, y| {
        let mut hit = 0;
        for i in 0..4 {
            for j in 0..4 {
                hit += inside(x as f32 + (i as f32 + 0.5) / 4.0, y as f32 + (j as f32 + 0.5) / 4.0) as u32;
            }
        }
        Rgba([0, 0, 0, (hit * 255 / 16) as u8])
    });
    // Shadow: the body's shape, blurred and nudged down.
    let mut shadow = image::imageops::blur(&mask, 14.0);
    for p in shadow.pixels_mut() {
        p[3] = (p[3] as f32 * 0.32) as u8;
    }
    let mut out = RgbaImage::new(S, S);
    image::imageops::overlay(&mut out, &shadow, 0, 12);
    let r = body * 0.39; // outer ring, as in the default icon
    let top = [0x3d, 0xde, 0x80];
    let bottom = [0x1a, 0xa8, 0x55];
    let mut face = RgbaImage::new(S, S);
    for (x, y, p) in face.enumerate_pixels_mut() {
        let a = mask.get_pixel(x, y)[3];
        if a == 0 {
            continue;
        }
        let t = ((y as f32 - (c - half)) / body).clamp(0.0, 1.0);
        let mut rgb = [0f32; 3];
        for k in 0..3 {
            rgb[k] = top[k] as f32 + (bottom[k] as f32 - top[k] as f32) * t;
        }
        let d = ((x as f32 + 0.5 - c).powi(2) + (y as f32 + 0.5 - c).powi(2)).sqrt();
        // White circles; the rings fade outwards like the default icon's.
        let white = band(d, -1.0, RINGS.dot * r)
            + band(d, RINGS.inner.0 * r, RINGS.inner.1 * r) * 0.72
            + band(d, RINGS.outer.0 * r, RINGS.outer.1 * r) * 0.42;
        let w = white.min(1.0);
        for v in &mut rgb {
            *v += (255.0 - *v) * w;
        }
        *p = Rgba([rgb[0] as u8, rgb[1] as u8, rgb[2] as u8, a]);
    }
    image::imageops::overlay(&mut out, &face, 0, 0);
    out
}

fn save(img: &RgbaImage, path: &Path) {
    img.save(path).unwrap_or_else(|e| panic!("write {}: {e}", path.display()));
    println!("wrote {}", path.display());
}

/// An .ico holding a PNG per size: Windows picks the closest for each place it shows the icon.
fn ico(colour: [u8; 3], path: &Path) {
    let sizes = [16u32, 20, 24, 32, 40, 48, 64, 128, 256];
    let pngs: Vec<Vec<u8>> = sizes
        .iter()
        .map(|&s| {
            let mut png = Vec::new();
            circles(s, colour).write_to(&mut std::io::Cursor::new(&mut png), ImageFormat::Png).unwrap();
            png
        })
        .collect();
    let mut out = vec![0, 0, 1, 0];
    out.extend_from_slice(&(sizes.len() as u16).to_le_bytes());
    let mut offset = 6 + 16 * sizes.len() as u32;
    for (&s, png) in sizes.iter().zip(&pngs) {
        let side = if s >= 256 { 0 } else { s as u8 }; // 0 means 256
        out.extend_from_slice(&[side, side, 0, 0]);
        out.extend_from_slice(&1u16.to_le_bytes()); // colour planes
        out.extend_from_slice(&32u16.to_le_bytes()); // bits per pixel
        out.extend_from_slice(&(png.len() as u32).to_le_bytes());
        out.extend_from_slice(&offset.to_le_bytes());
        offset += png.len() as u32;
    }
    for png in &pngs {
        out.extend_from_slice(png);
    }
    std::fs::write(path, out).unwrap();
    println!("wrote {}", path.display());
}

fn main() {
    let icons = Path::new("ui/src-tauri/icons");
    assert!(icons.is_dir(), "run this from the repository root");
    save(&circles(32, GREY), &icons.join("32x32.png"));
    save(&circles(128, GREY), &icons.join("128x128.png"));
    save(&circles(256, GREY), &icons.join("128x128@2x.png"));
    save(&circles(512, GREY), &icons.join("icon.png"));
    ico(GREY, &icons.join("icon.ico"));

    let green = icons.join("green");
    std::fs::create_dir_all(&green).unwrap();
    save(&circles(32, GREEN), &green.join("32x32.png"));
    save(&circles(256, GREEN), &green.join("256x256.png"));
    save(&mac_green(), &green.join("macos.png"));
}
