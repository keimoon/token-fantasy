//! Run a battle scenario without a window and save frames as PNG, with a
//! simulated laptop whose sessions burn tokens, call tools and change state:
//!   cargo run --release --example preview -- <scenario> <out-prefix> <seconds>... [-- tap_x tap_y ...]
//! Scenarios:
//!   demo        six sessions mid-fight that keep working, idling and asking
//!   full        ten sessions over five projects: 8 on the field, 2 waiting
//!   mages       the four mages (Claude Code), each using its special in turn, 3 s apart
//!   fighters    the four fighters (Codex), likewise
//!   ko          an idle session knocked out, then woken by a Phoenix Plume
//!   finish1     the finishers of goblin, bomb, cactoid and grudgling, 4 s apart (at 2, 6, 10, 14 s)
//!   finish2     those of ahriman, stenchvine, stormcat and behemoth
//!   leave       a project's last session ends: its monster is defeated
//!   limits1     four mages in four projects leave 4 s apart (at 1, 5, 9, 13 s): each finishes
//!               its project's monster with its final blow
//!   limits2     the same with the four fighters
//! Frames go to <out-prefix>-<seconds>.png at 2x (SCALE=n to change);
//! HOUR=h sets the local hour (the sky). Clicks (canvas coordinates) land at
//! 1 s. Battle events print as they happen.

use std::fs::File;
use std::io::BufWriter;

use token_fantasy::gfx::{H, W, rgb};
use token_fantasy::scene::Scene;
use token_fantasy::session::{Mana, SaidKind, Session, State, Tool, Usage, Why};

struct Sim {
    s: Session,
    live: bool,
    since: f64,
}

struct Laptop {
    sim: Vec<Sim>,
    rng: u32,
    now: f64,
    demo: bool,
}

impl Laptop {
    fn rnd(&mut self) -> f32 {
        self.rng = self.rng.wrapping_mul(1664525).wrapping_add(1013904223);
        (self.rng >> 8) as f32 / 16777216.0
    }

    fn add(&mut self, id: &str, proj: &str, codex: bool, st: State, why: Why, age: f64) {
        let tokens = 52e6 * self.rnd() as f64;
        let said = if st == State::Wait {
            "Which registry should CI use? US Pesde registry (Recommended) / Public registry"
        } else {
            "The patron card now shows what the agent said last. The text is the opening of its last reply, \
             cleaned of markdown, and the card wraps it into seven lines. Nothing else changed."
        };
        self.sim.push(Sim {
            s: Session {
                id: id.into(),
                proj: proj.into(),
                codex,
                term: true,
                state: st,
                why,
                age_s: 0,
                up_s: 5400,
                burn: 0.0,
                tokens,
                usd: tokens / 2.4e6,
                said: said.into(),
                said_kind: if st == State::Wait { SaidKind::Ask } else { SaidKind::Msg },
                said_age_s: 95,
            },
            live: true,
            since: self.now - age,
        });
    }

    fn send_list(&self, scene: &mut Scene) {
        let list: Vec<Session> = self
            .sim
            .iter()
            .filter(|x| x.live)
            .map(|x| Session { age_s: (self.now - x.since) as u32, ..x.s.clone() })
            .collect();
        scene.set_sessions(&list, 1);
    }

    fn set_state(&mut self, i: usize, st: State, why: Why) {
        self.sim[i].s.state = st;
        self.sim[i].s.why = why;
        self.sim[i].since = self.now;
    }

