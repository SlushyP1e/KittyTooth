//! Draws the in-headset overlay image: the cat-head logo, the BPM and a small label, on a
//! rounded slate card (red for a high alert, amber for a low one).
//!
//! No font files are needed: digits and the few capital letters we show are drawn as rounded
//! strokes with signed-distance anti-aliasing, so the image stays crisp at any overlay size.
//! Colours mirror the dark theme in `src/tokens.css`.

use crate::alerts::AlertKind;

pub const WIDTH: u32 = 512;
pub const HEIGHT: u32 = 256;

const CARD: [u8; 4] = [0x0e, 0x11, 0x12, 240]; // --card
const PINK: [u8; 4] = [0xff, 0xad, 0xc6, 255]; // --pink
const TEXT: [u8; 4] = [0xe8, 0xe8, 0xed, 255]; // --text
const RED: [u8; 4] = [0xe0, 0x6c, 0x75, 250]; // --red
const AMBER: [u8; 4] = [0xd9, 0xa4, 0x41, 250]; // --amber
const INK: [u8; 4] = [0x09, 0x0b, 0x0c, 255]; // --slate-0, text on the alert colours

/// What the overlay shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct View {
    /// `None` shows "--".
    pub bpm: Option<u16>,
    pub alert: Option<AlertKind>,
    /// The alert came from the "Test alert" button.
    pub test: bool,
}

impl View {
    pub fn label(&self) -> &'static str {
        match (self.alert, self.test) {
            (Some(AlertKind::High), false) => "HIGH",
            (Some(AlertKind::Low), false) => "LOW",
            (Some(AlertKind::High), true) => "TEST HIGH",
            (Some(AlertKind::Low), true) => "TEST LOW",
            (None, _) => "BPM",
        }
    }
}

type Pt = (f32, f32);
type Path = Vec<Pt>;

/// Straight-alpha RGBA canvas.
struct Canvas {
    w: i32,
    h: i32,
    px: Vec<u8>,
}

impl Canvas {
    fn new(w: u32, h: u32) -> Self {
        Self {
            w: w as i32,
            h: h as i32,
            px: vec![0; (w * h * 4) as usize],
        }
    }

    /// Source-over blend of `color` at `coverage` (0..1) into a pixel.
    fn blend(&mut self, x: i32, y: i32, color: [u8; 4], coverage: f32) {
        let a = coverage.clamp(0.0, 1.0) * color[3] as f32 / 255.0;
        if a <= 0.0 {
            return;
        }
        let i = ((y * self.w + x) * 4) as usize;
        let d = &mut self.px[i..i + 4];
        let da = d[3] as f32 / 255.0;
        let oa = a + da * (1.0 - a);
        for c in 0..3 {
            let v = (color[c] as f32 * a + d[c] as f32 * da * (1.0 - a)) / oa;
            d[c] = v.round().clamp(0.0, 255.0) as u8;
        }
        d[3] = (oa * 255.0).round() as u8;
    }

    /// Filled rounded rectangle, anti-aliased.
    fn rounded_rect(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, r: f32, color: [u8; 4]) {
        let (cx, cy) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
        let (hx, hy) = ((x1 - x0) / 2.0 - r, (y1 - y0) / 2.0 - r);
        for y in (y0.floor() as i32).max(0)..(y1.ceil() as i32).min(self.h) {
            for x in (x0.floor() as i32).max(0)..(x1.ceil() as i32).min(self.w) {
                let qx = ((x as f32 + 0.5 - cx).abs() - hx).max(0.0);
                let qy = ((y as f32 + 0.5 - cy).abs() - hy).max(0.0);
                let d = (qx * qx + qy * qy).sqrt() - r;
                self.blend(x, y, color, 0.5 - d);
            }
        }
    }

