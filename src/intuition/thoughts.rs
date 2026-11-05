//! Thoughts: what MapleSyrup could say on its own, thought out ahead of the
//! moment it is said.
//!
//! The mind makes a thought from a hook when something happens (a map
//! entered, a level gained, a long lull) — in the background, off the
//! voice's path. Each carries its basis: the map, the level, alive or
//! dead, how tense the game may be, and when it was made. A thought is
//! voiced only while its basis still holds (Ruskin's rule: a follow-up is
//! checked when it is about to play, not when it was queued, so a line
//! about a map they have left dies unspoken — the owner's rule that a
//! line about something no longer true is not said), and never past its
//! time to live.
//!
//! A thought's wish to be said is a 1–5 rating the mind gave it (Inner
//! Thoughts, CHI 2025: the model argues for and against, then rates), times
//! how fresh its topic is, times a boost that grows with silence.

use std::collections::VecDeque;

use serde::{Deserialize, Serialize};

use super::hooks::Kind;
use super::tempo::Tempo;
use super::world::World;

/// What a thought stands on. While it holds, the thought may be said.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Basis {
    /// The map it is about (None: not about a place).
    pub map: Option<String>,
    /// The level it assumed (None: not about the level).
    pub level: Option<u32>,
    /// Alive when thought.
    pub alive: bool,
    /// The game may be no tenser than this when it is said.
    pub tempo_at_most: Tempo,
    /// When it was thought, in session seconds.
    pub made_at: f64,
    /// How long it keeps, in seconds.
    pub ttl_s: f64,
}

/// What broke a thought's basis.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Broken {
    /// Past its time to live.
    Stale,
    /// They are not on the map it is about (or the reading is too old to
    /// say where they are).
    LeftTheMap,
    LevelChanged,
    Dead,
    /// The game is tenser than the thought bears — the one break that
    /// mends by itself when the tempo falls.
    Tempo,
}

impl Broken {
    /// For the log.
    pub fn word(self) -> &'static str {
        match self {
            Broken::Stale => "stale",
            Broken::LeftTheMap => "left the map",
            Broken::LevelChanged => "level changed",
            Broken::Dead => "dead",
            Broken::Tempo => "tempo",
        }
    }
}

impl Basis {
    /// Whether it still holds in `world` at `tempo`.
    pub fn holds(&self, world: &World, tempo: Tempo) -> bool {
        self.broken_by(world, tempo).is_none()
    }

    /// What does not hold any more (None: it holds).
    pub fn broken_by(&self, world: &World, tempo: Tempo) -> Option<Broken> {
        if world.now - self.made_at > self.ttl_s {
            return Some(Broken::Stale);
        }
        if let Some(map) = &self.map
            && world
                .map()
                .is_none_or(|m| !super::hooks::same_place(m, map))
        {
            return Some(Broken::LeftTheMap);
        }
        if let (Some(level), Some(now)) = (self.level, world.level)
            && now != level
        {
            return Some(Broken::LevelChanged);
        }
        if self.alive && world.dead {
            return Some(Broken::Dead);
        }
        if tempo > self.tempo_at_most || tempo == Tempo::Away {
            return Some(Broken::Tempo);
        }
        None
    }
}

/// One thought, ready to be said.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Thought {
    pub id: u64,
    /// The hook it came from.
    pub hook: String,
    pub topic: String,
    pub kind: Kind,
    /// The line, in the player's language, as the mind phrased it.
    pub text: String,
    /// How much it wants to be said, 1–5 (the mind's rating).
    pub rating: f32,
    pub basis: Basis,
    /// A melody to hum after it (`melodies.json`), if the hook has one and
    /// the mind felt like it.
    pub hum: Option<String>,
}

