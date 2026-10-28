//! The field and the windows: the battlefield for the time of day, the
//! monsters and the party back to front, the monster, party and info
//! windows, the banner, the status card and the config window.

use super::*;
use crate::art::{
    FIELD_W, FIELDS, ICON_BUBBLE_ASK, ICON_BUBBLE_PERM, ICON_CLAUDE, ICON_CODEX, ICON_CURSOR, ICON_GEAR, Img, Tod,
    UP_AX, UP_AY,
};
use crate::gfx::{Fx as Gfx_fx, Gfx, H as FF_H, text_width};

const SH: Option<u16> = Some(c::SHADOW);
use std::f32::consts::PI;

/// A project as its folder name, the last part of its path ("moon-garden" for
/// ~/projects/moon-garden), in at most max characters.
fn folder_name(path: &str, max: usize) -> String {
    let b = path.as_bytes();
    let mut n = b.len();
    while n > 1 && b[n - 1] == b'/' {
        n -= 1;
    }
    let mut i = n;
    while i > 0 && b[i - 1] != b'/' {
        i -= 1;
    }
    let seg = &path[i..n];
    if seg.len() > max { format!("{}..", cut(seg, max - 2)) } else { seg.to_string() }
}

/// "5S", "12M", "3H", "2D"
fn fmt_age(s: i64) -> String {
    let s = s.max(0);
    if s < 60 {
        format!("{s}S")
    } else if s < 3600 {
        format!("{}M", s / 60)
    } else if s < 86400 {
        format!("{}H", s / 3600)
    } else {
        format!("{}D", s / 86400)
    }
}

/// "45S", "12M", "3H 05M", "2D 3H"
fn fmt_span(s: i64) -> String {
    let s = s.max(0);
    if s < 3600 {
        fmt_age(s)
    } else if s < 86400 {
        format!("{}H {:02}M", s / 3600, s / 60 % 60)
    } else {
        format!("{}D {}H", s / 86400, s / 3600 % 24)
    }
}

fn fmt_usd(v: f64) -> String {
    if v < 9.995 {
        format!("${v:.2}")
    } else if v < 9999.5 {
        format!("${}", (v + 0.5).floor() as i64)
    } else {
        format!("${}K", (v / 1000.0 + 0.5).floor() as i64)
    }
}

/// Word-wrap s into at most max lines of cols characters, as (start, len)
/// byte spans; the flag says whether some of it did not fit.
fn wrap(s: &[u8], max: usize, cols: usize) -> (Vec<(usize, usize)>, bool) {
    let n = s.len();
    let (mut p, mut out) = (0, Vec::new());
    while out.len() < max {
        while p < n && s[p] == b' ' {
            p += 1;
        }
        if p >= n {
            break;
        }
        let mut len = n - p;
        if len > cols {
            len = cols;
            while len > 0 && s[p + len] != b' ' {
                len -= 1;
            }
            if len == 0 {
                len = cols;
            }
        }
        while len > 0 && s[p + len - 1] == b' ' {
            len -= 1;
        }
        out.push((p, len));
        p += len;
    }
    while p < n && s[p] == b' ' {
        p += 1;
    }
    (out, p < n)
}

impl Scene {
    fn local_hm(&self) -> (i64, i64) {
        if !self.have_time {
            return (12, 0);
        }
        let now = self.epoch + (self.ms - self.epoch_ms) / 1000 + self.tz as i64;
        let day = now.rem_euclid(86400);
        (day / 3600, day / 60 % 60)
    }

    fn tod_now(&self) -> Tod {
        let (h, _) = self.local_hm();
        if (7..17).contains(&h) {
            Tod::Day
        } else if (17..19).contains(&h) {
            Tod::Dusk
        } else {
            Tod::Night
        }
    }

