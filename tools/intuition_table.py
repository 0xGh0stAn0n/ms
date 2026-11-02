"""What MapleSyrup's thoughts of its own came to, as a table: the
`[intuition]` lines in a session's log.txt (src/intuition/, always on
while the intuition is), read into what the design is tuned by — of the
thoughts said, how many were answered within the window, let pass, or
shushed (Codellaborator's measured profile: about half answered, no more
than one in eight a disruption) — per hook, per trigger, and per dial
level, with the thoughts the mind made that were never said and why.

    python3 tools/intuition_table.py "C:\\Users\\me\\Documents\\MapleSyrup sessions\\2026-11-02 20-00-00"
    python3 tools/intuition_table.py log.txt other-session/log.txt --csv thoughts.csv
    python3 tools/intuition_table.py sessions/ --timeline

A folder is searched for its log.txt files. `--csv FILE` also writes every
thought said, one row each, for a spreadsheet; `--timeline` prints each
session's thoughts in order with what became of them. Standard library
only."""
import argparse
import csv
import os
import re
import sys

KIND = "[intuition] "
# A log line: the time of day, then `[who] text`.
LINE = re.compile(r"^(\d\d:\d\d:\d\d)\s+\[intuition\] (.*)$")
# The lines the intuition writes (see src/bin/maplesyrup/main.rs).
THINKING = re.compile(r"^thinking: (\w+) \(([^)]*)\)(, and the plan)?$")
THOUGHT = re.compile(r"^thought \(([^,]+), ([^,]+), ([\d.]+)/5\): (.*)$")
SAYS = re.compile(r"^says \(([^,]+), ([^,]+), ([\d.]+)/5, tempo (\w+)\): (.*)$")
OUTCOME = re.compile(r"^thought (engaged|ignored|rejected)$")
NOT_SAID = re.compile(r"^not said: (.*)$")
DROPPED = re.compile(r'^dropped "([^"]+)" \(([^)]+)\): (.*)$')
# The dial, at the start ("thoughts of its own: on; initiative: normal")
# and when it moves ("initiative: quiet").
DIAL = re.compile(r"initiative: (\w+)$")
CLOSING = re.compile(r"^said (\d+) thoughts: (\d+) answered, (\d+) let pass, (\d+) shushed; dial (\w+)$")
OUTCOMES = ("engaged", "ignored", "rejected", "not said", "open")
WORDS = {
    "engaged": "answered",
    "ignored": "let pass",
    "rejected": "shushed",
    "not said": "not said",
    "open": "no word yet",
}


def logs_in(paths):
    for path in paths:
        if os.path.isdir(path):
            for folder, _, files in os.walk(path):
                if "log.txt" in files:
                    yield os.path.join(folder, "log.txt")
        else:
            yield path


def session_of(path):
    """One log.txt read: the cues, the thoughts made, the thoughts said
    (each with what became of it), the thoughts dropped, the dial's moves
    and the closing counts."""
    session = {
        "path": path,
        "cues": [],
        "made": [],
        "said": [],
        "dropped": [],
        "dial": [],
        "closing": None,
    }
    dial = None
    with open(path, encoding="utf-8", errors="replace") as log:
        for raw in log:
            m = LINE.match(raw.rstrip("\n"))
            if not m:
                continue
            time, text = m.groups()
            if (t := THINKING.match(text)):
                hooks = [h.strip() for h in t.group(2).split(",") if h.strip()]
                session["cues"].append({"time": time, "trigger": t.group(1), "hooks": hooks, "plan": bool(t.group(3))})
            elif (t := THOUGHT.match(text)):
                session["made"].append({"time": time, "hook": t.group(1), "kind": t.group(2), "rating": float(t.group(3)), "text": t.group(4)})
            elif (t := SAYS.match(text)):
                session["said"].append({
                    "time": time,
                    "hook": t.group(1),
                    "kind": t.group(2),
                    "rating": float(t.group(3)),
                    "tempo": t.group(4),
                    "text": t.group(5),
                    # (The dial is logged when the session starts and when
                    # it moves; "?" is a log from before it was.)
                    "dial": dial or "?",
                    "outcome": "open",
                })
            elif (t := OUTCOME.match(text)):
                last = next((s for s in reversed(session["said"]) if s["outcome"] == "open"), None)
                if last:
                    last["outcome"] = t.group(1)
                elif t.group(1) == "rejected" and session["said"] and session["said"][-1]["outcome"] == "engaged":
                    # "Stop coaching" is heard as words first (answered),
                    # then as the command: the answer is taken back.
                    session["said"][-1]["outcome"] = "rejected"
            elif (t := NOT_SAID.match(text)):
                last = next((s for s in reversed(session["said"]) if s["outcome"] == "open"), None)
                if last:
                    last["outcome"] = "not said"
                    last["why"] = t.group(1)
            elif (t := DROPPED.match(text)):
                session["dropped"].append({"time": time, "hook": t.group(1), "kind": t.group(2), "why": t.group(3)})
            elif (t := DIAL.search(text)):
                dial = t.group(1)
                session["dial"].append({"time": time, "level": dial})
            elif (t := CLOSING.match(text)):
                session["closing"] = tuple(int(x) for x in t.groups()[:4]) + (t.group(5),)
                dial = t.group(5)
    return session


def ratio(part, whole):
    return f"{100 * part / whole:.0f}%" if whole else "-"


