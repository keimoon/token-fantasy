//! Which Claude Code and Codex sessions run on this machine and what each is
//! doing: its state, the last thing it said and the tool calls it makes.
//! Processes come from /proc on Linux (procfs.rs) and from libproc and sysctl
//! on macOS (macos.rs).
//!
//! Remote sessions (a remote-control server's, from claude.ai or the phone)
//! are found but left out of the battle: they were too hard to keep track of
//! (the user, 2026-10-27).

mod claude;
mod codex;
mod follow;
#[cfg(target_os = "macos")]
#[path = "macos.rs"]
mod process;
#[cfg(not(target_os = "macos"))]
#[path = "procfs.rs"]
mod process;
mod text;

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;

use crate::session::{SaidKind, State, Tool, Why};
use crate::terminal::{self, Terminal};
use crate::usage::truthy;

pub(crate) use follow::epoch;
pub(crate) use text::{pick_tool, py_space, word};

/// A JSON value as Python's str() writes it, for procStart.
fn py_str(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Bool(b) => if *b { "True" } else { "False" }.to_owned(),
        other => other.to_string(),
    }
}

/// A Claude session file whose procStart (the start ticks, on Linux) is
/// these: the process's identity, else None (dead, or the pid was reused).
#[cfg_attr(target_os = "macos", allow(dead_code))] // macOS compares dates (macos.rs); tests use it everywhere
fn same_start(ticks: Option<String>, session: &Value) -> Option<String> {
    let ticks = ticks?;
    let other = session.get("procStart").is_some_and(|s| truthy(s) && py_str(s) != ticks);
    (!other).then_some(ticks)
}

/// One live session, as found on disk.
#[derive(Clone, Debug, PartialEq)]
pub struct Found {
    /// 8 characters: the start of a Claude session's id, the end of a Codex
    /// thread's (a Codex TUI with no thread yet: its pid in hex).
    pub id: String,
    /// For the log: the session's name or its directory's.
    pub name: String,
    pub codex: bool,
    pub state: State,
    pub why: Why,
    /// Epoch seconds the session entered this state.
    pub since: f64,
    /// Epoch seconds the session started, 0 if unknown.
    pub started: f64,
    pub cwd: String,
    pub pid: u32,
    /// The kitty window it runs in, when that kitty can be remote controlled.
    pub term: Option<Terminal>,
    pub spend_key: SpendKey,
    pub said: Option<Said>,
    /// Tool calls since the previous poll, in order.
    pub tools: Vec<Tool>,
}

/// Which counted transcripts are a session's (usage::Spend::of).
#[derive(Clone, Debug, PartialEq)]
pub enum SpendKey {
    /// The full sessionId ("" if none): its transcript and its subagents'.
    Claude(String),
    /// The rollouts of its thread: compactions, sub-agents and guardian included.
    Codex(Vec<PathBuf>),
}

/// A session's last message: something it said, the question it waits on or
/// the tool call it waits to have approved.
#[derive(Clone, Debug, PartialEq)]
pub struct Said {
    pub kind: SaidKind,
    /// Plain ASCII, at most 400 characters.
    pub text: String,
    /// Epoch seconds, when known.
    pub at: Option<f64>,
}

/// Reads the sessions; keeps what it learnt between polls (where each
/// transcript was read up to, each process's terminal).
#[derive(Default)]
pub struct Collector {
    claude: claude::Claude,
    codex: codex::Codex,
    terms: Terms,
}

impl Collector {
    pub fn new() -> Self {
        Self::default()
    }

    /// Every live session but the remote ones, first come first seated.
    pub fn poll(&mut self, now: f64) -> Vec<Found> {
        let mut all = self.claude.sessions(now, &mut self.terms);
        all.extend(self.codex.sessions(now, &mut self.terms));
        let pids: HashSet<u32> = all.iter().map(|(f, _)| f.pid).collect();
        self.terms.0.retain(|pid, _| pids.contains(pid));
        let mut out: Vec<Found> = all.into_iter().filter(|(_, remote)| !remote).map(|(f, _)| f).collect();
        out.sort_by(|a, b| a.started.total_cmp(&b.started).then_with(|| a.id.cmp(&b.id)));
        out
    }
}

/// pid -> (its start ticks, its terminal): a process's environment is read
/// once, again only if the pid was reused.
#[derive(Default)]
struct Terms(HashMap<u32, (Option<String>, Option<Terminal>)>);

impl Terms {
    fn get(&mut self, pid: u32, ticks: Option<String>) -> Option<Terminal> {
        let ticks = ticks.filter(|t| !t.is_empty()).or_else(|| process::identity(pid));
        if let Some((t, term)) = self.0.get(&pid)
            && *t == ticks
        {
            return term.clone();
        }
        let term = process::environ(pid).and_then(|env| terminal::of_environ(&env));
        self.0.insert(pid, (ticks, term.clone()));
        term
    }
}

/// The last path component ("" after a trailing '/').
fn basename(p: &str) -> &str {
    p.rsplit('/').next().unwrap_or(p)
}

fn has(hay: &[u8], needle: &[u8]) -> bool {
    memchr::memmem::find(hay, needle).is_some()
}

fn mtime(meta: &std::fs::Metadata) -> f64 {
    meta.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map_or(0.0, |d| d.as_secs_f64())
}

fn now() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64())
}

#[cfg(test)]
pub(crate) struct TestDir(pub PathBuf);

#[cfg(test)]
impl TestDir {
    pub fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let unique = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("token-fantasy-{}-{unique}", std::process::id()));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}

#[cfg(test)]
impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
