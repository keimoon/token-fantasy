//! Codex: each running `codex` TUI -> the thread it works in -> that
//! thread's rollouts (~/.codex/sessions/<y>/<m>/<d>/rollout-*.jsonl, with
//! its compactions', sub-agents' and the guardian's), followed as they grow:
//! the last task_started (work) vs task_complete/turn_aborted (idle), the
//! last thing Codex said and the tool calls since the last look. Codex logs
//! no approval waits, so it never shows "wait".
//!
//! While the remote-control daemon (`codex app-server --remote-control`)
//! runs, a TUI runs its thread inside the daemon too: the daemon has the
//! rollout open, not the TUI, and keeps it open after the TUI quits. A TUI
//! with no rollout of its own takes the daemon's thread that was started from
//! a terminal in its directory and written since the TUI started, else the
//! newest thread for its directory in ~/.codex/state_5.sqlite. What is left
//! in the daemon is the phone's (remote) or a quit TUI's (nobody).

use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::{self, BufRead, BufReader, Read};
use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags, OptionalExtension};
use serde_json::Value;

use super::follow::{Follow, follow, parse_ts, scan_back};
use super::text::{Msg, codex_kind, py_space};
use super::{Found, SpendKey, Terms, basename, has, mtime, procfs};
use crate::session::{State, Tool, Why};
use crate::usage::{codex_dir, truthy};

/// The originator of threads started from a terminal.
const TUI: &str = "codex-tui";

/// codex subcommands that are not an interactive session a human answers
const NOT_TUI: &[&str] = &[
    "exec",
    "e",
    "app-server",
    "mcp-server",
    "mcp",
    "login",
    "logout",
    "completion",
    "debug",
    "apply",
    "a",
    "cloud",
    "sandbox",
    "proto",
    "features",
    "app",
    "help",
];

struct Proc {
    pid: u32,
    cwd: String,
    /// The rollouts it has open.
    rollouts: Vec<PathBuf>,
    /// Epoch seconds, 0 if unknown.
    start: f64,
    /// The remote-control daemon.
    remote: bool,
}

/// Per rollout file: the state found so far, the session it belongs to,
/// whether it is a thread the user talks in (not, say, the guardian that
/// reviews approvals) and the last thing Codex said there.
struct Rollout {
    state: State,
    since: f64,
    session: String,
    chat: bool,
    /// The session's own threads and its sub-agents', whose tool calls count.
    works: bool,
    said: Option<Msg>,
    tools: Vec<Tool>,
    started: Option<f64>,
    cwd: Option<String>,
    originator: Option<String>,
}

#[derive(Default)]
pub struct Codex {
    rollouts: HashMap<PathBuf, Follow<Rollout>>,
}

/// Every thread some Codex process has open, by session (a thread with its
/// compaction, sub-agent and guardian rollouts), in the order first seen.
#[derive(Default)]
struct Threads(Vec<(String, Vec<PathBuf>)>);

impl Threads {
    fn add(&mut self, session: &str, path: &Path) {
        match self.0.iter_mut().find(|(s, _)| s == session) {
            Some((_, paths)) if paths.iter().any(|p| p == path) => {}
            Some((_, paths)) => paths.push(path.to_path_buf()),
            None => self.0.push((session.to_owned(), vec![path.to_path_buf()])),
        }
    }

    fn get(&self, session: &str) -> &[PathBuf] {
        self.0.iter().find(|(s, _)| s == session).map_or(&[], |(_, p)| p)
    }
}