    fn pose_of(&self, p: &Player) -> (Pose, bool) {
        if p.walking != Walk::None {
            let step = (self.t * 8.0) as i32 % 2 != 0;
            return (if step { Pose::Step } else { Pose::Ready }, p.walking == Walk::Right);
        }
        let pose = if let Some(pose) = p.pose {
            pose
        } else if p.ko {
            Pose::Ko
        } else if p.hurt > 0.0 {
            Pose::Hurt
        } else if p.state == State::Wait {
            Pose::Raise
        } else if p.hp <= HP_MAX * CRITICAL {
            Pose::Kneel
        } else {
            Pose::Ready
        };
        (pose, false)
    }

    fn draw_player(&self, g: &mut Gfx, p: &Player) {
        if p.hidden {
            return;
        }
        let cls = &CLASSES[p.cls];
        let (pose, flip) = self.pose_of(p);
        let img = &cls.img[p.variant][pose as usize];
        let (x, y) = (iround(p.x - p.fwd * 8.0 + p.kb), iround(p.y));
        if !p.air {
            g.shade(x - 1, y, 9, 2, 90);
        }
        if p.diving {
            for i in [-6, -3, 0, 3, 6] {
                g.line(x + i, y - 70, x + i, y - 40 - i.abs() * 2, c::WHITE, 1);
            }
        }
        let fx = Gfx_fx { white: k256(p.white), flip, ..Gfx_fx::default() };
        if pose == Pose::Ko {
            g.blit(img, x - cls.ko_ax, y - cls.ko_ay, Some(&fx));
        } else {
            let bob = if pose == Pose::Raise && (self.t * 3.0) as i32 % 2 != 0 { -1 } else { 0 };
            g.blit(img, x - UP_AX, y - UP_AY + bob, Some(&fx));
        }
        if p.state == State::Wait && !p.ko && !p.leaving && p.walking == Walk::None {
            let bubble = if p.why == Why::Ask { &ICON_BUBBLE_ASK } else { &ICON_BUBBLE_PERM };
            g.blit(bubble, x + 6, y - 40 + (self.t * 2.0) as i32 % 2, None);
        }
    }

    fn draw_monster(&self, g: &mut Gfx, m: &Monster) {
        let mon = &MONSTERS[m.kind];
        let f: &Img = if m.attack_frame {
            &mon.attack_img
        } else if (self.t * 1.6 + m.phase) as i32 % 2 != 0 {
            &mon.idle2
        } else {
            &mon.idle
        };
        let fl = if mon.floats { iround((self.t * 2.4 + m.phase).sin() * 2.0) - 6 } else { 0 };
        let sx = if m.shake > 0.0 { if (self.t * 40.0) as i32 % 2 != 0 { 1 } else { -1 } } else { 0 };
        let x = iround(m.x - f.w as f32 / 2.0) + sx;
        let y = iround(m.y - f.h as f32 + 1.0 + fl as f32);
        let rx = (f.w as f32 * 0.38) as i32;
        g.shade(iround(m.x), iround(m.y), rx.max(6), 3, if mon.floats { 64 } else { 90 });
        let mut fx = Gfx_fx { white: k256(m.white), vis: m.vis, ..Gfx_fx::default() };
        if m.sweep >= 0.0 {
            fx.tint = hex(0xe060c0);
            fx.tint_k = k256(0.8f32.min(m.sweep * 2.0));
            fx.sweep = m.sweep;
        }
        g.blit(f, x, y, Some(&fx));
    }

    /* ---- the windows ---------------------------------------------------------------------- */

    fn draw_mon_window(&self, g: &mut Gfx) {
        g.window(MON_X, WIN_Y, MON_W, WIN_H);
        let live = self.live_monsters();
        if live.is_empty() {
            g.text("NO FOES", MON_X + 8, ROW_Y + 2, c::DIM, 1, SH);
            return;
        }
        for (i, &mi) in live.iter().enumerate().take(4) {
            let m = &self.mon[mi];
            let y = ROW_Y + i as i32 * 20;
            g.text(MONSTERS[m.kind].name, MON_X + 7, y, if m.white > 0.3 { c::GOLD } else { c::TEXT }, 1, SH);
            g.text_r(&fmt_tok(self.proj[m.pj].dmg), MON_X + MON_W - 7, y, c::TEXT, SH);
            g.text(&folder_name(&self.proj[m.pj].path, 18), MON_X + 7, y + 9, c::DIM, 1, SH);
        }
    }