    /// Once a second: working sessions burn tokens and call tools; now and then one changes state.
    fn second(&mut self, scene: &mut Scene) {
        for i in 0..self.sim.len() {
            if !self.sim[i].live || self.sim[i].s.state != State::Work {
                continue;
            }
            // f32 sums, as the C original computes them
            let b = 3000.0 + self.rnd() * 40000.0;
            self.sim[i].s.burn += b as f64;
            let t = 3000.0 + self.rnd() * 40000.0;
            self.sim[i].s.tokens += t as f64;
        }
        self.send_list(scene);
        const KS: [Tool; 11] = [
            Tool::Bash,
            Tool::Bash,
            Tool::Edit,
            Tool::Edit,
            Tool::Read,
            Tool::Read,
            Tool::Read,
            Tool::Web,
            Tool::Agent,
            Tool::Todo,
            Tool::Mcp,
        ];
        for i in 0..self.sim.len() {
            if !self.sim[i].live || self.sim[i].s.state != State::Work || self.rnd() > 0.35 {
                continue;
            }
            let k = (self.rnd() * 11.0) as usize % 11;
            scene.tool(&self.sim[i].s.id, KS[k]);
        }
        if !self.demo || self.rnd() > 0.3 {
            return;
        }
        let n = self.sim.len();
        let i = (self.rnd() * n as f32) as usize % n;
        if !self.sim[i].live {
            return;
        }
        let q = self.rnd();
        if self.sim[i].s.state == State::Work {
            let st = if q < 0.5 {
                State::Idle
            } else if q < 0.75 {
                State::Wait
            } else {
                State::Work
            };
            self.set_state(i, st, if q < 0.62 { Why::Ask } else { Why::Perm });
        } else if q < 0.5 {
            self.set_state(i, State::Work, Why::None);
        }
    }
}

