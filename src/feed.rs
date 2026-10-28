//! What the battle is told each poll: each session as a `Session` (its
//! card), and one move per session that called tools; and the made-up
//! battle of `--demo`.

use std::collections::{HashMap, HashSet};

use crate::collect::{Found, Said, pick_tool};
use crate::project::Projects;
use crate::session::{Mana, SaidKind, Session, State, Tool, Usage, Why};
use crate::usage::{Spend, cents};

/// Each session's tokens burned while this app runs, the damage it deals:
/// its spend less what it had at the first count, for a session that started
/// before the app; all of it for one that started after.
#[derive(Default)]
struct Burn {
    base: HashMap<String, f64>,
}

impl Burn {
    fn burn(&mut self, id: &str, started: f64, spend: Option<(f64, f64)>, start: f64) -> Option<f64> {
        let (tokens, _) = spend?;
        let base = self.base.entry(id.to_owned()).or_insert(if started < start - 5.0 { tokens } else { 0.0 });
        let b = tokens - *base;
        if b < 0.0 {
            // a new month counts the transcripts from the 1st
            *base = tokens;
            return Some(0.0);
        }
        Some(b)
    }

    fn forget(&mut self, found: &[Found]) {
        let live: HashSet<&str> = found.iter().map(|s| s.id.as_str()).collect();
        self.base.retain(|id, _| live.contains(id.as_str()));
    }
}

/// What a card is made from.
struct Card<'a> {
    id: &'a str,
    codex: bool,
    term: bool,
    state: State,
    why: Why,
    since: f64,
    started: f64,
    cwd: &'a str,
    spend: Option<(f64, f64)>,
    burn: Option<f64>,
    said: Option<&'a Said>,
}

impl Card<'_> {
    fn session(&self, projects: &mut Projects, now: f64) -> Session {
        let ago = |t: f64| (now - t).max(0.0);
        let said = self.said.filter(|s| !s.text.is_empty());
        Session {
            id: self.id.to_owned(),
            proj: if self.cwd.is_empty() { String::new() } else { projects.project_of(self.cwd, now) },
            codex: self.codex,
            term: self.term,
            state: self.state,
            why: self.why,
            age_s: ago(self.since) as u32,
            up_s: if self.started != 0.0 { ago(self.started) as u32 } else { 0 },
            burn: self.burn.map_or(-1.0, f64::trunc),
            tokens: self.spend.map_or(-1.0, |(t, _)| t),
            usd: self.spend.map_or(0.0, |(_, u)| cents(u)),
            said: said.map_or_else(String::new, |s| s.text.clone()),
            said_kind: said.map_or(SaidKind::Msg, |s| s.kind),
            said_age_s: said.and_then(|s| s.at).filter(|&t| t != 0.0).map_or(-1, |t| ago(t) as i32),
        }
    }
}

/// Turns the collector's sessions into the battle's.
pub struct Feed {
    start: f64,
    projects: Projects,
    burn: Burn,
}

impl Feed {
    /// `start`: when the app started, epoch seconds; damage counts from then.
    pub fn new(start: f64) -> Self {
        Feed { start, projects: Projects::default(), burn: Burn::default() }
    }

    /// agents_cmd() as data: each session's project, how long in its state and
    /// how long up, its spend and burn (`spend` from the latest usage count),
    /// its last message and whether its terminal can be brought up.
    pub fn sessions(&mut self, found: &[Found], now: f64, spend: &Spend) -> Vec<Session> {
        self.burn.forget(found);
        let mut out = Vec::with_capacity(found.len());
        for s in found {
            let spent = spend.of(&s.spend_key);
            let card = Card {
                id: &s.id,
                codex: s.codex,
                term: s.term.is_some(),
                state: s.state,
                why: s.why,
                since: s.since,
                started: s.started,
                cwd: &s.cwd,
                spend: spent,
                burn: self.burn.burn(&s.id, s.started, spent, self.start),
                said: s.said.as_ref(),
            };
            out.push(card.session(&mut self.projects, now));
        }
        out
    }
}

/// tool_cmds(): one move per session that called tools since the last poll,
/// as (session id, kind).
pub fn tool_calls(found: &[Found]) -> Vec<(String, Tool)> {
    found.iter().filter_map(|s| Some((s.id.clone(), pick_tool(&s.tools)?))).collect()
}

/* ---- the made-up battle -------------------------------------------------- */

