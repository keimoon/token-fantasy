//! The window: ticks the battle in fixed 33 ms steps, renders the 426x266
//! canvas and shows it at the largest whole-number scale that fits,
//! letterboxed, so the pixel art stays crisp at any size or DPI. Only the
//! part of the window that changed since the buffer was last shown is
//! repainted and reported to the compositor. A click is a tap; the
//! scene's events become sounds, settings and terminals brought up.

use std::collections::{HashMap, VecDeque};
use std::error::Error;
use std::num::NonZeroU32;
use std::rc::Rc;
use std::sync::mpsc::Receiver;
use std::thread;
use std::time::{Duration, Instant};

use softbuffer::{Context, Rect, Surface};
use token_fantasy::gfx::{H, W, rgb};
use token_fantasy::scene::{Scene, ev};
use token_fantasy::settings::Settings;
use token_fantasy::sound::{Sound, Speaker};
use token_fantasy::terminal::{Terminal, bring_up};
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalPosition};
use winit::event::{ElementState, KeyEvent, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{Key, NamedKey};
#[cfg(target_os = "linux")]
use winit::platform::wayland::WindowAttributesExtWayland;
use winit::window::{Fullscreen, Window, WindowId};

use crate::{Update, log};

const FRAME_MS: u64 = 33;
const FRAME: Duration = Duration::from_millis(FRAME_MS);
/// Presents remembered for repainting an older buffer (Wayland keeps two).
const HISTORY: usize = 4;

/// A half-open box of frame pixels.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Area {
    x0: usize,
    y0: usize,
    x1: usize,
    y1: usize,
}

impl Area {
    const ALL: Area = Area { x0: 0, y0: 0, x1: W, y1: H };

    fn union(self, o: Area) -> Area {
        Area { x0: self.x0.min(o.x0), y0: self.y0.min(o.y0), x1: self.x1.max(o.x1), y1: self.y1.max(o.y1) }
    }
}

/// The pixels that differ between two frames, or None.
fn diff(a: &[u32], b: &[u32]) -> Option<Area> {
    let mut area: Option<Area> = None;
    for (y, (ra, rb)) in a.as_chunks::<W>().0.iter().zip(b.as_chunks::<W>().0.iter()).enumerate() {
        let Some(x0) = ra.iter().zip(rb).position(|(p, q)| p != q) else { continue };
        let x1 = W - ra.iter().zip(rb).rev().position(|(p, q)| p != q).unwrap();
        let row = Area { x0, y0: y, x1, y1: y + 1 };
        area = Some(area.map_or(row, |a| a.union(row)));
    }
    area
}

/// Where the frame sits in a w x h window: scale k, offset (ox, oy).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Fit {
    k: usize,
    ox: usize,
    oy: usize,
}

fn fit(w: usize, h: usize) -> Fit {
    let k = (w / W).min(h / H).max(1);
    Fit { k, ox: w.saturating_sub(W * k) / 2, oy: h.saturating_sub(H * k) / 2 }
}

/// The canvas pixel under a window position, if it is on the frame.
fn to_canvas(w: usize, h: usize, pos: PhysicalPosition<f64>) -> Option<(i32, i32)> {
    let f = fit(w, h);
    let (x, y) = (pos.x - f.ox as f64, pos.y - f.oy as f64);
    if x < 0.0 || y < 0.0 {
        return None;
    }
    let (cx, cy) = ((x / f.k as f64) as usize, (y / f.k as f64) as usize);
    (cx < W && cy < H).then_some((cx as i32, cy as i32))
}

