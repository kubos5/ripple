//! The on-disk media library and the image helpers around it.
//!
//! Files are addressed by a *token*: a relative path inside the media
//! directory such as `m/9f2c4e.jpg`. The UI only ever sees tokens.

use std::io::Cursor;
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result, bail};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use image::imageops::FilterType;
use image::{DynamicImage, GenericImageView, ImageFormat};

use crate::api::Attachment;

pub struct MediaDir {
    root: PathBuf,
}

pub fn random_hex(bytes: usize) -> String {
    (0..bytes).map(|_| format!("{:02x}", rand::random::<u8>())).collect()
}

impl MediaDir {
    pub fn new(root: PathBuf) -> Result<MediaDir> {
        std::fs::create_dir_all(&root).with_context(|| format!("create {}", root.display()))?;
        Ok(MediaDir { root })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Resolves a token to a path, refusing anything that escapes the media directory.
    pub fn path(&self, token: &str) -> Option<PathBuf> {
        let rel = Path::new(token);
        if token.is_empty() || rel.components().any(|c| !matches!(c, Component::Normal(_))) {
            return None;
        }
        Some(self.root.join(rel))
    }

    /// Reserves a new file name under `folder` with the given extension.
    pub fn fresh(&self, folder: &str, ext: &str) -> Result<(String, PathBuf)> {
        let dir = self.root.join(folder);
        std::fs::create_dir_all(&dir)?;
        let ext = sanitize_ext(ext);
        let token = format!("{folder}/{}.{ext}", random_hex(10));
        let path = self.root.join(&token);
        Ok((token, path))
    }

    pub fn write(&self, folder: &str, ext: &str, bytes: &[u8]) -> Result<String> {
        let (token, path) = self.fresh(folder, ext)?;
        std::fs::write(&path, bytes)?;
        Ok(token)
    }

    /// Copies an attachment into the library. Returns its token and size.
    pub fn import(&self, att: &Attachment) -> Result<(String, u64)> {
        let ext = ext_for(&att.mime, &att.name);
        if let Some(src) = &att.path {
            let (token, dst) = self.fresh("m", &ext)?;
            let size = std::fs::copy(src, &dst).with_context(|| format!("read {src}"))?;
            return Ok((token, size));
        }
        if let Some(data) = &att.data {
            let bytes = B64.decode(strip_data_url(data)).context("attachment is not valid base64")?;
            let token = self.write("m", &ext, &bytes)?;
            return Ok((token, bytes.len() as u64));
        }
        bail!("attachment has neither a path nor data")
    }

    /// Total bytes on disk.
    pub fn usage(&self) -> u64 {
        fn walk(dir: &Path) -> u64 {
            let Ok(rd) = std::fs::read_dir(dir) else { return 0 };
            rd.flatten()
                .map(|e| match e.metadata() {
                    Ok(m) if m.is_dir() => walk(&e.path()),
                    Ok(m) => m.len(),
                    Err(_) => 0,
                })
                .sum()
        }
        walk(&self.root)
    }
}

fn strip_data_url(s: &str) -> &str {
    match s.find(";base64,") {
        Some(i) if s.starts_with("data:") => &s[i + 8..],
        _ => s,
    }
}

fn sanitize_ext(ext: &str) -> String {
    let e: String = ext.chars().filter(|c| c.is_ascii_alphanumeric()).take(8).collect();
    if e.is_empty() { "bin".into() } else { e.to_ascii_lowercase() }
}

pub fn ext_for(mime: &str, name: &str) -> String {
    if let Some(ext) = Path::new(name).extension().and_then(|e| e.to_str()) {
        return sanitize_ext(ext);
    }
    let base = mime.split(';').next().unwrap_or("").trim();
    match base {
        "image/jpeg" => "jpg",
        "image/png" => "png",
        "image/webp" => "webp",
        "image/gif" => "gif",
        "video/mp4" => "mp4",
        "video/quicktime" => "mov",
        "video/webm" => "webm",
        "audio/ogg" => "ogg",
        "audio/mpeg" => "mp3",
        "audio/mp4" | "audio/aac" => "m4a",
        "audio/wav" | "audio/x-wav" => "wav",
        "application/pdf" => "pdf",
        "text/plain" => "txt",
        "text/vcard" => "vcf",
        _ => "bin",
    }
    .to_string()
}

pub fn mime_for(name: &str) -> &'static str {
    let ext = Path::new(name).extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    match ext.as_str() {
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "webp" => "image/webp",
        "gif" => "image/gif",
        "svg" => "image/svg+xml",
        "mp4" | "m4v" => "video/mp4",
        "mov" => "video/quicktime",
        "webm" => "video/webm",
        "ogg" | "opus" | "oga" => "audio/ogg",
        "mp3" => "audio/mpeg",
        "m4a" | "aac" => "audio/mp4",
        "wav" => "audio/wav",
        "pdf" => "application/pdf",
        "txt" | "md" | "log" => "text/plain",
        "vcf" => "text/vcard",
        "zip" => "application/zip",
        "doc" => "application/msword",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "xls" => "application/vnd.ms-excel",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "ppt" => "application/vnd.ms-powerpoint",
        "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        "json" => "application/json",
        "csv" => "text/csv",
        _ => "application/octet-stream",
    }
}