pub const DEMO_USAGE: Usage = Usage { tokens: 1_624_000_000.0, usd: 693.70 };
pub const DEMO_MANA: Mana = Mana { claude_5h: 72, claude_week: 41, codex_week: 18 };
const DEMO_TOOLS: [Tool; 11] = [
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
/// Made-up projects, as the collectors name them; the demo never looks at real repositories.
const DEMO_PROJECTS: [&str; 4] = [
    "github.com/someone/moon-garden",
    "github.com/someone/star-charts",
    "~/lantern",
    "gitlab.com/someone/night-market",
];
const DEMO_ASKS: &str = "Which registry should CI use? US Pesde registry (Recommended) / Public registry";
const DEMO_SAYS: &str = "The battle now runs on the desktop. Each working session swings at its project for the \
                         tokens it burned, and every tool call is a spell.";

/// xorshift64*: enough dice for a demo.
struct Rng(u64);

impl Rng {
    fn new(seed: f64) -> Self {
        Rng((seed.to_bits() ^ u64::from(std::process::id()).rotate_left(32)) | 1)
    }

    /// In [0, 1).
    fn random(&mut self) -> f64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        (self.0.wrapping_mul(0x2545_f491_4f6c_dd1d) >> 11) as f64 / (1u64 << 53) as f64
    }

    /// In 0..n.
    fn below(&mut self, n: usize) -> usize {
        ((self.random() * n as f64) as usize).min(n - 1)
    }
}

struct Player {
    id: String,
    codex: bool,
    state: State,
    why: Why,
    since: f64,
    started: f64,
    proj: &'static str,
    burn: f64,
    spend: (f64, f64),
    tools: Vec<Tool>,
    said: Said,
}

/// A made-up battle: six sessions over three projects that burn tokens, call
/// tools, idle, ask and come and go, like the design page's director.
pub struct Demo {
    n: u32,
    players: Vec<Player>,
    rng: Rng,
    projects: Projects,
}

impl Demo {
    pub fn new(now: f64) -> Self {
        let mut d = Demo { n: 0, players: Vec::new(), rng: Rng::new(now), projects: Projects::default() };
        let (work, idle, wait) = (State::Work, State::Idle, State::Wait);
        for (proj, codex, state, why) in [
            (0, false, work, Why::None),
            (0, false, work, Why::None),
            (1, true, work, Why::None),
            (1, false, idle, Why::None),
            (2, false, wait, Why::Ask),
            (0, true, work, Why::None),
        ] {
            d.add(now, proj, codex, state, why);
        }
        d.players[3].since = now - 420.0; // idle a while: hurt already
        d
    }

    fn add(&mut self, now: f64, proj: usize, codex: bool, state: State, why: Why) {
        self.n += 1;
        let wait = state == State::Wait;
        let spend = (5e6 * self.rng.random(), 3.0 * self.rng.random());
        self.players.push(Player {
            id: format!("de{:06x}", self.n),
            codex,
            state,
            why,
            since: now,
            started: now - 600.0,
            proj: DEMO_PROJECTS[proj],
            burn: 0.0,
            spend,
            tools: Vec::new(),
            said: Said {
                kind: if wait { SaidKind::Ask } else { SaidKind::Msg },
                text: (if wait { DEMO_ASKS } else { DEMO_SAYS }).to_owned(),
                at: Some(now - 40.0),
            },
        });
    }

    /// A second of the battle (tokens, tools, and now and then a change):
    /// the sessions and their moves.
    pub fn second(&mut self, now: f64) -> (Vec<Session>, Vec<(String, Tool)>) {
        self.step(now);
        let mut sessions = Vec::with_capacity(self.players.len());
        for p in &self.players {
            let card = Card {
                id: &p.id,
                codex: p.codex,
                term: false,
                state: p.state,
                why: p.why,
                since: p.since,
                started: p.started,
                cwd: "",
                spend: Some(p.spend),
                burn: Some(p.burn),
                said: Some(&p.said),
            };
            sessions.push(Session { proj: p.proj.to_owned(), ..card.session(&mut self.projects, now) });
        }
        let tools = self.players.iter().filter_map(|p| Some((p.id.clone(), pick_tool(&p.tools)?))).collect();
        (sessions, tools)
    }

