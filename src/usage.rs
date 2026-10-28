//! This calendar month's tokens and list-price dollars (GOLD and EXP), Claude
//! Code and Codex summed, and each transcript's own total (a session's spend,
//! and from it the damage it deals).
//!
//! Both CLIs keep a full transcript of every session on disk, and every
//! assistant turn carries the token usage the API reported for it:
//!
//!   Claude   ~/.claude/projects/<slug>/<session>.jsonl   ($CLAUDE_CONFIG_DIR)
//!            assistant records -> message.usage, message.model
//!   Codex    ~/.codex/sessions/<y>/<m>/<d>/rollout-*.jsonl   ($CODEX_HOME)
//!            token_usage_record events (new), token_count events (older)
//!
//! The dollars are what the same traffic would have cost on each vendor's
//! API at list price: a measure of what was consumed, not of what was billed.
//! Per-file results are cached by size and mtime, so a re-count only re-reads
//! the transcripts that changed.

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use chrono::{DateTime, Datelike, Local, NaiveDate, NaiveDateTime, TimeZone};
use serde_json::{Value, json};

use crate::collect::SpendKey;
use crate::session::Usage;

const M: f64 = 1_000_000.0;
const CACHE_VERSION: u64 = 1;

/* ---- price list, US dollars per million tokens ---------------------------
 * Claude: cache writes are 1.25x input for the 5 minute TTL and 2x for the
 * 1 hour TTL; cache reads are 0.1x input, except Fable 5.1 at 0.025x.
 * Codex: cached input and cache writes are quoted directly by OpenAI. */

/// model prefix -> (input, output, cache read)
const CLAUDE_PRICES: &[(&str, [f64; 3])] = &[
    ("claude-fable-5-1", [10.00, 50.00, 0.25]),
    ("claude-mythos-5-1", [10.00, 50.00, 0.25]),
    ("claude-fable-5", [10.00, 50.00, 1.00]),
    ("claude-opus-5-5", [4.00, 20.00, 0.20]),
    ("claude-opus-5", [5.00, 25.00, 0.50]),
    ("claude-opus-4-8", [5.00, 25.00, 0.50]),
    ("claude-opus-4-7", [5.00, 25.00, 0.50]),
    ("claude-opus-4-6", [5.00, 25.00, 0.50]),
    ("claude-sonnet-5-5", [2.00, 10.00, 0.20]),
    ("claude-sonnet-5", [2.00, 10.00, 0.20]),
    ("claude-sonnet-4-6", [3.00, 15.00, 0.30]),
    ("claude-haiku-4-5", [1.00, 5.00, 0.10]),
];

/// model -> (input, output, cached input, cache write)
const CODEX_PRICES: &[(&str, [f64; 4])] = &[
    ("gpt-6-astra", [10.00, 50.00, 1.00, 12.50]),
    ("gpt-5.6-cyber", [12.50, 75.00, 1.25, 15.625]),
    ("gpt-5.6-sol", [4.00, 20.00, 0.40, 5.00]),
    ("gpt-5.6-terra", [2.00, 12.00, 0.20, 2.50]),
    ("gpt-5.6-luna", [0.20, 1.20, 0.02, 0.25]),
    ("gpt-5.5-pro", [30.00, 180.00, 30.00, 37.50]),
    ("gpt-5.5", [5.00, 30.00, 0.50, 6.25]),
    ("gpt-5.4-mini", [0.75, 4.50, 0.075, 0.9375]),
    ("gpt-5.4-nano", [0.20, 1.25, 0.02, 0.25]),
    ("gpt-5.4", [2.50, 15.00, 0.25, 3.125]),
];

/// Longest prefix first: "claude-opus-5" also prefixes "claude-opus-5-5".
fn price<const N: usize>(table: &[(&str, [f64; N])], model: &str) -> Option<[f64; N]> {
    table.iter().filter(|(p, _)| model.starts_with(p)).max_by_key(|(p, _)| p.len()).map(|&(_, v)| v)
}

/// Tokens and dollars.
#[derive(Clone, Copy, Default, Debug, PartialEq)]
struct Tally {
    tokens: i64,
    cost: f64,
}

impl Tally {
    fn add(&mut self, o: Tally) {
        self.tokens += o.tokens;
        self.cost += o.cost;
    }
}

/// Local date (YYYY-MM-DD) -> totals.
type Days = BTreeMap<String, Tally>;

