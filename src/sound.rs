//! The battle's sounds, synthesized: the chimes when a session starts waiting
//! ("!" three notes, "?" two), the menu blips, the victory fanfare's opening
//! when a monster is defeated, and the Phoenix Plume's rising arpeggio.
//!
//! Each sound is rendered once to a WAV file under $XDG_RUNTIME_DIR (else
//! the temp dir) and played by the desktop's player, `pw-play`, `paplay` or
//! `aplay` (Linux) or `afplay` (macOS), whichever is installed, on a thread
//! of its own.

use std::f32::consts::PI;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::thread;

const RATE: u32 = 44100;
/// The longest sound, the fanfare.
const MAX_MS: u32 = 2600;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sound {
    /// Someone needs your OK.
    Perm,
    /// Someone has a question.
    Ask,
    /// A click opened or stepped something.
    Blip,
    /// A click closed something.
    BlipLow,
    /// A monster was defeated.
    Fanfare,
    /// A Phoenix Plume.
    Revive,
}

impl Sound {
    pub const ALL: [Sound; 6] = [Sound::Perm, Sound::Ask, Sound::Blip, Sound::BlipLow, Sound::Fanfare, Sound::Revive];

    pub fn name(self) -> &'static str {
        ["perm", "ask", "blip", "close", "fanfare", "revive"][self as usize]
    }

    /// (frequency, start ms, length ms, amplitude) per note.
    fn notes(self) -> &'static [(f32, u32, u32, f32)] {
        const C: f32 = 1046.5;
        const AB: f32 = 830.61;
        const BB: f32 = 932.33;
        match self {
            // E5 A5 E6
            Sound::Perm => &[(659.25, 0, 420, 0.38), (880.0, 110, 420, 0.38), (1318.5, 220, 520, 0.34)],
            // G5 C6
            Sound::Ask => &[(783.99, 0, 380, 0.36), (1046.5, 150, 480, 0.34)],
            // E6 A6, a menu cursor's "pi-kon"
            Sound::Blip => &[(1318.5, 0, 150, 0.45), (1760.0, 60, 180, 0.42)],
            // A5 E5, falling
            Sound::BlipLow => &[(880.0, 0, 150, 0.45), (659.25, 60, 180, 0.45)],
            // C C C C, Ab Bb, C Bb C: triplets, then the long notes
            Sound::Fanfare => &[
                (C, 0, 120, 0.34),
                (C, 125, 120, 0.34),
                (C, 250, 120, 0.34),
                (C, 375, 360, 0.36),
                (AB, 750, 360, 0.34),
                (BB, 1125, 360, 0.34),
                (C, 1500, 220, 0.36),
                (BB, 1750, 120, 0.32),
                (C, 1875, 700, 0.38),
            ],
            // C6 E6 G6 C7, quick
            Sound::Revive => {
                &[(1046.5, 0, 220, 0.3), (1318.5, 70, 220, 0.3), (1568.0, 140, 220, 0.3), (2093.0, 210, 420, 0.28)]
            }
        }
    }
}

/// A chiptune-ish bell: sine plus a little third harmonic, 3 ms attack and
/// an exponential decay over the note, so overlapping notes ring into each
/// other. Mono 16-bit samples at RATE, scaled by volume (0..100).
pub fn synth(sound: Sound, volume: u8) -> Vec<i16> {
    let notes = sound.notes();
    let end = notes.iter().map(|n| n.1 + n.2).max().unwrap_or(0).min(MAX_MS);
    let frames = (RATE * end / 1000) as usize;
    let mut mix = vec![0f32; frames];
    let (att, fade) = ((RATE * 3 / 1000) as usize, (RATE / 100) as usize);
    for &(hz, at, ms, amp) in notes {
        let (s0, len) = ((RATE * at / 1000) as usize, (RATE * ms / 1000) as usize);
        let (w, tau) = (2.0 * PI * hz / RATE as f32, len as f32 / 4.0);
        for i in 0..len.min(frames.saturating_sub(s0)) {
            let mut env = if i < att { i as f32 / att as f32 } else { 1.0 } * (-(i as f32) / tau).exp();
            if len - i < fade {
                env *= (len - i) as f32 / fade as f32;
            }
            let wi = w * i as f32;
            mix[s0 + i] += amp * env * (wi.sin() + 0.25 * (3.0 * wi).sin());
        }
    }
    let gain = 26000.0 * volume.min(100) as f32 / 100.0;
    mix.into_iter().map(|v| (v.clamp(-1.0, 1.0) * gain) as i16).collect()
}

/// A mono 16-bit WAV file of the samples.
pub fn wav(samples: &[i16]) -> Vec<u8> {
    let data = (samples.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + data as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // mono
    out.extend_from_slice(&RATE.to_le_bytes());
    out.extend_from_slice(&(RATE * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data.to_le_bytes());
    for s in samples {
        out.extend_from_slice(&s.to_le_bytes());
    }
    out
}

/// The first of the players that is installed.
fn player() -> Option<&'static str> {
    static PLAYER: OnceLock<Option<&'static str>> = OnceLock::new();
    *PLAYER.get_or_init(|| {
        let path = std::env::var_os("PATH").unwrap_or_default();
        ["pw-play", "paplay", "aplay", "afplay"]
            .into_iter()
            .find(|p| std::env::split_paths(&path).any(|d| d.join(p).is_file()))
    })
}

fn dir() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("token-fantasy-desktop")
}

/// Plays the sounds at one volume; the WAV files are written on first use.
pub struct Speaker {
    volume: u8,
}

impl Speaker {
    pub fn new(volume: u8) -> Self {
        Speaker { volume: volume.min(100) }
    }

    /// Play a sound without waiting for it; errors go to stderr once per sound.
    pub fn play(&self, sound: Sound) {
        let Some(player) = player() else { return };
        let path = dir().join(format!("{}-{}.wav", sound.name(), self.volume));
        let volume = self.volume;
        thread::spawn(move || {
            if !path.exists() {
                let ok =
                    std::fs::create_dir_all(dir()).is_ok() && std::fs::write(&path, wav(&synth(sound, volume))).is_ok();
                if !ok {
                    eprintln!("token-fantasy: cannot write {}", path.display());
                    return;
                }
            }
            let run = Command::new(player)
                .arg(&path)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
            if let Err(e) = run {
                eprintln!("token-fantasy: {player}: {e}");
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sounds_have_their_length_and_stay_in_range() {
        let s = synth(Sound::Fanfare, 100);
        assert_eq!(s.len(), (RATE * 2575 / 1000) as usize);
        assert!(s.iter().any(|&v| v.abs() > 5000));
        assert!(synth(Sound::Blip, 0).iter().all(|&v| v == 0));
        let w = wav(&synth(Sound::Ask, 50));
        assert_eq!(&w[..4], b"RIFF");
        assert_eq!(w.len(), 44 + synth(Sound::Ask, 50).len() * 2);
    }
}
