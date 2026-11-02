//! What MapleSyrup may have a thought about, and what it likes: data.
//!
//! A hook is a rule in the shape of Valve's response system (Ruskin, GDC
//! 2012): a moment it fires at (`when`), where it applies (`where`), a
//! topic (for the cooldowns), the facts it may lean on and the notes
//! behind them, each with its source. The code knows ids, numbers and
//! tags; every word said comes from the model, phrasing the hook in its
//! own voice and the player's language (`hooks.json`).
//!
//! The persona (`persona.json`) is MapleSyrup's taste card: what it loves,
//! likes, shrugs at, and why, in tags. It is data so that a stance holds
//! across a session and across model changes (persona drift sets in within
//! a few turns otherwise), and so that agreement is said only where the
//! card really agrees — the one thing that tells a friend from a flatterer.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::tempo::Tempo;
use super::world::{Boundary, World};

const HOOKS: &str = include_str!("hooks.json");
const PERSONA: &str = include_str!("persona.json");

/// The moment a hook fires at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Trigger {
    /// The game came (back) into view.
    GameSeen,
    MapEntered,
    LevelUp,
    Revived,
    /// The player spoke after a long quiet.
    PlayerBack,
    /// Calm for a while (`where.calm_for_s`).
    CalmStretch,
}

impl Trigger {
    /// The trigger a boundary of play is.
    pub fn of(boundary: Boundary) -> Option<Trigger> {
        match boundary {
            Boundary::GameSeen => Some(Trigger::GameSeen),
            Boundary::MapEntered => Some(Trigger::MapEntered),
            Boundary::LevelUp => Some(Trigger::LevelUp),
            Boundary::Revived => Some(Trigger::Revived),
            Boundary::PlayerBack => Some(Trigger::PlayerBack),
            // A cut alone says nothing about where they are.
            Boundary::SceneCut => None,
        }
    }

    /// Its name in the data and the log.
    pub fn word(self) -> &'static str {
        match self {
            Trigger::GameSeen => "game_seen",
            Trigger::MapEntered => "map_entered",
            Trigger::LevelUp => "level_up",
            Trigger::Revived => "revived",
            Trigger::PlayerBack => "player_back",
            Trigger::CalmStretch => "calm_stretch",
        }
    }
}

/// What kind of thought a hook makes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// Something to do next, in first person plural.
    Suggestion,
    /// A thing noticed about where they are.
    Remark,
    /// What it thinks of something, from its taste card.
    Opinion,
    /// Picks up what they once said they like (the mind knows it from the
    /// notebook; any hook's thought may be one).
    Callback,
    /// A short word of company.
    Quip,
}

impl Kind {
    pub fn word(self) -> &'static str {
        match self {
            Kind::Suggestion => "suggestion",
            Kind::Remark => "remark",
            Kind::Opinion => "opinion",
            Kind::Callback => "callback",
            Kind::Quip => "quip",
        }
    }
}

/// Which world a hook holds in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GameWorld {
    #[default]
    Any,
    Classic,
    Live,
}

/// Where a hook applies. Everything is optional; what is left out does not
/// constrain.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Where {
    /// The map must be one of these (as the sight reads its name).
    pub maps: Vec<String>,
    pub level_min: Option<u32>,
    pub level_max: Option<u32>,
    pub world: GameWorld,
    /// The game may be no tenser than this.
    pub tempo_at_most: Option<Tempo>,
    /// For `calm_stretch`: calm for at least this long, in seconds.
    pub calm_for_s: Option<f64>,
    /// The session must have run at least this long, in seconds.
    pub session_at_least_s: Option<f64>,
}

/// One rule.
#[derive(Debug, Clone, Deserialize)]
pub struct Hook {
    pub id: String,
    pub when: Trigger,
    #[serde(default, rename = "where")]
    pub place: Where,
    pub topic: String,
    pub kind: Kind,
    /// How interesting it is in itself, 1–10.
    pub interest: f32,
    /// The chance it is thought about at all when it matches (0–1): a
    /// hook that fired every time would be a script.
    pub odds: f32,
    #[serde(default)]
    pub once_per_session: bool,
    /// What the thought may lean on, by name.
    #[serde(default)]
    pub facts: BTreeMap<String, Value>,
    /// What is known about it, for the model (sourced below).
    #[serde(default)]
    pub notes: Vec<String>,
    #[serde(default)]
    pub sources: Vec<String>,
    /// A melody of MapleSyrup's own it may hum after this thought
    /// (`melodies.json`).
    #[serde(default)]
    pub hum: Option<String>,
}

