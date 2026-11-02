//! How wild it talks: a bar on the phone, from a voice for young children
//! at one end to the wildest at the other. The levels are data
//! (`spectrum.json`), not code: each names the voice its own lines are
//! dealt in (one of the three decks of [`Attitude`]) and what the models
//! are told. The open levels never swear (the highest, direct with a
//! little sting); the levels past them open only with the password the
//! player chooses on the page, and the wildest swears.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use serde::Deserialize;
use serde_json::{Value, json};

use super::Attitude;

/// The levels, as `spectrum.json` has them (checked by the tests).
const DATA: &str = include_str!("spectrum.json");

/// A level of the bar.
#[derive(Debug, Clone, Deserialize)]
pub struct Level {
    /// Its name: the phone's, the settings', the words for it on the page.
    pub id: String,
    /// The voice its own lines (warnings, deaths, level-ups) are dealt in.
    pub voice: Attitude,
    /// Whether it opens only with the player's password (the levels past
    /// the highest open one: the wildest swears).
    #[serde(default)]
    pub locked: bool,
    /// How it talks at this level, for the models.
    pub instructions: String,
    /// What it says when the bar lands on this level, in this level's own
    /// words (one, the next each time), by language: the player hears that
    /// it took, in their language when there are lines in it
    /// ([`SET_LINES_FALLBACK`] otherwise).
    #[serde(default)]
    pub said_when_set: BTreeMap<String, Vec<String>>,
}

/// The language every level's lines for when the bar lands on it are in.
pub const SET_LINES_FALLBACK: &str = "en";

impl Level {
    /// The `n`th of its lines for when the bar lands on it (they come round
    /// in turn), in the language of `locale` (the phone's, as "he-IL") when
    /// it has lines in it, else in [`SET_LINES_FALLBACK`]; none if it has
    /// none.
    pub fn set_line(&self, n: usize, locale: Option<&str>) -> Option<&str> {
        let in_language = locale
            .and_then(crate::ai::language::code)
            .and_then(|code| self.said_when_set.get(&code))
            .filter(|lines| !lines.is_empty());
        let lines = in_language.or_else(|| self.said_when_set.get(SET_LINES_FALLBACK))?;
        if lines.is_empty() {
            None
        } else {
            Some(lines[n % lines.len()].as_str())
        }
    }
}

/// The bar: its levels, gentlest first.
#[derive(Debug, Deserialize)]
pub struct Spectrum {
    /// Where the bar starts, for a player who never moved it.
    default: String,
    levels: Vec<Level>,
}

