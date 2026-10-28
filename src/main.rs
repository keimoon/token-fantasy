//! Token Fantasy in a desktop window. A thread reads this machine's Claude
//! Code and Codex sessions every second (states, tool calls, last messages,
//! projects), another re-counts this month's tokens every 5 s (the damage,
//! GOLD and EXP), a third reads the plans' quota (the MP bars), and the window
//! plays the battle at 30 fps.
//!
//! Usage:
//!   token-fantasy              the live battle
//!   token-fantasy --demo       a made-up battle: sessions that work, idle, ask, come and go
//!   token-fantasy --print      print the sessions, tool calls, usage and quota, and exit
//!   token-fantasy --scale N    start at N times 426x266 (default 3)
//!   --mute / --unmute / --volume N / --pace wait|active   change the saved settings
//! Keys: 1-6 window size, f / F11 fullscreen, Esc close a window or leave
//! fullscreen, m mute, q quit. Click a player for its card, the info window
//! for the config window.

mod app;

use std::collections::HashMap;
use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use token_fantasy::collect::{Collector, Found};
use token_fantasy::feed::{DEMO_MANA, DEMO_USAGE, Demo, Feed, tool_calls};
use token_fantasy::quota::Quota;
use token_fantasy::session::{Mana, Session, State, Tool, Usage, Why};
use token_fantasy::settings::Settings;
use token_fantasy::terminal::Terminal;
use token_fantasy::usage::{Counter, Spend, human};

/// How often the sessions are read.
const POLL: Duration = Duration::from_secs(1);
/// How often this month's tokens and each session's are re-counted (the damage).
const USAGE_EVERY: Duration = Duration::from_secs(5);
/// How often the quota is looked at (the readings themselves are cached for minutes).
const QUOTA_EVERY: Duration = Duration::from_secs(30);

/// What the feeder threads hand the window.
pub enum Update {
    /// The full session list, the tool calls since the last one (one per
    /// session) and the terminals that can be brought up, by session id.
    Sessions {
        list: Vec<Session>,
        tools: Vec<(String, Tool)>,
        terms: HashMap<String, Terminal>,
    },
    Usage(Usage),
    Mana(Mana),
}

pub fn log(msg: &str) {
    println!("{} {msg}", chrono::Local::now().format("%H:%M:%S"));
}

fn now_epoch() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64())
}

/// "name=wait/perm"
fn describe(f: &Found) -> String {
    let state = match f.state {
        State::Work => "work",
        State::Idle => "idle",
        State::Wait => "wait",
    };
    let why = match f.why {
        Why::None => "",
        Why::Perm => "/perm",
        Why::Ask => "/ask",
    };
    format!("{}={state}{why}", f.name)
}

fn sessions_feed(tx: Sender<Update>, start: f64, spend: Arc<Mutex<Spend>>) {
    let mut collector = Collector::new();
    let mut feed = Feed::new(start);
    let mut last = None;
    loop {
        let now = now_epoch();
        let found = collector.poll(now);
        let seen: Vec<String> = found.iter().map(|f| format!("{}:{}", f.id, describe(f))).collect();
        if last.as_ref() != Some(&seen) {
            let names: Vec<String> = found.iter().map(describe).collect();
            log(&format!("sessions: {}", if names.is_empty() { "none".into() } else { names.join(", ") }));
            last = Some(seen);
        }
        let list = feed.sessions(&found, now, &spend.lock().unwrap());
        let tools = tool_calls(&found);
        let terms = found.iter().filter_map(|f| Some((f.id.clone(), f.term.clone()?))).collect();
        if tx.send(Update::Sessions { list, tools, terms }).is_err() {
            return;
        }
        thread::sleep(POLL);
    }
}

/// A changed transcript is re-read whole, which takes a moment for a long
/// session, so this runs apart from the session poll.
fn usage_feed(tx: Sender<Update>, spend: Arc<Mutex<Spend>>) {
    let mut counter = Counter::new();
    let mut last = None;
    loop {
        let (u, per_file) = counter.count();
        *spend.lock().unwrap() = per_file;
        if last.is_none() {
            log(&format!("this month: {} tokens, ${:.2}", human(u.tokens as i64), u.usd));
        }
        if last != Some(u) {
            if tx.send(Update::Usage(u)).is_err() {
                return;
            }
            last = Some(u);
        }
        thread::sleep(USAGE_EVERY);
    }
}

fn quota_feed(tx: Sender<Update>) {
    let mut quota = Quota::new();
    loop {
        quota.poll(&log);
        if tx.send(Update::Mana(quota.mana(now_epoch()))).is_err() {
            return;
        }
        thread::sleep(QUOTA_EVERY);
    }
}