    fn draw_party_window(&self, g: &mut Gfx) {
        g.window(PARTY_X, WIN_Y, PARTY_W, WIN_H);
        let x0 = PARTY_X + 18;
        let rows = self.party_rows();
        if rows.is_empty() {
            let stale = !self.listed || self.ms - self.list_ms > HOST_STALE_MS;
            g.text(if stale { "LOOKING FOR SESSIONS" } else { "NO ONE IS WORKING" }, x0 + 4, ROW_Y + 2, c::DIM, 1, SH);
        }
        let blink = (self.t * 3.0) as i32 % 2 != 0;
        for (i, &pi) in rows.iter().enumerate() {
            let p = &self.pl[pi];
            let y = ROW_Y + i as i32 * ROW_H;
            let w = p.waiting();
            g.blit(if p.codex { &ICON_CODEX } else { &ICON_CLAUDE }, x0, y, None);
            let name_c = if p.ko {
                c::KO
            } else if w && blink {
                c::GOLD
            } else {
                c::TEXT
            };
            g.text(&p.name, x0 + 10, y, name_c, 1, SH);
            let hp_c = if p.hp <= 0.0 {
                c::RED
            } else if p.hp <= HP_MAX * CRITICAL {
                c::GOLD
            } else {
                c::TEXT
            };
            g.text_r(&((p.hp + 0.5) as i32).to_string(), x0 + 71, y, hp_c, SH);
            g.text_r(&fmt_tok(p.dealt), x0 + 111, y, c::DIM, SH);
            let (bx, bw) = (x0 + 118, 62);
            g.rect(bx - 1, y, bw + 2, 7, c::OUTER);
            g.rect(bx, y + 1, bw, 5, c::ATB_OFF);
            if p.ko {
                g.text("KO", bx + bw / 2 - 6, y, c::RED, 1, Some(c::OUTER));
            } else if w {
                g.rect(bx, y + 1, bw, 5, if blink { c::ATB_FULL } else { c::ATB_FILL });
            } else {
                let fill = iround(bw as f32 * p.atb);
                g.rect(bx, y + 1, fill, 5, if p.state == State::Work { c::ATB_FILL } else { c::ATB_IDLE });
                if p.state == State::Work {
                    g.rect(bx, y + 1, fill, 1, c::ATB_FULL);
                }
            }
            let st = if p.ko {
                ""
            } else if w {
                if p.why == Why::Ask { "?" } else { "!" }
            } else if p.state == State::Idle {
                "z"
            } else {
                ""
            };
            if !st.is_empty() {
                g.text(st, bx + bw + 4, y, if w { c::GOLD } else { c::DIM }, 1, SH);
            }
            if w || self.card == p.order {
                g.blit(&ICON_CURSOR, PARTY_X + 3 + (self.t * 4.0) as i32 % 2, y - 1, None);
            }
        }
        // sessions past the eighth wait off the field: a tab on the window's edge
        let bench = self.pl.iter().filter(|p| p.used && p.bench && !p.leaving).count();
        if bench > 0 {
            let s = format!("+{bench} WAIT");
            let w = s.len() as i32 * 6 + 12;
            g.window(BENCH_X, WIN_Y - 13, w, 16);
            g.text(&s, BENCH_X + 7, WIN_Y - 9, c::GOLD, 1, SH);
        }
    }

