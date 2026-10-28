//! The project a session works in, its monster: the git repository its
//! directory is in, known by the repository's remote (origin's, else the
//! first), so clones, worktrees and subfolders of one repository meet one
//! monster; `git@host:a/b.git` and `https://host/a/b` are both host/a/b. A
//! directory outside a repository with a remote is a project of its own,
//! with home as ~. The git files are read directly: no `git` runs in the
//! user's repositories.

use std::collections::HashMap;
use std::fs;
use std::path::Path;

use crate::collect::{py_space as space, word};
use crate::usage::home;

/// How long a directory's repository is remembered, seconds.
const REPO_S: f64 = 300.0;
/// The longest project name sent (the battle shows its last part).
const PROJ_LEN: usize = 40;

/// Home as Python's os.path.expanduser('~') has it: no trailing '/'.
fn home_dir() -> String {
    let h = home().to_string_lossy().into_owned();
    let t = h.trim_end_matches('/');
    if t.is_empty() { "/".into() } else { t.into() }
}

/// Home as ~, and the end of a long path after '..' ('../work/lantern').
pub(crate) fn short_path(path: &str, n: usize) -> String {
    if path.is_empty() {
        return String::new();
    }
    let home = home_dir();
    let path = match path.strip_prefix(&home) {
        Some(rest) if rest.is_empty() || rest.starts_with('/') => format!("~{rest}"),
        _ => path.to_owned(),
    };
    let chars: Vec<char> = path.chars().collect();
    if chars.len() <= n {
        return path;
    }
    let tail = &chars[chars.len() - (n - 2)..];
    match tail.iter().position(|&c| c == '/') {
        Some(cut) if cut < tail.len() - 1 => format!("..{}", tail[cut..].iter().collect::<String>()),
        _ => format!("..{}", tail.iter().collect::<String>()),
    }
}

/* ---- POSIX path arithmetic, as Python's posixpath does it ---------------- */

fn join(a: &str, b: &str) -> String {
    if b.starts_with('/') {
        b.to_owned()
    } else if a.is_empty() || a.ends_with('/') {
        format!("{a}{b}")
    } else {
        format!("{a}/{b}")
    }
}

fn dirname(p: &str) -> String {
    let head = &p[..p.rfind('/').map_or(0, |i| i + 1)];
    if !head.is_empty() && head.bytes().any(|b| b != b'/') {
        head.trim_end_matches('/').to_owned()
    } else {
        head.to_owned()
    }
}

fn normpath(p: &str) -> String {
    if p.is_empty() {
        return ".".into();
    }
    let slashes = if p.starts_with("//") && !p.starts_with("///") { 2 } else { usize::from(p.starts_with('/')) };
    let mut comps: Vec<&str> = Vec::new();
    for c in p.split('/') {
        if c.is_empty() || c == "." {
            continue;
        }
        if c != ".." || (slashes == 0 && comps.is_empty()) || comps.last() == Some(&"..") {
            comps.push(c);
        } else if !comps.is_empty() {
            comps.pop();
        }
    }
    let out = format!("{}{}", "/".repeat(slashes), comps.join("/"));
    if out.is_empty() { ".".into() } else { out }
}

/// The first line of a file, as Python's readline() in text mode gives it.
fn first_line(text: &str) -> &str {
    text.split(['\n', '\r']).next().unwrap_or("")
}

/// The config file of the git repository cwd is in, a worktree's or a
/// submodule's included (their .git file points at the git dir, a
/// worktree's commondir at the main one), or None outside a repository.
fn git_config(cwd: &str) -> Option<String> {
    let mut d = cwd.to_owned();
    let gitdir = loop {
        let dot = join(&d, ".git");
        let p = Path::new(&dot);
        if p.is_dir() {
            break dot;
        }
        if p.is_file() {
            let text = String::from_utf8_lossy(&fs::read(p).ok()?).into_owned();
            let line = first_line(&text).trim_matches(space);
            let to = line.strip_prefix("gitdir:")?;
            break normpath(&join(&d, to.trim_matches(space)));
        }
        let parent = dirname(&d);
        if parent == d {
            return None;
        }
        d = parent;
    };
    let gitdir = match fs::read_to_string(join(&gitdir, "commondir")) {
        Ok(common) => normpath(&join(&gitdir, common.trim_matches(space))),
        Err(_) => gitdir,
    };
    Some(join(&gitdir, "config"))
}

