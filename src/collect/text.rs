//! What a session says, as the card shows it (one line of printable ASCII,
//! the battle's font), and the kind of each tool call it makes (the spell it
//! casts).

use std::cell::OnceCell;

use serde_json::Value;
use unicode_normalization::UnicodeNormalization;
use unicode_normalization::char::is_combining_mark;

use super::Said;
use crate::session::{SaidKind, Tool};
use crate::usage::truthy;

/// Characters of the last message kept; the card shows what fits.
pub const SAID_LEN: usize = 400;

/// Python's str.isspace() (and regex \s): Unicode White_Space plus the
/// ASCII separators \x1c-\x1f.
pub fn py_space(c: char) -> bool {
    c.is_whitespace() || ('\x1c'..='\x1f').contains(&c)
}

/// Python's regex \w: a letter, digit or underscore. Rust's alphanumeric
/// also takes the marks and symbols Unicode calls Other_Alphabetic
/// (a vowel sign, a circled letter); Python's does not.
pub fn word(c: char) -> bool {
    c == '_'
        || (c.is_alphanumeric()
            && !is_combining_mark(c)
            && !matches!(c, '\u{24b6}'..='\u{24e9}' | '\u{1f130}'..='\u{1f189}'))
}

/// Python's str.splitlines().
fn split_lines(s: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut it = s.char_indices().peekable();
    while let Some((i, c)) = it.next() {
        if matches!(c, '\n' | '\r' | '\x0b' | '\x0c' | '\x1c' | '\x1d' | '\x1e' | '\u{85}' | '\u{2028}' | '\u{2029}') {
            out.push(&s[start..i]);
            start = i + c.len_utf8();
            if c == '\r' && it.next_if(|&(_, n)| n == '\n').is_some() {
                start += 1;
            }
        }
    }
    if start < s.len() {
        out.push(&s[start..]);
    }
    out
}

/// Typography the battle's font lacks, as ASCII.
fn ascii(c: char) -> Option<&'static str> {
    Some(match c {
        '\u{2018}' | '\u{2019}' => "'",
        '\u{201c}' | '\u{201d}' => "\"",
        '\u{2013}' => "-",
        '\u{2014}' => " - ",
        '\u{2026}' => "...",
        '\u{2192}' => "->",
        '\u{2190}' => "<-",
        '\u{21d2}' => "=>",
        '\u{00d7}' => "x",
        '\u{2022}' | '\u{00b7}' => "-",
        '\u{2248}' => "~",
        '\u{2265}' => ">=",
        '\u{2264}' => "<=",
        '\u{2260}' => "!=",
        '\u{0111}' => "d",
        '\u{0110}' => "D",
        '\u{00a0}' => " ",
        _ => return None,
    })
}

/// s as one line of printable ASCII (the battle's font), at most n
/// characters, cut at a word with "..." when longer.
pub fn flat(s: &str, n: usize) -> String {
    let mut mapped = String::with_capacity(s.len());
    for c in s.chars() {
        match ascii(c) {
            Some(r) => mapped.push_str(r),
            None => mapped.push(c),
        }
    }
    // accents come off as combining marks
    let kept: String = mapped.nfkd().filter(|&c| (' '..='~').contains(&c) || c == '\t' || c == '\n').collect();
    let s = kept.split_ascii_whitespace().collect::<Vec<_>>().join(" ");
    if s.len() <= n {
        return s;
    }
    let cut = &s[..n.saturating_sub(3)];
    let head = cut.rsplit_once(' ').map_or(cut, |(h, _)| h);
    format!("{}...", head.trim_end_matches([' ', ',', ';', ':', '-']))
}

/// The opening of a markdown reply as one line of plain text: code blocks,
/// tables, headings and rules dropped, emphasis, code spans and links
/// reduced to their text, bullets as "-". It works as a chain of regular
/// expressions would; each step below says which one it stands for.
pub fn plain(md: &str, n: usize) -> String {
    let mut lines = Vec::new();
    let mut fence = false;
    for line in split_lines(md) {
        let t = line.trim_matches(py_space);
        if t.starts_with("```") || t.starts_with("~~~") {
            fence = !fence;
            continue;
        }
        let rule = t.chars().count() >= 3 && t.chars().all(|c| "-*_=".contains(c)); // [-*_=]{3,}
        if fence || t.is_empty() || t.starts_with(['|', '#']) || rule {
            continue;
        }
        let t = t.strip_prefix('>').map_or(t, |r| r.trim_start_matches(py_space)); // ^>\s*
        let t = match t.strip_prefix(['*', '+']) {
            Some(r) if r.starts_with(py_space) => format!("- {}", r.trim_start_matches(py_space)), // ^[*+]\s+
            _ => t.to_owned(),
        };
        lines.push(t);
    }
    let t = autolinks(&links(&lines.join(" ")));
    let t = t.replace("**", "").replace("__", "").replace('`', "");
    flat(&emphasis(&emphasis(&t, '*'), '_'), n)
}