fn num(v: Option<&Value>) -> i64 {
    match v {
        Some(Value::Number(n)) => n.as_i64().unwrap_or_else(|| n.as_f64().unwrap_or(0.0) as i64),
        _ => 0,
    }
}

/// An ISO-8601 timestamp or epoch seconds -> local date.
fn local_day(ts: Option<&Value>) -> Option<String> {
    let day = match ts? {
        Value::Number(n) => Local.timestamp_opt(n.as_f64()? as i64, 0).single()?.date_naive(),
        Value::String(s) => match DateTime::parse_from_rfc3339(s) {
            Ok(dt) => dt.with_timezone(&Local).date_naive(),
            Err(_) => NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S%.f").ok()?.date(),
        },
        _ => return None,
    };
    Some(day.format("%Y-%m-%d").to_string())
}

fn lines(path: &Path) -> std::io::Result<impl Iterator<Item = Vec<u8>>> {
    let f = BufReader::new(fs::File::open(path)?);
    Ok(f.split(b'\n').map_while(Result::ok))
}

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    memchr::memmem::find(hay, needle).is_some()
}

/// One API response is written as several lines, one per content block, and
/// each line repeats that response's usage, partially filled in on the early
/// lines while the response is still streaming. So collapse by (message id,
/// request id) and keep the line with the most output tokens, which is the
/// final one. De-duplication is per file.
fn parse_claude(path: &Path) -> std::io::Result<Days> {
    type Key = (Option<String>, Option<String>); // (message id, request id)
    let mut best: HashMap<Key, (Value, String, Value)> = HashMap::new();
    let mut order = Vec::new();
    for line in lines(path)? {
        if !contains(&line, b"\"usage\"") || !contains(&line, b"\"assistant\"") {
            continue;
        }
        let Ok(d) = serde_json::from_slice::<Value>(&line) else { continue };
        if d.get("type").and_then(Value::as_str) != Some("assistant") {
            continue;
        }
        let msg = d.get("message");
        let usage = msg.and_then(|m| m.get("usage"));
        let Some(usage) = usage.filter(|u| u.as_object().is_some_and(|o| !o.is_empty())) else { continue };
        let Some(ts) = d.get("timestamp").filter(|t| truthy(t)) else { continue };
        let model = msg.and_then(|m| m.get("model")).and_then(Value::as_str).unwrap_or("");
        if model.is_empty() || model.starts_with('<') {
            continue; // <synthetic>: no API call happened
        }
        let id = |v: Option<&Value>| v.and_then(Value::as_str).map(str::to_owned);
        let key = (id(msg.and_then(|m| m.get("id"))), id(d.get("requestId")));
        let out = num(usage.get("output_tokens"));
        match best.get(&key) {
            Some(prev) if out <= num(prev.0.get("output_tokens")) => {}
            prev => {
                if prev.is_none() {
                    order.push(key.clone());
                }
                best.insert(key, (usage.clone(), model.to_owned(), ts.clone()));
            }
        }
    }
    let mut days = Days::new();
    for key in order {
        let (usage, model, ts) = &best[&key];
        let inp = num(usage.get("input_tokens"));
        let read = num(usage.get("cache_read_input_tokens"));
        let out = num(usage.get("output_tokens"));
        let creation = usage.get("cache_creation");
        let mut w5 = num(creation.and_then(|c| c.get("ephemeral_5m_input_tokens")));
        let w1h = num(creation.and_then(|c| c.get("ephemeral_1h_input_tokens")));
        if w5 == 0 && w1h == 0 {
            w5 = num(usage.get("cache_creation_input_tokens"));
        }
        let mut t = Tally { tokens: inp + w5 + w1h + read + out, cost: 0.0 };
        if let Some([pin, pout, pread]) = price(CLAUDE_PRICES, model) {
            t.cost = (inp as f64 * pin
                + w5 as f64 * pin * 1.25
                + w1h as f64 * pin * 2.0
                + read as f64 * pread
                + out as f64 * pout)
                / M;
        }
        let Some(day) = local_day(Some(ts)) else { continue };
        days.entry(day).or_default().add(t);
    }
    Ok(days)
}

fn charge(sink: &mut Days, day: Option<String>, model: &str, inp: i64, cached: i64, written: i64, out: i64) {
    let Some(day) = day else { return };
    let uncached = (inp - cached - written).max(0);
    let mut t = Tally { tokens: uncached + cached + written + out, cost: 0.0 };
    if let Some([pin, pout, pcached, pwrite]) = price(CODEX_PRICES, model) {
        t.cost = (uncached as f64 * pin + cached as f64 * pcached + written as f64 * pwrite + out as f64 * pout) / M;
    }
    sink.entry(day).or_default().add(t);
}

