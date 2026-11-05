# Intuition: thoughts of its own

MapleSyrup answered what it was asked and called out what the numbers made
certain. It had no thought of its own: nothing to say on the way through
Henesys, no idea of what to do next until asked, no taste. The owner's
brief (1.11.2026) was a friend on the couch:

> Player: "Hey, what should I do now?"
> MapleSyrup: "You know what? Let's go to Kerning City and do the party
> quest with a few others — that'll level you up two levels really fast."
> (passing through Henesys) "Wow, the music at the Henesys training ground
> is so good, right?"
> Player: "Nah… the most beautiful is Ellinia's."
> MapleSyrup: "I swear it is! The piano there is insane" *(hums)*

This folder holds the research that shaped `src/intuition/` (the seven
notes under `research/`, the Hebrew report `research-report.he.md`) and
the reasons for the design, in short.

## What the example asks for, and what answers it

| Moment | What it is | What makes it |
| --- | --- | --- |
| "Let's go do the Kerning PQ" | A decision in the first person plural, with a reason, before the question | The standing plan (`mind::Plan`), thought in the background and handed to the conversation (`Brain::set_plan`) |
| "The music here is so good" | A shared experience with no use, in a calm moment | A hook on the map (`hooks.json`), the tempo calm, the director's gap |
| "right?" | An invitation that costs nothing to ignore | What became of a thought: answered, let pass, shushed (`director::Outcome`) — and the gaps that stretch |
| "I swear it is! The piano" | Agreement with a concrete detail | The taste card (`persona.json`): a stance held in data, agreement only where the card agrees |
| the hum | Play, with no purpose | `vocal.rs` and `melodies.json`: MapleSyrup's own tunes, synthesised |

