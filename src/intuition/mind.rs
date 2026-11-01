//! The mind: a model thinking in the background, phrasing hooks into
//! thoughts and keeping a standing plan.
//!
//! It runs on its own thread with its own client, like the learner and the
//! look-ups, and never on the voice's path: a thought is made when
//! something happens (a cue) and waits in the pool for its moment, so
//! saying it costs no model time at all — the way a person has the remark
//! ready before the gap opens. The same call keeps the standing plan (the
//! Talker-Reasoner split: the slow side writes a belief the fast side
//! reads), so "what should I do now?" is answered from a plan already
//! thought through, and the suggestion the mind voices on its own is the
//! same one the conversation gives when asked.
//!
//! The model is given the moment as the vision engine reads it, the hooks
//! that fired with their facts and notes, MapleSyrup's own tastes that bear
//! on them, and what it knows of the player; it answers in a schema: a few
//! candidate thoughts, each in the player's language with a 1–5 rating of
//! how much it deserves saying (it argues for and against first — the one
//! step that stopped Inner Thoughts' model from over-rating its own ideas),
//! and the plan. Facts come only from the hooks; a thought that leans on
//! anything else is not kept.

use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Duration;

use serde::Deserialize;
use serde_json::{Value, json};

use super::director::Initiative;
use super::hooks::{Hooks, Kind, Persona};
use super::thoughts::{Basis, Thought};
use super::world::World;
use super::{Cue, Tempo};
use crate::ai::brain;
use crate::ai::language;
use crate::ai::memory::Learning;
use crate::ai::openai::{AiError, Ask, OpenAi};

/// How many thoughts one cue may yield at most.
pub const MOST_THOUGHTS: usize = 3;
/// How many of the player's last sentences the mind is given (the same
/// conversation, not a transcript).
pub const LATELY_MOST: usize = 6;
/// How long a thought may take to be written.
const TIMEOUT: Duration = Duration::from_secs(25);
/// Room for three weighed lines in Hebrew and a plan; a cut-off answer
/// fails the schema and loses the plan with it.
const MAX_OUTPUT_TOKENS: u32 = 900;
/// A thought this short is not a thought ("ok.").
const SHORTEST_THOUGHT: usize = 6;

/// What the mind is told it is doing.
pub const GUIDE: &str = "Right now nobody is talking to you. You are thinking to yourself while they play, the way a \
friend on the couch has a thought ready before there's a gap to say it in. Your game watcher brings you the moment \
(the game as your vision engine reads it) and what just happened, with the few things worth a thought about it: \
each has an id, facts you may lean on, and notes with what is known. Write what you might say later, if the \
moment stays calm — at most one line per thing, one short sentence of at most 14 words, in the player's language, \
in your own voice and attitude, first person. A suggestion is what we'd do next and the one reason, as a friend \
decides, not a menu. An opinion is yours: your taste card says what you love, like or shrug at and why; say it \
with one concrete detail from the facts, never a list. A callback picks up what they once said they like, in one \
clause. A quip is a word of company. Never a question (never ask whether they're there), never a report of the \
screen, never a greeting, nothing you were not given a fact for: no composers, no numbers, no names of maps, NPCs \
or quests that aren't in the facts or the reading. For each line, first weigh it in a few words — why it would land \
now, why it might not (said before, obvious, off the moment) — then rate 1 to 5 how much it deserves saying: 5 only \
for something a friend would not keep in. Leave out what rates under 2. When asked for the plan: what we're working \
towards, the one next thing to do and why, from the reading and the facts, in the player's language, each a short \
sentence; keep the plan as it was unless something changed.";

/// A thought as the model writes it.
#[derive(Debug, Clone, Deserialize, PartialEq)]
struct Written {
    hook: String,
    kind: String,
    #[serde(default)]
    weighed: String,
    text: String,
    rating: f32,
    #[serde(default)]
    hum: bool,
}