impl Hook {
    /// Whether it applies to `world` at `tempo`, calm for `calm_for_s`.
    pub fn applies(&self, world: &World, tempo: Tempo, calm_for_s: f64) -> bool {
        let w = &self.place;
        if !w.maps.is_empty() {
            let Some(map) = world.map() else {
                return false;
            };
            if !w.maps.iter().any(|m| same_place(m, map)) {
                return false;
            }
        }
        if let Some(min) = w.level_min
            && world.level.is_none_or(|l| l < min)
        {
            return false;
        }
        if let Some(max) = w.level_max
            && world.level.is_none_or(|l| l > max)
        {
            return false;
        }
        match w.world {
            GameWorld::Any => {}
            GameWorld::Classic if !world.classic => return false,
            GameWorld::Live if world.classic => return false,
            _ => {}
        }
        if let Some(at_most) = w.tempo_at_most
            && (tempo > at_most || tempo == Tempo::Away)
        {
            return false;
        }
        if let Some(calm) = w.calm_for_s
            && calm_for_s < calm
        {
            return false;
        }
        if let Some(least) = w.session_at_least_s
            && world.session_s < least
        {
            return false;
        }
        true
    }

    /// How specific it is: the number of criteria it sets (the most
    /// specific rule wins a tie).
    pub fn specificity(&self) -> u32 {
        let w = &self.place;
        [
            !w.maps.is_empty(),
            w.level_min.is_some(),
            w.level_max.is_some(),
            w.world != GameWorld::Any,
            w.tempo_at_most.is_some(),
            w.calm_for_s.is_some(),
            w.session_at_least_s.is_some(),
        ]
        .into_iter()
        .filter(|set| *set)
        .count() as u32
    }

    /// The music this hook is about, when it names one.
    pub fn music(&self) -> Option<&str> {
        self.facts.get("bgm").and_then(Value::as_str)
    }
}

/// Whether two map names are the same place: as read, give or take case,
/// spaces and a trailing map number ("Henesys Hunting Ground I" is the
/// hunting ground; it is not Henesys).
pub fn same_place(a: &str, b: &str) -> bool {
    fn numeral(word: &str) -> bool {
        word.chars().all(|c| c.is_ascii_digit())
            || word.chars().all(|c| matches!(c, 'i' | 'v' | 'x'))
    }
    fn fold(s: &str) -> String {
        let mut words: Vec<String> = s
            .split(|c: char| !c.is_alphanumeric())
            .filter(|w| !w.is_empty())
            .map(str::to_lowercase)
            .collect();
        if words.len() > 1 && words.last().is_some_and(|w| numeral(w)) {
            words.pop();
        }
        words.concat()
    }
    let (a, b) = (fold(a), fold(b));
    !a.is_empty() && a == b
}

/// A topic, and how long two thoughts on it stay apart.
#[derive(Debug, Clone, Deserialize)]
pub struct Topic {
    pub id: String,
    pub cooldown_s: f64,
}

/// The rules, read once from `hooks.json`.
#[derive(Debug, Deserialize)]
pub struct Hooks {
    topics: Vec<Topic>,
    hooks: Vec<Hook>,
}

impl Hooks {
    pub fn get() -> &'static Hooks {
        static HOOKS_DATA: OnceLock<Hooks> = OnceLock::new();
        HOOKS_DATA.get_or_init(|| {
            serde_json::from_str(HOOKS).unwrap_or_else(|e| panic!("hooks.json: {e}"))
        })
    }

    pub fn all(&self) -> &[Hook] {
        &self.hooks
    }

    pub fn topics(&self) -> &[Topic] {
        &self.topics
    }

    /// The hook named `id`.
    pub fn named(&self, id: &str) -> Option<&Hook> {
        self.hooks.iter().find(|h| h.id == id)
    }

    /// How long two thoughts on `topic` stay apart, in seconds (a topic not
    /// in the data: [`DEFAULT_COOLDOWN_S`]).
    pub fn cooldown(&self, topic: &str) -> f64 {
        self.topics
            .iter()
            .find(|t| t.id == topic)
            .map(|t| t.cooldown_s)
            .unwrap_or(DEFAULT_COOLDOWN_S)
    }

    /// The hooks that fire at `trigger` and apply to the moment, the most
    /// specific first.
    pub fn matching(
        &self,
        trigger: Trigger,
        world: &World,
        tempo: Tempo,
        calm_for_s: f64,
    ) -> Vec<&Hook> {
        let mut found: Vec<&Hook> = self
            .hooks
            .iter()
            .filter(|h| h.when == trigger && h.applies(world, tempo, calm_for_s))
            .collect();
        found.sort_by(|a, b| {
            b.specificity()
                .cmp(&a.specificity())
                .then(b.interest.total_cmp(&a.interest))
        });
        found
    }
}

