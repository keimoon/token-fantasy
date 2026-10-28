//! The little of /proc the collector needs. Linux only.

use std::fs;
use std::path::PathBuf;

/// Field 22 of /proc/<pid>/stat (start time in clock ticks since boot), or
/// None if the process is gone.
pub fn start_ticks(pid: u32) -> Option<String> {
    let s = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let rest = s.get(s.rfind(')')? + 2..)?;
    rest.split_whitespace().nth(19).map(str::to_owned)
}

/// When the machine booted, epoch seconds.
pub fn boot_time() -> f64 {
    let s = fs::read_to_string("/proc/stat").unwrap_or_default();
    s.lines().find_map(|l| l.strip_prefix("btime ")?.trim().parse::<i64>().ok()).unwrap_or(0) as f64
}

pub fn clk_tck() -> f64 {
    // SAFETY: sysconf has no preconditions.
    let v = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
    if v > 0 { v as f64 } else { 100.0 }
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
        assert!(start_ticks(me).is_some_and(|t| t.parse::<u64>().is_ok()));
        assert!(boot_time() > 0.0);
        assert!(pids().contains(&me));
        assert!(start_ticks(u32::MAX).is_none());
    }
}