/// The plan as the model writes it.
#[derive(Debug, Clone, Deserialize, PartialEq, Default)]
struct WrittenPlan {
    goal: String,
    next: String,
    why: String,
}

#[derive(Debug, Clone, Deserialize)]
struct Answer {
    #[serde(default)]
    thoughts: Vec<Written>,
    #[serde(default)]
    plan: Option<WrittenPlan>,
}

/// The standing plan: what MapleSyrup would suggest, thought through.
#[derive(Debug, Clone, PartialEq)]
pub struct Plan {
    pub goal: String,
    pub next: String,
    pub why: String,
    /// The level it was made for (None: unknown then).
    pub level: Option<u32>,
}

impl Plan {
    /// Whether it still fits `world`: the level has not changed (a plan
    /// outlives a map change; the mind renews it then anyway). A plan that
    /// no longer fits is taken away from the conversation rather than kept
    /// stale — the Talker-Reasoner failure, a fast side answering from an
    /// old belief.
    pub fn fits(&self, world: &World) -> bool {
        match (self.level, world.level) {
            (Some(planned), Some(now)) => planned == now,
            _ => true,
        }
    }

    /// The plan for the conversation's instructions.
    pub fn prompt(&self) -> String {
        format!(
            "What you'd suggest they do next, thought through already (say it when they ask what to do, as your own \
idea, short; never recite it): goal — {}; next — {}; because — {}.",
            self.goal.trim(),
            self.next.trim(),
            self.why.trim()
        )
    }
}

/// What came back from a cue.
#[derive(Debug, Clone, PartialEq)]
pub enum Thinking {
    Thoughts(Vec<Thought>),
    Plan(Plan),
    Failed(String),
}

/// What goes with a cue to the mind.
#[derive(Debug, Clone, Default)]
pub struct Context {
    /// The player's language setting (a locale such as `he-IL`).
    pub language: Option<String>,
    /// The game right now, read by the vision engine.
    pub snapshot: String,
    /// The plan as it stands, if any.
    pub plan: Option<Plan>,
    /// What the player said lately, newest last (to think in the same
    /// conversation, and never answer it here).
    pub lately: Vec<String>,
}

/// The input text for a cue: the moment, what happened, the hooks.
pub fn compose(cue: &Cue, ctx: &Context, tastes: &str, known: &str) -> String {
    let hooks = Hooks::get();
    let w = &cue.world;
    let mut text = format!(
        "[Your game watcher, not the player: nobody said anything.]\nThe game right now, read by your vision \
engine:\n{}\n\nTempo: {} (how tense the game is).",
        ctx.snapshot.trim(),
        cue.tempo.word()
    );
    if let Some(map) = w.map() {
        text.push_str(&format!("\nThe map: {map}."));
    }
    if w.classic {
        text.push_str("\nThey play MapleStory Classic World.");
    }
    text.push_str(&format!(
        "\n\nWhat just happened: {}.",
        cue.trigger.word().replace('_', " ")
    ));
    if cue.hooks.is_empty() {
        text.push_str(
            "\nNothing in particular is worth a thought this time; think over the plan only.",
        );
    } else {
        text.push_str("\n\nWorth a thought:");
        for id in &cue.hooks {
            let Some(hook) = hooks.named(id) else {
                continue;
            };
            text.push_str(&format!(
                "\n- id \"{}\" ({}, topic {})",
                hook.id,
                hook.kind.word(),
                hook.topic
            ));
            if !hook.facts.is_empty() {
                let facts: Vec<String> = hook
                    .facts
                    .iter()
                    .map(|(k, v)| format!("{k}: {}", plain(v)))
                    .collect();
                text.push_str(&format!("\n  facts: {}", facts.join("; ")));
            }
            for note in &hook.notes {
                text.push_str(&format!("\n  note: {note}"));
            }
            if hook.hum.is_some() {
                text.push_str("\n  (you may feel like humming after this one: say so with hum)");
            }
        }
    }
    if !tastes.is_empty() {
        text.push_str(&format!(
            "\n\nYour taste card, where it bears on this:\n{tastes}"
        ));
    }
    if !known.is_empty() {
        text.push_str(&format!("\n\n{known}"));
    }
    if !ctx.lately.is_empty() {
        text.push_str("\n\nWhat the player said lately (newest last; don't answer it here):");
        for line in ctx
            .lately
            .iter()
            .rev()
            .take(4)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
        {
            text.push_str(&format!("\n- {line}"));
        }
    }
    if cue.plan {
        match &ctx.plan {
            Some(plan) => text.push_str(&format!(
                "\n\nThe plan as it stands: goal — {}; next — {}; because — {}. Give it back as it is, or changed \
if something changed.",
                plan.goal, plan.next, plan.why
            )),
            None => text.push_str("\n\nThere is no plan yet: make one."),
        }
    } else {
        text.push_str("\n\n(No plan this time: leave plan null.)");
    }
    if let Some(l) = ctx.language.as_deref().filter(|l| !language::is_english(l)) {
        text.push_str(&format!(
            "\n\nThe player's language setting is {}: write the lines and the plan in it.",
            language::name(l)
        ));
    }
    text
}