pub fn dimensions(path: &Path) -> Option<(u32, u32)> {
    image::image_dimensions(path).ok()
}

fn encode_jpeg(img: &DynamicImage, quality: u8) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    let rgb = img.to_rgb8();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, quality).encode_image(&rgb)?;
    Ok(out)
}

/// A small base64 JPEG used as a blurred placeholder and as the thumbnail WhatsApp expects.
pub fn thumb_b64(path: &Path, max_side: u32) -> Option<String> {
    let img = image::open(path).ok()?;
    let small = img.resize(max_side, max_side, FilterType::Triangle);
    encode_jpeg(&small, 55).ok().map(|b| B64.encode(b))
}

pub fn thumb_from_bytes(bytes: &[u8], max_side: u32) -> Option<String> {
    let img = image::load_from_memory(bytes).ok()?;
    let small = img.resize(max_side, max_side, FilterType::Triangle);
    encode_jpeg(&small, 55).ok().map(|b| B64.encode(b))
}

/// Converts any image to the 512×512 WebP sticker format (letterboxed on transparency).
pub fn to_sticker(path: &Path) -> Result<Vec<u8>> {
    let img = image::open(path).context("could not read that image")?;
    let fitted = img.resize(512, 512, FilterType::Lanczos3);
    let mut canvas = image::RgbaImage::new(512, 512);
    let (w, h) = fitted.dimensions();
    image::imageops::overlay(&mut canvas, &fitted.to_rgba8(), ((512 - w) / 2) as i64, ((512 - h) / 2) as i64);
    let mut out = Vec::new();
    DynamicImage::ImageRgba8(canvas).write_to(&mut Cursor::new(&mut out), ImageFormat::WebP)?;
    Ok(out)
}

/// Centre-crops to a square JPEG for profile and group pictures.
pub fn to_avatar(path: &Path) -> Result<Vec<u8>> {
    let img = image::open(path).context("could not read that image")?;
    let (w, h) = img.dimensions();
    let side = w.min(h);
    let square = img.crop_imm((w - side) / 2, (h - side) / 2, side, side).resize(640, 640, FilterType::Lanczos3);
    encode_jpeg(&square, 85)
}

/// WebP files with an `ANIM` chunk are animated stickers.
pub fn is_animated_webp(bytes: &[u8]) -> bool {
    bytes.len() > 32 && &bytes[0..4] == b"RIFF" && bytes[..bytes.len().min(256)].windows(4).any(|w| w == b"ANIM")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_cannot_escape() {
        let dir = std::env::temp_dir().join(format!("ripple-media-test-{}", random_hex(4)));
        let m = MediaDir::new(dir.clone()).unwrap();
        assert!(m.path("m/abc.jpg").is_some());
        assert!(m.path("../secret").is_none());
        assert!(m.path("/etc/passwd").is_none());
        assert!(m.path("m/../../x").is_none());
        assert!(m.path("").is_none());
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn import_thumb_and_sticker() {
        let dir = std::env::temp_dir().join(format!("ripple-media-test-{}", random_hex(4)));
        let m = MediaDir::new(dir.clone()).unwrap();
        let img = DynamicImage::ImageRgb8(image::RgbImage::from_fn(200, 100, |x, y| {
            image::Rgb([(x % 255) as u8, (y % 255) as u8, 120])
        }));
        let mut png = Vec::new();
        img.write_to(&mut Cursor::new(&mut png), ImageFormat::Png).unwrap();
        let att = Attachment {
            data: Some(format!("data:image/png;base64,{}", B64.encode(&png))),
            name: "pic.png".into(),
            mime: "image/png".into(),
            ..Default::default()
        };
        let (token, size) = m.import(&att).unwrap();
        assert!(token.starts_with("m/") && token.ends_with(".png"));
        assert_eq!(size as usize, png.len());
        let path = m.path(&token).unwrap();
        assert_eq!(dimensions(&path), Some((200, 100)));
        assert!(thumb_b64(&path, 48).is_some());
        let sticker = to_sticker(&path).unwrap();
        assert_eq!(&sticker[0..4], b"RIFF");
        assert!(!is_animated_webp(&sticker));
        assert!(to_avatar(&path).unwrap().len() > 100);
        assert!(m.usage() > 0);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn extension_guessing() {
        assert_eq!(ext_for("image/jpeg", ""), "jpg");
        assert_eq!(ext_for("", "Report Final.PDF"), "pdf");
        assert_eq!(ext_for("weird/thing", "noext"), "bin");
        assert_eq!(mime_for("a.OGG"), "audio/ogg");
    }
}
