//! How tense the game is right now, and so whether a thought may be voiced.
//!
//! Every companion that was tested with players failed the same way first:
//! it talked during fights (LeagueBot's pilot took proactive speech out for
//! exactly that; TechRadar on PUBG Ally: "constant chatter could mask audio
//! cues"). So the intuition keeps an intensity in the shape of Left 4
//! Dead's director (Booth, GDC 2009): events — HP lost, a death, the
//! picture cutting, the screen moving — push it up, it decays with time,
//! and the decay waits while HP keeps falling. The intensity falls in one
//! of a few bands, the [`Tempo`]: at a peak only a warning speaks; in
//! combat only what helps now; travel and calm are where company belongs.
//!
//! The vision engine reports no mobs and no damage numbers to this code,
//! so the intensity is read off HP, death, the scene's activity and its
//! cuts. The weights, the decay and the bands are data (`tempo.json`),
//! tuned by replaying sessions.

use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

use super::world::World;

const DATA: &str = include_str!("tempo.json");

/// How tense the game is, calm to peak. Ordered: `Tempo::Calm < Tempo::Peak`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Tempo {
    /// Nobody is playing: the game idles, or is out of sight.
    Away,
    /// A town, a menu, a lull: the best moment for company.
    Calm,
    /// Moving between maps, a stretch between packs.
    Travel,
    /// Fighting.
    Combat,
    /// A boss, a close call: nothing but a warning.
    Peak,
}

impl Tempo {
    /// Its name, as the data and the log have it.
    pub fn word(self) -> &'static str {
        match self {
            Tempo::Away => "away",
            Tempo::Calm => "calm",
            Tempo::Travel => "travel",
            Tempo::Combat => "combat",
            Tempo::Peak => "peak",
        }
    }

    /// The tempo named `word`, as the data has it.
    pub fn named(word: &str) -> Option<Tempo> {
        [
            Tempo::Away,
            Tempo::Calm,
            Tempo::Travel,
            Tempo::Combat,
            Tempo::Peak,
        ]
        .into_iter()
        .find(|t| t.word().eq_ignore_ascii_case(word.trim()))
    }
}

/// What pushes the intensity up, per event (`tempo.json`).
#[derive(Debug, Deserialize)]
pub struct Events {
    /// Per percent of HP lost between two readings.
    pub hp_lost_per_percent: f32,
    /// HP under this percent is low.
    pub hp_low_percent: f32,
    /// Per second with HP low (while it fell lately).
    pub hp_low_per_second: f32,
    pub death: f32,
    pub scene_cut: f32,
    /// Per second, times the scene's activity (0–1).
    pub activity_per_second: f32,
}

#[derive(Debug, Deserialize)]
struct Band {
    id: Tempo,
    at_least: f32,
}

/// The tuning, read once from `tempo.json`.
#[derive(Debug, Deserialize)]
pub struct Tuning {
    /// The intensity is multiplied by this each second.
    pub decay_per_second: f32,
    /// After HP fell, the decay waits this long, in seconds.
    pub hold_decay_while_losing_hp_for_s: f64,
    pub events: Events,
    /// Below this much going on, nobody is playing.
    pub activity_floor: f32,
    /// The intensity's ceiling (a long fight does not take minutes to
    /// come down from).
    pub at_most: f32,
    /// Nothing going on for this long (nor the game in view) is away.
    pub away_after_s: f64,
    /// The bands, highest first; the first whose floor the intensity
    /// reaches is the tempo.
    bands: Vec<Band>,
}

impl Tuning {
    pub fn get() -> &'static Tuning {
        static TUNING: OnceLock<Tuning> = OnceLock::new();
        TUNING.get_or_init(|| {
            serde_json::from_str(DATA).unwrap_or_else(|e| panic!("tempo.json: {e}"))
        })
    }

    /// The band `intensity` falls in.
    pub fn band(&self, intensity: f32) -> Tempo {
        self.bands
            .iter()
            .find(|b| intensity >= b.at_least)
            .map(|b| b.id)
            .unwrap_or(Tempo::Calm)
    }
}

/// The intensity as it runs, frame by frame.
#[derive(Debug)]
pub struct Intensity {
    value: f32,
    last: Option<f64>,
    last_hp: Option<f32>,
    /// Until when the decay waits (HP fell lately).
    losing_until: f64,
    /// Since when nothing has been going on.
    quiet_since: Option<f64>,
    was_dead: bool,
}

impl Default for Intensity {
    fn default() -> Self {
        Self::new()
    }
}

impl Intensity {
    pub fn new() -> Self {
        Self {
            value: 0.0,
            last: None,
            last_hp: None,
            losing_until: f64::NEG_INFINITY,
            quiet_since: None,
            was_dead: false,
        }
    }

