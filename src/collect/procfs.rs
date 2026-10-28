//! The little of /proc the collectors need, on Linux. The same functions as
//! macos.rs.

use std::fs;
use std::path::PathBuf;

use serde_json::Value;

/// Field 22 of /proc/<pid>/stat (start time in clock ticks since boot), or
/// None if the process is gone.
fn start_ticks(pid: u32) -> Option<String> {
    let s = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let rest = s.get(s.rfind(')')? + 2..)?;
    rest.split_whitespace().nth(19).map(str::to_owned)
}

/// When the machine booted, epoch seconds.
fn boot_time() -> f64 {
    let s = fs::read_to_string("/proc/stat").unwrap_or_default();
    s.lines().find_map(|l| l.strip_prefix("btime ")?.trim().parse::<i64>().ok()).unwrap_or(0) as f64
}

fn clk_tck() -> f64 {
    // SAFETY: sysconf has no preconditions.
    let v = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
    if v > 0 { v as f64 } else { 100.0 }
}

/// The process's start, as text that changes when the pid is reused: its
/// start in clock ticks since boot.
pub fn identity(pid: u32) -> Option<String> {
    start_ticks(pid)
}

/// Process start as epoch seconds, or 0 if unknown.
pub fn start_epoch(pid: u32) -> f64 {
    start_ticks(pid).and_then(|t| t.parse::<f64>().ok()).map_or(0.0, |t| boot_time() + t / clk_tck())
}

/// The identity of the live process that wrote a Claude session file, else
/// None (dead, or the pid was reused). On Linux Claude writes the start
/// ticks as procStart.
pub fn alive(pid: u32, session: &Value) -> Option<String> {
    super::same_start(start_ticks(pid), session)
}

/// The environment as NUL-separated NAME=value entries.
pub fn environ(pid: u32) -> Option<Vec<u8>> {
    fs::read(format!("/proc/{pid}/environ")).ok()
}

pub fn pids() -> Vec<u32> {
    let Ok(rd) = fs::read_dir("/proc") else { return Vec::new() };
    rd.flatten().filter_map(|e| e.file_name().to_str()?.parse().ok()).collect()
}

pub fn exe(pid: u32) -> Option<PathBuf> {
    fs::read_link(format!("/proc/{pid}/exe")).ok()
}

pub fn cwd(pid: u32) -> Option<PathBuf> {
    fs::read_link(format!("/proc/{pid}/cwd")).ok()
}

/// argv without argv[0], empty arguments dropped.
pub fn args(pid: u32) -> Option<Vec<String>> {
    let raw = fs::read(format!("/proc/{pid}/cmdline")).ok()?;
    Some(
        raw.split(|&b| b == 0)
            .skip(1)
            .filter(|a| !a.is_empty())
            .map(|a| String::from_utf8_lossy(a).into_owned())
            .collect(),
    )
}

/// Where the process's open file descriptors point.
pub fn open_files(pid: u32) -> Vec<PathBuf> {
    let Ok(rd) = fs::read_dir(format!("/proc/{pid}/fd")) else { return Vec::new() };
    rd.flatten().filter_map(|e| fs::read_link(e.path()).ok()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_this_process() {
        let me = std::process::id();
        assert!(identity(me).is_some_and(|t| t.parse::<u64>().is_ok()));
        assert!(boot_time() > 0.0);
        assert!(start_epoch(me) > boot_time());
        assert!(pids().contains(&me));
        assert!(environ(me).is_some_and(|e| !e.is_empty()));
        assert!(identity(u32::MAX).is_none());
        assert_eq!(start_epoch(u32::MAX), 0.0);
    }
}