fn processes() -> Vec<Proc> {
    let (btime, tck) = (procfs::boot_time(), procfs::clk_tck());
    let mut sessions = codex_dir().join("sessions").to_string_lossy().into_owned();
    sessions.push('/');
    let mut out = Vec::new();
    for pid in procfs::pids() {
        let Some(exe) = procfs::exe(pid) else { continue };
        if basename(&exe.to_string_lossy()) != "codex" {
            continue;
        }
        let (Some(args), Some(cwd)) = (procfs::args(pid), procfs::cwd(pid)) else { continue };
        let positional: Vec<&str> = args.iter().map(String::as_str).filter(|a| !a.starts_with('-')).collect();
        let remote = positional.first() == Some(&"app-server") && args.iter().any(|a| a == "--remote-control");
        if positional.first().is_some_and(|p| NOT_TUI.contains(p)) && !remote {
            continue;
        }
        let mut rollouts: Vec<PathBuf> = procfs::open_files(pid)
            .into_iter()
            .filter(|t| {
                let (t, b) = (t.to_string_lossy(), t.file_name().map(|b| b.to_string_lossy().into_owned()));
                let b = b.unwrap_or_default();
                (t.contains("/.codex/sessions/") || t.starts_with(&sessions))
                    && b.starts_with("rollout-")
                    && b.ends_with(".jsonl")
            })
            .collect();
        rollouts.sort();
        rollouts.dedup();
        let start = procfs::start_ticks(pid).and_then(|t| t.parse::<f64>().ok()).map_or(0.0, |t| btime + t / tck);
        out.push(Proc { pid, cwd: cwd.to_string_lossy().into_owned(), rollouts, start, remote });
    }
    out
}

/// Newest thread for this cwd that changed since the process started.
fn thread_for(cwd: &str, start: f64) -> Option<PathBuf> {
    let db = codex_dir().join("state_5.sqlite");
    let con =
        Connection::open_with_flags(&db, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX).ok()?;
    con.busy_timeout(std::time::Duration::from_secs(1)).ok()?;
    con.query_row(
        "SELECT rollout_path FROM threads WHERE cwd = ? AND archived = 0 AND updated_at_ms >= ? \
         AND coalesce(thread_source, 'user') = 'user' ORDER BY updated_at_ms DESC LIMIT 1",
        (cwd, ((start - 5.0) * 1000.0) as i64),
        |r| r.get::<_, String>(0),
    )
    .optional()
    .ok()
    .flatten()
    .map(PathBuf::from)
}

/// work|idle and when, for a task event line.
fn task_event(raw: &[u8]) -> Option<(State, Option<f64>)> {
    if !has(raw, b"\"event_msg\"")
        || !(has(raw, b"task_started") || has(raw, b"task_complete") || has(raw, b"turn_aborted"))
    {
        return None;
    }
    let o: Value = serde_json::from_slice(raw).ok()?;
    let ts = parse_ts(o.get("timestamp"));
    match o.get("payload")?.get("type")?.as_str()? {
        "task_started" => Some((State::Work, ts)),
        "task_complete" | "turn_aborted" => Some((State::Idle, ts)),
        _ => None,
    }
}

/// (text, when) for an assistant message line.
fn said_line(raw: &[u8]) -> Option<(String, Option<f64>)> {
    if !has(raw, b"\"response_item\"") || !has(raw, b"\"assistant\"") {
        return None;
    }
    let o: Value = serde_json::from_slice(raw).ok()?;
    let p = o.get("payload")?;
    let is = |k: &str, v: &str| p.get(k).and_then(Value::as_str) == Some(v);
    if !is("type", "message") || !is("role", "assistant") {
        return None;
    }
    let text: String = p
        .get("content")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|c| c.get("type").and_then(Value::as_str) == Some("output_text"))
        .map(|c| c.get("text").and_then(Value::as_str).unwrap_or(""))
        .collect();
    (!text.trim_matches(py_space).is_empty()).then(|| (text, parse_ts(o.get("timestamp"))))
}

/// The kind of a tool call line.
fn tool_line(raw: &[u8]) -> Option<Tool> {
    if !has(raw, b"\"response_item\"") || !has(raw, b"_call\"") {
        return None;
    }
    let o: Value = serde_json::from_slice(raw).ok()?;
    if o.get("type").and_then(Value::as_str) != Some("response_item") {
        return None;
    }
    codex_kind(o.get("payload").unwrap_or(&Value::Null))
}

