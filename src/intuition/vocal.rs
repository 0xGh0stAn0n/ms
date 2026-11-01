//! Humming: MapleSyrup's own tunes, made into sound.
//!
//! No voice model hums on request (ElevenLabs marks singing experimental;
//! ChatGPT's voice is told never to), and a hummed game theme would be a
//! performance of Nexon's composition in a paid product. So the melodies
//! are MapleSyrup's own (`melodies.json`, each marked with its rights), and
//! the hum is synthesised here: a closed-mouth tone — the fundamental with
//! a few quiet harmonics, a little vibrato, soft edges, a breath of noise —
//! at the voice's own 24 kHz, so it plays through the same path as a line.
//! Zero inference, zero latency, and the owner (a pianist) writes the
//! leitmotifs.

use std::sync::OnceLock;

use serde::Deserialize;

const DATA: &str = include_str!("melodies.json");

/// A rest, in the data.
const REST: &str = "-";

/// A note: a pitch ("G4", "F#4", "Bb3") or a rest, and its length in beats.
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct Note(pub String, pub f32);

/// One of MapleSyrup's tunes.
#[derive(Debug, Clone, Deserialize)]
pub struct Melody {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub mood: Vec<String>,
    /// Whose it is: "original", or a licence's name.
    pub rights: String,
    pub bpm: f32,
    pub notes: Vec<Note>,
}

impl Melody {
    /// How long it plays, in seconds.
    pub fn seconds(&self) -> f32 {
        self.notes.iter().map(|n| n.1).sum::<f32>() * 60.0 / self.bpm
    }
}

/// The tunes, read once from `melodies.json`.
#[derive(Debug, Deserialize)]
pub struct Melodies {
    pub sample_rate: u32,
    melodies: Vec<Melody>,
}

impl Melodies {
    pub fn get() -> &'static Melodies {
        static MELODIES: OnceLock<Melodies> = OnceLock::new();
        MELODIES.get_or_init(|| {
            serde_json::from_str(DATA).unwrap_or_else(|e| panic!("melodies.json: {e}"))
        })
    }

    pub fn all(&self) -> &[Melody] {
        &self.melodies
    }

    pub fn named(&self, id: &str) -> Option<&Melody> {
        self.melodies.iter().find(|m| m.id == id)
    }
}

/// The frequency of a pitch name ("A4" is 440 Hz); None for a rest or a
/// name that is no pitch.
pub fn frequency(pitch: &str) -> Option<f32> {
    let pitch = pitch.trim();
    if pitch == REST || pitch.is_empty() {
        return None;
    }
    let mut chars = pitch.chars();
    let letter = chars.next()?.to_ascii_uppercase();
    let base: i32 = match letter {
        'C' => 0,
        'D' => 2,
        'E' => 4,
        'F' => 5,
        'G' => 7,
        'A' => 9,
        'B' => 11,
        _ => return None,
    };
    let rest: String = chars.collect();
    let (accidental, octave) = match rest.chars().next()? {
        '#' => (1, &rest[1..]),
        'b' => (-1, &rest[1..]),
        _ => (0, rest.as_str()),
    };
    let octave: i32 = octave.parse().ok()?;
    let midi = (octave + 1) * 12 + base + accidental;
    Some(440.0 * 2f32.powf((midi - 69) as f32 / 12.0))
}

/// The hum's harmonics: a closed mouth lets little past the fundamental.
const HARMONICS: [f32; 4] = [1.0, 0.32, 0.11, 0.04];
/// Vibrato: how fast, and how far (a fraction of the pitch).
const VIBRATO_HZ: f32 = 5.3;
const VIBRATO_DEPTH: f32 = 0.004;
/// How each note comes in and goes out, in seconds.
const ATTACK_S: f32 = 0.06;
const RELEASE_S: f32 = 0.09;
/// A breath of noise under the tone, as a fraction of it.
const BREATH: f32 = 0.012;
/// How loud, of full scale.
const GAIN: f32 = 0.28;

