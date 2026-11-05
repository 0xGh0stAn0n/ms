//! The session's intuition written down as numbers, and replayed.
//!
//! Everything the director decides from is a [`World`] per frame and a few
//! events (a thought from the mind, the player's words, the voice done,
//! the dial moved), none of it words of the player's. Written to
//! `intuition.jsonl` in the session's folder as it happens, a session can
//! be run again here with today's `initiative.json`, `tempo.json` and
//! `hooks.json` — and without the game, the model or the voice — to see
//! when it would speak now, and why it held its tongue the rest of the
//! time. That is how the data is tuned: change a number, replay last
//! week's sessions, read the table (`intuition_replay`), and only then
//! play. The mind's thoughts are replayed as they were made (the model's
//! part is not re-run), so a replay answers "when would the director say
//! these?", not "what would the mind think?".
//!
//! A frame is written at most once a second, and sooner when what the
//! trail and the director read changed (the game seen, the map, the level,
//! a death, who is talking, muted, held): the replay crosses the same
//! boundaries the session did. A trace is about a hundred bytes a second.

use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::Path;

use serde::{Deserialize, Serialize};

use super::director::{Hold, Stats};
use super::thoughts::Thought;
use super::world::World;
use super::{Act, Intuition};

/// The trace's file name in the session's folder.
pub const FILE: &str = "intuition.jsonl";
/// A frame is written at least this far after the last, in seconds, unless
/// something the director reads changed.
pub const FRAME_EVERY_S: f64 = 1.0;
/// In a replay, the voice is taken to be done with a thought this long
/// after it was decided (a line and a hum): the player's window opens then.
pub const REPLAY_VOICE_S: f64 = 4.0;

/// What happened, as the director saw it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "what", content = "data", rename_all = "snake_case")]
pub enum Event {
    /// The moment (before the intuition looked at it).
    World(World),
    /// The mind came back with this thought.
    Thought(Thought),
    /// The mind was asked (what the session did; the replay cues on its
    /// own, this is for the comparison).
    Think { trigger: String, hooks: Vec<String> },
    /// The session said this thought (for the comparison).
    Said { hook: String },
    /// The voice was done with it at `at` (which may be ahead of the entry's
    /// time: what was queued had still to play).
    Voiced { at: f64 },
    /// The thought never made a sound.
    Unsaid,
    /// The player said something.
    PlayerSpoke,
    /// The player asked for quiet.
    Stop,
    /// The dial moved to this level.
    Dial(String),
}

/// One line of the trace.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    /// Session seconds.
    pub t: f64,
    #[serde(flatten)]
    pub event: Event,
}

/// Writes the trace as the session goes.
#[derive(Debug)]
pub struct Tracer {
    file: Option<BufWriter<File>>,
    last: Option<World>,
    last_at: f64,
}

impl Tracer {
    /// A tracer writing to [`FILE`] under `dir` (none when the file cannot
    /// be opened: the session goes on without a trace).
    pub fn open(dir: &Path) -> Tracer {
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(dir.join(FILE))
            .ok()
            .map(BufWriter::new);
        Tracer {
            file,
            last: None,
            last_at: f64::NEG_INFINITY,
        }
    }

    /// A tracer that writes nowhere.
    pub fn off() -> Tracer {
        Tracer {
            file: None,
            last: None,
            last_at: f64::NEG_INFINITY,
        }
    }

    /// Whether anything is written.
    pub fn is_on(&self) -> bool {
        self.file.is_some()
    }

    /// The frame, if it is time or something the director reads changed.
    pub fn world(&mut self, world: &World) {
        if self.file.is_none() {
            return;
        }
        let changed = match &self.last {
            None => true,
            Some(last) => {
                last.seen != world.seen
                    || last.place.as_ref().map(|p| &p.name) != world.place.as_ref().map(|p| &p.name)
                    || last.level != world.level
                    || last.dead != world.dead
                    || last.talking != world.talking
                    || last.muted != world.muted
                    || last.held != world.held
                    || last.scene_cut != world.scene_cut
            }
        };
        if !changed && world.now - self.last_at < FRAME_EVERY_S {
            return;
        }
        self.last = Some(world.clone());
        self.last_at = world.now;
        self.write(world.now, Event::World(world.clone()));
    }