/// A topic the data does not pace: this far apart, in seconds.
pub const DEFAULT_COOLDOWN_S: f64 = 300.0;

/// How MapleSyrup feels about one thing.
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct Taste {
    /// `music:<title>`, `place:<map>`, `activity:<what>`.
    pub about: String,
    pub stance: String,
    /// How strongly it is held, 1–5.
    pub strength: u8,
    #[serde(default)]
    pub because: Vec<String>,
}

impl Taste {
    /// The kind of thing it is about (`music`, `place`, `activity`) and the
    /// thing itself.
    pub fn split(&self) -> (&str, &str) {
        self.about
            .split_once(':')
            .map(|(k, v)| (k.trim(), v.trim()))
            .unwrap_or(("", self.about.trim()))
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Character {
    pub name: String,
    #[serde(default)]
    pub traits: Vec<String>,
    #[serde(default)]
    pub habits: Vec<String>,
}

/// The taste card, read once from `persona.json`.
#[derive(Debug, Deserialize)]
pub struct Persona {
    pub character: Character,
    tastes: Vec<Taste>,
    /// The stances a taste may take, warmest first.
    stances: Vec<String>,
}

impl Persona {
    pub fn get() -> &'static Persona {
        static PERSONA_DATA: OnceLock<Persona> = OnceLock::new();
        PERSONA_DATA.get_or_init(|| {
            serde_json::from_str(PERSONA).unwrap_or_else(|e| panic!("persona.json: {e}"))
        })
    }

    pub fn tastes(&self) -> &[Taste] {
        &self.tastes
    }

    pub fn stances(&self) -> &[String] {
        &self.stances
    }

    /// What it feels about `thing` (a title, a map, an activity), by name.
    pub fn about(&self, thing: &str) -> Option<&Taste> {
        self.tastes.iter().find(|t| same_place(t.split().1, thing))
    }

    /// The tastes that bear on a hook: the place it is about and the music
    /// it names.
    pub fn for_hook<'a>(&'a self, hook: &Hook, world: &World) -> Vec<&'a Taste> {
        let mut found = Vec::new();
        if let Some(music) = hook.music()
            && let Some(t) = self.about(music)
        {
            found.push(t);
        }
        if let Some(map) = world.map()
            && hook.place.maps.iter().any(|m| same_place(m, map))
            && let Some(t) = self
                .tastes
                .iter()
                .find(|t| t.split().0 == "place" && same_place(t.split().1, map))
        {
            found.push(t);
        }
        found
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::intuition::world::Place;

    fn world_in(map: &str, level: u32, classic: bool) -> World {
        World {
            now: 1000.0,
            seen: true,
            place: Some(Place {
                name: map.into(),
                age_s: 3.0,
            }),
            level: Some(level),
            classic,
            session_s: 1200.0,
            ..World::default()
        }
    }