/// Newer Codex writes one `token_usage_record` per API response. Older
/// versions wrote `token_count` events carrying a running total for the
/// session, so those are differenced. A current rollout contains *both* kinds
/// for the same responses, so the two are accumulated separately and the
/// modern one wins; adding them double-counts every token.
fn parse_codex(path: &Path) -> std::io::Result<Days> {
    let (mut modern, mut legacy) = (Days::new(), Days::new());
    let mut model = "unknown".to_string();
    let mut running: Option<[i64; 4]> = None;
    let mut seen = std::collections::HashSet::new();
    for line in lines(path)? {
        if !contains(&line, b"\"model\"") && !contains(&line, b"token") {
            continue;
        }
        let Ok(d) = serde_json::from_slice::<Value>(&line) else { continue };
        let kind = d.get("type").and_then(Value::as_str).unwrap_or("");
        let p = d.get("payload").unwrap_or(&Value::Null);
        if kind == "turn_context" || kind == "session_meta" {
            let m = p.get("model").filter(|m| truthy(m)).or_else(|| p.get("payload").and_then(|q| q.get("model")));
            if let Some(m) = m.and_then(Value::as_str).filter(|m| !m.is_empty()) {
                model = m.to_string();
            }
            continue;
        }
        let ts = d.get("timestamp");
        if kind == "token_usage_record" {
            let u = p.get("usage").unwrap_or(&Value::Null);
            if let Some(rid) = p.get("response_id").and_then(Value::as_str).filter(|r| !r.is_empty())
                && !seen.insert(rid.to_string())
            {
                continue;
            }
            let f = |k| num(u.get(k));
            charge(
                &mut modern,
                local_day(ts),
                &model,
                f("input_tokens"),
                f("cached_input_tokens"),
                f("cache_write_input_tokens"),
                f("output_tokens"),
            );
        } else if kind == "event_msg" && p.get("type").and_then(Value::as_str) == Some("token_count") {
            let Some(info) = p.get("info").filter(|i| truthy(i)) else { continue };
            let tot = info.get("total_token_usage").unwrap_or(&Value::Null);
            let cur = ["input_tokens", "cached_input_tokens", "cache_write_input_tokens", "output_tokens"]
                .map(|k| num(tot.get(k)));
            let delta = match running {
                Some(r) if cur[0] >= r[0] => [cur[0] - r[0], cur[1] - r[1], cur[2] - r[2], cur[3] - r[3]],
                _ => cur, // first record, or the session restarted
            };
            running = Some(cur);
            if delta.iter().any(|&v| v != 0) {
                charge(&mut legacy, local_day(ts), &model, delta[0], delta[1], delta[2], delta[3]);
            }
        }
    }
    Ok(if modern.is_empty() { legacy } else { modern })
}

/// Python's truth: null, false, 0, "", [] and {} are false.
pub(crate) fn truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64() != Some(0.0),
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
    }
}

/* ---- collection ---------------------------------------------------------- */

pub(crate) fn claude_dir() -> PathBuf {
    std::env::var_os("CLAUDE_CONFIG_DIR").map(PathBuf::from).unwrap_or_else(|| home().join(".claude"))
}

pub(crate) fn codex_dir() -> PathBuf {
    std::env::var_os("CODEX_HOME").map(PathBuf::from).unwrap_or_else(|| home().join(".codex"))
}

pub(crate) fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/"))
}

/// $XDG_CACHE_HOME/token-fantasy-desktop, else ~/.cache/token-fantasy-desktop.
pub(crate) fn cache_dir() -> PathBuf {
    let base = std::env::var_os("XDG_CACHE_HOME").filter(|d| !d.is_empty()).map(PathBuf::from);
    base.unwrap_or_else(|| home().join(".cache")).join("token-fantasy-desktop")
}

/// Every *.jsonl under dir, like glob's `**/*.jsonl`: hidden names skipped.
fn jsonl_under(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let name = e.file_name();
        let name = name.to_string_lossy();
        if name.starts_with('.') {
            continue;
        }
        let path = e.path();
        if path.is_dir() {
            jsonl_under(&path, out);
        } else if name.ends_with(".jsonl") {
            out.push(path);
        }
    }
}

struct Entry {
    stamp: String,
    days: Days,
}

