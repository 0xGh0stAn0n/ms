//! Replays a session's intuition with today's data: when would MapleSyrup
//! speak up now, and why did it hold its tongue the rest of the time.
//!
//! ```text
//! intuition_replay "C:\Users\me\Documents\MapleSyrup sessions\2026-11-02 20-00-00"
//! intuition_replay sessions\ --level quiet
//! intuition_replay intuition.jsonl --seed 3
//! ```
//!
//! A folder is searched for its `intuition.jsonl` files (one per session,
//! written while the intuition is on). The mind's thoughts are replayed
//! as the session made them; the director decides again from
//! `initiative.json`, `tempo.json` and `hooks.json` as built into this
//! binary — change a number, build, replay, compare. `--level` tries a
//! dial throughout instead of the session's own; `--seed` rolls the hooks'
//! odds another way.

use std::path::{Path, PathBuf};

use ms::intuition::trace;

fn traces_in(path: &Path, out: &mut Vec<PathBuf>) {
    if path.is_dir() {
        if let Ok(entries) = std::fs::read_dir(path) {
            let mut children: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
            children.sort();
            for child in children {
                traces_in(&child, out);
            }
        }
    } else if path.file_name().is_some_and(|n| n == trace::FILE) {
        out.push(path.to_path_buf());
    }
}

fn main() {
    let mut args = std::env::args().skip(1);
    let mut paths = Vec::new();
    let mut level: Option<String> = None;
    let mut seed: u64 = 1;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--level" => level = args.next(),
            "--seed" => seed = args.next().and_then(|s| s.parse().ok()).unwrap_or(seed),
            "-h" | "--help" => {
                println!(
                    "intuition_replay <session folder | intuition.jsonl>... [--level quiet|normal|talkative] [--seed N]"
                );
                return;
            }
            _ => paths.push(PathBuf::from(arg)),
        }
    }
    if paths.is_empty() {
        eprintln!("Give a session folder or an {} file.", trace::FILE);
        std::process::exit(2);
    }
    let mut traces = Vec::new();
    for path in &paths {
        traces_in(path, &mut traces);
    }
    if traces.is_empty() {
        eprintln!("No {} under {:?}.", trace::FILE, paths);
        std::process::exit(1);
    }
    let (mut did, mut would) = (0usize, 0usize);
    for path in &traces {
        let entries = match trace::read(path) {
            Ok(entries) => entries,
            Err(e) => {
                eprintln!("{e}");
                continue;
            }
        };
        let replayed = trace::replay(&entries, level.as_deref(), seed);
        did += replayed.did_say.len();
        would += replayed.would_say.len();
        println!("# {}\n\n{}", path.display(), replayed.report());
    }
    println!(
        "\n{} session(s): said {did} thought(s); with today's data{} it would say {would}.",
        traces.len(),
        level
            .as_deref()
            .map(|l| format!(" on the {l} dial"))
            .unwrap_or_default()
    );
}