    fn draw_info_window(&self, g: &mut Gfx) {
        g.window(INFO_X, WIN_Y, INFO_W, WIN_H);
        let (x0, xr) = (INFO_X + 7, INFO_X + INFO_W - 7);
        let mut y = ROW_Y;
        g.text("GOLD", x0, y, c::DIM, 1, SH);
        let mut buf =
            if self.have_usage { format!("${}", (self.usage.usd + 0.5).floor() as i64) } else { "--".to_string() };
        g.text_r(&buf, xr, y, c::TEXT, SH);
        y += 10;
        g.text("EXP", x0, y, c::DIM, 1, SH);
        if self.have_usage {
            buf = fmt_tok(self.usage.tokens);
        }
        g.text_r(&buf, xr, y, c::TEXT, SH);
        y += 10;
        let (h, m) = self.local_hm();
        let buf = if self.have_time {
            format!("{h:02}{}{m:02}", if (self.t * 2.0) as i32 % 2 != 0 { ':' } else { ' ' })
        } else {
            "--:--".to_string()
        };
        g.text("TIME", x0, y, c::DIM, 1, SH);
        g.text_r(&buf, xr, y, c::TEXT, SH);
        y += 12;
        g.rect(x0, y - 2, INFO_W - 14, 1, c::BORDER_LO);
        g.text("MP", x0, y, c::GOLD, 1, SH);
        if self.muted {
            g.text("MUTE", xr - 23, y, c::DIM, 1, SH);
        }
        y += 9;
        let bars = [
            (false, "5H", self.mana.claude_5h, c::MANA5H),
            (false, "WK", self.mana.claude_week, c::MANA_WEEK),
            (true, "WK", self.mana.codex_week, c::MANA_CODEX),
        ];
        for (codex, lbl, v, col) in bars {
            g.blit(if codex { &ICON_CODEX } else { &ICON_CLAUDE }, x0, y, None);
            g.text(lbl, x0 + 9, y, c::TEXT, 1, SH);
            let (bx, bw) = (x0 + 22, INFO_W - 14 - 22);
            if v < 0 {
                g.text("--", bx + bw / 2 - 5, y, c::DIM, 1, SH);
            } else {
                let v = v.min(100);
                g.rect(bx - 1, y + 1, bw + 2, 5, c::OUTER);
                let fill = if v < 10 {
                    c::RED
                } else if v < 25 {
                    c::GOLD
                } else {
                    col
                };
                g.rect(bx, y + 2, iround(bw as f32 * v as f32 / 100.0), 3, fill);
            }
            y += 9;
        }
        // the whole window opens the config window; the gear says so
        g.blit(&ICON_GEAR, INFO_X + INFO_W - 15, FF_H as i32 - 15, None);
    }

    fn draw_banner(&self, g: &mut Gfx) {
        let b = &self.banner;
        if !b.on {
            return;
        }
        let (tl, sl) = (b.text.len() as i32, b.sub.len() as i32);
        let w = (tl.max(sl) * 6 + 18).max(60);
        let (x, h) = ((FF_W as i32 - w) / 2, if sl > 0 { 26 } else { 17 });
        g.window(x, 3, w, h);
        g.text(&b.text, x + (w - tl * 6) / 2, 8, c::TEXT, 1, SH);
        if sl > 0 {
            g.text(&b.sub, x + (w - sl * 6) / 2, 17, c::GOLD, 1, SH);
        }
    }

    /* ---- the status card ------------------------------------------------------------- */

