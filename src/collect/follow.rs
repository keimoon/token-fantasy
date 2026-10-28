//! Growing JSONL files (Claude transcripts, Codex rollouts): the first look
//! reads back from the end, later looks only what was appended.

use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Local, NaiveDateTime, TimeZone};
use serde_json::Value;

/// A long turn writes megabytes (377 KB of a Codex rollout in 11 minutes was
/// seen), so no fixed tail is enough: the first look at a file scans back
/// from its end until it has what it needs, later looks read only what was
/// appended.
const SCAN_CHUNK: u64 = 256 * 1024;

/// One followed file: where reading stopped and what was made of it.
pub struct Follow<S> {
    pub offset: u64,
    pub state: S,
}

/// -> (pick's first hit going back from the end, or None; the offset just
/// past the last complete line).
pub fn scan_back<T>(f: &mut File, size: u64, mut pick: impl FnMut(&[u8]) -> Option<T>) -> io::Result<(Option<T>, u64)> {
    let start0 = size.saturating_sub(SCAN_CHUNK);
    f.seek(SeekFrom::Start(start0))?;
    let mut tail = Vec::new();
    f.by_ref().take(size - start0).read_to_end(&mut tail)?;
    let done = memchr::memrchr(b'\n', &tail).map_or(0, |nl| start0 + nl as u64 + 1);
    let (mut pos, mut carry) = (done, Vec::new());
    while pos > 0 {
        let start = pos.saturating_sub(SCAN_CHUNK);
        f.seek(SeekFrom::Start(start))?;
        let mut chunk = vec![0; (pos - start) as usize];
        f.read_exact(&mut chunk)?;
        chunk.extend_from_slice(&carry);
        // a line cut at `start` is finished with the next chunk back
        let rest = if start > 0 {
            match memchr::memchr(b'\n', &chunk) {
                Some(i) => {
                    carry = chunk[..i].to_vec();
                    &chunk[i + 1..]
                }
                None => {
                    carry = chunk.clone();
                    &chunk[chunk.len()..]
                }
            }
        } else {
            &chunk[..]
        };
        if let Some(hit) = rest.split(|&b| b == b'\n').rev().find_map(&mut pick) {
            return Ok((Some(hit), done));
        }
        pos = start;
    }
    Ok((None, done))
}

/// What `cache` keeps for one growing JSONL file: `first` makes it on the
/// first look (or after the file shrank) and says where it stopped reading,
/// `step` folds in each complete line appended since. None if the file
/// cannot be read.
pub fn follow<'a, S>(
    path: &Path,
    cache: &'a mut HashMap<PathBuf, Follow<S>>,
    first: impl FnOnce(&mut File, u64) -> io::Result<(S, u64)>,
    mut step: impl FnMut(&[u8], &mut S),
) -> Option<&'a mut S> {
    let size = fs::metadata(path).ok()?.len();
    let mut f = File::open(path).ok()?;
    if cache.get(path).is_none_or(|r| size < r.offset) {
        let (state, offset) = first(&mut f, size).ok()?;
        cache.insert(path.to_path_buf(), Follow { offset, state });
    } else {
        let r = cache.get_mut(path)?;
        if size > r.offset {
            f.seek(SeekFrom::Start(r.offset)).ok()?;
            let mut data = Vec::new();
            f.take(size - r.offset).read_to_end(&mut data).ok()?;
            // leave a half-written last line for next time
            let end = memchr::memrchr(b'\n', &data).map_or(0, |i| i + 1);
            for raw in data[..end].split(|&b| b == b'\n') {
                step(raw, &mut r.state);
            }
            r.offset += end as u64;
        }
    }
    cache.get_mut(path).map(|r| &mut r.state)
}

/// An ISO-8601 timestamp as epoch seconds (one without a zone is local time).
pub fn epoch(s: &str) -> Option<f64> {
    let secs = |t: i64, ns: u32| t as f64 + ns as f64 / 1e9;
    match DateTime::parse_from_rfc3339(s) {
        Ok(dt) => Some(secs(dt.timestamp(), dt.timestamp_subsec_nanos())),
        Err(_) => {
            let naive = NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S%.f").ok()?;
            let dt = Local.from_local_datetime(&naive).earliest()?;
            Some(secs(dt.timestamp(), dt.timestamp_subsec_nanos()))
        }
    }
}

/// A line's "timestamp" as epoch seconds.
pub fn parse_ts(v: Option<&Value>) -> Option<f64> {
    epoch(v?.as_str()?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collect::TestDir;
    use std::io::Write;

    fn number(raw: &[u8]) -> Option<u64> {
        std::str::from_utf8(raw).ok()?.strip_prefix("n=")?.parse().ok()
    }

    #[test]
    fn scans_back_across_chunks_to_the_last_hit() {
        let dir = TestDir::new();
        let path = dir.0.join("long.jsonl");
        // one hit near the start, then 600 KB of other lines and a half-written one
        let mut text = String::from("x\nn=7\n");
        let filler = format!("{}\n", "y".repeat(999));
        for _ in 0..600 {
            text.push_str(&filler);
        }
        text.push_str("n=8"); // no newline yet: not a line
        fs::write(&path, &text).unwrap();
        let size = text.len() as u64;
        let mut f = File::open(&path).unwrap();
        let (hit, done) = scan_back(&mut f, size, number).unwrap();
        assert_eq!(hit, Some(7));
        assert_eq!(done, size - 3);
        let (miss, _) = scan_back(&mut f, size, |raw| (raw == b"z").then_some(())).unwrap();
        assert!(miss.is_none());
    }

    #[test]
    fn follows_appended_lines_and_starts_over_when_the_file_shrinks() {
        let dir = TestDir::new();
        let path = dir.0.join("grow.jsonl");
        fs::write(&path, "n=1\nn=2\n").unwrap();
        let mut cache: HashMap<PathBuf, Follow<Vec<u64>>> = HashMap::new();
        let first = |f: &mut File, size: u64| {
            let (hit, done) = scan_back(f, size, number)?;
            Ok((hit.into_iter().collect(), done))
        };
        let step = |raw: &[u8], s: &mut Vec<u64>| s.extend(number(raw));
        assert_eq!(follow(&path, &mut cache, first, step).unwrap(), &vec![2]);
        let mut file = fs::OpenOptions::new().append(true).open(&path).unwrap();
        write!(file, "n=3\nn=4").unwrap();
        assert_eq!(follow(&path, &mut cache, first, step).unwrap(), &vec![2, 3]);
        writeln!(file).unwrap();
        assert_eq!(follow(&path, &mut cache, first, step).unwrap(), &vec![2, 3, 4]);
        fs::write(&path, "n=9\n").unwrap();
        assert_eq!(follow(&path, &mut cache, first, step).unwrap(), &vec![9]);
        fs::remove_file(&path).unwrap();
        assert!(follow(&path, &mut cache, first, step).is_none());
    }

    #[test]
    fn parses_timestamps() {
        assert_eq!(epoch("2026-10-12T01:00:00Z"), Some(1791766800.0));
        assert_eq!(epoch("2026-10-12T01:00:00.250+00:00"), Some(1791766800.25));
        assert!(epoch("2026-10-12T01:00:00").is_some());
        assert_eq!(epoch("soon"), None);
        assert_eq!(parse_ts(Some(&Value::from(5))), None);
    }
}