/// Paint one area of the frame into a w x h window buffer, scaled by the fit;
/// a full paint also blacks out the letterbox. Returns the window pixels
/// touched, or None when the area falls outside the window.
fn paint(fb: &[u32], out: &mut [u32], w: usize, h: usize, area: Area, full: bool) -> Option<Rect> {
    let f = fit(w, h);
    if full {
        out.fill(0);
    }
    let (x0, x1) = ((f.ox + area.x0 * f.k).min(w), (f.ox + area.x1 * f.k).min(w));
    let (y0, y1) = ((f.oy + area.y0 * f.k).min(h), (f.oy + area.y1 * f.k).min(h));
    if x0 >= x1 || y0 >= y1 {
        return None;
    }
    let mut row = vec![0u32; x1 - x0];
    let mut row_of = usize::MAX;
    for y in y0..y1 {
        let sy = (y - f.oy) / f.k;
        if sy != row_of {
            let src = &fb[sy * W..(sy + 1) * W];
            for (i, px) in row.iter_mut().enumerate() {
                *px = src[(x0 + i - f.ox) / f.k];
            }
            row_of = sy;
        }
        out[y * w + x0..y * w + x1].copy_from_slice(&row);
    }
    let nz = |v: usize| NonZeroU32::new(v as u32).unwrap();
    Some(Rect { x: x0 as u32, y: y0 as u32, width: nz(x1 - x0), height: nz(y1 - y0) })
}

struct Shown {
    window: Rc<Window>,
    surface: Surface<Rc<Window>, Rc<Window>>,
}

struct App {
    rx: Receiver<Update>,
    scale: u32,
    host: i64,
    shown: Option<Shown>,
    scene: Scene,
    settings: Settings,
    speaker: Speaker,
    /// The terminals of the sessions in the last list, by session id.
    terms: HashMap<String, Terminal>,
    canvas: Vec<u16>,
    /// RGB565 -> 0x00RRGGBB.
    lut: Vec<u32>,
    fb: Vec<u32>,
    on_screen: Vec<u32>,
    /// What each recent present changed, newest last; None = everything.
    history: VecDeque<Option<Area>>,
    size: (usize, usize),
    cursor: Option<PhysicalPosition<f64>>,
    /// Real time handed to the scene so far, in whole milliseconds.
    last: Instant,
    acc: u64,
    next: Instant,
    stale: bool,
    occluded: bool,
}

pub fn run(rx: Receiver<Update>, scale: u32, host: i64, settings: Settings) -> Result<(), Box<dyn Error>> {
    let event_loop = EventLoop::new()?;
    let now = Instant::now();
    let mut scene = Scene::new((chrono::Local::now().timestamp_subsec_nanos() ^ std::process::id()) | 1);
    scene.set_muted(settings.muted);
    scene.set_pace_active(settings.pace_active);
    let mut app = App {
        rx,
        scale,
        host,
        shown: None,
        scene,
        settings,
        speaker: Speaker::new(settings.volume),
        terms: HashMap::new(),
        canvas: vec![0; W * H],
        lut: (0..=u16::MAX).map(rgb).collect(),
        fb: vec![0; W * H],
        on_screen: vec![0; W * H],
        history: VecDeque::new(),
        size: (0, 0),
        cursor: None,
        last: now,
        acc: 0,
        next: now,
        stale: true,
        occluded: false,
    };
    event_loop.run_app(&mut app)?;
    Ok(())
}

impl App {
    /// Apply what the feeders sent, advance the scene, act on its events and render it.
    fn step(&mut self) {
        while let Ok(u) = self.rx.try_recv() {
            match u {
                Update::Sessions { list, tools, terms } => {
                    self.scene.set_sessions(&list, self.host);
                    for (id, tool) in tools {
                        self.scene.tool(&id, tool);
                    }
                    self.terms = terms;
                }
                Update::Usage(u) => self.scene.set_usage(u),
                Update::Mana(m) => self.scene.set_mana(m),
            }
        }
        let local = chrono::Local::now();
        self.scene.set_time(local.timestamp(), local.offset().local_minus_utc());
        let now = Instant::now();
        let ms = (now - self.last).as_millis() as u64;
        self.last += Duration::from_millis(ms);
        self.acc = (self.acc + ms).min(1000); // after a stall, do not fast-forward
        while self.acc >= FRAME_MS {
            self.scene.tick(FRAME_MS as u32);
            self.acc -= FRAME_MS;
        }
        self.events();
        self.scene.render(&mut self.canvas);
        for line in self.scene.take_log() {
            log(&line);
        }
        for (o, &c) in self.fb.iter_mut().zip(&self.canvas) {
            *o = self.lut[c as usize];
        }
        if let Some(s) = &self.shown
            && !self.occluded
            && (self.stale || self.fb != self.on_screen)
        {
            s.window.request_redraw();
        }
    }