    /// An event at `t`.
    pub fn event(&mut self, t: f64, event: Event) {
        self.write(t, event);
    }

    fn write(&mut self, t: f64, event: Event) {
        let Some(file) = self.file.as_mut() else {
            return;
        };
        if let Ok(line) = serde_json::to_string(&Entry { t, event }) {
            let _ = writeln!(file, "{line}");
            // (A crash loses at most the frame under way.)
            let _ = file.flush();
        }
    }
}

/// The trace at `path`, read whole. A line that does not parse is skipped
/// (a crash mid-write leaves one).
pub fn read(path: &Path) -> Result<Vec<Entry>, String> {
    let file = File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut entries = Vec::new();
    for line in BufReader::new(file).lines() {
        let line = line.map_err(|e| e.to_string())?;
        if line.trim().is_empty() {
            continue;
        }
        if let Ok(entry) = serde_json::from_str::<Entry>(&line) {
            entries.push(entry);
        }
    }
    Ok(entries)
}

/// What a thought came to in the session, or would in the replay.
#[derive(Debug, Clone, PartialEq)]
pub struct Line {
    pub t: f64,
    pub hook: String,
    pub text: String,
}

/// A session run again with today's data.
#[derive(Debug, Default)]
pub struct Replayed {
    /// How long the trace ran, in seconds.
    pub seconds: f64,
    /// What the replay would have said, and when.
    pub would_say: Vec<Line>,
    /// What the session did say.
    pub did_say: Vec<Line>,
    /// How often the replay asked the mind, and how often the session did.
    pub would_think: u32,
    pub did_think: u32,
    /// Seconds spent in each hold (why nothing was said), over the frames.
    pub holds: BTreeMap<Hold, f64>,
    /// What the replay's thoughts came to, with the player's words as they
    /// came in the session.
    pub stats: Stats,
}

impl Replayed {
    /// The replay as a table, for the terminal.
    pub fn report(&self) -> String {
        let mut out = String::new();
        let minutes = self.seconds / 60.0;
        out.push_str(&format!(
            "{minutes:.0} minutes. The session said {} thought(s) and asked the mind {} time(s); \
with today's data it would say {} and ask {}.\n",
            self.did_say.len(),
            self.did_think,
            self.would_say.len(),
            self.would_think
        ));
        let s = self.stats;
        out.push_str(&format!(
            "Of what it would say: {} answered, {} let pass, {} shushed (by the player's words as they came).\n\n",
            s.engaged, s.ignored, s.rejected
        ));
        out.push_str("| when | the session said | the replay would say |\n|---|---|---|\n");
        let mut did = self.did_say.iter().peekable();
        let mut would = self.would_say.iter().peekable();
        while did.peek().is_some() || would.peek().is_some() {
            let (d, w) = (did.peek().map(|l| l.t), would.peek().map(|l| l.t));
            // The two columns in time order; within a few seconds of each
            // other they share a row.
            let row = match (d, w) {
                (Some(d), Some(w)) if (d - w).abs() <= 15.0 => (did.next(), would.next()),
                (Some(d), Some(w)) if d < w => (did.next(), None),
                (Some(_), None) => (did.next(), None),
                _ => (None, would.next()),
            };
            let t = row.0.or(row.1).map(|l| l.t).unwrap_or(0.0);
            let cell = |l: Option<&Line>| {
                l.map(|l| format!("{} — {}", l.hook, l.text))
                    .unwrap_or_default()
            };
            out.push_str(&format!(
                "| {:02}:{:02} | {} | {} |\n",
                (t / 60.0) as u32,
                (t % 60.0) as u32,
                cell(row.0),
                cell(row.1)
            ));
        }
        out.push_str("\nWhy nothing was said, by seconds:\n");
        let mut holds: Vec<(&Hold, &f64)> = self.holds.iter().collect();
        holds.sort_by(|a, b| b.1.total_cmp(a.1));
        for (hold, secs) in holds {
            out.push_str(&format!(
                "- {}: {:.0} s ({:.0}%)\n",
                hold.word(),
                secs,
                100.0 * secs / self.seconds.max(1.0)
            ));
        }
        out
    }
}

