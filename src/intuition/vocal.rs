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
//! leitmotifs. A hum follows a thought, and the player's window to answer
//! opens when it ends, so a tune is a phrase of a few seconds
//! ([`LONGEST_S`]), not a song.
//!
//! A synthesised hum is a stand-in: a real one is a voice. So a recording
//! of the tune, hummed by a person, takes its place when there is one —
//! `hums/<melody id>.wav` under the settings folder ([`recorded`]): 16-bit
//! PCM, mono or stereo, any sample rate (brought to the voice's), trimmed
//! and levelled here. The owner hums the four phrases once, and MapleSyrup
//! hums in his voice.

use std::path::Path;
use std::sync::OnceLock;

use serde::Deserialize;

const DATA: &str = include_str!("melodies.json");

/// A rest, in the data.
const REST: &str = "-";
/// No tune runs longer than this, in seconds (the data is tested against
/// it): the hum sits between a thought and the player's chance to answer.
pub const LONGEST_S: f32 = 6.0;

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

/// The tune named `id`, hummed — made once, the first time any is asked
/// for, and kept: a hum is asked for from the frame loop, where a million
/// sines would be felt.
pub fn hummed(id: &str) -> Option<&'static [i16]> {
    static HUMMED: OnceLock<Vec<Vec<i16>>> = OnceLock::new();
    let melodies = Melodies::get();
    let at = melodies.melodies.iter().position(|m| m.id == id)?;
    let all = HUMMED.get_or_init(|| melodies.all().iter().map(hum).collect());
    all.get(at).map(Vec::as_slice)
}

/// Where recorded hums live under the settings folder.
pub const RECORDINGS_DIR: &str = "hums";
/// A recording longer than this is cut there, in seconds.
const RECORDING_AT_MOST_S: f32 = 8.0;
/// Below this, of full scale, the start and end of a recording is silence
/// to trim.
const SILENCE: f32 = 0.01;

/// A recorded hum of the melody `id` from `settings/hums/<id>.wav`, at the
/// voice's rate and level: None when there is none, or the file is not a
/// WAV this reads (then the tune is synthesised).
pub fn recorded(settings: &Path, id: &str) -> Option<Vec<i16>> {
    let path = settings.join(RECORDINGS_DIR).join(format!("{id}.wav"));
    let bytes = std::fs::read(path).ok()?;
    let (rate, channels, samples) = wav_pcm16(&bytes)?;
    // Mono.
    let mono: Vec<f32> = samples
        .chunks(channels.max(1))
        .map(|frame| frame.iter().map(|s| *s as f32).sum::<f32>() / frame.len() as f32)
        .collect();
    // At the voice's rate.
    let out_rate = Melodies::get().sample_rate as f32;
    let step = rate as f32 / out_rate;
    let n = ((mono.len() as f32 / step) as usize).min((RECORDING_AT_MOST_S * out_rate) as usize);
    let mut resampled: Vec<f32> = (0..n)
        .map(|i| {
            let at = i as f32 * step;
            let (a, b) = (
                at.floor() as usize,
                (at.floor() as usize + 1).min(mono.len() - 1),
            );
            let frac = at - at.floor();
            mono[a.min(mono.len() - 1)] * (1.0 - frac) + mono[b] * frac
        })
        .collect();
    // Trimmed of silence at either end, levelled to the synthesised hum's
    // peak.
    let peak = resampled.iter().fold(0f32, |m, s| m.max(s.abs()));
    if peak < 1.0 {
        return None;
    }
    let loud = |s: &f32| s.abs() / peak >= SILENCE;
    let start = resampled.iter().position(loud)?;
    let end = resampled.iter().rposition(loud)? + 1;
    resampled.drain(end..);
    resampled.drain(..start);
    if resampled.len() < (0.3 * out_rate) as usize {
        return None;
    }
    let scale = GAIN * i16::MAX as f32 / peak;
    Some(resampled.into_iter().map(|s| (s * scale) as i16).collect())
}

