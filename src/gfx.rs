//! Software drawing into the 426x266 RGB565 canvas, as design/engine.js
//! draws: pixels, boxes, lines, discs and rings, blends, indexed sprites
//! with flash/tint/dither, the 5x7 font and the window frame. Everything clips
//! to the canvas and the clip box.

use crate::art::{FONT, FONT_HAS, Img, c};

/// The canvas, in its own pixels; the window shows it at a whole-number scale.
pub const W: usize = 426;
pub const H: usize = 266;

/// RGB565 from 0xRRGGBB, at compile time.
pub const fn hex(v: u32) -> u16 {
    ((((v >> 19) & 31) << 11) | (((v >> 10) & 63) << 5) | ((v >> 3) & 31)) as u16
}

/// RGB565 to 0x00RRGGBB, each channel's top bits repeated into the bottom.
pub fn rgb(v: u16) -> u32 {
    let (r, g, b) = ((v >> 11) as u32, ((v >> 5) & 63) as u32, (v & 31) as u32);
    ((r << 3 | r >> 2) << 16) | ((g << 2 | g >> 4) << 8) | (b << 3 | b >> 2)
}

pub const BAYER: [u8; 16] = [0, 8, 2, 10, 12, 4, 14, 6, 3, 11, 1, 9, 15, 7, 13, 5];

/// Blend a towards b by k / 256.
pub fn mix(a: u16, b: u16, k: i32) -> u16 {
    if k <= 0 {
        return a;
    }
    if k >= 256 {
        return b;
    }
    let (a, b) = (a as i32, b as i32);
    let r = (a >> 11) + ((((b >> 11) - (a >> 11)) * k) >> 8);
    let g = ((a >> 5) & 63) + (((((b >> 5) & 63) - ((a >> 5) & 63)) * k) >> 8);
    let bl = (a & 31) + ((((b & 31) - (a & 31)) * k) >> 8);
    ((r << 11) | (g << 5) | bl) as u16
}

/// How to draw a sprite. white and tint_k go 0..256; vis 0..1 dithers it
/// away; sweep (>= 0) dissolves it from the bottom up as it grows to 1.
#[derive(Clone, Copy, Debug)]
pub struct Fx {
    pub flip: bool,
    pub white: i32,
    pub tint: u16,
    pub tint_k: i32,
    pub vis: f32,
    pub sweep: f32,
}

impl Default for Fx {
    fn default() -> Self {
        Fx { flip: false, white: 0, tint: 0, tint_k: 0, vis: 1.0, sweep: -1.0 }
    }
}

fn glyph(ch: u8) -> &'static [u8; 7] {
    let mut c = ch as usize;
    if !(32..=127).contains(&c) || !FONT_HAS[c - 32] {
        c = b'?' as usize;
    }
    &FONT[c - 32]
}

/// 5x7 text is 6 px a character at scale 1.
pub fn text_width(s: &str, k: i32) -> i32 {
    let n = s.len() as i32;
    if n > 0 { (n * 6 - 1) * k } else { 0 }
}

pub struct Gfx<'a> {
    fb: &'a mut [u16],
    cx0: i32,
    cy0: i32,
    cx1: i32,
    cy1: i32,
}

impl<'a> Gfx<'a> {
    pub fn new(fb: &'a mut [u16]) -> Self {
        assert_eq!(fb.len(), W * H);
        Gfx { fb, cx0: 0, cy0: 0, cx1: W as i32, cy1: H as i32 }
    }

    /// Half-open box.
    pub fn clip(&mut self, x0: i32, y0: i32, x1: i32, y1: i32) {
        self.cx0 = x0.max(0);
        self.cy0 = y0.max(0);
        self.cx1 = x1.min(W as i32);
        self.cy1 = y1.min(H as i32);
    }

    pub fn noclip(&mut self) {
        self.clip(0, 0, W as i32, H as i32);
    }

    fn in_clip(&self, x: i32, y: i32) -> bool {
        x >= self.cx0 && y >= self.cy0 && x < self.cx1 && y < self.cy1
    }

    pub fn fb(&mut self) -> &mut [u16] {
        self.fb
    }

    pub fn px(&mut self, x: i32, y: i32, c: u16) {
        if self.in_clip(x, y) {
            self.fb[y as usize * W + x as usize] = c;
        }
    }

    pub fn rect(&mut self, x: i32, y: i32, w: i32, h: i32, c: u16) {
        let (x0, y0) = (x.max(self.cx0), y.max(self.cy0));
        let (x1, y1) = ((x + w).min(self.cx1), (y + h).min(self.cy1));
        for j in y0..y1 {
            let row = j as usize * W;
            for i in x0..x1 {
                self.fb[row + i as usize] = c;
            }
        }
    }

    /// Blend one pixel towards c by k / 256.
    pub fn blend(&mut self, x: i32, y: i32, c: u16, k: i32) {
        if self.in_clip(x, y) {
            let p = &mut self.fb[y as usize * W + x as usize];
            *p = mix(*p, c, k);
        }
    }