/// A JSON value in a sentence.
fn plain(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Array(items) => items.iter().map(plain).collect::<Vec<_>>().join(", "),
        other => other.to_string(),
    }
}

/// The taste card, where it bears on the cue's hooks (and the place).
pub fn tastes_for(cue: &Cue) -> String {
    let hooks = Hooks::get();
    let persona = Persona::get();
    let mut lines: Vec<String> = Vec::new();
    for id in &cue.hooks {
        let Some(hook) = hooks.named(id) else {
            continue;
        };
        for taste in persona.for_hook(hook, &cue.world) {
            let line = format!(
                "- {}: you {} it ({}/5){}",
                taste.about,
                taste.stance,
                taste.strength,
                if taste.because.is_empty() {
                    String::new()
                } else {
                    format!(" — {}", taste.because.join(", "))
                }
            );
            if !lines.contains(&line) {
                lines.push(line);
            }
        }
    }
    if lines.is_empty() {
        return String::new();
    }
    let character = &persona.character;
    let mut text = format!(
        "You are {}: {}; {}.",
        character.name,
        character.traits.join(", "),
        character.habits.join(", ")
    );
    text.push('\n');
    text.push_str(&lines.join("\n"));
    text
}

/// The schema the model answers in.
pub fn schema() -> Value {
    let kinds: Vec<&str> = [
        Kind::Suggestion,
        Kind::Remark,
        Kind::Opinion,
        Kind::Callback,
        Kind::Quip,
    ]
    .iter()
    .map(|k| k.word())
    .collect();
    json!({
        "type": "object",
        "properties": {
            "thoughts": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "hook": {"type": "string"},
                        "kind": {"type": "string", "enum": kinds},
                        "weighed": {"type": "string"},
                        "text": {"type": "string"},
                        "rating": {"type": "number"},
                        "hum": {"type": "boolean"}
                    },
                    "required": ["hook", "kind", "weighed", "text", "rating", "hum"],
                    "additionalProperties": false
                }
            },
            "plan": {
                "anyOf": [
                    {
                        "type": "object",
                        "properties": {
                            "goal": {"type": "string"},
                            "next": {"type": "string"},
                            "why": {"type": "string"}
                        },
                        "required": ["goal", "next", "why"],
                        "additionalProperties": false
                    },
                    {"type": "null"}
                ]
            }
        },
        "required": ["thoughts", "plan"],
        "additionalProperties": false
    })
}

