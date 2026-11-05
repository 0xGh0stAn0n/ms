//! The cast of MapleSyrup's story ("One, the boy who stepped on snails"):
//! characters the player can pick as MapleSyrup's voice on the phone. Who
//! they are lives in [`CAST`] (`assets/voices/cast.json`), not here: each
//! one's name and a few words about it in the languages it has, how it
//! sounds, what it says while its voice is made, and what it says once it
//! is MapleSyrup's voice.
//!
//! A character's voice is made on the player's own ElevenLabs account the
//! first time it is picked ([`Eleven::design`]: a voice from a description,
//! no one's recording), named after it and carrying the cast's label, and
//! found there again from then on — by that label, or by its name on an
//! account that kept no labels. Only the characters picked take up the
//! account's voices and credits, and none gets two: the account is looked
//! at again just before a voice is made, and again after a failure.
//!
//! [`Casting`] keeps track of what the player picked (the latest pick is
//! the one that counts) and of the voice being made (one at a time);
//! [`Maker`] makes them, each on a thread of its own.

use std::collections::{BTreeMap, BTreeSet};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, mpsc};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::eleven::{DESIGN_DESCRIPTION, DESIGN_TEXT, Design, Eleven, Voice};
use super::openai::AiError;
use crate::util::plain_id;

/// The cast as it ships, built into the program.
pub const CAST: &str = include_str!("../../assets/voices/cast.json");

/// What stands for a character's name in [`Cast::voice_name`].
pub const NAME_SLOT: &str = "{name}";

/// Why a voice wasn't made when the work on it stopped short (a panic on
/// its thread).
const STOPPED: &str = "the work on it stopped before it was done";

/// The characters, and how their voices are made and found.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct Cast {
    /// The language the cast is written in: every character has its name
    /// and few words in it (a phone in a language a character doesn't have
    /// shows these). Its lines — `sample`, `hello`, and the cast's own
    /// `hello` — are written in English, like MapleSyrup's other lines,
    /// and translated for the player as they are.
    pub language: String,
    /// The label a character's voice carries on the account; its value is
    /// the character's id.
    pub label: String,
    /// The name a character's voice gets on the account, [`NAME_SLOT`]
    /// standing for the character's name.
    pub voice_name: String,
    /// ElevenLabs's voice-design models, tried in order.
    pub design_models: Vec<String>,
    /// What a voice says once it is picked when it is not a character's
    /// (one of the account's own, or OpenAI's).
    pub hello: String,
    pub characters: Vec<Character>,
}

/// One of the characters.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct Character {
    /// How the program and the phone know it.
    pub id: String,
    /// Its name, by language.
    pub name: BTreeMap<String, String>,
    /// A few words about it, by language.
    pub about: BTreeMap<String, String>,
    /// How it sounds: its voice is made from this.
    pub description: String,
    /// What it says while its voice is made.
    pub sample: String,
    /// What it says once it is MapleSyrup's voice.
    pub hello: String,
}

impl Cast {
    /// The cast built into the program.
    pub fn built_in() -> Result<Cast, String> {
        Cast::parse(CAST)
    }

    /// A cast from its JSON, checked: each character has an id the phone
    /// can send, its own; a name and a few words in the cast's language;
    /// how it sounds and what it says while it is made, as long as
    /// ElevenLabs allows; and what it says once it is the voice.
    pub fn parse(text: &str) -> Result<Cast, String> {
        let cast: Cast = serde_json::from_str(text).map_err(|e| e.to_string())?;
        cast.check()?;
        Ok(cast)
    }