    pub fn blend_rect(&mut self, x: i32, y: i32, w: i32, h: i32, c: u16, k: i32) {
        let (x0, y0) = (x.max(self.cx0), y.max(self.cy0));
        let (x1, y1) = ((x + w).min(self.cx1), (y + h).min(self.cy1));
        for j in y0..y1 {
            let row = j as usize * W;
            for i in x0..x1 {
                let p = &mut self.fb[row + i as usize];
                *p = mix(*p, c, k);
            }
        }
    }

    /// A line wdt px thick (a square pen).
    pub fn line(&mut self, mut x0: i32, mut y0: i32, x1: i32, y1: i32, c: u16, wdt: i32) {
        let dx = (x1 - x0).abs();
        let dy = -(y1 - y0).abs();
        let sx = if x0 < x1 { 1 } else { -1 };
        let sy = if y0 < y1 { 1 } else { -1 };
        let mut e = dx + dy;
        for _ in 0..2000 {
            if wdt <= 1 {
                self.px(x0, y0, c);
            } else {
                self.rect(x0 - (wdt >> 1), y0 - (wdt >> 1), wdt, wdt, c);
            }
            if x0 == x1 && y0 == y1 {
                break;
            }
            let e2 = 2 * e;
            if e2 >= dy {
                e += dy;
                x0 += sx;
            }
            if e2 <= dx {
                e += dx;
                y0 += sy;
            }
        }
    }

    pub fn disc(&mut self, cx: f32, cy: f32, r: i32, c: u16) {
        if r < 0 {
            return;
        }
        let lim = (r * r) as f32 + r as f32 * 0.6;
        for j in -r..=r {
            for i in -r..=r {
                if ((i * i + j * j) as f32) <= lim {
                    self.px((cx + i as f32) as i32, (cy + j as f32) as i32, c);
                }
            }
        }
    }

    /// A disc drawn only where the 4x4 Bayer matrix is under vis (0..1).
    pub fn disc_dither(&mut self, cx: f32, cy: f32, r: i32, c: u16, vis: f32) {
        if r < 0 {
            return;
        }
        let lim = (r * r) as f32 + r as f32 * 0.6;
        for j in -r..=r {
            for i in -r..=r {
                if ((i * i + j * j) as f32) > lim {
                    continue;
                }
                let (x, y) = ((cx + i as f32) as i32, (cy + j as f32) as i32);
                if vis < 1.0 && BAYER[((y & 3) * 4 + (x & 3)) as usize] as f32 >= vis * 16.0 {
                    continue;
                }
                self.px(x, y, c);
            }
        }
    }

    /// An ellipse outline of dots.
    pub fn ring(&mut self, cx: f32, cy: f32, rx: f32, ry: f32, c: u16) {
        let n = (((rx + ry) * 4.0) as i32).max(12);
        for k in 0..n {
            let a = k as f32 / n as f32 * std::f32::consts::TAU;
            self.px((cx + a.cos() * rx) as i32, (cy + a.sin() * ry) as i32, c);
        }
    }

    /// A filled ellipse darkened by k / 256 (shadows).
    pub fn shade(&mut self, cx: i32, cy: i32, rx: i32, ry: i32, k: i32) {
        if rx <= 0 || ry <= 0 {
            return;
        }
        for j in -ry..=ry {
            for i in -rx..=rx {
                if (i * i) as f32 / (rx * rx) as f32 + (j * j) as f32 / (ry * ry) as f32 <= 1.0 {
                    self.blend(cx + i, cy + j, 0, k);
                }
            }
        }
    }

    pub fn blit(&mut self, img: &Img, x: i32, y: i32, fx: Option<&Fx>) {
        let mut pal = [0u16; 256];
        let mut p: &[u16] = img.pal;
        if let Some(fx) = fx
            && (fx.white > 0 || fx.tint_k > 0)
        {
            for (i, slot) in pal.iter_mut().enumerate().take(img.pal.len()).skip(1) {
                let mut c = img.pal[i];
                if fx.tint_k > 0 {
                    c = mix(c, fx.tint, fx.tint_k);
                }
                if fx.white > 0 {
                    c = mix(c, 0xffff, fx.white);
                }
                *slot = c;
            }
            p = &pal;
        }
        let flip = fx.is_some_and(|f| f.flip);
        let (w, h) = (img.w as i32, img.h as i32);
        let px = img.px();
        for j in 0..h {
            let yy = y + j;
            if yy < self.cy0 || yy >= self.cy1 {
                continue;
            }
            let mut vis = fx.map_or(1.0, |f| f.vis);
            if let Some(f) = fx
                && f.sweep >= 0.0
            {
                let v = ((1.0 - f.sweep) * 2.2 - j as f32 / h as f32 * 1.2).clamp(0.0, 1.0);
                if v < vis {
                    vis = v;
                }
            }
            if vis <= 0.0 {
                continue;
            }
            let row = &px[(j * w) as usize..((j + 1) * w) as usize];
            let out = yy as usize * W;
            for i in 0..w {
                let v = row[if flip { w - 1 - i } else { i } as usize];
                if v == 0 {
                    continue;
                }
                let xx = x + i;
                if xx < self.cx0 || xx >= self.cx1 {
                    continue;
                }
                if vis < 1.0 && BAYER[((yy & 3) * 4 + (xx & 3)) as usize] as f32 >= vis * 16.0 {
                    continue;
                }
                self.fb[out + xx as usize] = p[v as usize];
            }
        }
    }

