//! The effects, drawn: spells, specials, the monsters' attacks and
//! finishers, the final blows. Each draws from its own seed, so it looks
//! the same every frame of its life.

use super::*;
use crate::art::{ICON_FEATHER, ICON_ROCK, ICON_SKULL};
use crate::gfx::{BAYER, Gfx, mix};

use std::f32::consts::PI;

fn flame(g: &mut Gfx, cx: f32, by: f32, h: i32, seed: i32) {
    const COLS: [u16; 4] = [hex(0xa82018), hex(0xe8641c), hex(0xf8c840), hex(0xfff4c0)];
    let mut r = Lcg(seed as u32);
    for k in 0..h {
        let t = k as f32 / h as f32;
        let hw = 0f32.max((PI * (1.0 - t).powf(0.8)).sin() * h as f32 * 0.32 + (r.next() - 0.5) * 1.6);
        let sway = (t * 4.0 + seed as f32).sin() * t * 2.0;
        let n = hw.ceil() as i32;
        for i in -n..=n {
            let e = (i as f32).abs() / (hw + 0.01);
            let lvl = (((1.0 - e) * 3.2 - t * 1.5 + 0.6).floor() as i32).clamp(0, 3);
            g.px((cx + i as f32 + sway) as i32, (by - k as f32) as i32, COLS[lvl as usize]);
        }
    }
}

fn rays(g: &mut Gfx, cx: f32, cy: f32, r0: f32, r1: f32, rot: f32, c: u16) {
    for a in 0..8 {
        let an = a as f32 / 8.0 * 2.0 * PI + rot;
        g.line(
            (cx + an.cos() * r0) as i32,
            (cy + an.sin() * r0) as i32,
            (cx + an.cos() * r1) as i32,
            (cy + an.sin() * r1) as i32,
            c,
            1,
        );
    }
}

/// Jagged lightning from the top of the field down to (x, y).
fn lightning(g: &mut Gfx, x: f32, y: f32, seed: u32, w: i32) {
    let mut r = Lcg(seed);
    let (mut px, mut py) = (x + (r.next() - 0.5) * 30.0, 0.0f32);
    for k in 1..=8 {
        let nx = x + (r.next() - 0.5) * 26.0 * (1.0 - k as f32 / 9.0);
        let ny = y * k as f32 / 8.0;
        g.line(px as i32, py as i32, nx as i32, ny as i32, hex(0xa080ff), w + 3);
        g.line(px as i32, py as i32, nx as i32, ny as i32, hex(0xf8f070), w);
        g.line(px as i32, py as i32, nx as i32, ny as i32, c::WHITE, 1);
        px = nx;
        py = ny;
    }
}

/// Four nested dithered discs, the bursts of FLARE, EXPLODE and friends.
fn burst(g: &mut Gfx, x: f32, y: f32, rad: [(i32, u16); 4], vis: f32) {
    for (r, c) in rad {
        g.disc_dither(x, y, r, c, vis);
    }
}

impl Scene {
    fn pl_center(&self, p: usize) -> (f32, f32) {
        (self.pl[p].x - 2.0, self.pl[p].y - 14.0)
    }

    fn travel(&self, m: usize, p: usize, k: f32) -> (f32, f32) {
        let (bx, by) = self.pl_center(p);
        let (mx, my) = (self.mon[m].cx(), self.mon[m].cy());
        (mx + (bx - mx) * k, my + (by - my) * k)
    }

    /// One of the ordinary effects inside a bigger one: kind on the monster,
    /// from t0 (seconds into the big one) for dur.
    fn sub_fx(&mut self, g: &mut Gfx, e: &Fx, kind: Ek, t0: f32, dur: f32, seed: u32) {
        let t = (e.t * e.dur - t0) / dur;
        if !(0.0..1.0).contains(&t) {
            return;
        }
        let sub = Fx { kind, mon: e.mon, pl: None, t, dur, seed, arg: 0, x: 0.0, y: 0.0, col: 0 };
        self.draw_fx(g, &sub);
    }

