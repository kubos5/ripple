//! Procedural pictures for the demo, so it needs no bundled photos.

use std::io::Cursor;

use image::{ImageFormat, Rgb, RgbImage, Rgba, RgbaImage};

type C = [f32; 3];

fn mix(a: C, b: C, t: f32) -> C {
    let t = t.clamp(0.0, 1.0);
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

fn px(c: C) -> Rgb<u8> {
    Rgb([(c[0] * 255.0) as u8, (c[1] * 255.0) as u8, (c[2] * 255.0) as u8])
}

fn hex(v: u32) -> C {
    [((v >> 16) & 255) as f32 / 255.0, ((v >> 8) & 255) as f32 / 255.0, (v & 255) as f32 / 255.0]
}

/// (sky top, sky bottom, sun, far hills, near hills)
const PALETTES: [[u32; 5]; 6] = [
    [0x2b3a67, 0xf7a072, 0xfff1c9, 0x6d597a, 0x1f2041],
    [0x8ecae6, 0xfefae0, 0xffffff, 0x52796f, 0x1b4332],
    [0x16213e, 0x533483, 0xf5c7a9, 0x30305a, 0x0f0f24],
    [0xffb4a2, 0xffe5d9, 0xfffaf0, 0xb5838d, 0x6d6875],
    [0x90e0ef, 0xcaf0f8, 0xffffff, 0x0096c7, 0x023e8a],
    [0xf4a261, 0xe9c46a, 0xfff3d6, 0x2a9d8f, 0x264653],
];

/// A flat-illustration landscape: sky gradient, sun with glow, layered hills.
pub fn scene(seed: u32, w: u32, h: u32) -> RgbImage {
    let p = PALETTES[seed as usize % PALETTES.len()].map(hex);
    let s = seed as f32;
    let sun = (0.22 + 0.56 * ((s * 0.37).sin() * 0.5 + 0.5), 0.30 + 0.12 * (s * 0.71).cos());
    RgbImage::from_fn(w, h, |x, y| {
        let u = x as f32 / w as f32;
        let v = y as f32 / h as f32;
        let mut c = mix(p[0], p[1], v * 1.25);
        let aspect = w as f32 / h as f32;
        let d = (((u - sun.0) * aspect).powi(2) + (v - sun.1).powi(2)).sqrt();
        c = mix(c, p[2], (1.0 - d / 0.42).max(0.0).powi(2) * 0.55);
        if d < 0.07 {
            c = p[2];
        }
        for layer in 0..4 {
            let lf = layer as f32;
            let base = 0.58 + lf * 0.10;
            let wave = (u * (3.0 + lf * 1.7) + s + lf * 2.1).sin() * 0.055
                + (u * (7.0 + lf * 2.3) + s * 1.7 + lf).sin() * 0.022;
            if v > base + wave {
                let depth = lf / 3.0;
                let fog = mix(p[3], p[4], depth);
                c = mix(fog, mix(fog, p[1], 0.25), ((base + wave + 0.12 - v) * 6.0).max(0.0) * (1.0 - depth));
            }
        }
        px(c)
    })
}

pub fn jpeg(img: &RgbImage, quality: u8) -> Vec<u8> {
    let mut out = Vec::new();
    let _ = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, quality).encode_image(img);
    out
}

const AVATAR_TONES: [[u32; 2]; 8] = [
    [0xf6bd60, 0xf28482],
    [0x84a59d, 0x3d5a80],
    [0xcdb4db, 0x6d6875],
    [0x90be6d, 0x277da1],
    [0xffafcc, 0xa2d2ff],
    [0xe76f51, 0x264653],
    [0xb8c0ff, 0x5e60ce],
    [0xffd166, 0x06d6a0],
];

/// A soft two-tone portrait placeholder: head and shoulders on a gradient.
pub fn avatar(seed: u32) -> RgbImage {
    let t = AVATAR_TONES[seed as usize % AVATAR_TONES.len()].map(hex);
    let n = 320u32;
    RgbImage::from_fn(n, n, |x, y| {
        let u = x as f32 / n as f32;
        let v = y as f32 / n as f32;
        let mut c = mix(t[0], t[1], (u * 0.6 + v * 0.8).clamp(0.0, 1.0));
        let light = mix(c, [1.0, 1.0, 1.0], 0.55);
        let head = ((u - 0.5).powi(2) + (v - 0.40).powi(2)).sqrt();
        let body = ((u - 0.5) / 0.36).powi(2) + ((v - 1.02) / 0.34).powi(2);
        if head < 0.165 || body < 1.0 {
            c = light;
        }
        px(c)
    })
}

