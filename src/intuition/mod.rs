//! Intuition: MapleSyrup thinking on its own while the player plays, and
//! saying some of it.
//!
//! The companion says what is certain from the numbers; the coach has a
//! model look when something happened. Neither has a thought of its own:
//! every unprompted line is one model call made at the moment it is
//! wanted, and nothing is ever said for the pleasure of company. This
//! module is the rest of a friend on the couch — the one who, passing
//! through Henesys on the way to Kerning City, says the music here is
//! great, or who has an idea of what to do next before being asked.
//!
//! How it works, in the shape of Inner Thoughts (Liu et al., CHI 2025),
//! trimmed for a voice that must answer within a second:
//!
//! ```text
//!  frame ──► World ──► Tempo (how tense) ──► Trail (what changed)
//!                                                │ boundaries
//!                                                ▼
//!                           Hooks (data) ─► matching hooks ─► Cue ──► Mind (a model, in the background)
//!                                                                         │ thoughts, rated 1–5
//!                                                                         ▼
//!                           Director (when, whether) ◄──────────────── Pool (thoughts, each with its basis)
//!                                │ a thought whose basis still holds, in a calm gap, past the dial's threshold
//!                                ▼
//!                           said (the worker's voice, or the live call's), then: answered / let pass / "shh"
//! ```
//!
//! - [`world`]: the moment, as numbers; and the boundaries of play.
//! - [`tempo`]: how tense the game is, from HP, deaths and the picture.
//! - [`hooks`]: what may be thought about, and MapleSyrup's own tastes
//!   (data: `hooks.json`, `persona.json`).
//! - [`thoughts`]: a thought, its basis, and the pool they wait in.
//! - [`director`]: the pacing — gaps, thresholds, the player's dial, what
//!   the player's silence does (data: `initiative.json`).
//! - [`mind`]: the background thread that phrases hooks into thoughts and
//!   keeps a standing plan for "what now?".
//! - [`vocal`]: humming, from MapleSyrup's own melodies (data:
//!   `melodies.json`).
//! - [`trace`]: the session written down as numbers, and replayed with
//!   today's data — how the director is tuned without a model or a game.
//!
//! Nothing said lives in this code: the hooks carry facts and tags, the
//! mind puts them in words in the player's language, and the director
//! only ever sees ids, ratings and seconds. Everything is replayable from
//! a `World` per frame, and tested that way.

pub mod director;
pub mod hooks;
pub mod mind;
pub mod tempo;
pub mod thoughts;
pub mod trace;
pub mod vocal;
pub mod world;

use std::collections::{HashSet, VecDeque};

pub use director::{Director, Hold, Outcome};
pub use hooks::{Hook, Hooks, Kind, Persona, Trigger};
pub use tempo::{Intensity, Tempo};
pub use thoughts::{Basis, Pool, Thought};
pub use world::{Boundary, Place, Trail, World};

/// What the mind is asked to think about: a moment, and the hooks that
/// matched it.
#[derive(Debug, Clone, PartialEq)]
pub struct Cue {
    pub trigger: Trigger,
    /// The hooks that matched, most specific first (their ids).
    pub hooks: Vec<String>,
    pub world: World,
    pub tempo: Tempo,
    /// Whether the standing plan should be thought over too.
    pub plan: bool,
}

/// What the intuition asks the main loop to do this frame.
#[derive(Debug, Clone, PartialEq)]
pub enum Act {
    /// Hand this to the mind.
    Think(Cue),
    /// Say this thought now.
    Say(Thought),
    /// Hum this melody (its id in `melodies.json`) after the thought.
    Hum(String),
    /// A line for the session log.
    Note(String),
}

/// A deterministic die (splitmix64), seeded per session so a replay rolls
/// the same.
#[derive(Debug, Clone)]
pub struct Dice {
    state: u64,
}

impl Dice {
    pub fn seeded(seed: u64) -> Self {
        Self { state: seed }
    }

