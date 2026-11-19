//! TStudio logo: an orange "T" whose crossbar is a row of meter/waveform bars, on a dark rounded
//! square. One geometry (256×256 units) feeds the window icon, the .exe icon (build.rs), the
//! menu-bar logo and `assets/logo.svg`. No dependencies so build.rs can include this file.
#![allow(dead_code)]

/// Background corner radius.
pub const BG_RADIUS: f32 = 56.0;
/// Background gradient (top, bottom).
pub const BG_TOP: [u8; 3] = [38, 42, 49];
pub const BG_BOTTOM: [u8; 3] = [20, 22, 26];
/// Foreground gradient (top, bottom) – TStudio's accent orange.
pub const FG_TOP: [u8; 3] = [255, 178, 80];
pub const FG_BOTTOM: [u8; 3] = [242, 112, 40];

/// Crossbar bars: (center x, height); centred on y = `BAR_Y`.
pub const BARS: [(f32, f32); 9] = [(56.0, 18.0), (74.0, 32.0), (92.0, 44.0), (110.0, 54.0), (128.0, 60.0), (146.0, 54.0), (164.0, 44.0), (182.0, 32.0), (200.0, 18.0)];
pub const BAR_W: f32 = 14.0;
pub const BAR_Y: f32 = 74.0;
/// The stem of the T: x0, y0, x1, y1.
pub const STEM: [f32; 4] = [109.0, 74.0, 147.0, 208.0];

/// All foreground shapes as rounded rects (x0, y0, x1, y1, radius) in 256-unit space.
pub fn shapes() -> Vec<[f32; 5]> {
    let mut v: Vec<[f32; 5]> = BARS.iter().map(|&(cx, h)| [cx - BAR_W / 2.0, BAR_Y - h / 2.0, cx + BAR_W / 2.0, BAR_Y + h / 2.0, 5.0]).collect();
    v.push([STEM[0], STEM[1], STEM[2], STEM[3], 11.0]);
    v
}

fn in_rrect(x: f32, y: f32, r: [f32; 5]) -> bool {
    let [x0, y0, x1, y1, rad] = r;
    if x < x0 || x > x1 || y < y0 || y > y1 {
        return false;
    }
    let cx = x.clamp(x0 + rad, x1 - rad);
    let cy = y.clamp(y0 + rad, y1 - rad);
    (x - cx).powi(2) + (y - cy).powi(2) <= rad * rad
}

fn lerp(a: [u8; 3], b: [u8; 3], t: f32) -> [f32; 3] {
    [0, 1, 2].map(|i| a[i] as f32 + (b[i] as f32 - a[i] as f32) * t)
}

/// Renders the logo as straight (non-premultiplied) RGBA, `size`×`size`, 4×4 supersampled.
pub fn rgba(size: u32) -> Vec<u8> {
    let shapes = shapes();
    let bg = [0.0, 0.0, 256.0, 256.0, BG_RADIUS];
    let ss = 4;
    let mut out = Vec::with_capacity((size * size * 4) as usize);
    let scale = 256.0 / size as f32;
    for py in 0..size {
        for px in 0..size {
            let mut acc = [0.0f32; 4]; // premultiplied
            for sy in 0..ss {
                for sx in 0..ss {
                    let x = (px as f32 + (sx as f32 + 0.5) / ss as f32) * scale;
                    let y = (py as f32 + (sy as f32 + 0.5) / ss as f32) * scale;
                    let t = y / 256.0;
                    let c = if shapes.iter().any(|s| in_rrect(x, y, *s)) {
                        lerp(FG_TOP, FG_BOTTOM, ((y - 40.0) / 170.0).clamp(0.0, 1.0))
                    } else if in_rrect(x, y, bg) {
                        lerp(BG_TOP, BG_BOTTOM, t)
                    } else {
                        continue;
                    };
                    acc[0] += c[0];
                    acc[1] += c[1];
                    acc[2] += c[2];
                    acc[3] += 1.0;
                }
            }
            if acc[3] == 0.0 {
                out.extend_from_slice(&[0, 0, 0, 0]);
            } else {
                let a = acc[3] / (ss * ss) as f32;
                out.extend_from_slice(&[(acc[0] / acc[3]) as u8, (acc[1] / acc[3]) as u8, (acc[2] / acc[3]) as u8, (a * 255.0).round() as u8]);
            }
        }
    }
    out
}

