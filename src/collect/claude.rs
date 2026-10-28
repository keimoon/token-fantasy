//! Claude Code: ~/.claude/sessions/<pid>.json, one per interactive session
//! (status busy -> work, idle -> idle, waiting -> wait; waitingFor
//! "permission prompt" -> "!", anything else -> "?"), and its transcript
//! ~/.claude/projects/<project>/<sessionId>.jsonl, followed as it grows: the
//! last text Claude wrote on the main thread, the tool calls since that have
//! no result yet (what a waiting session waits on) and the tool calls made
//! since the last look.

use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

use super::follow::{Follow, follow, parse_ts, scan_back};
use super::text::{Msg, claude_kind, py_space, tool_said};
use super::{Found, Said, SpendKey, Terms, basename, has, process};
use crate::session::{State, Tool, Why};
use crate::usage::{claude_dir, truthy};

/// A tool call with no result yet.
struct Pending {
    id: Option<String>,
    call: Value,
    at: Option<f64>,
}

/// Per transcript: the last text Claude wrote (main thread only), the tool
/// calls since that have no result yet, in the order made, and the tool
/// kinds called since the last look.
#[derive(Default)]
struct Transcript {
    said: Option<Msg>,
    pending: Vec<Pending>,
    tools: Vec<Tool>,
}

impl Transcript {
    /// A call keeps its place when its id comes again.
    fn call(&mut self, id: Option<String>, call: Value, at: Option<f64>) {
        match self.pending.iter_mut().find(|p| p.id == id) {
            Some(p) => (p.call, p.at) = (call, at),
            None => self.pending.push(Pending { id, call, at }),
        }
    }
}

#[derive(Default)]
pub struct Claude {
    transcripts: HashMap<PathBuf, Follow<Transcript>>,
    /// sessionId -> its transcript
    transcript_of: HashMap<String, PathBuf>,
}

fn millis(d: &Value, keys: &[&str]) -> Option<f64> {
    keys.iter().find_map(|k| d.get(*k).filter(|v| truthy(v))).and_then(Value::as_f64)
}

fn id_of(v: Option<&Value>) -> Option<String> {
    v.and_then(Value::as_str).map(str::to_owned)
}

/// The text of a text block, if it says anything.
fn text_of(c: &Map<String, Value>) -> Option<&str> {
    c.get("text").and_then(Value::as_str).filter(|t| !t.trim_matches(py_space).is_empty())
}

/// A main-thread assistant or user line that may hold text, a tool call or
/// a tool result.
fn entry(raw: &[u8]) -> Option<Value> {
    if !has(raw, b"\"assistant\"") && !has(raw, b"\"tool_result\"") {
        return None;
    }
    let o: Value = serde_json::from_slice(raw).ok()?;
    let main = !o.get("isSidechain").is_some_and(truthy);
    let kind = matches!(o.get("type").and_then(Value::as_str), Some("assistant" | "user"));
    (main && kind && o.get("message")?.get("content")?.is_array()).then_some(o)
}

/// An entry's content blocks.
fn blocks(o: &Value) -> impl DoubleEndedIterator<Item = &Map<String, Value>> {
    o["message"]["content"].as_array().into_iter().flatten().filter_map(Value::as_object)
}

/// The first look: back from the end to the last text, gathering the tool
/// calls after it that have no result (lines come last first, so a call's
/// result before the call).
fn first(f: &mut File, size: u64) -> io::Result<(Transcript, u64)> {
    let mut results = HashSet::new();
    let mut pending = Vec::new();
    let (said, done) = scan_back(f, size, |raw| {
        let o = entry(raw)?;
        let at = parse_ts(o.get("timestamp"));
        let assistant = o["type"] == "assistant";
        for c in blocks(&o).rev() {
            match c.get("type").and_then(Value::as_str) {
                Some("tool_result") => {
                    results.insert(id_of(c.get("tool_use_id")));
                }
                _ if !assistant => {}
                Some("tool_use") if !results.contains(&id_of(c.get("id"))) => {
                    pending.push((id_of(c.get("id")), Value::Object(c.clone()), at));
                }
                Some("text") => {
                    if let Some(text) = text_of(c) {
                        return Some(Msg::new(text.to_owned(), at));
                    }
                }
                _ => {}
            }
        }
        None
    })?;
    let mut t = Transcript { said, ..Transcript::default() };
    for (id, call, at) in pending.into_iter().rev() {
        t.call(id, call, at);
    }
    Ok((t, done))
}