    /// What the scene asked for: sounds, settings, a terminal.
    fn events(&mut self) {
        let e = self.scene.take_events();
        if e & ev::MUTE != 0 {
            self.settings.muted = !self.settings.muted;
            self.settings.save();
            self.scene.set_muted(self.settings.muted);
            log(if self.settings.muted { "sound muted" } else { "sound on" });
            if !self.settings.muted {
                self.speaker.play(Sound::Blip);
            }
        }
        if e & ev::PACE != 0 {
            self.settings.pace_active = !self.settings.pace_active;
            self.settings.save();
            self.scene.set_pace_active(self.settings.pace_active);
            log(if self.settings.pace_active { "pace set to active" } else { "pace set to wait" });
        }
        if e & ev::FULLSCREEN != 0 {
            self.toggle_fullscreen();
        }
        if !self.settings.muted {
            let sounds = [
                (ev::CHIME_PERM, Sound::Perm),
                (ev::CHIME_ASK, Sound::Ask),
                (ev::FANFARE, Sound::Fanfare),
                (ev::REVIVE, Sound::Revive),
                (ev::BLIP, Sound::Blip),
                (ev::BLIP_LOW, Sound::BlipLow),
            ];
            let mut chimed = false;
            for (bit, sound) in sounds {
                // one chime at a time: "!" wins over "?"
                if e & bit != 0 && !(chimed && bit == ev::CHIME_ASK) {
                    self.speaker.play(sound);
                    chimed |= bit == ev::CHIME_PERM;
                }
            }
        }
        if e & ev::TERMINAL != 0
            && let Some(id) = self.scene.terminal_id()
        {
            match self.terms.get(&id).cloned() {
                Some(t) => {
                    thread::spawn(move || match bring_up(&t) {
                        Ok(()) => log(&format!("brought up the terminal of {id}")),
                        Err(e) => log(&format!("could not bring up the terminal of {id}: {e}")),
                    });
                }
                None => log(&format!("click on a portrait: no terminal known for {id}")),
            }
        }
    }

    fn toggle_fullscreen(&mut self) {
        let Some(s) = &self.shown else { return };
        let full = s.window.fullscreen().is_none();
        s.window.set_fullscreen(full.then_some(Fullscreen::Borderless(None)));
        self.scene.set_fullscreen(full);
    }