    fn check(&self) -> Result<(), String> {
        let blank = |s: &str| s.trim().is_empty();
        if blank(&self.language) || blank(&self.label) || blank(&self.hello) {
            return Err("the cast needs its language, a label and a hello".into());
        }
        if !self.voice_name.contains(NAME_SLOT) {
            return Err(format!("the voices' name must hold {NAME_SLOT}"));
        }
        if self.design_models.is_empty() || self.design_models.iter().any(|m| blank(m)) {
            return Err("the cast needs voice-design models, none of them blank".into());
        }
        let within = |what: &str, text: &str, bounds: &std::ops::RangeInclusive<usize>| {
            let length = text.chars().count();
            if bounds.contains(&length) {
                Ok(())
            } else {
                Err(format!(
                    "its {what} is {length} characters, not {}–{}",
                    bounds.start(),
                    bounds.end()
                ))
            }
        };
        let mut ids = BTreeSet::new();
        for c in &self.characters {
            if !plain_id(&c.id) {
                return Err(format!("{:?}: not an id the phone can send", c.id));
            }
            if !ids.insert(c.id.as_str()) {
                return Err(format!("{}: in the cast twice", c.id));
            }
            for (what, by_language) in [("name", &c.name), ("about", &c.about)] {
                if by_language.get(&self.language).is_none_or(|s| blank(s)) {
                    return Err(format!("{}: no {what} in {}", c.id, self.language));
                }
            }
            if blank(&c.hello) {
                return Err(format!("{}: no hello", c.id));
            }
            within("description", &c.description, &DESIGN_DESCRIPTION)
                .and_then(|()| within("sample", &c.sample, &DESIGN_TEXT))
                .map_err(|why| format!("{}: {why}", c.id))?;
        }
        Ok(())
    }

    pub fn character(&self, id: &str) -> Option<&Character> {
        self.characters.iter().find(|c| c.id == id)
    }

    /// A character's name in the cast's language.
    pub fn name_of<'a>(&self, character: &'a Character) -> &'a str {
        character
            .name
            .get(&self.language)
            .map(String::as_str)
            .unwrap_or(&character.id)
    }

    /// The name a character's voice has on the account.
    pub fn voice_name_of(&self, character: &Character) -> String {
        self.voice_name.replace(NAME_SLOT, self.name_of(character))
    }

    /// The voice on the account that is this character's: the one with its
    /// label, else (an account that kept no labels) the one with its name.
    pub fn voice_of<'a>(&self, character: &Character, voices: &'a [Voice]) -> Option<&'a Voice> {
        voices
            .iter()
            .find(|v| v.labels.get(&self.label) == Some(&character.id))
            .or_else(|| {
                let name = self.voice_name_of(character);
                voices.iter().find(|v| v.name == name)
            })
    }

    /// Makes `character`'s voice on the account behind `eleven`, unless it
    /// is there already (made before, or by a save whose answer was lost):
    /// the account's voices are looked at first, and again after a failure,
    /// so a character never gets two. Nothing is designed, nor its credits
    /// spent, on an account with no room left for another voice.
    pub fn make(&self, character: &Character, eleven: &Eleven) -> Result<Voice, AiError> {
        if let Some(voice) = self.on_account(character, eleven)? {
            return Ok(voice);
        }
        eleven.room_for_a_voice()?;
        let name = self.voice_name_of(character);
        let labels = BTreeMap::from([(self.label.clone(), character.id.clone())]);
        eleven
            .design(&Design {
                description: &character.description,
                text: &character.sample,
                models: &self.design_models,
                name: &name,
                labels: &labels,
            })
            .or_else(|error| match self.on_account(character, eleven) {
                Ok(Some(voice)) => Ok(voice),
                _ => Err(error),
            })
    }

    /// The character's voice as the account has it now, if it has one.
    fn on_account(&self, character: &Character, eleven: &Eleven) -> Result<Option<Voice>, AiError> {
        Ok(self.voice_of(character, &eleven.voices()?).cloned())
    }

    /// The cast for the phone: each character's name and few words in the
    /// languages it has, and its voice on the account (`voices`) when it has
    /// one; and the characters whose voices are being made or are next.
    pub fn status(&self, voices: &[Voice], making: &[&str]) -> Value {
        let characters: Vec<Value> = self
            .characters
            .iter()
            .map(|c| {
                json!({
                    "id": c.id,
                    "name": c.name,
                    "about": c.about,
                    "voice": self.voice_of(c, voices).map(|v| v.id.as_str()),
                })
            })
            .collect();
        json!({
            "language": self.language,
            "making": making,
            "characters": characters,
        })
    }
}