    fn next(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// A number in [0, 1).
    pub fn roll(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }

    /// True with probability `odds`.
    pub fn chance(&mut self, odds: f32) -> bool {
        self.roll() < odds as f64
    }
}

/// How often the standing plan is thought over when nothing else asks for
/// it, in seconds.
pub const PLAN_EVERY_S: f64 = 600.0;
/// A boundary of play may have the mind think sooner than its usual pace,
/// but no sooner than this after the last cue, in seconds.
pub const BOUNDARY_THINK_GAP_S: f64 = 8.0;

/// The whole of it, one per session.
#[derive(Debug)]
pub struct Intuition {
    pub on: bool,
    pub intensity: Intensity,
    pub trail: Trail,
    pub director: Director,
    pub pool: Pool,
    dice: Dice,
    tempo: Tempo,
    /// Since when the tempo has been calm.
    calm_since: Option<f64>,
    last_cue_at: f64,
    last_plan_at: f64,
    /// Cues held for the mind's pace.
    waiting: VecDeque<Cue>,
    /// The calm-stretch hooks whose odds were rolled in this calm (each
    /// gets one roll per calm, when its own wait is up).
    calm_rolled: HashSet<String>,
    /// Why nothing was said in the last frame (None: something was).
    hold: Option<Hold>,
}

impl Intuition {
    /// `level`: where the player's dial stands (None: where it starts);
    /// `seed`: the session's, for the odds.
    pub fn new(on: bool, level: Option<&str>, seed: u64) -> Self {
        Self {
            on,
            intensity: Intensity::new(),
            trail: Trail::new(),
            director: Director::new(level),
            pool: Pool::new(),
            dice: Dice::seeded(seed),
            tempo: Tempo::Calm,
            calm_since: None,
            last_cue_at: f64::NEG_INFINITY,
            last_plan_at: f64::NEG_INFINITY,
            waiting: VecDeque::new(),
            calm_rolled: HashSet::new(),
            hold: None,
        }
    }

    /// The tempo as of the last frame.
    pub fn tempo(&self) -> Tempo {
        self.tempo
    }

    /// How long the game has been calm, in seconds (0 when it isn't).
    pub fn calm_for(&self, now: f64) -> f64 {
        self.calm_since.map(|s| now - s).unwrap_or(0.0)
    }

    /// One frame.
    pub fn observe(&mut self, world: &World) -> Vec<Act> {
        let now = world.now;
        let mut acts = Vec::new();
        self.tempo = self.intensity.observe(world);
        if self.tempo <= Tempo::Calm && self.tempo != Tempo::Away {
            self.calm_since.get_or_insert(now);
        } else {
            self.calm_since = None;
            self.calm_rolled.clear();
        }
        let boundaries = self.trail.step(world);
        if !self.on {
            return acts;
        }
        for (thought, why) in self.pool.prune(world, self.tempo) {
            acts.push(Act::Note(format!(
                "dropped \"{}\" ({}): {}",
                thought.hook,
                thought.kind.word(),
                why.word()
            )));
        }
        // What happened: the hooks it fires.
        let hooks = Hooks::get();
        let calm_for = self.calm_for(now);
        for boundary in boundaries {
            self.director.boundary(now);
            if let Some(trigger) = Trigger::of(boundary) {
                let plan = matches!(
                    trigger,
                    Trigger::LevelUp | Trigger::MapEntered | Trigger::GameSeen
                );
                self.cue(trigger, world, calm_for, plan, hooks);
            }
        }
        // A long calm: each calm-stretch hook gets its roll once per calm,
        // when its own wait is up (a town breather at two and a half
        // minutes, "what next" at four), so a later one is not spent by an
        // earlier one's roll.
        if hooks.all().iter().any(|h| {
            h.when == Trigger::CalmStretch
                && !self.calm_rolled.contains(&h.id)
                && h.applies(world, self.tempo, calm_for)
        }) {
            let rolled = self.cue(Trigger::CalmStretch, world, calm_for, false, hooks);
            self.calm_rolled.extend(rolled);
        }
        // The standing plan, now and then.
        if world.seen && now - self.last_plan_at >= PLAN_EVERY_S && self.waiting.is_empty() {
            self.waiting.push_back(Cue {
                trigger: Trigger::CalmStretch,
                hooks: Vec::new(),
                world: world.clone(),
                tempo: self.tempo,
                plan: true,
            });
        }
        // The mind's pace: a boundary's hooks go soon, the rest at the
        // usual pace; a plan waiting rides along with whatever goes.
        if let Some(mut cue) = self.release(now) {
            cue.world = world.clone();
            cue.tempo = self.tempo;
            if cue.plan {
                self.last_plan_at = now;
            }
            self.last_cue_at = now;
            acts.push(Act::Think(cue));
        }
        // Whether a thought is said now.
        match self.director.consider(world, self.tempo, &mut self.pool) {
            Ok(thought) => {
                self.hold = None;
                self.director.said(&thought, now);
                let hum = thought.hum.clone();
                acts.push(Act::Say(thought));
                if let Some(melody) = hum {
                    acts.push(Act::Hum(melody));
                }
            }
            Err(hold) => self.hold = Some(hold),
        }
        acts
    }