    /// s word-wrapped at x, y in at most max lines of cols characters. Too
    /// long, it ends at the last sentence end on its last two lines, or with "...".
    fn draw_wrapped(g: &mut Gfx, s: &str, x: i32, y: i32, max: usize, cols: usize, c: u16) -> usize {
        let b = s.as_bytes();
        let (mut ln, cut_off) = wrap(b, max.min(8), cols);
        let mut rows = ln.len();
        let mut dots = false;
        if cut_off {
            let mut end = None;
            let mut r = rows as i32 - 1;
            while r >= 0 && r >= rows as i32 - 2 {
                let (start, len) = ln[r as usize];
                for i in (0..len).rev() {
                    let p = start + i;
                    if b".!?".contains(&b[p]) && b.get(p + 1) == Some(&b' ') {
                        end = Some(i + 1);
                        break;
                    }
                }
                if end.is_some() {
                    break;
                }
                r -= 1;
            }
            if let Some(end) = end {
                rows = r as usize + 1;
                ln[r as usize].1 = end;
            } else {
                let l = &mut ln[rows - 1];
                if l.1 > cols - 3 {
                    let mut len = cols - 3;
                    while len > 0 && b[l.0 + len] != b' ' {
                        len -= 1;
                    }
                    l.1 = if len > 0 { len } else { cols - 3 };
                }
                while l.1 > 0 && b" ,;:-".contains(&b[l.0 + l.1 - 1]) {
                    l.1 -= 1;
                }
                dots = true;
            }
        }
        for (r, &(start, len)) in ln.iter().enumerate().take(rows) {
            let line = format!("{}{}", &s[start..start + len], if dots && r == rows - 1 { "..." } else { "" });
            g.text(&line, x, y + r as i32 * SAID_PITCH, c, 1, SH);
        }
        rows
    }

    fn card_row(g: &mut Gfx, r: i32, label: &str, value: &str, c: u16) {
        let y = CARD_ROW_Y + r * CARD_ROW_H;
        g.text(label, CARD_COL_X, y, c::DIM, 1, SH);
        g.text(value, CARD_COL_X + 60, y, c, 1, SH);
    }

