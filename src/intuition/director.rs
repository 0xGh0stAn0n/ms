//! The director: whether a thought is said now, and which.
//!
//! The mind thinks; the director keeps the room. It is the piece every
//! study of proactive agents says must exist apart from the model: left to
//! themselves, models over-propose (half their offers are false alarms on
//! ProactiveBench), and the speech models open their mouths when addressed
//! or when it is quiet, never because something is worth saying. So the
//! decision is made here, from numbers, and tested by replaying sessions:
//!
//! - nothing while anyone talks, nor within a gap of the last word (two
//!   lines in a row is nagging);
//! - nothing in combat or at a peak; small talk waits for travel or calm;
//! - a thought has to want it enough (its [`Thought::wish`]) to pass the
//!   threshold of the player's dial — quiet, normal, talkative
//!   (`initiative.json`) — with a bonus right after a boundary of play (a
//!   map entered, a level gained) and a penalty for a tense moment;
//! - a topic spoken lately is fresh again only after its cooldown; a
//!   hook marked once per session is said once;
//! - at most so many thoughts in ten minutes, and the gaps grow each time
//!   the player lets a thought pass without a word (and shrink back when
//!   they answer one): the player's silence is the dial they never have
//!   to touch.

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::sync::OnceLock;

use serde::Deserialize;

use super::hooks::Hooks;
use super::tempo::Tempo;
use super::thoughts::{Pool, Thought, Weights};
use super::world::World;

const DATA: &str = include_str!("initiative.json");

/// One position of the player's dial.
#[derive(Debug, Clone, Deserialize)]
pub struct Level {
    pub id: String,
    /// A thought's wish must reach this (1–5) to be said.
    pub threshold: f64,
    /// The pacing's gaps are multiplied by this.
    pub gap_factor: f64,
    /// At most this many thoughts in [`Pacing::cap_window_s`].
    pub at_most_per_window: u32,
}

/// The director's pacing (`initiative.json`), in seconds and ratings.
#[derive(Debug, Clone, Deserialize)]
pub struct Pacing {
    /// The window the levels' cap counts in.
    pub cap_window_s: f64,
    pub min_gap_after_anyone_spoke_s: f64,
    pub min_gap_between_own_thoughts_s: f64,
    /// The game newly in view gets this long before the first thought.
    pub settle_in_s: f64,
    pub silence_boost_per_s: f64,
    pub silence_boost_at_most: f64,
    /// What is left of a thought's wish right after its topic was spoken.
    pub spoken_topic_floor: f64,
    /// Right after a boundary of play, for this long, a thought's wish gets
    /// this bonus.
    pub boundary_window_s: f64,
    pub boundary_bonus: f64,
    /// Taken off a thought's wish at each tempo (9 and up: nothing is said).
    pub tempo_penalty: BTreeMap<String, f64>,
    /// A player's word within this long of a thought is an answer to it.
    pub response_window_s: f64,
    /// A thought let pass unanswered multiplies the gaps by this, up to the
    /// ceiling; an answered one resets them.
    pub ignored_backoff_factor: f64,
    pub ignored_backoff_at_most: f64,
    /// How long a thought keeps, in seconds.
    pub thought_ttl_s: f64,
    /// The mind is asked to think no more often than this.
    pub think_every_at_least_s: f64,
}

/// A penalty at or over this says: nothing at this tempo.
pub const PENALTY_IS_SILENCE: f64 = 9.0;

/// The dial and the pacing, read once.
#[derive(Debug, Deserialize)]
pub struct Initiative {
    default: String,
    levels: Vec<Level>,
    pub pacing: Pacing,
}

