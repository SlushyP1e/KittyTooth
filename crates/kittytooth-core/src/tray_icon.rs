//! Renders the live BPM into a small RGBA bitmap for the system tray icon.
//! Uses a built-in 3x5 pixel font scaled up, so it stays crisp at 16-32 px.

pub const ICON_SIZE: u32 = 32;

const FONT: [[u8; 5]; 11] = [
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
    [0b000, 0b000, 0b111, 0b000, 0b000], // -
];

/// Tray color for an accent setting: preset name or `#rrggbb` (mirrors the UI tokens).
pub fn accent_rgb(accent: &str) -> [u8; 3] {
    match accent {
        "peach" => [0xff, 0xc6, 0xa5],
        "lilac" => [0xd4, 0xb8, 0xff],
        "sky" => [0xa8, 0xd1, 0xff],
        other => parse_hex(other).unwrap_or([0xff, 0xad, 0xc6]),
    }
}

/// Parses `#rrggbb`.
pub fn parse_hex(s: &str) -> Option<[u8; 3]> {
    let s = s.strip_prefix('#')?;
    if s.len() != 6 {
        return None;
    }
    let v = u32::from_str_radix(s, 16).ok()?;
    Some([(v >> 16) as u8, (v >> 8) as u8, v as u8])
}

/// A 32x32 RGBA image: rounded accent tile with the BPM in white ("--" when `None`).
pub fn render_bpm_icon(bpm: Option<u16>, accent: [u8; 3]) -> Vec<u8> {
    let n = ICON_SIZE as i32;
    let mut px = vec![0u8; (n * n * 4) as usize];
    let radius = 7i32;
    let bg = if bpm.is_some() { accent } else { [0x2a, 0x2f, 0x3a] };
    // Rounded square background with 1px antialias-ish edge.
    for y in 0..n {
        for x in 0..n {
            let cx = x.clamp(radius, n - 1 - radius);
            let cy = y.clamp(radius, n - 1 - radius);
            let d2 = (x - cx).pow(2) + (y - cy).pow(2);
            let alpha = if d2 <= (radius - 1).pow(2) {
                255
            } else if d2 <= radius.pow(2) {
                140
            } else {
                0
            };
            if alpha > 0 {
                let i = ((y * n + x) * 4) as usize;
                px[i..i + 4].copy_from_slice(&[bg[0], bg[1], bg[2], alpha]);
            }
        }
    }
    // Charcoal digits on the pastel accent; light digits on the idle tile.
    let fg = if bpm.is_some() {
        [0x0e, 0x10, 0x14]
    } else {
        [0xe8, 0xe8, 0xed]
    };
    let glyphs: Vec<usize> = match bpm {
        Some(v) => v
            .min(999)
            .to_string()
            .bytes()
            .map(|b| (b - b'0') as usize)
            .collect(),
        None => vec![10, 10],
    };
    let scale: i32 = if glyphs.len() >= 3 { 3 } else { 4 };
    let gw = 3 * scale;
    let gap = if glyphs.len() >= 3 { 1 } else { 2 };
    let total_w = glyphs.len() as i32 * gw + (glyphs.len() as i32 - 1) * gap;
    let h = 5 * scale;
    let x0 = (n - total_w) / 2;
    let y0 = (n - h) / 2;
    for (gi, &g) in glyphs.iter().enumerate() {
        let gx = x0 + gi as i32 * (gw + gap);
        for (row, bits) in FONT[g].iter().enumerate() {
            for col in 0..3 {
                if bits & (0b100 >> col) == 0 {
                    continue;
                }
                for dy in 0..scale {
                    for dx in 0..scale {
                        let x = gx + col * scale + dx;
                        let y = y0 + row as i32 * scale + dy;
                        if (0..n).contains(&x) && (0..n).contains(&y) {
                            let i = ((y * n + x) * 4) as usize;
                            px[i..i + 4].copy_from_slice(&[fg[0], fg[1], fg[2], 255]);
                        }
                    }
                }
            }
        }
    }
    px
}

#[cfg(test)]
mod tests {
    use super::*;

    fn white_pixels(px: &[u8]) -> usize {
        // Counts digit pixels (charcoal on accent, light on idle).
        px.chunks(4)
            .filter(|p| p == &[0x0e, 0x10, 0x14, 255] || p == &[0xe8, 0xe8, 0xed, 255])
            .count()
    }

    #[test]
    fn renders_sizes() {
        for v in [None, Some(7), Some(72), Some(128), Some(5000)] {
            let px = render_bpm_icon(v, [255, 111, 168]);
            assert_eq!(px.len(), 32 * 32 * 4);
            assert!(white_pixels(&px) > 0, "{v:?}");
        }
        // "88" has more lit pixels than "11".
        assert!(
            white_pixels(&render_bpm_icon(Some(88), [0, 0, 0]))
                > white_pixels(&render_bpm_icon(Some(11), [0, 0, 0]))
        );
    }

    #[test]
    fn hex_parse() {
        assert_eq!(parse_hex("#ff6fa8"), Some([255, 111, 168]));
        assert_eq!(parse_hex("ff6fa8"), None);
        assert_eq!(parse_hex("#zzzzzz"), None);
        assert_eq!(accent_rgb("pink"), [0xff, 0xad, 0xc6]);
        assert_eq!(accent_rgb("sky"), [0xa8, 0xd1, 0xff]);
        assert_eq!(accent_rgb("#010203"), [1, 2, 3]);
    }
}