    /// Strokes polylines with round caps and joins.
    fn stroke(&mut self, paths: &[Path], width: f32, color: [u8; 4]) {
        let segs: Vec<(Pt, Pt)> = paths
            .iter()
            .flat_map(|p| {
                if p.len() == 1 {
                    vec![(p[0], p[0])]
                } else {
                    p.windows(2).map(|w| (w[0], w[1])).collect()
                }
            })
            .collect();
        if segs.is_empty() {
            return;
        }
        let half = width / 2.0;
        let pad = half + 1.0;
        let (mut minx, mut miny, mut maxx, mut maxy) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        for &((ax, ay), (bx, by)) in &segs {
            minx = minx.min(ax).min(bx);
            miny = miny.min(ay).min(by);
            maxx = maxx.max(ax).max(bx);
            maxy = maxy.max(ay).max(by);
        }
        let xs = ((minx - pad).floor() as i32).max(0)..((maxx + pad).ceil() as i32).min(self.w);
        let ys = ((miny - pad).floor() as i32).max(0)..((maxy + pad).ceil() as i32).min(self.h);
        for y in ys {
            for x in xs.clone() {
                let p = (x as f32 + 0.5, y as f32 + 0.5);
                let d = segs
                    .iter()
                    .map(|&(a, b)| seg_dist(p, a, b))
                    .fold(f32::MAX, f32::min);
                let cov = half + 0.5 - d;
                if cov > 0.0 {
                    self.blend(x, y, color, cov);
                }
            }
        }
    }
}

