//! Quota left on the Claude and Codex plans: the MP bars.
//!
//! Both numbers come from the endpoints the CLIs use for their own status
//! displays, read with the OAuth tokens the CLIs keep on disk. The tokens only
//! ever go to their own API, on curl's stdin (never a command line); they are
//! never printed or logged, and never refreshed here (the CLIs do that):
//!
//!   Claude  ~/.claude/.credentials.json -> GET https://api.anthropic.com/api/oauth/usage
//!           five_hour / seven_day: {utilization, resets_at}
//!   Codex   ~/.codex/auth.json -> GET https://chatgpt.com/backend-api/wham/usage
//!           rate_limit.primary_window / secondary_window:
//!           {used_percent, limit_window_seconds, reset_at}; the weekly one is
//!           whichever is longer than 12 h (this user's Business plan has only it)
//!
//! Both endpoints answer a chatty client with 429, so each response is cached
//! in ~/.cache/token-fantasy-desktop/<tool>-usage.json, fetched at most every FETCH_S,
//! and a 429 starts a cooldown that restarts respect too. For Claude the
//! waybar module's cache is taken instead when it is the fresher, so the two
//! pollers share one reading.

use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

use crate::collect::epoch;
use crate::session::Mana;
use crate::usage::{cache_dir, claude_dir, codex_dir, home, truthy};

/// The numbers move slowly and the endpoints rate-limit.
const FETCH_S: f64 = 180.0;
/// After a 429 without a longer Retry-After.
const COOLDOWN_S: f64 = 900.0;
/// An older reading is shown as unknown.
const STALE_S: f64 = 4.0 * FETCH_S;
const TIMEOUT_S: &str = "8";
/// How often to call Quota::poll: the cache keeps the endpoints at FETCH_S;
/// this picks up waybar's reading sooner.
pub const POLL_S: f64 = FETCH_S / 6.0;

const CLAUDE_URL: &str = "https://api.anthropic.com/api/oauth/usage";
const CODEX_URL: &str = "https://chatgpt.com/backend-api/wham/usage";

/// The windows a tool's reading gave: (key, used %, resets at).
type Windows = Vec<(&'static str, f64, Option<f64>)>;

/// An endpoint's answer.
struct Reply {
    status: u16,
    retry_after: Option<String>,
    body: Vec<u8>,
}

fn read_json(path: &Path) -> Option<Value> {
    serde_json::from_slice(&fs::read(path).ok()?).ok()
}

fn write_json(path: &Path, v: &Value) {
    let Some(dir) = path.parent() else { return };
    let tmp = path.with_extension(format!("json.{}.tmp", std::process::id()));
    if fs::create_dir_all(dir).is_ok() && fs::write(&tmp, v.to_string()).is_ok() {
        let _ = fs::rename(&tmp, path);
    }
}

fn mtime(path: &Path) -> Option<f64> {
    let t = fs::metadata(path).ok()?.modified().ok()?;
    Some(t.duration_since(UNIX_EPOCH).ok()?.as_secs_f64())
}

fn now() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64())
}

/// A number, or a string holding one, as Python's float() takes it.
fn float(v: &Value) -> Option<f64> {
    v.as_f64().or_else(|| v.as_str()?.trim().parse().ok())
}