    /// A player's final blow on a monster whose project has no session left
    /// (act_limit). T: seconds into it; the blow lands at LIMITS[].impact.
    fn draw_limit(&mut self, g: &mut Gfx, e: &Fx, m: usize, p: Option<usize>) {
        let tt = e.t * e.dur;
        let (white, gold) = (c::WHITE, c::GOLD);
        let (cx, cy) = (self.mon[m].cx(), self.mon[m].cy());
        let mut r = Lcg(e.seed);
        let lb = LIMITS.iter().find(|l| l.lb as i32 == e.arg);
        let imp = lb.map_or(1.0, |l| l.impact);
        match lb.map(|l| l.lb) {
            Some(Lb::ThousandCuts) => {
                // six slashes from every side, then a great X
                for i in 0..6 {
                    let a = r.next() * PI;
                    let t0 = 0.05 + i as f32 * 0.17;
                    let k = (tt - t0) / 0.12;
                    if !(0.0..=1.0).contains(&k) {
                        continue;
                    }
                    let l = 56.0;
                    let (dx, dy) = (a.cos() * l / 2.0, a.sin() * l / 2.0);
                    let (x0, y0) = ((cx - dx) as i32, (cy - dy) as i32);
                    let (x1, y1) = ((cx - dx + 2.0 * dx * k) as i32, (cy - dy + 2.0 * dy * k) as i32);
                    g.line(x0, y0, x1, y1, hex(0x9ad8ff), 4);
                    g.line(x0, y0, x1, y1, white, 2);
                }
                if tt >= imp - 0.12 && tt < imp + 0.3 {
                    let k = ((tt - imp + 0.12) / 0.12).clamp(0.0, 1.0);
                    let l = 76.0;
                    for (col, w) in [(hex(0x9ad8ff), 7), (white, 3)] {
                        let (a, b) = (l / 2.0, l * k);
                        g.line((cx + a) as i32, (cy - a) as i32, (cx + a - b) as i32, (cy - a + b) as i32, col, w);
                        g.line((cx - a) as i32, (cy - a) as i32, (cx - a + b) as i32, (cy - a + b) as i32, col, w);
                    }
                }
            }
            Some(Lb::Meteor) => {
                // burning rocks rain on it
                const TRAIL: [u16; 5] = [hex(0xfff4c0), hex(0xf8c840), hex(0xf8a040), hex(0xe8641c), hex(0xa82018)];
                for n in 0..9 {
                    let ox = (r.next() - 0.5) * 48.0;
                    let oy = (r.next() - 0.5) * 24.0;
                    let t0 = n as f32 * 0.1;
                    let k = (tt - t0) / 0.5;
                    if k < 0.0 {
                        continue;
                    }
                    if k > 1.0 {
                        if k < 1.5 {
                            let rr = (k - 1.0) * 30.0;
                            g.ring(cx + ox, cy + oy, rr, rr * 0.5, hex(0xf8a040));
                        }
                        continue;
                    }
                    let (x, y) = (cx + ox + 90.0 * (1.0 - k), cy + oy - 150.0 * (1.0 - k));
                    for tr in 1..20 {
                        let w = (5 - tr / 4).max(1);
                        let (rx, ry) = (x + tr as f32 * 0.6 - w as f32 / 2.0, y - tr as f32 - w as f32 / 2.0);
                        g.rect(rx as i32, ry as i32, w, w, TRAIL[(tr / 4).min(4) as usize]);
                    }
                    g.blit(&ICON_ROCK, (x - 4.0) as i32, (y - 4.0) as i32, None);
                }
            }
            Some(Lb::SacredHymn) => {
                // light falls everywhere, a pillar opens on it
                let fh = FIELD_H as f32;
                self.flash = self.flash.max(0.25 * (tt / 0.5).clamp(0.0, 1.0) * if tt < imp { 1.0 } else { 0.0 });
                for n in 0..40 {
                    let x = r.next() * FF_W as f32;
                    let sp = 60.0 + r.next() * 80.0;
                    let ph = r.next();
                    let y = ((ph * fh + tt * sp) % fh) as i32;
                    let col = if n % 3 != 0 { gold } else { white };
                    g.px(x as i32, y, col);
                    g.px(x as i32, y - 1, col);
                }
                if tt > 0.4 {
                    let k = ((tt - 0.4) / (imp - 0.4)).clamp(0.0, 1.0);
                    let mut w = (4.0 + k * 40.0) as i32;
                    if tt > imp {
                        w = (w as f32 * 0f32.max(1.0 - (tt - imp) / 0.5)) as i32;
                    }
                    let mut y = 0;
                    while (y as f32) < cy + 20.0 {
                        let band = if (y + (self.t * 60.0) as i32) % 8 < 4 { hex(0xf8f0a0) } else { white };
                        g.rect((cx - w as f32 / 2.0) as i32, y, w, 1, band);
                        g.rect((cx - w as f32 / 4.0) as i32, y, w / 2, 1, white);
                        y += 1;
                    }
                    g.ring(cx, cy + 18.0, (w + 10) as f32, 5.0, hex(0xf8f0a0));
                }
                for n in 0..4 {
                    let x = cx - 30.0 + (n * 20) as f32 + (tt * 6.0 + n as f32).sin() * 6.0;
                    let y = -12.0 + (tt * 70.0 + (n * 30) as f32) % cy;
                    g.blit(&ICON_FEATHER, x as i32, y as i32, None);
                }
            }
            Some(Lb::SpellCascade) => {
                // spell after spell, then FLARE
                self.sub_fx(g, e, Ek::Fire, 0.0, 0.5, e.seed);
                self.sub_fx(g, e, Ek::Ice, 0.25, 0.5, e.seed.wrapping_add(1));
                self.sub_fx(g, e, Ek::Bolt, 0.5, 0.4, e.seed.wrapping_add(2));
                self.sub_fx(g, e, Ek::Gale, 0.7, 0.45, e.seed.wrapping_add(3));
                self.sub_fx(g, e, Ek::Flare, imp - 0.36, 0.9, e.seed.wrapping_add(4));
            }
            Some(Lb::TwelveKnights) => {
                // the knights strike one after another, the last with all its might
                self.dim = self.dim.max(0.6 * (tt / 0.3).min((e.dur - tt) / 0.3).clamp(0.0, 1.0));
                for n in 0..12 {
                    let ox = (r.next() - 0.5) * 44.0;
                    let t0 = 0.3 + n as f32 * 0.12;
                    let k = (tt - t0) / 0.15;
                    if !(0.0..=1.0).contains(&k) {
                        continue;
                    }
                    let (x, y1) = ((cx + ox) as i32, (cy * 1f32.min(k * 2.0)) as i32);
                    g.line(x, 0, x, y1, hex(0xf8c840), 5);
                    g.line(x, 0, x, y1, white, 2);
                    if k > 0.5 {
                        g.ring(x as f32, cy, 4.0 + k * 10.0, 2.0 + k * 5.0, white);
                    }
                }
                if tt > imp - 0.15 && tt < imp + 0.3 {
                    let w = (16.0 * (1.0 - ((tt - imp) / 0.3).clamp(0.0, 1.0)) + 4.0) as i32;
                    g.rect((cx - w as f32 / 2.0) as i32, 0, w, cy as i32 + 10, hex(0xf8c840));
                    g.rect((cx - w as f32 / 4.0) as i32, 0, w / 2, cy as i32 + 10, white);
                }
            }
            Some(Lb::StarPiercer) => 'arm: {
                // a charged beam
                let Some(p) = p else { break 'arm };
                let (ax, ay) = (self.pl[p].x - 14.0, self.pl[p].y - 18.0);
                if tt < 0.5 {
                    let k = tt / 0.5;
                    g.ring(ax, ay, 24.0 * (1.0 - k) + 3.0, 24.0 * (1.0 - k) + 3.0, hex(0x58c8f8));
                    g.disc(ax, ay, (1.0 + k * 5.0) as i32, white);
                    break 'arm;
                }
                let k = ((tt - 0.5) / 0.15).clamp(0.0, 1.0);
                let w = if tt < imp { 2.0 + (tt - 0.5) * 24.0 } else { 14.0 * 0f32.max(1.0 - (tt - imp) / 0.4) };
                let (bx, by) = ((ax + (cx - ax) * k) as i32, (ay + (cy - ay) * k) as i32);
                if w >= 1.0 {
                    g.line(ax as i32, ay as i32, bx, by, hex(0x2a70b8), w as i32 + 4);
                    g.line(ax as i32, ay as i32, bx, by, hex(0x58c8f8), w as i32);
                    g.line(ax as i32, ay as i32, bx, by, white, (w / 3.0) as i32 + 1);
                }
            }
            Some(Lb::HeavensFist) => 'arm: {
                // a flurry, then a blast of blue
                for i in 0..5 {
                    let ox = (r.next() - 0.5) * 20.0;
                    let oy = (r.next() - 0.5) * 20.0;
                    let k = (tt - i as f32 * 0.1) / 0.12;
                    if (0.0..=1.0).contains(&k) {
                        rays(g, cx + ox, cy + oy, (3.0 + k * 9.0) * 0.4, 3.0 + k * 9.0, 0.4 * i as f32, white);
                    }
                }
                if tt < imp - 0.05 {
                    break 'arm;
                }
                let k = (tt - imp + 0.05) / (e.dur - imp + 0.05);
                let vis = if k < 0.5 { 1.0 } else { (1.0 - k) / 0.5 };
                let rr = (6.0 + k.sqrt() * 40.0) as i32;
                let rf = rr as f32;
                burst(
                    g,
                    cx,
                    cy,
                    [
                        (rr, hex(0x2a4ad8)),
                        ((rf * 0.75) as i32, hex(0x3a7af8)),
                        ((rf * 0.5) as i32, hex(0xa8d8ff)),
                        ((rf * 0.25) as i32, white),
                    ],
                    vis,
                );
            }
            Some(Lb::LanceRain) => {
                // lances rain from the sky
                for n in 0..7 {
                    let ox = (r.next() - 0.5) * 44.0;
                    let t0 = 0.1 + n as f32 * 0.13;
                    let k = (tt - t0) / 0.15;
                    if !(0.0..=1.6).contains(&k) {
                        continue;
                    }
                    let (x, y) = ((cx + ox) as i32, (cy * 1f32.min(k)) as i32);
                    if k <= 1.0 {
                        for i in [-3, 0, 3] {
                            g.line(x + i, y - 50, x + i, y - 30 - i.abs() * 3, white, 1);
                        }
                        g.line(x, y - 22, x, y, hex(0xb8a8f8), 2);
                        g.line(x, y - 4, x, y, white, 3);
                    } else {
                        rays(g, x as f32, cy, 2.0, 4.0 + (k - 1.0) * 14.0, 0.0, white);
                    }
                }
            }
            None => {}
        }
        // every final blow lands with a flash and shock rings
        let k = (tt - imp) / 0.35;
        if (0.0..=1.0).contains(&k) {
            g.ring(cx, cy, 8.0 + k * 50.0, 5.0 + k * 30.0, white);
            g.ring(cx, cy, 4.0 + k * 34.0, 3.0 + k * 20.0, gold);
        }
    }

    /// A monster's finisher on a player idle too long: the field darkens, the
    /// monster's own attack made big, and a blast as the player goes down at
    /// FIN_IMPACT.
    fn draw_finisher(&mut self, g: &mut Gfx, e: &Fx, m: Option<usize>, p: usize) {
        let t = e.t;
        let imp = FIN_IMPACT / FIN_DUR;
        let white = c::WHITE;
        let (px, py) = self.pl_center(p);
        let (mx, my) = m.map_or((px - 120.0, py), |m| (self.mon[m].cx(), self.mon[m].cy()));
        let mut r = Lcg(e.seed);
        self.dim = self.dim.max(0.5 * (t / 0.12).min((1.0 - t) / 0.2).clamp(0.0, 1.0));
        const FXS: [MonFx; 8] = [
            MonFx::Punch,
            MonFx::Blaze,
            MonFx::Needles,
            MonFx::Knife,
            MonFx::Doom,
            MonFx::Breath,
            MonFx::Blaster,
            MonFx::Meteor,
        ];
        match FXS.get(e.arg as usize) {
            Some(MonFx::Punch) => {
                // GOBLIN RUSH: a flurry of blows, then the big one
                let mut off = [[0f32; 2]; 6];
                for o in off.iter_mut() {
                    o[0] = (r.next() - 0.5) * 22.0;
                    o[1] = (r.next() - 0.5) * 26.0;
                }
                for (i, o) in off.iter().enumerate() {
                    let k = (t - 0.08 - i as f32 * 0.07) / 0.12;
                    if (0.0..=1.0).contains(&k) {
                        rays(g, px + o[0], py + o[1], (3.0 + k * 9.0) * 0.4, 3.0 + k * 9.0, 0.3 * i as f32, white);
                    }
                }
                let k = (t - imp + 0.02) / 0.2;
                if (0.0..=1.0).contains(&k) {
                    rays(g, px, py, 4.0 + k * 8.0, 8.0 + k * 26.0, 0.0, white);
                    rays(g, px, py, 2.0 + k * 6.0, 6.0 + k * 18.0, 0.39, c::GOLD);
                }
            }
            Some(MonFx::Blaze) => 'arm: {
                // EXPLODE: a big fireball, and the blast
                if t < imp {
                    let k = ((t - 0.1) / (imp - 0.1)).clamp(0.0, 1.0);
                    let x = mx + (px - mx) * k;
                    let y = my + (py - my) * k - (k * PI).sin() * 30.0;
                    let rr = (4.0 + k * 6.0) as i32;
                    g.disc(x, y, rr + 2, hex(0xa82018));
                    g.disc(x, y, rr, hex(0xe8641c));
                    g.disc(x, y, rr - 2, hex(0xf8c840));
                    g.disc(x, y, rr - 4, hex(0xfff4c0));
                    break 'arm;
                }
                let k = (t - imp) / (1.0 - imp);
                let vis = if k < 0.5 { 1.0 } else { (1.0 - k) / 0.5 };
                let rr = (8.0 + k.sqrt() * 34.0) as i32;
                let rf = rr as f32;
                burst(
                    g,
                    px,
                    py,
                    [
                        (rr, hex(0xa82018)),
                        ((rf * 0.8) as i32, hex(0xe8641c)),
                        ((rf * 0.6) as i32, hex(0xf8c840)),
                        ((rf * 0.35) as i32, hex(0xfff4c0)),
                    ],
                    vis,
                );
                let ring = (rr + 6) as f32 + k * 16.0;
                g.ring(px, py, ring, ring * 0.6, hex(0xfff4c0));
                if k < 0.7 {
                    let h = ((k / 0.7 * PI).sin() * 22.0) as i32;
                    let seed = (self.t * 30.0) as i32;
                    flame(g, px - 12.0, py + 16.0, h, seed);
                    flame(g, px + 10.0, py + 18.0, (h as f32 * 0.8) as i32, seed + 5);
                }
            }
            Some(MonFx::Needles) => {
                // NEEDLE STORM: a storm of them
                let Some(m) = m else { return self.finisher_rings(g, t, imp, px, py) };
                for _ in 0..64 {
                    let d = r.next() * 0.42;
                    let ox = (r.next() - 0.5) * 30.0;
                    let oy = (r.next() - 0.5) * 34.0;
                    let k = (t - 0.06 - d) / 0.2;
                    if !(0.0..=1.0).contains(&k) {
                        continue;
                    }
                    let (x, y) = self.travel(m, p, k);
                    let (nx, ny) = (x + ox * k, y + oy * k);
                    g.line(nx as i32, ny as i32, (nx - 4.0) as i32, ny as i32, hex(0xe8f8c0), 1);
                    g.px(nx as i32, ny as i32, hex(0x58a040));
                }
            }
            Some(MonFx::Knife) => {
                // ANCIENT GRUDGE: a dark aura, knives rain, a red cross
                let k = ((t - 0.05) / 0.4).clamp(0.0, 1.0);
                if t < 0.9 {
                    g.disc_dither(px, py, (6.0 + k * 18.0) as i32, hex(0x4a1060), 0.5);
                    let rr = 8.0 + k * 18.0 + (t * 40.0).sin();
                    g.ring(px, py, rr, rr, hex(0xc060ff));
                }
                for i in 0..9 {
                    let kk = (t - 0.12 - i as f32 * 0.04) / 0.12;
                    if !(0.0..=1.0).contains(&kk) {
                        continue;
                    }
                    let (x, y) = ((px + ((i - 4) * 6) as f32) as i32, (py - 90.0 + 86.0 * kk) as i32);
                    g.line(x, y - 9, x, y, white, 2);
                    g.rect(x - 1, y - 13, 3, 4, hex(0x8a5a30));
                }
                let k1 = ((t - imp) / 0.1).clamp(0.0, 1.0);
                let k2 = ((t - imp - 0.06) / 0.1).clamp(0.0, 1.0);
                if t >= imp && t < imp + 0.35 {
                    let l = 40.0;
                    let a = l / 2.0;
                    for (col, w) in [(hex(0xff5a50), 5), (white, 2)] {
                        let b = l * k1;
                        g.line((px + a) as i32, (py - a) as i32, (px + a - b) as i32, (py - a + b) as i32, col, w);
                    }
                    if k2 > 0.0 {
                        for (col, w) in [(hex(0xff5a50), 5), (white, 2)] {
                            let b = l * k2;
                            g.line((px - a) as i32, (py - a) as i32, (px - a + b) as i32, (py - a + b) as i32, col, w);
                        }
                    }
                }
            }
            Some(MonFx::Doom) => {
                // DEATH: a dark sphere and a great skull
                let k = ((t / 0.95).clamp(0.0, 1.0) * PI).sin();
                let rr = (k * 26.0) as i32;
                if rr > 0 {
                    g.disc(px, py, rr, hex(0x2a1040));
                    g.disc(px, py, if rr > 6 { rr - 6 } else { 0 }, hex(0x6a2a98));
                    g.ring(px, py, (rr + 1) as f32, (rr + 1) as f32, hex(0xe080ff));
                    g.ring(px, py, (rr + 5) as f32, (rr + 5) as f32, hex(0x6a2a98));
                }
                if t > 0.2 && t < imp + 0.15 && ((t * 12.0) as i32 % 2 != 0 || t > imp - 0.1) {
                    g.blit_scaled(&ICON_SKULL, (px - 7.0) as i32, (py - 9.0) as i32, 3, false);
                }
            }
            Some(MonFx::Breath) => {
                // PUTRID BREATH: a great cloud
                let Some(m) = m else { return self.finisher_rings(g, t, imp, px, py) };
                for _ in 0..36 {
                    let d = r.next() * 0.35;
                    let ox = (r.next() - 0.5) * 34.0;
                    let oy = (r.next() - 0.5) * 28.0;
                    let purple = r.next() < 0.5;
                    let k = 1f32.min((t - 0.05 - d) / 0.4);
                    if k < 0.0 {
                        continue;
                    }
                    let (x, y) = self.travel(m, p, k);
                    let mut rr = (3.0 + k * 6.0 + if t > imp { (t - imp) * 8.0 } else { 0.0 }) as i32;
                    if t > 0.9 {
                        rr = (rr as f32 * (1.0 - t) / 0.1) as i32;
                    }
                    let (c0, c1) = if purple { (hex(0xa868c8), hex(0xd8a8f0)) } else { (hex(0x78c048), hex(0xb8f088)) };
                    g.disc(x + ox * k, y + oy * k, rr, c0);
                    g.disc(x + ox * k - 1.0, y + oy * k - 1.0, if rr > 2 { rr - 2 } else { 0 }, c1);
                }
            }
            Some(MonFx::Blaster) => {
                // MEGAVOLT: three great bolts, the last as it lands
                for i in 0..3u32 {
                    let t0 = imp - 0.3 + i as f32 * 0.15;
                    if t >= t0 && t < t0 + 0.12 {
                        let seed = e.seed.wrapping_add(i * 7).wrapping_add((t * 30.0) as u32);
                        lightning(g, px + ((i as i32 - 1) * 6) as f32, py, seed, 4);
                        if t < t0 + 0.03 {
                            self.flash = self.flash.max(0.45);
                        }
                    }
                }
                if m.is_some() && t < imp - 0.3 {
                    // the stormcat gathers it
                    let k = t / (imp - 0.3);
                    g.ring(mx, my, 30.0 * (1.0 - k) + 4.0, 18.0 * (1.0 - k) + 3.0, hex(0xf8f070));
                    g.ring(mx, my, 20.0 * (1.0 - k) + 2.0, 12.0 * (1.0 - k) + 2.0, white);
                }
            }
            Some(MonFx::Meteor) => 'arm: {
                // CATACLYSM: a green sphere gathers on the player and bursts
                if t < imp {
                    let k = t / imp;
                    for n in 0..10 {
                        let a = n as f32 / 10.0 * 2.0 * PI + t * 9.0;
                        let rr = 50.0 * (1.0 - k) + 4.0;
                        let col = if n % 2 != 0 { white } else { hex(0x7cf8a0) };
                        g.rect((px + a.cos() * rr) as i32, (py + a.sin() * rr * 0.7) as i32, 2, 2, col);
                    }
                    g.disc_dither(px, py, (3.0 + k * 10.0) as i32, hex(0x2a8040), 1.0);
                    g.disc(px, py, (1.0 + k * 4.0) as i32, white);
                    break 'arm;
                }
                let k = (t - imp) / (1.0 - imp);
                let vis = if k < 0.5 { 1.0 } else { (1.0 - k) / 0.5 };
                let rr = (10.0 + k.sqrt() * 44.0) as i32;
                let rf = rr as f32;
                burst(
                    g,
                    px,
                    py,
                    [
                        (rr, hex(0x1a6030)),
                        ((rf * 0.75) as i32, hex(0x58c070)),
                        ((rf * 0.45) as i32, hex(0xc8f8c8)),
                        ((rf * 0.2) as i32, white),
                    ],
                    vis,
                );
            }
            None => {}
        }
        self.finisher_rings(g, t, imp, px, py);
    }

    /// Every finisher lands with a flash and shock rings.
    fn finisher_rings(&mut self, g: &mut Gfx, t: f32, imp: f32, px: f32, py: f32) {
        let k = (t - imp) / 0.3;
        if (0.0..=1.0).contains(&k) {
            g.ring(px, py, 6.0 + k * 40.0, 4.0 + k * 24.0, c::WHITE);
            g.ring(px, py, 3.0 + k * 28.0, 2.0 + k * 16.0, c::GOLD);
            if k < 0.15 {
                self.flash = self.flash.max(0.75);
            }
        }
    }

    pub(super) fn draw_fx(&mut self, g: &mut Gfx, e: &Fx) {
        let t = e.t;
        let m = self.mon_at(e.mon);
        let p = e.pl.filter(|&i| self.pl[i].used);
        let mut r = Lcg(e.seed);
        let white = c::WHITE;
        let (cx, cy) = m.map_or((0.0, 0.0), |m| (self.mon[m].cx(), self.mon[m].cy()));
        match e.kind {
            Ek::Fire => {
                let Some(m) = m else { return };
                let img = &MONSTERS[self.mon[m].kind].idle;
                let (w, h) = (img.w as i32, img.h as f32);
                let offs = [[-w as f32 * 0.25, 4.0], [w as f32 * 0.2, 6.0], [0.0, -2.0]];
                for (i, o) in offs.iter().enumerate() {
                    let tt = (t * 1.6 - i as f32 * 0.18).clamp(0.0, 1.0);
                    let fh = (tt * PI).sin() * 18.0;
                    if fh > 1.0 {
                        let seed = (self.t * 30.0) as i32 + i as i32 * 7;
                        flame(g, cx + o[0], cy + h * 0.35 + o[1], fh as i32, seed);
                    }
                }
            }
            Ek::Ice => {
                if m.is_none() {
                    return;
                }
                let mut sh = [[0f32; 3]; 6];
                let mut parts = [[0f32; 2]; 18];
                for s in sh.iter_mut() {
                    s[0] = (r.next() - 0.5) * 34.0;
                    s[1] = (r.next() - 0.5) * 30.0;
                    s[2] = 3.0 + r.next() * 4.0;
                }
                for q in parts.iter_mut() {
                    q[0] = r.next() * 2.0 * PI;
                    q[1] = 10.0 + r.next() * 30.0;
                }
                let (ice, deep) = (hex(0x9ae8ff), hex(0x2a6ab8));
                if t < 0.6 {
                    for (i, s) in sh.iter().enumerate() {
                        let gr = 1f32.min(t * 4.0 - i as f32 * 0.12);
                        if gr <= 0.0 {
                            continue;
                        }
                        let sz = iround(s[2] * gr);
                        for j in -sz * 2..=sz * 2 {
                            let w = sz as f32 - j.abs() as f32 / 2.0;
                            let mut i2 = -w;
                            while i2 <= w {
                                let col = if i2.abs() >= w - 0.5 {
                                    deep
                                } else if i2 < 0.0 {
                                    white
                                } else {
                                    ice
                                };
                                g.px((cx + s[0] + i2) as i32, (cy + s[1] + j as f32) as i32, col);
                                i2 += 1.0;
                            }
                        }
                    }
                } else {
                    let k = (t - 0.6) / 0.4;
                    for q in parts {
                        let x = cx + q[0].cos() * q[1] * k * 1.2;
                        let y = cy + q[0].sin() * q[1] * k + k * k * 10.0;
                        g.rect(x as i32, y as i32, 2, 2, if k < 0.5 { white } else { ice });
                    }
                }
            }
            Ek::Bolt => {
                if m.is_none() || (t * 10.0) as i32 % 3 == 2 {
                    return;
                }
                let mut x = cx + (r.next() - 0.5) * 4.0;
                let mut y = 0.0;
                let (mut px, mut py) = (x, y);
                let mut rr = Lcg(((t * 12.0) as i32 + 99) as u32);
                for k in 1..=7 {
                    x = cx + (rr.next() - 0.5) * 18.0 * (1.0 - k as f32 / 8.0);
                    y = cy * k as f32 / 7.0;
                    g.line(px as i32, py as i32, x as i32, y as i32, hex(0xa080ff), 4);
                    g.line(px as i32, py as i32, x as i32, y as i32, hex(0xf8f070), 2);
                    g.line(px as i32, py as i32, x as i32, y as i32, white, 1);
                    px = x;
                    py = y;
                }
                if t < 0.15 {
                    self.flash = self.flash.max(0.35);
                }
            }
            Ek::Scan => {
                let Some(m) = m else { return };
                let img = &MONSTERS[self.mon[m].kind].idle;
                let gc = hex(0x7cf8a0);
                let k = 1f32.min(t / 0.35);
                let hw = img.w as f32 / 2.0 + 4.0 + (1.0 - k) * 30.0;
                let hh = img.h as f32 / 2.0 + 4.0 + (1.0 - k) * 24.0;
                for s in 0..4 {
                    let sx = if s & 1 != 0 { 1.0 } else { -1.0 };
                    let sy = if s & 2 != 0 { 1.0 } else { -1.0 };
                    let (x, y) = (cx + sx * hw, cy + sy * hh);
                    g.line(x as i32, y as i32, (x - sx * 6.0) as i32, y as i32, gc, 1);
                    g.line(x as i32, y as i32, x as i32, (y - sy * 6.0) as i32, gc, 1);
                }
                if t > 0.35 {
                    let yy = cy - hh + 2.0 * hh * ((t - 0.35) / 0.65);
                    g.line((cx - hw + 1.0) as i32, yy as i32, (cx + hw - 1.0) as i32, yy as i32, gc, 1);
                    let mut y = cy - hh;
                    while y < yy {
                        let mut x = cx - hw;
                        while x < cx + hw {
                            if BAYER[(((y as i32) & 3) * 4 + ((x as i32) & 3)) as usize] < 3 {
                                g.blend(x as i32, y as i32, gc, 128);
                            }
                            x += 2.0;
                        }
                        y += 2.0;
                    }
                }
            }
            Ek::Gale => {
                if m.is_none() {
                    return;
                }
                for k in 0..16 {
                    let a = k as f32 / 16.0 * 2.0 * PI + t * 12.0;
                    let rr = 26.0 * (1.0 - t) + 6.0 + ((k % 3) * 3) as f32;
                    let x = cx + a.cos() * rr;
                    let y = cy + a.sin() * rr * 0.6 - t * 6.0;
                    g.rect(x as i32, y as i32, 2, 1, if k % 2 != 0 { hex(0xc8f8d0) } else { hex(0x58d080) });
                    g.px((x - (a + 1.6).cos() * 2.0) as i32, (y - (a + 1.6).sin()) as i32, hex(0x58d080));
                }
            }
            Ek::Flare => {
                if m.is_none() {
                    return;
                }
                if t < 0.3 {
                    let k = t / 0.3;
                    let rad = 34.0 * (1.0 - k) + 2.0;
                    g.ring(cx, cy, rad, rad, white);
                    g.ring(cx, cy, rad * 0.75, rad * 0.75, hex(0xf8a040));
                    g.disc(cx, cy, (1.0 + k * 2.0) as i32, white);
                    return;
                }
                let k = (t - 0.3) / 0.7;
                let vis = if k < 0.6 { 1.0 } else { (1.0 - k) / 0.4 };
                let rr = (6.0 + k.sqrt() * 22.0) as i32;
                if k < 0.12 {
                    self.flash = self.flash.max(0.5);
                }
                let rf = rr as f32;
                burst(
                    g,
                    cx,
                    cy,
                    [
                        (rr, hex(0xa82018)),
                        ((rf * 0.8) as i32, hex(0xe8641c)),
                        ((rf * 0.6) as i32, hex(0xf8c840)),
                        ((rf * 0.35) as i32, hex(0xfff4c0)),
                    ],
                    vis,
                );
                let ring = rr as f32 + 6.0 + k * 14.0;
                g.ring(cx, cy, ring, ring * 0.6, hex(0xfff4c0));
            }
            Ek::Holy => {
                if m.is_none() {
                    return;
                }
                let gold = hex(0xf8f0a0);
                if t < 0.6 {
                    let k = t / 0.6;
                    let rad = 34.0 * (1.0 - k) + 4.0;
                    for n in 0..6 {
                        let a = n as f32 / 6.0 * 2.0 * PI + k * 6.0;
                        let (x, y) = (cx + a.cos() * rad, cy + a.sin() * rad * 0.7);
                        g.disc(x, y, 3, gold);
                        g.disc(x, y, 1, white);
                    }
                    return;
                }
                let k = (t - 0.6) / 0.4;
                let w = (18.0 * (1.0 - k) + 2.0) as i32;
                if k < 0.15 {
                    self.flash = self.flash.max(0.45);
                }
                let mut y = 0;
                while (y as f32) < cy + 14.0 {
                    let band = if (y + (self.t * 60.0) as i32) % 6 < 3 { gold } else { white };
                    g.rect((cx - w as f32 / 2.0) as i32, y, w, 1, band);
                    g.rect((cx - w as f32 / 4.0) as i32, y, w / 2, 1, white);
                    y += 1;
                }
                g.ring(cx, cy + 12.0, (w + 8) as f32, 4.0, gold);
            }
            Ek::Summon => {
                let Some(m) = m else { return };
                let [c0, c1] = SUMMON_COL[e.arg as usize];
                let my = self.mon[m].y;
                let d = if t < 0.2 {
                    t / 0.2 * 0.55
                } else if t > 0.8 {
                    0f32.max((1.0 - t) / 0.2) * 0.55
                } else {
                    0.55
                };
                self.dim = self.dim.max(d);
                if t > 0.1 && t < 0.9 {
                    let k = 1f32.min((t - 0.1) / 0.25);
                    let (rx, ry) = (34.0 * k, 10.0 * k);
                    g.ring(cx, my, rx, ry, c1);
                    g.ring(cx, my, rx - 4.0, ry - 1.5, c0);
                    for n in 0..8 {
                        let a = n as f32 / 8.0 * 2.0 * PI + t * 4.0;
                        g.rect((cx + a.cos() * (rx - 2.0)) as i32, (my + a.sin() * (ry - 0.7)) as i32, 2, 2, c0);
                    }
                }
                if t > 0.45 && t < 0.85 {
                    let k = 1f32.min((t - 0.45) / 0.4 * 1.5);
                    let (x0, y0) = ((FF_W - 30) as f32, -10.0f32);
                    let (x1, y1) = ((x0 + (cx - x0) * k) as i32, (y0 + (cy - y0) * k) as i32);
                    g.line(x0 as i32, y0 as i32, x1, y1, c1, 9);
                    g.line(x0 as i32, y0 as i32, x1, y1, c0, 5);
                    g.line(x0 as i32, y0 as i32, x1, y1, white, 2);
                }
                if t > 0.7 {
                    let k = (t - 0.7) / 0.3;
                    g.ring(cx, cy, 10.0 + k * 60.0, 6.0 + k * 34.0, c0);
                    g.ring(cx, cy, 6.0 + k * 46.0, 4.0 + k * 26.0, c1);
                    if k < 0.25 {
                        self.flash = self.flash.max(0.6);
                    }
                }
            }
            Ek::Chant => {
                let Some(p) = p else { return };
                let (hx, hy) = (self.pl[p].x - 12.0, self.pl[p].y - 30.0);
                for k in 0..8 {
                    let a = k as f32 / 8.0 * 2.0 * PI + t * 6.0;
                    let rr = 16.0 * (1.0 - t);
                    g.px(
                        (hx + a.cos() * rr) as i32,
                        (hy + a.sin() * rr) as i32,
                        if k % 2 != 0 { white } else { c::GOLD },
                    );
                }
                self.pl[p].white = self.pl[p].white.max(0.25 * (t * PI).sin());
            }
            Ek::Protect => {
                let Some(p) = p else { return };
                if (t * 14.0) as i32 % 2 != 0 {
                    return;
                }
                let col = hex(0xf8f0a0);
                let (x, y) = (self.pl[p].x - 1.0, self.pl[p].y - 16.0);
                g.ring(x, y, 16.0, 20.0, col);
                g.ring(x, y, 14.0, 18.0, mix(col, white, 154));
            }
            Ek::Sparkle => {
                let (bx, by) = p.map_or((e.x, e.y), |p| (self.pl[p].x, self.pl[p].y));
                for _ in 0..12 {
                    let ox = (r.next() - 0.5) * 22.0;
                    let d = r.next() * 0.5;
                    let s = r.next();
                    let kk = (t - d) / (1.0 - d);
                    if !(0.0..=1.0).contains(&kk) {
                        continue;
                    }
                    let (x, y) = ((bx - 2.0 + ox) as i32, (by - 4.0 - kk * 34.0) as i32);
                    g.px(x, y, if s > 0.5 { white } else { e.col });
                    g.px(x - 1, y, e.col);
                    g.px(x + 1, y, e.col);
                    g.px(x, y - 1, e.col);
                    g.px(x, y + 1, e.col);
                }
            }
            Ek::Feather => {
                let Some(p) = p else { return };
                let (x, y) = (self.pl[p].x - 4.0 + (t * 9.0).sin() * 8.0, -10.0 + (self.pl[p].y - 16.0) * t);
                g.blit(&ICON_FEATHER, x as i32, y as i32, None);
            }
            Ek::Cross => {
                if m.is_none() || (t > 0.6 && (t * 20.0) as i32 % 2 != 0) {
                    return;
                }
                let l = 40.0;
                let glow = hex(0x9ad8ff);
                let k1 = 1f32.min(t / 0.2);
                let k2 = ((t - 0.18) / 0.2).clamp(0.0, 1.0);
                let (ax, ay) = ((cx + l / 2.0) as i32, (cy - l / 2.0) as i32);
                let (bx, by) = ((cx + l / 2.0 - l * k1) as i32, (cy - l / 2.0 + l * k1) as i32);
                g.line(ax, ay, bx, by, glow, 4);
                g.line(ax, ay, bx, by, white, 2);
                if k2 > 0.0 {
                    let ax = (cx - l / 2.0) as i32;
                    let (bx, by) = ((cx - l / 2.0 + l * k2) as i32, (cy - l / 2.0 + l * k2) as i32);
                    g.line(ax, ay, bx, by, glow, 4);
                    g.line(ax, ay, bx, by, white, 2);
                }
            }
            Ek::Shadow => {
                let Some(m) = m else { return };
                let (x, y) = (iround(self.mon[m].x + 6.0), iround(self.mon[m].y));
                g.shade(x, y, (4.0 + t * 10.0) as i32, (1.0 + t * 2.0) as i32, 141);
            }
            Ek::ChiBurst => {
                let (Some(_), Some(p)) = (m, p) else { return };
                let (ax, ay) = (self.pl[p].x - 16.0, self.pl[p].y - 18.0);
                if t < 0.45 {
                    let k = t / 0.45;
                    for n in (0..=4).rev() {
                        let kk = 0f32.max(k - n as f32 * 0.05);
                        let (x, y) = (ax + (cx - ax) * kk, ay + (cy - ay) * kk);
                        g.disc(x, y, if n != 0 { (4.0 - n as f32 * 0.6) as i32 } else { 6 }, hex(0x3a7af8));
                        if n == 0 {
                            g.disc(x, y, 4, hex(0xa8d8ff));
                            g.disc(x, y, 2, white);
                        }
                    }
                    return;
                }
                let k = (t - 0.45) / 0.55;
                g.ring(cx, cy, 4.0 + k * 22.0, 4.0 + k * 16.0, hex(0xa8d8ff));
                g.ring(cx, cy, 2.0 + k * 14.0, 2.0 + k * 10.0, white);
                if k < 0.3 {
                    g.disc(cx, cy, (6.0 * (1.0 - k)) as i32, white);
                }
            }
            Ek::Strike => {
                if m.is_none() {
                    return;
                }
                let gl = hex(0xc8d8ff);
                let k = 1f32.min(t / 0.15);
                let slash = Strike::Slash as i32;
                if e.arg == slash || e.arg == Strike::Slash2 as i32 {
                    let n = if e.arg == slash { 1 } else { 2 };
                    for i in 0..n {
                        let ox = (i * 8) as f32 - (n - 1) as f32 * 4.0;
                        let l = 26.0;
                        let (x0, y0) = (cx + ox + l / 2.0, cy - l / 2.0);
                        g.line(x0 as i32, y0 as i32, (x0 - l * k) as i32, (y0 + l * k) as i32, gl, 3);
                        g.line(x0 as i32, y0 as i32, (x0 - l * k) as i32, (y0 + l * k) as i32, white, 1);
                    }
                } else if e.arg == Strike::Thrust as i32 {
                    let (x0, x1) = (cx + 26.0, cx + 26.0 - 40.0 * k);
                    g.line(x0 as i32, cy as i32, x1 as i32, cy as i32, gl, 3);
                    g.line(x0 as i32, cy as i32, x1 as i32, cy as i32, white, 1);
                    if k >= 1.0 {
                        rays(g, x1, cy, 0.0, 5.0, 0.0, white);
                    }
                } else {
                    const PTS: [[f32; 2]; 3] = [[-6.0, -4.0], [5.0, 2.0], [-2.0, 7.0]];
                    let n = if e.arg == Strike::Flurry as i32 { 3 } else { 1 };
                    for (i, pt) in PTS.iter().enumerate().take(n) {
                        let kk = t * 3.3 - i as f32 * 0.6;
                        if !(0.0..=1.0).contains(&kk) {
                            continue;
                        }
                        let rr = 3.0 + kk * 6.0;
                        let (ox, oy) = if n == 1 { (0.0, 0.0) } else { (pt[0], pt[1]) };
                        rays(g, cx + ox, cy + oy, rr * 0.4, rr, 0.0, white);
                    }
                }
            }
            Ek::Tap => {
                let rr = 2 + (t * 8.0) as i32;
                let (x, y) = (e.x as i32, e.y as i32);
                let col = if t < 0.5 { white } else { c::GOLD };
                for d in -rr..=rr {
                    g.px(x + d, y - rr + d.abs(), col);
                    g.px(x + d, y + rr - d.abs(), col);
                }
            }
            /* the monsters' attacks, from the monster to the player */
            Ek::MPunch => {
                let Some(p) = p else { return };
                if t < 0.15 {
                    return;
                }
                let k = (t - 0.15) / 0.35;
                let rr = 3.0 + k * 8.0;
                let (px, py) = self.pl_center(p);
                rays(g, px, py, rr * 0.4, rr, 0.3, white);
            }
            Ek::MBlaze => {
                let (Some(m), Some(p)) = (m, p) else { return };
                if t < 0.4 {
                    let (x, mut y) = self.travel(m, p, t / 0.4);
                    y -= (t / 0.4 * PI).sin() * 14.0;
                    g.disc(x, y, 5, hex(0xe8641c));
                    g.disc(x, y, 3, hex(0xf8c840));
                    g.disc(x, y, 1, hex(0xfff4c0));
                } else {
                    let k = (t - 0.4) / 0.6;
                    let h = (k * PI).sin() * 16.0;
                    let (px, py) = self.pl_center(p);
                    let seed = (self.t * 30.0) as i32;
                    flame(g, px - 5.0, py + 12.0, h as i32, seed);
                    flame(g, px + 4.0, py + 14.0, (h * 0.8) as i32, seed + 5);
                }
            }
            Ek::MNeedles => {
                let (Some(m), Some(p)) = (m, p) else { return };
                for _ in 0..16 {
                    let d = r.next() * 0.35;
                    let ox = (r.next() - 0.5) * 16.0;
                    let oy = (r.next() - 0.5) * 20.0;
                    let k = (t - d) / 0.4;
                    if !(0.0..=1.0).contains(&k) {
                        continue;
                    }
                    let (x, y) = self.travel(m, p, k);
                    let (nx, ny) = (x + ox * k, y + oy * k);
                    g.line(nx as i32, ny as i32, (nx - 3.0) as i32, ny as i32, hex(0xe8f8c0), 1);
                    g.px(nx as i32, ny as i32, hex(0x58a040));
                }
            }
            Ek::MKnife => {
                let (Some(_), Some(p)) = (m, p) else { return };
                if t < 0.3 {
                    let s = (t / 0.3 * PI).sin() * 4.0;
                    g.line(
                        (cx + 8.0 - s) as i32,
                        (cy - 4.0) as i32,
                        (cx + 8.0 + s) as i32,
                        (cy - 4.0) as i32,
                        white,
                        1,
                    );
                    g.line(
                        (cx + 8.0) as i32,
                        (cy - 4.0 - s) as i32,
                        (cx + 8.0) as i32,
                        (cy - 4.0 + s) as i32,
                        white,
                        1,
                    );
                    return;
                }
                let k = 1f32.min((t - 0.3) / 0.15);
                let (px, py) = self.pl_center(p);
                let (x0, y0, x1, y1) = (
                    (px + 10.0) as i32,
                    (py - 12.0) as i32,
                    (px + 10.0 - 22.0 * k) as i32,
                    (py - 12.0 + 24.0 * k) as i32,
                );
                g.line(x0, y0, x1, y1, hex(0xff5a50), 3);
                g.line(x0, y0, x1, y1, white, 1);
            }
            Ek::MDoom => {
                let Some(p) = p else { return };
                let k = (1f32.min(t / 0.8) * PI).sin();
                let rr = (k * 12.0) as i32;
                let (px, py) = self.pl_center(p);
                if rr > 0 {
                    g.disc(px, py, rr, hex(0x2a1040));
                    g.disc(px, py, if rr > 4 { rr - 4 } else { 0 }, hex(0x6a2a98));
                    g.ring(px, py, (rr + 1) as f32, (rr + 1) as f32, hex(0xe080ff));
                }
                if t > 0.3 && t < 0.75 && (t * 16.0) as i32 % 2 != 0 {
                    g.blit(&ICON_SKULL, (px - 3.0) as i32, (py - 3.0) as i32, None);
                }
            }
            Ek::MBreath => {
                let (Some(m), Some(p)) = (m, p) else { return };
                for _ in 0..12 {
                    let d = r.next() * 0.4;
                    let ox = (r.next() - 0.5) * 18.0;
                    let oy = (r.next() - 0.5) * 14.0;
                    let purple = r.next() < 0.5;
                    let k = 1f32.min((t - d) / 0.5);
                    if k < 0.0 {
                        continue;
                    }
                    let (x, y) = self.travel(m, p, k);
                    let rr = (2.0 + k * 4.0) as i32;
                    let (c0, c1) = if purple { (hex(0xa868c8), hex(0xd8a8f0)) } else { (hex(0x78c048), hex(0xb8f088)) };
                    g.disc(x + ox * k, y + oy * k, rr, c0);
                    g.disc(x + ox * k - 1.0, y + oy * k - 1.0, if rr > 2 { rr - 2 } else { 0 }, c1);
                }
            }
            Ek::MBlaster => {
                let (Some(m), Some(p)) = (m, p) else { return };
                for n in 0..5 {
                    let k = (t - n as f32 * 0.07) / 0.5;
                    if !(0.0..=1.0).contains(&k) {
                        continue;
                    }
                    let (x, y) = self.travel(m, p, k);
                    g.ring(x, y, 2.0 + k * 5.0, 4.0 + k * 9.0, if n % 2 != 0 { hex(0xf8f070) } else { white });
                }
            }
            Ek::MFinish => {
                if let Some(p) = p {
                    self.draw_finisher(g, e, m, p);
                }
            }
            Ek::Limit => {
                if let Some(m) = m {
                    self.draw_limit(g, e, m, p);
                }
            }
            Ek::LimitAura => {
                // a player gathers itself for its final blow
                let Some(p) = p else { return };
                let (x, y) = (self.pl[p].x, self.pl[p].y);
                for k in 0..10 {
                    let ox = (r.next() - 0.5) * 24.0;
                    let d = r.next() * 0.4;
                    let kk = (t - d) / 0.6;
                    if !(0.0..=1.0).contains(&kk) {
                        continue;
                    }
                    let col = if k % 2 != 0 { white } else { c::GOLD };
                    g.rect((x - 2.0 + ox) as i32, (y - 4.0 - kk * 40.0) as i32, 2, 2, col);
                }
                g.ring(x - 1.0, y - 16.0, 22.0 * (1.0 - t) + 6.0, 26.0 * (1.0 - t) + 8.0, c::GOLD);
                self.pl[p].white = self.pl[p].white.max(0.5 * (t * PI * 3.0).sin().abs());
            }
            Ek::MMeteor => {
                let Some(p) = p else { return };
                const TRAIL: [u16; 5] = [hex(0xfff4c0), hex(0xf8c840), hex(0xf8a040), hex(0xe8641c), hex(0xa82018)];
                let (px, py) = self.pl_center(p);
                for n in 0..4 {
                    let d = n as f32 * 0.12;
                    let ox = (r.next() - 0.5) * 20.0;
                    let oy = (r.next() - 0.5) * 10.0;
                    let k = (t - d) / 0.5;
                    if k < 0.0 {
                        continue;
                    }
                    if k > 1.0 {
                        if k < 1.4 {
                            let rr = (k - 1.0) * 20.0;
                            g.ring(px + ox, py + oy + 8.0, rr, rr * 0.5, hex(0xf8a040));
                        }
                        continue;
                    }
                    let (x, y) = (px + ox - 70.0 * (1.0 - k), py + oy + 8.0 - 110.0 * (1.0 - k));
                    for tr in 1..16 {
                        let w = (4 - tr / 4).max(1);
                        let (rx, ry) = (x - tr as f32 * 0.64 - w as f32 / 2.0, y - tr as f32 - w as f32 / 2.0);
                        g.rect(rx as i32, ry as i32, w, w, TRAIL[(tr / 3).min(4) as usize]);
                    }
                    g.blit(&ICON_ROCK, (x - 4.0) as i32, (y - 4.0) as i32, None);
                }
            }
        }
    }
}