    /// Why nothing was said in the last frame (None: something was, or no
    /// frame yet).
    pub fn hold(&self) -> Option<Hold> {
        self.hold
    }

    /// The hooks `trigger` fires now, each kept with its odds, go to the
    /// mind as one cue (with the plan when `plan`). Returns the ids of the
    /// hooks whose odds were rolled (for the calm stretch, where a hook
    /// gets one roll per calm: for the CalmStretch trigger only the hooks
    /// not rolled in this calm are rolled).
    fn cue(
        &mut self,
        trigger: Trigger,
        world: &World,
        calm_for: f64,
        plan: bool,
        hooks: &Hooks,
    ) -> Vec<String> {
        let mut rolled = Vec::new();
        let mut matched = Vec::new();
        for hook in hooks.matching(trigger, world, self.tempo, calm_for) {
            if !self.director.hook_open(&hook.id)
                || (trigger == Trigger::CalmStretch && self.calm_rolled.contains(&hook.id))
            {
                continue;
            }
            rolled.push(hook.id.clone());
            if self.dice.chance(hook.odds) {
                matched.push(hook.id.clone());
            }
        }
        if matched.is_empty() && !plan {
            return rolled;
        }
        // One cue per trigger waits at a time; a newer one replaces it.
        self.waiting.retain(|c| c.trigger != trigger);
        self.waiting.push_back(Cue {
            trigger,
            hooks: matched,
            world: world.clone(),
            tempo: self.tempo,
            plan,
        });
        rolled
    }

    /// The cue whose turn it is, if any: one with a boundary's hooks after
    /// [`BOUNDARY_THINK_GAP_S`], any other after the usual pace. A plan
    /// waiting in another cue goes with it.
    fn release(&mut self, now: f64) -> Option<Cue> {
        let pace = director::Initiative::get().pacing.think_every_at_least_s;
        let since = now - self.last_cue_at;
        let urgent = |c: &Cue| !c.hooks.is_empty() && c.trigger != Trigger::CalmStretch;
        let at = self.waiting.iter().position(|c| {
            since
                >= if urgent(c) {
                    BOUNDARY_THINK_GAP_S
                } else {
                    pace
                }
        })?;
        let mut cue = self.waiting.remove(at)?;
        if self.waiting.iter().any(|c| c.plan) {
            cue.plan = true;
            self.waiting.retain(|c| !(c.plan && c.hooks.is_empty()));
            for other in &mut self.waiting {
                other.plan = false;
            }
        }
        Some(cue)
    }

    /// The mind came back with a thought.
    pub fn thought(&mut self, thought: Thought) {
        self.pool.add(thought);
    }

    /// The player said something (for what became of the last thought).
    pub fn player_spoke(&mut self, now: f64) -> Option<Outcome> {
        self.director.player_spoke(now)
    }

    /// The voice finished the last thought (and its hum): the player's
    /// window to answer opens now.
    pub fn voiced(&mut self, now: f64) {
        self.director.voiced(now);
    }

    /// The last thought never made a sound (talked over, or the voice
    /// failed): as if it had not been decided.
    pub fn unsaid(&mut self) {
        self.director.unsaid();
    }

    /// The player asked for quiet at `now`.
    pub fn told_to_stop(&mut self, now: f64) -> Option<Outcome> {
        self.waiting.clear();
        self.director.told_to_stop(now)
    }

    /// Turn it on or off (off drops what was waiting).
    pub fn set_on(&mut self, on: bool) {
        self.on = on;
        if !on {
            self.waiting.clear();
            self.pool = Pool::new();
        }
    }