fn first(f: &mut File, size: u64, path: &Path) -> io::Result<(Rollout, u64)> {
    let mut head = Vec::new();
    BufReader::new(&mut *f).take(1 << 20).read_until(b'\n', &mut head)?;
    let head: Value = serde_json::from_slice(&head).unwrap_or(Value::Null);
    let meta = head.get("payload").filter(|p| p.is_object()).unwrap_or(&Value::Null);
    let text = |k: &str| meta.get(k).and_then(Value::as_str).map(str::to_owned);
    // a compaction or sub-agent thread carries its parent's session_id
    let session = text("session_id").filter(|s| !s.is_empty()).unwrap_or_else(|| {
        let name: Vec<char> = basename(&path.to_string_lossy()).chars().collect();
        let stem = &name[..name.len().saturating_sub(".jsonl".len())];
        stem[stem.len().saturating_sub(36)..].iter().collect()
    });
    let (ev, done) = scan_back(f, size, task_event)?;
    let (said, _) = scan_back(f, size, said_line)?;
    let source = match meta.get("thread_source") {
        Some(v) if truthy(v) => v.as_str(),
        _ => Some("user"),
    };
    let rollout = Rollout {
        state: ev.map_or(State::Idle, |e| e.0),
        since: ev.and_then(|e| e.1).filter(|&t| t != 0.0).unwrap_or_else(|| f.metadata().map_or(0.0, |m| mtime(&m))),
        session,
        chat: source == Some("user"),
        works: matches!(source, Some("user" | "subagent")),
        said: said.map(|(text, at)| Msg::new(text, at)),
        tools: Vec::new(),
        started: parse_ts(meta.get("timestamp")),
        cwd: text("cwd"),
        originator: text("originator"),
    };
    Ok((rollout, done))
}

fn step(raw: &[u8], r: &mut Rollout) {
    if let Some((state, at)) = task_event(raw) {
        r.state = state;
        r.since = at.filter(|&t| t != 0.0).unwrap_or_else(super::now);
    }
    if let Some((text, at)) = said_line(raw) {
        r.said = Some(Msg::new(text, at));
    }
    if r.works
        && let Some(k) = tool_line(raw)
    {
        r.tools.push(k);
    }
}

/// The first item with the largest key, as Python's max() picks it.
fn first_max<T>(items: impl IntoIterator<Item = T>, key: impl Fn(&T) -> f64) -> Option<T> {
    let mut best: Option<(T, f64)> = None;
    for it in items {
        let k = key(&it);
        if best.as_ref().is_none_or(|(_, b)| k > *b) {
            best = Some((it, k));
        }
    }
    best.map(|(it, _)| it)
}

fn last_chars(s: &str, n: usize) -> String {
    let count = s.chars().count();
    s.chars().skip(count.saturating_sub(n)).collect()
}

impl Codex {
    fn info(&mut self, path: &Path) -> Option<&mut Rollout> {
        follow(path, &mut self.rollouts, |f, size| first(f, size, path), step)
    }

    fn get(&self, path: &Path) -> Option<&Rollout> {
        self.rollouts.get(path).map(|r| &r.state)
    }

    /// Every session, with whether it is a remote one (the phone's, in the daemon).
    pub fn sessions(&mut self, now: f64, terms: &mut Terms) -> Vec<(Found, bool)> {
        self.collect(now, &processes(), terms, thread_for)
    }