impl Initiative {
    pub fn get() -> &'static Initiative {
        static INITIATIVE: OnceLock<Initiative> = OnceLock::new();
        INITIATIVE.get_or_init(|| {
            serde_json::from_str(DATA).unwrap_or_else(|e| panic!("initiative.json: {e}"))
        })
    }

    pub fn levels(&self) -> &[Level] {
        &self.levels
    }

    /// Where the dial stands for a player who never moved it.
    pub fn default_index(&self) -> usize {
        self.index(&self.default).unwrap_or(0)
    }

    /// The level named `id`.
    pub fn index(&self, id: &str) -> Option<usize> {
        let id = id.trim();
        self.levels
            .iter()
            .position(|l| l.id.eq_ignore_ascii_case(id))
    }

    pub fn level(&self, index: usize) -> &Level {
        &self.levels[index.min(self.levels.len() - 1)]
    }

    /// The names of the levels, quietest first.
    pub fn ids(&self) -> Vec<&str> {
        self.levels.iter().map(|l| l.id.as_str()).collect()
    }

    /// What the tempo takes off a thought's wish.
    pub fn penalty(&self, tempo: Tempo) -> f64 {
        self.pacing
            .tempo_penalty
            .get(tempo.word())
            .copied()
            .unwrap_or(PENALTY_IS_SILENCE)
    }

    pub fn weights(&self) -> Weights {
        Weights {
            silence_boost_per_s: self.pacing.silence_boost_per_s,
            silence_boost_at_most: self.pacing.silence_boost_at_most,
            spoken_topic_floor: self.pacing.spoken_topic_floor,
        }
    }
}

/// What became of a thought that was said.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// The player said something within the window.
    Engaged,
    /// Nothing came back.
    Ignored,
    /// The player asked for quiet.
    Rejected,
}

/// Counts for the session's record.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Stats {
    pub spoken: u32,
    pub engaged: u32,
    pub ignored: u32,
    pub rejected: u32,
}

/// A thought handed to the voice, until the player's answer (or none)
/// settles what became of it.
#[derive(Debug, Clone)]
struct Awaiting {
    hook: String,
    topic: String,
    /// What the topic's last time was before this one (to put back when the
    /// thought turns out not to have been said).
    topic_before: Option<f64>,
    /// When the voice finished saying it; None while it is still being
    /// made. The player's window runs from here, not from the decision: a
    /// line takes seconds to make and say, and a hum after it longer.
    heard_at: Option<f64>,
}

/// Why nothing was said this frame, for the log and the replay.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Hold {
    NotWatching,
    Settling,
    Talking,
    TooSoon,
    TooMany,
    Tempo,
    NothingReady,
    NotWanted,
}

impl Hold {
    pub fn word(self) -> &'static str {
        match self {
            Hold::NotWatching => "not watching",
            Hold::Settling => "settling in",
            Hold::Talking => "someone talking",
            Hold::TooSoon => "too soon after the last word",
            Hold::TooMany => "enough for now",
            Hold::Tempo => "tempo",
            Hold::NothingReady => "nothing ready",
            Hold::NotWanted => "not wanted enough",
        }
    }
}

#[derive(Debug)]
pub struct Director {
    level: usize,
    /// When each topic was last spoken.
    topic_spoken: HashMap<String, f64>,
    /// Hooks said this session (for once-per-session ones).
    hooks_spoken: HashSet<String>,
    last_thought_at: f64,
    /// When thoughts were said lately (for the ten-minute cap).
    spoken_at: VecDeque<f64>,
    /// When the last boundary of play was crossed.
    boundary_at: f64,
    awaiting: Option<Awaiting>,
    /// What became of the last thought, and when it was heard (so a "shh"
    /// right after an answer still counts against the thought).
    last_outcome: Option<(Outcome, f64)>,
    /// The gaps' multiplier from thoughts let pass (1.0: none).
    backoff: f64,
    seen_since: Option<f64>,
    pub stats: Stats,
}

impl Director {
    /// A director at the dial position named `level` (None, or an unknown
    /// name: where the dial starts).
    pub fn new(level: Option<&str>) -> Self {
        let dial = Initiative::get();
        Self {
            level: level
                .and_then(|id| dial.index(id))
                .unwrap_or_else(|| dial.default_index()),
            topic_spoken: HashMap::new(),
            hooks_spoken: HashSet::new(),
            last_thought_at: f64::NEG_INFINITY,
            spoken_at: VecDeque::new(),
            boundary_at: f64::NEG_INFINITY,
            awaiting: None,
            last_outcome: None,
            backoff: 1.0,
            seen_since: None,
            stats: Stats::default(),
        }
    }