fn step(raw: &[u8], t: &mut Transcript) {
    let Some(o) = entry(raw) else { return };
    let at = parse_ts(o.get("timestamp"));
    let assistant = o["type"] == "assistant";
    for c in blocks(&o) {
        match c.get("type").and_then(Value::as_str) {
            Some("tool_result") => {
                let id = id_of(c.get("tool_use_id"));
                t.pending.retain(|p| p.id != id);
            }
            _ if !assistant => {}
            Some("tool_use") => {
                t.call(id_of(c.get("id")), Value::Object(c.clone()), at);
                t.tools.extend(claude_kind(c.get("name").and_then(Value::as_str).unwrap_or("")));
            }
            Some("text") => {
                if let Some(text) = text_of(c) {
                    t.said = Some(Msg::new(text.to_owned(), at));
                    t.pending.clear(); // Claude speaks again only once its tools have answered
                }
            }
            _ => {}
        }
    }
}

impl Claude {
    /// Every live session, with whether it is a remote-control server's.
    pub fn sessions(&mut self, now: f64, terms: &mut Terms) -> Vec<(Found, bool)> {
        let dir = claude_dir();
        self.sessions_in(&dir.join("sessions"), &dir.join("projects"), now, terms, process::alive)
    }

    /// alive: the identity of the live process that wrote a session file, else None.
    fn sessions_in(
        &mut self,
        sessions: &Path,
        projects: &Path,
        now: f64,
        terms: &mut Terms,
        alive: impl Fn(u32, &Value) -> Option<String>,
    ) -> Vec<(Found, bool)> {
        let mut out = Vec::new();
        let mut used = HashSet::new();
        for e in fs::read_dir(sessions).into_iter().flatten().flatten() {
            let name = e.file_name();
            let name = name.to_string_lossy();
            if name.starts_with('.') || !name.ends_with(".json") {
                continue;
            }
            let Ok(raw) = fs::read(e.path()) else { continue };
            let Ok(d) = serde_json::from_slice::<Value>(&raw) else { continue };
            match d.get("kind") {
                None => {}
                Some(Value::String(k)) if k == "interactive" => {}
                _ => continue,
            }
            let Some(pid) = d.get("pid").filter(|p| truthy(p)).and_then(Value::as_u64) else { continue };
            let Ok(pid) = u32::try_from(pid) else { continue };
            let Some(ticks) = alive(pid, &d) else { continue }; // dead, or the pid was reused
            let state = match d.get("status").and_then(Value::as_str) {
                Some("busy") => State::Work,
                Some("waiting") => State::Wait,
                _ => State::Idle,
            };
            let why = match d.get("waitingFor").filter(|w| truthy(w)) {
                _ if state != State::Wait => Why::None,
                None => Why::Perm,
                Some(Value::String(w)) if w == "permission prompt" => Why::Perm,
                Some(_) => Why::Ask,
            };
            let since = millis(&d, &["statusUpdatedAt", "updatedAt", "startedAt"]).unwrap_or(now * 1000.0) / 1000.0;
            let full = d.get("sessionId").and_then(Value::as_str).unwrap_or("").to_owned();
            let sid = if full.is_empty() { pid.to_string() } else { full.replace('-', "") };
            let id: String = sid.chars().take(8).collect();
            let cwd = d.get("cwd").and_then(Value::as_str).unwrap_or("").to_owned();
            let name = [d.get("name").and_then(Value::as_str).unwrap_or(""), basename(&cwd)]
                .into_iter()
                .find(|s| !s.is_empty())
                .map_or_else(|| id.clone(), str::to_owned);
            let transcript = if full.is_empty() { None } else { self.transcript(projects, &full) };
            let remote = d.get("entrypoint").and_then(Value::as_str) == Some("sdk-cli");
            let size = transcript.as_ref().and_then(|p| fs::metadata(p).ok()).map_or(0, |m| m.len());
            if remote && size == 0 {
                continue; // a remote-control server's standby: nobody has prompted it yet
            }
            used.insert(transcript.clone());
            let said = self.said(transcript.as_deref(), state == State::Wait, &cwd);
            let tools = transcript.as_ref().and_then(|p| self.transcripts.get_mut(p));
            let found = Found {
                id,
                name,
                codex: false,
                state,
                why,
                since,
                started: millis(&d, &["startedAt"]).unwrap_or(0.0) / 1000.0,
                cwd,
                pid,
                term: terms.get(pid, Some(ticks)),
                spend_key: SpendKey::Claude(full),
                said,
                tools: tools.map(|r| std::mem::take(&mut r.state.tools)).unwrap_or_default(),
            };
            out.push((found, remote));
        }
        self.transcripts.retain(|p, _| used.contains(&Some(p.clone())));
        self.transcript_of.retain(|_, p| used.contains(&Some(p.clone())));
        out
    }