/// Re-counts the month on demand, re-reading only transcripts that changed.
pub struct Counter {
    cache: HashMap<PathBuf, Entry>,
    cache_path: PathBuf,
}

/// Each counted transcript's own total, all its days, this month's and any
/// before: what a session has spent.
#[derive(Clone, Debug, Default)]
pub struct Spend {
    files: HashMap<PathBuf, Tally>,
    /// Claude's transcripts by path as text, for the match by session id.
    claude: Vec<(String, Tally)>,
}

impl Spend {
    /// (tokens, usd) of one session's transcripts, or None if none was counted.
    pub fn of(&self, key: &SpendKey) -> Option<(f64, f64)> {
        let mut hits = Vec::new();
        match key {
            SpendKey::Claude(id) if id.is_empty() => return None,
            SpendKey::Claude(id) => {
                // its transcript, and its subagents' under <project>/<sessionId>/
                let (file, dir) = (format!("/{id}.jsonl"), format!("/{id}/"));
                hits.extend(
                    self.claude.iter().filter(|(p, _)| p.ends_with(&file) || p.contains(&dir)).map(|(_, t)| *t),
                );
            }
            SpendKey::Codex(paths) => hits.extend(paths.iter().filter_map(|p| self.files.get(p))),
        }
        if hits.is_empty() {
            return None;
        }
        let mut sum = Tally::default();
        hits.into_iter().for_each(|t| sum.add(t));
        Some((sum.tokens as f64, sum.cost))
    }
}

impl Default for Counter {
    fn default() -> Self {
        Self::new()
    }
}

/// Dollars to the cent (Python's round(x, 2)).
pub(crate) fn cents(usd: f64) -> f64 {
    (usd * 100.0).round_ties_even() / 100.0
}

impl Counter {
    pub fn new() -> Self {
        let cache_path = cache_dir().join("usage.json");
        let cache = Self::load(&cache_path).unwrap_or_default();
        Counter { cache, cache_path }
    }

    fn load(path: &Path) -> Option<HashMap<PathBuf, Entry>> {
        let v: Value = serde_json::from_slice(&fs::read(path).ok()?).ok()?;
        if v.get("version")?.as_u64()? != CACHE_VERSION {
            return None;
        }
        let mut out = HashMap::new();
        for (path, e) in v.get("files")?.as_object()? {
            let mut days = Days::new();
            for (day, t) in e.get("days")?.as_object()? {
                days.insert(day.clone(), Tally { tokens: t.get(0)?.as_i64()?, cost: t.get(1)?.as_f64()? });
            }
            out.insert(PathBuf::from(path), Entry { stamp: e.get("stamp")?.as_str()?.to_string(), days });
        }
        Some(out)
    }

    fn save(&self) {
        let files: serde_json::Map<String, Value> = self
            .cache
            .iter()
            .map(|(p, e)| {
                let days: serde_json::Map<String, Value> =
                    e.days.iter().map(|(d, t)| (d.clone(), json!([t.tokens, t.cost]))).collect();
                (p.to_string_lossy().into_owned(), json!({ "stamp": e.stamp, "days": days }))
            })
            .collect();
        let doc = json!({ "version": CACHE_VERSION, "files": files });
        let tmp = self.cache_path.with_extension("json.tmp");
        let ok = self.cache_path.parent().is_some_and(|d| fs::create_dir_all(d).is_ok())
            && fs::write(&tmp, doc.to_string()).is_ok();
        if ok {
            let _ = fs::rename(&tmp, &self.cache_path);
        }
    }