/// What makes a thought want to be said now (`initiative.json`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Weights {
    /// The boost per second of silence (1.0: none), and its ceiling.
    pub silence_boost_per_s: f64,
    pub silence_boost_at_most: f64,
    /// What is left of a thought's wish right after its topic was spoken
    /// (0–1; it climbs back to 1 over the topic's cooldown).
    pub spoken_topic_floor: f64,
}

impl Thought {
    /// Its wish to be said now: the rating, scaled by how fresh its topic
    /// is (`topic_fresh`, 0–1: 0 right after the topic was spoken, 1 once
    /// its cooldown has passed) and by the silence so far.
    pub fn wish(&self, topic_fresh: f64, since_anyone_spoke: f64, w: &Weights) -> f64 {
        let boost = w
            .silence_boost_per_s
            .powf(since_anyone_spoke.max(0.0))
            .min(w.silence_boost_at_most);
        let floor = w.spoken_topic_floor.clamp(0.0, 1.0);
        let fresh = floor + (1.0 - floor) * topic_fresh.clamp(0.0, 1.0);
        (self.rating as f64).clamp(1.0, 5.0) * fresh * boost
    }
}

/// How many thoughts are kept at once: the newest and the most wanted
/// stay; a crowd of stale thoughts would only ever say the oldest.
pub const POOL_SIZE: usize = 6;

/// The thoughts waiting for their moment.
#[derive(Debug, Default)]
pub struct Pool {
    thoughts: VecDeque<Thought>,
}

impl Pool {
    pub fn new() -> Self {
        Self::default()
    }

    /// A thought came in: kept, the pool trimmed to [`POOL_SIZE`] (a
    /// thought from the same hook replaces the earlier one).
    pub fn add(&mut self, thought: Thought) {
        self.thoughts.retain(|t| t.hook != thought.hook);
        self.thoughts.push_back(thought);
        while self.thoughts.len() > POOL_SIZE {
            self.thoughts.pop_front();
        }
    }

    /// Drops what no longer holds; returns what was dropped and why.
    pub fn prune(&mut self, world: &World, tempo: Tempo) -> Vec<(Thought, Broken)> {
        let mut dropped = Vec::new();
        self.thoughts
            .retain(|t| match t.basis.broken_by(world, tempo) {
                // (The tempo may fall again: a thought waits that out.)
                None | Some(Broken::Tempo) => true,
                Some(why) => {
                    dropped.push((t.clone(), why));
                    false
                }
            });
        dropped
    }