/// A WAV's sample rate, channels and 16-bit PCM samples, from its RIFF
/// chunks (whatever else is in the file is passed over); None for anything
/// that is not 16-bit PCM.
fn wav_pcm16(bytes: &[u8]) -> Option<(u32, usize, Vec<i16>)> {
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return None;
    }
    let (mut rate, mut channels, mut bits, mut format) = (0u32, 0usize, 0u16, 0u16);
    let mut data: Option<&[u8]> = None;
    let mut at = 12;
    while at + 8 <= bytes.len() {
        let id = &bytes[at..at + 4];
        let len = u32::from_le_bytes([bytes[at + 4], bytes[at + 5], bytes[at + 6], bytes[at + 7]])
            as usize;
        let body = bytes.get(at + 8..(at + 8 + len).min(bytes.len()))?;
        match id {
            b"fmt " if body.len() >= 16 => {
                format = u16::from_le_bytes([body[0], body[1]]);
                channels = u16::from_le_bytes([body[2], body[3]]) as usize;
                rate = u32::from_le_bytes([body[4], body[5], body[6], body[7]]);
                bits = u16::from_le_bytes([body[14], body[15]]);
            }
            b"data" => data = Some(body),
            _ => {}
        }
        // (Chunks are padded to an even length.)
        at += 8 + len + (len & 1);
    }
    let data = data?;
    if format != 1 || bits != 16 || channels == 0 || rate == 0 {
        return None;
    }
    let samples = data
        .as_chunks::<2>()
        .0
        .iter()
        .map(|b| i16::from_le_bytes(*b))
        .collect();
    Some((rate, channels, samples))
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
/// Vibrato: how fast, how far (a fraction of the pitch), and how long into
/// a note it takes to come (a voice settles on the note first).
const VIBRATO_HZ: f32 = 5.3;
const VIBRATO_DEPTH: f32 = 0.004;
const VIBRATO_AFTER_S: f32 = 0.18;
/// A voice slides from one note to the next, in seconds.
const GLIDE_S: f32 = 0.05;
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
    let mut previous: Option<f32> = None;
    for note in &melody.notes {
        let len = (note.1 * beat * rate) as usize;
        if let Some(f) = frequency(&note.0) {
            let attack = (ATTACK_S * rate) as usize;
            let release = (RELEASE_S * rate) as usize;
            let glide = (GLIDE_S * rate) as usize;
            let from = previous.unwrap_or(f);
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
                // The pitch slides in from the last note, and the vibrato
                // comes once the voice has settled.
                let pitch = if i < glide {
                    from + (f - from) * i as f32 / glide as f32
                } else {
                    f
                };
                let depth = VIBRATO_DEPTH * ((t - VIBRATO_AFTER_S) / 0.1).clamp(0.0, 1.0);
                let vibrato = 1.0 + depth * (2.0 * std::f32::consts::PI * VIBRATO_HZ * t).sin();
                phase += 2.0 * std::f32::consts::PI * pitch * vibrato / rate;
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
            previous = Some(f);
        } else {
            previous = None;
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
                melody.seconds() > 2.0 && melody.seconds() <= LONGEST_S,
                "{}: {:.1} s",
                melody.id,
                melody.seconds()
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
    fn a_recorded_hum_takes_the_tunes_place_at_the_voices_rate_trimmed_and_levelled() {
        let dir =
            std::env::temp_dir().join(format!("ms-hums-{}", crate::phone::tls::random_hex(4)));
        std::fs::create_dir_all(dir.join(RECORDINGS_DIR)).unwrap();
        // A stereo 48 kHz recording: half a second of silence, a 220 Hz
        // tone for two seconds, silence again — and a LIST chunk before the
        // data, as editors write.
        let rate = 48_000u32;
        let mut pcm: Vec<i16> = Vec::new();
        for i in 0..(rate as usize * 3) {
            let t = i as f32 / rate as f32;
            let v = if (0.5..2.5).contains(&t) {
                ((2.0 * std::f32::consts::PI * 220.0 * t).sin() * 12_000.0) as i16
            } else {
                0
            };
            pcm.push(v);
            pcm.push(v / 2);
        }
        let mut data = Vec::new();
        for s in &pcm {
            data.extend_from_slice(&s.to_le_bytes());
        }
        let mut wav = Vec::new();
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&0u32.to_le_bytes());
        wav.extend_from_slice(b"WAVE");
        wav.extend_from_slice(b"fmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&2u16.to_le_bytes());
        wav.extend_from_slice(&rate.to_le_bytes());
        wav.extend_from_slice(&(rate * 4).to_le_bytes());
        wav.extend_from_slice(&4u16.to_le_bytes());
        wav.extend_from_slice(&16u16.to_le_bytes());
        wav.extend_from_slice(b"LIST");
        wav.extend_from_slice(&5u32.to_le_bytes());
        wav.extend_from_slice(b"INFOx\0");
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&(data.len() as u32).to_le_bytes());
        wav.extend_from_slice(&data);
        std::fs::write(dir.join(RECORDINGS_DIR).join("morning-leaf.wav"), &wav).unwrap();
        let samples = recorded(&dir, "morning-leaf").expect("read");
        let out_rate = Melodies::get().sample_rate as f32;
        // Two seconds at 24 kHz, give or take the trim.
        assert!(
            (samples.len() as f32 / out_rate - 2.0).abs() < 0.05,
            "{}",
            samples.len()
        );
        // 220 Hz: that many zero crossings per second.
        let crossings = samples
            .windows(2)
            .filter(|w| (w[0] < 0) != (w[1] < 0))
            .count();
        assert!(
            (crossings as f32 / 2.0 / 2.0 - 220.0).abs() < 10.0,
            "{crossings}"
        );
        // Levelled like the synthesised hum.
        let peak = samples.iter().map(|s| s.unsigned_abs()).max().unwrap();
        assert!(
            (peak as f32 - GAIN * i16::MAX as f32).abs() < 300.0,
            "{peak}"
        );
        // No recording, or a file that is not a WAV: none.
        assert!(recorded(&dir, "harbor-swing").is_none());
        std::fs::write(
            dir.join(RECORDINGS_DIR).join("harbor-swing.wav"),
            b"not a wav",
        )
        .unwrap();
        assert!(recorded(&dir, "harbor-swing").is_none());
        let _ = std::fs::remove_dir_all(&dir);
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
        // The kept one is the same sound, and a name that is no tune is
        // none.
        assert_eq!(hummed("morning-leaf"), Some(samples.as_slice()));
        assert!(hummed("ellinia-theme").is_none());
    }
}