/// For each position, the first one at or after it holding `ch` (usize::MAX for none).
fn next_of(c: &[char], ch: char) -> Vec<usize> {
    let mut next = vec![usize::MAX; c.len() + 1];
    for j in (0..c.len()).rev() {
        next[j] = if c[j] == ch { j } else { next[j + 1] };
    }
    next
}

/// `!?\[([^\]]*)\]\([^)]*\)` -> `\1`: [text](url) and ![alt](src) as their text.
fn links(t: &str) -> String {
    let c: Vec<char> = t.chars().collect();
    let n = c.len();
    let (close, paren) = (next_of(&c, ']'), next_of(&c, ')'));
    let mut out = String::with_capacity(t.len());
    let mut i = 0;
    while i < n {
        let open = match c[i] {
            '[' => Some(i),
            '!' if c.get(i + 1) == Some(&'[') => Some(i + 1),
            _ => None,
        };
        if let Some(s) = open {
            let j = close[s + 1];
            if j != usize::MAX && c.get(j + 1) == Some(&'(') && paren[j + 2] != usize::MAX {
                out.extend(&c[s + 1..j]);
                i = paren[j + 2] + 1;
                continue;
            }
        }
        out.push(c[i]);
        i += 1;
    }
    out
}

/// `<(https?://[^>]+)>` -> `\1`.
fn autolinks(t: &str) -> String {
    let c: Vec<char> = t.chars().collect();
    let n = c.len();
    let gt = next_of(&c, '>');
    let at = |p: usize, s: &str| s.chars().enumerate().all(|(k, ch)| c.get(p + k) == Some(&ch));
    let mut out = String::with_capacity(t.len());
    let mut i = 0;
    while i < n {
        if c[i] == '<' && at(i + 1, "http") {
            let mut p = i + 5;
            if c.get(p) == Some(&'s') {
                p += 1;
            }
            if at(p, "://") && gt[p + 3] != usize::MAX && gt[p + 3] > p + 3 {
                out.extend(&c[i + 1..gt[p + 3]]);
                i = gt[p + 3] + 1;
                continue;
            }
        }
        out.push(c[i]);
        i += 1;
    }
    out
}

/// `(?<![\w*])\*(\S(?:.*?\S)?)\*(?![\w*])` -> `\1` for mark '*', and
/// `(?<!\w)_(\S(?:.*?\S)?)_(?!\w)` -> `\1` for '_'. From an opening mark the
/// expression takes the nearest closing one at least two characters on,
/// and only failing that the one right after its first character (`a* b*`
/// is one span, `*a* b*` loses its outer marks).
fn emphasis(t: &str, mark: char) -> String {
    let c: Vec<char> = t.chars().collect();
    let n = c.len();
    let blocked = |x: char| word(x) || x == mark;
    // c[j] ends a span: its closing mark at j + 1
    let closer =
        |j: usize| j + 1 < n && !py_space(c[j]) && c[j + 1] == mark && c.get(j + 2).is_none_or(|&x| !blocked(x));
    // the first closer at or after each position, not past a line break ('.' stops there)
    let mut next = vec![usize::MAX; n + 1];
    for j in (0..n).rev() {
        next[j] = if c[j] == '\n' {
            usize::MAX
        } else if closer(j) {
            j
        } else {
            next[j + 1]
        };
    }
    let mut out = String::with_capacity(t.len());
    let mut i = 0;
    while i < n {
        if c[i] == mark && (i == 0 || !blocked(c[i - 1])) && i + 1 < n && !py_space(c[i + 1]) {
            let far = next[i + 2];
            let end = if far != usize::MAX { Some(far) } else { closer(i + 1).then_some(i + 1) };
            if let Some(e) = end {
                out.extend(&c[i + 1..=e]);
                i = e + 2;
                continue;
            }
        }
        out.push(c[i]);
        i += 1;
    }
    out
}

