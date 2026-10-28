//! The kitty window a session runs in, found in its process's environment,
//! and bringing it to the front: `kitten @ focus-window`, then hyprctl on
//! Linux or `open -a kitty` on macOS. Needs kitty's remote control socket
//! (`listen_on` in kitty.conf).

use std::collections::HashMap;
use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Where a session's kitty window is.
#[derive(Clone, Debug, PartialEq)]
pub struct Terminal {
    /// kitty's remote control socket (KITTY_LISTEN_ON).
    pub to: String,
    /// The window's id in that kitty (KITTY_WINDOW_ID).
    pub window: u64,
    /// kitty's pid, 0 if unknown: Hyprland finds the OS window by it.
    pub kitty: u32,
    /// The Hyprland instance kitty runs under, "" if unknown.
    pub hypr: String,
}

/// The terminal of a process from its environment (NUL-separated NAME=value
/// entries), or None when it does not run in a kitty with a remote control
/// socket.
pub(crate) fn of_environ(raw: &[u8]) -> Option<Terminal> {
    let mut env: HashMap<&[u8], &[u8]> = HashMap::new();
    for x in raw.split(|&b| b == 0) {
        if let Some(i) = memchr::memchr(b'=', x) {
            env.insert(&x[..i], &x[i + 1..]);
        }
    }
    let var = |k: &str| std::str::from_utf8(env.get(k.as_bytes()).copied().unwrap_or(b"")).ok();
    let (to, window) = (var("KITTY_LISTEN_ON")?, var("KITTY_WINDOW_ID")?);
    if to.is_empty() || window.is_empty() || !window.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let kitty = var("KITTY_PID")?.trim();
    Some(Terminal {
        to: to.to_owned(),
        window: window.parse().ok()?,
        kitty: if kitty.is_empty() { 0 } else { kitty.parse().ok()? },
        hypr: var("HYPRLAND_INSTANCE_SIGNATURE")?.to_owned(),
    })
}

const TIMEOUT: Duration = Duration::from_secs(5);

/// Runs a command to its end, at most TIMEOUT: (it succeeded, stdout, stderr).
fn run(cmd: &mut Command) -> Result<(bool, String, String), String> {
    let program = cmd.get_program().to_string_lossy().into_owned();
    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("{program}: {e}"))?;
    let deadline = Instant::now() + TIMEOUT;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(10)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("{program} timed out after {} seconds", TIMEOUT.as_secs()));
            }
            Err(e) => return Err(format!("{program}: {e}")),
        }
    };
    let read = |r: Option<&mut dyn Read>| {
        let mut buf = Vec::new();
        if let Some(r) = r {
            let _ = r.read_to_end(&mut buf);
        }
        String::from_utf8_lossy(&buf).into_owned()
    };
    let out = read(child.stdout.as_mut().map(|r| r as &mut dyn Read));
    let err = read(child.stderr.as_mut().map(|r| r as &mut dyn Read));
    Ok((status.success(), out, err))
}

/// What a failed command said: its stderr, else its stdout.
fn what(out: &str, err: &str) -> String {
    if err.is_empty() { out } else { err }.trim().to_owned()
}

/// Brings a kitty window to the front, or says what went wrong. kitty
/// switches to its tab and split, then the window has to come forward:
/// Hyprland ignores kitty's own request to be focused (misc:focus_on_activate
/// is off), so hyprctl raises it (Lua dispatch syntax, Hyprland 0.56; with
/// several kitty OS windows Hyprland picks the first by pid); on macOS
/// `open -a kitty` activates the app. Blocks up to 5 s per command.
pub fn bring_up(t: &Terminal) -> Result<(), String> {
    let window = format!("id:{}", t.window);
    let (ok, out, err) = run(Command::new("kitten").args(["@", "--to", &t.to, "focus-window", "--match", &window]))?;
    if !ok {
        return Err(format!("kitty: {}", what(&out, &err)));
    }
    if cfg!(target_os = "macos") {
        let (ok, out, err) = run(Command::new("open").args(["-a", "kitty"]))?;
        if !ok {
            return Err(format!("open: {}", what(&out, &err)));
        }
    } else if t.kitty != 0 {
        let mut cmd = Command::new("hyprctl");
        cmd.args(["dispatch", &format!("hl.dsp.focus({{ window = \"pid:{}\" }})", t.kitty)]);
        if !t.hypr.is_empty() {
            cmd.env("HYPRLAND_INSTANCE_SIGNATURE", &t.hypr);
        }
        let (ok, out, err) = run(&mut cmd)?;
        if !ok || out.trim() != "ok" {
            return Err(format!("hyprctl: {}", what(&out, &err)));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runs_commands_with_a_deadline() {
        assert_eq!(
            run(Command::new("sh").args(["-c", "echo ok; echo oops >&2"])),
            Ok((true, "ok\n".into(), "oops\n".into()))
        );
        assert_eq!(run(Command::new("sh").args(["-c", "exit 3"])).map(|r| r.0), Ok(false));
        assert!(run(&mut Command::new("/nonexistent/kitten")).unwrap_err().starts_with("/nonexistent/kitten: "));
        assert_eq!(what("out\n", ""), "out");
        assert_eq!(what("out", " err "), "err");
    }

    #[test]
    fn finds_the_kitty_window_in_the_environment() {
        assert_eq!(of_environ(b"PATH=/bin\0KITTY_WINDOW_ID=7\0"), None);
        let env = b"PATH=/bin\0KITTY_LISTEN_ON=unix:/tmp/kitty-0\0KITTY_WINDOW_ID=7\0KITTY_PID=42\0\
                    HYPRLAND_INSTANCE_SIGNATURE=sig\0";
        let want = Terminal { to: "unix:/tmp/kitty-0".into(), window: 7, kitty: 42, hypr: "sig".into() };
        assert_eq!(of_environ(env), Some(want));
        // macOS: no Hyprland, maybe no KITTY_PID
        let mac = b"KITTY_LISTEN_ON=unix:/tmp/kitty-1\0KITTY_WINDOW_ID=3\0";
        assert_eq!(
            of_environ(mac),
            Some(Terminal { to: "unix:/tmp/kitty-1".into(), window: 3, kitty: 0, hypr: "".into() })
        );
        assert_eq!(of_environ(b"KITTY_LISTEN_ON=x\0KITTY_WINDOW_ID=seven\0"), None);
    }
}