/// GET url with curl. The headers go on its stdin, so the token is on no
/// command line; curl's own messages are dropped, the exit code says enough.
fn curl(url: &str, headers: &[(&str, &str)]) -> Result<Reply, String> {
    let mut lines = String::new();
    for (k, v) in [("Accept", "application/json"), ("User-Agent", "token-fantasy-desktop/1")].iter().chain(headers) {
        if v.contains(['\r', '\n']) {
            return Err(format!("bad {k} header"));
        }
        // curl sends an empty header as "Name;"; "Name:" would drop it
        lines += &if v.is_empty() { format!("{k};\n") } else { format!("{k}: {v}\n") };
    }
    let mut child = Command::new("curl")
        .args(["-sS", "--max-time", TIMEOUT_S, "-H", "@-", "-w", "\n%{http_code} %header{retry-after}", url])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("cannot run curl: {}", e.kind()))?;
    let wrote = child.stdin.take().is_some_and(|mut stdin| stdin.write_all(lines.as_bytes()).is_ok());
    let out = child.wait_with_output().map_err(|e| format!("curl: {}", e.kind()))?;
    if !out.status.success() {
        return Err(match out.status.code() {
            Some(6) => "cannot resolve host".into(),
            Some(7) => "cannot connect".into(),
            Some(28) => "timed out".into(),
            Some(35) => "TLS handshake failed".into(),
            Some(code) => format!("curl exit {code}"),
            None => "curl killed".into(),
        });
    }
    if !wrote {
        return Err("curl took no headers".into());
    }
    let nl = memchr::memrchr(b'\n', &out.stdout).ok_or("bad reply")?;
    let trailer = String::from_utf8_lossy(&out.stdout[nl + 1..]).into_owned();
    let (code, retry) = trailer.split_once(' ').unwrap_or((&trailer, ""));
    let status = code.trim().parse().map_err(|_| "bad reply")?;
    let retry_after = Some(retry.trim().to_owned()).filter(|r| !r.is_empty());
    Ok(Reply { status, retry_after, body: out.stdout[..nl].to_vec() })
}

/// Polls both plans; mana() is what the bars show.
pub struct Quota {
    /// key -> (used %, resets at, when fetched)
    windows: HashMap<&'static str, (f64, Option<f64>, f64)>,
    /// tool -> the problem last logged
    errors: HashMap<&'static str, String>,
    cache: PathBuf,
    waybar: PathBuf,
}

impl Default for Quota {
    fn default() -> Self {
        Self::new()
    }
}

impl Quota {
    pub fn new() -> Self {
        Quota {
            windows: HashMap::new(),
            errors: HashMap::new(),
            cache: cache_dir(),
            waybar: home().join(".cache/polybar/claude-usage.json"),
        }
    }

    /// -> (body or None, when it was fetched, cooldown until).
    fn cached(&self, name: &str) -> (Option<Value>, f64, f64) {
        let c = read_json(&self.cache.join(format!("{name}-usage.json"))).unwrap_or(Value::Null);
        let n = |k: &str| c.get(k).and_then(float).unwrap_or(0.0);
        let mut best = (c.get("body").filter(|b| !b.is_null()).cloned(), n("fetched"), n("cooldown_until"));
        if name == "claude"
            && let Some(raw) = read_json(&self.waybar)
            && raw.get("five_hour").is_some()
            && let Some(at) = mtime(&self.waybar)
            && at > best.1
        {
            best = (Some(raw), at, best.2);
        }
        best
    }

    /// -> (body, when fetched) from the cache while it is fresh or cooling
    /// down, else from the endpoint (`request`).
    fn fetch(
        &self,
        name: &str,
        now: f64,
        request: impl FnOnce() -> Result<Reply, String>,
    ) -> Result<(Option<Value>, f64), String> {
        let (body, fetched, cooldown) = self.cached(name);
        if (body.is_some() && now - fetched < FETCH_S) || now < cooldown {
            return Ok((body, fetched));
        }
        let reply = request()?;
        let path = self.cache.join(format!("{name}-usage.json"));
        if reply.status == 429 {
            let asked = reply.retry_after.and_then(|r| r.trim().parse::<f64>().ok()).filter(|w| w.is_finite());
            let wait = asked.map_or(COOLDOWN_S, |w| w.max(COOLDOWN_S));
            write_json(&path, &json!({"body": body, "fetched": fetched, "cooldown_until": now + wait}));
            return Err("HTTP 429".into());
        }
        if !(200..300).contains(&reply.status) {
            return Err(format!("HTTP {}", reply.status));
        }
        let fresh: Value = serde_json::from_slice(&reply.body).map_err(|_| "bad reply")?;
        write_json(&path, &json!({"body": fresh, "fetched": now, "cooldown_until": 0}));
        Ok((Some(fresh).filter(|b| !b.is_null()), now))
    }