/// `entries` run again through a fresh intuition at `level` (None: the
/// session's own dial moves, as traced; a level: that one throughout) with
/// `seed` for the odds. The mind's thoughts are replayed as made; the
/// player's words as they came.
pub fn replay(entries: &[Entry], level: Option<&str>, seed: u64) -> Replayed {
    let mut out = Replayed::default();
    let first = entries.first().map(|e| e.t).unwrap_or(0.0);
    let mut intuition = Intuition::new(true, level, seed);
    let mut last_frame_at: Option<f64> = None;
    for entry in entries {
        out.seconds = out.seconds.max(entry.t - first);
        match &entry.event {
            Event::World(world) => {
                for act in intuition.observe(world) {
                    match act {
                        Act::Think(_) => out.would_think += 1,
                        Act::Say(thought) => {
                            out.would_say.push(Line {
                                t: world.now - first,
                                hook: thought.hook.clone(),
                                text: thought.text.clone(),
                            });
                            // (No voice here: taken as done a few seconds on.)
                            intuition.voiced(world.now + REPLAY_VOICE_S);
                        }
                        Act::Hum(_) | Act::Note(_) => {}
                    }
                }
                if let Some(hold) = intuition.hold() {
                    let dt = last_frame_at
                        .map(|t| (world.now - t).clamp(0.0, FRAME_EVERY_S * 5.0))
                        .unwrap_or(0.0);
                    *out.holds.entry(hold).or_insert(0.0) += dt;
                }
                last_frame_at = Some(world.now);
            }
            Event::Thought(thought) => intuition.thought(thought.clone()),
            Event::Think { .. } => out.did_think += 1,
            Event::Said { hook } => out.did_say.push(Line {
                t: entry.t - first,
                hook: hook.clone(),
                text: String::new(),
            }),
            Event::Voiced { .. } | Event::Unsaid => {}
            Event::PlayerSpoke => {
                intuition.player_spoke(entry.t);
            }
            Event::Stop => {
                intuition.told_to_stop(entry.t);
            }
            Event::Dial(id) => {
                if level.is_none() {
                    intuition.director.set_level(id);
                }
            }
        }
    }
    // The session's own lines carry their words in the trace's thoughts.
    for line in &mut out.did_say {
        if let Some(thought) = entries.iter().rev().find_map(|e| match &e.event {
            Event::Thought(t) if t.hook == line.hook && e.t <= line.t + first => Some(t),
            _ => None,
        }) {
            line.text = thought.text.clone();
        }
    }
    out.stats = intuition.director.stats;
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::intuition::hooks::Kind;
    use crate::intuition::tempo::Tempo;
    use crate::intuition::thoughts::Basis;
    use crate::intuition::world::Place;

    fn frame(now: f64, map: &str) -> World {
        World {
            now,
            seen: true,
            place: Some(Place {
                name: map.into(),
                age_s: 1.0,
            }),
            level: Some(25),
            hp: Some(100.0),
            activity: 0.004,
            since_anyone_spoke: now,
            session_s: now + 1200.0,
            classic: true,
            ..World::default()
        }
    }

    fn thought(now: f64) -> Thought {
        Thought {
            id: 1,
            hook: "ellinia-music".into(),
            topic: "music".into(),
            kind: Kind::Opinion,
            text: "The piano here is the best in the game.".into(),
            rating: 4.5,
            basis: Basis {
                map: Some("Ellinia".into()),
                level: None,
                alive: true,
                tempo_at_most: Tempo::Travel,
                made_at: now,
                ttl_s: 400.0,
            },
            hum: Some("morning-leaf".into()),
        }
    }

    #[test]
    fn a_trace_round_trips_and_a_frame_is_written_once_a_second_or_on_a_change() {
        let dir = std::env::temp_dir().join(format!(
            "ms-intuition-trace-{}",
            crate::phone::tls::random_hex(4)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let mut tracer = Tracer::open(&dir);
        assert!(tracer.is_on());
        // Ten frames a second for three seconds: three frames written, plus
        // the one where the map changed.
        for i in 0..30 {
            let now = i as f64 / 10.0;
            let map = if i < 25 { "Henesys" } else { "Ellinia" };
            tracer.world(&frame(now, map));
        }
        tracer.event(2.6, Event::Thought(thought(2.6)));
        tracer.event(2.7, Event::PlayerSpoke);
        let mut first = frame(3.5, "Ellinia");
        first.since_anyone_spoke = f64::INFINITY;
        tracer.world(&first);
        drop(tracer);
        let entries = read(&dir.join(FILE)).unwrap();
        let worlds: Vec<&Entry> = entries
            .iter()
            .filter(|e| matches!(e.event, Event::World(_)))
            .collect();
        assert_eq!(worlds.len(), 5, "{entries:?}");
        assert!(
            matches!(&worlds[3].event, Event::World(w) if w.map() == Some("Ellinia") && (w.now - 2.5).abs() < 1e-9)
        );
        assert!(matches!(&worlds[4].event, Event::World(w) if w.since_anyone_spoke.is_infinite()));
        assert!(matches!(&entries[4].event, Event::Thought(t) if t.hook == "ellinia-music"));
        assert_eq!(entries[5].event, Event::PlayerSpoke);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_replay_says_the_traced_thought_in_the_gap_and_a_quieter_dial_says_less() {
        let mut entries = Vec::new();
        for s in 0..120 {
            let now = s as f64;
            let map = if s < 30 { "Henesys" } else { "Ellinia" };
            entries.push(Entry {
                t: now,
                event: Event::World(frame(now, map)),
            });
            if s == 40 {
                entries.push(Entry {
                    t: now,
                    event: Event::Thought(thought(now)),
                });
            }
            if s == 45 {
                entries.push(Entry {
                    t: now,
                    event: Event::Think {
                        trigger: "map_entered".into(),
                        hooks: vec!["ellinia-music".into()],
                    },
                });
            }
            if s == 70 {
                entries.push(Entry {
                    t: now,
                    event: Event::Said {
                        hook: "ellinia-music".into(),
                    },
                });
            }
        }
        let replayed = replay(&entries, None, 7);
        assert_eq!(replayed.would_say.len(), 1, "{replayed:?}");
        assert_eq!(replayed.would_say[0].hook, "ellinia-music");
        assert!(replayed.would_say[0].t > 40.0);
        assert_eq!(replayed.did_say.len(), 1);
        assert_eq!(replayed.did_say[0].text, thought(0.0).text);
        assert_eq!(replayed.did_think, 1);
        assert!(replayed.would_think >= 1);
        assert!((replayed.seconds - 119.0).abs() < 1e-9);
        // Every second but the one it spoke in is accounted for by a hold:
        // settling in at first, nothing ready until the thought came, then
        // (after it was said) too soon for another.
        let held: f64 = replayed.holds.values().sum();
        assert!((held - 118.0).abs() < 1e-6, "{:?}", replayed.holds);
        for hold in [Hold::Settling, Hold::NothingReady, Hold::TooSoon] {
            assert!(replayed.holds.contains_key(&hold), "{:?}", replayed.holds);
        }
        // Nobody answered: let pass.
        assert_eq!(replayed.stats.ignored, 1);
        let report = replayed.report();
        assert!(report.contains("ellinia-music"), "{report}");
        assert!(report.contains("nothing ready") && report.contains("too soon"));
        // On the quiet dial a 4.5 waits for a longer silence than two
        // minutes of a fresh session give.
        let quiet = replay(&entries, Some("quiet"), 7);
        assert!(quiet.would_say.len() <= replayed.would_say.len());
    }
}
