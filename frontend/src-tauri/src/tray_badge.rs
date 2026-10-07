//! Tray icon status badges, drawn over the app icon at runtime so no extra
//! image assets have to be shipped or kept in sync with the UI.

/// What the tray badge shows, in priority order: recording beats listening.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayIndicator {
    /// Nothing to show: plain app icon.
    Idle,
    /// Automatic recording is waiting for speech (same glyph as the activity bar).
    Listening,
    /// A recording is in progress (red dot).
    Recording,
    /// The recording is paused (amber dot).
    Paused,
}

/// The tray is rendered at 16–32 px; 64 px keeps the badge crisp after scaling.
pub const SIZE: u32 = 64;

const WHITE: [u8; 3] = [255, 255, 255];
const BLUE: [u8; 3] = [37, 99, 235]; // Tailwind blue-600, as in the activity bar.
const RED: [u8; 3] = [239, 68, 68]; // red-500
const AMBER: [u8; 3] = [245, 158, 11]; // amber-500

/// Returns `SIZE`×`SIZE` RGBA pixels: the downscaled base icon plus the badge.
pub fn compose(base_rgba: &[u8], base_width: u32, base_height: u32, indicator: TrayIndicator) -> Vec<u8> {
    let mut canvas = downscale(base_rgba, base_width, base_height);
    if indicator == TrayIndicator::Idle {
        return canvas;
    }

    // Badge in the bottom-right corner, large enough to read at 16 px.
    let (cx, cy, radius) = (45.0, 45.0, 18.5);
    paint(&mut canvas, WHITE, |x, y| circle(x, y, cx, cy, radius));
    match indicator {
        TrayIndicator::Recording => paint(&mut canvas, RED, |x, y| circle(x, y, cx, cy, radius - 4.0)),
        TrayIndicator::Paused => paint(&mut canvas, AMBER, |x, y| circle(x, y, cx, cy, radius - 4.0)),
        TrayIndicator::Listening => {
            // Centre dot and two pairs of arcs: the "((•))" automatic-recording glyph.
            paint(&mut canvas, BLUE, |x, y| circle(x, y, cx, cy, 3.4));
            for arc_radius in [8.0, 13.5] {
                paint(&mut canvas, BLUE, |x, y| arc(x, y, cx, cy, arc_radius, 2.6));
            }
        }
        TrayIndicator::Idle => {}
    }
    canvas
}

/// Area-average downscale to `SIZE`×`SIZE` (the source is square app art).
fn downscale(rgba: &[u8], width: u32, height: u32) -> Vec<u8> {
    let mut out = vec![0u8; (SIZE * SIZE * 4) as usize];
    if width == 0 || height == 0 || rgba.len() < (width * height * 4) as usize {
        return out;
    }
    for oy in 0..SIZE {
        let y0 = oy * height / SIZE;
        let y1 = ((oy + 1) * height / SIZE).max(y0 + 1);
        for ox in 0..SIZE {
            let x0 = ox * width / SIZE;
            let x1 = ((ox + 1) * width / SIZE).max(x0 + 1);
            let mut sum = [0u64; 4];
            for y in y0..y1 {
                for x in x0..x1 {
                    let i = ((y * width + x) * 4) as usize;
                    let alpha = rgba[i + 3] as u64;
                    // Premultiply so transparent pixels don't darken edges.
                    for c in 0..3 {
                        sum[c] += rgba[i + c] as u64 * alpha;
                    }
                    sum[3] += alpha;
                }
            }
            let count = ((y1 - y0) * (x1 - x0)) as u64;
            let o = ((oy * SIZE + ox) * 4) as usize;
            if sum[3] > 0 {
                for c in 0..3 {
                    out[o + c] = (sum[c] / sum[3]) as u8;
                }
            }
            out[o + 3] = (sum[3] / count) as u8;
        }
    }
    out
}

/// Alpha-blend `color` over the canvas with per-pixel `coverage` in 0..=1.
fn paint(canvas: &mut [u8], color: [u8; 3], coverage: impl Fn(f32, f32) -> f32) {
    for y in 0..SIZE {
        for x in 0..SIZE {
            let a = coverage(x as f32 + 0.5, y as f32 + 0.5).clamp(0.0, 1.0);
            if a <= 0.0 {
                continue;
            }
            let i = ((y * SIZE + x) * 4) as usize;
            for c in 0..3 {
                canvas[i + c] = (color[c] as f32 * a + canvas[i + c] as f32 * (1.0 - a)).round() as u8;
            }
            canvas[i + 3] = (255.0 * a + canvas[i + 3] as f32 * (1.0 - a)).round() as u8;
        }
    }
}

/// Anti-aliased coverage of a filled circle.
fn circle(x: f32, y: f32, cx: f32, cy: f32, radius: f32) -> f32 {
    radius + 0.5 - ((x - cx).hypot(y - cy))
}

/// Anti-aliased coverage of the left and right ±45° arcs of a ring.
fn arc(x: f32, y: f32, cx: f32, cy: f32, radius: f32, width: f32) -> f32 {
    let (dx, dy) = (x - cx, y - cy);
    if dy.abs() > dx.abs() {
        return 0.0;
    }
    width / 2.0 + 0.5 - (dx.hypot(dy) - radius).abs()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pixel(canvas: &[u8], x: u32, y: u32) -> [u8; 4] {
        let i = ((y * SIZE + x) * 4) as usize;
        [canvas[i], canvas[i + 1], canvas[i + 2], canvas[i + 3]]
    }

    #[test]
    fn idle_is_the_plain_downscaled_icon() {
        let base = [10u8, 20, 30, 255].repeat(128 * 128);
        let canvas = compose(&base, 128, 128, TrayIndicator::Idle);
        assert_eq!(canvas.len(), (SIZE * SIZE * 4) as usize);
        assert_eq!(pixel(&canvas, 45, 45), [10, 20, 30, 255]);
    }

    #[test]
    fn badges_use_their_colours_at_the_centre_and_keep_the_corner_clear() {
        let base = [10u8, 20, 30, 255].repeat(32 * 32);
        let recording = compose(&base, 32, 32, TrayIndicator::Recording);
        assert_eq!(pixel(&recording, 45, 45), [239, 68, 68, 255]);
        assert_eq!(pixel(&recording, 5, 5), [10, 20, 30, 255]);
        let paused = compose(&base, 32, 32, TrayIndicator::Paused);
        assert_eq!(pixel(&paused, 45, 45), [245, 158, 11, 255]);
        let listening = compose(&base, 32, 32, TrayIndicator::Listening);
        assert_eq!(pixel(&listening, 45, 45), [37, 99, 235, 255]);
        // Between the dot and the first arc the badge background is white.
        assert_eq!(pixel(&listening, 45, 39), [255, 255, 255, 255]);
    }
}