fn seg_dist(p: Pt, a: Pt, b: Pt) -> f32 {
    let (vx, vy) = (b.0 - a.0, b.1 - a.1);
    let (wx, wy) = (p.0 - a.0, p.1 - a.1);
    let len2 = vx * vx + vy * vy;
    let t = if len2 > 0.0 {
        ((wx * vx + wy * vy) / len2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let (dx, dy) = (wx - t * vx, wy - t * vy);
    (dx * dx + dy * dy).sqrt()
}

/// Elliptical arc as a polyline. Angles in radians, y pointing down (0 = right, pi/2 = bottom).
fn arc(cx: f32, cy: f32, rx: f32, ry: f32, t0: f32, t1: f32) -> Path {
    let n = ((t1 - t0).abs() * 10.0).ceil().max(4.0) as usize;
    (0..=n)
        .map(|i| {
            let t = t0 + (t1 - t0) * i as f32 / n as f32;
            (cx + rx * t.cos(), cy + ry * t.sin())
        })
        .collect()
}

fn cubic(p0: Pt, c1: Pt, c2: Pt, p1: Pt) -> Path {
    (1..=12)
        .map(|i| {
            let t = i as f32 / 12.0;
            let u = 1.0 - t;
            let (a, b, c, d) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
            (
                a * p0.0 + b * c1.0 + c * c2.0 + d * p1.0,
                a * p0.1 + b * c1.1 + c * c2.1 + d * p1.1,
            )
        })
        .collect()
}

const PI: f32 = std::f32::consts::PI;

/// A glyph in a 16-unit-tall box: advance width and strokes.
fn glyph(c: char) -> Option<(f32, Vec<Path>)> {
    let l = |pts: &[Pt]| pts.to_vec();
    Some(match c {
        '0' => (10.0, vec![arc(5.0, 8.0, 5.0, 8.0, 0.0, 2.0 * PI)]),
        '1' => (10.0, vec![l(&[(2.0, 3.0), (5.5, 0.0), (5.5, 16.0)])]),
        '2' => {
            let mut p = arc(5.0, 5.0, 5.0, 5.0, PI, 2.0 * PI + 0.6);
            p.extend([(0.0, 16.0), (10.0, 16.0)]);
            (10.0, vec![p])
        }
        '3' => (
            10.0,
            vec![
                arc(5.0, 4.0, 4.5, 4.0, 1.15 * PI, 2.5 * PI),
                arc(5.0, 12.0, 5.0, 4.0, 1.5 * PI, 2.85 * PI),
            ],
        ),
        '4' => (
            10.0,
            vec![l(&[(7.5, 16.0), (7.5, 0.0), (0.0, 11.0), (10.0, 11.0)])],
        ),
        '5' => {
            let mut p = l(&[(9.5, 0.0), (1.2, 0.0), (0.9, 7.0)]);
            p.extend(arc(5.0, 10.5, 5.0, 5.5, 1.22 * PI, 2.85 * PI));
            (10.0, vec![p])
        }
        '6' => (
            10.0,
            vec![
                arc(5.0, 11.0, 5.0, 5.0, 0.0, 2.0 * PI),
                arc(10.0, 11.0, 10.0, 10.5, PI, 1.42 * PI),
            ],
        ),
        '7' => (10.0, vec![l(&[(0.0, 0.0), (10.0, 0.0), (3.5, 16.0)])]),
        '8' => (
            10.0,
            vec![
                arc(5.0, 4.0, 4.3, 4.0, 0.0, 2.0 * PI),
                arc(5.0, 12.0, 5.0, 4.0, 0.0, 2.0 * PI),
            ],
        ),
        '9' => {
            // A 6 turned upside down.
            let (_, paths) = glyph('6')?;
            let flip = |p: &Path| p.iter().map(|&(x, y)| (10.0 - x, 16.0 - y)).collect();
            (10.0, paths.iter().map(flip).collect())
        }
        '-' => (8.0, vec![l(&[(1.0, 8.0), (7.0, 8.0)])]),
        ' ' => (5.0, vec![]),
        'B' => {
            let mut top = l(&[(0.0, 8.0), (0.0, 0.0), (5.5, 0.0)]);
            top.extend(arc(5.5, 4.0, 4.0, 4.0, 1.5 * PI, 2.5 * PI));
            top.push((0.0, 8.0));
            let mut bottom = l(&[(0.0, 8.0), (6.0, 8.0)]);
            bottom.extend(arc(6.0, 12.0, 4.0, 4.0, 1.5 * PI, 2.5 * PI));
            bottom.extend([(0.0, 16.0), (0.0, 8.0)]);
            (10.0, vec![top, bottom])
        }
        'P' => {
            let mut p = l(&[(0.0, 16.0), (0.0, 0.0), (5.5, 0.0)]);
            p.extend(arc(5.5, 4.5, 4.5, 4.5, 1.5 * PI, 2.5 * PI));
            p.push((0.0, 9.0));
            (10.0, vec![p])
        }
        'M' => (
            12.0,
            vec![l(&[
                (0.0, 16.0),
                (0.0, 0.0),
                (6.0, 11.0),
                (12.0, 0.0),
                (12.0, 16.0),
            ])],
        ),
        'H' => (
            10.0,
            vec![
                l(&[(0.0, 0.0), (0.0, 16.0)]),
                l(&[(10.0, 0.0), (10.0, 16.0)]),
                l(&[(0.0, 8.0), (10.0, 8.0)]),
            ],
        ),
        'I' => (0.0, vec![l(&[(0.0, 0.0), (0.0, 16.0)])]),
        'G' => {
            // Counter-clockwise from the upper right, round to the right middle, then the bar.
            let mut p = arc(6.0, 8.0, 6.0, 8.0, 1.78 * PI, 0.0);
            p.push((7.0, 8.0));
            (12.0, vec![p])
        }
        'L' => (9.0, vec![l(&[(0.0, 0.0), (0.0, 16.0), (9.0, 16.0)])]),
        'O' => (12.0, vec![arc(6.0, 8.0, 6.0, 8.0, 0.0, 2.0 * PI)]),
        'W' => (
            14.0,
            vec![l(&[
                (0.0, 0.0),
                (3.5, 16.0),
                (7.0, 4.0),
                (10.5, 16.0),
                (14.0, 0.0),
            ])],
        ),
        'T' => (
            10.0,
            vec![l(&[(0.0, 0.0), (10.0, 0.0)]), l(&[(5.0, 0.0), (5.0, 16.0)])],
        ),
        'E' => (
            9.0,
            vec![
                l(&[(9.0, 0.0), (0.0, 0.0), (0.0, 16.0), (9.0, 16.0)]),
                l(&[(0.0, 8.0), (7.5, 8.0)]),
            ],
        ),
        'S' => {
            let mut p = arc(5.0, 4.0, 4.6, 4.0, 1.85 * PI, 0.5 * PI);
            p.extend(arc(5.0, 12.0, 5.0, 4.0, 1.5 * PI, 2.85 * PI));
            (10.0, vec![p])
        }
        _ => return None,
    })
}

/// Lays out `text` with its top-left at (x, y), `height` px tall. Returns strokes and width.
fn text_paths(text: &str, height: f32, x: f32, y: f32) -> (Vec<Path>, f32) {
    let s = height / 16.0;
    let gap = 3.5;
    let mut pen = 0.0f32;
    let mut out = Vec::new();
    for (i, c) in text.chars().enumerate() {
        let Some((adv, paths)) = glyph(c) else { continue };
        if i > 0 {
            pen += gap;
        }
        out.extend(paths.into_iter().map(|p| {
            p.into_iter()
                .map(|(gx, gy)| (x + (pen + gx) * s, y + gy * s))
                .collect()
        }));
        pen += adv;
    }
    (out, pen * s)
}

/// Width of `text` at `height` px.
fn text_width(text: &str, height: f32) -> f32 {
    text_paths(text, height, 0.0, 0.0).1
}

/// The cat-head outline from the app logo (24-unit box, see `Logo.svelte`).
fn cat_head() -> Path {
    let mut p: Path = vec![(4.2, 8.4), (4.6, 3.4), (8.3, 6.1)];
    // The two arcs between the ears (radius 5.6), centres worked out from the SVG path.
    p.extend(arc(
        7.51,
        11.64,
        5.6,
        5.6,
        (-81.9f32).to_radians(),
        (-36.6f32).to_radians(),
    ));
    p.extend(arc(
        16.49,
        11.64,
        5.6,
        5.6,
        (-143.4f32).to_radians(),
        (-98.1f32).to_radians(),
    ));
    p.extend([(19.4, 3.4), (19.8, 8.4)]);
    p.extend(cubic((19.8, 8.4), (21.3, 11.2), (19.7, 14.2), (17.4, 16.4)));
    p.extend(cubic((17.4, 16.4), (15.6, 18.2), (12.0, 20.5), (12.0, 20.5)));
    p.extend(cubic((12.0, 20.5), (12.0, 20.5), (8.4, 18.2), (6.6, 16.4)));
    p.extend(cubic((6.6, 16.4), (4.3, 14.2), (2.7, 11.2), (4.2, 8.4)));
    p
}

/// Renders the overlay as `WIDTH` x `HEIGHT` straight-alpha RGBA.
pub fn render(view: &View) -> Vec<u8> {
    let mut c = Canvas::new(WIDTH, HEIGHT);
    let (w, h) = (WIDTH as f32, HEIGHT as f32);
    let (bg, accent, digits) = match view.alert {
        Some(AlertKind::High) => (RED, INK, INK),
        Some(AlertKind::Low) => (AMBER, INK, INK),
        None => (CARD, PINK, TEXT),
    };
    c.rounded_rect(4.0, 4.0, w - 4.0, h - 4.0, 48.0, bg);
    if view.alert.is_none() {
        // A faint pink edge, like the app's cards.
        let edge = [PINK[0], PINK[1], PINK[2], 70];
        let inner = CARD;
        c.rounded_rect(4.0, 4.0, w - 4.0, h - 4.0, 48.0, edge);
        c.rounded_rect(
            7.0,
            7.0,
            w - 7.0,
            h - 7.0,
            45.0,
            [inner[0], inner[1], inner[2], 255],
        );
    }

    // Logo, left.
    let scale = 7.2;
    let (ox, oy) = (118.0 - 12.0 * scale, 128.0 - 12.0 * scale);
    let logo: Path = cat_head()
        .into_iter()
        .map(|(x, y)| (ox + x * scale, oy + y * scale))
        .collect();
    c.stroke(&[logo], 1.8 * scale, accent);

    // BPM, right, centred in the space next to the logo.
    let (left, right) = (222.0, w - 30.0);
    let mid = (left + right) / 2.0;
    let text = match view.bpm {
        Some(v) => v.min(999).to_string(),
        None => "--".into(),
    };
    let dh = 100.0;
    let dw = text_width(&text, dh);
    let (paths, _) = text_paths(&text, dh, mid - dw / 2.0, 52.0);
    c.stroke(&paths, 15.0, digits);

    let label = view.label();
    let lh = 28.0;
    let lw = text_width(label, lh);
    let (paths, _) = text_paths(label, lh, mid - lw / 2.0, 180.0);
    c.stroke(&paths, 5.5, accent);
    c.px
}

#[cfg(test)]
mod tests {
    use super::*;

    fn count(px: &[u8], rgb: [u8; 3]) -> usize {
        px.chunks(4).filter(|p| p[..3] == rgb && p[3] > 200).count()
    }

    #[test]
    fn every_shown_char_has_a_glyph() {
        for s in ["0123456789", "--", "BPM", "HIGH", "LOW", "TEST HIGH", "TEST LOW"] {
            for ch in s.chars() {
                assert!(glyph(ch).is_some(), "{ch}");
            }
        }
    }

    #[test]
    fn renders_normal_card() {
        let px = render(&View {
            bpm: Some(128),
            ..Default::default()
        });
        assert_eq!(px.len(), (WIDTH * HEIGHT * 4) as usize);
        // Corners stay transparent (rounded card), the middle is the card colour.
        assert_eq!(px[3], 0);
        assert!(
            count(&px, [PINK[0], PINK[1], PINK[2]]) > 2000,
            "logo and label in pink"
        );
        assert!(count(&px, [TEXT[0], TEXT[1], TEXT[2]]) > 4000, "digits in white");
        assert!(count(&px, [CARD[0], CARD[1], CARD[2]]) > 40_000);
    }

    #[test]
    fn alerts_change_the_card_colour() {
        let high = render(&View {
            bpm: Some(162),
            alert: Some(AlertKind::High),
            test: false,
        });
        let low = render(&View {
            bpm: Some(41),
            alert: Some(AlertKind::Low),
            test: false,
        });
        assert!(count(&high, [RED[0], RED[1], RED[2]]) > 40_000);
        assert!(count(&low, [AMBER[0], AMBER[1], AMBER[2]]) > 40_000);
        assert!(count(&high, [INK[0], INK[1], INK[2]]) > 3000, "dark text on red");
        assert_eq!(count(&high, [CARD[0], CARD[1], CARD[2]]), 0);
    }

    #[test]
    fn digits_differ_and_fit() {
        let a = render(&View {
            bpm: Some(111),
            ..Default::default()
        });
        let b = render(&View {
            bpm: Some(188),
            ..Default::default()
        });
        let white = [TEXT[0], TEXT[1], TEXT[2]];
        assert!(count(&b, white) > count(&a, white));
        // The widest strings stay inside the space right of the logo.
        assert!(text_width("188", 100.0) + 15.0 < (WIDTH as f32 - 30.0) - 222.0);
        assert!(text_width("TEST HIGH", 28.0) + 6.0 < (WIDTH as f32 - 30.0) - 222.0);
        let none = render(&View::default());
        assert!(count(&none, white) > 0, "-- when there is no reading");
    }

    /// `KT_DUMP_OVERLAY=<dir> cargo test -p kittytooth-core dump_overlay -- --ignored` writes raw
    /// RGBA frames (512x256) for a visual check.
    #[test]
    #[ignore]
    fn dump_overlay() {
        let Some(dir) = std::env::var_os("KT_DUMP_OVERLAY") else {
            return;
        };
        let dir = std::path::PathBuf::from(dir);
        for (name, view) in [
            (
                "normal",
                View {
                    bpm: Some(128),
                    ..Default::default()
                },
            ),
            ("none", View::default()),
            (
                "high",
                View {
                    bpm: Some(162),
                    alert: Some(AlertKind::High),
                    test: false,
                },
            ),
            (
                "low",
                View {
                    bpm: Some(41),
                    alert: Some(AlertKind::Low),
                    test: true,
                },
            ),
            (
                "digits",
                View {
                    bpm: Some(90),
                    ..Default::default()
                },
            ),
            (
                "d345",
                View {
                    bpm: Some(345),
                    ..Default::default()
                },
            ),
            (
                "d670",
                View {
                    bpm: Some(670),
                    ..Default::default()
                },
            ),
        ] {
            std::fs::write(dir.join(format!("{name}.rgba")), render(&view)).unwrap();
        }
    }

    #[test]
    fn labels() {
        assert_eq!(View::default().label(), "BPM");
        let v = View {
            alert: Some(AlertKind::Low),
            test: true,
            bpm: None,
        };
        assert_eq!(v.label(), "TEST LOW");
    }
}