/// The model's answer, as thoughts (each on the cue's basis) and a plan.
/// The thoughts kept are numbered from `first_id` up, with no gaps (the
/// ids must be unique across cues: the pool hands a thought back by id).
pub fn parse(text: &str, cue: &Cue, first_id: u64) -> Result<(Vec<Thought>, Option<Plan>), String> {
    let answer: Answer = serde_json::from_str(text).map_err(|e| format!("not the schema: {e}"))?;
    let hooks = Hooks::get();
    let ttl = Initiative::get().pacing.thought_ttl_s;
    let w = &cue.world;
    let mut thoughts: Vec<Thought> = Vec::new();
    for written in answer.thoughts.into_iter().take(MOST_THOUGHTS) {
        // Only a hook of this cue, by its id: a thought about something
        // that did not happen is dropped.
        let Some(hook) = hooks
            .named(&written.hook)
            .filter(|h| cue.hooks.contains(&h.id))
        else {
            continue;
        };
        let line = written.text.trim();
        if line.chars().filter(|c| c.is_alphanumeric()).count() < SHORTEST_THOUGHT
            || line.contains('?')
            || !(1.0..=5.0).contains(&written.rating)
        {
            continue;
        }
        let about_place = !hook.place.maps.is_empty();
        thoughts.push(Thought {
            id: first_id + thoughts.len() as u64,
            hook: hook.id.clone(),
            topic: hook.topic.clone(),
            kind: hook.kind,
            text: line.to_string(),
            rating: written.rating,
            basis: Basis {
                map: if about_place {
                    w.map().map(str::to_string)
                } else {
                    None
                },
                level: if hook.place.level_min.is_some() || hook.place.level_max.is_some() {
                    w.level
                } else {
                    None
                },
                alive: true,
                tempo_at_most: hook.place.tempo_at_most.unwrap_or(Tempo::Travel),
                made_at: w.now,
                ttl_s: ttl,
            },
            hum: hook.hum.clone().filter(|_| written.hum),
        });
    }
    let plan = answer
        .plan
        .filter(|p| !p.next.trim().is_empty())
        .map(|p| Plan {
            goal: p.goal,
            next: p.next,
            why: p.why,
            level: w.level,
        });
    Ok((thoughts, plan))
}

/// One cue thought through: the call, and what it came to.
pub fn think(
    chat: &OpenAi,
    learning: &Learning,
    cue: &Cue,
    ctx: &Context,
    first_id: u64,
) -> Vec<Thinking> {
    let mut instructions = brain::persona_for(learning);
    instructions.push_str("\n\n");
    instructions.push_str(GUIDE);
    instructions.push_str("\n\n");
    instructions.push_str(&crate::companion::fresh::guide());
    let known = learning.prompt();
    let input = compose(cue, ctx, &tastes_for(cue), &known);
    let ask = Ask {
        instructions,
        input: vec![json!({"role": "user", "content": input})],
        schema: Some(("thoughts".into(), schema())),
        max_output_tokens: MAX_OUTPUT_TOKENS,
        timeout: TIMEOUT,
        ..Default::default()
    };
    match chat.ask(&ask, None) {
        Err(AiError::Cancelled) => Vec::new(),
        Err(e) => vec![Thinking::Failed(e.detail())],
        Ok(answer) => match parse(&answer.text, cue, first_id) {
            Err(why) => vec![Thinking::Failed(why)],
            Ok((thoughts, plan)) => {
                let mut out = Vec::new();
                if !thoughts.is_empty() {
                    out.push(Thinking::Thoughts(thoughts));
                }
                if let Some(plan) = plan {
                    out.push(Thinking::Plan(plan));
                }
                out
            }
        },
    }
}

/// The mind's thread: cues in, thinking out.
#[derive(Debug)]
pub struct Mind {
    cues: Sender<(Cue, Context)>,
    thinking: Receiver<Thinking>,
}