    fn state_text(p: &Player) -> &'static str {
        if p.ko {
            "KO"
        } else if p.state == State::Wait {
            if p.why == Why::Ask { "HAS A QUESTION" } else { "NEEDS YOUR OK!" }
        } else if p.state == State::Work {
            "FIGHTING"
        } else {
            "IDLE"
        }
    }

    fn draw_card(&self, g: &mut Gfx) {
        let Some(pi) = self.card_player() else { return };
        let p = &self.pl[pi];
        let cls = &CLASSES[p.cls];
        g.window(CARD_X, CARD_Y, CARD_W, CARD_H);
        // the portrait: the player at 2x
        let lit = self.term_at != 0 && p.order == self.term_order && self.ms - self.term_at < TERM_FLASH_MS;
        let (fx, fy) = (CARD_PORT_X, CARD_PORT_Y);
        g.rect(fx, fy, CARD_PORT_W, CARD_PORT_H, if lit { c::GOLD } else { c::BORDER_LO });
        g.rect(fx + 1, fy + 1, CARD_PORT_W - 2, CARD_PORT_H - 2, c::WIN1);
        g.clip(fx + 1, fy + 1, fx + CARD_PORT_W - 1, fy + CARD_PORT_H - 1);
        let pose = if p.state == State::Wait && !p.ko {
            Pose::Raise
        } else if p.ko || p.hp <= HP_MAX * CRITICAL {
            Pose::Kneel
        } else {
            Pose::Ready
        };
        let img = &cls.img[p.variant][pose as usize];
        let (iw, ih) = (img.w as i32, img.h as i32);
        g.blit_scaled(img, fx + (CARD_PORT_W - iw * 2) / 2, fy + CARD_PORT_H - 2 - ih * 2, 2, false);
        g.noclip();
        if p.term {
            let (bx, by) = (fx + CARD_PORT_W - 11, fy + CARD_PORT_H - 8);
            g.rect(bx, by, 15, 11, if lit { c::GOLD } else { c::BORDER });
            g.rect(bx + 1, by + 1, 13, 9, c::WIN1);
            g.text(">_", bx + 2, by + 2, c::CODEX_HI, 1, None);
        }
        g.text(&p.name, CARD_COL_X, CARD_Y + 12, c::GOLD, 2, SH);
        g.text(cls.job, CARD_COL_X, CARD_Y + 31, c::TEXT, 1, SH);
        let (tool, tool_c) = if p.codex { ("CODEX", c::CODEX_HI) } else { ("CLAUDE CODE", c::CLAUDE) };
        g.text(tool, CARD_COL_X + text_width(cls.job, 1) + 12, CARD_Y + 31, tool_c, 1, SH);
        Self::card_row(g, 0, "PROJECT", &folder_name(&p.proj, 38), c::TEXT);
        let v = format!("{}  {}", Self::state_text(p), fmt_span((self.ms - p.since_ms) / 1000));
        let state_c = if p.ko {
            c::RED
        } else if p.state == State::Wait {
            c::GOLD
        } else {
            c::TEXT
        };
        Self::card_row(g, 1, "STATE", &v, state_c);
        let hp_c = if p.hp <= 0.0 {
            c::RED
        } else if p.hp <= HP_MAX * CRITICAL {
            c::GOLD
        } else {
            c::TEXT
        };
        Self::card_row(g, 2, "HP", &format!("{}/{}", (p.hp + 0.5) as i32, HP_MAX as i32), hp_c);
        let dmg = fmt_tok(p.dealt + p.pending);
        let v = match self.mon_at(self.target(pi)) {
            Some(m) => format!("{dmg}  ON {}", MONSTERS[self.mon[m].kind].name),
            None => dmg,
        };
        Self::card_row(g, 3, "DAMAGE", &v, c::TEXT);
        let v = if p.tokens >= 0.0 {
            format!("{} TOKENS  {}", fmt_tok(p.tokens), fmt_usd(p.usd))
        } else {
            "--".to_string()
        };
        Self::card_row(g, 4, "SPENT", &v, c::TEXT);
        let v = if p.have_started { format!("UP {}", fmt_span((self.ms - p.started_ms) / 1000)) } else { "--".into() };
        Self::card_row(g, 5, "SESSION", &v, c::TEXT);
        // the last message
        let title = match p.said_kind {
            SaidKind::Msg => "LAST MESSAGE",
            SaidKind::Ask => "ASKS YOU",
            SaidKind::Tool => "WANTS TO USE",
        };
        g.rect(CARD_X + 10, SAID_Y, CARD_W - 20, 1, c::BORDER_LO);
        g.text(title, CARD_X + 12, SAID_Y + 5, if p.said_kind == SaidKind::Msg { c::DIM } else { c::GOLD }, 1, SH);
        if p.have_said_at && !p.said.is_empty() {
            let ago = format!("{} AGO", fmt_age((self.ms - p.said_ms) / 1000));
            g.text_r(&ago, CARD_X + CARD_W - 12, SAID_Y + 5, c::DIM, SH);
        }
        if Self::draw_wrapped(g, &p.said, CARD_X + 12, SAID_Y + 17, SAID_ROWS, SAID_COLS, c::TEXT) == 0 {
            g.text("--", CARD_X + 12, SAID_Y + 17, c::DIM, 1, SH);
        }
    }

    /* ---- the config window -------------------------------------------------------------- */

    fn button(g: &mut Gfx, x: i32, y: i32, label: &str) {
        g.rect(x, y, CFG_BTN_W, CFG_BTN_H, c::BORDER);
        g.rect(x + 1, y + 1, CFG_BTN_W - 2, CFG_BTN_H - 2, c::WIN1);
        g.text(label, x + (CFG_BTN_W - text_width(label, 1)) / 2, y + 3, c::TEXT, 1, SH);
    }

    fn draw_config(&self, g: &mut Gfx) {
        if !self.config {
            return;
        }
        g.window(CFG_X, CFG_Y, CFG_W, CFG_H);
        g.text("CONFIG", CFG_X + 10, CFG_Y + 9, c::GOLD, 1, SH);
        let vx = CFG_X + 64;
        let rows = [
            (
                "SCREEN",
                if self.fullscreen { "FULL" } else { "WINDOW" },
                if self.fullscreen { "WINDOW" } else { "FULL" },
            ),
            ("SOUND", if self.muted { "OFF" } else { "ON" }, if self.muted { "UNMUTE" } else { "MUTE" }),
            (
                "PACE",
                if self.pace_active { "ACTIVE" } else { "WAIT" },
                if self.pace_active { "WAIT" } else { "ACTIVE" },
            ),
        ];
        for (r, (label, value, button)) in rows.into_iter().enumerate() {
            let y = cfg_row_y(r as i32);
            g.text(label, CFG_X + 10, y + 3, c::DIM, 1, SH);
            g.text(value, vx, y + 3, c::TEXT, 1, SH);
            Self::button(g, CFG_BTN_X, y, button);
        }
        Self::button(g, CFG_BTN_X, CFG_CLOSE_Y, "CLOSE");
    }

    /* ---- render ------------------------------------------------------------------------ */

    /// Draw the canvas: W x H RGB565 pixels.
    pub fn render(&mut self, fb: &mut [u16]) {
        let mut g = Gfx::new(fb);
        let tod = self.tod_now() as usize;
        if self.tod_shown != Some(tod) {
            let img = &FIELDS[tod];
            for (o, &v) in self.field.iter_mut().zip(img.px()) {
                *o = img.pal[v as usize];
            }
            self.tod_shown = Some(tod);
        }
        g.fb()[..FIELD_W * FIELD_H as usize].copy_from_slice(&self.field);
        if self.dim > 0.0 {
            g.blend_rect(0, 0, FF_W as i32, FIELD_H, hex(0x06061a), k256(self.dim));
        }
        self.dim = 0.0;
        // monsters and players, back to front
        let mut ents: Vec<(f32, bool, usize)> = Vec::with_capacity(MAX_MONS + MAX_PLAYERS);
        for (i, m) in self.mon.iter().enumerate() {
            if m.used {
                ents.push((m.y, false, i));
            }
        }
        for (i, p) in self.pl.iter().enumerate() {
            if p.used && !p.bench {
                ents.push((if p.air { p.hy } else { p.y }, true, i));
            }
        }
        ents.sort_by(|a, b| a.0.total_cmp(&b.0));
        g.clip(0, 0, FF_W as i32, FIELD_H);
        for &(_, player, i) in &ents {
            if player {
                self.draw_player(&mut g, &self.pl[i]);
            } else {
                self.draw_monster(&mut g, &self.mon[i]);
            }
        }
        g.noclip();
        for i in 0..self.fx.len() {
            let e = self.fx[i];
            if e.kind != Ek::Tap {
                self.draw_fx(&mut g, &e);
            }
        }
        for u in &self.nums {
            let bounce = if u.t < 0.3 { -(u.t / 0.3 * PI).sin() * 7.0 } else { 0.0 };
            g.num(&u.s, u.x as i32, iround(u.y + bounce * u.k as f32), u.c, u.k);
        }
        if self.flash > 0.0 {
            g.blend_rect(0, 0, FF_W as i32, FIELD_H, c::WHITE, k256(self.flash));
        }
        g.rect(0, FIELD_H - 1, FF_W as i32, 1, c::OUTER);
        self.draw_mon_window(&mut g);
        self.draw_party_window(&mut g);
        self.draw_info_window(&mut g);
        self.draw_banner(&mut g);
        self.draw_card(&mut g);
        self.draw_config(&mut g);
        for i in 0..self.fx.len() {
            let e = self.fx[i];
            if e.kind == Ek::Tap {
                self.draw_fx(&mut g, &e);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folders_and_spans_read_short() {
        assert_eq!(folder_name("github.com/someone/moon-garden", 18), "moon-garden");
        assert_eq!(folder_name("~/projects/a-very-long-folder-name-indeed/", 18), "a-very-long-fold..");
        assert_eq!(fmt_span(3 * 3600 + 5 * 60), "3H 05M");
        assert_eq!(fmt_usd(3.456), "$3.46");
        assert_eq!(fmt_usd(12_345.0), "$12K");
    }

    #[test]
    fn wrap_breaks_at_words() {
        let (rows, cut) = wrap(b"one two three four", 2, 8);
        assert_eq!(rows, vec![(0, 7), (8, 5)]); // "one two", "three"
        assert!(cut);
    }
}
