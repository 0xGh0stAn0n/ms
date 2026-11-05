//! What the intuition knows of the moment: the game as the vision engine
//! reads it, and the room — who is talking, for how long nobody has.
//!
//! The main loop builds one [`World`] per frame from what it already has
//! (the companion's observation, the sight's facts, the scene verdict, the
//! turns) and hands it to the intuition. Nothing in here reads the screen:
//! it is a view, so the thinking can be replayed and tested from numbers.
//!
//! [`Trail`] keeps the last frame and names what changed between the two —
//! a map entered, a level gained, the character back from the dead, the
//! game back in view. Those are the breakpoints (Iqbal & Bailey, CHI 2008:
//! a message deferred to a boundary of the task frustrates far less than
//! one dropped into the middle of it), and the moments a thought is
//! welcome.

use serde::{Deserialize, Serialize};

/// Where the character is, as last read off the minimap (the sight reads
/// it now and then, not every frame), and how old that reading is.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Place {
    pub name: String,
    /// Seconds since the name was read.
    pub age_s: f64,
}

/// The moment, as the intuition sees it. Written to the session's trace
/// as JSON ([`super::trace`]), so a session can be replayed from numbers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct World {
    pub now: f64,
    /// The game is captured and in view.
    pub seen: bool,
    pub place: Option<Place>,
    pub level: Option<u32>,
    /// HP in percent, when read this frame.
    pub hp: Option<f32>,
    /// The character is dead (HP at zero), until it comes back.
    pub dead: bool,
    /// How much the picture moves (the scene fingerprint's activity, 0–1).
    pub activity: f32,
    /// The picture cut and settled this frame (a portal, a dialog…).
    pub scene_cut: bool,
    /// Someone is talking: the player, or MapleSyrup (a reply being made
    /// or said), or a look is under way.
    pub talking: bool,
    /// Seconds since the player last said anything (`None`: not yet).
    pub since_player_spoke: Option<f64>,
    /// Seconds since anyone said anything (the player, a reply, an alert);
    /// infinite when nobody has yet (written as null: JSON has no
    /// infinity).
    #[serde(with = "endless")]
    pub since_anyone_spoke: f64,
    /// The companion holds its alerts (nothing the player does answers
    /// them: away, or the readings are wrong), or is muted.
    pub held: bool,
    pub muted: bool,
    /// How long the session has run, in seconds.
    pub session_s: f64,
    /// They play MapleStory Classic World.
    pub classic: bool,
}

impl Default for World {
    fn default() -> Self {
        Self {
            now: 0.0,
            seen: false,
            place: None,
            level: None,
            hp: None,
            dead: false,
            activity: 0.0,
            scene_cut: false,
            talking: false,
            since_player_spoke: None,
            since_anyone_spoke: f64::INFINITY,
            held: false,
            muted: false,
            session_s: 0.0,
            classic: false,
        }
    }
}

/// A number that may be infinite, as JSON: null for infinity.
mod endless {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(value: &f64, s: S) -> Result<S::Ok, S::Error> {
        if value.is_finite() {
            s.serialize_some(value)
        } else {
            s.serialize_none()
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<f64, D::Error> {
        Ok(Option::<f64>::deserialize(d)?.unwrap_or(f64::INFINITY))
    }
}

/// A map name read longer ago than this is where they were, not where they
/// are (the sight says as much), in seconds.
pub const PLACE_FRESH_S: f64 = 600.0;

impl World {
    /// The map they are on, when the reading is young enough to trust.
    pub fn map(&self) -> Option<&str> {
        self.place
            .as_ref()
            .filter(|p| p.age_s <= PLACE_FRESH_S)
            .map(|p| p.name.as_str())
    }
}

/// What changed since the last frame: the boundaries of play.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Boundary {
    /// The game came (back) into view.
    GameSeen,
    /// A fresh reading names another map than the last reading did (the
    /// same name read again, however long after, is not a boundary: the
    /// sight refreshes a name now and then while they stay).
    MapEntered,
    /// The level went up.
    LevelUp,
    /// Alive again after a death.
    Revived,
    /// The picture cut and settled (a portal, a dialog, a cutscene).
    SceneCut,
    /// The player spoke after a long quiet.
    PlayerBack,
}

/// How long the player is quiet before their next words count as coming
/// back, in seconds.
pub const PLAYER_BACK_AFTER_S: f64 = 300.0;

/// The last frame, to name what changed in this one.
#[derive(Debug, Default)]
pub struct Trail {
    last: Option<World>,
}

impl Trail {
    pub fn new() -> Self {
        Self::default()
    }

