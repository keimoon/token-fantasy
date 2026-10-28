//! What survives a restart: sound on or off, its volume, and the pace
//! (Wait or Active). A small JSON file in
//! $XDG_CONFIG_HOME/token-fantasy-desktop/ (else ~/.config/token-fantasy-desktop/).

use std::fs;
use std::path::PathBuf;

use serde_json::{Value, json};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Settings {
    pub muted: bool,
    /// 0..100
    pub volume: u8,
    /// Active pace: monsters also hit sessions waiting on you.
    pub pace_active: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings { muted: false, volume: 60, pace_active: false }
    }
}

fn path() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap_or_else(|| "/".into())).join(".config"))
        .join("token-fantasy-desktop")
        .join("settings.json")
}

impl Settings {
    /// The saved settings, defaults for anything never saved.
    pub fn load() -> Settings {
        let d = Settings::default();
        let Some(v) = fs::read(path()).ok().and_then(|raw| serde_json::from_slice::<Value>(&raw).ok()) else {
            return d;
        };
        Settings {
            muted: v.get("muted").and_then(Value::as_bool).unwrap_or(d.muted),
            volume: v.get("volume").and_then(Value::as_u64).map_or(d.volume, |n| n.min(100) as u8),
            pace_active: v.get("pace").and_then(Value::as_str).map_or(d.pace_active, |s| s == "active"),
        }
    }

    pub fn save(&self) {
        let doc = json!({
            "muted": self.muted,
            "volume": self.volume,
            "pace": if self.pace_active { "active" } else { "wait" },
        });
        let p = path();
        let tmp = p.with_extension("json.tmp");
        let ok = p.parent().is_some_and(|d| fs::create_dir_all(d).is_ok()) && fs::write(&tmp, doc.to_string()).is_ok();
        if !ok || fs::rename(&tmp, &p).is_err() {
            eprintln!("token-fantasy: cannot save {}", p.display());
        }
    }
}