/// Windows .ico with 32-bit BMP entries for the given sizes (each ≤ 256).
pub fn ico(sizes: &[u32]) -> Vec<u8> {
    let mut images: Vec<Vec<u8>> = Vec::new();
    for &s in sizes {
        let px = rgba(s);
        let mut d = Vec::new();
        // BITMAPINFOHEADER (height doubled: XOR + AND mask)
        d.extend_from_slice(&40u32.to_le_bytes());
        d.extend_from_slice(&(s as i32).to_le_bytes());
        d.extend_from_slice(&((s * 2) as i32).to_le_bytes());
        d.extend_from_slice(&1u16.to_le_bytes());
        d.extend_from_slice(&32u16.to_le_bytes());
        d.extend_from_slice(&[0u8; 24]);
        for y in (0..s).rev() {
            for x in 0..s {
                let i = ((y * s + x) * 4) as usize;
                d.extend_from_slice(&[px[i + 2], px[i + 1], px[i], px[i + 3]]);
            }
        }
        let mask_row = ((s + 31) / 32 * 4) as usize;
        d.extend(std::iter::repeat(0u8).take(mask_row * s as usize));
        images.push(d);
    }
    let mut f = Vec::new();
    f.extend_from_slice(&[0, 0, 1, 0]);
    f.extend_from_slice(&(sizes.len() as u16).to_le_bytes());
    let mut offset = 6 + 16 * sizes.len() as u32;
    for (&s, d) in sizes.iter().zip(&images) {
        let b = if s >= 256 { 0 } else { s as u8 };
        f.extend_from_slice(&[b, b, 0, 0]);
        f.extend_from_slice(&1u16.to_le_bytes());
        f.extend_from_slice(&32u16.to_le_bytes());
        f.extend_from_slice(&(d.len() as u32).to_le_bytes());
        f.extend_from_slice(&offset.to_le_bytes());
        offset += d.len() as u32;
    }
    for d in images {
        f.extend(d);
    }
    f
}

/// The same logo as SVG (for README/assets).
pub fn svg() -> String {
    let hex = |c: [u8; 3]| format!("#{:02X}{:02X}{:02X}", c[0], c[1], c[2]);
    let mut s = String::new();
    s.push_str("<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 256 256\" width=\"256\" height=\"256\">\n");
    s.push_str(&format!(
        "  <defs>\n    <linearGradient id=\"bg\" x1=\"0\" y1=\"0\" x2=\"0\" y2=\"1\"><stop offset=\"0\" stop-color=\"{}\"/><stop offset=\"1\" stop-color=\"{}\"/></linearGradient>\n    <linearGradient id=\"fg\" gradientUnits=\"userSpaceOnUse\" x1=\"0\" y1=\"40\" x2=\"0\" y2=\"210\"><stop offset=\"0\" stop-color=\"{}\"/><stop offset=\"1\" stop-color=\"{}\"/></linearGradient>\n  </defs>\n",
        hex(BG_TOP),
        hex(BG_BOTTOM),
        hex(FG_TOP),
        hex(FG_BOTTOM)
    ));
    s.push_str(&format!("  <rect width=\"256\" height=\"256\" rx=\"{BG_RADIUS}\" fill=\"url(#bg)\"/>\n  <g fill=\"url(#fg)\">\n"));
    for [x0, y0, x1, y1, r] in shapes() {
        s.push_str(&format!("    <rect x=\"{x0}\" y=\"{y0}\" width=\"{}\" height=\"{}\" rx=\"{r}\"/>\n", x1 - x0, y1 - y0));
    }
    s.push_str("  </g>\n</svg>\n");
    s
}

#[cfg(test)]
mod tests {
    #[test]
    fn logo_renders_and_ico_is_valid() {
        let px = super::rgba(32);
        assert_eq!(px.len(), 32 * 32 * 4);
        assert_eq!(px[3], 0, "corner is transparent");
        // middle of the stem is opaque orange
        let i = ((24 * 32 + 16) * 4) as usize;
        assert!(px[i] > 200 && px[i + 3] == 255);
        let ico = super::ico(&[16, 32]);
        assert_eq!(&ico[..6], &[0, 0, 1, 0, 2, 0]);
        assert!(super::svg().contains("<rect"));
    }
}

#[cfg(test)]
mod export {
    /// `cargo test -p tstudio write_logo_assets -- --ignored` regenerates assets/logo.svg + logo.ico
    #[test]
    #[ignore]
    fn write_logo_assets() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("logo.svg"), super::svg()).unwrap();
        std::fs::write(dir.join("logo.ico"), super::ico(&[16, 32, 48, 256])).unwrap();
    }
}