    fn claude(&self, now: f64) -> Result<(Windows, f64), String> {
        let creds = read_json(&claude_dir().join(".credentials.json")).ok_or("cannot read .credentials.json")?;
        let token = creds.pointer("/claudeAiOauth/accessToken").and_then(Value::as_str);
        let auth = format!("Bearer {}", token.ok_or("no token in .credentials.json")?);
        let headers = [("Authorization", auth.as_str()), ("anthropic-beta", "oauth-2025-04-20")];
        let (body, fetched) = self.fetch("claude", now, || curl(CLAUDE_URL, &headers))?;
        let mut out = Windows::new();
        for (key, field) in [("claude_5h", "five_hour"), ("claude_week", "seven_day")] {
            let Some(w) = body.as_ref().and_then(|b| b.get(field)) else { continue };
            let Some(used) = w.get("utilization").filter(|u| !u.is_null()) else { continue };
            let reset = match w.get("resets_at").filter(|r| truthy(r)) {
                Some(r) => Some(r.as_str().and_then(epoch).ok_or("bad reply")?),
                None => None,
            };
            out.push((key, float(used).ok_or("bad reply")?, reset));
        }
        Ok((out, fetched))
    }

    fn codex(&self, now: f64) -> Result<(Windows, f64), String> {
        let auth = read_json(&codex_dir().join("auth.json")).ok_or("cannot read auth.json")?;
        let tokens = auth.get("tokens").ok_or("no token in auth.json")?;
        let token = tokens.get("access_token").and_then(Value::as_str).ok_or("no token in auth.json")?;
        let bearer = format!("Bearer {token}");
        let account = tokens.get("account_id").and_then(Value::as_str).unwrap_or("");
        let headers = [("Authorization", bearer.as_str()), ("ChatGPT-Account-Id", account)];
        let (body, fetched) = self.fetch("codex", now, || curl(CODEX_URL, &headers))?;
        let mut out = Windows::new();
        let rl = body.as_ref().and_then(|b| b.get("rate_limit"));
        for w in ["primary_window", "secondary_window"].map(|k| rl.and_then(|r| r.get(k)).filter(|w| truthy(w))) {
            let Some(w) = w else { continue };
            let Some(used) = w.get("used_percent").filter(|u| !u.is_null()) else { continue };
            if w.get("limit_window_seconds").and_then(float).unwrap_or(0.0) > 12.0 * 3600.0 {
                let reset = w.get("reset_at").filter(|r| truthy(r)).map(|r| float(r).ok_or("bad reply")).transpose()?;
                out.retain(|(k, _, _)| *k != "codex_week");
                out.push(("codex_week", float(used).ok_or("bad reply")?, reset));
            }
        }
        Ok((out, fetched))
    }

    /// Takes in one tool's reading, logging each new problem once, and its end.
    fn note(&mut self, name: &'static str, got: Result<(Windows, f64), String>, log: &dyn Fn(&str)) {
        let err = match got {
            Ok((windows, fetched)) => {
                for (k, used, reset) in windows {
                    self.windows.insert(k, (used, reset, fetched));
                }
                None
            }
            Err(e) => Some(e),
        };
        if err.as_ref() != self.errors.get(name) {
            match &err {
                Some(e) => log(&format!("{name} quota: {e}")),
                None => log(&format!("{name} quota: reading again")),
            }
            match err {
                Some(e) => self.errors.insert(name, e),
                None => self.errors.remove(name),
            };
        }
    }

    /// One round over Claude and Codex: from the caches, or the endpoints
    /// when those are due (blocks up to 8 s each then). Call it every POLL_S.
    pub fn poll(&mut self, log: &dyn Fn(&str)) {
        let got = self.claude(now());
        self.note("claude", got, log);
        let got = self.codex(now());
        self.note("codex", got, log);
    }