/// What a pick on the phone, or a voice made, comes to.
#[derive(Debug, Clone, PartialEq)]
pub enum Step {
    /// Speak in this voice from now on, and say `hello` in it.
    Speak { voice: String, hello: String },
    /// Make this character's voice ([`Maker::start`]); what comes of it goes
    /// back to [`Casting::made`].
    Make(Character),
}

/// Who MapleSyrup speaks as, as the player picks on the phone: a voice, or
/// a character, whose voice may first have to be made on the account. One
/// voice is made at a time, and the latest pick is the one that counts: a
/// character picked while another's voice is made waits its turn (only the
/// last one picked does), and a voice finished after the player picked
/// something else joins the list without being switched to.
#[derive(Debug, Default)]
pub struct Casting {
    /// The character whose voice is being made.
    making: Option<String>,
    /// The character picked last, while it has no voice yet.
    wanted: Option<String>,
}

impl Casting {
    /// The player picked one of the account's voices, or OpenAI's: it
    /// speaks at once, and a character still waiting for its voice no
    /// longer counts.
    pub fn voice(&mut self, cast: &Cast, voice: &str) -> Vec<Step> {
        self.wanted = None;
        vec![Step::Speak {
            voice: voice.to_string(),
            hello: cast.hello.clone(),
        }]
    }

    /// The player picked a character: it speaks at once in its voice on the
    /// account when it has one; else its voice is made, now or after the one
    /// under way. `voices`: the account's voices, `None` while they are not
    /// known (no account, or not listed yet), when nothing can be told.
    pub fn character(&mut self, cast: &Cast, id: &str, voices: Option<&[Voice]>) -> Vec<Step> {
        let (Some(character), Some(voices)) = (cast.character(id), voices) else {
            return Vec::new();
        };
        if let Some(voice) = cast.voice_of(character, voices) {
            self.wanted = None;
            return vec![Step::Speak {
                voice: voice.id.clone(),
                hello: character.hello.clone(),
            }];
        }
        self.wanted = Some(character.id.clone());
        self.next(cast)
    }

    /// Character `id`'s voice was made (`made`), or couldn't be: it speaks
    /// in it if it is still the player's latest pick, and the character
    /// waiting, if any, is made next. (Adding the voice to the account's
    /// list is the caller's.)
    pub fn made(&mut self, cast: &Cast, id: &str, made: Option<&Voice>) -> Vec<Step> {
        if self.making.as_deref() == Some(id) {
            self.making = None;
        }
        let mut steps = Vec::new();
        if self.wanted.as_deref() == Some(id) {
            self.wanted = None;
            if let (Some(voice), Some(character)) = (made, cast.character(id)) {
                steps.push(Step::Speak {
                    voice: voice.id.clone(),
                    hello: character.hello.clone(),
                });
            }
        }
        steps.extend(self.next(cast));
        steps
    }

    /// The characters whose voices are being made, then the one next.
    pub fn under_way(&self) -> Vec<&str> {
        let mut ids: Vec<&str> = self.making.iter().map(String::as_str).collect();
        if let Some(wanted) = self.wanted.as_deref()
            && !ids.contains(&wanted)
        {
            ids.push(wanted);
        }
        ids
    }

