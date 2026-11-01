//! How wild it talks: a bar on the phone, from a voice for young children
//! at one end to the wildest at the other. The levels are data
//! (`spectrum.json`), not code: each names the voice its own lines are
//! dealt in (one of the three decks of [`Attitude`]) and what the models
//! are told. No level swears — the wild end roasts in clean words — so the
//! whole bar is open to everyone.

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

    /// The bar as the phone draws it: each level's name.
    pub fn for_phone(&self) -> Value {
        Value::Array(self.levels.iter().map(|l| json!({"id": l.id})).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bar_starts_where_it_says_and_no_level_swears() {
        let bar = Spectrum::get();
        assert!(bar.levels().len() >= 3, "{:?}", bar.ids());
        // Names are distinct, and the default is one of them.
        let mut ids = bar.ids();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), bar.levels().len(), "{:?}", bar.ids());
        assert!(bar.index(&bar.default).is_some(), "{}", bar.default);
        // Every level tells the models how to talk, and that it never
        // swears: the bar is open to everyone, children included.
        for level in bar.levels() {
            assert!(
                level.instructions.split_whitespace().count() > 10,
                "{}",
                level.id
            );
            assert!(
                level.instructions.contains("no swearing")
                    || level.instructions.contains("No swearing"),
                "{} may swear: {}",
                level.id,
                level.instructions
            );
        }
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
        // even mild; the wildest roasts harder than the savage voice, in
        // clean words, and keeps its limits.
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
        assert!(
            wildest
                .instructions
                .contains("over-the-top, theatrical roasting")
        );
        assert!(wildest.instructions.contains("Clean words only"));
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
        // The phone gets the names, gentlest first.
        let phone = bar.for_phone();
        assert_eq!(phone.as_array().unwrap().len(), bar.levels().len());
        assert_eq!(phone[0]["id"], bar.levels()[0].id.as_str());
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
            instructions: String::new(),
            said_when_set: BTreeMap::new(),
        };
        assert_eq!(none.set_line(3, Some("he-IL")), None);
    }
}