    /// The boundaries crossed between the last frame and `world`, then
    /// `world` becomes the last frame.
    pub fn step(&mut self, world: &World) -> Vec<Boundary> {
        let mut crossed = Vec::new();
        if let Some(last) = &self.last {
            if world.seen && !last.seen {
                crossed.push(Boundary::GameSeen);
            }
            if let Some(map) = world.map()
                && !last
                    .place
                    .as_ref()
                    .is_some_and(|p| super::hooks::same_place(&p.name, map))
            {
                crossed.push(Boundary::MapEntered);
            }
            if let (Some(now), Some(before)) = (world.level, last.level)
                && now > before
            {
                crossed.push(Boundary::LevelUp);
            }
            if last.dead && !world.dead {
                crossed.push(Boundary::Revived);
            }
            if world.scene_cut && !last.scene_cut {
                crossed.push(Boundary::SceneCut);
            }
            if let (Some(now), Some(before)) = (world.since_player_spoke, last.since_player_spoke)
                && now < before
                && before >= PLAYER_BACK_AFTER_S
            {
                crossed.push(Boundary::PlayerBack);
            }
        } else if world.seen {
            crossed.push(Boundary::GameSeen);
        }
        self.last = Some(world.clone());
        crossed
    }

    /// The last frame seen.
    pub fn last(&self) -> Option<&World> {
        self.last.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(now: f64) -> World {
        World {
            now,
            seen: true,
            ..World::default()
        }
    }

    fn in_map(now: f64, name: &str, age_s: f64) -> World {
        World {
            place: Some(Place {
                name: name.into(),
                age_s,
            }),
            ..at(now)
        }
    }

    #[test]
    fn a_fresh_map_name_is_a_boundary_and_a_stale_one_is_nowhere() {
        let mut trail = Trail::new();
        assert_eq!(trail.step(&at(0.0)), vec![Boundary::GameSeen]);
        assert_eq!(
            trail.step(&in_map(1.0, "Henesys", 2.0)),
            vec![Boundary::MapEntered]
        );
        // The same map again: nothing crossed.
        assert_eq!(trail.step(&in_map(2.0, "Henesys", 3.0)), vec![]);
        // A new map, read just now.
        assert_eq!(
            trail.step(&in_map(3.0, "Ellinia", 0.5)),
            vec![Boundary::MapEntered]
        );
        // A reading older than ten minutes is where they were: no map, and
        // leaving it is no boundary either.
        let stale = in_map(4.0, "Ellinia", PLACE_FRESH_S + 1.0);
        assert_eq!(stale.map(), None);
        assert_eq!(trail.step(&stale), vec![]);
        // The same name read afresh after that: they never left, so no
        // boundary (grinding a quarter of an hour on one map).
        assert_eq!(trail.step(&in_map(5.0, "Ellinia", 0.0)), vec![]);
        assert_eq!(trail.step(&in_map(6.0, "ellinia", 0.0)), vec![]);
        // Another name read afresh is.
        assert_eq!(
            trail.step(&in_map(7.0, "Perion", 0.0)),
            vec![Boundary::MapEntered]
        );
    }

    #[test]
    fn level_death_and_the_game_coming_back_are_boundaries() {
        let mut trail = Trail::new();
        let mut w = at(0.0);
        w.level = Some(30);
        trail.step(&w);
        w.level = Some(31);
        assert_eq!(trail.step(&w), vec![Boundary::LevelUp]);
        w.dead = true;
        assert_eq!(trail.step(&w), vec![]);
        w.dead = false;
        assert_eq!(trail.step(&w), vec![Boundary::Revived]);
        w.seen = false;
        assert_eq!(trail.step(&w), vec![]);
        w.seen = true;
        assert_eq!(trail.step(&w), vec![Boundary::GameSeen]);
    }

    #[test]
    fn the_player_back_after_a_long_quiet_is_a_boundary_but_chatter_is_not() {
        let mut trail = Trail::new();
        let mut w = at(0.0);
        w.since_player_spoke = Some(PLAYER_BACK_AFTER_S + 10.0);
        trail.step(&w);
        w.since_player_spoke = Some(0.0);
        assert_eq!(trail.step(&w), vec![Boundary::PlayerBack]);
        w.since_player_spoke = Some(8.0);
        trail.step(&w);
        w.since_player_spoke = Some(0.0);
        assert_eq!(trail.step(&w), vec![]);
    }
}