impl Mind {
    /// Start thinking with `chat`, as the MapleSyrup `learning` knows; None
    /// when the thread could not be started (then there is no mind, and
    /// the intuition stays off rather than cueing a thread that never
    /// answers).
    pub fn spawn(chat: Arc<OpenAi>, learning: Learning) -> Option<Mind> {
        let (cues, cues_rx) = channel::<(Cue, Context)>();
        let (tx, thinking) = channel::<Thinking>();
        std::thread::Builder::new()
            .name("mind".into())
            .spawn(move || {
                let mut next_id: u64 = 1;
                while let Ok((cue, ctx)) = cues_rx.recv() {
                    // Only the newest cue waiting is thought about: the
                    // moment moved on.
                    let mut latest = (cue, ctx);
                    while let Ok(newer) = cues_rx.try_recv() {
                        latest = newer;
                    }
                    let (cue, ctx) = latest;
                    for thought in think(&chat, &learning, &cue, &ctx, next_id) {
                        if let Thinking::Thoughts(ts) = &thought {
                            next_id += ts.len() as u64;
                        }
                        if tx.send(thought).is_err() {
                            return;
                        }
                    }
                }
            })
            .ok()?;
        Some(Mind { cues, thinking })
    }

    /// Think about `cue` (never waits).
    pub fn think(&self, cue: Cue, ctx: Context) {
        let _ = self.cues.send((cue, ctx));
    }