    /// k-times enlarged, no effects.
    pub fn blit_scaled(&mut self, img: &Img, x: i32, y: i32, k: i32, flip: bool) {
        let (w, h) = (img.w as i32, img.h as i32);
        let px = img.px();
        for j in 0..h {
            for i in 0..w {
                let v = px[(j * w + if flip { w - 1 - i } else { i }) as usize];
                if v != 0 {
                    self.rect(x + i * k, y + j * k, k, k, img.pal[v as usize]);
                }
            }
        }
    }

    /// 5x7 text, 6 px advance per character at scale k. shadow: a colour
    /// drawn 1 px down-right first.
    pub fn text(&mut self, s: &str, x: i32, y: i32, c: u16, k: i32, shadow: Option<u16>) {
        let passes: &[(u16, i32)] = match shadow {
            Some(sh) => &[(sh, 1), (c, 0)],
            None => &[(c, 0)],
        };
        for &(col, o) in passes {
            for (n, ch) in s.bytes().enumerate() {
                let g = glyph(ch);
                let n = n as i32;
                for (j, bits) in g.iter().enumerate() {
                    for i in 0..5 {
                        if (bits >> (4 - i)) & 1 == 0 {
                            continue;
                        }
                        if k == 1 {
                            self.px(x + n * 6 + i + o, y + j as i32 + o, col);
                        } else {
                            self.rect(x + (n * 6 + i) * k + o, y + j as i32 * k + o, k, k, col);
                        }
                    }
                }
            }
        }
    }

    /// Right-aligned: the last character ends at xr.
    pub fn text_r(&mut self, s: &str, xr: i32, y: i32, c: u16, shadow: Option<u16>) {
        self.text(s, xr - s.len() as i32 * 6 + 1, y, c, 1, shadow);
    }

    /// A damage number: outlined all round, centred on cx, k times enlarged.
    pub fn num(&mut self, s: &str, cx: i32, y: i32, c: u16, k: i32) {
        let x0 = cx - text_width(s, k) / 2;
        for pass in 0..2 {
            for (n, ch) in s.bytes().enumerate() {
                let g = glyph(ch);
                let x = x0 + n as i32 * 6 * k;
                for (j, bits) in g.iter().enumerate() {
                    let j = j as i32;
                    for i in 0..5 {
                        if (bits >> (4 - i)) & 1 == 0 {
                            continue;
                        }
                        if pass == 1 {
                            self.rect(x + i * k, y + j * k, k, k, c);
                        } else {
                            self.rect(x + i * k - 1, y + j * k - 1, k + 2, k + 2, c::OUTER);
                        }
                    }
                }
            }
        }
    }

    /// A window: dark rim, white border with clipped corners, banded blue fill.
    pub fn window(&mut self, x: i32, y: i32, w: i32, h: i32) {
        self.rect(x + 2, y, w - 4, h, c::OUTER);
        self.rect(x, y + 2, w, h - 4, c::OUTER);
        self.rect(x + 1, y + 1, w - 2, h - 2, c::OUTER);
        self.rect(x + 2, y + 1, w - 4, h - 2, c::BORDER);
        self.rect(x + 1, y + 2, w - 2, h - 4, c::BORDER);
        self.rect(x + 3, y + 3, w - 6, h - 6, c::BORDER_LO);
        let (ih, bands) = (h - 8, 8);
        for j in 0..ih {
            let band = j * bands / ih;
            self.rect(x + 4, y + 4 + j, w - 8, 1, mix(c::WIN0, c::WIN1, band * 256 / (bands - 1)));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colours_round_trip() {
        assert_eq!(hex(0xffffff), 0xffff);
        assert_eq!(rgb(0xffff), 0xffffff);
        assert_eq!(rgb(hex(0xf8fcf8)), 0xffffff);
        assert_eq!(rgb(hex(0xf8f8f8)), 0xfffbff);
        assert_eq!(mix(0x0000, 0xffff, 128), (15 << 11) | (31 << 5) | 15);
    }

    #[test]
    fn drawing_clips_to_the_canvas() {
        let mut fb = vec![0u16; W * H];
        let mut g = Gfx::new(&mut fb);
        g.rect(-5, -5, 10, 10, 1);
        g.line(-20, 300, 500, -40, 2, 3);
        g.disc(420.0, 260.0, 12, 3);
        g.text("HELLO", 410, 262, 4, 2, Some(5));
        assert_eq!(fb[0], 1);
        assert_eq!(fb[5 * W + 5], 0);
    }
}
