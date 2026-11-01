//! A crash, kept on this PC: the panic, where, the backtrace and the end
//! of the session's log without a word of conversation, written to
//! `crash-report.txt` in the settings folder. Nothing is sent: the next
//! start says there is one and where it is.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// The report a crash leaves until the next start picks it up.
const REPORT: &str = "crash-report.txt";
/// How much of the session's log goes in.
const LOG_LINES: usize = 200;
/// The log's kinds that never carry what the player or MapleSyrup said
/// (`[heard]`, `[reply]` and the rest are left out).
const PLAIN: &[&str] = &[
    "info", "update", "sight", "capture", "timing", "warning", "alert",
];

static LOG: OnceLock<PathBuf> = OnceLock::new();

/// Keep a report of any panic in `settings`, after Rust's own message.
pub fn install(settings: &Path) {
    let report = settings.join(REPORT);
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        default(info);
        let log = LOG
            .get()
            .and_then(|p| fs::read_to_string(p).ok())
            .unwrap_or_default();
        let text = format!(
            "MapleSyrup {} on {} {}\n{}\nthread: {}\n{info}\n\nbacktrace:\n{}\n\nthe log's last lines (no conversation):\n{}",
            env!("CARGO_PKG_VERSION"),
            std::env::consts::OS,
            std::env::consts::ARCH,
            chrono::Local::now().format("%Y-%m-%d %H:%M:%S"),
            std::thread::current().name().unwrap_or("unnamed"),
            std::backtrace::Backtrace::force_capture(),
            plain_tail(&log, LOG_LINES),
        );
        let _ = fs::write(&report, text);
    }));
}

/// The session's log, for the report (set once the session is open).
pub fn log_at(path: PathBuf) {
    let _ = LOG.set(path);
}

/// The report the last run left, renamed to `crash-<time>.txt` so it is
/// told once: its path.
pub fn left(settings: &Path) -> Option<PathBuf> {
    let report = settings.join(REPORT);
    let modified = fs::metadata(&report).ok()?.modified().ok()?;
    let stamp = chrono::DateTime::<chrono::Local>::from(modified).format("%Y-%m-%d %H-%M-%S");
    let kept = settings.join(format!("crash-{stamp}.txt"));
    fs::rename(&report, &kept).ok()?;
    Some(kept)
}

/// The last `n` lines of `log` whose kind is [`PLAIN`].
fn plain_tail(log: &str, n: usize) -> String {
    let plain: Vec<&str> = log
        .lines()
        .filter(|line| {
            line.split_once("  [")
                .and_then(|(_, rest)| rest.split_once(']'))
                .is_some_and(|(kind, _)| PLAIN.contains(&kind))
        })
        .collect();
    plain[plain.len().saturating_sub(n)..].join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_report_keeps_no_conversation() {
        let log = "13:42:10  [heard] my password is hunter2\n\
                   13:42:11  [reply] Level 57, HP 82 percent\n\
                   13:42:12  [sight] HUD found\n\
                   13:42:13  [info] Workshop off\n\
                   no kind at all";
        assert_eq!(
            plain_tail(log, 200),
            "13:42:12  [sight] HUD found\n13:42:13  [info] Workshop off"
        );
        assert_eq!(plain_tail(log, 1), "13:42:13  [info] Workshop off");
    }

    #[test]
    fn a_panic_leaves_a_report() {
        let dir = std::env::temp_dir().join(format!("ms-panic-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        install(&dir);
        let _ = std::panic::catch_unwind(|| panic!("a test crash"));
        let _ = std::panic::take_hook();
        let report = fs::read_to_string(dir.join(REPORT)).unwrap();
        assert!(report.contains("a test crash") && report.contains("backtrace:"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_report_left_is_told_once() {
        let dir = std::env::temp_dir().join(format!("ms-crash-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        assert_eq!(left(&dir), None);
        fs::write(dir.join(REPORT), "panicked").unwrap();
        let kept = left(&dir).expect("the report");
        assert_eq!(fs::read_to_string(&kept).unwrap(), "panicked");
        assert_eq!(left(&dir), None);
        let _ = fs::remove_dir_all(&dir);
    }
}
