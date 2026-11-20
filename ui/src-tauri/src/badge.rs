//! The unread count as a red badge over Ripple's taskbar button. Windows has no badge
//! API for desktop apps, only an "overlay icon", so the number is drawn here.

/// 3×5 pixel digits (and "+"), one row per `u8`, top to bottom, bits left to right.
const GLYPHS: [[u8; 5]; 11] = [
    [0b111, 0b101, 0b101, 0b101, 0b111], // 0
    [0b010, 0b110, 0b010, 0b010, 0b111], // 1
    [0b111, 0b001, 0b111, 0b100, 0b111], // 2
    [0b111, 0b001, 0b111, 0b001, 0b111], // 3
    [0b101, 0b101, 0b111, 0b001, 0b001], // 4
    [0b111, 0b100, 0b111, 0b001, 0b111], // 5
    [0b111, 0b100, 0b111, 0b101, 0b111], // 6
    [0b111, 0b001, 0b001, 0b001, 0b001], // 7
    [0b111, 0b101, 0b111, 0b101, 0b111], // 8
    [0b111, 0b101, 0b111, 0b001, 0b111], // 9
    [0b000, 0b010, 0b111, 0b010, 0b000], // +
];

const SIZE: u32 = 32;
const RED: [u8; 3] = [0xe0, 0x3b, 0x3b];

/// A 32×32 RGBA badge for `count` unread chats, or `None` for no badge.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn overlay(count: u32) -> Option<(Vec<u8>, u32, u32)> {
    if count == 0 {
        return None;
    }
    let text: Vec<usize> = if count > 99 {
        vec![9, 9, 10]
    } else {
        count.to_string().bytes().map(|b| (b - b'0') as usize).collect()
    };
    // Bigger pixels for fewer characters, so a single digit reads at taskbar size.
    let scale = match text.len() {
        1 => 4,
        2 => 3,
        _ => 2,
    };
    let gap = scale;
    let width = text.len() as u32 * 3 * scale + (text.len() as u32 - 1) * gap;
    let height = 5 * scale;
    let (left, top) = ((SIZE - width) / 2, (SIZE - height) / 2);
    let mut rgba = vec![0u8; (SIZE * SIZE * 4) as usize];
    let c = SIZE as f32 / 2.0;
    for y in 0..SIZE {
        for x in 0..SIZE {
            let d = ((x as f32 + 0.5 - c).powi(2) + (y as f32 + 0.5 - c).powi(2)).sqrt();
            let cover = (c - d + 0.5).clamp(0.0, 1.0);
            if cover == 0.0 {
                continue;
            }
            let lit = x >= left && y >= top && y < top + height && {
                let col = (x - left) / scale;
                let (glyph, bit) = (col / (3 + gap / scale), col % (3 + gap / scale));
                glyph < text.len() as u32
                    && bit < 3
                    && GLYPHS[text[glyph as usize]][((y - top) / scale) as usize] & (0b100 >> bit) != 0
            };
            let i = ((y * SIZE + x) * 4) as usize;
            let rgb = if lit { [255, 255, 255] } else { RED };
            rgba[i..i + 3].copy_from_slice(&rgb);
            rgba[i + 3] = (cover * 255.0).round() as u8;
        }
    }
    Some((rgba, SIZE, SIZE))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pixel(img: &[u8], x: u32, y: u32) -> [u8; 4] {
        let i = ((y * SIZE + x) * 4) as usize;
        [img[i], img[i + 1], img[i + 2], img[i + 3]]
    }

    #[test]
    fn draws_the_count() {
        assert!(overlay(0).is_none());
        let (one, w, h) = overlay(1).unwrap();
        assert_eq!((w, h, one.len()), (32, 32, 32 * 32 * 4));
        assert_eq!(pixel(&one, 0, 0)[3], 0, "outside the circle is clear");
        // "1" is 12×20 px at (10, 6): its stem runs down the middle column.
        assert_eq!(pixel(&one, 15, 16), [255, 255, 255, 255]);
        assert_eq!(pixel(&one, 11, 16)[..3], RED, "beside the stem is red");
        let (many, ..) = overlay(250).unwrap();
        let (two, ..) = overlay(12).unwrap();
        assert_ne!(many, two);
        assert!(many.chunks(4).filter(|p| p[..3] == [255, 255, 255]).count() > 20);
    }
}