/// `[remote "name"]` -> name (`\[\s*remote\s+"([^"]*)"\s*\]`, then anything).
fn remote_section(t: &str) -> Option<&str> {
    let r = t.strip_prefix('[')?.trim_start_matches(space).strip_prefix("remote")?;
    let name = r.strip_prefix(space)?.trim_start_matches(space).strip_prefix('"')?;
    let (name, rest) = name.split_once('"')?;
    rest.trim_start_matches(space).starts_with(']').then_some(name)
}

/// `url = value` -> value (`url\s*=\s*(.*)`, stripped of spaces and quotes).
fn url_value(t: &str) -> Option<&str> {
    let v = t.strip_prefix("url")?.trim_start_matches(space).strip_prefix('=')?;
    Some(v.trim_matches(space).trim_matches('"'))
}

/// origin's URL in a git config file, else the first remote's; None without one.
fn git_remote(config: &str) -> Option<String> {
    let text = String::from_utf8_lossy(&fs::read(config).ok()?).into_owned();
    let mut urls: Vec<(&str, &str)> = Vec::new();
    let mut section = None;
    for line in text.split(['\n', '\r']) {
        let t = line.trim_matches(space);
        if t.starts_with('[') {
            section = remote_section(t);
        } else if let Some(s) = section
            && !urls.iter().any(|(k, _)| *k == s)
            && let Some(url) = url_value(t)
        {
            urls.push((s, url));
        }
    }
    let origin = urls.iter().find(|(k, _)| *k == "origin").map(|(_, u)| *u).filter(|u| !u.is_empty());
    origin.or_else(|| urls.first().map(|(_, u)| *u)).map(str::to_owned)
}

/// Splits an optional `user@` (`(?:[^@/]+@)?`) off s and hands the rest to
/// host, as the regular expression tries it: with the user first.
fn with_user<'a>(s: &'a str, host: impl Fn(&'a str) -> Option<(&'a str, &'a str)>) -> Option<(&'a str, &'a str)> {
    if let Some(at) = s.find('@')
        && at > 0
        && !s[..at].contains('/')
        && let Some(hit) = host(&s[at + 1..])
    {
        return Some(hit);
    }
    host(s)
}

/// `^[\w+.-]+://(?:[^@/]+@)?([^/:]+)(?::\d+)?/(.*)$` -> (host, path).
fn url_parts(u: &str) -> Option<(&str, &str)> {
    let (scheme, rest) = u.split_once("://")?;
    if scheme.is_empty() || !scheme.chars().all(|c| word(c) || "+.-".contains(c)) {
        return None;
    }
    with_user(rest, |s| {
        let end = s.find(['/', ':']).unwrap_or(s.len());
        if end == 0 {
            return None;
        }
        let mut after = &s[end..];
        if let Some(port) = after.strip_prefix(':') {
            let digits = port.bytes().take_while(u8::is_ascii_digit).count();
            if digits == 0 {
                return None;
            }
            after = &port[digits..];
        }
        let path = after.strip_prefix('/')?;
        (!path.contains('\n')).then_some((&s[..end], path))
    })
}

/// `^(?:[^@/]+@)?([^/:]+):(?!/)(.*)$` (scp-like) -> (host, path).
fn scp_parts(u: &str) -> Option<(&str, &str)> {
    with_user(u, |s| {
        let end = s.find(['/', ':'])?;
        let path = s[end..].strip_prefix(':')?;
        (end > 0 && !path.starts_with('/') && !path.contains('\n')).then_some((&s[..end], path))
    })
}

/// A remote URL as host/path: git@github.com:a/b.git, ssh://git@github.com/a/b
/// and https://github.com/a/b.git are all github.com/a/b.
fn remote_key(url: &str) -> String {
    let u = url.trim_matches(space);
    let key = match url_parts(u).or_else(|| scp_parts(u)) {
        Some((host, path)) => format!("{}/{path}", host.to_lowercase()),
        None => u.to_owned(), // a path on this machine
    };
    let key = key.trim_end_matches('/');
    key.strip_suffix(".git").unwrap_or(key).to_owned()
}

/// Each directory's project, remembered for 5 minutes.
#[derive(Default)]
pub struct Projects {
    repos: HashMap<String, (f64, String)>,
}