    /// The character waiting, made now when none is under way.
    fn next(&mut self, cast: &Cast) -> Vec<Step> {
        if self.making.is_some() {
            return Vec::new();
        }
        match self.wanted.as_deref().and_then(|id| cast.character(id)) {
            Some(character) => {
                self.making = Some(character.id.clone());
                vec![Step::Make(character.clone())]
            }
            None => {
                self.wanted = None;
                Vec::new()
            }
        }
    }
}

/// A character's voice, made on the account or not, from [`Maker`].
#[derive(Debug)]
pub struct Made {
    pub character: Character,
    /// The voice on the account, or why it couldn't be made.
    pub voice: Result<Voice, String>,
}

/// Makes characters' voices on an ElevenLabs account, each on a thread of
/// its own, and hands back what came of each.
pub struct Maker {
    key: String,
    base: String,
    cast: Arc<Cast>,
    done: mpsc::Sender<Made>,
}

impl Maker {
    /// `base`: [`super::eleven::BASE`], or a stand-in for tests.
    pub fn new(key: &str, base: &str, cast: Arc<Cast>, done: mpsc::Sender<Made>) -> Maker {
        Maker {
            key: key.to_string(),
            base: base.to_string(),
            cast,
            done,
        }
    }

    /// Starts making `character`'s voice. What comes of it is sent back
    /// whatever happens, a panic on the way included; an error here means
    /// it never started, and nothing will be sent.
    pub fn start(&self, character: Character) -> std::io::Result<()> {
        let (key, base, cast, done) = (
            self.key.clone(),
            self.base.clone(),
            Arc::clone(&self.cast),
            self.done.clone(),
        );
        std::thread::Builder::new()
            .name("cast".into())
            .spawn(move || {
                let voice = catch_unwind(AssertUnwindSafe(|| {
                    cast.make(&character, &Eleven::new(&key, &base))
                        .map_err(|e| e.detail())
                }))
                .unwrap_or_else(|_| Err(STOPPED.into()));
                let _ = done.send(Made { character, voice });
            })
            .map(drop)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cast() -> Cast {
        Cast::built_in().expect("the cast as it ships")
    }

    #[test]
    fn the_cast_as_it_ships_is_whole() {
        let cast = cast();
        assert!(!cast.characters.is_empty());
        // Each language one character has, they all have: none is left in
        // the cast's language on a phone in another.
        let languages: BTreeSet<&String> = cast
            .characters
            .iter()
            .flat_map(|c| c.name.keys().chain(c.about.keys()))
            .collect();
        for c in &cast.characters {
            for language in &languages {
                assert!(
                    c.name.contains_key(*language),
                    "{}: name in {language}",
                    c.id
                );
                assert!(
                    c.about.contains_key(*language),
                    "{}: about in {language}",
                    c.id
                );
            }
            // Every voice has its own name on the account.
            let name = cast.voice_name_of(c);
            assert!(name.contains(cast.name_of(c)) && !name.contains(NAME_SLOT));
        }
        let names: BTreeSet<String> = cast
            .characters
            .iter()
            .map(|c| cast.voice_name_of(c))
            .collect();
        assert_eq!(names.len(), cast.characters.len());
    }

    /// The cast as it ships with one thing changed in its JSON.
    fn changed(change: impl FnOnce(&mut Value)) -> Result<Cast, String> {
        let mut json: Value = serde_json::from_str(CAST).unwrap();
        change(&mut json);
        Cast::parse(&json.to_string())
    }

    #[test]
    fn a_broken_cast_says_what_is_wrong() {
        type Change = Box<dyn FnOnce(&mut Value)>;
        let first = |json: &mut Value| json["characters"][0].clone();
        let cases: Vec<(Change, &str)> = vec![
            (
                Box::new(|j| j["characters"][0]["id"] = json!("../up")),
                "not an id",
            ),
            (
                Box::new(move |j| {
                    let again = first(j);
                    j["characters"].as_array_mut().unwrap().push(again);
                }),
                "twice",
            ),
            (
                Box::new(|j| j["characters"][0]["sample"] = json!("Too short.")),
                "sample",
            ),
            (
                Box::new(|j| j["characters"][0]["description"] = json!("Deep.")),
                "description",
            ),
            (
                Box::new(|j| {
                    let language = j["language"].as_str().unwrap().to_string();
                    j["characters"][0]["name"]
                        .as_object_mut()
                        .unwrap()
                        .remove(&language);
                }),
                "no name",
            ),
            (
                Box::new(|j| j["voice_name"] = json!("MapleSyrup")),
                NAME_SLOT,
            ),
            (Box::new(|j| j["design_models"] = json!([])), "model"),
            (
                Box::new(|j| j["design_models"] = json!(["eleven_ttv_v3", " "])),
                "blank",
            ),
            (
                Box::new(|j| j["characters"][0]["hello"] = json!(" ")),
                "hello",
            ),
            (Box::new(|j| j["hello"] = json!("")), "hello"),
        ];
        for (change, says) in cases {
            let why = changed(change).unwrap_err();
            assert!(why.contains(says), "{why}");
        }
        assert!(changed(|_| {}).is_ok());
    }

    fn voice(id: &str, name: &str, labels: &[(&str, &str)]) -> Voice {
        Voice {
            id: id.into(),
            name: name.into(),
            labels: labels
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            ..Voice::default()
        }
    }

    #[test]
    fn a_characters_voice_is_found_by_its_label_or_else_by_its_name() {
        let cast = cast();
        let (first, second) = (&cast.characters[0], &cast.characters[1]);
        let labelled = voice(
            "v-labelled",
            "Renamed by the player",
            &[(&cast.label, &first.id)],
        );
        let named = voice("v-named", &cast.voice_name_of(second), &[]);
        let other = voice("v-other", cast.name_of(first), &[]);
        let voices = [other, labelled.clone(), named.clone()];
        assert_eq!(cast.voice_of(first, &voices), Some(&labelled));
        assert_eq!(cast.voice_of(second, &voices), Some(&named));
        assert_eq!(cast.voice_of(&cast.characters[2], &voices), None);

        // For the phone: names in every language, the voice when there is
        // one, who is being made; never how they sound or their lines.
        let making = [cast.characters[2].id.as_str()];
        let status = cast.status(&voices, &making);
        assert_eq!(status["language"], cast.language);
        assert_eq!(status["making"], json!(making));
        let listed = status["characters"].as_array().unwrap();
        assert_eq!(listed.len(), cast.characters.len());
        assert_eq!(listed[0]["voice"], "v-labelled");
        assert_eq!(listed[1]["voice"], "v-named");
        assert!(listed[2]["voice"].is_null());
        assert_eq!(listed[0]["name"], json!(first.name));
        for kept in ["description", "sample", "hello"] {
            assert!(listed[0].get(kept).is_none(), "{kept}");
        }
        assert_eq!(cast.status(&[], &[])["making"], json!([]));
    }

    /// A voice made for character `id` (what [`Maker`] hands back).
    fn made_for(cast: &Cast, id: &str) -> Voice {
        let character = cast.character(id).unwrap();
        voice(
            &format!("made-{id}"),
            &cast.voice_name_of(character),
            &[(&cast.label, id)],
        )
    }

    fn speak(voice: &str, hello: &str) -> Step {
        Step::Speak {
            voice: voice.into(),
            hello: hello.into(),
        }
    }

    #[test]
    fn a_character_with_a_voice_speaks_at_once_and_one_without_is_made() {
        let cast = cast();
        let (x, y) = (&cast.characters[0], &cast.characters[1]);
        let mut casting = Casting::default();
        let voices = [made_for(&cast, &x.id)];

        // In its voice, with its hello.
        assert_eq!(
            casting.character(&cast, &x.id, Some(&voices)),
            [speak(&voices[0].id, &x.hello)]
        );
        // No voice yet: made.
        assert_eq!(
            casting.character(&cast, &y.id, Some(&voices)),
            [Step::Make(y.clone())]
        );
        assert_eq!(casting.under_way(), [y.id.as_str()]);
        // Made: it speaks in it.
        let made = made_for(&cast, &y.id);
        assert_eq!(
            casting.made(&cast, &y.id, Some(&made)),
            [speak(&made.id, &y.hello)]
        );
        assert!(casting.under_way().is_empty());

        // Nothing to go on: not known (the account's voices not listed
        // yet, or no account), or no such character.
        assert!(casting.character(&cast, &y.id, None).is_empty());
        assert!(casting.character(&cast, "nobody", Some(&voices)).is_empty());
        assert!(casting.under_way().is_empty());

        // A voice of the account's own, or OpenAI's: its own hello.
        assert_eq!(casting.voice(&cast, "v-zed"), [speak("v-zed", &cast.hello)]);
    }

    #[test]
    fn the_latest_pick_counts_and_one_voice_is_made_at_a_time() {
        let cast = cast();
        let (x, y, z) = (
            &cast.characters[0],
            &cast.characters[1],
            &cast.characters[2],
        );
        let mut casting = Casting::default();
        assert_eq!(
            casting.character(&cast, &x.id, Some(&[])),
            [Step::Make(x.clone())]
        );

        // Picked meanwhile: they wait, and only the last one picked does.
        assert!(casting.character(&cast, &y.id, Some(&[])).is_empty());
        assert!(casting.character(&cast, &z.id, Some(&[])).is_empty());
        assert!(casting.character(&cast, &x.id, Some(&[])).is_empty());
        assert_eq!(casting.under_way(), [x.id.as_str()]);
        assert!(casting.character(&cast, &z.id, Some(&[])).is_empty());
        assert_eq!(casting.under_way(), [x.id.as_str(), z.id.as_str()]);
        // X's voice made: not spoken in (Z was picked since), Z made now.
        let made = made_for(&cast, &x.id);
        assert_eq!(
            casting.made(&cast, &x.id, Some(&made)),
            [Step::Make(z.clone())]
        );
        assert_eq!(casting.under_way(), [z.id.as_str()]);

        // A voice picked while Z is made: it speaks at once, and Z, made
        // later, only joins the list.
        assert_eq!(casting.voice(&cast, "v-zed"), [speak("v-zed", &cast.hello)]);
        assert_eq!(casting.under_way(), [z.id.as_str()]);
        assert!(
            casting
                .made(&cast, &z.id, Some(&made_for(&cast, &z.id)))
                .is_empty()
        );
        assert!(casting.under_way().is_empty());
    }

    #[test]
    fn a_voice_that_couldnt_be_made_lets_the_next_one_go() {
        let cast = cast();
        let (x, y) = (&cast.characters[0], &cast.characters[1]);
        let mut casting = Casting::default();
        // Failed (or never started): nothing spoken, nothing left waiting,
        // and picking it again tries again.
        assert_eq!(
            casting.character(&cast, &x.id, Some(&[])),
            [Step::Make(x.clone())]
        );
        assert!(casting.made(&cast, &x.id, None).is_empty());
        assert!(casting.under_way().is_empty());
        assert_eq!(
            casting.character(&cast, &x.id, Some(&[])),
            [Step::Make(x.clone())]
        );
        // Failed while another was picked: that one is made next.
        assert!(casting.character(&cast, &y.id, Some(&[])).is_empty());
        assert_eq!(casting.made(&cast, &x.id, None), [Step::Make(y.clone())]);
        assert_eq!(casting.under_way(), [y.id.as_str()]);
        // A voice made for a character it wasn't making changes nothing.
        assert!(
            casting
                .made(&cast, &x.id, Some(&made_for(&cast, &x.id)))
                .is_empty()
        );
        assert_eq!(casting.under_way(), [y.id.as_str()]);
    }
}
