//! The battle: this machine's agent sessions as the party, their projects as
//! the monsters, the windows along the bottom, the status card and the
//! config window. The collectors only say which sessions exist, what state
//! each is in, how many tokens each has burned and which tools they call;
//! everything that moves is decided here.
//!
//! The rules, moves, effects and windows follow design/engine.js. The design runs
//! every action as a generator; here an action is a timeline: each frame it
//! gets the time it had reached and the time it reaches now, and fires
//! whatever falls in between. This file has the rules and the cast; fx.rs
//! draws the effects, ui.rs the field and the windows.

mod fx;
mod ui;

use crate::art::{CLASSES, MONSTERS, MonFx, Pose, Strike, c};
use crate::gfx::{W as FF_W, hex};
use crate::layout::*;
use crate::session::{Mana, SaidKind, Session, State, Tool, Usage, Why};

/* the rules (design/engine.js RULES) */
const HP_MAX: f32 = 999.0;
/// A working player attacks this often, for the tokens burned since.
const ATB_S: f32 = 8.0;
/// At most one move per player this often; tool calls in between fold.
const CAST_GAP_S: f32 = 2.5;
/// An idle player's monster hits it about this often.
const HIT_S: f32 = 30.0;
/// Idle this long and the player is knocked out.
const KO_S: f32 = 600.0;
/// Kneels under this much HP.
const CRITICAL: f32 = 0.25;
/// A monster's finisher, the hit that knocks a player out.
const FIN_DUR: f32 = 2.0;
const FIN_IMPACT: f32 = 1.1;

/// The field's 8 and those waiting off it.
const MAX_PLAYERS: usize = 24;
/// 4 on the field and any still dissolving.
const MAX_MONS: usize = 8;
const MAX_PROJ: usize = 32;
const MAX_FX: usize = 48;
const MAX_NUMS: usize = 24;
const QLEN: usize = 4;
const HOST_STALE_MS: i64 = 15000;
const CONFIG_MS: i64 = 30000;
const CHIME_GAP_MS: i64 = 2000;
const TERM_FLASH_MS: i64 = 600;
const ID_LEN: usize = 16;
const PROJ_LEN: usize = 40;
const SAID_LEN: usize = 400;

/// What the scene wants from the app, collected by `take_events()`.
pub mod ev {
    /// Someone started waiting for an OK.
    pub const CHIME_PERM: u32 = 1 << 0;
    /// Someone started waiting with a question.
    pub const CHIME_ASK: u32 = 1 << 1;
    /// A click opened or stepped something.
    pub const BLIP: u32 = 1 << 2;
    /// A click closed something.
    pub const BLIP_LOW: u32 = 1 << 3;
    /// The config window's sound button: toggle mute.
    pub const MUTE: u32 = 1 << 4;
    /// The config window's screen button: toggle fullscreen.
    pub const FULLSCREEN: u32 = 1 << 5;
    /// A portrait was clicked: bring that session's terminal to the front.
    pub const TERMINAL: u32 = 1 << 6;
    /// The config window's pace button: toggle Wait / Active.
    pub const PACE: u32 = 1 << 7;
    /// A monster was defeated.
    pub const FANFARE: u32 = 1 << 8;
    /// A Phoenix Plume.
    pub const REVIVE: u32 = 1 << 9;
}

/* ---- moves -------------------------------------------------------------------- */

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Default)]
enum Mv {
    #[default]
    Fire,
    Ice,
    Bolt,
    Scan,
    Gale,
    Protect,
    Summon,
    Flare,
    Holy,
    Doublecast,
    Cross,
    Steal,
    ChiBurst,
    Jump,
}

const MV_NAME: [&str; 14] = [
    "FIRE",
    "BLIZZARD",
    "THUNDER",
    "SCAN",
    "GALE",
    "PROTECT",
    "SUMMON",
    "FLARE",
    "HOLY",
    "DOUBLECAST",
    "CROSS CUT",
    "STEAL",
    "CHI BURST",
    "JUMP",
];

fn mv_name(mv: Mv) -> &'static str {
    MV_NAME[mv as usize]
}

/// The spell a mage casts for each tool.
fn tool_mv(t: Tool) -> Mv {
    match t {
        Tool::Bash => Mv::Fire,
        Tool::Edit => Mv::Ice,
        Tool::Read => Mv::Scan,
        Tool::Web => Mv::Gale,
        Tool::Agent => Mv::Summon,
        Tool::Todo => Mv::Protect,
        Tool::Mcp => Mv::Bolt,
    }
}

fn tool_label(t: Tool) -> &'static str {
    ["Bash", "Edit", "Read", "WebFetch", "Agent", "TodoWrite", "MCP"][t as usize]
}

/// Each class's special move: mages use it for a subagent, fighters for every tool call.
const SPECIALS: [(&str, Mv); 8] = [
    ("sorcerer", Mv::Flare),
    ("cleric", Mv::Holy),
    ("spellsword", Mv::Doublecast),
    ("summoner", Mv::Summon),
    ("warrior", Mv::Cross),
    ("thief", Mv::Steal),
    ("monk", Mv::ChiBurst),
    ("lancer", Mv::Jump),
];
/// A Spellsword's doublecast draws two different spells from these.
const DUAL: [Mv; 4] = [Mv::Fire, Mv::Ice, Mv::Bolt, Mv::Gale];
const SUMMON: [&str; 4] = ["DRAGON KING", "SALAMANDER", "FROST QUEEN", "RAIJIN"];
const SUMMON_COL: [[u16; 2]; 4] = [
    [hex(0xfff4c0), hex(0xf89040)],
    [hex(0xfff0a0), hex(0xe84020)],
    [hex(0xffffff), hex(0x58c8f8)],
    [hex(0xffffa0), hex(0xa080ff)],
];
const LOOT: [&str; 9] =
    ["POTION", "GREAT POTION", "ETHER", "ELIXIR", "PHOENIX PLUME", "TENT", "MAX POTION", "REMEDY", "GRAND ELIXIR"];

/// Each monster's finisher, by its attack (MonFx): goblin, bomb, cactoid,
/// grudgling, ahriman, stenchvine, stormcat, behemoth.
const FINISHER: [&str; 8] =
    ["GOBLIN RUSH", "EXPLODE", "NEEDLE STORM", "ANCIENT GRUDGE", "DEATH", "PUTRID BREATH", "MEGAVOLT", "CATACLYSM"];
/// The damage it shows, overkill (the player has little HP
/// left by then): at least lo, up to lo + spread.
const FINISHER_DMG: [[i32; 2]; 8] =
    [[1800, 800], [3000, 999], [10000, 0], [7777, 0], [9999, 0], [4000, 999], [5000, 1999], [9999, 0]];

/// Each class's final blow: the big attack that finishes a monster whose
/// project has no session left. dur and impact in seconds from when it starts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Lb {
    ThousandCuts,
    Meteor,
    SacredHymn,
    SpellCascade,
    TwelveKnights,
    StarPiercer,
    HeavensFist,
    LanceRain,
}

struct Limit {
    cls: &'static str,
    name: &'static str,
    lb: Lb,
    melee: bool,
    dur: f32,
    impact: f32,
}

const LIMITS: [Limit; 8] = [
    Limit { cls: "warrior", name: "THOUSAND CUTS", lb: Lb::ThousandCuts, melee: true, dur: 1.6, impact: 1.3 },
    Limit { cls: "sorcerer", name: "METEOR", lb: Lb::Meteor, melee: false, dur: 1.8, impact: 1.4 },
    Limit { cls: "cleric", name: "SACRED HYMN", lb: Lb::SacredHymn, melee: false, dur: 1.8, impact: 1.3 },
    Limit { cls: "spellsword", name: "SPELL CASCADE", lb: Lb::SpellCascade, melee: false, dur: 2.0, impact: 1.6 },
    Limit { cls: "summoner", name: "TWELVE KNIGHTS", lb: Lb::TwelveKnights, melee: false, dur: 2.4, impact: 2.0 },
    Limit { cls: "thief", name: "STAR PIERCER", lb: Lb::StarPiercer, melee: false, dur: 1.4, impact: 1.0 },
    Limit { cls: "monk", name: "HEAVEN'S FIST", lb: Lb::HeavensFist, melee: true, dur: 1.4, impact: 0.9 },
    Limit { cls: "lancer", name: "LANCE RAIN", lb: Lb::LanceRain, melee: false, dur: 1.6, impact: 1.3 },
];
/// After the aura: the final blow itself starts.
const LIMIT_GO: f32 = 0.75;
/// It shows 3-4x the monster's biggest hit (cosmetic, like a spell).
const LIMIT_X: f32 = 3.0;
/// A doomed monster nobody finished goes anyway.
const DOOM_MS: i64 = 10000;

/// The monster formations, by how many are on the field.
const MON_SLOTS: [&[[i32; 2]]; 4] = [
    &[[112, 144]],
    &[[78, 122], [168, 160]],
    &[[68, 106], [170, 124], [96, 168]],
    &[[62, 106], [164, 106], [82, 168], [184, 168]],
];

/* ---- the cast --------------------------------------------------------------------- */

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
enum Ak {
    #[default]
    RunIn,
    Attack,
    Move,
    Dash,
    Jump,
    Revive,
    Cure,
    Leave,
    Limit,
}

#[derive(Clone, Copy, Debug, Default)]
struct Act {
    kind: Ak,
    tool: Option<Tool>,
    /// A final blow's monster.
    mon: Option<usize>,
    dmg: f64,
}