    fn present(&mut self) {
        let Some(s) = &mut self.shown else { return };
        let px = s.window.inner_size();
        let (Some(nw), Some(nh)) = (NonZeroU32::new(px.width), NonZeroU32::new(px.height)) else { return };
        let size = (px.width as usize, px.height as usize);
        if size != self.size {
            self.size = size;
            self.stale = true;
        }
        let changed = if self.stale { Some(None) } else { diff(&self.on_screen, &self.fb).map(Some) };
        let Some(changed) = changed else { return };
        if let Err(e) = s.surface.resize(nw, nh) {
            eprintln!("token-fantasy: cannot resize the surface: {e}");
            return;
        }
        let mut buf = match s.surface.buffer_mut() {
            Ok(b) => b,
            Err(e) => {
                eprintln!("token-fantasy: no surface buffer: {e}");
                return;
            }
        };
        // The buffer shows the frame from `age` presents ago (0 = unknown):
        // repaint what changed in the presents since, plus this frame.
        let age = buf.age() as usize;
        let mut area = changed;
        if age == 0 || age > self.history.len() + 1 {
            area = None;
        } else {
            for past in self.history.iter().rev().take(age - 1) {
                area = match (area, past) {
                    (Some(a), Some(p)) => Some(a.union(*p)),
                    _ => None,
                };
            }
        }
        let (w, h) = size;
        let result = match paint(&self.fb, &mut buf, w, h, area.unwrap_or(Area::ALL), area.is_none()) {
            Some(damage) if area.is_some() => buf.present_with_damage(&[damage]),
            _ => buf.present(),
        };
        if let Err(e) = result {
            eprintln!("token-fantasy: cannot present: {e}");
            self.history.clear();
            self.stale = true;
            return;
        }
        self.history.push_back(changed);
        if self.history.len() > HISTORY {
            self.history.pop_front();
        }
        self.on_screen.copy_from_slice(&self.fb);
        self.stale = false;
    }

    fn key(&mut self, el: &ActiveEventLoop, key: Key) {
        let Some(s) = &self.shown else { return };
        match key.as_ref() {
            Key::Character("q") => el.exit(),
            Key::Character("f") | Key::Named(NamedKey::F11) => self.toggle_fullscreen(),
            Key::Character("m") => {
                self.settings.muted = !self.settings.muted;
                self.settings.save();
                self.scene.set_muted(self.settings.muted);
                log(if self.settings.muted { "sound muted" } else { "sound on" });
            }
            Key::Named(NamedKey::Escape) => {
                if !self.scene.dismiss() && s.window.fullscreen().is_some() {
                    self.toggle_fullscreen();
                }
            }
            Key::Character(c) => {
                if let Some(k) = c.parse::<u32>().ok().filter(|k| (1..=6).contains(k)) {
                    let _ = s.window.request_inner_size(LogicalSize::new(W as u32 * k, H as u32 * k));
                }
            }
            _ => {}
        }
    }

    fn click(&mut self) {
        let (Some(pos), (w, h)) = (self.cursor, self.size) else { return };
        if let Some((x, y)) = to_canvas(w, h, pos) {
            self.scene.tap(x, y);
        }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        if self.shown.is_some() {
            return;
        }
        let attrs = Window::default_attributes()
            .with_title("Token Fantasy")
            .with_inner_size(LogicalSize::new(W as u32 * self.scale, H as u32 * self.scale))
            .with_min_inner_size(LogicalSize::new(W as u32, H as u32));
        // app_id (Wayland) / WM_CLASS (X11) "token-fantasy", for window rules
        #[cfg(target_os = "linux")]
        let attrs = WindowAttributesExtWayland::with_name(attrs, "token-fantasy", "token-fantasy");
        let made = el.create_window(attrs).map_err(|e| e.to_string()).and_then(|window| {
            let window = Rc::new(window);
            let context = Context::new(window.clone()).map_err(|e| e.to_string())?;
            let surface = Surface::new(&context, window.clone()).map_err(|e| e.to_string())?;
            Ok(Shown { window, surface })
        });
        match made {
            Ok(s) => {
                s.window.request_redraw();
                self.shown = Some(s);
                self.stale = true;
            }
            Err(e) => {
                eprintln!("token-fantasy: cannot open the window: {e}");
                el.exit();
            }
        }
    }