    #[test]
    fn the_hooks_read_and_every_one_is_sound() {
        let hooks = Hooks::get();
        assert!(hooks.all().len() >= 8);
        let mut ids: Vec<&str> = hooks.all().iter().map(|h| h.id.as_str()).collect();
        ids.sort_unstable();
        let n = ids.len();
        ids.dedup();
        assert_eq!(ids.len(), n, "a hook id repeats");
        for hook in hooks.all() {
            assert!(
                hooks.topics().iter().any(|t| t.id == hook.topic),
                "{}: topic {} is not in the data",
                hook.id,
                hook.topic
            );
            assert!((1.0..=10.0).contains(&hook.interest), "{}", hook.id);
            assert!((0.0..=1.0).contains(&hook.odds), "{}", hook.id);
            // A fact leans on a source; a hook with none has no facts.
            if !hook.facts.is_empty() && hook.when != Trigger::CalmStretch {
                assert!(
                    !hook.sources.is_empty(),
                    "{} names facts without a source",
                    hook.id
                );
            }
            // Nothing in the data is a sentence to say: notes are for the
            // model, facts are names and numbers.
            for (key, value) in &hook.facts {
                assert!(!key.contains(' '), "{}: fact key {key:?}", hook.id);
                assert!(!value.is_null(), "{}: {key}", hook.id);
            }
            // No composer is credited anywhere (none was found).
            for note in &hook.notes {
                assert!(
                    !note.to_lowercase().contains("composer"),
                    "{}: {note}",
                    hook.id
                );
            }
        }
        for topic in hooks.topics() {
            assert!(topic.cooldown_s >= 60.0, "{}", topic.id);
        }
        assert_eq!(hooks.cooldown("music"), 540.0);
        assert_eq!(hooks.cooldown("nothing"), DEFAULT_COOLDOWN_S);
    }

    #[test]
    fn a_map_hook_fires_on_its_map_at_a_calm_tempo_and_nowhere_else() {
        let hooks = Hooks::get();
        let ellinia = world_in("Ellinia", 25, true);
        let found = hooks.matching(Trigger::MapEntered, &ellinia, Tempo::Calm, 0.0);
        assert_eq!(
            found.iter().map(|h| h.id.as_str()).collect::<Vec<_>>(),
            ["ellinia-music"]
        );
        // In a fight, nothing.
        assert!(
            hooks
                .matching(Trigger::MapEntered, &ellinia, Tempo::Combat, 0.0)
                .is_empty()
        );
        // Away, nothing.
        assert!(
            hooks
                .matching(Trigger::MapEntered, &ellinia, Tempo::Away, 0.0)
                .is_empty()
        );
        // Another map: its own hooks.
        let henesys = world_in("Henesys", 25, true);
        let found = hooks.matching(Trigger::MapEntered, &henesys, Tempo::Travel, 0.0);
        assert_eq!(
            found.iter().map(|h| h.id.as_str()).collect::<Vec<_>>(),
            ["henesys-music"]
        );
        // The hunting grounds are read with their number.
        let ground = world_in("Henesys Hunting Ground II", 25, true);
        let found = hooks.matching(Trigger::MapEntered, &ground, Tempo::Travel, 0.0);
        assert_eq!(found[0].id, "henesys-hunting-ground-music");
        // A stale map name is nowhere.
        let mut stale = ellinia.clone();
        stale.place.as_mut().unwrap().age_s = 1e6;
        assert!(
            hooks
                .matching(Trigger::MapEntered, &stale, Tempo::Calm, 0.0)
                .is_empty()
        );
    }

    #[test]
    fn the_maps_of_classic_world_are_matched_as_the_minimap_names_them() {
        let hooks = Hooks::get();
        let first = |name: &str, level: u32| {
            hooks
                .matching(
                    Trigger::MapEntered,
                    &world_in(name, level, true),
                    Tempo::Travel,
                    0.0,
                )
                .first()
                .map(|h| h.id.clone())
        };
        // Numbered areas and bracketed names fold to their place.
        assert_eq!(
            first("Line 2 <Area 3>", 47).as_deref(),
            Some("kerning-subway")
        );
        assert_eq!(
            first("Transfer Area", 18).as_deref(),
            Some("kerning-subway")
        );
        assert_eq!(
            first("Cabin <To Orbis>", 40).as_deref(),
            Some("airship-ride")
        );
        assert_eq!(
            first("Deep Ant Tunnel II", 26).as_deref(),
            Some("ant-tunnel")
        );
        assert_eq!(first("Ant Tunnel Park", 26).as_deref(), Some("ant-tunnel"));
        assert_eq!(first("Lorang Lorang", 38).as_deref(), Some("florina-beach"));
        assert_eq!(
            first("Sleepy Dungeon V", 58).as_deref(),
            Some("sleepy-dungeon")
        );
        assert_eq!(first("El Nath", 60).as_deref(), Some("el-nath-music"));
        // The town and its dungeon are two places.
        assert_eq!(first("Sleepywood", 40).as_deref(), Some("sleepywood-music"));
        // Every hook about a place carries the tune and a source for it.
        for hook in hooks.all().iter().filter(|h| !h.place.maps.is_empty()) {
            assert!(hook.facts.contains_key("bgm"), "{}", hook.id);
            assert!(!hook.sources.is_empty(), "{}", hook.id);
        }
    }