    fn step(&mut self, now: f64) {
        for p in &mut self.players {
            p.tools.clear();
            if p.state != State::Work {
                continue;
            }
            let n = 2000.0 + self.rng.random() * 30000.0;
            p.burn += n;
            p.spend = (p.spend.0 + n, p.spend.1 + n / 2e6);
            if self.rng.random() < 0.3 {
                p.tools = vec![DEMO_TOOLS[self.rng.below(DEMO_TOOLS.len())]];
            }
        }
        if self.rng.random() > 0.35 {
            return;
        }
        let len = self.players.len();
        if len < 4 || (self.rng.random() < 0.1 && len < 10) {
            let proj = self.rng.below(DEMO_PROJECTS.len());
            let codex = self.rng.below(3) == 2; // claude, claude, codex
            self.add(now, proj, codex, State::Work, Why::None);
            return;
        }
        let i = self.rng.below(len);
        let q = self.rng.random();
        let p = &mut self.players[i];
        if q < 0.08 && len > 5 {
            self.players.remove(i);
        } else if p.state == State::Work {
            (p.state, p.why) = if q < 0.6 {
                (State::Idle, Why::None)
            } else {
                (State::Wait, if q < 0.8 { Why::Ask } else { Why::Perm })
            };
            p.since = now;
        } else if self.rng.random() < 0.6 {
            (p.state, p.why, p.since) = (State::Work, Why::None, now);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collect::SpendKey;

    fn found(id: &str, started: f64, tools: Vec<Tool>) -> Found {
        Found {
            id: id.into(),
            name: format!("garden-{id}"),
            codex: false,
            state: State::Wait,
            why: Why::Perm,
            since: 990.5,
            started,
            cwd: String::new(),
            pid: 1,
            term: None,
            spend_key: SpendKey::Codex(vec![]),
            said: Some(Said { kind: SaidKind::Tool, text: "Bash: make tea".into(), at: Some(950.0) }),
            tools,
        }
    }

    #[test]
    fn burn_counts_from_the_start_or_the_first_count() {
        let mut b = Burn::default();
        let start = 1000.0;
        assert_eq!(b.burn("old", 10.0, Some((5000.0, 1.0)), start), Some(0.0));
        assert_eq!(b.burn("old", 10.0, Some((7000.0, 1.0)), start), Some(2000.0));
        assert_eq!(b.burn("new", 999.0, Some((300.0, 1.0)), start), Some(300.0));
        assert_eq!(b.burn("new", 999.0, None, start), None);
        // a new month starts the count again
        assert_eq!(b.burn("old", 10.0, Some((100.0, 1.0)), start), Some(0.0));
        assert_eq!(b.burn("old", 10.0, Some((150.0, 1.0)), start), Some(50.0));
        b.forget(&[found("new", 0.0, vec![])]);
        assert_eq!(b.base.keys().collect::<Vec<_>>(), ["new"]);
    }

    #[test]
    fn cards_and_moves() {
        let mut feed = Feed::new(1000.0);
        let list = [found("aaaa0001", 0.0, vec![Tool::Read, Tool::Bash]), found("aaaa0002", 400.0, vec![])];
        let s = feed.sessions(&list, 1001.0, &Spend::default());
        assert_eq!(s.len(), 2);
        assert_eq!((s[0].age_s, s[0].up_s, s[1].up_s), (10, 0, 601));
        assert_eq!((s[0].tokens, s[0].burn, s[0].usd), (-1.0, -1.0, 0.0));
        assert_eq!((s[0].said.as_str(), s[0].said_kind, s[0].said_age_s), ("Bash: make tea", SaidKind::Tool, 51));
        assert_eq!((s[0].proj.as_str(), s[0].term, s[0].why), ("", false, Why::Perm));
        assert_eq!(tool_calls(&list), [("aaaa0001".to_owned(), Tool::Bash)]);
    }

    #[test]
    fn the_demo_plays() {
        // the dice are seeded from the clock: the first second may already
        // send one of the six off, and the one asking may be it
        let mut demo = Demo::new(1000.0);
        let (s, _) = demo.second(1001.0);
        assert!(s.len() >= 5);
        assert!(s.iter().all(|s| s.id.starts_with("de") && s.tokens >= 0.0 && s.up_s >= 600 && !s.proj.is_empty()));
        let (mut moves, mut asked) = (0, s.iter().any(|s| s.said_kind == SaidKind::Ask));
        for t in 2..300 {
            let (s, tools) = demo.second(1000.0 + t as f64);
            assert!((4..=10).contains(&s.len()));
            moves += tools.len();
            asked |= s.iter().any(|s| s.said_kind == SaidKind::Ask);
        }
        assert!(moves > 0 && asked);
    }
}