    /// <projects>/<project>/<sessionId>.jsonl, the newest if several; None
    /// before the first prompt.
    fn transcript(&mut self, projects: &Path, sid: &str) -> Option<PathBuf> {
        if let Some(p) = self.transcript_of.get(sid)
            && p.exists()
        {
            return Some(p.clone());
        }
        let file = format!("{sid}.jsonl");
        let mut best: Option<(PathBuf, f64)> = None;
        for e in fs::read_dir(projects).into_iter().flatten().flatten() {
            if e.file_name().to_string_lossy().starts_with('.') {
                continue;
            }
            let path = e.path().join(&file);
            let Ok(meta) = fs::metadata(&path) else { continue };
            let t = super::mtime(&meta);
            if best.as_ref().is_none_or(|(_, b)| t > *b) {
                best = Some((path, t));
            }
        }
        match best {
            Some((path, _)) => {
                self.transcript_of.insert(sid.to_owned(), path.clone());
                Some(path)
            }
            None => {
                self.transcript_of.remove(sid);
                None
            }
        }
    }

    /// What a session's card says: the call it waits on, else the last thing it said.
    fn said(&mut self, path: Option<&Path>, waiting: bool, cwd: &str) -> Option<Said> {
        let r = follow(path?, &mut self.transcripts, first, step)?;
        if waiting && let Some(p) = r.pending.first() {
            let (kind, text) = tool_said(&p.call, cwd);
            return Some(Said { kind, text, at: p.at });
        }
        r.said.as_ref().map(Msg::said)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collect::TestDir;
    use crate::session::SaidKind;
    use serde_json::json;
    use std::io::Write;

    fn line(kind: &str, ts: &str, content: Value) -> String {
        json!({"type": kind, "timestamp": ts, "message": {"role": kind, "content": content}}).to_string() + "\n"
    }

    fn text(t: &str) -> Value {
        json!({"type": "text", "text": t})
    }

    fn call(id: &str, name: &str, input: Value) -> Value {
        json!({"type": "tool_use", "id": id, "name": name, "input": input})
    }

    fn result(id: &str) -> Value {
        json!({"type": "tool_result", "tool_use_id": id, "content": "ok"})
    }

    #[test]
    fn follows_a_transcript() {
        let dir = TestDir::new();
        let path = dir.0.join("s.jsonl");
        let mut text_lines = line("user", "2026-10-12T01:00:00Z", json!([text("Bake bread")]));
        text_lines += &line(
            "assistant",
            "2026-10-12T01:00:01Z",
            json!([text("I'll **check** the oven."), call("a", "Read", json!({"file_path": "/garden/oven.md"}))]),
        );
        text_lines += &line("user", "2026-10-12T01:00:02Z", json!([result("a")]));
        text_lines +=
            &line("assistant", "2026-10-12T01:00:03Z", json!([call("b", "Bash", json!({"command": "make bread"}))]));
        fs::write(&path, &text_lines).unwrap();
        let mut c = Claude::default();
        // the first look: the last text, and the call still open
        let said = c.said(Some(&path), true, "/garden").unwrap();
        assert_eq!((said.kind, said.text.as_str()), (SaidKind::Tool, "Bash: make bread"));
        assert_eq!(said.at, epoch_of("2026-10-12T01:00:03Z"));
        let said = c.said(Some(&path), false, "/garden").unwrap();
        assert_eq!((said.kind, said.text.as_str()), (SaidKind::Msg, "I'll check the oven."));
        assert!(c.transcripts[&path].state.tools.is_empty()); // none called since the first look
        // what comes after
        let mut f = fs::OpenOptions::new().append(true).open(&path).unwrap();
        f.write_all(line("user", "2026-10-12T01:00:04Z", json!([result("b")])).as_bytes()).unwrap();
        let sub =
            json!({"type": "assistant", "isSidechain": true, "message": {"content": [call("x", "Grep", json!({}))]}});
        writeln!(f, "{sub}").unwrap();
        let q =
            json!({"questions": [{"question": "Rye or wheat?", "options": [{"label": "Rye"}, {"label": "Wheat"}]}]});
        f.write_all(
            line(
                "assistant",
                "2026-10-12T01:00:05Z",
                json!([call("c", "Agent", json!({})), call("d", "AskUserQuestion", q)]),
            )
            .as_bytes(),
        )
        .unwrap();
        f.write_all(line("user", "2026-10-12T01:00:06Z", json!([result("c")])).as_bytes()).unwrap();
        let said = c.said(Some(&path), true, "/garden").unwrap();
        assert_eq!((said.kind, said.text.as_str()), (SaidKind::Ask, "Rye or wheat? Rye / Wheat"));
        assert_eq!(std::mem::take(&mut c.transcripts.get_mut(&path).unwrap().state.tools), [Tool::Agent]);
        f.write_all(line("assistant", "2026-10-12T01:00:07Z", json!([text("Rye it is.")])).as_bytes()).unwrap();
        let said = c.said(Some(&path), true, "/garden").unwrap();
        assert_eq!((said.kind, said.text.as_str()), (SaidKind::Msg, "Rye it is."));
    }

    fn epoch_of(s: &str) -> Option<f64> {
        super::super::follow::epoch(s)
    }

    #[test]
    fn reads_live_sessions() {
        let dir = TestDir::new();
        let (sessions, projects) = (dir.0.join("sessions"), dir.0.join("projects"));
        fs::create_dir_all(projects.join("-garden")).unwrap();
        fs::create_dir_all(&sessions).unwrap();
        let sid = "0a1b2c3d-4e5f-6789-abcd-ef0123456789";
        fs::write(
            projects.join("-garden").join(format!("{sid}.jsonl")),
            line("assistant", "2026-10-12T01:00:00Z", json!([text("Hi")])),
        )
        .unwrap();
        let write = |pid: u32, d: Value| fs::write(sessions.join(format!("{pid}.json")), d.to_string()).unwrap();
        write(
            10,
            json!({"pid": 10, "procStart": "500", "sessionId": sid, "cwd": "/garden/moon", "status": "waiting",
            "waitingFor": "a question", "statusUpdatedAt": 1_000_000, "startedAt": 900_000}),
        );
        write(11, json!({"pid": 11, "procStart": "999", "sessionId": "dead", "status": "busy"})); // reused pid
        write(12, json!({"pid": 12, "kind": "bg", "status": "busy"}));
        write(13, json!({"pid": 13, "status": "busy", "entrypoint": "sdk-cli", "sessionId": "standby"}));
        write(14, json!({"pid": 14, "status": "busy"}));
        let mut c = Claude::default();
        let mut terms = Terms::default();
        let alive = |pid, d: &Value| super::super::same_start((pid != 99).then(|| "500".into()), d);
        let found = c.sessions_in(&sessions, &projects, 2000.0, &mut terms, alive);
        let mut found: Vec<Found> = found.into_iter().map(|(f, remote)| (assert!(!remote), f).1).collect();
        found.sort_by_key(|f| f.pid);
        assert_eq!(found.len(), 2);
        let s = &found[0];
        assert_eq!((s.id.as_str(), s.name.as_str(), s.state, s.why), ("0a1b2c3d", "moon", State::Wait, Why::Ask));
        assert_eq!((s.since, s.started), (1000.0, 900.0));
        assert_eq!(s.spend_key, SpendKey::Claude(sid.into()));
        assert_eq!(s.said.as_ref().map(|s| s.text.as_str()), Some("Hi"));
        let s = &found[1];
        assert_eq!(
            (s.id.as_str(), s.name.as_str(), s.state, s.since, s.started),
            ("14", "14", State::Work, 2000.0, 0.0)
        );
        assert_eq!(c.transcripts.len(), 1);
    }
}
