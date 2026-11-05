//! What is not vision and not the companion: the per-stage timing of the
//! vision path, read from its tracing spans, the per-stage timing of a
//! voice turn (off unless asked for), the vision engine's worker
//! threads, and what an id taken from outside may look like.
//!
//! The pixel, drawing and timing helpers that used to live here are
//! Syrup's now (`syrup::color`, `syrup::draw`, `syrup::timing`).

pub mod latency;
pub mod pool;
pub mod stages;

/// The longest [`plain_id`].
pub const ID_MAX: usize = 64;

/// An id safe to take from the phone or a file and use in a path, a
/// request or a log: 1 to [`ID_MAX`] ASCII letters, digits, `_` or `-` (an
/// ElevenLabs voice's id, a character's in the cast).
pub fn plain_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= ID_MAX
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plain_id_is_letters_digits_underscores_and_dashes() {
        for id in ["syrup", "nana-ping", "pNInz6obpgDQGcFmaJgB", "a_b"] {
            assert!(plain_id(id), "{id}");
        }
        for id in ["", "../x", "a b", "סירופ", &"x".repeat(ID_MAX + 1)] {
            assert!(!plain_id(id), "{id}");
        }
    }
}