/// An AskUserQuestion question and its options as one line.
fn question_said(q: &Value) -> String {
    let opts: Vec<&str> = q
        .get("options")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|o| o.is_object())
        .map(|o| o.get("label").and_then(Value::as_str).unwrap_or(""))
        .collect();
    let question = q.get("question").and_then(Value::as_str).unwrap_or("");
    plain(&format!("{question} {}", opts.join(" / ")), SAID_LEN)
}

/// (ask or tool, text) for the tool call a session waits on.
pub fn tool_said(call: &Value, cwd: &str) -> (SaidKind, String) {
    let name = call.get("name").and_then(Value::as_str).filter(|s| !s.is_empty()).unwrap_or("?");
    let none = serde_json::Map::new();
    let inp = call.get("input").and_then(Value::as_object).unwrap_or(&none);
    let text = |k: &str| inp.get(k).and_then(Value::as_str).unwrap_or("");
    let tool = |s: String| (SaidKind::Tool, flat(&s, SAID_LEN));
    if name == "AskUserQuestion"
        && let Some(qs) = inp.get("questions").filter(|q| truthy(q))
    {
        return (SaidKind::Ask, question_said(qs.get(0).unwrap_or(&Value::Null)));
    }
    if name == "ExitPlanMode" {
        return (SaidKind::Ask, flat(&format!("Approve the plan? {}", plain(text("plan"), SAID_LEN)), SAID_LEN));
    }
    let name = name.split("__").last().unwrap_or(name); // mcp__server__tool -> tool
    if name == "Bash" {
        let (cmd, desc) = (text("command"), text("description"));
        return tool(if desc.is_empty() { format!("Bash: {cmd}") } else { format!("Bash ({desc}): {cmd}") });
    }
    let path = ["file_path", "notebook_path", "path"].iter().find_map(|k| inp.get(*k).filter(|v| truthy(v)));
    if let Some(Value::String(path)) = path {
        let base = cwd.trim_end_matches('/');
        let rel = match path.strip_prefix(base).and_then(|r| r.strip_prefix('/')) {
            Some(rel) if !cwd.is_empty() => rel,
            _ => path,
        };
        return tool(format!("{name}: {rel}"));
    }
    for k in ["url", "query", "pattern", "prompt", "description"] {
        if !text(k).is_empty() {
            return tool(format!("{name}: {}", text(k)));
        }
    }
    tool(name.to_owned())
}

/// Something an agent said, and its plain form, worked out the first time
/// a card needs it.
pub struct Msg {
    pub text: String,
    pub at: Option<f64>,
    plain: OnceCell<String>,
}

impl Msg {
    pub fn new(text: String, at: Option<f64>) -> Self {
        Msg { text, at, plain: OnceCell::new() }
    }

    pub fn said(&self) -> Said {
        let text = self.plain.get_or_init(|| plain(&self.text, SAID_LEN)).clone();
        Said { kind: SaidKind::Msg, text, at: self.at }
    }
}

/* ---- tool calls -----------------------------------------------------------
 * The seven kinds: bash FIRE, edit BLIZZARD, read SCAN, web GALE,
 * agent (the class special), todo PROTECT, mcp THUNDER; a fighter answers
 * every kind with its special. */

const CLAUDE_KIND: &[(&str, Tool)] = &[
    ("Bash", Tool::Bash),
    ("BashOutput", Tool::Bash),
    ("KillShell", Tool::Bash),
    ("KillBash", Tool::Bash),
    ("Monitor", Tool::Bash),
    ("TaskStop", Tool::Bash),
    ("Edit", Tool::Edit),
    ("MultiEdit", Tool::Edit),
    ("Write", Tool::Edit),
    ("NotebookEdit", Tool::Edit),
    ("Read", Tool::Read),
    ("Grep", Tool::Read),
    ("Glob", Tool::Read),
    ("LS", Tool::Read),
    ("NotebookRead", Tool::Read),
    ("ToolSearch", Tool::Read),
    ("Skill", Tool::Read),
    ("ReadNotifications", Tool::Read),
    ("ListAgents", Tool::Read),
    ("WebFetch", Tool::Web),
    ("WebSearch", Tool::Web),
    ("Agent", Tool::Agent),
    ("Task", Tool::Agent),
    ("SendMessage", Tool::Agent),
    ("TodoWrite", Tool::Todo),
    ("TaskCreate", Tool::Todo),
    ("TaskUpdate", Tool::Todo),
    ("TaskList", Tool::Todo),
    ("EnterPlanMode", Tool::Todo),
];