    #[test]
    fn the_party_quest_suggestion_holds_only_in_classic_world_at_its_levels() {
        let hooks = Hooks::get();
        let ids = |w: &World| {
            hooks
                .matching(Trigger::LevelUp, w, Tempo::Calm, 0.0)
                .iter()
                .map(|h| h.id.clone())
                .collect::<Vec<_>>()
        };
        // At 22 in Classic World: the party quest first (it is the more
        // specific: a level band of ten against sixty), then the training
        // map for the level.
        assert_eq!(
            ids(&world_in("Henesys", 22, true)),
            ["kerning-pq-suggestion", "training-map-for-the-level"]
        );
        assert!(ids(&world_in("Henesys", 22, false)).is_empty());
        assert_eq!(
            ids(&world_in("Henesys", 45, true)),
            ["training-map-for-the-level"]
        );
        assert!(ids(&world_in("Henesys", 80, true)).is_empty());
        let mut unknown_level = world_in("Henesys", 22, true);
        unknown_level.level = None;
        assert!(ids(&unknown_level).is_empty());
    }

    #[test]
    fn a_calm_stretch_fires_only_after_its_quiet_and_the_most_specific_first() {
        let hooks = Hooks::get();
        let w = world_in("Henesys", 22, true);
        assert!(
            hooks
                .matching(Trigger::CalmStretch, &w, Tempo::Calm, 60.0)
                .is_empty()
        );
        let found = hooks.matching(Trigger::CalmStretch, &w, Tempo::Calm, 300.0);
        let ids: Vec<&str> = found.iter().map(|h| h.id.as_str()).collect();
        assert!(
            ids.contains(&"town-breather") && ids.contains(&"what-next"),
            "{ids:?}"
        );
        // Travel is too tense for a calm stretch.
        assert!(
            hooks
                .matching(Trigger::CalmStretch, &w, Tempo::Travel, 300.0)
                .is_empty()
        );
        let mut young = w.clone();
        young.session_s = 100.0;
        assert!(
            hooks
                .matching(Trigger::CalmStretch, &young, Tempo::Calm, 300.0)
                .is_empty()
        );
    }

    #[test]
    fn the_taste_card_reads_and_knows_what_it_loves() {
        let persona = Persona::get();
        assert!(!persona.character.name.is_empty());
        assert!(persona.tastes().len() >= 8);
        for taste in persona.tastes() {
            assert!(persona.stances().contains(&taste.stance), "{:?}", taste);
            assert!((1..=5).contains(&taste.strength), "{:?}", taste);
            let (kind, thing) = taste.split();
            assert!(
                ["music", "place", "activity"].contains(&kind),
                "{:?}",
                taste
            );
            assert!(!thing.is_empty());
        }
        let morning = persona.about("When The Morning Comes").unwrap();
        assert_eq!(morning.stance, "love");
        assert!(morning.because.iter().any(|b| b == "piano"));
        assert!(persona.about("Elevator Music").is_none());
        // For the Ellinia hook: the music and the place.
        let hooks = Hooks::get();
        let hook = hooks.named("ellinia-music").unwrap();
        let about: Vec<&str> = persona
            .for_hook(hook, &world_in("Ellinia", 30, true))
            .iter()
            .map(|t| t.about.as_str())
            .collect();
        assert_eq!(about, ["music:When The Morning Comes", "place:Ellinia"]);
    }

    #[test]
    fn places_match_as_the_sight_reads_them() {
        assert!(same_place("Henesys", "henesys"));
        assert!(same_place(
            "Henesys Hunting Ground I",
            "Henesys Hunting Ground"
        ));
        assert!(same_place(
            "Henesys Hunting Ground I",
            "Henesys Hunting Ground 2"
        ));
        assert!(same_place("Kerning City", "KerningCity"));
        assert!(!same_place("Henesys Hunting Ground I", "Henesys"));
        assert!(!same_place("Ellinia", "Perion"));
        assert!(!same_place("", ""));
    }
}