    /// Cues waiting for the mind.
    pub fn waiting(&self) -> usize {
        self.waiting.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(now: f64, map: &str, hp: f32, activity: f32) -> World {
        World {
            now,
            seen: true,
            place: Some(Place {
                name: map.into(),
                age_s: 1.0,
            }),
            level: Some(25),
            hp: Some(hp),
            activity,
            since_anyone_spoke: now,
            session_s: now + 1200.0,
            classic: true,
            ..World::default()
        }
    }

    fn thought_for(id: u64, hook: &str, map: &str, now: f64) -> Thought {
        let h = Hooks::get().named(hook).unwrap();
        Thought {
            id,
            hook: hook.into(),
            topic: h.topic.clone(),
            kind: h.kind,
            text: format!("t{id}"),
            rating: 4.5,
            basis: Basis {
                map: Some(map.into()),
                level: Some(25),
                alive: true,
                tempo_at_most: Tempo::Travel,
                made_at: now,
                ttl_s: 400.0,
            },
            hum: h.hum.clone(),
        }
    }

    #[test]
    fn entering_ellinia_cues_the_mind_and_its_thought_is_said_in_the_next_calm_gap() {
        let mut it = Intuition::new(true, None, 7);
        let mut cues = Vec::new();
        // Settling in on Henesys, a still screen.
        for s in 0..30 {
            for act in it.observe(&frame(s as f64, "Henesys", 100.0, 0.004)) {
                if let Act::Think(c) = act {
                    cues.push(c);
                }
            }
        }
        // The game coming into view asked for the plan.
        assert!(cues.iter().any(|c| c.plan), "{cues:?}");
        cues.clear();
        // Into Ellinia.
        let acts = it.observe(&frame(30.0, "Ellinia", 100.0, 0.004));
        let cue = acts
            .iter()
            .find_map(|a| match a {
                Act::Think(c) if c.trigger == Trigger::MapEntered => Some(c.clone()),
                _ => None,
            })
            .or_else(|| {
                // (The mind's pace may hold it a frame or two.)
                let mut found = None;
                for s in 31..45 {
                    for act in it.observe(&frame(s as f64, "Ellinia", 100.0, 0.004)) {
                        if let Act::Think(c) = act
                            && c.trigger == Trigger::MapEntered
                        {
                            found = Some(c);
                        }
                    }
                }
                found
            })
            .expect("a cue for the map");
        assert_eq!(cue.hooks, vec!["ellinia-music".to_string()]);
        assert!(cue.plan);
        // The mind answers; the thought is said once the gap has passed,
        // with a hum after it.
        it.thought(thought_for(1, "ellinia-music", "Ellinia", 45.0));
        let mut said = None;
        let mut hummed = None;
        for s in 46..200 {
            for act in it.observe(&frame(s as f64, "Ellinia", 100.0, 0.004)) {
                match act {
                    Act::Say(t) => said = Some((s, t)),
                    Act::Hum(m) => hummed = Some(m),
                    _ => {}
                }
            }
        }
        let (at, thought) = said.expect("said");
        assert_eq!(thought.hook, "ellinia-music");
        assert_eq!(hummed.as_deref(), Some("morning-leaf"));
        assert!(at >= 46, "{at}");
        // The voice never reported finishing: nothing is settled yet.
        assert_eq!(it.director.stats.spoken, 1);
        assert_eq!(it.director.stats.ignored, 0);
        assert_eq!(it.director.awaiting(), Some("ellinia-music"));
        // It finishes, and nobody answers within the window: let pass, and
        // the gaps stretch.
        it.voiced(200.0);
        for s in 201..230 {
            it.observe(&frame(s as f64, "Ellinia", 100.0, 0.004));
        }
        assert_eq!(it.director.stats.ignored, 1);
        assert!(it.director.backoff() > 1.0);
    }

    #[test]
    fn a_long_calm_gives_each_calm_hook_its_own_roll_and_what_next_its_turn() {
        // The town breather (two and a half minutes) must not spend the
        // roll of "what next" (four minutes): over many seeds, both come
        // up, and a calm with no cue at all is the exception.
        let hooks = Hooks::get();
        let breather = hooks.named("town-breather").unwrap();
        let what_next = hooks.named("what-next").unwrap();
        assert!(breather.place.calm_for_s.unwrap() < what_next.place.calm_for_s.unwrap());
        let (mut breathers, mut what_nexts, mut quiet_calms) = (0, 0, 0);
        for seed in 0..60u64 {
            let mut it = Intuition::new(true, None, seed);
            let mut cued = HashSet::new();
            // Twenty minutes into a session, ten minutes of calm.
            for s in 0..600 {
                let mut w = frame(s as f64, "Henesys", 100.0, 0.004);
                w.session_s = 1200.0 + s as f64;
                for act in it.observe(&w) {
                    if let Act::Think(c) = act {
                        cued.extend(c.hooks);
                    }
                }
            }
            breathers += cued.contains("town-breather") as u32;
            what_nexts += cued.contains("what-next") as u32;
            quiet_calms += cued.is_empty() as u32;
        }
        assert!(breathers > 10, "{breathers}");
        assert!(what_nexts > 10, "{what_nexts}");
        assert!(quiet_calms < 30, "{quiet_calms}");
    }

    #[test]
    fn a_thought_about_a_map_they_left_is_dropped_and_a_fight_holds_everything() {
        let mut it = Intuition::new(true, None, 3);
        for s in 0..20 {
            it.observe(&frame(s as f64, "Ellinia", 100.0, 0.004));
        }
        // A fight breaks out; the mind's thought about the place arrives
        // in the middle of it.
        let mut hp = 100.0;
        let mut said = false;
        for s in 20..45 {
            hp -= 4.0;
            if s == 26 {
                it.thought(thought_for(1, "ellinia-music", "Ellinia", 26.0));
            }
            for act in it.observe(&frame(s as f64, "Ellinia", hp, 0.024)) {
                if matches!(act, Act::Say(_)) {
                    said = true;
                }
            }
        }
        assert!(!said, "spoke in a fight");
        assert!(it.tempo() >= Tempo::Combat, "{:?}", it.tempo());
        assert_eq!(it.pool.len(), 1, "a tense moment is waited out");
        // They leave for Henesys: the thought dies, with a note.
        let acts = it.observe(&frame(45.0, "Henesys", hp, 0.012));
        assert!(
            acts.iter()
                .any(|a| matches!(a, Act::Note(n) if n.contains("left the map"))),
            "{acts:?}"
        );
        assert!(it.pool.is_empty());
    }

    #[test]
    fn off_it_thinks_nothing_and_a_shh_clears_what_waited() {
        let mut it = Intuition::new(false, Some("talkative"), 1);
        for s in 0..40 {
            let acts = it.observe(&frame(s as f64, "Ellinia", 100.0, 0.004));
            assert!(acts.is_empty(), "{acts:?}");
        }
        it.set_on(true);
        // Into Kerning City: its arrival is a boundary, and with nothing
        // asked of the mind before, the cue goes to it at once (with the
        // plan, as a map entered asks).
        let acts = it.observe(&frame(40.0, "Kerning City", 100.0, 0.004));
        assert!(
            acts.iter()
                .any(|a| matches!(a, Act::Think(c) if c.trigger == Trigger::MapEntered && c.plan)),
            "{acts:?}"
        );
        // Another boundary right after waits for the mind's pace.
        let mut w = frame(41.0, "Kerning City", 100.0, 0.004);
        w.level = Some(26);
        let acts = it.observe(&w);
        assert!(!acts.iter().any(|a| matches!(a, Act::Think(_))), "{acts:?}");
        assert_eq!(it.waiting(), 1);
        // "Shh" before anything was said: the waiting cue goes, the gaps go
        // to their longest, and no thought is blamed.
        assert_eq!(it.told_to_stop(42.0), None);
        assert_eq!(it.waiting(), 0);
        assert_eq!(it.director.stats.rejected, 0);
        assert!(it.director.backoff() > 1.0);
    }

    #[test]
    fn the_dice_roll_the_same_for_the_same_seed_and_evenly() {
        let mut a = Dice::seeded(42);
        let mut b = Dice::seeded(42);
        for _ in 0..10 {
            assert_eq!(a.roll(), b.roll());
        }
        let mut hits = 0;
        for _ in 0..10_000 {
            if a.chance(0.3) {
                hits += 1;
            }
        }
        assert!((2700..3300).contains(&hits), "{hits}");
        assert!(a.roll() < 1.0);
    }
}