/// waits on the human: the raised hand says it, no spell
const CLAUDE_NO_MOVE: &[&str] = &["AskUserQuestion", "ExitPlanMode"];

const CODEX_KIND: &[(&str, Tool)] = &[
    ("shell", Tool::Bash),
    ("exec_command", Tool::Bash),
    ("local_shell", Tool::Bash),
    ("write_stdin", Tool::Bash),
    ("unified_exec", Tool::Bash),
    ("apply_patch", Tool::Edit),
    ("update_plan", Tool::Todo),
    ("web_search", Tool::Web),
    ("view_image", Tool::Read),
    ("spawn_agent", Tool::Agent),
    ("followup_task", Tool::Agent),
    ("send_message", Tool::Agent),
];

/// bookkeeping, or a question that does not stop Codex: no move
const CODEX_NO_MOVE: &[&str] =
    &["wait_agent", "list_agents", "sleep", "request_user_input_async", "request_user_input"];

fn kind_of(table: &[(&str, Tool)], name: &str) -> Tool {
    table.iter().find(|(n, _)| *n == name).map_or(Tool::Mcp, |&(_, k)| k)
}

/// The kind of a Claude tool call by the tool's name; anything unknown is an MCP tool.
pub fn claude_kind(name: &str) -> Option<Tool> {
    (!CLAUDE_NO_MOVE.contains(&name)).then(|| kind_of(CLAUDE_KIND, name))
}

/// The kind of a Codex response_item payload that calls a tool, else None.
pub fn codex_kind(p: &Value) -> Option<Tool> {
    let name = p.get("name").and_then(Value::as_str).unwrap_or("");
    match p.get("type").and_then(Value::as_str) {
        Some("web_search_call") => return Some(Tool::Web),
        Some("local_shell_call") => return Some(Tool::Bash),
        Some("function_call" | "custom_tool_call") if !CODEX_NO_MOVE.contains(&name) => {}
        _ => return None,
    }
    if name == "exec" {
        // code mode: a script that calls the tools
        let code = p.get("input").and_then(Value::as_str).unwrap_or("");
        return Some(if code.contains("apply_patch") {
            Tool::Edit
        } else if code.contains("web_search") {
            Tool::Web
        } else {
            Tool::Bash
        });
    }
    Some(kind_of(CODEX_KIND, name))
}

