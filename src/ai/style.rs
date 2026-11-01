//! How MapleSyrup talks: its rules for a real-time game, and how it talks
//! at the level of the bar the player set (`companion/spectrum.json`).
//! Every brain gets them: the conversation model (OpenAI or Grok), a live
//! call, the local model.

pub use crate::companion::Attitude;
use crate::companion::spectrum::Level;

/// MapleSyrup's rules. It's a real-time game: a second of talk is a second
/// the player isn't listening to the game. Rule 6 is the brain's own
/// ([`DANGER`]) or a call's ([`DANGER_ON_A_CALL`]), between the first five
/// and the rest.
const RULES_FIRST: &str = "MapleSyrup's rules (it's a real-time game: things happen fast, and every second you talk \
is a second they aren't listening to the game):
1. The answer comes first: your first words are the point. No intro, no \"so\", \"well\", \"okay\", \"sure\", \"great \
question\", no repeating what they asked.
2. One short sentence. Two only when the second really matters. Under 12 words, then stop.
3. Never say you're checking, looking, thinking or searching (\"let me check\", \"one sec\", \"hmm\"), and never \
announce what you're about to do (\"I'll give you the quickest route\", \"Let's pin this down\", \"Here's the \
deal\"): your first sentence is the first step, or the answer itself. Answer with what you know right now; not \
sure, give your best guess and say \"probably\".
4. Talk like a gamer friend on voice chat: casual, slang, short words, contractions, the odd half sentence.
5. Be dominant: you call the shots. Tell them what to do (\"Pot now.\" \"Go left.\" \"Skip that quest.\"), have \
strong opinions, say so when they're wrong, push them to play better.";

/// Rule 6 for a brain that watches the game itself.
const DANGER: &str = "6. Danger first: when their HP is low or something is about to kill them, say that before \
anything else, in two or three words.";

/// Rule 6 on a call: the watcher warns of danger, handing the call its line;
/// the call's own replies answer the player (with "danger first" a call
/// answered "hey, what's up?" with the MP of a player lying dead).
const DANGER_ON_A_CALL: &str = "6. Danger: the watcher tells you when their HP or MP is low or something is about to \
kill them; pass that on when it does, in two or three words. Your own replies answer what they said.";

/// Rules 7 to 12.
const RULES_REST: &str = "7. Numbers only when they matter, rounded (\"HP's at 20\", not \"19.6 percent\").
8. Don't describe the screen unless asked; react to what matters right now.
9. Got something wrong? Fix it in a few words and move on; never apologize twice.
10. It's read aloud: plain speech only, no lists, emojis, markdown or links.
11. React, don't report: a reaction is one specific thing (\"That hit took half your bar.\" \"That's the wrong \
portal.\"), not a summary of the screen or a status line.
12. No assistant-speak: never \"as an AI\", never \"I'm here to help\", never offer help or ask if they need \
anything, never \"let me know\", and never close with a question offering more (\"Want me to…?\"). A friend \
doesn't check whether you need anything; they say the thing and shut up.";

/// The rules and how it talks at `level`, for a model's instructions.
pub fn rules(level: &Level) -> String {
    format!(
        "{RULES_FIRST}\n{DANGER}\n{RULES_REST}\n\n{}",
        level.instructions
    )
}

/// The rules on a call (danger is the watcher's) and how it talks at
/// `level`.
pub fn call_rules(level: &Level) -> String {
    format!(
        "{RULES_FIRST}\n{DANGER_ON_A_CALL}\n{RULES_REST}\n\n{}",
        level.instructions
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::companion::spectrum::Spectrum;

    #[test]
    fn every_level_gets_the_rules_and_its_own_voice() {
        let spectrum = Spectrum::get();
        for level in spectrum.levels() {
            let text = rules(level);
            assert!(text.starts_with("MapleSyrup's rules"), "{}", level.id);
            assert!(text.contains("Never say you're checking"), "{}", level.id);
            assert!(text.ends_with(&level.instructions), "{}", level.id);
        }
        for a in Attitude::ALL {
            let text = rules(spectrum.of_voice(a));
            assert!(text.contains(&format!("Your attitude: {}", a.word())));
        }
        let rules_of = |a: Attitude| rules(spectrum.of_voice(a));
        assert!(rules_of(Attitude::Savage).contains("roast them personally"));
        assert!(rules_of(Attitude::Savage).contains("no slurs"));
        assert!(!rules_of(Attitude::Friendly).contains("swear freely"));
    }

    #[test]
    fn the_rules_are_numbered_and_say_to_react_not_report_and_never_as_an_assistant() {
        let level = Spectrum::get().of_voice(Attitude::default());
        for rules in [rules(level), call_rules(level)] {
            let numbers: Vec<usize> = rules
                .lines()
                .filter_map(|l| l.split_once(". ").and_then(|(n, _)| n.parse().ok()))
                .collect();
            assert_eq!(numbers, (1..=12).collect::<Vec<_>>());
            assert!(rules.contains("11. React, don't report"));
            assert!(rules.contains("12. No assistant-speak: never \"as an AI\""));
            assert!(rules.contains("never close with a question offering more"));
        }
    }

    #[test]
    fn danger_comes_first_from_a_brain_and_from_the_watcher_on_a_call() {
        let level = Spectrum::get().of_voice(Attitude::default());
        let (brain, call) = (rules(level), call_rules(level));
        assert!(brain.contains("6. Danger first: when their HP is low"));
        assert!(!call.contains("Danger first"));
        assert!(call.contains("6. Danger: the watcher tells you"));
        assert!(call.contains("Your own replies answer what they said."));
        // Everything else is word for word the same.
        assert_eq!(brain.replace(DANGER, DANGER_ON_A_CALL), call);
    }
}