    fn collect(
        &mut self,
        now: f64,
        procs: &[Proc],
        terms: &mut Terms,
        thread_for: impl Fn(&str, f64) -> Option<PathBuf>,
    ) -> Vec<(Found, bool)> {
        let mut threads = Threads::default();
        for p in procs {
            for path in &p.rollouts {
                if let Some(i) = self.info(path) {
                    let session = i.session.clone();
                    threads.add(&session, path);
                }
            }
        }
        let mut out = Vec::new();
        let mut claimed: HashSet<String> = HashSet::new();
        let mut tuis: Vec<&Proc> = procs.iter().filter(|p| !p.remote).collect();
        tuis.sort_by(|a, b| b.start.total_cmp(&a.start)); // newest first
        for p in tuis {
            let mut own: Vec<String> = Vec::new();
            for path in &p.rollouts {
                if let Some(i) = self.info(path)
                    && !own.contains(&i.session)
                {
                    own.push(i.session.clone());
                }
            }
            if own.is_empty() {
                // Its thread is in the daemon: the one in its cwd, started from a
                // terminal and written since it started (newest TUI first, so two
                // in one directory do not take the same thread).
                let cands = threads.0.iter().filter(|(sid, paths)| {
                    !claimed.contains(sid)
                        && paths
                            .iter()
                            .filter_map(|x| self.get(x))
                            .any(|i| i.chat && i.originator.as_deref() == Some(TUI) && i.cwd.as_deref() == Some(&p.cwd))
                        && paths.iter().map(|x| fs::metadata(x).map_or(0.0, |m| mtime(&m))).fold(0.0, f64::max)
                            >= p.start - 5.0
                });
                let started = |(_, paths): &&(String, Vec<PathBuf>)| {
                    paths.iter().filter_map(|x| self.get(x)).map(|i| i.started.unwrap_or(0.0)).fold(0.0, f64::max)
                };
                own.extend(first_max(cands, started).map(|(sid, _)| sid.clone()));
            }
            if own.is_empty()
                && let Some(path) = thread_for(&p.cwd, p.start)
                && let Some(i) = self.info(&path)
                && !claimed.contains(&i.session)
            {
                let session = i.session.clone();
                threads.add(&session, &path);
                own.push(session);
            }
            claimed.extend(own.iter().cloned());
            let paths: Vec<PathBuf> = own.iter().flat_map(|sid| threads.get(sid)).cloned().collect();
            out.push((self.patron(p, paths, now, None, terms), false));
        }
        // What is left in the daemon: threads started from the phone are remote
        // patrons; a terminal's thread whose TUI has quit is nobody.
        if let Some(daemon) = procs.iter().find(|p| p.remote) {
            for (sid, paths) in &threads.0 {
                let chats: Vec<&Rollout> = paths.iter().filter_map(|x| self.get(x)).filter(|i| i.chat).collect();
                let Some(chat) = chats.first() else { continue };
                if claimed.contains(sid) || chat.originator.as_deref() == Some(TUI) {
                    continue;
                }
                let started = chats.iter().map(|i| i.started.filter(|&s| s != 0.0).unwrap_or(daemon.start));
                let started = started.fold(f64::INFINITY, f64::min);
                let cwd = chat.cwd.clone().filter(|c| !c.is_empty()).unwrap_or_else(|| daemon.cwd.clone());
                out.push((self.patron(daemon, paths.clone(), now, Some((started, cwd)), terms), true));
            }
        }
        let open: HashSet<&PathBuf> = threads.0.iter().flat_map(|(_, paths)| paths).collect();
        self.rollouts.retain(|path, _| open.contains(path));
        // tool calls no session took (a quit TUI's thread) are not saved up for later
        for r in self.rollouts.values_mut() {
            r.state.tools.clear();
        }
        out
    }