def counts_by(said, key):
    """For each value of `key`: how many said, and how many of each outcome."""
    groups = {}
    for s in said:
        row = groups.setdefault(s[key], {o: 0 for o in OUTCOMES})
        row[s["outcome"]] += 1
    return groups


def table(title, groups):
    out = [f"## {title}", ""]
    out.append("| | said | answered | let pass | shushed | not said | no word yet | answered of heard | disruption |")
    out.append("|---|---|---|---|---|---|---|---|---|")
    for name, row in sorted(groups.items(), key=lambda kv: -sum(kv[1].values())):
        said = sum(row.values())
        heard = said - row["not said"] - row["open"]
        out.append(
            f"| {name} | {said} | {row['engaged']} | {row['ignored']} | {row['rejected']} | "
            f"{row['not said']} | {row['open']} | {ratio(row['engaged'], heard)} | {ratio(row['rejected'], heard)} |"
        )
    out.append("")
    return out


def report(sessions, timeline):
    said = [s for session in sessions for s in session["said"]]
    made = [m for session in sessions for m in session["made"]]
    cues = [c for session in sessions for c in session["cues"]]
    dropped = [d for session in sessions for d in session["dropped"]]
    heard = [s for s in said if s["outcome"] not in ("not said", "open")]
    out = [f"# Thoughts of its own: {len(sessions)} session(s)", ""]
    out.append(
        f"{len(cues)} cues to the mind ({sum(c['plan'] for c in cues)} with the plan), "
        f"{len(made)} thoughts made, {len(said)} said, {len(dropped)} dropped unsaid."
    )
    if heard:
        answered = sum(s["outcome"] == "engaged" for s in heard)
        shushed = sum(s["outcome"] == "rejected" for s in heard)
        out.append(
            f"Of the {len(heard)} heard: {ratio(answered, len(heard))} answered "
            f"(target about 50%), {ratio(shushed, len(heard))} shushed (target at most 12%)."
        )
    out.append("")
    out += table("By hook", counts_by(said, "hook"))
    out += table("By kind", counts_by(said, "kind"))
    out += table("By dial", counts_by(said, "dial"))
    out += table("By tempo when said", counts_by(said, "tempo"))
    # What the mind made that was never said, by hook and why.
    said_by_hook = {}
    for s in said:
        said_by_hook[s["hook"]] = said_by_hook.get(s["hook"], 0) + 1
    out += ["## Made, said and dropped, by hook", ""]
    out.append("| hook | made | said | dropped (why) |")
    out.append("|---|---|---|---|")
    hooks = sorted({m["hook"] for m in made} | {d["hook"] for d in dropped} | set(said_by_hook))
    for hook in hooks:
        whys = {}
        for d in dropped:
            if d["hook"] == hook:
                whys[d["why"]] = whys.get(d["why"], 0) + 1
        why = ", ".join(f"{w} {n}" for w, n in sorted(whys.items(), key=lambda kv: -kv[1]))
        out.append(f"| {hook} | {sum(m['hook'] == hook for m in made)} | {said_by_hook.get(hook, 0)} | {why} |")
    out.append("")
    # Cues by trigger.
    out += ["## Cues to the mind, by trigger", ""]
    out.append("| trigger | cues | hooks per cue | with the plan |")
    out.append("|---|---|---|---|")
    triggers = {}
    for c in cues:
        triggers.setdefault(c["trigger"], []).append(c)
    for trigger, group in sorted(triggers.items(), key=lambda kv: -len(kv[1])):
        per = sum(len(c["hooks"]) for c in group) / len(group)
        out.append(f"| {trigger} | {len(group)} | {per:.1f} | {sum(c['plan'] for c in group)} |")
    out.append("")
    if timeline:
        for session in sessions:
            out += [f"## {session['path']}", ""]
            if session["closing"]:
                spoken, answered, let_pass, shushed, dial = session["closing"]
                out.append(f"closing line: said {spoken}, answered {answered}, let pass {let_pass}, shushed {shushed}; dial {dial}")
                out.append("")
            for s in session["said"]:
                why = f" ({s['why']})" if "why" in s else ""
                out.append(f"- {s['time']}  {s['hook']} ({s['kind']}, {s['rating']:.1f}/5, {s['tempo']}, dial {s['dial']}): {s['text']}  → {WORDS[s['outcome']]}{why}")
            out.append("")
    return out


def write_csv(path, sessions):
    with open(path, "w", newline="", encoding="utf-8") as file:
        rows = csv.writer(file)
        rows.writerow(["log", "time", "hook", "kind", "rating", "tempo", "dial", "outcome", "why", "text"])
        for session in sessions:
            for s in session["said"]:
                rows.writerow([session["path"], s["time"], s["hook"], s["kind"], s["rating"], s["tempo"], s["dial"], WORDS[s["outcome"]], s.get("why", ""), s["text"]])


def main(argv):
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("paths", nargs="+", help="log.txt files, or folders to search for them")
    parser.add_argument("--csv", help="also write every thought said, one row each")
    parser.add_argument("--timeline", action="store_true", help="each session's thoughts in order")
    args = parser.parse_args(argv)
    sessions = [session_of(path) for path in logs_in(args.paths)]
    sessions = [s for s in sessions if s["cues"] or s["said"] or s["closing"]]
    if not sessions:
        print("No [intuition] lines in any log.txt there.", file=sys.stderr)
        return 1
    print("\n".join(report(sessions, args.timeline)))
    if args.csv:
        write_csv(args.csv, sessions)
        print(f"\n{sum(len(s['said']) for s in sessions)} thoughts written to {args.csv}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