The dangerous moment is the agreement. A companion that agrees with
everything reads as fake within minutes (the audit of companion apps,
the GPT-4o rollback, ELEPHANT's measurements — `research/persona_memory_ethics.md`);
what saves it is the detail, and a stance that does not flip under
pushback.

## The shape, and where each piece comes from

```text
 frame ─► World ─► Tempo ─► Trail (boundaries) ─► Hooks ─► Cue ─► Mind ─► Pool ─► Director ─► said
```

- **Thoughts ahead of the moment** (`mind.rs`, `thoughts.rs`): Inner
  Thoughts (Liu et al., CHI 2025) — a loop that forms candidate thoughts
  in the background, rates each for how much it wants saying, and speaks
  only past a threshold; the moderately proactive setting won with
  players, the quiet one was rated worst, the chatty one ignored pauses.
  Pre-making the thought is also what makes saying it free of latency:
  the voice has nothing to compute when the gap opens.
- **A standing plan the fast side reads** (`mind::Plan`): DeepMind's
  Talker-Reasoner — the slow side writes a belief, the fast side talks
  from it. Its documented failure (stale beliefs) is why the plan carries
  the level it was made for and is renewed at every boundary.
- **The director apart from the model** (`director.rs`): every study of
  proactive agents finds that models over-propose (half their offers are
  false alarms on ProactiveBench) and that full-duplex speech models open
  their mouths when addressed or when it is quiet, never because content
  calls for it (`research/timing_turn_taking.md`). So whether and when is
  decided from numbers here, and the model only phrases.
- **Boundaries, not idle time** (`world::Trail`): Iqbal & Bailey (CHI
  2008) — a message deferred to a boundary of the task frustrates far
  less than one in the middle; every 2025–2026 deployment found idle
  time the worst cue (people are concentrating, not stuck) and task
  boundaries the best. MapleStory's boundaries the vision engine sees: a
  map entered, a level gained, a revive, the game back in view, the
  player back from a long quiet.
- **The tempo** (`tempo.rs`): Left 4 Dead's director (Booth, GDC 2009) —
  an intensity events push up and time decays, the decay paused while the
  danger goes on. Small talk during fights was the first thing every
  tested companion had to remove (LeagueBot's pilot; critics on PUBG
  Ally).
- **Rules as data** (`hooks.json`, Ruskin's response system, GDC 2012):
  facts, criteria, the most specific rule wins, follow-ups re-checked when
  about to play — a thought about a map they have left dies unsaid
  (`Basis::holds`), the owner's rule that a line about something no
  longer true is not said.
- **The player's silence as the dial**: ComPeer (UIST 2024) backed off
  after an ignored initiative; here a thought let pass stretches the
  gaps, an answer brings them back, "stop coaching" sets the longest.
- **Humming as an asset**: no voice model hums on request (ElevenLabs
  marks singing experimental; ChatGPT's voice is told never to), and a
  hummed game theme is a performance of Nexon's composition in a paid
  product — Nexon's own guide allows no commercial use. So the tunes are
  MapleSyrup's own, marked so in `melodies.json`, and the owner, a
  pianist, writes the leitmotifs.

## What the numbers in the data mean

`initiative.json`: the dial's thresholds are on the mind's 1–5 scale. A
thought rated 4 or more is said in the first calm gap at the normal
level; a 3 waits for a boundary (`boundary_bonus`) or a minute or two of
silence (`silence_boost_per_s`, capped); nothing is said within
`min_gap_after_anyone_spoke_s` of the last word, nor two of its own within
`min_gap_between_own_thoughts_s`, nor more than `at_most_per_10_min`. The
quiet level more than doubles the gaps and allows one in ten minutes; the
talkative level lets a 3 through. `tempo.json`: a death is a peak, a
fight is combat within the scene's five-second window (the picture's
activity counts at once, `activity_weight`; HP falling in bites adds to
it), walking is travel, a still screen or a town's idle animation is calm,
nothing for `away_after_s` is away, and a fight is followed by `relax_s`
at travel or above; only calm and travel may be spoken in
(`tempo_penalty`). The activity scale is the scene's own, measured on a
real recording at 10 frames a second: a boss fight 0.02–0.03, moving
about 0.012, a dialog open or a town's idle animation 0.003–0.006, the
coach's idle line 0.002. Replaying that recording's activity through
`intuition_replay` holds the tempo for the whole fight.

The target the research gives: about half of the thoughts said get a
word back and no more than one in eight is a disruption (Codellaborator,
CHI 2025, the best measured profile). The session log's closing
`[intuition]` line counts answered, let pass and shushed; tune the data
until the ratio holds — on replays first: every session writes
`intuition.jsonl` (`trace.rs`), and `intuition_replay <session folder>
[--level quiet]` runs it again with the data as built, printing what the
session said beside what it would say now, and why nothing was said the
rest of the time; `tools/intuition_table.py <sessions>` reads the log's
`[intuition]` lines into the per-hook, per-kind, per-dial tables. Those
counts are only worth tuning by if they are honest, so: the player's window to answer opens when the voice is done
with the line and its hum (`Done::Voiced`, plus what is still queued to
play), not when the director decided; a thought talked over before a
sound was made is taken back whole (its once-per-session hook stays open,
its topic as fresh as before); MapleSyrup's own voice heard back by the
phone never counts as an answer (the bookkeeping runs after the echo
filter); and "stop coaching" right after a thought counts against the
thought even though its words were heard first.

## What is not in yet, by design

- **Hebrew end-of-turn detection**: none of the three dedicated
  end-of-turn classifiers checked lists Hebrew (LiveKit's detector, Smart
  Turn v3, Deepgram Flux; `research/timing_turn_taking.md`), and the
  vendors' own semantic VADs publish no list except Gemini's, which does
  name Hebrew; the phone's settle timer and the call's semantic VAD stay.
  Smart Turn is open and could be fine-tuned on Hebrew gameplay speech.
- **A phone control for the dial**: the PC side is there (`/api/initiative`,
  `initiative` in `/api/state`, the `set_initiative` tool, the setting in
  the notebook); the page's control is UX work for Yohai.
- **Thoughts on a live call**: the call relays them as its own (the
  instructions say so, and tell a thought from a watcher line by its
  content; a marker on the relayed line would make that certain, and is
  the phone page's to add); when the call is done saying one is the call's
  to know, so on a call the answer window opens at the hand-over; the hum
  waits for the PC's voice.
- **A phone card for the counts**: the session record carries them
  (`thoughts`, `initiative` in `metrics/sessions.jsonl`), the log has
  them, `tools/intuition_table.py` tables them and `intuition_replay`
  replays a session's `intuition.jsonl` with today's data; the phone's
  stats page does not show them yet.
- **A callback trigger**: the mind sees the player's tastes and moments
  from the notebook with every cue, so a callback can ride any hook; a
  trigger of its own (the player's words naming a place or a tune they
  like) would need the words matched against data in the player's
  language, which is not done.

## Two things the research says to settle before scaling

1. **Nexon's policy on third-party programs** names "unauthorized screen
   recognition" software as a banned category, enforced device-wide
   (`research/game_knowledge_grounding.md`). The test is "not expressly
   authorized by us": authorization or partnership, through the B2B
   relationship, is the way.
2. **Companion-chatbot laws** (California SB 243, New York GBL Art. 47,
   Oregon and Washington from 2027; `research/persona_memory_ethics.md`):
   a companion that remembers preferences and starts conversations is
   inside New York's definition with no game carve-out. The baseline
   duties — a spoken "I'm an AI" at the start, a break reminder every
   three hours, a crisis protocol, no manipulative goodbyes — are all
   data-driven to add.