fn demo_feed(tx: Sender<Update>) {
    let mut demo = Demo::new(now_epoch());
    let _ = tx.send(Update::Usage(DEMO_USAGE));
    let _ = tx.send(Update::Mana(DEMO_MANA));
    loop {
        let (list, tools) = demo.second(now_epoch());
        if tx.send(Update::Sessions { list, tools, terms: HashMap::new() }).is_err() {
            return;
        }
        thread::sleep(POLL);
    }
}

fn print_once() {
    let now = now_epoch();
    let (usage, spend) = Counter::new().count();
    let found = Collector::new().poll(now);
    let list = Feed::new(now).sessions(&found, now, &spend);
    if list.is_empty() {
        println!("no sessions");
    }
    for (s, f) in list.iter().zip(&found) {
        let kind = if s.codex { "codex" } else { "claude" };
        let said: String = s.said.chars().take(60).collect();
        let spent = if s.tokens >= 0.0 { format!("{} ${:.2}", human(s.tokens as i64), s.usd) } else { "--".into() };
        println!(
            "{:<8}  {:<6}  {:<28}  {:<36}  for {:>5}s  {:<14}  {}{}",
            s.id,
            kind,
            describe(f),
            s.proj,
            s.age_s,
            spent,
            if s.term { ">_ " } else { "" },
            said
        );
    }
    let tools: Vec<String> = tool_calls(&found).iter().map(|(id, t)| format!("{id}:{}", t.name())).collect();
    if !tools.is_empty() {
        println!("tool calls: {}", tools.join(", "));
    }
    println!("this month: {} tokens ({}), ${:.2}", human(usage.tokens as i64), usage.tokens, usage.usd);
    let mut quota = Quota::new();
    quota.poll(&log);
    let m = quota.mana(now);
    let pct = |v: i32| if v < 0 { "--".to_string() } else { format!("{v}%") };
    println!(
        "quota left: claude 5h {}, claude week {}, codex week {}",
        pct(m.claude_5h),
        pct(m.claude_week),
        pct(m.codex_week)
    );
}

fn usage_text() -> &'static str {
    "usage: token-fantasy [--demo | --print] [--scale N] [--mute | --unmute] [--volume N] [--pace wait|active]\n\
     \n  --demo          a made-up battle: sessions that work, idle, ask, come and go\
     \n  --print         print the sessions, tool calls, usage and quota, and exit\
     \n  --scale N       start at N times 426x266 (default 3)\
     \n  --mute          no sounds (saved)\
     \n  --unmute        sounds on (saved)\
     \n  --volume N      sound volume 0-100 (saved; default 60)\
     \n  --pace MODE     wait: monsters spare sessions waiting on you; active: they don't (saved)\
     \n\nkeys: 1-6 window size, f / F11 fullscreen, Esc close a window or leave fullscreen, m mute, q quit\
     \nclick a player for its card (its portrait brings up its terminal), the info window for the config"
}

fn main() {
    let (mut demo, mut print, mut scale) = (false, false, 3u32);
    let mut settings = Settings::load();
    let mut changed = false;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--demo" => demo = true,
            "--print" => print = true,
            "--scale" => match args.next().and_then(|v| v.parse().ok()).filter(|&k| (1..=16).contains(&k)) {
                Some(k) => scale = k,
                None => return eprintln!("--scale wants a whole number from 1 to 16\n\n{}", usage_text()),
            },
            "--mute" | "--unmute" => {
                settings.muted = a == "--mute";
                changed = true;
            }
            "--volume" => match args.next().and_then(|v| v.parse::<u8>().ok()).filter(|&v| v <= 100) {
                Some(v) => {
                    settings.volume = v;
                    changed = true;
                }
                None => return eprintln!("--volume wants a whole number from 0 to 100\n\n{}", usage_text()),
            },
            "--pace" => match args.next().as_deref() {
                Some(m @ ("wait" | "active")) => {
                    settings.pace_active = m == "active";
                    changed = true;
                }
                _ => return eprintln!("--pace wants wait or active\n\n{}", usage_text()),
            },
            "-h" | "--help" => return println!("{}", usage_text()),
            other => return eprintln!("unknown argument {other}\n\n{}", usage_text()),
        }
    }
    if changed {
        settings.save();
    }
    if print {
        return print_once();
    }
    let start = now_epoch();
    let (tx, rx) = channel();
    if demo {
        thread::spawn(move || demo_feed(tx));
    } else {
        let spend = Arc::new(Mutex::new(Spend::default()));
        let (tx2, tx3, spend2) = (tx.clone(), tx.clone(), spend.clone());
        thread::spawn(move || sessions_feed(tx, start, spend));
        thread::spawn(move || usage_feed(tx2, spend2));
        thread::spawn(move || quota_feed(tx3));
    }
    if let Err(e) = app::run(rx, scale, start as i64, settings) {
        eprintln!("token-fantasy: {e}");
        std::process::exit(1);
    }
}