impl Spectrum {
    /// The bar, read once from `spectrum.json`.
    pub fn get() -> &'static Spectrum {
        static SPECTRUM: OnceLock<Spectrum> = OnceLock::new();
        SPECTRUM.get_or_init(|| {
            serde_json::from_str(DATA).unwrap_or_else(|e| panic!("spectrum.json: {e}"))
        })
    }

    /// Its levels, gentlest first.
    pub fn levels(&self) -> &[Level] {
        &self.levels
    }

    /// The level at `index`; past the end, the wildest.
    pub fn level(&self, index: usize) -> &Level {
        &self.levels[index.min(self.levels.len() - 1)]
    }

    /// Where the level named `id` is on the bar.
    pub fn index(&self, id: &str) -> Option<usize> {
        let id = id.trim();
        self.levels
            .iter()
            .position(|l| l.id.eq_ignore_ascii_case(id))
    }

    /// Where the bar starts.
    pub fn default_index(&self) -> usize {
        self.index(&self.default).unwrap_or(0)
    }

    /// Where `voice`'s own level is: the one named after it (each voice has
    /// one), else the first in that voice, else where the bar starts.
    pub fn voice_index(&self, voice: Attitude) -> usize {
        self.index(voice.word())
            .or_else(|| self.levels.iter().position(|l| l.voice == voice))
            .unwrap_or_else(|| self.default_index())
    }

    /// The level that is `voice`'s own ([`voice_index`](Spectrum::voice_index)).
    pub fn of_voice(&self, voice: Attitude) -> &Level {
        self.level(self.voice_index(voice))
    }

    /// The names of the levels, gentlest first.
    pub fn ids(&self) -> Vec<&str> {
        self.levels.iter().map(|l| l.id.as_str()).collect()
    }

    /// Whether some levels open only with the player's password.
    pub fn has_locked(&self) -> bool {
        self.levels.iter().any(|l| l.locked)
    }

    /// Where it talks when the bar is set at `index`: there, or, while the
    /// locked levels are shut, at the nearest open level below it (the
    /// first level when none is).
    pub fn open_at(&self, index: usize, unlocked: bool) -> usize {
        let index = index.min(self.levels.len() - 1);
        if unlocked {
            return index;
        }
        (0..=index)
            .rev()
            .find(|&at| !self.levels[at].locked)
            .unwrap_or(0)
    }

    /// The bar as the page draws it: each level's name, the locked ones
    /// only while they are open.
    pub fn for_phone(&self, unlocked: bool) -> Value {
        Value::Array(
            self.levels
                .iter()
                .filter(|l| unlocked || !l.locked)
                .map(|l| json!({"id": l.id}))
                .collect(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bar_starts_where_it_says_and_no_open_level_swears() {
        let bar = Spectrum::get();
        assert!(bar.levels().len() >= 3, "{:?}", bar.ids());
        // Names are distinct, and the default is one of them.
        let mut ids = bar.ids();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), bar.levels().len(), "{:?}", bar.ids());
        assert!(bar.index(&bar.default).is_some(), "{}", bar.default);
        // Every level tells the models how to talk; every open one, that
        // it never swears: those are everyone's, children included. The
        // locked ones are past all the open ones.
        for level in bar.levels() {
            assert!(
                level.instructions.split_whitespace().count() > 10,
                "{}",
                level.id
            );
            assert!(
                level.locked
                    || level.instructions.contains("no swearing")
                    || level.instructions.contains("No swearing"),
                "{} may swear: {}",
                level.id,
                level.instructions
            );
        }
        let first_locked = bar.levels().iter().position(|l| l.locked).unwrap();
        assert!(bar.levels()[first_locked..].iter().all(|l| l.locked));
        assert!(first_locked >= 3, "{:?}", bar.ids());
    }

    #[test]
    fn each_voice_has_a_level_of_its_own_and_the_ends_go_further_than_the_voices() {
        let bar = Spectrum::get();
        for voice in Attitude::ALL {
            let level = bar.of_voice(voice);
            assert_eq!(level.voice, voice);
            assert_eq!(level.id, voice.word());
        }
        // The gentlest end is for young children: nothing rude at all, not
        // even mild; the highest open level is direct with a little sting;
        // the wildest, behind the password, swears, and keeps its limits.
        let gentlest = &bar.levels()[0];
        let wildest = bar.levels().last().unwrap();
        assert!(
            gentlest.instructions.contains("under ten"),
            "{}",
            gentlest.instructions
        );
        assert!(
            gentlest
                .instructions
                .contains("no swearing or rude words of any kind")
        );
        let highest_open = bar.levels().iter().rev().find(|l| !l.locked).unwrap();
        assert!(
            highest_open
                .instructions
                .contains("Direct, with a little sting"),
            "{}",
            highest_open.instructions
        );
        assert!(wildest.locked);
        assert!(
            wildest
                .instructions
                .contains("behind the player's own password")
        );
        assert!(wildest.instructions.contains("curse in every sentence"));
        for limit in [
            "no slurs",
            "never tell them to hurt themselves",
            "no real-world threats",
        ] {
            assert!(wildest.instructions.contains(limit), "{limit}");
        }
    }

    #[test]
    fn a_level_is_found_by_its_name_and_the_phone_gets_them_all() {
        let bar = Spectrum::get();
        let wildest = bar.levels().len() - 1;
        assert_eq!(bar.level(wildest + 5).id, bar.level(wildest).id);
        // Named either way the phone or a setting may have it.
        let savage = bar.of_voice(Attitude::Savage);
        assert_eq!(
            bar.index(&format!(" {} ", savage.id.to_uppercase())),
            bar.index(&savage.id)
        );
        assert_eq!(bar.index("loud"), None);
        // The page gets the names, gentlest first: the locked ones only
        // while they are open.
        let open = bar.for_phone(false);
        let all = bar.for_phone(true);
        assert_eq!(all.as_array().unwrap().len(), bar.levels().len());
        assert_eq!(
            open.as_array().unwrap().len(),
            bar.levels().iter().filter(|l| !l.locked).count()
        );
        assert_eq!(open[0]["id"], bar.levels()[0].id.as_str());
    }

    #[test]
    fn a_bar_set_on_a_locked_level_talks_at_the_highest_open_one_until_unlocked() {
        let bar = Spectrum::get();
        let wildest = bar.levels().len() - 1;
        let highest_open = bar.levels().iter().rposition(|l| !l.locked).unwrap();
        assert!(bar.has_locked());
        assert_eq!(bar.open_at(wildest, false), highest_open);
        assert_eq!(bar.open_at(wildest, true), wildest);
        assert_eq!(bar.open_at(wildest + 9, false), highest_open);
        for at in 0..=highest_open {
            assert_eq!(bar.open_at(at, false), at);
        }
    }

    #[test]
    fn each_level_says_it_took_in_its_own_words_and_the_players_language() {
        for level in Spectrum::get().levels() {
            // English always, and Hebrew (a call in Hebrew heard "Yay, kids
            // mode!" when the bar moved, 1 Nov 2026).
            for code in [SET_LINES_FALLBACK, "he"] {
                let lines = &level.said_when_set[code];
                let n = lines.len();
                assert!(
                    n > 0,
                    "{} says nothing in {code} when the bar lands on it",
                    level.id
                );
                assert!(lines.iter().all(|l| !l.trim().is_empty()), "{}", level.id);
                let locale = Some(code);
                assert_eq!(
                    level.set_line(0, locale),
                    level.set_line(n, locale),
                    "{}",
                    level.id
                );
                assert_eq!(level.set_line(n - 1, locale), Some(lines[n - 1].as_str()));
            }
            // By the phone's locale; a language with no lines of its own
            // hears English, as does a phone with no language.
            let hebrew = level.set_line(0, Some("he-IL")).unwrap();
            assert!(
                hebrew
                    .chars()
                    .any(|c| ('\u{05D0}'..='\u{05EA}').contains(&c)),
                "{hebrew}"
            );
            assert_eq!(level.set_line(0, Some("iw")), Some(hebrew));
            let english = level.said_when_set[SET_LINES_FALLBACK][0].as_str();
            assert_eq!(level.set_line(0, Some("fr-FR")), Some(english));
            assert_eq!(level.set_line(0, None), Some(english));
        }
        let none = Level {
            id: "x".into(),
            voice: Attitude::Blunt,
            locked: false,
            instructions: String::new(),
            said_when_set: BTreeMap::new(),
        };
        assert_eq!(none.set_line(3, Some("he-IL")), None);
    }
}