fn write_png(path: &str, fb: &[u16], k: usize) -> std::io::Result<()> {
    let (w, h) = (W * k, H * k);
    let mut enc = png::Encoder::new(BufWriter::new(File::create(path)?), w as u32, h as u32);
    enc.set_color(png::ColorType::Rgb);
    enc.set_depth(png::BitDepth::Eight);
    let mut data = Vec::with_capacity(w * h * 3);
    for y in 0..h {
        for x in 0..w {
            let c = rgb(fb[(y / k) * W + x / k]);
            data.extend_from_slice(&[(c >> 16) as u8, (c >> 8) as u8, c as u8]);
        }
    }
    enc.write_header()?.write_image_data(&data)?;
    Ok(())
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() < 3 {
        eprintln!(
            "usage: preview demo|full|ko|finish1|finish2|limits1|limits2|leave|mages|fighters out-prefix seconds... [-- x y ...]"
        );
        std::process::exit(1);
    }
    let (scenario, prefix) = (args[0].as_str(), args[1].as_str());
    let split = args.iter().position(|a| a == "--").unwrap_or(args.len());
    let shots: Vec<f64> = args[2..split].iter().filter_map(|a| a.parse().ok()).collect();
    let nums: Vec<i32> = args.get(split + 1..).unwrap_or(&[]).iter().filter_map(|a| a.parse().ok()).collect();
    let taps: Vec<(i32, i32)> = nums.chunks_exact(2).map(|c| (c[0], c[1])).collect();
    let env = |k: &str, d: i64| std::env::var(k).ok().and_then(|v| v.parse().ok()).unwrap_or(d);
    let (scale, hour) = (env("SCALE", 2) as usize, env("HOUR", 14));

    let mut scene = Scene::new(7);
    scene.set_time(1791417600 + hour * 3600, 0);
    scene.set_usage(Usage { tokens: 1624e6, usd: 693.70 });
    scene.set_mana(Mana { claude_5h: 72, claude_week: 41, codex_week: 18 });

    let mut lap = Laptop { sim: Vec::new(), rng: 12345, now: 0.0, demo: false };
    let (e, g, d, b, m) = (
        "~/projects/moon-garden",
        "~/work/star-charts",
        "~/projects/lantern",
        "~/projects/night-market",
        "~/work/paper-boats",
    );
    let mut special = false;
    use State::*;
    match scenario {
        "demo" => {
            lap.demo = true;
            lap.add("o0", e, false, Work, Why::None, 30.0);
            lap.add("o1", e, false, Work, Why::None, 30.0);
            lap.add("o2", g, true, Work, Why::None, 30.0);
            lap.add("o3", g, false, Idle, Why::None, 340.0);
            lap.add("o4", d, false, Wait, Why::Ask, 40.0);
            lap.add("o5", e, true, Work, Why::None, 30.0);
        }
        "full" => {
            let projs = [e, e, g, g, d, e, b, m, m, b];
            for (k, p) in projs.iter().enumerate() {
                let idle = k == 4;
                lap.add(
                    &format!("f{k}"),
                    p,
                    k % 3 == 2,
                    if idle { Idle } else { Work },
                    Why::None,
                    if idle { 470.0 } else { 20.0 },
                );
            }
        }
        "ko" => {
            lap.add("k0", e, false, Idle, Why::None, 590.0);
            lap.add("k1", e, true, Work, Why::None, 20.0);
        }
        "finish1" | "finish2" => {
            // project paths whose hash picks each monster in turn
            let quest = [
                "~/p/quest3",
                "~/p/quest0",
                "~/p/quest5",
                "~/p/quest2",
                "~/p/quest7",
                "~/p/quest4",
                "~/p/quest1",
                "~/p/quest6",
            ];
            let base = if scenario == "finish2" { 4 } else { 0 };
            for k in 0..4 {
                lap.add(&format!("q{k}"), quest[base + k], k % 2 == 1, Idle, Why::None, 598.0 - 4.0 * k as f64);
            }
        }
        "limits1" | "limits2" => {
            let quest = ["~/p/quest3", "~/p/quest0", "~/p/quest5", "~/p/quest2"];
            for (k, q) in quest.iter().enumerate() {
                lap.add(&format!("l{k}"), q, scenario == "limits2", Work, Why::None, 20.0);
            }
        }
        "leave" => {
            lap.add("l0", e, false, Work, Why::None, 20.0);
            lap.add("l1", d, true, Work, Why::None, 20.0);
        }
        "mages" | "fighters" => {
            special = true;
            for k in 0..4 {
                // the least-used class each: all four
                lap.add(&format!("s{k}"), if k < 2 { e } else { g }, scenario == "fighters", Work, Why::None, 20.0);
            }
        }
        _ => {
            eprintln!("unknown scenario {scenario}");
            std::process::exit(1);
        }
    }
    lap.send_list(&mut scene);

    let mut fb = vec![0u16; W * H];
    let dt = 0.033;
    let (mut next_shot, mut tapped, mut next_second, mut did_event, mut next_special) = (0, 0, 1.0, false, 0);
    let print_log = |scene: &mut Scene, now: f64| {
        for line in scene.take_log() {
            println!("{now:6.2}  {line}");
        }
    };
    print_log(&mut scene, 0.0);
    while next_shot < shots.len() {
        lap.now += dt;
        scene.tick(33);
        if lap.now >= next_second {
            next_second += 1.0;
            if !special {
                lap.second(&mut scene);
            } else {
                lap.send_list(&mut scene);
            }
        }
        if special && next_special < 4 && lap.now >= 0.5 + next_special as f64 * 3.0 {
            let s = &lap.sim[next_special].s;
            scene.tool(&s.id, if s.codex { Tool::Bash } else { Tool::Agent });
            next_special += 1;
        }
        if scenario.starts_with("limits") {
            for k in 0..4 {
                if lap.sim[k].live && lap.now >= 1.0 + 4.0 * k as f64 {
                    lap.sim[k].live = false;
                    lap.send_list(&mut scene);
                }
            }
        }
        if !did_event && lap.now >= 0.5 {
            did_event = true;
            if scenario == "leave" {
                lap.sim[1].live = false;
                lap.send_list(&mut scene);
            }
        }
        if scenario == "ko" && lap.now >= 12.0 && lap.sim[0].s.state == Idle {
            lap.set_state(0, Work, Why::None);
            lap.send_list(&mut scene);
        }
        if tapped < taps.len() && lap.now >= 1.0 {
            for &(x, y) in &taps[tapped..] {
                scene.tap(x, y);
            }
            tapped = taps.len();
        }
        print_log(&mut scene, lap.now);
        if lap.now + 1e-9 >= shots[next_shot] {
            scene.render(&mut fb);
            print_log(&mut scene, lap.now);
            let path = format!("{prefix}-{}.png", shots[next_shot]);
            if let Err(err) = write_png(&path, &fb, scale) {
                eprintln!("{path}: {err}");
                std::process::exit(1);
            }
            println!("saved {path}");
            next_shot += 1;
        }
    }
}