/// The one move a poll makes for a session: a subagent's special wins, else the latest.
pub fn pick_tool(kinds: &[Tool]) -> Option<Tool> {
    if kinds.contains(&Tool::Agent) { Some(Tool::Agent) } else { kinds.last().copied() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Markdown in, the card's line out.
    #[test]
    fn plain_strips_markdown() {
        let cases: &[(&str, &str)] = &[
            ("Done. The **menu** now lists `tea` and *coffee*.", "Done. The menu now lists tea and coffee."),
            (
                "# Plan\n\n1. Bake bread\n* Brew tea\n+ Wipe tables\n\n```rust\nfn main() {}\n```\nThat's all.",
                "1. Bake bread - Brew tea - Wipe tables That's all.",
            ),
            ("| col | col |\n|---|---|\n| a | b |\n\n---\n\nAfter the table.", "After the table."),
            ("> Quoted *advice*\n>> twice", "Quoted advice > twice"),
            (
                "See [the guide](https://example.org/guide) and ![logo](logo.png), or <https://example.org>.",
                "See the guide and logo, or https://example.org.",
            ),
            ("snake_case_name stays, _this_ goes, __bold__ too", "snake_case_name stays, this goes, bold too"),
            ("*a* b*", "a* b"),
            ("*a *b* c*", "a *b c*"),
            ("2 * 3 * 4 = 24", "2 * 3 * 4 = 24"),
            ("~~~\nhidden\n~~~\nshown", "shown"),
            (
                "Cr\u{e8}me \u{201c}menu\u{201d} \u{2014} 3\u{d7}4 \u{2192} done\u{2026}",
                "Creme \"menu\" - 3x4 -> done...",
            ),
        ];
        for (md, want) in cases {
            assert_eq!(plain(md, SAID_LEN), *want, "plain({md:?})");
        }
        assert_eq!(plain(&"word ".repeat(90), SAID_LEN), format!("{}...", ["word"; 79].join(" ")));
    }

    #[test]
    fn flat_cuts_at_a_word() {
        assert_eq!(flat("Crème “ok” — done…", SAID_LEN), "Creme \"ok\" - done...");
        assert_eq!(flat("a  b\t\nc", SAID_LEN), "a b c");
        assert_eq!(flat("one two, three", 12), "one two...");
        assert_eq!(flat("abcdefghijkl", 8), "abcde...");
        assert_eq!(flat("日本 x", SAID_LEN), "x");
    }

    #[test]
    fn says_what_a_waiting_session_waits_on() {
        let cwd = "/projects/moon-garden";
        let call = |name: &str, input: Value| json!({"type": "tool_use", "id": "t1", "name": name, "input": input});
        let ask = call(
            "AskUserQuestion",
            json!({"questions": [{"question": "Which **menu**?", "options": [{"label": "Tea"}, {"label": "Coffee"}]}]}),
        );
        assert_eq!(tool_said(&ask, cwd), (SaidKind::Ask, "Which menu? Tea / Coffee".into()));
        let plan = call("ExitPlanMode", json!({"plan": "# Plan\n- bake *bread*"}));
        assert_eq!(tool_said(&plan, cwd), (SaidKind::Ask, "Approve the plan? - bake bread".into()));
        let bash = call("Bash", json!({"command": "ls -l", "description": "List files"}));
        assert_eq!(tool_said(&bash, cwd), (SaidKind::Tool, "Bash (List files): ls -l".into()));
        assert_eq!(tool_said(&call("Bash", json!({"command": "pwd"})), cwd).1, "Bash: pwd");
        let edit = call("Edit", json!({"file_path": "/projects/moon-garden/src/menu.rs"}));
        assert_eq!(tool_said(&edit, cwd).1, "Edit: src/menu.rs");
        assert_eq!(tool_said(&edit, "/projects/moon-garden/").1, "Edit: src/menu.rs");
        assert_eq!(tool_said(&edit, "/projects/moon").1, "Edit: /projects/moon-garden/src/menu.rs");
        let mcp = call("mcp__kitchen__order", json!({"query": "toast"}));
        assert_eq!(tool_said(&mcp, cwd).1, "order: toast");
        assert_eq!(tool_said(&json!({"input": 3}), cwd), (SaidKind::Tool, "?".into()));
    }

    #[test]
    fn kinds_of_tool_calls() {
        assert_eq!(claude_kind("Grep"), Some(Tool::Read));
        assert_eq!(claude_kind("mcp__kitchen__order"), Some(Tool::Mcp));
        assert_eq!(claude_kind("AskUserQuestion"), None);
        let call = |t: &str, name: &str, input: &str| json!({"type": t, "name": name, "input": input});
        assert_eq!(codex_kind(&call("custom_tool_call", "exec", "tools.apply_patch(x)")), Some(Tool::Edit));
        assert_eq!(codex_kind(&call("custom_tool_call", "exec", "tools.web_search(x)")), Some(Tool::Web));
        assert_eq!(codex_kind(&call("custom_tool_call", "exec", "tools.exec_command(x)")), Some(Tool::Bash));
        assert_eq!(codex_kind(&call("function_call", "spawn_agent", "")), Some(Tool::Agent));
        assert_eq!(codex_kind(&call("function_call", "wait_agent", "")), None);
        assert_eq!(codex_kind(&call("function_call", "kitchen_order", "")), Some(Tool::Mcp));
        assert_eq!(codex_kind(&call("web_search_call", "", "")), Some(Tool::Web));
        assert_eq!(codex_kind(&call("message", "", "")), None);
        assert_eq!(pick_tool(&[Tool::Bash, Tool::Agent, Tool::Read]), Some(Tool::Agent));
        assert_eq!(pick_tool(&[Tool::Bash, Tool::Read]), Some(Tool::Read));
        assert_eq!(pick_tool(&[]), None);
    }

    #[test]
    fn splits_lines_like_python() {
        assert_eq!(split_lines("a\r\nb\rc\u{2028}d\n"), ["a", "b", "c", "d"]);
        assert_eq!(split_lines("a\n\nb"), ["a", "", "b"]);
        assert!(split_lines("").is_empty());
    }
}