/// A simple round face sticker on transparency. `mood` picks the expression.
pub fn sticker(mood: u32) -> Vec<u8> {
    let n = 512i32;
    let tones = [hex(0xffd166), hex(0xff8fab), hex(0x8ecae6), hex(0xb8f2c8)];
    let base = tones[mood as usize % tones.len()];
    let ink = [0.16f32, 0.13, 0.20];
    let mut img = RgbaImage::new(n as u32, n as u32);
    for y in 0..n {
        for x in 0..n {
            let (fx, fy) = (x as f32 - 256.0, y as f32 - 256.0);
            let r = (fx * fx + fy * fy).sqrt();
            if r > 232.0 {
                continue;
            }
            let shade = 1.0 - ((fx + 70.0).powi(2) + (fy + 80.0).powi(2)).sqrt() / 620.0;
            let mut c = [base[0] * shade.max(0.72), base[1] * shade.max(0.72), base[2] * shade.max(0.72)];
            if r > 220.0 {
                c = ink;
            }
            // eyes
            for ex in [-78.0f32, 78.0] {
                let (dx, dy) = (fx - ex, fy + 40.0);
                let eye = match mood % 4 {
                    1 => dy.abs() < 9.0 && dx.abs() < 34.0 && dy < (dx.abs() * -0.2 + 6.0), // content squint
                    3 => (dx * dx + dy * dy).sqrt() < 34.0 && dy > -6.0,                     // sleepy
                    _ => (dx / 22.0).powi(2) + (dy / 30.0).powi(2) < 1.0,
                };
                if eye {
                    c = ink;
                }
            }
            // mouth
            let (mx, my) = (fx, fy - 62.0);
            let mouth = match mood % 4 {
                0 => {
                    let d = (mx * mx + (my + 44.0).powi(2)).sqrt();
                    (d - 70.0).abs() < 9.0 && my > 2.0
                }
                1 => (mx / 62.0).powi(2) + (my / 46.0).powi(2) < 1.0 && my > 0.0,
                2 => (mx * mx + my * my).sqrt() < 26.0,
                _ => my.abs() < 8.0 && mx.abs() < 46.0,
            };
            if mouth {
                c = if mood % 4 == 1 { [0.72, 0.20, 0.28] } else { ink };
            }
            // cheeks
            for cxk in [-130.0f32, 130.0] {
                if ((fx - cxk) / 34.0).powi(2) + ((fy - 26.0) / 20.0).powi(2) < 1.0 {
                    c = mix(c, [1.0, 0.45, 0.50], 0.45);
                }
            }
            let edge = ((232.0 - r) / 1.5).clamp(0.0, 1.0);
            img.put_pixel(
                x as u32,
                y as u32,
                Rgba([(c[0] * 255.0) as u8, (c[1] * 255.0) as u8, (c[2] * 255.0) as u8, (edge * 255.0) as u8]),
            );
        }
    }
    let mut out = Vec::new();
    let _ = image::DynamicImage::ImageRgba8(img).write_to(&mut Cursor::new(&mut out), ImageFormat::WebP);
    out
}

/// A one-page PDF with a title line, small enough to build by hand.
pub fn pdf(title: &str) -> Vec<u8> {
    let safe: String = title.chars().filter(|c| c.is_ascii() && !"()\\".contains(*c)).collect();
    let stream = format!("BT /F1 26 Tf 72 720 Td ({safe}) Tj ET\nBT /F1 12 Tf 72 690 Td (Generated for the Ripple demo.) Tj ET");
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>".to_string(),
        format!("<< /Length {} >>\nstream\n{stream}\nendstream", stream.len()),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_string(),
    ];
    let mut out = b"%PDF-1.4\n".to_vec();
    let mut offsets = vec![];
    for (i, o) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n{o}\nendobj\n", i + 1).as_bytes());
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes());
    for off in offsets {
        out.extend_from_slice(format!("{off:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objects.len() + 1).as_bytes(),
    );
    out
}

/// A short synthetic "voice" clip: a few pitched syllables with pauses.
pub fn speech_pcm(seed: u32, secs: f32) -> Vec<f32> {
    let rate = 48_000f32;
    let n = (rate * secs) as usize;
    let mut out = Vec::with_capacity(n);
    let mut phase = 0f32;
    for i in 0..n {
        let t = i as f32 / rate;
        let syll = (t * 3.1 + seed as f32 * 0.37).fract();
        let word = ((t * 0.9 + seed as f32).sin() * 0.5 + 0.5).powf(0.6);
        let env = (syll * std::f32::consts::PI).sin().powf(1.6) * word;
        let pitch = 150.0 + 60.0 * ((t * 1.7 + seed as f32).sin()) + 25.0 * (syll * 6.0).sin();
        phase += pitch / rate;
        let s = (phase * std::f32::consts::TAU).sin() * 0.55
            + (phase * 2.0 * std::f32::consts::TAU).sin() * 0.25
            + (phase * 3.0 * std::f32::consts::TAU).sin() * 0.12;
        out.push(s * env * 0.5);
    }
    out
}