    /// What the mind came to since last asked.
    pub fn drain(&self) -> Vec<Thinking> {
        self.thinking.try_iter().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::intuition::Trigger;
    use crate::intuition::world::Place;

    fn cue() -> Cue {
        Cue {
            trigger: Trigger::MapEntered,
            hooks: vec!["ellinia-music".into()],
            world: World {
                now: 300.0,
                seen: true,
                place: Some(Place {
                    name: "Ellinia".into(),
                    age_s: 2.0,
                }),
                level: Some(27),
                classic: true,
                ..World::default()
            },
            tempo: Tempo::Calm,
            plan: true,
        }
    }

    #[test]
    fn the_input_carries_the_moment_the_hooks_facts_and_the_taste_card() {
        let ctx = Context {
            language: Some("he-IL".into()),
            snapshot: "Level 27. HP 100%.".into(),
            plan: None,
            lately: vec!["where to now".into()],
        };
        let c = cue();
        let tastes = tastes_for(&c);
        assert!(
            tastes.contains("music:When The Morning Comes: you love it (5/5)"),
            "{tastes}"
        );
        assert!(tastes.contains("place:Ellinia"));
        let text = compose(
            &c,
            &ctx,
            &tastes,
            "What you know about them:\n- likes piano",
        );
        assert!(text.contains("The map: Ellinia."));
        assert!(text.contains("Tempo: calm"));
        assert!(text.contains("What just happened: map entered."));
        assert!(text.contains("id \"ellinia-music\" (opinion, topic music)"));
        assert!(text.contains("bgm: When The Morning Comes"));
        assert!(text.contains("mood: forest, morning, gentle"));
        assert!(text.contains("humming"));
        assert!(text.contains("There is no plan yet: make one."));
        assert!(text.contains("Hebrew"));
        assert!(text.contains("- where to now"));
        assert!(text.contains("likes piano"));
        // Without hooks, only the plan.
        let mut bare = c.clone();
        bare.hooks.clear();
        bare.plan = false;
        let text = compose(&bare, &Context::default(), "", "");
        assert!(text.contains("Nothing in particular"));
        assert!(text.contains("leave plan null"));
    }

    #[test]
    fn the_guide_forbids_questions_invented_facts_and_narration() {
        for must in [
            "at most 14 words",
            "Never a question",
            "no composers",
            "rate 1 to 5",
            "first person",
        ] {
            assert!(GUIDE.contains(must), "{must}");
        }
        let s = schema();
        assert_eq!(s["required"], json!(["thoughts", "plan"]));
        assert!(
            s["properties"]["thoughts"]["items"]["properties"]["kind"]["enum"]
                .as_array()
                .unwrap()
                .iter()
                .any(|k| k == "opinion")
        );
    }

    #[test]
    fn the_answer_becomes_thoughts_on_the_cues_basis_and_a_plan() {
        let c = cue();
        let text = json!({
            "thoughts": [
                {"hook": "ellinia-music", "kind": "opinion", "weighed": "fits; new", "text": "יואו, המוזיקה פה עם הפסנתר — הכי יפה במשחק.", "rating": 4.5, "hum": true},
                {"hook": "kerning-pq-suggestion", "kind": "suggestion", "weighed": "off the moment", "text": "Let's go do the Kerning PQ.", "rating": 3.0, "hum": false},
                {"hook": "ellinia-music", "kind": "opinion", "weighed": "a question", "text": "Nice music, right?", "rating": 4.0, "hum": false},
                {"hook": "ellinia-music", "kind": "quip", "weighed": "too short", "text": "Yes.", "rating": 5.0, "hum": false}
            ],
            "plan": {"goal": "level 30", "next": "Kerning City PQ", "why": "fast EXP at this level"}
        })
        .to_string();
        let (thoughts, plan) = parse(&text, &c, 10).unwrap();
        // The second is not a hook of this cue; the third asks; the fourth
        // is too short.
        assert_eq!(thoughts.len(), 1, "{thoughts:?}");
        let t = &thoughts[0];
        assert_eq!(t.id, 10);
        assert_eq!(t.hook, "ellinia-music");
        assert_eq!(t.kind, Kind::Opinion);
        assert_eq!(t.topic, "music");
        assert_eq!(t.basis.map.as_deref(), Some("Ellinia"));
        assert_eq!(t.basis.level, None, "the music is not about the level");
        assert_eq!(t.basis.tempo_at_most, Tempo::Travel);
        assert_eq!(t.basis.made_at, 300.0);
        assert_eq!(t.hum.as_deref(), Some("morning-leaf"));
        let plan = plan.unwrap();
        assert_eq!(plan.next, "Kerning City PQ");
        assert_eq!(plan.level, Some(27));
        assert!(plan.fits(&c.world));
        let mut leveled = c.world.clone();
        leveled.level = Some(28);
        assert!(!plan.fits(&leveled));
        assert!(plan.prompt().contains("next — Kerning City PQ"));
        // A level-bound hook keeps the level in its basis.
        let mut c2 = c.clone();
        c2.hooks = vec!["kerning-pq-suggestion".into()];
        let text = json!({"thoughts": [{"hook": "kerning-pq-suggestion", "kind": "suggestion", "weighed": "", "text": "נלך לעשות את ה-PQ בקרנינג, זה הכי מהיר עכשיו.", "rating": 4.0, "hum": false}], "plan": null}).to_string();
        let (thoughts, plan) = parse(&text, &c2, 1).unwrap();
        assert_eq!(thoughts[0].basis.level, Some(27));
        assert_eq!(thoughts[0].basis.map, None);
        assert!(plan.is_none());
        assert!(parse("not json", &c, 1).is_err());
        // Ids are numbered by the thoughts kept, with no gaps: dropped
        // entries leave no hole, so the next cue's ids follow on without a
        // clash (the pool hands thoughts back by id).
        let mut c3 = c.clone();
        c3.hooks = vec!["ellinia-music".into(), "kerning-pq-suggestion".into()];
        let text = json!({"thoughts": [
            {"hook": "nowhere", "kind": "quip", "weighed": "", "text": "A line about nothing that happened.", "rating": 3.0, "hum": false},
            {"hook": "ellinia-music", "kind": "opinion", "weighed": "", "text": "The piano in this forest is the best in the game.", "rating": 4.0, "hum": false},
            {"hook": "kerning-pq-suggestion", "kind": "suggestion", "weighed": "", "text": "Let's go do the party quest in Kerning City.", "rating": 4.0, "hum": false}
        ], "plan": null}).to_string();
        let (thoughts, _) = parse(&text, &c3, 7).unwrap();
        assert_eq!(
            thoughts.iter().map(|t| t.id).collect::<Vec<_>>(),
            vec![7, 8]
        );
    }
}
