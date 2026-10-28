//! Renders the README's images in docs/ from the scene, with made-up
//! sessions and projects:
//!   cargo run --release --example screenshots
//! docs/battle.png   mid-fight on a sunny afternoon, a summon on its way
//! docs/card.png     a player's status card
//! docs/limit.png    a project's last session gone: its monster meets a final blow, at night
//! docs/finish.png   a session idle for 10 minutes meets its monster's finisher, at dusk

use std::fs::File;
use std::io::BufWriter;

use token_fantasy::gfx::{H, W, rgb};
use token_fantasy::scene::Scene;
use token_fantasy::session::{Mana, SaidKind, Session, State, Tool, Usage, Why};

const SCALE: usize = 2;
const SAYS: &str = "The checkout flow now retries a declined card once before it shows the error. The new test \
                    covers both paths, and the whole suite passes.";
const ASKS: &str = "Which registry should CI publish to? The private registry (Recommended) / The public one";

/// A battle and the sessions behind it, run without a window.
struct Run {
    scene: Scene,
    list: Vec<(Session, f64)>,
    t: f64,
    rng: u32,
}

fn session(id: &str, proj: &str, codex: bool, state: State, why: Why) -> Session {
    Session {
        id: id.into(),
        proj: proj.into(),
        codex,
        term: true,
        state,
        why,
        up_s: 5400,
        burn: 0.0,
        tokens: 18e6,
        usd: 7.4,
        said: if state == State::Wait { ASKS } else { SAYS }.into(),
        said_kind: if state == State::Wait { SaidKind::Ask } else { SaidKind::Msg },
        said_age_s: 95,
        ..Session::default()
    }
}

impl Run {
    /// The sessions as (session, seconds in its state so far).
    fn new(hour: i64, list: Vec<(Session, f64)>) -> Run {
        let mut scene = Scene::new(11);
        scene.set_time(1791417600 + hour * 3600, 0);
        scene.set_usage(Usage { tokens: 1_624_000_000.0, usd: 693.70 });
        scene.set_mana(Mana { claude_5h: 72, claude_week: 41, codex_week: 18 });
        let list = list.into_iter().map(|(s, age)| (s, -age)).collect();
        let mut run = Run { scene, list, t: 0.0, rng: 2026 };
        run.send();
        run
    }

    fn rnd(&mut self) -> f64 {
        self.rng = self.rng.wrapping_mul(1664525).wrapping_add(1013904223);
        (self.rng >> 8) as f64 / 16777216.0
    }

    fn send(&mut self) {
        let t = self.t;
        let list: Vec<Session> =
            self.list.iter().map(|(s, since)| Session { age_s: (t - since) as u32, ..s.clone() }).collect();
        self.scene.set_sessions(&list, 1);
    }

    /// Run on to time t: every second the working sessions burn tokens.
    fn until(&mut self, t: f64) {
        while self.t + 1e-9 < t {
            let before = self.t;
            self.t += 0.033;
            self.scene.tick(33);
            if before.floor() != self.t.floor() {
                for i in 0..self.list.len() {
                    if self.list[i].0.state == State::Work {
                        let n = 4000.0 + self.rnd() * 60000.0;
                        self.list[i].0.burn += n;
                        self.list[i].0.tokens += n;
                    }
                }
                self.send();
            }
        }
    }

    fn tool(&mut self, id: &str, tool: Tool) {
        self.scene.tool(id, tool);
    }

    fn leave(&mut self, id: &str) {
        self.list.retain(|(s, _)| s.id != id);
        self.send();
    }

    fn shot(&mut self, path: &str) {
        let mut fb = vec![0u16; W * H];
        self.scene.render(&mut fb);
        let (w, h) = (W * SCALE, H * SCALE);
        let mut enc = png::Encoder::new(BufWriter::new(File::create(path).unwrap()), w as u32, h as u32);
        enc.set_color(png::ColorType::Rgb);
        enc.set_depth(png::BitDepth::Eight);
        let mut data = Vec::with_capacity(w * h * 3);
        for y in 0..h {
            for x in 0..w {
                let c = rgb(fb[(y / SCALE) * W + x / SCALE]);
                data.extend_from_slice(&[(c >> 16) as u8, (c >> 8) as u8, c as u8]);
            }
        }
        enc.write_header().unwrap().write_image_data(&data).unwrap();
        println!("saved {path}");
    }
}

fn main() {
    std::fs::create_dir_all("docs").unwrap();
    let (moon, stars, lantern, market) =
        ("~/projects/moon-garden", "~/projects/star-charts", "~/projects/lantern", "~/projects/night-market");
    use State::*;

    // a sunny afternoon: three projects, six sessions; one subagent summons
    let mut run = Run::new(
        14,
        vec![
            (session("a0", moon, false, Work, Why::None), 30.0),
            (session("a1", moon, false, Work, Why::None), 30.0),
            (session("a2", stars, true, Work, Why::None), 30.0),
            (session("a3", stars, false, Idle, Why::None), 380.0),
            (session("a4", lantern, false, Wait, Why::Ask), 40.0),
            (session("a5", moon, true, Work, Why::None), 30.0),
        ],
    );
    run.until(9.0);
    for id in ["a0", "a1"] {
        run.tool(id, Tool::Agent);
    }
    run.until(10.3);
    run.shot("docs/battle.png");
    run.until(14.0);
    run.scene.tap(352, 84); // a player in the front column
    run.until(14.5);
    run.shot("docs/card.png");

    // night: the last session of a project ends, and a final blow finishes its monster
    let mut run = Run::new(
        22,
        vec![
            (session("b0", moon, false, Work, Why::None), 30.0),
            (session("b1", stars, true, Work, Why::None), 30.0),
            (session("b2", lantern, false, Work, Why::None), 30.0),
            (session("b3", market, false, Work, Why::None), 30.0),
        ],
    );
    run.until(12.0);
    run.leave("b2");
    run.until(13.7);
    run.shot("docs/limit.png");

    // dusk: a session idle for ten minutes meets its monster's finisher
    let mut run = Run::new(
        18,
        vec![
            (session("c0", moon, false, Idle, Why::None), 596.0),
            (session("c1", moon, true, Work, Why::None), 30.0),
            (session("c2", stars, false, Work, Why::None), 30.0),
        ],
    );
    run.until(5.7);
    run.shot("docs/finish.png");
}