impl Act {
    fn of(kind: Ak) -> Act {
        Act { kind, ..Act::default() }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
enum Walk {
    #[default]
    None,
    Left,
    Right,
}

#[derive(Clone, Debug, Default)]
struct Player {
    used: bool,
    seen: bool,
    leaving: bool,
    bench: bool,
    /// Unique for the life of the scene.
    order: u32,
    id: String,
    proj: String,
    /// Its project in Scene::proj.
    pj: usize,
    codex: bool,
    term: bool,
    state: State,
    why: Why,
    cls: usize,
    variant: usize,
    name: String,
    /// 0..7 on the field, None while benched.
    slot: Option<usize>,
    x: f32,
    y: f32,
    hx: f32,
    hy: f32,
    hp: f32,
    atb: f32,
    cast_wait: f32,
    hurt: f32,
    white: f32,
    fwd: f32,
    /// Knocked back this far by a finisher, easing off.
    kb: f32,
    ko: bool,
    air: bool,
    hidden: bool,
    diving: bool,
    /// Forced by an action; None = from the state.
    pose: Option<Pose>,
    walking: Walk,
    /// Damage done; tokens burned and not yet swung.
    dealt: f64,
    pending: f64,
    /// The collectors' count of tokens burned, as last seen.
    burn: f64,
    have_burn: bool,
    next_tool: Option<Tool>,
    /// Idle time, and when its monster hits it next.
    idle_s: f32,
    hit_in: f32,
    /* the action running, and those queued after it */
    q: Vec<Act>,
    cur: Act,
    running: bool,
    at: f32,
    at_prev: f32,
    /// Where the current walk started.
    ax: f32,
    ay: f32,
    /// The monster the action aims at.
    tm: Option<usize>,
    /// The move, and its summon or doublecast picks.
    mv: Mv,
    sub0: i32,
    sub1: i32,
    /* the card */
    since_ms: i64,
    started_ms: i64,
    said_ms: i64,
    have_started: bool,
    have_said_at: bool,
    tokens: f64,
    usd: f64,
    said: String,
    said_kind: SaidKind,
}

impl Player {
    fn busy(&self) -> bool {
        self.running || !self.q.is_empty()
    }

    fn waiting(&self) -> bool {
        self.state == State::Wait && !self.ko
    }

    fn push(&mut self, a: Act) {
        if self.q.len() < QLEN {
            self.q.push(a);
        }
    }

    fn push_front(&mut self, a: Act) {
        if self.q.len() == QLEN {
            self.q.pop();
        }
        self.q.insert(0, a);
    }

    /// A walk from wherever the player is at a to (x, y) at b.
    #[allow(clippy::too_many_arguments)]
    fn walk(&mut self, tl: Tl, a: f32, b: f32, x: f32, y: f32, dir: Walk) {
        if tl.at(a) || (a == 0.0 && tl.t0 < 0.0) {
            self.ax = self.x;
            self.ay = self.y;
            self.walking = dir;
        }
        let k = tl.seg(a, b);
        if k < 0.0 {
            return;
        }
        self.x = self.ax + (x - self.ax) * k;
        self.y = self.ay + (y - self.ay) * k;
        if k >= 1.0 {
            self.walking = Walk::None;
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
enum Mk {
    #[default]
    Appear,
    Go,
    Attack,
}

#[derive(Clone, Copy, Debug, Default)]
struct Mact {
    kind: Mk,
    finisher: bool,
    prey: u32,
}

#[derive(Clone, Debug, Default)]
struct Monster {
    used: bool,
    leaving: bool,
    defeated: bool,
    attack_frame: bool,
    /// Its project has no session left: a final blow will finish it.
    doomed: bool,
    doom_ms: i64,
    order: u32,
    pj: usize,
    kind: usize,
    x: f32,
    y: f32,
    tx: f32,
    ty: f32,
    vis: f32,
    white: f32,
    shake: f32,
    sweep: f32,
    phase: f32,
    q: Vec<Mact>,
    cur: Mact,
    running: bool,
    at: f32,
    at_prev: f32,
    impact: f32,
}

impl Monster {
    fn busy(&self) -> bool {
        self.running || !self.q.is_empty()
    }

    fn push(&mut self, a: Mact) {
        if self.q.len() < QLEN {
            self.q.push(a);
        }
    }

    fn idle_h(&self) -> f32 {
        MONSTERS[self.kind].idle.h as f32
    }

    fn idle_w(&self) -> f32 {
        MONSTERS[self.kind].idle.w as f32
    }

    fn cx(&self) -> f32 {
        self.x
    }

    fn cy(&self) -> f32 {
        self.y - self.idle_h() / 2.0
    }
}

#[derive(Clone, Debug, Default)]
struct Proj {
    used: bool,
    path: String,
    /// Damage its monster has taken while this app runs.
    dmg: f64,
    /// And its biggest hit.
    big: f64,
    seen: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Ek {
    Fire,
    Ice,
    Bolt,
    Scan,
    Gale,
    Flare,
    Holy,
    Summon,
    Chant,
    Protect,
    Sparkle,
    Feather,
    Cross,
    Shadow,
    ChiBurst,
    Strike,
    Tap,
    MPunch,
    MBlaze,
    MNeedles,
    MKnife,
    MDoom,
    MBreath,
    MBlaster,
    MMeteor,
    MFinish,
    Limit,
    LimitAura,
}

/// A monster's ordinary attack effect, by its MonFx.
const MON_EK: [Ek; 8] =
    [Ek::MPunch, Ek::MBlaze, Ek::MNeedles, Ek::MKnife, Ek::MDoom, Ek::MBreath, Ek::MBlaster, Ek::MMeteor];

#[derive(Clone, Copy, Debug)]
struct Fx {
    kind: Ek,
    mon: Option<usize>,
    pl: Option<usize>,
    /// Runs 0..1.
    t: f32,
    dur: f32,
    seed: u32,
    arg: i32,
    x: f32,
    y: f32,
    col: u16,
}

#[derive(Clone, Debug)]
struct Num {
    x: f32,
    y: f32,
    y0: f32,
    t: f32,
    s: String,
    c: u16,
    k: i32,
}

#[derive(Clone, Debug, Default)]
struct Banner {
    text: String,
    sub: String,
    t: f32,
    dur: f32,
    prio: i32,
    on: bool,
}

/// What a timeline covered this frame: from t0 (exclusive) to t1.
#[derive(Clone, Copy, Debug)]
struct Tl {
    t0: f32,
    t1: f32,
}

impl Tl {
    /// Did the action's time pass t this frame?
    fn at(self, t: f32) -> bool {
        self.t0 < t && self.t1 >= t
    }

    /// How far into [a, b] it is (1 once past b), or -1 outside it this frame.
    fn seg(self, a: f32, b: f32) -> f32 {
        if self.t1 < a || self.t0 >= b {
            return -1.0;
        }
        ((self.t1 - a) / (b - a)).clamp(0.0, 1.0)
    }
}

/// The design's rng(seed): the same sequence per seed, so an effect draws alike every frame.
struct Lcg(u32);

impl Lcg {
    fn next(&mut self) -> f32 {
        self.0 = self.0.wrapping_mul(1664525).wrapping_add(1013904223);
        self.0 as f32 / 4294967296.0
    }
}

fn iround(v: f32) -> i32 {
    (v + 0.5).floor() as i32
}

fn k256(t: f32) -> i32 {
    (t.clamp(0.0, 1.0) * 256.0) as i32
}

/// At most n bytes of s, cut at a character boundary.
fn cut(s: &str, n: usize) -> String {
    let mut end = s.len().min(n);
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    s[..end].to_string()
}

/// 9999 / 12.3K / 386K / 1.24M / 52.1M / 1624M
fn fmt_tok(v: f64) -> String {
    let r = (v + 0.5).floor() as i64;
    if r < 10000 {
        format!("{r}")
    } else if r < 100000 {
        format!("{:.1}K", r as f64 / 1e3)
    } else if r < 1000000 {
        format!("{}K", (r as f64 / 1e3 + 0.5).floor() as i64)
    } else if r < 10000000 {
        format!("{:.2}M", r as f64 / 1e6)
    } else if r < 100000000 {
        format!("{:.1}M", r as f64 / 1e6)
    } else {
        format!("{}M", (r as f64 / 1e6 + 0.5).floor() as i64)
    }
}

fn fnv1a(s: &str) -> u32 {
    s.bytes().fold(2166136261u32, |h, b| (h ^ b as u32).wrapping_mul(16777619))
}

/// An idle player's HP follows how long it has been idle: KO at KO_S.
fn idle_hp(idle_s: f32) -> f32 {
    let hp = HP_MAX * (1.0 - idle_s / KO_S);
    if hp < 0.0 { 0.0 } else { hp }
}

fn inside(x: i32, y: i32, bx: i32, by: i32, bw: i32, bh: i32) -> bool {
    x >= bx && y >= by && x < bx + bw && y < by + bh
}

pub struct Scene {
    pl: Vec<Player>,
    mon: Vec<Monster>,
    proj: Vec<Proj>,
    fx: Vec<Fx>,
    nums: Vec<Num>,
    banner: Banner,
    next_order: u32,
    seen_order: u32,
    rng: u32,
    /// The biggest hit on any monster since start.
    big: f64,
    /// Seconds.
    time: f64,
    ms: i64,
    /// Time for flicker and bobbing, wrapped.
    t: f32,
    flash: f32,
    dim: f32,
    listed: bool,
    instant: bool,
    defer: bool,
    pace_active: bool,
    muted: bool,
    fullscreen: bool,
    host: i64,
    list_ms: i64,
    field: Vec<u16>,
    tod_shown: Option<usize>,
    special: [Mv; 8],
    /// Into LIMITS.
    limit: [usize; 8],
    /* the clock */
    have_time: bool,
    epoch: i64,
    epoch_ms: i64,
    tz: i32,
    have_usage: bool,
    usage: Usage,
    mana: Mana,
    /* the card and the config window */
    card: u32,
    term_order: u32,
    term_at: i64,
    chime_at: i64,
    config_at: i64,
    config: bool,
    events: u32,
    log: Vec<String>,
}

impl Scene {
    pub fn new(seed: u32) -> Scene {
        let mut special = [Mv::Fire; 8];
        let mut limit = [0; 8];
        for (c, cls) in CLASSES.iter().enumerate() {
            if let Some(&(_, mv)) = SPECIALS.iter().find(|(id, _)| *id == cls.id) {
                special[c] = mv;
            }
            if let Some(k) = LIMITS.iter().position(|l| l.cls == cls.id) {
                limit[c] = k;
            }
        }
        Scene {
            pl: vec![Player::default(); MAX_PLAYERS],
            mon: vec![Monster::default(); MAX_MONS],
            proj: vec![Proj::default(); MAX_PROJ],
            fx: Vec::with_capacity(MAX_FX),
            nums: Vec::with_capacity(MAX_NUMS),
            banner: Banner::default(),
            next_order: 0,
            seen_order: 0,
            rng: if seed != 0 { seed } else { 0x2545f491 },
            big: 0.0,
            time: 0.0,
            ms: 0,
            t: 0.0,
            flash: 0.0,
            dim: 0.0,
            listed: false,
            instant: false,
            defer: false,
            pace_active: false,
            muted: false,
            fullscreen: false,
            host: 0,
            list_ms: 0,
            field: vec![0; crate::art::FIELD_W * crate::art::FIELD_H],
            tod_shown: None,
            special,
            limit,
            have_time: false,
            epoch: 0,
            epoch_ms: 0,
            tz: 0,
            have_usage: false,
            usage: Usage::default(),
            mana: Mana::default(),
            card: 0,
            term_order: 0,
            term_at: 0,
            chime_at: 0,
            config_at: 0,
            config: false,
            events: 0,
            log: Vec::new(),
        }
    }

    /* ---- small helpers ------------------------------------------------------------------ */

    fn rnd(&mut self) -> f32 {
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng = x;
        (x >> 8) as f32 / 16777216.0
    }

    fn rndi(&mut self, n: i32) -> i32 {
        let v = (self.rnd() * n as f32) as i32;
        if v < n { v } else { n - 1 }
    }

    fn logf(&mut self, line: String) {
        self.log.push(line);
    }

    fn banner(&mut self, text: &str, sub: &str, prio: i32) {
        if self.instant {
            return;
        }
        let b = &mut self.banner;
        if b.on && b.prio > prio && b.t < b.dur {
            return;
        }
        b.text = cut(text, 23);
        b.sub = cut(sub, 31);
        b.t = 0.0;
        b.dur = if prio > 1 { 2.0 } else { 1.3 };
        b.prio = prio;
        b.on = true;
    }

    /// Numbers on the same spot stack upwards instead of overprinting; k = 2 for a finisher's.
    fn pop_num_k(&mut self, x: f32, y: f32, s: &str, c: u16, k: i32) {
        if self.nums.len() == MAX_NUMS {
            return;
        }
        let near = self.nums.iter().filter(|n| (n.x - x).abs() < 24.0 && (n.y0 - y).abs() < 30.0 && n.t < 0.9).count();
        self.nums.push(Num { x, y: y - (near as i32 * 9) as f32, y0: y, t: 0.0, s: cut(s, 7), c, k });
    }

    fn pop_num(&mut self, x: f32, y: f32, s: &str, c: u16) {
        self.pop_num_k(x, y, s, c, 1);
    }

    fn fx_add(&mut self, kind: Ek, dur: f32, mon: Option<usize>, pl: Option<usize>) -> Option<&mut Fx> {
        if self.fx.len() == MAX_FX {
            return None;
        }
        let seed = (self.rnd() * 1e9) as u32;
        self.fx.push(Fx { kind, mon, pl, t: 0.0, dur, seed, arg: 0, x: 0.0, y: 0.0, col: 0 });
        self.fx.last_mut()
    }

    /// An actor is gone: so are the effects that follow it.
    fn fx_forget(&mut self, mon: Option<usize>, pl: Option<usize>) {
        self.fx.retain(|e| !((mon.is_some() && e.mon == mon) || (pl.is_some() && e.pl == pl)));
    }

    /* ---- projects ------------------------------------------------------------------ */

    fn project(&mut self, path: &str) -> usize {
        let mut free = None;
        for i in 0..MAX_PROJ {
            if self.proj[i].used && self.proj[i].path == path {
                return i;
            }
            if !self.proj[i].used && free.is_none() {
                free = Some(i);
            }
        }
        let free = match free {
            Some(f) => f,
            None => {
                // full: the one seen longest ago with nobody in it
                let mut old: Option<usize> = None;
                for i in 0..MAX_PROJ {
                    let busy =
                        self.pl.iter().any(|p| p.used && p.pj == i) || self.mon.iter().any(|m| m.used && m.pj == i);
                    if !busy && old.is_none_or(|o| self.proj[i].seen < self.proj[o].seen) {
                        old = Some(i);
                    }
                }
                match old {
                    Some(o) => o,
                    None => return 0,
                }
            }
        };
        self.seen_order += 1;
        self.proj[free] = Proj { used: true, path: cut(path, PROJ_LEN), seen: self.seen_order, ..Proj::default() };
        free
    }

    /* ---- monsters ------------------------------------------------------------------ */

    fn monster_of(&self, pj: usize) -> Option<usize> {
        (0..MAX_MONS).find(|&i| self.mon[i].used && !self.mon[i].leaving && self.mon[i].pj == pj)
    }

    fn mon_at(&self, i: Option<usize>) -> Option<usize> {
        i.filter(|&i| i < MAX_MONS && self.mon[i].used)
    }

    /// The live monsters in the order they came.
    fn live_monsters(&self) -> Vec<usize> {
        let mut out: Vec<usize> = (0..MAX_MONS).filter(|&i| self.mon[i].used && !self.mon[i].leaving).collect();
        out.sort_by_key(|&i| self.mon[i].order);
        out
    }

    fn relayout_monsters(&mut self) {
        let live = self.live_monsters();
        let slots = MON_SLOTS[live.len().clamp(1, 4) - 1];
        for (i, &mi) in live.iter().enumerate().take(4) {
            let m = &mut self.mon[mi];
            m.tx = slots[i][0] as f32;
            m.ty = slots[i][1] as f32;
            if self.instant || m.x == 0.0 {
                m.x = m.tx;
                m.y = m.ty;
            }
        }
    }

    /// A project always meets the same monster, unless another project has it on the field.
    fn kind_for(&self, proj: &str) -> usize {
        let h = fnv1a(proj);
        let n = MONSTERS.len() as u32;
        for k in 0..n {
            let kk = (h.wrapping_add(k) % n) as usize;
            if !self.mon.iter().any(|m| m.used && !m.leaving && m.kind == kk) {
                return kk;
            }
        }
        (h % n) as usize
    }

    fn add_monster(&mut self, pj: usize) {
        let Some(i) = (0..MAX_MONS).find(|&i| !self.mon[i].used) else { return };
        let kind = self.kind_for(&self.proj[pj].path); // before the slot counts as used
        self.next_order += 1;
        let phase = self.rnd() * 6.0;
        self.mon[i] =
            Monster { used: true, order: self.next_order, pj, kind, sweep: -1.0, phase, ..Monster::default() };
        self.relayout_monsters();
        if self.instant {
            self.mon[i].vis = 1.0;
        } else {
            self.mon[i].push(Mact { kind: Mk::Appear, ..Mact::default() });
        }
        let line = format!("{} appears: project {}", MONSTERS[kind].name, self.proj[pj].path);
        self.logf(line);
    }

    /// Defeated: its project's last session has ended; else it flees, as a
    /// busier project takes its place.
    fn remove_monster(&mut self, mi: usize, defeated: bool) {
        let m = &mut self.mon[mi];
        m.leaving = true;
        m.doomed = false;
        m.defeated = defeated;
        m.q.clear();
        m.running = false;
        m.attack_frame = false;
        m.push(Mact { kind: Mk::Go, ..Mact::default() });
        let (kind, pj) = (m.kind, m.pj);
        let line = format!(
            "{} {} ({}): {} damage in all",
            MONSTERS[kind].name,
            if defeated { "defeated" } else { "flees" },
            self.proj[pj].path,
            fmt_tok(self.proj[pj].dmg)
        );
        self.logf(line);
    }

    fn has_monster(&self, pj: usize) -> bool {
        self.monster_of(pj).is_some()
    }

    /// The 4 projects with the most sessions on the field get a monster; on a
    /// tie the one that has it keeps it, then the one seen first.
    fn rebalance(&mut self) {
        if self.defer {
            return;
        }
        let mut count = [0i32; MAX_PROJ];
        for p in &self.pl {
            if p.used && !p.leaving && !p.bench {
                count[p.pj] += 1;
            }
        }
        let mut ranked: Vec<usize> = (0..MAX_PROJ).filter(|&i| count[i] != 0).collect();
        for i in 1..ranked.len() {
            let mut j = i;
            while j > 0 {
                let (a, b) = (ranked[j - 1], ranked[j]);
                let mut d = count[b] - count[a];
                if d == 0 {
                    d = self.has_monster(b) as i32 - self.has_monster(a) as i32;
                }
                if d == 0 {
                    d = (self.proj[b].seen < self.proj[a].seen) as i32;
                }
                if d <= 0 {
                    break;
                }
                ranked.swap(j - 1, j);
                j -= 1;
            }
        }
        ranked.truncate(4);
        for i in 0..MAX_MONS {
            let m = &self.mon[i];
            if !m.used || m.leaving {
                continue;
            }
            let want = ranked.contains(&m.pj);
            if !want && count[m.pj] != 0 {
                self.remove_monster(i, false);
            } else if !want {
                self.defeat(i);
            }
        }
        for &pj in &ranked {
            if self.monster_of(pj).is_none() {
                self.add_monster(pj);
            }
        }
        self.relayout_monsters();
    }

    /* ---- players ------------------------------------------------------------------ */

    fn find(&self, id: &str) -> Option<usize> {
        (0..MAX_PLAYERS).find(|&i| self.pl[i].used && self.pl[i].id == id)
    }

    fn by_order(&self, order: u32) -> Option<usize> {
        if order == 0 {
            return None;
        }
        (0..MAX_PLAYERS).find(|&i| self.pl[i].used && self.pl[i].order == order)
    }

    /// A project's last session has ended: a player finishes its monster with
    /// its final blow before the monster goes. The session that ended, on its
    /// way out, if it is on the field; else the nearest player standing.
    fn defeat(&mut self, mi: usize) {
        let (mx, my, mpj) = (self.mon[mi].x, self.mon[mi].y, self.mon[mi].pj);
        let mut best = (0..MAX_PLAYERS)
            .find(|&i| self.pl[i].used && !self.pl[i].bench && self.pl[i].leaving && self.pl[i].pj == mpj);
        let mut bd = 1e9f32;
        if best.is_none() {
            for i in 0..MAX_PLAYERS {
                let p = &self.pl[i];
                if !p.used || p.bench || p.leaving || p.ko || p.hidden {
                    continue;
                }
                let d = (p.hx - mx) * (p.hx - mx) + (p.hy - my) * (p.hy - my);
                if d < bd {
                    bd = d;
                    best = Some(i);
                }
            }
        }
        let Some(best) = best.filter(|_| !self.instant) else {
            self.remove_monster(mi, true);
            return;
        };
        let m = &mut self.mon[mi];
        m.leaving = true;
        m.doomed = true;
        m.doom_ms = self.ms;
        m.q.clear();
        m.running = false;
        m.attack_frame = false;
        let big = if self.proj[mpj].big > 0.0 { self.proj[mpj].big } else { self.big };
        let dmg = 9999f64.max(big * (LIMIT_X + self.rnd()) as f64);
        self.pl[best].push_front(Act { kind: Ak::Limit, mon: Some(mi), dmg, tool: None });
    }

    fn free_slot(&self) -> Option<usize> {
        (0..PARTY).find(|&s| !self.pl.iter().any(|p| p.used && !p.leaving && !p.bench && p.slot == Some(s)))
    }

    /// Claude Code sessions are mages, Codex sessions fighters. A new session
    /// takes the class of its group with the fewest players, and the palette
    /// swap when that class is already on the field.
    fn pick_class(&mut self, codex: bool) -> (usize, usize) {
        let mut n = [0i32; 8];
        for p in &self.pl {
            if p.used && !p.leaving {
                n[p.cls] += 1;
            }
        }
        let group = |c: usize| CLASSES[c].fighter == codex;
        let least = (0..CLASSES.len()).filter(|&c| group(c)).map(|c| n[c]).min().unwrap_or(1 << 30);
        let pick: Vec<usize> = (0..CLASSES.len()).filter(|&c| group(c) && n[c] == least).collect();
        let cls = if pick.is_empty() { 0 } else { pick[self.rndi(pick.len() as i32) as usize] };
        let mut used = [false; 2];
        for p in &self.pl {
            if p.used && !p.leaving && p.cls == cls {
                used[p.variant] = true;
            }
        }
        let variant = if !used[0] {
            0
        } else if !used[1] {
            1
        } else {
            (least % 2) as usize
        };
        (cls, variant)
    }

    /// A name of that class nobody on the field has.
    fn pick_name(&mut self, cls: usize) -> String {
        let c = &CLASSES[cls];
        let free: Vec<usize> = (0..c.names.len().min(16))
            .filter(|&k| !self.pl.iter().any(|p| p.used && !p.leaving && p.name == c.names[k]))
            .collect();
        if !free.is_empty() {
            let k = free[self.rndi(free.len() as i32) as usize];
            cut(c.names[k], 7)
        } else {
            format!("{}{}", cut(c.names[0], 5), self.next_order % 10)
        }
    }

    fn exposed(&self, p: &Player) -> bool {
        !p.leaving && !p.ko && !p.bench && (p.state == State::Idle || (p.state == State::Wait && self.pace_active))
    }

    fn seat(&mut self, i: usize) -> bool {
        let Some(slot) = self.free_slot() else { return false };
        let instant = self.instant;
        let p = &mut self.pl[i];
        p.slot = Some(slot);
        p.bench = false;
        p.hx = party_sx(slot);
        p.hy = party_sy(slot);
        p.y = p.hy;
        p.pose = None;
        if instant {
            p.x = p.hx;
        } else {
            p.x = (FF_W + 30) as f32;
            p.push(Act::of(Ak::RunIn));
        }
        self.rebalance();
        true
    }

    fn join(&mut self, s: &Session) -> Option<usize> {
        let i = (0..MAX_PLAYERS).find(|&i| !self.pl[i].used)?;
        self.next_order += 1;
        let proj = if s.proj.is_empty() { "?".to_string() } else { cut(&s.proj, PROJ_LEN) };
        let pj = self.project(&proj);
        self.pl[i] =
            Player { order: self.next_order, id: cut(&s.id, ID_LEN), proj, pj, codex: s.codex, ..Player::default() };
        // not yet counted as its own class-mate (used is still false)
        let (cls, variant) = self.pick_class(s.codex);
        let name = self.pick_name(cls);
        let atb = self.rnd() * 0.5;
        let hit_in = HIT_S * self.rnd();
        let p = &mut self.pl[i];
        p.used = true;
        p.name = name;
        p.cls = cls;
        p.variant = variant;
        p.slot = None;
        p.bench = true;
        p.state = s.state;
        p.why = s.why;
        p.hp = HP_MAX;
        p.atb = atb;
        p.hit_in = hit_in;
        if s.state == State::Idle {
            p.idle_s = s.age_s as f32;
            p.hp = idle_hp(p.idle_s);
            p.ko = p.hp <= 0.0;
        }
        let line = format!(
            "{} ({}) joins: a {} session in {}",
            p.name,
            CLASSES[cls].job,
            if s.codex { "Codex" } else { "Claude Code" },
            p.proj
        );
        self.logf(line);
        if !self.seat(i) {
            let line = format!("{} waits off the field: the party is full", self.pl[i].name);
            self.logf(line);
        }
        Some(i)
    }

    fn credit(&mut self, i: usize, pj: usize, dmg: f64) {
        self.pl[i].dealt += dmg;
        self.proj[pj].dmg += dmg;
    }

    fn gone(&mut self, i: usize) {
        self.pl[i].used = false;
        self.fx_forget(None, Some(i));
        if self.card == self.pl[i].order {
            self.card = 0;
        }
        for k in 0..MAX_PLAYERS {
            let q = &self.pl[k];
            if q.used && q.bench && !q.leaving && self.seat(k) {
                return; // seat() rebalances
            }
        }
        self.rebalance();
    }

    fn leave(&mut self, i: usize) {
        if self.pl[i].leaving {
            return;
        }
        self.pl[i].leaving = true;
        self.pl[i].q.clear();
        if self.pl[i].pending > 0.0 {
            let (pj, pending) = (self.pl[i].pj, self.pl[i].pending);
            self.credit(i, pj, pending);
            self.pl[i].pending = 0.0;
        }
        let line = format!("{} leaves (session ended)", self.pl[i].name);
        self.logf(line);
        if self.pl[i].bench {
            self.gone(i);
            return;
        }
        self.pl[i].push(Act::of(Ak::Leave));
    }

    /// A session whose project has no monster fights the nearest one.
    fn target(&self, i: usize) -> Option<usize> {
        let p = &self.pl[i];
        if let Some(own) = self.monster_of(p.pj) {
            return Some(own);
        }
        let mut best = None;
        let mut bd = 1e9f32;
        for (k, m) in self.mon.iter().enumerate() {
            if !m.used || m.leaving {
                continue;
            }
            let d = (m.tx - p.hx) * (m.tx - p.hx) + (m.ty - p.hy) * (m.ty - p.hy);
            if d < bd {
                bd = d;
                best = Some(k);
            }
        }
        best
    }

    fn move_for(&self, i: usize, tool: Tool) -> Mv {
        let cls = self.pl[i].cls;
        if CLASSES[cls].fighter || tool == Tool::Agent { self.special[cls] } else { tool_mv(tool) }
    }

    /* ---- effects ---------------------------------------------------------------------- */

    fn fx_on_monster(&mut self, mv: Mv, mon: Option<usize>) {
        let (k, dur) = match mv {
            Mv::Fire => (Ek::Fire, 1.0),
            Mv::Ice => (Ek::Ice, 1.0),
            Mv::Bolt => (Ek::Bolt, 0.7),
            Mv::Scan => (Ek::Scan, 1.0),
            Mv::Gale => (Ek::Gale, 1.0),
            Mv::Flare => (Ek::Flare, 1.2),
            Mv::Holy => (Ek::Holy, 1.3),
            _ => return,
        };
        self.fx_add(k, dur, mon, None);
    }

    fn fx_sparkle_at(&mut self, pl: Option<usize>, x: f32, y: f32, c: u16) {
        if let Some(e) = self.fx_add(Ek::Sparkle, 0.9, None, pl) {
            e.x = x;
            e.y = y;
            e.col = c;
        }
    }

    fn fx_strike(&mut self, mon: Option<usize>, kind: Strike) {
        if let Some(e) = self.fx_add(Ek::Strike, 0.3, mon, None) {
            e.arg = kind as i32;
        }
    }

    /* ---- player actions -------------------------------------------------------------------- */

    fn act_attack(&mut self, i: usize, tl: Tl) -> bool {
        let m = self.mon_at(self.pl[i].tm);
        if tl.at(0.0) && m.is_none() {
            return true;
        }
        let p = &mut self.pl[i];
        p.walk(tl, 0.0, 0.18, p.hx - 16.0, p.hy, Walk::Left);
        if tl.at(0.18) {
            p.pose = Some(Pose::Attack);
        }
        if tl.at(0.24) {
            let dmg = p.cur.dmg;
            let pj = m.map_or(p.pj, |m| self.mon[m].pj);
            self.credit(i, pj, dmg);
            self.big = self.big.max(dmg);
            if let Some(m) = m {
                let pj = self.mon[m].pj;
                self.proj[pj].big = self.proj[pj].big.max(dmg);
                self.fx_strike(Some(m), CLASSES[self.pl[i].cls].strike);
                let mm = &mut self.mon[m];
                mm.white = 0.25;
                mm.shake = 0.25;
                let (x, y) = (mm.cx(), mm.cy() - 6.0);
                self.pop_num(x, y, &fmt_tok(dmg), c::WHITE);
            }
        }
        let p = &mut self.pl[i];
        if tl.at(0.64) {
            p.pose = None;
        }
        p.walk(tl, 0.64, 0.82, p.hx, p.hy, Walk::Left);
        tl.t1 >= 0.82
    }

    /// A spell, or a mage's special; a Monk's chi burst runs here too.
    fn act_move(&mut self, i: usize, tl: Tl) -> bool {
        let m = self.mon_at(self.pl[i].tm);
        let (mv, tool, cls) = (self.pl[i].mv, self.pl[i].cur.tool.unwrap_or(Tool::Bash), self.pl[i].cls);
        let special = CLASSES[cls].fighter || tool == Tool::Agent;
        let w1 = if mv == Mv::ChiBurst { 0.15 } else { 0.45 };
        let t1 = 0.14 + w1; // the spell lands
        let t1b = if mv == Mv::Doublecast { t1 + 0.55 } else { t1 }; // the second of a doublecast
        let t2 = t1b
            + match mv {
                Mv::Summon => 1.2,
                Mv::Flare | Mv::Holy => 0.9,
                _ => 0.5,
            };
        let t3 = t2 + 0.25;
        let p = &mut self.pl[i];
        p.walk(tl, 0.0, 0.14, p.hx - 10.0, p.hy, Walk::Left);
        if tl.at(0.14) {
            p.pose = Some(if mv == Mv::ChiBurst { Pose::Attack } else { Pose::Cast });
            let (sub0, sub1) = (p.sub0 as usize, p.sub1 as usize);
            let mut sub = String::new();
            let mut name = mv_name(mv);
            if mv == Mv::Summon {
                name = SUMMON[sub0];
                sub = "SUMMON".into();
            }
            if mv == Mv::Doublecast {
                sub = format!("{} + {}", mv_name(DUAL[sub0]), mv_name(DUAL[sub1]));
            }
            self.banner(name, &sub, if special { 2 } else { 1 });
            let verb = if mv == Mv::Summon {
                "summons"
            } else if special {
                "uses"
            } else {
                "casts"
            };
            let dual = if mv == Mv::Doublecast { format!(" ({sub})") } else { String::new() };
            let line = format!("{} {verb} {name}{dual} for {}", self.pl[i].name, tool_label(tool));
            self.logf(line);
            if mv != Mv::ChiBurst {
                self.fx_add(Ek::Chant, 0.5, None, Some(i));
            }
        }
        if tl.at(t1) {
            if mv == Mv::Protect {
                self.fx_add(Ek::Protect, 1.0, None, Some(i));
            } else if m.is_some() {
                let tm = self.pl[i].tm;
                if mv == Mv::Summon {
                    let sub0 = self.pl[i].sub0;
                    if let Some(e) = self.fx_add(Ek::Summon, 1.8, tm, None) {
                        e.arg = sub0;
                    }
                } else if mv == Mv::ChiBurst {
                    self.fx_add(Ek::ChiBurst, 0.75, tm, Some(i));
                } else if mv == Mv::Doublecast {
                    self.fx_on_monster(DUAL[self.pl[i].sub0 as usize], tm);
                } else {
                    self.fx_on_monster(mv, tm);
                }
            }
        }
        if mv == Mv::Doublecast && m.is_some() && tl.at(t1b) {
            self.fx_on_monster(DUAL[self.pl[i].sub1 as usize], self.pl[i].tm);
        }
        if let Some(m) = m
            && tl.at(t2)
            && mv != Mv::Protect
        {
            self.mon[m].white = 0.2;
        }
        let p = &mut self.pl[i];
        if tl.at(t3) {
            p.pose = None;
        }
        p.walk(tl, t3, t3 + 0.14, p.hx, p.hy, Walk::Left);
        tl.t1 >= t3 + 0.14
    }

    /// The Warrior's cross cut and the Thief's steal: a dash to the monster and back.
    fn act_dash(&mut self, i: usize, tl: Tl) -> bool {
        let m = self.mon_at(self.pl[i].tm);
        let mv = self.pl[i].mv;
        if tl.at(0.0) {
            if m.is_none() {
                return true;
            }
            self.banner(mv_name(mv), "", 2);
        }
        let tool = tool_label(self.pl[i].cur.tool.unwrap_or(Tool::Bash));
        let (tx, ty) = match m {
            Some(m) => (self.mon[m].x + self.mon[m].idle_w() / 2.0 + 12.0, self.mon[m].y),
            None => (self.pl[i].hx, self.pl[i].hy),
        };
        self.pl[i].walk(tl, 0.0, 0.22, tx, ty, Walk::Left);
        if mv == Mv::Cross {
            if tl.at(0.22) {
                self.pl[i].pose = Some(Pose::Attack);
                let line = format!("{} uses CROSS CUT for {tool}", self.pl[i].name);
                self.logf(line);
                if m.is_some() {
                    self.fx_add(Ek::Cross, 0.55, m, None);
                }
            }
            if let Some(m) = m
                && tl.at(0.34)
            {
                self.mon[m].white = 0.5;
                self.mon[m].shake = 0.35;
            }
            let p = &mut self.pl[i];
            if tl.at(0.74) {
                p.pose = None;
            }
            p.walk(tl, 0.74, 1.04, p.hx, p.hy, Walk::Right);
            return tl.t1 >= 1.04;
        }
        if tl.at(0.22) {
            self.pl[i].pose = Some(Pose::Attack);
            if let Some(m) = m {
                let (x, y) = (self.mon[m].x + 6.0, self.mon[m].y - 4.0);
                self.fx_sparkle_at(None, x, y, c::GOLD);
            }
        }
        if tl.at(0.37) {
            let loot = if self.rnd() < 0.15 { None } else { Some(self.rndi(LOOT.len() as i32) as usize) };
            let sub = match loot {
                None => "MISSED!".to_string(),
                Some(k) => format!("GOT {}!", LOOT[k]),
            };
            self.banner("STEAL", &sub, 2);
            let line = format!("{} uses STEAL for {tool}: {}", self.pl[i].name, loot.map_or("missed", |k| LOOT[k]));
            self.logf(line);
        }
        let p = &mut self.pl[i];
        if tl.at(0.72) {
            p.pose = None;
        }
        p.walk(tl, 0.72, 1.02, p.hx, p.hy, Walk::Right);
        tl.t1 >= 1.02
    }

    /// The Lancer's jump: up off the screen, a shadow on the monster, down on it, back.
    fn act_jump(&mut self, i: usize, tl: Tl) -> bool {
        let m = self.mon_at(self.pl[i].tm);
        if tl.at(0.0) {
            if m.is_none() {
                return true;
            }
            self.banner("JUMP", "", 2);
            let tool = tool_label(self.pl[i].cur.tool.unwrap_or(Tool::Bash));
            let line = format!("{} uses JUMP for {tool}", self.pl[i].name);
            self.logf(line);
            let p = &mut self.pl[i];
            p.air = true;
            p.pose = Some(Pose::Kneel);
            p.ay = p.y;
        }
        let (mx, my) = match m {
            Some(m) => (self.mon[m].x, self.mon[m].y),
            None => (self.pl[i].hx, self.pl[i].hy),
        };
        let p = &mut self.pl[i];
        if tl.at(0.15) {
            p.pose = Some(Pose::Ready);
        }
        let k = tl.seg(0.15, 0.37);
        if k >= 0.0 {
            p.y = p.ay - k * k * 200.0;
        }
        if tl.at(0.37) {
            p.hidden = true;
            if m.is_some() {
                self.fx_add(Ek::Shadow, 0.6, m, None);
            }
        }
        let p = &mut self.pl[i];
        if tl.at(0.97) {
            p.hidden = false;
            p.diving = true;
            p.pose = Some(Pose::Kneel);
            p.x = mx + 6.0;
        }
        let k = tl.seg(0.97, 1.11);
        if k >= 0.0 {
            let top = my - 160.0;
            p.y = top + (my + 2.0 - top) * k;
        }
        if tl.at(1.11) {
            p.diving = false;
            if let Some(m) = m {
                self.fx_strike(Some(m), Strike::Bonk);
                self.mon[m].white = 0.6;
                self.mon[m].shake = 0.4;
            }
            if self.flash < 0.25 {
                self.flash = 0.25;
            }
        }
        let p = &mut self.pl[i];
        if tl.at(1.36) {
            p.pose = Some(Pose::Ready);
        }
        let k = tl.seg(1.36, 1.54);
        if k >= 0.0 {
            p.y = my + 2.0 - k * 170.0;
        }
        if tl.at(1.54) {
            p.x = p.hx;
        }
        let k = tl.seg(1.54, 1.84);
        if k >= 0.0 {
            p.y = p.hy - 170.0 + 170.0 * k;
        }
        if tl.at(1.84) {
            p.pose = None;
            p.air = false;
            p.y = p.hy;
            return true;
        }
        false
    }

    fn act_revive(&mut self, i: usize, tl: Tl) -> bool {
        if tl.at(0.0) {
            self.banner("PHOENIX PLUME", "", 2);
            let line = format!("PHOENIX PLUME on {}: the session woke up", self.pl[i].name);
            self.logf(line);
            self.fx_add(Ek::Feather, 1.0, None, Some(i));
            if !self.instant {
                self.events |= ev::REVIVE;
            }
        }
        if tl.at(1.0) {
            self.fx_sparkle_at(Some(i), 0.0, 0.0, c::GOLD);
            self.pl[i].white = 0.6;
        }
        if tl.at(1.3) {
            let p = &mut self.pl[i];
            p.ko = false;
            p.hp = HP_MAX;
            let (x, y) = (p.x, p.y - 34.0);
            self.pop_num(x, y, "999", c::GREEN);
        }
        tl.t1 >= 1.7
    }

    fn act_cure(&mut self, i: usize, tl: Tl) -> bool {
        if tl.at(0.0) {
            self.pl[i].sub0 = (HP_MAX - self.pl[i].hp + 0.5) as i32;
            self.banner("CURE", "", 1);
            self.fx_sparkle_at(Some(i), 0.0, 0.0, c::GREEN);
        }
        if tl.at(0.5) {
            let p = &mut self.pl[i];
            p.hp = HP_MAX;
            let (x, y, s) = (p.x, p.y - 34.0, p.sub0.to_string());
            self.pop_num(x, y, &s, c::GREEN);
        }
        tl.t1 >= 0.8
    }

    /// The final blow that finishes a monster (defeat()).
    fn act_limit(&mut self, i: usize, tl: Tl) -> bool {
        let m = self.mon_at(self.pl[i].tm);
        let li = self.limit[self.pl[i].cls];
        let lim = &LIMITS[li];
        let (lancer, melee) = (lim.lb == Lb::LanceRain, lim.melee);
        let (hit, end) = (LIMIT_GO + lim.impact, LIMIT_GO + lim.dur);
        if tl.at(0.0) {
            let p = &mut self.pl[i];
            p.ko = false;
            p.hidden = false;
            p.air = false;
            p.diving = false;
            p.pose = Some(Pose::Ready);
            if let Some(m) = m {
                self.banner(lim.name, "FINAL BLOW", 3);
                let line = format!(
                    "{}'s final blow: {} on {} for {}",
                    self.pl[i].name,
                    lim.name,
                    MONSTERS[self.mon[m].kind].name,
                    fmt_tok(self.pl[i].cur.dmg)
                );
                self.logf(line);
                self.fx_add(Ek::LimitAura, 0.7, None, Some(i));
            }
        }
        if m.is_none() && tl.t1 < hit {
            // the monster went without it: stand down
            let p = &mut self.pl[i];
            p.hidden = false;
            p.air = false;
            p.pose = None;
            p.x = p.hx;
            p.y = p.hy;
            return true;
        }
        if lancer {
            let p = &mut self.pl[i];
            if tl.at(0.5) {
                p.air = true;
                p.pose = Some(Pose::Kneel);
                p.ay = p.y;
            }
            let k = tl.seg(0.55, LIMIT_GO);
            if k >= 0.0 {
                p.pose = Some(Pose::Ready);
                p.y = p.ay - k * k * 200.0;
            }
            if tl.at(LIMIT_GO) {
                p.hidden = true;
            }
            if tl.at(hit + 0.3) {
                p.hidden = false;
                p.x = p.hx;
                p.pose = Some(Pose::Kneel);
            }
            let k = tl.seg(hit + 0.3, hit + 0.6);
            if k >= 0.0 {
                p.y = p.hy - 170.0 + 170.0 * k;
            }
        } else if let Some(m) = m {
            let (mx, my, mw) = (self.mon[m].x, self.mon[m].y, self.mon[m].idle_w());
            let p = &mut self.pl[i];
            if melee {
                p.walk(tl, 0.5, 0.72, mx + mw / 2.0 + 12.0, my, Walk::Left);
            } else {
                p.walk(tl, 0.5, 0.64, p.hx - 10.0, p.hy, Walk::Left);
            }
        }
        if m.is_some() && tl.at(LIMIT_GO) {
            if !lancer {
                self.pl[i].pose = Some(if melee { Pose::Attack } else { Pose::Cast });
            }
            let tm = self.pl[i].tm;
            if let Some(e) = self.fx_add(Ek::Limit, lim.dur, tm, Some(i)) {
                e.arg = lim.lb as i32;
            }
        }
        if let Some(m) = m
            && tl.at(hit)
        {
            let mm = &mut self.mon[m];
            mm.white = 1.0;
            mm.shake = 0.5;
            self.flash = self.flash.max(0.8);
            let (x, y) = (mm.cx(), mm.cy() - 14.0);
            let s = fmt_tok(self.pl[i].cur.dmg);
            self.pop_num_k(x, y, &s, c::WHITE, 2);
            if self.mon[m].doomed {
                self.remove_monster(m, true);
            }
        }
        let p = &mut self.pl[i];
        if tl.at(end) && !lancer {
            p.pose = None;
        }
        if melee {
            p.walk(tl, end, end + 0.3, p.hx, p.hy, Walk::Right);
        } else if !lancer {
            p.walk(tl, end, end + 0.14, p.hx, p.hy, Walk::Left);
        }
        let done = if lancer { hit + 0.6 } else { end + 0.3 };
        if tl.t1 >= done {
            p.pose = None;
            p.air = false;
            p.hidden = false;
            p.x = p.hx;
            p.y = p.hy;
            return true;
        }
        false
    }

    fn act_start(&mut self, i: usize) {
        self.pl[i].tm = if self.pl[i].cur.kind == Ak::Limit { self.pl[i].cur.mon } else { self.target(i) };
        if self.pl[i].cur.kind == Ak::Move {
            let mv = self.move_for(i, self.pl[i].cur.tool.unwrap_or(Tool::Bash));
            self.pl[i].mv = mv;
            if mv == Mv::Cross || mv == Mv::Steal {
                self.pl[i].cur.kind = Ak::Dash;
            } else if mv == Mv::Jump {
                self.pl[i].cur.kind = Ak::Jump;
            }
            self.pl[i].sub0 = self.rndi(SUMMON.len() as i32);
            if mv == Mv::Doublecast {
                let n = DUAL.len() as i32;
                let s0 = self.rndi(n);
                let s1 = (s0 + 1 + self.rndi(n - 1)) % n;
                self.pl[i].sub0 = s0;
                self.pl[i].sub1 = s1;
            }
        }
    }

    /// Run the current action (a leave frees the player).
    fn p_step(&mut self, i: usize, dt: f32) {
        let p = &mut self.pl[i];
        if !p.running && !p.q.is_empty() {
            p.cur = p.q.remove(0);
            p.running = true;
            p.at_prev = -1.0;
            p.at = 0.0;
            self.act_start(i);
        } else if p.running {
            p.at_prev = p.at;
            p.at += dt;
        }
        let p = &mut self.pl[i];
        if !p.running {
            return;
        }
        let tl = Tl { t0: p.at_prev, t1: p.at };
        let done = match p.cur.kind {
            Ak::RunIn => {
                p.walk(tl, 0.0, 0.9, p.hx, p.hy, Walk::Left);
                tl.t1 >= 0.9
            }
            Ak::Attack => self.act_attack(i, tl),
            Ak::Move => self.act_move(i, tl),
            Ak::Dash => self.act_dash(i, tl),
            Ak::Jump => self.act_jump(i, tl),
            Ak::Revive => self.act_revive(i, tl),
            Ak::Cure => self.act_cure(i, tl),
            Ak::Limit => self.act_limit(i, tl),
            Ak::Leave => {
                if tl.at(0.0) {
                    p.ko = false;
                    p.pose = None;
                    p.air = false;
                    p.hidden = false;
                    p.diving = false;
                }
                p.walk(tl, 0.0, 0.8, (FF_W + 40) as f32, p.y, Walk::Right);
                if tl.t1 >= 0.8 {
                    p.running = false;
                    self.gone(i);
                    return;
                }
                false
            }
        };
        if done {
            let p = &mut self.pl[i];
            p.running = false;
            p.walking = Walk::None;
        }
    }

    /* ---- monster actions ------------------------------------------------------------------ */

    fn mon_attack_fx(&mut self, mi: usize, pi: usize) {
        const DUR: [f32; 8] = [0.5, 1.0, 0.8, 0.8, 1.0, 1.1, 0.8, 1.1];
        const IMPACT: [f32; 8] = [0.15, 0.45, 0.5, 0.4, 0.65, 0.65, 0.5, 0.65];
        let fx = MONSTERS[self.mon[mi].kind].fx as usize;
        self.fx_add(MON_EK[fx], DUR[fx], Some(mi), Some(pi));
        self.mon[mi].impact = IMPACT[fx];
    }

    fn m_step(&mut self, mi: usize, dt: f32) {
        let m = &mut self.mon[mi];
        if !m.running && !m.q.is_empty() {
            m.cur = m.q.remove(0);
            m.running = true;
            m.at_prev = -1.0;
            m.at = 0.0;
        } else if m.running {
            m.at_prev = m.at;
            m.at += dt;
        }
        if !m.running {
            return;
        }
        let tl = Tl { t0: m.at_prev, t1: m.at };
        let mon = &MONSTERS[m.kind];
        match m.cur.kind {
            Mk::Appear => {
                if tl.at(0.0) {
                    self.banner(&format!("{} APPEARS", mon.name), "", 1);
                }
                let m = &mut self.mon[mi];
                m.vis = (m.at / 0.7).clamp(0.0, 1.0);
                if m.at >= 0.7 {
                    m.running = false;
                }
            }
            Mk::Go => {
                if tl.at(0.0) {
                    let defeated = m.defeated;
                    let b = format!("{} {}", mon.name, if defeated { "DEFEATED" } else { "FLEES" });
                    let sub = format!("{} DMG", fmt_tok(self.proj[m.pj].dmg));
                    // over the final blow's
                    self.banner(&b, if defeated { &sub } else { "" }, if defeated { 3 } else { 1 });
                    if defeated && !self.instant {
                        self.events |= ev::FANFARE;
                    }
                }
                let m = &mut self.mon[mi];
                m.sweep = (m.at / 1.1).clamp(0.0, 1.0);
                if m.at >= 1.1 {
                    m.used = false;
                    self.fx_forget(Some(mi), None);
                    for p in self.pl.iter_mut() {
                        if p.tm == Some(mi) {
                            p.tm = None;
                        }
                    }
                    self.relayout_monsters();
                }
            }
            Mk::Attack => self.m_attack(mi, tl),
        }
    }

    /// An idle player's hit; past KO_S the monster's finisher, which knocks it out.
    fn m_attack(&mut self, mi: usize, tl: Tl) {
        let mon = &MONSTERS[self.mon[mi].kind];
        let prey = self.by_order(self.mon[mi].cur.prey).filter(|&p| !self.pl[p].leaving && !self.pl[p].bench);
        let Some(pi) = prey else {
            self.mon[mi].running = false;
            self.mon[mi].attack_frame = false;
            return;
        };
        let fin = self.mon[mi].cur.finisher;
        let go = if fin { 0.45 } else { 0.32 };
        if tl.at(0.0) {
            self.banner(if fin { FINISHER[mon.fx as usize] } else { mon.attack }, "", if fin { 2 } else { 1 });
            let line = format!(
                "{} uses {} on {} (idle{})",
                mon.name,
                if fin { FINISHER[mon.fx as usize] } else { mon.attack },
                self.pl[pi].name,
                if fin { " 10 min" } else { "" }
            );
            self.logf(line);
            if fin {
                self.mon[mi].shake = 0.6;
            }
        }
        for k in 0..(if fin { 3 } else { 2 }) {
            let t0 = k as f32 * if fin { 0.15 } else { 0.16 };
            if tl.at(t0) {
                self.mon[mi].white = if fin { 0.9 } else { 0.7 };
            }
            if tl.at(t0 + if fin { 0.08 } else { 0.09 }) {
                self.mon[mi].white = 0.0;
            }
        }
        if tl.at(go) {
            self.mon[mi].attack_frame = true;
            if fin {
                if let Some(e) = self.fx_add(Ek::MFinish, FIN_DUR, Some(mi), Some(pi)) {
                    e.arg = mon.fx as i32;
                }
                self.mon[mi].impact = FIN_IMPACT;
            } else {
                self.mon_attack_fx(mi, pi);
            }
        }
        let impact = self.mon[mi].impact;
        if self.mon[mi].at_prev >= go && tl.at(go + impact) {
            let (px, py) = (self.pl[pi].x, self.pl[pi].y);
            if !self.exposed(&self.pl[pi]) {
                // it woke up in time
                self.pop_num(px, py - 34.0, "MISS", c::WHITE);
                let line = format!("{} misses {}: the session woke up", mon.name, self.pl[pi].name);
                self.logf(line);
            } else if fin {
                let [lo, spread] = FINISHER_DMG[mon.fx as usize];
                let s = (lo + self.rndi(spread + 1)).to_string();
                self.pop_num_k(px, py - 40.0, &s, c::RED, 2);
                let p = &mut self.pl[pi];
                p.hp = 0.0;
                p.ko = true;
                p.hurt = 0.35;
                p.kb = 16.0;
                let line = format!("{} is knocked out (idle 10 min)", self.pl[pi].name);
                self.logf(line);
            } else {
                // HP follows the idle time; only the finisher takes the last of it
                let p = &mut self.pl[pi];
                let dmg = (1f32.max(p.hp - idle_hp(p.idle_s)) + 0.5).floor();
                p.hp = 1f32.max(p.hp - dmg);
                p.hurt = 0.35;
                self.pop_num(px, py - 34.0, &(dmg as i32).to_string(), c::WHITE);
            }
        }
        let m = &mut self.mon[mi];
        if m.at >= go + m.impact + if fin { 0.6 } else { 0.35 } {
            m.attack_frame = false;
            m.running = false;
        }
    }

    /* ---- the collectors' news ------------------------------------------------------------------ */

    fn chime(&mut self, why: Why) {
        if self.chime_at != 0 && self.ms - self.chime_at < CHIME_GAP_MS {
            return;
        }
        self.chime_at = if self.ms != 0 { self.ms } else { 1 };
        self.events |= if why == Why::Ask { ev::CHIME_ASK } else { ev::CHIME_PERM };
    }

    fn set_state(&mut self, i: usize, st: State, why: Why) {
        let p = &mut self.pl[i];
        let prev = p.state;
        p.state = st;
        p.why = why;
        if p.leaving || p.bench {
            return;
        }
        match st {
            State::Work => {
                if p.ko {
                    p.push(Act::of(Ak::Revive));
                } else if p.hp < HP_MAX {
                    p.push(Act::of(Ak::Cure));
                }
                if prev != State::Work {
                    let line = format!("{} gets to work", p.name);
                    self.logf(line);
                }
            }
            State::Wait => {
                let line = format!(
                    "{} raises a hand: {}",
                    p.name,
                    if why == Why::Ask { "has a question" } else { "needs your OK" }
                );
                if p.ko {
                    p.push(Act::of(Ak::Revive));
                }
                self.logf(line);
            }
            State::Idle => {
                let line = format!("{} is idle: the monster will attack", p.name);
                self.logf(line);
                let k = self.rnd();
                self.pl[i].hit_in = HIT_S * (0.8 + 0.4 * k);
            }
        }
    }

    /// Reconcile with the full session list. host: when the collecting side
    /// started; a new one starts the damage counters over. The first list
    /// sets the field at once; later newcomers run in and leavers run off.
    pub fn set_sessions(&mut self, list: &[Session], host: i64) {
        self.instant = !self.listed;
        self.defer = true;
        if self.listed && host != 0 && self.host != 0 && host != self.host {
            self.logf("a new collector: the damage counters start over".into());
            for p in self.proj.iter_mut() {
                p.dmg = 0.0;
                p.big = 0.0;
            }
            for p in self.pl.iter_mut() {
                p.dealt = 0.0;
                p.pending = 0.0;
                p.have_burn = false;
            }
        }
        self.host = host;
        self.listed = true;
        self.list_ms = self.ms;
        for p in self.pl.iter_mut() {
            p.seen = false;
        }
        for s in list {
            let id = cut(&s.id, ID_LEN);
            let (i, fresh) = match self.find(&id) {
                Some(i) if !self.pl[i].leaving => (i, false),
                Some(_) => continue, // it went and came back while running off: next list
                None => match self.join(s) {
                    Some(i) => (i, true),
                    None => continue,
                },
            };
            self.pl[i].seen = true;
            let changed = fresh || self.pl[i].state != s.state || self.pl[i].why != s.why;
            if changed {
                self.pl[i].since_ms = self.ms - s.age_s as i64 * 1000;
                if !self.instant && s.state == State::Wait {
                    self.chime(s.why);
                }
                if !fresh {
                    self.set_state(i, s.state, s.why);
                }
            }
            let proj = if s.proj.is_empty() { "?".to_string() } else { cut(&s.proj, PROJ_LEN) };
            if self.pl[i].proj != proj {
                self.pl[i].pj = self.project(&proj);
                self.pl[i].proj = proj;
            }
            let ms = self.ms;
            let pace_active = self.pace_active;
            let p = &mut self.pl[i];
            if (p.state == State::Idle || (p.state == State::Wait && pace_active)) && !p.ko {
                p.idle_s = s.age_s as f32;
            }
            p.term = s.term;
            p.have_started = s.up_s > 0;
            p.started_ms = ms - s.up_s as i64 * 1000;
            p.tokens = s.tokens;
            p.usd = s.usd;
            let said = cut(&s.said, SAID_LEN);
            let new_said = fresh || p.said_kind != s.said_kind || p.said != said;
            if new_said {
                p.said = said;
                p.said_kind = s.said_kind;
            }
            if new_said || (!p.have_said_at && s.said_age_s >= 0) {
                p.have_said_at = s.said_age_s >= 0;
                p.said_ms = ms - s.said_age_s as i64 * 1000;
            }
            // the tokens burned: the first count at start is done damage, later ones are swung
            if s.burn >= 0.0 {
                if !p.have_burn {
                    if self.instant {
                        let pj = p.pj;
                        self.credit(i, pj, s.burn);
                    } else {
                        p.pending += s.burn;
                    }
                } else if s.burn > p.burn {
                    p.pending += s.burn - p.burn;
                }
                let p = &mut self.pl[i];
                p.burn = s.burn;
                p.have_burn = true;
            }
        }
        for i in 0..MAX_PLAYERS {
            let p = &self.pl[i];
            if p.used && !p.seen && !p.leaving {
                self.leave(i);
            }
        }
        self.defer = false;
        self.rebalance();
        self.instant = false;
    }

    /// A tool call by a session.
    pub fn tool(&mut self, id: &str, tool: Tool) {
        let Some(i) = self.find(&cut(id, ID_LEN)) else { return };
        let p = &mut self.pl[i];
        if p.leaving || p.bench || p.ko || p.state != State::Work {
            return;
        }
        if p.next_tool == Some(Tool::Agent) && tool != Tool::Agent {
            return; // a subagent's special wins
        }
        p.next_tool = Some(tool);
    }

    /// Local time for the clock and the sky: UTC epoch seconds plus the zone offset.
    pub fn set_time(&mut self, epoch: i64, tz: i32) {
        self.have_time = true;
        self.epoch = epoch;
        self.epoch_ms = self.ms;
        self.tz = tz;
    }

    /// This month's totals (GOLD and EXP), Claude and Codex summed; "--" until set.
    pub fn set_usage(&mut self, usage: Usage) {
        self.have_usage = true;
        self.usage = usage;
    }

    /// The MP bars; "--" until set.
    pub fn set_mana(&mut self, mana: Mana) {
        self.mana = mana;
    }

    /// What the config window shows; pace_active also changes who monsters hit.
    pub fn set_muted(&mut self, muted: bool) {
        self.muted = muted;
    }

    pub fn set_fullscreen(&mut self, full: bool) {
        self.fullscreen = full;
    }

    pub fn set_pace_active(&mut self, active: bool) {
        self.pace_active = active;
    }

    /// Sessions in the battle, on the field or waiting off it.
    pub fn agent_count(&self) -> usize {
        self.pl.iter().filter(|p| p.used && !p.leaving).count()
    }

    /// Battle events as one line each ("MORVEN casts FIRE for Bash"), since the last call.
    pub fn take_log(&mut self) -> Vec<String> {
        std::mem::take(&mut self.log)
    }

    /* ---- tick ---------------------------------------------------------------------- */

    pub fn tick(&mut self, dt_ms: u32) {
        let dt = dt_ms as f32 / 1000.0;
        self.ms += dt_ms as i64;
        self.time += dt as f64;
        self.t = (self.time % 1200.0) as f32;
        if self.config && self.ms - self.config_at > CONFIG_MS {
            self.config = false;
        }
        for i in 0..MAX_PLAYERS {
            if !self.pl[i].used {
                continue;
            }
            if self.pl[i].bench {
                // off the field: its tokens still count, unseen
                if self.pl[i].pending > 0.0 {
                    let (pj, pending) = (self.pl[i].pj, self.pl[i].pending);
                    self.credit(i, pj, pending);
                    self.pl[i].pending = 0.0;
                }
                continue;
            }
            let p = &mut self.pl[i];
            p.hurt = 0f32.max(p.hurt - dt);
            p.kb -= p.kb * 1f32.min(dt * 10.0);
            p.white = 0f32.max(p.white - dt * 2.0);
            p.cast_wait = 0f32.max(p.cast_wait - dt);
            if !p.leaving && !p.ko && p.state == State::Work {
                p.atb = 1f32.min(p.atb + dt / ATB_S);
                if !p.busy() {
                    if p.atb >= 1.0 && p.pending > 0.0 {
                        p.push(Act { kind: Ak::Attack, dmg: p.pending, ..Act::default() });
                        p.pending = 0.0;
                        p.atb = 0.0;
                    } else if let Some(t) = p.next_tool.filter(|_| p.cast_wait == 0.0) {
                        p.push(Act { kind: Ak::Move, tool: Some(t), ..Act::default() });
                        p.next_tool = None;
                        p.cast_wait = CAST_GAP_S;
                    }
                }
            } else {
                if p.state != State::Wait {
                    p.atb = 0f32.max(p.atb - dt / 2.0);
                }
                // tokens are counted a few seconds behind: a session that has just
                // stopped still swings what it burned, at once
                if !p.leaving && !p.ko && p.pending > 0.0 && !p.busy() {
                    p.push(Act { kind: Ak::Attack, dmg: p.pending, ..Act::default() });
                    p.pending = 0.0;
                }
            }
            let want = if p.waiting() && !p.leaving { 1.0 } else { 0.0 };
            p.fwd += (want - p.fwd) * 1f32.min(dt * 8.0);
            if self.exposed(&self.pl[i]) {
                let p = &mut self.pl[i];
                p.idle_s += dt;
                p.hit_in -= dt;
                if (p.hit_in <= 0.0 || p.idle_s >= KO_S) && !p.busy() {
                    let (finisher, prey) = (p.idle_s >= KO_S, p.order);
                    if let Some(m) = self.mon_at(self.target(i))
                        && self.mon[m].vis >= 1.0
                        && !self.mon[m].busy()
                    {
                        self.mon[m].push(Mact { kind: Mk::Attack, finisher, prey });
                        let k = self.rnd();
                        self.pl[i].hit_in = HIT_S * (0.8 + 0.4 * k);
                    }
                }
            }
            self.p_step(i, dt);
        }
        for i in 0..MAX_MONS {
            if !self.mon[i].used {
                continue;
            }
            let m = &mut self.mon[i];
            m.white = 0f32.max(m.white - dt * 2.0);
            m.shake = 0f32.max(m.shake - dt);
            m.x += (m.tx - m.x) * 1f32.min(dt * 4.0);
            m.y += (m.ty - m.y) * 1f32.min(dt * 4.0);
            if m.doomed && self.ms - m.doom_ms > DOOM_MS {
                self.remove_monster(i, true);
            }
            self.m_step(i, dt);
        }
        self.fx.retain_mut(|e| {
            e.t += dt / e.dur;
            e.t < 1.0
        });
        self.nums.retain_mut(|n| {
            n.t += dt;
            n.t < 1.2
        });
        if self.banner.on {
            self.banner.t += dt;
            if self.banner.t > self.banner.dur {
                self.banner.on = false;
            }
        }
        self.flash = 0f32.max(self.flash - dt * 2.5);
    }

    /* ---- clicks ---------------------------------------------------------------------- */

    fn card_player(&self) -> Option<usize> {
        self.by_order(self.card)
    }

    /// The field's party in slot order (not those running off).
    fn party_rows(&self) -> Vec<usize> {
        (0..PARTY)
            .filter_map(|s| {
                (0..MAX_PLAYERS).find(|&i| {
                    let p = &self.pl[i];
                    p.used && !p.bench && !p.leaving && p.slot == Some(s)
                })
            })
            .collect()
    }

    fn hit(&self, x: i32, y: i32) -> Option<usize> {
        let mut best = None;
        let mut bd = 1 << 30;
        for (i, p) in self.pl.iter().enumerate() {
            if !p.used || p.leaving || p.bench || p.hidden {
                continue;
            }
            let (dx, dy) = (x - iround(p.x - p.fwd * 8.0), y - iround(p.y - 16.0));
            if dx.abs() < 14 && dy.abs() < 20 && dx * dx + dy * dy < bd {
                best = Some(i);
                bd = dx * dx + dy * dy;
            }
        }
        if best.is_none() && y >= ROW_Y && inside(x, y, PARTY_X, WIN_Y, PARTY_W, WIN_H) {
            let rows = self.party_rows();
            let r = ((y - ROW_Y) / ROW_H) as usize;
            best = rows.get(r).copied();
        }
        best
    }

    fn close_card(&mut self) {
        self.card = 0;
        self.events |= ev::BLIP_LOW;
    }

    fn config_tap(&mut self, x: i32, y: i32) {
        self.config_at = self.ms;
        if inside(x, y, CFG_BTN_X, cfg_row_y(0), CFG_BTN_W, CFG_BTN_H) {
            self.events |= ev::FULLSCREEN | ev::BLIP;
        } else if inside(x, y, CFG_BTN_X, cfg_row_y(1), CFG_BTN_W, CFG_BTN_H) {
            self.events |= ev::MUTE;
        } else if inside(x, y, CFG_BTN_X, cfg_row_y(2), CFG_BTN_W, CFG_BTN_H) {
            self.events |= ev::PACE | ev::BLIP;
        } else if inside(x, y, CFG_BTN_X, CFG_CLOSE_Y, CFG_BTN_W, CFG_BTN_H)
            || !inside(x, y, CFG_X, CFG_Y, CFG_W, CFG_H)
        {
            self.config = false;
            self.events |= ev::BLIP_LOW;
        }
    }

    /// A click at canvas coordinates.
    pub fn tap(&mut self, x: i32, y: i32) {
        if let Some(e) = self.fx_add(Ek::Tap, 0.3, None, None) {
            e.x = x as f32;
            e.y = y as f32;
        }
        if self.config {
            self.config_tap(x, y);
            return;
        }
        let c = self.card_player();
        if let Some(ci) = c
            && self.pl[ci].term
            && inside(x, y, CARD_PORT_X, CARD_PORT_Y, CARD_PORT_W, CARD_PORT_H)
        {
            self.term_order = self.pl[ci].order;
            self.term_at = self.ms;
            self.events |= ev::TERMINAL | ev::BLIP;
            return;
        }
        if c.is_some() && inside(x, y, CARD_X, CARD_Y, CARD_W, CARD_H) {
            self.close_card();
            return;
        }
        if inside(x, y, INFO_X, WIN_Y, INFO_W, WIN_H) {
            self.card = 0;
            self.config = true;
            self.config_at = self.ms;
            self.events |= ev::BLIP;
            return;
        }
        match self.hit(x, y) {
            Some(p) if Some(p) != c => {
                self.card = self.pl[p].order;
                self.events |= ev::BLIP;
            }
            _ if c.is_some() => self.close_card(),
            _ => {}
        }
    }

    /// Escape: close the config window or the card, if one is open.
    pub fn dismiss(&mut self) -> bool {
        if self.config {
            self.config = false;
        } else if self.card != 0 {
            self.card = 0;
        } else {
            return false;
        }
        self.events |= ev::BLIP_LOW;
        true
    }

    /// What the scene wants done since the last call (`ev::*` bits).
    pub fn take_events(&mut self) -> u32 {
        std::mem::take(&mut self.events)
    }

    /// After `ev::TERMINAL`: the session whose terminal was asked for.
    pub fn terminal_id(&self) -> Option<String> {
        self.by_order(self.term_order).map(|i| self.pl[i].id.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_counts_read_short() {
        assert_eq!(fmt_tok(9999.0), "9999");
        assert_eq!(fmt_tok(12345.0), "12.3K");
        assert_eq!(fmt_tok(386_400.0), "386K");
        assert_eq!(fmt_tok(1_240_000.0), "1.24M");
        assert_eq!(fmt_tok(52_100_000.0), "52.1M");
        assert_eq!(fmt_tok(1_624_000_000.0), "1624M");
    }

    #[test]
    fn timelines_fire_once() {
        let tl = Tl { t0: -1.0, t1: 0.0 };
        assert!(tl.at(0.0));
        let tl = Tl { t0: 0.0, t1: 0.033 };
        assert!(!tl.at(0.0));
        assert!(tl.at(0.02));
        assert_eq!(tl.seg(0.5, 1.0), -1.0);
    }
}