impl Projects {
    /// The project a session works in: its repository's remote, else its
    /// directory, at most 40 characters.
    pub fn project_of(&mut self, cwd: &str, now: f64) -> String {
        if let Some((at, proj)) = self.repos.get(cwd)
            && now - at < REPO_S
        {
            return proj.clone();
        }
        let url = git_config(cwd).and_then(|c| git_remote(&c)).filter(|u| !u.is_empty());
        let proj = short_path(&url.map_or_else(|| cwd.to_owned(), |u| remote_key(&u)), PROJ_LEN);
        self.repos.insert(cwd.to_owned(), (now, proj.clone()));
        proj
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collect::TestDir;

    #[test]
    fn remote_keys() {
        let cases = [
            ("git@github.com:moon/garden.git", "github.com/moon/garden"),
            ("https://GitHub.com/moon/garden.git/", "github.com/moon/garden"),
            ("ssh://git@github.com:22/moon/garden", "github.com/moon/garden"),
            ("https://user:pw@git.example.org/moon/garden", "git.example.org/moon/garden"),
            ("user:pw@host:moon/garden", "host/moon/garden"),
            ("host:/srv/garden.git", "host:/srv/garden"),
            ("ssh://host:x/garden", "ssh://host:x/garden"),
            ("/srv/git/garden.git", "/srv/git/garden"),
            ("  git@gitlab.com:moon/garden  ", "gitlab.com/moon/garden"),
            ("a@b/c@d:e", "a@b/c@d:e"),
            ("x@:y", "x@/y"),
            ("https://@host/p", "@host/p"),
            ("file:///srv/garden", "file:///srv/garden"),
        ];
        for (url, key) in cases {
            assert_eq!(remote_key(url), key, "{url}");
        }
    }

    #[test]
    fn paths() {
        assert_eq!(normpath("/a/b/../c/./d/"), "/a/c/d");
        assert_eq!(normpath("//a//b"), "//a/b");
        assert_eq!(normpath("../a/../../b"), "../../b");
        assert_eq!(normpath("/.."), "/");
        assert_eq!(dirname("/a/b"), "/a");
        assert_eq!(dirname("/a"), "/");
        assert_eq!(dirname("/"), "/");
        assert_eq!(dirname("a"), "");
        assert_eq!(join("/a", "/b"), "/b");
        assert_eq!(
            short_path("/srv/projects/a-rather-long-folder-name/for-the-moon-garden", 40),
            "../for-the-moon-garden"
        );
        assert_eq!(
            short_path("github.com/a-very-long-owner-name/with-a-long-repository", 40),
            "../with-a-long-repository"
        );
        assert_eq!(
            short_path("/srv/a/averyveryveryveryveryveryverylongfoldername/", 40),
            "..eryveryveryveryveryverylongfoldername/"
        );
        assert_eq!(short_path("github.com/moon/garden", 40), "github.com/moon/garden");
        assert_eq!(short_path("", 40), "");
    }

    #[test]
    fn finds_the_remote_of_repositories_worktrees_and_plain_folders() {
        let dir = TestDir::new();
        let root = dir.0.to_string_lossy().into_owned();
        let main = format!("{root}/moon-garden");
        fs::create_dir_all(format!("{main}/.git/worktrees/wt")).unwrap();
        fs::create_dir_all(format!("{main}/src/menu")).unwrap();
        fs::write(
            format!("{main}/.git/config"),
            "[core]\n\turl = nope\n[remote \"upstream\"]\n\turl = https://example.org/up/garden\n\
             [remote \"origin\"]\n\tfetch = +refs/heads/*\n\turl = \"git@github.com:moon/garden.git\"\n\turl = second\n",
        )
        .unwrap();
        fs::write(format!("{main}/.git/worktrees/wt/commondir"), "../..\n").unwrap();
        let wt = format!("{root}/garden-wt");
        fs::create_dir_all(&wt).unwrap();
        fs::write(format!("{wt}/.git"), format!("gitdir: {main}/.git/worktrees/wt\n")).unwrap();
        let plain = format!("{root}/plain");
        fs::create_dir_all(&plain).unwrap();
        let mut p = Projects::default();
        assert_eq!(p.project_of(&format!("{main}/src/menu"), 0.0), "github.com/moon/garden");
        assert_eq!(p.project_of(&wt, 0.0), "github.com/moon/garden");
        assert_eq!(git_config(&wt), Some(format!("{main}/.git/config")));
        assert_eq!(p.project_of(&plain, 0.0), short_path(&plain, PROJ_LEN));
        // remembered for 5 minutes
        fs::write(format!("{main}/.git/config"), "[remote \"x\"]\nurl = /srv/other\n").unwrap();
        assert_eq!(p.project_of(&wt, 299.0), "github.com/moon/garden");
        assert_eq!(p.project_of(&wt, 300.0), "/srv/other");
    }
}