    /// Totals per local day over every transcript written since `since`
    /// (epoch seconds): an older file holds nothing newer than its mtime, and
    /// is dropped from the cache. Returns the days and how many files were
    /// re-read.
    fn collect(&mut self, claude: &Path, codex: &Path, since: f64) -> (Days, usize) {
        let (mut claude_files, mut codex_files) = (Vec::new(), Vec::new());
        jsonl_under(claude, &mut claude_files);
        jsonl_under(codex, &mut codex_files);
        let mut fresh = HashMap::new();
        let mut totals = Days::new();
        let mut reparsed = 0;
        type Parse = fn(&Path) -> std::io::Result<Days>;
        for (files, parse) in [(claude_files, parse_claude as Parse), (codex_files, parse_codex as Parse)] {
            for path in files {
                let Ok(st) = fs::metadata(&path) else { continue };
                let mtime = st.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok());
                let Some(mtime) = mtime else { continue };
                if mtime.as_secs_f64() < since {
                    continue;
                }
                let stamp = format!("{}:{}", st.len(), mtime.as_nanos());
                let entry = match self.cache.remove(&path) {
                    Some(e) if e.stamp == stamp => e,
                    _ => {
                        reparsed += 1;
                        // a transcript that cannot be read counts as empty until it changes
                        Entry { stamp, days: parse(&path).unwrap_or_default() }
                    }
                };
                for (d, t) in &entry.days {
                    totals.entry(d.clone()).or_default().add(*t);
                }
                fresh.insert(path, entry);
            }
        }
        let dropped = !self.cache.is_empty();
        self.cache = fresh;
        if reparsed > 0 || dropped {
            self.save();
        }
        (totals, reparsed)
    }

    /// Each transcript's total, for the sessions' cards.
    fn spend(&self, claude: &Path) -> Spend {
        let prefix = claude.to_string_lossy();
        let mut spend = Spend::default();
        for (path, e) in &self.cache {
            if e.days.is_empty() {
                continue; // no usage in it: not counted
            }
            let mut sum = Tally::default();
            e.days.values().for_each(|t| sum.add(*t));
            let text = path.to_string_lossy();
            if text.starts_with(&*prefix) {
                spend.claude.push((text.into_owned(), sum));
            }
            spend.files.insert(path.clone(), sum);
        }
        spend
    }

    /// This calendar month (local time), Claude and Codex summed, and each
    /// transcript's own total. Cheap when nothing changed: a stat per
    /// transcript of the month.
    pub fn count(&mut self) -> (Usage, Spend) {
        let today = Local::now().date_naive();
        let first = NaiveDate::from_ymd_opt(today.year(), today.month(), 1).unwrap();
        let since = Local
            .from_local_datetime(&first.and_hms_opt(0, 0, 0).unwrap())
            .earliest()
            .map_or(0.0, |t| t.timestamp() as f64);
        let claude = claude_dir().join("projects");
        let (days, _) = self.collect(&claude, &codex_dir().join("sessions"), since);
        let (lo, hi) = (first.format("%Y-%m-%d").to_string(), today.format("%Y-%m-%d").to_string());
        let mut sum = Tally::default();
        for (_, t) in days.range(lo..=hi) {
            sum.add(*t);
        }
        (Usage { tokens: sum.tokens as f64, usd: cents(sum.cost) }, self.spend(&claude))
    }
}