    /// The thoughts that could be said right now, in `world` at `tempo`.
    pub fn ready<'a>(&'a self, world: &World, tempo: Tempo) -> impl Iterator<Item = &'a Thought> {
        let world = world.clone();
        self.thoughts
            .iter()
            .filter(move |t| t.basis.holds(&world, tempo))
    }

    /// Takes the thought with this id out.
    pub fn take(&mut self, id: u64) -> Option<Thought> {
        let at = self.thoughts.iter().position(|t| t.id == id)?;
        self.thoughts.remove(at)
    }

    pub fn len(&self) -> usize {
        self.thoughts.len()
    }

    pub fn is_empty(&self) -> bool {
        self.thoughts.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = &Thought> {
        self.thoughts.iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::intuition::world::Place;

    fn thought(id: u64, hook: &str, map: Option<&str>, made_at: f64) -> Thought {
        Thought {
            id,
            hook: hook.into(),
            topic: "music".into(),
            kind: Kind::Opinion,
            text: format!("thought {id}"),
            rating: 4.0,
            basis: Basis {
                map: map.map(str::to_string),
                level: Some(25),
                alive: true,
                tempo_at_most: Tempo::Travel,
                made_at,
                ttl_s: 300.0,
            },
            hum: None,
        }
    }

    fn world(now: f64, map: &str) -> World {
        World {
            now,
            seen: true,
            place: Some(Place {
                name: map.into(),
                age_s: 1.0,
            }),
            level: Some(25),
            ..World::default()
        }
    }

    #[test]
    fn a_thought_holds_on_its_map_alive_and_calm_and_dies_when_any_of_that_changes() {
        let t = thought(1, "ellinia-music", Some("Ellinia"), 100.0);
        let w = world(120.0, "Ellinia");
        assert!(t.basis.holds(&w, Tempo::Calm));
        assert!(t.basis.holds(&w, Tempo::Travel));
        assert_eq!(t.basis.broken_by(&w, Tempo::Combat), Some(Broken::Tempo));
        assert_eq!(
            t.basis.broken_by(&world(120.0, "Henesys"), Tempo::Calm),
            Some(Broken::LeftTheMap)
        );
        let mut leveled = w.clone();
        leveled.level = Some(26);
        assert_eq!(
            t.basis.broken_by(&leveled, Tempo::Calm),
            Some(Broken::LevelChanged)
        );
        let mut dead = w.clone();
        dead.dead = true;
        assert_eq!(t.basis.broken_by(&dead, Tempo::Calm), Some(Broken::Dead));
        assert_eq!(
            t.basis.broken_by(&world(500.0, "Ellinia"), Tempo::Calm),
            Some(Broken::Stale)
        );
        assert_eq!(t.basis.broken_by(&w, Tempo::Away), Some(Broken::Tempo));
        assert_eq!(Broken::LeftTheMap.word(), "left the map");
    }

    #[test]
    fn the_wish_grows_with_silence_and_falls_right_after_its_topic_was_spoken() {
        let t = thought(1, "h", None, 0.0);
        let w = Weights {
            silence_boost_per_s: 1.015,
            silence_boost_at_most: 1.7,
            spoken_topic_floor: 0.35,
        };
        let fresh_quiet = t.wish(1.0, 0.0, &w);
        assert!((fresh_quiet - 4.0).abs() < 1e-9);
        assert!(t.wish(1.0, 60.0, &w) > fresh_quiet);
        assert!((t.wish(1.0, 3600.0, &w) - 4.0 * 1.7).abs() < 1e-9);
        assert!((t.wish(0.0, 0.0, &w) - 4.0 * 0.35).abs() < 1e-9);
        // Silence never counted against it (a clock skew, say).
        assert!((t.wish(1.0, -5.0, &w) - 4.0).abs() < 1e-9);
    }

    #[test]
    fn the_pool_keeps_one_thought_per_hook_prunes_the_broken_and_stays_small() {
        let mut pool = Pool::new();
        pool.add(thought(1, "a", Some("Ellinia"), 0.0));
        pool.add(thought(2, "a", Some("Ellinia"), 1.0));
        assert_eq!(pool.len(), 1);
        assert_eq!(pool.iter().next().unwrap().id, 2);
        for i in 3..20 {
            pool.add(thought(i, &format!("h{i}"), None, i as f64));
        }
        assert_eq!(pool.len(), POOL_SIZE);
        // The newest survive.
        assert!(pool.iter().all(|t| t.id >= 14));
        // A thought about Ellinia dies on leaving it; the rest hold.
        let mut pool = Pool::new();
        pool.add(thought(1, "a", Some("Ellinia"), 0.0));
        pool.add(thought(2, "b", None, 0.0));
        let dropped = pool.prune(&world(10.0, "Henesys"), Tempo::Calm);
        assert_eq!(dropped.len(), 1);
        assert_eq!(dropped[0].1, Broken::LeftTheMap);
        assert_eq!(pool.len(), 1);
        // A tense moment is waited out, not pruned.
        assert!(pool.prune(&world(11.0, "Henesys"), Tempo::Peak).is_empty());
        assert_eq!(pool.ready(&world(11.0, "Henesys"), Tempo::Peak).count(), 0);
        assert_eq!(pool.ready(&world(11.0, "Henesys"), Tempo::Calm).count(), 1);
        assert!(pool.take(2).is_some());
        assert!(pool.is_empty());
    }
}