    fn window_event(&mut self, el: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => el.exit(),
            WindowEvent::RedrawRequested => self.present(),
            WindowEvent::Resized(px) => {
                self.size = (px.width as usize, px.height as usize);
                self.stale = true;
                if let Some(s) = &self.shown {
                    // the compositor may have changed it (a fullscreen key binding)
                    self.scene.set_fullscreen(s.window.fullscreen().is_some());
                    s.window.request_redraw();
                }
            }
            WindowEvent::ScaleFactorChanged { .. } => {
                self.stale = true;
                if let Some(s) = &self.shown {
                    s.window.request_redraw();
                }
            }
            WindowEvent::Occluded(o) => {
                self.occluded = o;
                self.stale = true;
            }
            WindowEvent::CursorMoved { position, .. } => self.cursor = Some(position),
            WindowEvent::CursorLeft { .. } => self.cursor = None,
            WindowEvent::MouseInput { state: ElementState::Pressed, button: MouseButton::Left, .. } => self.click(),
            WindowEvent::KeyboardInput {
                event: KeyEvent { logical_key, state: ElementState::Pressed, repeat: false, .. },
                ..
            } => self.key(el, logical_key),
            _ => {}
        }
    }

    fn about_to_wait(&mut self, el: &ActiveEventLoop) {
        let now = Instant::now();
        if now >= self.next {
            self.step();
            self.next += FRAME;
            if self.next < now {
                self.next = now + FRAME;
            }
        }
        el.set_control_flow(ControlFlow::WaitUntil(self.next));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame() -> Vec<u32> {
        (0..W * H).map(|i| (i * 7 + i / W) as u32 & 0xffff).collect()
    }

    #[test]
    fn fit_letterboxes_at_whole_scales() {
        assert_eq!(fit(2560, 1600), Fit { k: 6, ox: 2, oy: 2 });
        assert_eq!(fit(1278, 798), Fit { k: 3, ox: 0, oy: 0 });
        assert_eq!(fit(200, 100), Fit { k: 1, ox: 0, oy: 0 }); // smaller than the frame: clipped
    }

    #[test]
    fn clicks_land_on_canvas_pixels() {
        let (w, h) = (1300, 820); // k = 3, offset (11, 11)
        assert_eq!(to_canvas(w, h, PhysicalPosition::new(11.0, 11.0)), Some((0, 0)));
        assert_eq!(
            to_canvas(w, h, PhysicalPosition::new(11.0 + 3.0 * 425.0 + 2.9, 11.0 + 3.0 * 265.0)),
            Some((425, 265))
        );
        assert_eq!(to_canvas(w, h, PhysicalPosition::new(5.0, 400.0)), None);
        assert_eq!(to_canvas(w, h, PhysicalPosition::new(1295.0, 400.0)), None);
    }

    #[test]
    fn diff_finds_the_changed_box() {
        let a = frame();
        let mut b = a.clone();
        assert_eq!(diff(&a, &b), None);
        b[10 * W + 20] ^= 1;
        b[30 * W + 5] ^= 1;
        assert_eq!(diff(&a, &b), Some(Area { x0: 5, y0: 10, x1: 21, y1: 31 }));
    }

    #[test]
    fn partial_paint_equals_full_paint() {
        let (w, h) = (1300, 820);
        let old = frame();
        let mut new = old.clone();
        for y in 40..60 {
            for x in 100..130 {
                new[y * W + x] = 0xff0000;
            }
        }
        let mut full = vec![7u32; w * h];
        paint(&new, &mut full, w, h, Area::ALL, true);
        let mut part = vec![7u32; w * h];
        paint(&old, &mut part, w, h, Area::ALL, true);
        let area = diff(&old, &new).unwrap();
        let damage = paint(&new, &mut part, w, h, area, false).unwrap();
        assert!(full == part, "partial repaint differs from a full one");
        let f = fit(w, h);
        assert_eq!((damage.x, damage.y), ((f.ox + 100 * f.k) as u32, (f.oy + 40 * f.k) as u32));
        assert_eq!((damage.width.get(), damage.height.get()), (30 * f.k as u32, 20 * f.k as u32));
        // corners of the scaled frame and the letterbox
        assert_eq!(full[f.oy * w + f.ox], new[0]);
        assert_eq!(full[(f.oy + H * f.k - 1) * w + f.ox + W * f.k - 1], new[W * H - 1]);
        assert_eq!(full[0], 0);
    }
}