    /// One frame: the intensity moves, and the tempo it makes is returned.
    pub fn observe(&mut self, w: &World) -> Tempo {
        let t = Tuning::get();
        let now = w.now;
        let dt = self
            .last
            .map(|last| (now - last).clamp(0.0, 1.0) as f32)
            .unwrap_or(0.0);
        self.last = Some(now);
        if !w.seen {
            // Out of sight: the intensity fades, and nobody is playing.
            self.value *= t.decay_per_second.powf(dt);
            self.last_hp = None;
            let since = *self.quiet_since.get_or_insert(now);
            return if now - since >= t.away_after_s {
                Tempo::Away
            } else {
                t.band(self.value)
            };
        }
        // Events this frame.
        if let (Some(hp), Some(before)) = (w.hp, self.last_hp)
            && hp < before
            && !w.dead
        {
            self.value += (before - hp) * t.events.hp_lost_per_percent;
            self.losing_until = now + t.hold_decay_while_losing_hp_for_s;
        }
        // Low HP in a fight (HP fell lately) keeps the tension up; standing
        // at low HP in town does not — the companion's warnings are for
        // that.
        if let Some(hp) = w.hp
            && hp < t.events.hp_low_percent
            && !w.dead
            && now < self.losing_until
        {
            self.value += t.events.hp_low_per_second * dt;
        }
        if w.dead && !self.was_dead {
            self.value += t.events.death;
        }
        self.was_dead = w.dead;
        if w.scene_cut {
            self.value += t.events.scene_cut;
        }
        self.value += w.activity.clamp(0.0, 1.0) * t.events.activity_per_second * dt;
        self.last_hp = w.hp;
        // Decay, unless HP is still falling.
        if now >= self.losing_until {
            self.value *= t.decay_per_second.powf(dt);
        }
        self.value = self.value.clamp(0.0, t.at_most);
        // Away: nothing going on for a while, nobody talking.
        if w.activity <= t.activity_floor && !w.talking {
            let since = *self.quiet_since.get_or_insert(now);
            if now - since >= t.away_after_s {
                return Tempo::Away;
            }
        } else {
            self.quiet_since = None;
        }
        t.band(self.value)
    }

    /// The intensity now (0 upward).
    pub fn value(&self) -> f32 {
        self.value
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(now: f64, hp: f32, activity: f32) -> World {
        World {
            now,
            seen: true,
            hp: Some(hp),
            activity,
            ..World::default()
        }
    }

    #[test]
    fn the_tuning_reads_and_its_bands_run_from_peak_down_to_calm() {
        let t = Tuning::get();
        assert!(t.decay_per_second > 0.0 && t.decay_per_second < 1.0);
        assert!(t.bands.len() >= 3);
        assert_eq!(t.bands.first().map(|b| b.id), Some(Tempo::Peak));
        assert_eq!(t.bands.last().map(|b| b.id), Some(Tempo::Calm));
        for pair in t.bands.windows(2) {
            assert!(pair[0].at_least > pair[1].at_least, "{pair:?}");
        }
        assert_eq!(t.band(0.0), Tempo::Calm);
        assert_eq!(t.band(10.0), Tempo::Peak);
        assert_eq!(Tempo::named(" Combat "), Some(Tempo::Combat));
        assert_eq!(Tempo::named("loud"), None);
        assert!(Tempo::Calm < Tempo::Travel && Tempo::Travel < Tempo::Combat);
    }

    #[test]
    fn standing_in_town_is_calm_and_a_fight_climbs_to_combat_then_fades() {
        let mut i = Intensity::new();
        // A near-still screen, full HP: calm. (The scene's activity is the
        // mean frame-to-frame change: a scrolling camera is a few
        // hundredths; a fight with effects more.)
        for s in 0..20 {
            assert_eq!(i.observe(&frame(s as f64, 100.0, 0.004)), Tempo::Calm);
        }
        // Walking: travel, not combat.
        let mut t = Tempo::Calm;
        for s in 20..60 {
            t = i.observe(&frame(s as f64, 100.0, 0.025));
        }
        assert_eq!(t, Tempo::Travel);
        // A fight: the picture moves more and HP drops in bites.
        let mut hp = 100.0;
        let mut tempos = Vec::new();
        for s in 60..90 {
            if s % 3 == 0 {
                hp -= 9.0;
            }
            tempos.push(i.observe(&frame(s as f64, hp, 0.06)));
        }
        assert!(
            tempos.iter().any(|t| *t >= Tempo::Combat),
            "never reached combat: {tempos:?}"
        );
        // Standing still again, HP steady: it fades back to calm within a
        // minute or two.
        for s in 90..240 {
            t = i.observe(&frame(s as f64, hp, 0.004));
        }
        assert_eq!(t, Tempo::Calm);
    }

    #[test]
    fn a_death_is_a_peak_and_the_decay_waits_while_hp_falls() {
        let mut i = Intensity::new();
        i.observe(&frame(0.0, 100.0, 0.1));
        let mut w = frame(1.0, 0.0, 0.1);
        w.dead = true;
        assert_eq!(i.observe(&w), Tempo::Peak);
        // Falling HP holds the intensity where it is for a few seconds.
        let mut j = Intensity::new();
        j.observe(&frame(0.0, 100.0, 0.5));
        let before = j.value();
        j.observe(&frame(1.0, 80.0, 0.5));
        let after_drop = j.value();
        assert!(after_drop > before);
        j.observe(&frame(2.0, 80.0, 0.0));
        assert!(j.value() >= after_drop - 1e-6, "decayed while losing hp");
    }

    #[test]
    fn nothing_going_on_for_long_enough_is_away_and_a_hidden_game_too() {
        let mut i = Intensity::new();
        let t = Tuning::get();
        let mut tempo = Tempo::Calm;
        for s in 0..(t.away_after_s as i64 + 5) {
            tempo = i.observe(&frame(s as f64, 100.0, 0.0));
        }
        assert_eq!(tempo, Tempo::Away);
        // The player moving again ends it at once.
        assert_eq!(i.observe(&frame(500.0, 100.0, 0.3)), Tempo::Calm);
        let mut hidden = Intensity::new();
        let mut w = frame(0.0, 100.0, 0.0);
        w.seen = false;
        for s in 0..(t.away_after_s as i64 + 5) {
            w.now = s as f64;
            tempo = hidden.observe(&w);
        }
        assert_eq!(tempo, Tempo::Away);
    }
}