    /// The session for a TUI's thread, or for a remote thread in the
    /// remote-control daemon p (`remote` then gives its start and cwd).
    fn patron(
        &mut self,
        p: &Proc,
        paths: Vec<PathBuf>,
        now: f64,
        remote: Option<(f64, String)>,
        terms: &mut Terms,
    ) -> Found {
        let infos: Vec<&Rollout> = paths.iter().filter_map(|x| self.get(x)).collect();
        let (uuid, state, since, said) = if infos.is_empty() {
            // a fresh TUI that has not started a thread yet
            (format!("{:08x}", p.pid), State::Idle, if p.start != 0.0 { p.start } else { now }, None)
        } else {
            // The TUI keeps its thread and any compaction/sub-agent threads open;
            // it is working while any of them is mid-turn.
            let working: Vec<&Rollout> = infos.iter().copied().filter(|i| i.state == State::Work).collect();
            let pool = if working.is_empty() { &infos } else { &working };
            let since = pool.iter().map(|i| i.since).fold(f64::NEG_INFINITY, f64::max);
            let newest = first_max(infos.iter(), |i| i.since).unwrap();
            let said =
                first_max(infos.iter().filter(|i| i.chat).filter_map(|i| i.said.as_ref()), |m| m.at.unwrap_or(0.0));
            let state = if working.is_empty() { State::Idle } else { State::Work };
            (newest.session.replace('-', ""), state, since, said.map(Msg::said))
        };
        let mut tools = Vec::new();
        for x in &paths {
            if let Some(r) = self.rollouts.get_mut(x) {
                tools.append(&mut r.state.tools);
            }
        }
        let (started, cwd) = match &remote {
            Some((started, cwd)) => (*started, cwd.clone()),
            None => (0.0, p.cwd.clone()),
        };
        let dir = basename(&cwd);
        let name = format!("{}-{}", if dir.is_empty() { "codex" } else { dir }, last_chars(&uuid, 2));
        Found {
            id: last_chars(&uuid, 8),
            name,
            codex: true,
            state,
            why: Why::None,
            since,
            started: if started != 0.0 { started } else { p.start },
            cwd,
            pid: p.pid,
            term: if remote.is_some() { None } else { terms.get(p.pid, None) },
            spend_key: SpendKey::Codex(paths),
            said,
            tools,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collect::TestDir;
    use serde_json::json;
    use std::io::Write;

    struct Rollouts(TestDir);

    impl Rollouts {
        /// A rollout: its meta line, then the given lines.
        fn write(&self, id: &str, meta: Value, lines: &[Value]) -> PathBuf {
            let path = self.0.0.join(format!("rollout-2026-10-12T01-00-00-{id}.jsonl"));
            let mut text =
                json!({"type": "session_meta", "timestamp": "2026-10-12T01:00:00Z", "payload": meta}).to_string();
            for l in lines {
                text.push('\n');
                text.push_str(&l.to_string());
            }
            text.push('\n');
            fs::write(&path, text).unwrap();
            path
        }
    }

    fn meta(session: &str, cwd: &str, originator: &str, source: &str, started: &str) -> Value {
        json!({"id": session, "session_id": session, "cwd": cwd, "originator": originator,
            "thread_source": source, "timestamp": started})
    }

    fn event(kind: &str, ts: &str) -> Value {
        json!({"type": "event_msg", "timestamp": ts, "payload": {"type": kind}})
    }

    fn says(text: &str, ts: &str) -> Value {
        json!({"type": "response_item", "timestamp": ts, "payload": {"type": "message", "role": "assistant",
            "content": [{"type": "output_text", "text": text}]}})
    }

    fn calls(name: &str) -> Value {
        json!({"type": "response_item", "timestamp": "2026-10-12T01:05:00Z",
            "payload": {"type": "function_call", "name": name, "arguments": "{}"}})
    }

    fn proc(pid: u32, cwd: &str, rollouts: &[&PathBuf], start: f64, remote: bool) -> Proc {
        Proc { pid, cwd: cwd.into(), rollouts: rollouts.iter().map(|p| p.to_path_buf()).collect(), start, remote }
    }

    const A: &str = "11111111-1111-1111-1111-1111111111aa";
    const B: &str = "22222222-2222-2222-2222-2222222222bb";
    const C: &str = "33333333-3333-3333-3333-3333333333cc";
    const D: &str = "44444444-4444-4444-4444-4444444444dd";

    #[test]
    fn tuis_find_their_threads_in_the_daemon() {
        let dir = Rollouts(TestDir::new());
        let t0 = "2026-10-12T01:00:00Z";
        // a TUI holding its own rollout, working, plus its guardian
        let own =
            dir.write(A, meta(A, "/garden/moon", TUI, "user", t0), &[event("task_started", "2026-10-12T01:01:00Z")]);
        let guard = dir.write("guard", meta(A, "/garden/moon", TUI, "guardian_review", t0), &[calls("exec_command")]);
        // a TUI whose thread lives in the daemon
        let in_daemon = dir.write(
            B,
            meta(B, "/garden/star", TUI, "user", "2026-10-12T01:02:00Z"),
            &[
                event("task_started", "2026-10-12T01:03:00Z"),
                says("Brewing **now**.", "2026-10-12T01:03:30Z"),
                event("task_complete", "2026-10-12T01:04:00Z"),
            ],
        );
        // the phone's thread, and a quit TUI's
        let phone = dir.write(C, meta(C, "/garden/sun", "codex_mobile", "user", t0), &[event("task_started", t0)]);
        let quit = dir.write(D, meta(D, "/garden/star", TUI, "user", "2026-10-12T00:00:00Z"), &[]);
        let procs = [
            proc(100, "/garden/moon", &[&own, &guard], 50.0, false),
            proc(200, "/garden/star", &[], 60.0, false),
            proc(300, "/garden/home", &[], 70.0, false),
            proc(1, "/home", &[&own, &guard, &in_daemon, &phone, &quit], 10.0, true),
        ];
        let mut codex = Codex::default();
        let mut terms = Terms::default();
        let found = codex.collect(5000.0, &procs, &mut terms, |_, _| None);
        let line: Vec<_> = found.iter().map(|(f, r)| (f.pid, f.id.as_str(), f.name.as_str(), f.state, *r)).collect();
        assert_eq!(
            line,
            [
                (300, "0000012c", "home-2c", State::Idle, false),
                (200, "222222bb", "star-bb", State::Idle, false),
                (100, "111111aa", "moon-aa", State::Work, false),
                (1, "333333cc", "sun-cc", State::Work, true),
            ]
        );
        let star = &found[1].0;
        assert_eq!(star.said.as_ref().map(|s| s.text.as_str()), Some("Brewing now."));
        assert_eq!(star.spend_key, SpendKey::Codex(vec![in_daemon.clone()]));
        assert_eq!(star.started, 60.0);
        assert_eq!(found[3].0.started, epoch_of("2026-10-12T01:00:00Z"));
        assert_eq!(found[2].0.spend_key, SpendKey::Codex(vec![own.clone(), guard.clone()]));
        assert_eq!(found[0].0.since, 70.0);

        // tool calls as the rollouts grow: the guardian's do not count
        let append = |path: &PathBuf, v: Value| {
            writeln!(fs::OpenOptions::new().append(true).open(path).unwrap(), "{v}").unwrap()
        };
        append(&own, calls("apply_patch"));
        append(&guard, calls("exec_command"));
        append(&in_daemon, calls("spawn_agent"));
        append(&in_daemon, calls("exec_command"));
        append(&quit, calls("exec_command"));
        let found = codex.collect(5000.0, &procs, &mut terms, |_, _| None);
        let tools: Vec<_> = found.iter().map(|(f, _)| (f.pid, f.tools.clone())).collect();
        assert_eq!(tools[1], (200, vec![Tool::Agent, Tool::Bash]));
        assert_eq!(tools[2], (100, vec![Tool::Edit]));
        let found = codex.collect(5000.0, &procs, &mut terms, |_, _| None);
        assert!(found.iter().all(|(f, _)| f.tools.is_empty()));

        // the database finds the thread of a TUI the daemon does not hold
        let procs = [proc(400, "/garden/sun", &[], 80.0, false)];
        let found = codex.collect(5000.0, &procs, &mut terms, |cwd, _| (cwd == "/garden/sun").then(|| phone.clone()));
        assert_eq!(found[0].0.id, "333333cc");
        assert_eq!(codex.rollouts.len(), 1);
    }

    fn epoch_of(s: &str) -> f64 {
        super::super::follow::epoch(s).unwrap()
    }

    #[test]
    fn two_tuis_in_one_directory_take_different_threads() {
        let dir = Rollouts(TestDir::new());
        let old = dir.write(A, meta(A, "/garden", TUI, "user", "2026-10-12T01:00:00Z"), &[]);
        let new = dir.write(B, meta(B, "/garden", TUI, "user", "2026-10-12T02:00:00Z"), &[]);
        let procs = [
            proc(10, "/garden", &[], 1.0, false),
            proc(11, "/garden", &[], 2.0, false),
            proc(1, "/", &[&old, &new], 0.5, true),
        ];
        let found = Codex::default().collect(5000.0, &procs, &mut Terms::default(), |_, _| None);
        let ids: Vec<_> = found.iter().map(|(f, _)| (f.pid, f.id.as_str())).collect();
        assert_eq!(ids, [(11, "222222bb"), (10, "111111aa")]);
    }

    #[test]
    fn a_rollout_without_meta_is_named_by_its_file() {
        let dir = TestDir::new();
        let path = dir.0.join(format!("rollout-2026-10-12T01-00-00-{A}.jsonl"));
        fs::write(&path, "{\"type\":\n").unwrap();
        let mut codex = Codex::default();
        let r = codex.info(&path).unwrap();
        assert_eq!((r.session.as_str(), r.chat, r.works, r.state), (A, true, true, State::Idle));
    }
}