/// `melody` hummed: 16-bit mono samples at [`Melodies::sample_rate`].
pub fn hum(melody: &Melody) -> Vec<i16> {
    let rate = Melodies::get().sample_rate as f32;
    let beat = 60.0 / melody.bpm.max(20.0);
    let total: usize = melody
        .notes
        .iter()
        .map(|n| (n.1 * beat * rate) as usize)
        .sum();
    let mut out = vec![0f32; total + (RELEASE_S * rate) as usize];
    let mut at = 0usize;
    let mut phase = 0f32;
    let mut noise: u32 = 0x9E37_79B9;
    for note in &melody.notes {
        let len = (note.1 * beat * rate) as usize;
        if let Some(f) = frequency(&note.0) {
            let attack = (ATTACK_S * rate) as usize;
            let release = (RELEASE_S * rate) as usize;
            for i in 0..len + release {
                let t = i as f32 / rate;
                let env = if i < attack {
                    i as f32 / attack as f32
                } else if i >= len {
                    1.0 - (i - len) as f32 / release as f32
                } else {
                    1.0
                };
                let env = env.clamp(0.0, 1.0);
                let vibrato =
                    1.0 + VIBRATO_DEPTH * (2.0 * std::f32::consts::PI * VIBRATO_HZ * t).sin();
                phase += 2.0 * std::f32::consts::PI * f * vibrato / rate;
                if phase > 2.0 * std::f32::consts::PI {
                    phase -= 2.0 * std::f32::consts::PI;
                }
                let mut tone = 0.0;
                for (k, amp) in HARMONICS.iter().enumerate() {
                    tone += amp * (phase * (k as f32 + 1.0)).sin();
                }
                // A hum swells a little into each note.
                let swell = 0.85
                    + 0.15 * (std::f32::consts::PI * (i as f32 / len.max(1) as f32).min(1.0)).sin();
                noise ^= noise << 13;
                noise ^= noise >> 17;
                noise ^= noise << 5;
                let breath = ((noise as f32 / u32::MAX as f32) - 0.5) * BREATH;
                if let Some(sample) = out.get_mut(at + i) {
                    *sample += env * (tone * swell + breath);
                }
            }
        }
        at += len;
    }
    let peak = out.iter().fold(0f32, |m, s| m.max(s.abs())).max(1e-6);
    out.into_iter()
        .map(|s| (s / peak * GAIN * i16::MAX as f32) as i16)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_melodies_read_and_every_one_is_original_and_short() {
        let m = Melodies::get();
        assert_eq!(m.sample_rate, 24_000);
        assert!(m.all().len() >= 2);
        for melody in m.all() {
            assert_eq!(melody.rights, "original", "{}", melody.id);
            assert!(
                melody.seconds() > 2.0 && melody.seconds() < 20.0,
                "{}",
                melody.id
            );
            for note in &melody.notes {
                assert!(note.1 > 0.0, "{}: {:?}", melody.id, note);
                assert!(
                    note.0 == REST || frequency(&note.0).is_some(),
                    "{}: {:?}",
                    melody.id,
                    note
                );
            }
        }
        assert!(m.named("morning-leaf").is_some());
        assert!(m.named("ellinia-theme").is_none());
    }

    #[test]
    fn pitches_are_read_as_players_write_them() {
        assert!((frequency("A4").unwrap() - 440.0).abs() < 1e-3);
        assert!((frequency("a4").unwrap() - 440.0).abs() < 1e-3);
        assert!((frequency("C4").unwrap() - 261.63).abs() < 0.1);
        assert!((frequency("F#4").unwrap() - 369.99).abs() < 0.1);
        assert!((frequency("Bb3").unwrap() - 233.08).abs() < 0.1);
        assert_eq!(frequency("-"), None);
        assert_eq!(frequency("H4"), None);
        assert_eq!(frequency("C"), None);
    }

    #[test]
    fn a_hum_is_the_melodys_length_soft_and_in_tune() {
        let melody = Melodies::get().named("morning-leaf").unwrap();
        let samples = hum(melody);
        let rate = Melodies::get().sample_rate as f32;
        let expected = (melody.seconds() * rate) as usize;
        assert!(samples.len() >= expected && samples.len() < expected + rate as usize);
        let peak = samples.iter().map(|s| s.unsigned_abs()).max().unwrap();
        assert!(peak > 5_000 && peak < 12_000, "{peak}");
        // The first note (G4, ~392 Hz) shows as about that many zero
        // crossings per second once the attack has passed.
        let second = &samples[(0.1 * rate) as usize..(0.6 * rate) as usize];
        let crossings = second
            .windows(2)
            .filter(|w| (w[0] < 0) != (w[1] < 0))
            .count();
        let hz = crossings as f32 / 2.0 / 0.5;
        assert!((hz - 392.0).abs() < 25.0, "{hz}");
        // It starts from silence and ends in it.
        assert!(samples[0].unsigned_abs() < 200);
        assert!(samples.last().unwrap().unsigned_abs() < 400);
    }
}