    /// The dial's level now.
    pub fn level(&self) -> &'static Level {
        Initiative::get().level(self.level)
    }

    /// Move the dial to the level named `id`; false when there is no such
    /// level.
    pub fn set_level(&mut self, id: &str) -> bool {
        match Initiative::get().index(id) {
            Some(i) => {
                self.level = i;
                true
            }
            None => false,
        }
    }

    /// A boundary of play was crossed: for a little while a thought is
    /// welcome.
    pub fn boundary(&mut self, now: f64) {
        self.boundary_at = now;
    }

    /// How fresh `topic` is, 0–1: 0 right after it was spoken, 1 once its
    /// cooldown has passed.
    pub fn topic_fresh(&self, topic: &str, now: f64) -> f64 {
        match self.topic_spoken.get(topic) {
            None => 1.0,
            Some(at) => {
                let cooldown = Hooks::get().cooldown(topic).max(1.0);
                ((now - at) / cooldown).clamp(0.0, 1.0)
            }
        }
    }

    /// Whether the hook may still be said (a once-per-session hook that was).
    pub fn hook_open(&self, hook: &str) -> bool {
        match Hooks::get().named(hook) {
            Some(h) if h.once_per_session => !self.hooks_spoken.contains(hook),
            _ => true,
        }
    }

    /// The gaps' multiplier now.
    pub fn backoff(&self) -> f64 {
        self.backoff
    }

    /// One frame: the thought to say now, taken out of `pool`, or why not.
    pub fn consider(
        &mut self,
        world: &World,
        tempo: Tempo,
        pool: &mut Pool,
    ) -> Result<Thought, Hold> {
        let dial = Initiative::get();
        let p = &dial.pacing;
        let now = world.now;
        self.settle(now);
        if !world.seen || world.muted || world.held {
            self.seen_since = None;
            return Err(Hold::NotWatching);
        }
        let since = *self.seen_since.get_or_insert(now);
        if now - since < p.settle_in_s {
            return Err(Hold::Settling);
        }
        if world.talking {
            return Err(Hold::Talking);
        }
        let level = dial.level(self.level);
        let stretch = level.gap_factor * self.backoff;
        if world.since_anyone_spoke < p.min_gap_after_anyone_spoke_s * stretch
            || now - self.last_thought_at < p.min_gap_between_own_thoughts_s * stretch
        {
            return Err(Hold::TooSoon);
        }
        while self
            .spoken_at
            .front()
            .is_some_and(|at| now - at > p.cap_window_s)
        {
            self.spoken_at.pop_front();
        }
        if self.spoken_at.len() >= level.at_most_per_window as usize {
            return Err(Hold::TooMany);
        }
        let penalty = dial.penalty(tempo);
        if penalty >= PENALTY_IS_SILENCE {
            return Err(Hold::Tempo);
        }
        let bonus = if now - self.boundary_at <= p.boundary_window_s {
            p.boundary_bonus
        } else {
            0.0
        };
        let weights = dial.weights();
        let best = pool
            .ready(world, tempo)
            .filter(|t| self.hook_open(&t.hook))
            .map(|t| {
                let wish = t.wish(
                    self.topic_fresh(&t.topic, now),
                    world.since_anyone_spoke,
                    &weights,
                ) + bonus
                    - penalty;
                (wish, t.id)
            })
            .max_by(|a, b| a.0.total_cmp(&b.0));
        let Some((wish, id)) = best else {
            return Err(Hold::NothingReady);
        };
        if wish < level.threshold {
            return Err(Hold::NotWanted);
        }
        pool.take(id).ok_or(Hold::NothingReady)
    }

    /// `thought` goes to the voice at `now`. The player's window opens
    /// when [`Director::voiced`] says the voice is done; a thought that
    /// never made a sound is taken back with [`Director::unsaid`].
    pub fn said(&mut self, thought: &Thought, now: f64) {
        let topic_before = self.topic_spoken.insert(thought.topic.clone(), now);
        self.hooks_spoken.insert(thought.hook.clone());
        self.last_thought_at = now;
        self.spoken_at.push_back(now);
        self.awaiting = Some(Awaiting {
            hook: thought.hook.clone(),
            topic: thought.topic.clone(),
            topic_before,
            heard_at: None,
        });
        self.stats.spoken += 1;
    }

    /// The voice finished the thought (and its hum) at `now`: from here the
    /// player has the window to answer.
    pub fn voiced(&mut self, now: f64) {
        if let Some(a) = self.awaiting.as_mut()
            && a.heard_at.is_none()
        {
            a.heard_at = Some(now);
        }
    }

    /// The thought handed to the voice was never said (called off before a
    /// sound, or the voice failed): as if it had not been decided. Its
    /// hook stays open and its topic as fresh as it was.
    pub fn unsaid(&mut self) {
        let Some(a) = self.awaiting.take() else {
            return;
        };
        if a.heard_at.is_some() {
            // (It was heard after all: nothing to take back.)
            self.awaiting = Some(a);
            return;
        }
        match a.topic_before {
            Some(at) => {
                self.topic_spoken.insert(a.topic, at);
            }
            None => {
                self.topic_spoken.remove(&a.topic);
            }
        }
        self.hooks_spoken.remove(&a.hook);
        self.spoken_at.pop_back();
        self.stats.spoken = self.stats.spoken.saturating_sub(1);
    }

    /// The player said something: an answer to the last thought when it is
    /// within the window of its being heard (the gaps shrink back to
    /// normal); nothing is settled by words said before it was heard.
    pub fn player_spoke(&mut self, now: f64) -> Option<Outcome> {
        let p = &Initiative::get().pacing;
        let heard_at = self.awaiting.as_ref()?.heard_at?;
        self.awaiting = None;
        let outcome = if now - heard_at <= p.response_window_s {
            self.backoff = 1.0;
            self.stats.engaged += 1;
            Outcome::Engaged
        } else {
            self.ignored();
            Outcome::Ignored
        };
        self.last_outcome = Some((outcome, heard_at));
        Some(outcome)
    }

    /// The player asked for quiet at `now`: the gaps go to their longest.
    /// Counted against the last thought when one was heard within the
    /// window (even one their words just before had counted as answered);
    /// None when no thought was near.
    pub fn told_to_stop(&mut self, now: f64) -> Option<Outcome> {
        let p = &Initiative::get().pacing;
        self.backoff = p.ignored_backoff_at_most;
        let near = match (self.awaiting.take(), self.last_outcome) {
            (Some(_), _) => true,
            (None, Some((Outcome::Engaged, heard_at))) if now - heard_at <= p.response_window_s => {
                // Their "shh" was the answer, not whatever came before it.
                self.stats.engaged = self.stats.engaged.saturating_sub(1);
                true
            }
            _ => false,
        };
        if !near {
            return None;
        }
        self.stats.rejected += 1;
        self.last_outcome = Some((Outcome::Rejected, now));
        Some(Outcome::Rejected)
    }

    /// The hook of the thought waiting for an answer, if one is.
    pub fn awaiting(&self) -> Option<&str> {
        self.awaiting.as_ref().map(|a| a.hook.as_str())
    }

    /// A thought's window closed with no word: let pass.
    fn settle(&mut self, now: f64) -> Option<Outcome> {
        let p = &Initiative::get().pacing;
        if let Some(heard_at) = self.awaiting.as_ref().and_then(|a| a.heard_at)
            && now - heard_at > p.response_window_s
        {
            self.awaiting = None;
            self.ignored();
            self.last_outcome = Some((Outcome::Ignored, heard_at));
            return Some(Outcome::Ignored);
        }
        None
    }

    fn ignored(&mut self) {
        let p = &Initiative::get().pacing;
        self.backoff = (self.backoff * p.ignored_backoff_factor).min(p.ignored_backoff_at_most);
        self.stats.ignored += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::intuition::hooks::Kind;
    use crate::intuition::thoughts::Basis;
    use crate::intuition::world::Place;

    fn thought(id: u64, hook: &str, topic: &str, rating: f32) -> Thought {
        Thought {
            id,
            hook: hook.into(),
            topic: topic.into(),
            kind: Kind::Opinion,
            text: format!("t{id}"),
            rating,
            basis: Basis {
                map: None,
                level: None,
                alive: true,
                tempo_at_most: Tempo::Travel,
                made_at: 0.0,
                ttl_s: 1e9,
            },
            hum: None,
        }
    }

    fn calm(now: f64, since_anyone_spoke: f64) -> World {
        World {
            now,
            seen: true,
            place: Some(Place {
                name: "Henesys".into(),
                age_s: 1.0,
            }),
            level: Some(25),
            since_anyone_spoke,
            session_s: now,
            ..World::default()
        }
    }

    #[test]
    fn the_dial_reads_quietest_first_and_the_tempo_penalties_silence_a_fight() {
        let dial = Initiative::get();
        assert_eq!(dial.ids(), ["quiet", "normal", "talkative"]);
        assert_eq!(dial.level(dial.default_index()).id, "normal");
        for pair in dial.levels().windows(2) {
            assert!(pair[0].threshold > pair[1].threshold);
            assert!(pair[0].gap_factor > pair[1].gap_factor);
            assert!(pair[0].at_most_per_window <= pair[1].at_most_per_window);
        }
        assert!(dial.penalty(Tempo::Combat) >= PENALTY_IS_SILENCE);
        assert!(dial.penalty(Tempo::Peak) >= PENALTY_IS_SILENCE);
        assert!(dial.penalty(Tempo::Away) >= PENALTY_IS_SILENCE);
        assert!(dial.penalty(Tempo::Calm) < dial.penalty(Tempo::Travel));
        assert!(dial.pacing.response_window_s > 0.0 && dial.pacing.ignored_backoff_factor > 1.0);
        assert!(dial.pacing.cap_window_s >= 60.0);
        assert!((0.0..1.0).contains(&dial.pacing.spoken_topic_floor));
        assert_eq!(dial.index(" NORMAL "), Some(1));
        assert_eq!(dial.index("loud"), None);
    }

    #[test]
    fn a_wanted_thought_is_said_in_a_calm_gap_and_held_while_anyone_talks_or_fights() {
        let mut d = Director::new(None);
        let mut pool = Pool::new();
        pool.add(thought(1, "ellinia-music", "music", 4.5));
        let settle = Initiative::get().pacing.settle_in_s;
        // Newly in view: settling.
        assert_eq!(
            d.consider(&calm(0.0, 100.0), Tempo::Calm, &mut pool),
            Err(Hold::Settling)
        );
        let mut w = calm(settle + 1.0, 100.0);
        w.talking = true;
        assert_eq!(d.consider(&w, Tempo::Calm, &mut pool), Err(Hold::Talking));
        let w = calm(settle + 2.0, 100.0);
        assert_eq!(d.consider(&w, Tempo::Combat, &mut pool), Err(Hold::Tempo));
        assert_eq!(
            d.consider(&calm(settle + 3.0, 2.0), Tempo::Calm, &mut pool),
            Err(Hold::TooSoon)
        );
        let said = d
            .consider(&calm(settle + 4.0, 100.0), Tempo::Calm, &mut pool)
            .unwrap();
        assert_eq!(said.id, 1);
        assert!(pool.is_empty());
    }

    #[test]
    fn a_lukewarm_thought_waits_for_a_boundary_or_a_longer_silence() {
        let mut d = Director::new(Some("normal"));
        let mut pool = Pool::new();
        pool.add(thought(1, "henesys-music", "music", 3.0));
        let t0 = Initiative::get().pacing.settle_in_s + 1.0;
        // Rating 3 a short while after the last word: not wanted enough.
        d.consider(&calm(0.0, 100.0), Tempo::Calm, &mut pool).ok();
        assert_eq!(
            d.consider(&calm(t0, 25.0), Tempo::Calm, &mut pool),
            Err(Hold::NotWanted)
        );
        // Right after a map was entered, it is.
        d.boundary(t0 + 1.0);
        assert!(
            d.consider(&calm(t0 + 2.0, 26.0), Tempo::Calm, &mut pool)
                .is_ok()
        );
        // A long silence alone brings it over the line too.
        let mut e = Director::new(Some("normal"));
        let mut pool = Pool::new();
        pool.add(thought(3, "henesys-music", "music", 3.0));
        e.consider(&calm(0.0, 100.0), Tempo::Calm, &mut pool).ok();
        assert_eq!(
            e.consider(&calm(t0, 30.0), Tempo::Calm, &mut pool),
            Err(Hold::NotWanted)
        );
        assert!(
            e.consider(&calm(t0 + 1.0, 120.0), Tempo::Calm, &mut pool)
                .is_ok()
        );
        // On the quiet dial neither the boundary nor a minute's silence does.
        let mut quiet = Director::new(Some("quiet"));
        let mut pool = Pool::new();
        pool.add(thought(2, "henesys-music", "music", 3.0));
        quiet
            .consider(&calm(0.0, 100.0), Tempo::Calm, &mut pool)
            .ok();
        quiet.boundary(t0 + 1.0);
        assert_eq!(
            quiet.consider(&calm(t0 + 2.0, 50.0), Tempo::Calm, &mut pool),
            Err(Hold::NotWanted)
        );
        assert_eq!(
            quiet.consider(&calm(t0 + 60.0, 60.0), Tempo::Calm, &mut pool),
            Err(Hold::NotWanted)
        );
    }

    #[test]
    fn a_topic_just_spoken_waits_its_cooldown_and_a_once_hook_is_said_once() {
        let mut d = Director::new(Some("talkative"));
        let mut pool = Pool::new();
        let t0 = Initiative::get().pacing.settle_in_s + 1.0;
        d.consider(&calm(0.0, 100.0), Tempo::Calm, &mut pool).ok();
        pool.add(thought(1, "ellinia-music", "music", 5.0));
        let first = d
            .consider(&calm(t0, 100.0), Tempo::Calm, &mut pool)
            .unwrap();
        d.said(&first, t0);
        assert!((d.topic_fresh("music", t0) - 0.0).abs() < 1e-9);
        assert!(!d.hook_open("ellinia-music"));
        assert!(d.hook_open("town-breather"));
        // The same hook again: never, even when it is thought again.
        pool.add(thought(2, "ellinia-music", "music", 5.0));
        let later = t0 + 1000.0;
        assert_eq!(
            d.consider(&calm(later, 500.0), Tempo::Calm, &mut pool),
            Err(Hold::NothingReady)
        );
        // Another music thought, past the cooldown: fresh again.
        let cooldown = Hooks::get().cooldown("music");
        assert!(d.topic_fresh("music", t0 + cooldown) >= 1.0 - 1e-9);
        assert!(d.topic_fresh("music", t0 + cooldown / 2.0) < 0.6);
    }

    #[test]
    fn thoughts_let_pass_stretch_the_gaps_and_an_answer_brings_them_back() {
        let mut d = Director::new(Some("normal"));
        let p = &Initiative::get().pacing;
        let t = thought(1, "henesys-music", "music", 5.0);
        d.said(&t, 100.0);
        assert_eq!(d.awaiting(), Some("henesys-music"));
        // Words while the line is still being made settle nothing.
        assert_eq!(d.player_spoke(101.0), None);
        assert_eq!(d.awaiting(), Some("henesys-music"));
        // The voice is done at 106 (a line and a hum); the player answers
        // within the window of that: engaged.
        d.voiced(106.0);
        assert_eq!(
            d.player_spoke(106.0 + p.response_window_s - 1.0),
            Some(Outcome::Engaged)
        );
        assert_eq!(d.backoff(), 1.0);
        assert_eq!(d.awaiting(), None);
        assert_eq!(d.player_spoke(200.0), None);
        // Said again and let pass: the gaps stretch — counted from when it
        // was heard, not from the decision.
        d.said(&t, 300.0);
        d.voiced(310.0);
        let mut pool = Pool::new();
        let _ = d.consider(
            &calm(300.0 + p.response_window_s + 1.0, 50.0),
            Tempo::Calm,
            &mut pool,
        );
        assert_eq!(d.stats.ignored, 0, "the window had not closed yet");
        let _ = d.consider(
            &calm(310.0 + p.response_window_s + 1.0, 50.0),
            Tempo::Calm,
            &mut pool,
        );
        assert!((d.backoff() - p.ignored_backoff_factor).abs() < 1e-9);
        assert_eq!(d.stats.ignored, 1);
        // Asked to stop right after one: the longest gaps, and it counts
        // against the thought.
        d.said(&t, 400.0);
        d.voiced(404.0);
        assert_eq!(d.told_to_stop(406.0), Some(Outcome::Rejected));
        assert!((d.backoff() - p.ignored_backoff_at_most).abs() < 1e-9);
        assert_eq!(d.stats.spoken, 3);
        assert_eq!(d.stats.engaged, 1);
        assert_eq!(d.stats.rejected, 1);
        // "Shh" with no thought near: the gaps still go to their longest,
        // but no thought is blamed.
        d.backoff = 1.0;
        assert_eq!(d.told_to_stop(900.0), None);
        assert!((d.backoff() - p.ignored_backoff_at_most).abs() < 1e-9);
        assert_eq!(d.stats.rejected, 1);
    }

    #[test]
    fn a_shh_right_after_an_answer_counts_against_the_thought_not_for_it() {
        // "Stop coaching" is heard as words first; the words count as an
        // answer, then the command arrives: the answer is taken back.
        let mut d = Director::new(Some("normal"));
        let t = thought(1, "henesys-music", "music", 5.0);
        d.said(&t, 100.0);
        d.voiced(104.0);
        assert_eq!(d.player_spoke(105.0), Some(Outcome::Engaged));
        assert_eq!(d.told_to_stop(105.0), Some(Outcome::Rejected));
        assert_eq!(d.stats.engaged, 0);
        assert_eq!(d.stats.rejected, 1);
    }

    #[test]
    fn a_thought_that_never_made_a_sound_is_taken_back_whole() {
        let mut d = Director::new(Some("normal"));
        let p = &Initiative::get().pacing;
        d.topic_spoken.insert("plan".into(), 50.0);
        let t = thought(1, "kerning-pq-suggestion", "plan", 5.0);
        d.said(&t, 100.0);
        assert!(!d.hook_open("kerning-pq-suggestion"));
        assert_eq!(d.stats.spoken, 1);
        // The player talked over it before a sound was made.
        d.unsaid();
        assert!(
            d.hook_open("kerning-pq-suggestion"),
            "a once hook not said is still open"
        );
        assert_eq!(d.stats.spoken, 0);
        assert_eq!(d.awaiting(), None);
        assert_eq!(
            d.topic_spoken.get("plan"),
            Some(&50.0),
            "the topic is as fresh as before"
        );
        assert!(d.spoken_at.is_empty(), "nothing counts towards the cap");
        // Once heard, there is nothing to take back.
        d.said(&t, 200.0);
        d.voiced(203.0);
        d.unsaid();
        assert_eq!(d.stats.spoken, 1);
        assert_eq!(d.awaiting(), Some("kerning-pq-suggestion"));
        let _ = d.consider(
            &calm(203.0 + p.response_window_s + 1.0, 50.0),
            Tempo::Calm,
            &mut Pool::new(),
        );
        assert_eq!(d.stats.ignored, 1);
    }

    #[test]
    fn no_more_than_the_dial_allows_in_the_window() {
        let mut d = Director::new(Some("normal"));
        let p = &Initiative::get().pacing;
        let at_most = d.level().at_most_per_window;
        let t = thought(1, "town-breather", "mood", 5.0);
        for i in 0..at_most {
            d.said(&t, 100.0 + i as f64);
            d.voiced(100.0 + i as f64);
        }
        let mut pool = Pool::new();
        pool.add(thought(9, "back-from-away", "mood", 5.0));
        d.consider(&calm(0.0, 100.0), Tempo::Calm, &mut pool).ok();
        assert_eq!(
            d.consider(
                &calm(100.0 + p.settle_in_s + 400.0, 400.0),
                Tempo::Calm,
                &mut pool
            ),
            Err(Hold::TooMany)
        );
        // The window on, the cap has passed: the gaps from the thoughts let
        // pass are long, but they are the next hold, not the cap.
        d.backoff = 1.0;
        let later = 100.0 + p.cap_window_s + 1.0;
        assert_eq!(
            d.consider(&calm(later, 1.0), Tempo::Calm, &mut pool),
            Err(Hold::TooSoon)
        );
        assert!(
            d.consider(&calm(later, p.cap_window_s), Tempo::Calm, &mut pool)
                .is_ok()
        );
    }
}