/// 1.6 B / 53.0 M / 999.0 K, for the log.
pub fn human(n: i64) -> String {
    for (unit, div) in [("B", 1_000_000_000), ("M", 1_000_000), ("K", 1_000)] {
        if n >= div {
            return format!("{:.1} {unit}", n as f64 / div as f64);
        }
    }
    n.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collect::TestDir;

    fn claude_line(id: &str, out: i64, ts: &str) -> String {
        json!({"type": "assistant", "timestamp": ts, "requestId": "r", "message": {"id": id, "model": "claude-opus-5-5",
            "usage": {"input_tokens": 1000, "output_tokens": out, "cache_read_input_tokens": 10000,
                "cache_creation": {"ephemeral_5m_input_tokens": 0, "ephemeral_1h_input_tokens": 2000}}}})
        .to_string()
            + "\n"
    }

    #[test]
    fn counts_transcripts_and_each_sessions_spend() {
        let dir = TestDir::new();
        let (claude, codex) = (dir.0.join("projects"), dir.0.join("sessions"));
        let sid = "0a1b2c3d-0000-0000-0000-000000000001";
        fs::create_dir_all(claude.join("-garden").join(sid).join("subagents")).unwrap();
        fs::create_dir_all(codex.join("2026/10/12")).unwrap();
        // the streamed lines of one response count once, at their last
        let main = claude.join("-garden").join(format!("{sid}.jsonl"));
        fs::write(
            &main,
            claude_line("m1", 10, "2026-10-12T01:00:00Z") + &claude_line("m1", 500, "2026-10-12T01:00:01Z"),
        )
        .unwrap();
        let sub = claude.join("-garden").join(sid).join("subagents").join("agent-1.jsonl");
        fs::write(&sub, claude_line("m2", 100, "2026-10-12T01:00:00Z")).unwrap();
        fs::write(claude.join("-garden").join("other.jsonl"), "{\"type\":\"user\"}\n").unwrap();
        let rollout = codex.join("2026/10/12/rollout-x.jsonl");
        let lines = [
            json!({"type": "turn_context", "payload": {"model": "gpt-5.5"}}),
            json!({"type": "token_usage_record", "timestamp": "2026-10-12T01:00:00Z", "payload": {"response_id": "a",
                "usage": {"input_tokens": 3000, "cached_input_tokens": 2000, "output_tokens": 100}}}),
            json!({"type": "token_usage_record", "timestamp": "2026-10-12T01:00:00Z", "payload": {"response_id": "a",
                "usage": {"input_tokens": 3000, "cached_input_tokens": 2000, "output_tokens": 100}}}),
            json!({"type": "event_msg", "timestamp": "2026-10-12T01:00:00Z", "payload": {"type": "token_count",
                "info": {"total_token_usage": {"input_tokens": 3000, "output_tokens": 100}}}}),
        ];
        fs::write(&rollout, lines.iter().map(|l| l.to_string() + "\n").collect::<String>()).unwrap();
        let mut counter = Counter { cache: HashMap::new(), cache_path: dir.0.join("cache/usage.json") };
        let (days, reparsed) = counter.collect(&claude, &codex, 0.0);
        assert_eq!(reparsed, 4);
        // claude: 1000 + 2000 + 10000 + 500 at 4/20/0.2 with the 1 h write at 2x; then the subagent's
        let main_cost = (1000.0 * 4.0 + 2000.0 * 8.0 + 10000.0 * 0.2 + 500.0 * 20.0) / M;
        let sub_cost = (1000.0 * 4.0 + 2000.0 * 8.0 + 10000.0 * 0.2 + 100.0 * 20.0) / M;
        let codex_cost = (1000.0 * 5.0 + 2000.0 * 0.5 + 100.0 * 30.0) / M;
        let day = local_day(Some(&json!("2026-10-12T01:00:00Z"))).unwrap();
        assert_eq!(days[&day].tokens, 13500 + 13100 + 3100);
        assert!((days[&day].cost - (main_cost + sub_cost + codex_cost)).abs() < 1e-12);
        let spend = counter.spend(&claude);
        let (tokens, usd) = spend.of(&SpendKey::Claude(sid.into())).unwrap();
        assert_eq!(tokens, 26600.0);
        assert!((usd - (main_cost + sub_cost)).abs() < 1e-12);
        assert_eq!(spend.of(&SpendKey::Codex(vec![rollout.clone(), dir.0.join("gone.jsonl")])).unwrap().0, 3100.0);
        assert_eq!(spend.of(&SpendKey::Claude("other".into())), None); // no usage in it
        assert_eq!(spend.of(&SpendKey::Claude(String::new())), None);
        assert_eq!(spend.of(&SpendKey::Codex(vec![])), None);
        // a second pass reads nothing, from memory or from the saved cache
        assert_eq!(counter.collect(&claude, &codex, 0.0).1, 0);
        let mut again = Counter { cache: Counter::load(&counter.cache_path).unwrap(), cache_path: counter.cache_path };
        assert_eq!(again.collect(&claude, &codex, 0.0), (days, 0));
        // files older than `since` are skipped
        assert!(again.collect(&claude, &codex, 4e9).0.is_empty());
    }

    #[test]
    fn older_codex_running_totals_are_differenced() {
        let dir = TestDir::new();
        let path = dir.0.join("rollout.jsonl");
        let count = |inp: i64, out: i64| {
            json!({"type": "event_msg", "timestamp": "2026-10-12T01:00:00Z", "payload": {"type": "token_count",
                "info": {"total_token_usage": {"input_tokens": inp, "output_tokens": out}}}})
            .to_string()
                + "\n"
        };
        fs::write(&path, count(100, 10) + &count(300, 30) + &count(300, 30) + &count(50, 5)).unwrap();
        let days = parse_codex(&path).unwrap();
        assert_eq!(days.values().map(|t| t.tokens).sum::<i64>(), 330 + 55);
    }

    #[test]
    fn rounds_and_prints() {
        assert_eq!(human(1_624_000_000), "1.6 B");
        assert_eq!(human(999), "999");
        assert_eq!(cents(693.704), 693.7);
        assert_eq!(price(CLAUDE_PRICES, "claude-opus-5-5-20261001"), Some([4.0, 20.0, 0.2]));
        assert_eq!(price(CLAUDE_PRICES, "claude-opus-5[1m]"), Some([5.0, 25.0, 0.5]));
        assert_eq!(price(CODEX_PRICES, "gpt-5.5-pro"), Some([30.0, 180.0, 30.0, 37.5]));
    }
}