    /// Percent left on each window: unknown (-1) while never read or stale;
    /// 100 once the window has reset since the reading.
    pub fn mana(&self, now: f64) -> Mana {
        let left = |k: &str| match self.windows.get(k) {
            Some(&(_, _, fetched)) if now - fetched > STALE_S => -1,
            Some(&(_, Some(reset), _)) if reset != 0.0 && now >= reset => 100,
            Some(&(used, _, _)) => ((100.0 - used).round_ties_even() as i32).clamp(0, 100),
            None => -1,
        };
        Mana { claude_5h: left("claude_5h"), claude_week: left("claude_week"), codex_week: left("codex_week") }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collect::TestDir;
    use std::cell::RefCell;

    fn quota(dir: &TestDir) -> Quota {
        Quota {
            windows: HashMap::new(),
            errors: HashMap::new(),
            cache: dir.0.join("cache"),
            waybar: dir.0.join("waybar.json"),
        }
    }

    fn reply(status: u16, retry_after: Option<&str>, body: &str) -> Result<Reply, String> {
        Ok(Reply { status, retry_after: retry_after.map(str::to_owned), body: body.as_bytes().to_vec() })
    }

    fn never() -> Result<Reply, String> {
        panic!("the endpoint was asked")
    }

    #[test]
    fn caches_and_cools_down() {
        let dir = TestDir::new();
        let q = quota(&dir);
        // nothing cached: ask, and keep the answer
        let body = r#"{"five_hour": {"utilization": 27.5, "resets_at": "2026-10-12T05:00:00Z"}}"#;
        let (got, at) = q.fetch("claude", 1000.0, || reply(200, None, body)).unwrap();
        assert_eq!((got.unwrap()["five_hour"]["utilization"].as_f64(), at), (Some(27.5), 1000.0));
        // fresh for FETCH_S
        assert_eq!(q.fetch("claude", 1179.0, never).unwrap().1, 1000.0);
        // then a 429: the cached reading stays, the cooldown is at least COOLDOWN_S
        assert_eq!(q.fetch("claude", 1180.0, || reply(429, Some("30"), "")), Err("HTTP 429".into()));
        assert_eq!(q.cached("claude").2, 1180.0 + COOLDOWN_S);
        assert_eq!(q.fetch("claude", 2079.0, never).unwrap().1, 1000.0);
        assert_eq!(q.fetch("claude", 2080.0, || reply(429, Some("3600"), "")), Err("HTTP 429".into()));
        assert_eq!(q.cached("claude").2, 2080.0 + 3600.0);
        assert_eq!(q.fetch("codex", 0.0, || reply(500, None, "")), Err("HTTP 500".into()));
        assert_eq!(q.fetch("codex", 0.0, || reply(200, None, "<html>")), Err("bad reply".into()));
        // a fresher waybar reading wins for Claude
        fs::write(&q.waybar, r#"{"five_hour": {"utilization": 50}}"#).unwrap();
        let (got, _) = q.fetch("claude", 2080.0, never).unwrap();
        assert_eq!(got.unwrap()["five_hour"]["utilization"], 50);
    }

    #[test]
    fn mana_left() {
        let dir = TestDir::new();
        let mut q = quota(&dir);
        q.windows.insert("claude_5h", (27.5, Some(1600.0), 1000.0));
        q.windows.insert("claude_week", (100.4, None, 1000.0));
        q.windows.insert("codex_week", (12.0, None, 900.0));
        // Python's round(): halves to even
        assert_eq!(q.mana(1500.0), Mana { claude_5h: 72, claude_week: 0, codex_week: 88 });
        assert_eq!(q.mana(1621.0), Mana { claude_5h: 100, claude_week: 0, codex_week: -1 });
        assert_eq!(q.mana(1721.0), Mana { claude_5h: -1, claude_week: -1, codex_week: -1 });
        assert_eq!(Quota::new().mana(0.0), Mana::default());
    }

    #[test]
    fn logs_each_problem_once() {
        let dir = TestDir::new();
        let mut q = quota(&dir);
        let lines = RefCell::new(Vec::new());
        let log = |s: &str| lines.borrow_mut().push(s.to_owned());
        q.note("codex", Ok((vec![("codex_week", 40.0, None)], 100.0)), &log);
        q.note("codex", Err("HTTP 429".into()), &log);
        q.note("codex", Err("HTTP 429".into()), &log);
        q.note("codex", Err("timed out".into()), &log);
        q.note("codex", Ok((vec![], 100.0)), &log);
        assert_eq!(*lines.borrow(), ["codex quota: HTTP 429", "codex quota: timed out", "codex quota: reading again"]);
        assert_eq!(q.mana(200.0).codex_week, 60);
    }

    #[test]
    #[ignore = "hits the real endpoints once"]
    fn reads_the_real_quota() {
        let mut q = Quota::new();
        q.poll(&|s| eprintln!("{s}"));
        eprintln!("{:?}", q.mana(now()));
    }
}
