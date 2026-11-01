//! One frame through the companion's eyes.
//!
//! The detectors the frame still needs ([`PerceptionPipeline`]), then what
//! MapleSyrup learned about this screen ([`Sight`]), into the companion's
//! [`Observation`]. The `maplesyrup` binary and `vision_bench` both run
//! this one function, so what the bench times is what the companion does.
//!
//! Once the sight sees the HUD — a layout that fits this screen and bars
//! it measured on the last frame — the pipeline's HUD geometry detector
//! has nothing to add: the sight's bars and numbers replace its reading
//! in [`Sight::apply`]. It is skipped on those frames, unless something
//! else (the preview window) is showing its picture.

use std::time::Instant;

use image::RgbaImage;

use crate::companion::Observation;
use crate::sight::{Seen, Sight};
use crate::vision::{Detectors, PerceptionPipeline, WorldState};

/// What one frame gave.
pub struct Perceived {
    /// The detectors' reading (the ones that ran; the rest say so).
    pub world: WorldState,
    /// The companion's view of the frame: the sight's values where it has
    /// them, the detectors' otherwise.
    pub obs: Observation,
    /// What the sight saw, for the alerts it fired and the frame's record.
    pub seen: Seen,
}

/// One captured frame, and when and how it was taken.
pub struct Look<'a> {
    /// The window's title.
    pub title: &'a str,
    pub frame: &'a RgbaImage,
    pub frame_id: u64,
    /// When: paces the sight's searches, so a bench can run them on a
    /// simulated clock.
    pub now: Instant,
    /// Whether the game is in view (in front, or behind another window
    /// while the capture gets it as it is drawn); the sight looks only then.
    pub in_view: bool,
}

/// One frame: the detectors in `wanted` that are still needed, then the
/// sight (when there is one and the game is in view), into an observation.
pub fn perceive(
    pipeline: &mut PerceptionPipeline,
    sight: Option<&mut Sight>,
    wanted: Detectors,
    look: &Look<'_>,
) -> Perceived {
    let (frame, frame_id) = (look.frame, look.frame_id);
    // With a sight, the HUD's geometry detector runs only for something
    // that shows its own picture of it (the preview, which asks for its
    // OCR too): without its OCR, nothing it finds reaches the observation
    // (a bar it only measured is a guess, left out by
    // `Observation::from_world`), whether the sight sees the HUD or not,
    // and whether the game is in view or not — and it costs 1 ms a frame
    // at 1080p, 4 at 4K.
    let wanted = match &sight {
        Some(_) if wanted.hud && !wanted.hud_text => Detectors {
            hud: false,
            ..wanted
        },
        _ => wanted,
    };
    let world =
        tracing::trace_span!("vision").in_scope(|| pipeline.detect_some(frame, frame_id, wanted));
    let mut obs = tracing::trace_span!("observation")
        .in_scope(|| Observation::from_world(look.title, &world));
    let mut seen = Seen::default();
    if let Some(sight) = sight {
        let _span = tracing::trace_span!("sight").entered();
        if look.in_view {
            seen = sight.observe(frame, look.now);
        }
        sight.apply(&mut obs, &seen);
    }
    Perceived { world, obs, seen }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOT_RUN: Option<&str> = Some("not run: nothing is showing it");

    #[test]
    fn the_hud_detector_runs_only_for_the_preview_once_there_is_a_sight() {
        // A status panel the way the sight's own tests draw one: HP 320 of
        // 400 (80%), with the MP and EXP bars beside it.
        let (frame, _) = crate::sight::numbers::tests::hud((320, 400), (675, 1351), 30.0);
        let mut pipeline = PerceptionPipeline::new();
        let dir = std::env::temp_dir().join(format!("ms-perceive-{}", std::process::id()));
        let mut sight = Sight::load(&dir);
        let now = Instant::now();
        let look = |frame_id| Look {
            title: "test",
            frame: &frame,
            frame_id,
            now,
            in_view: true,
        };
        // The game not in view: the detector rests (nothing it measures
        // would be used), and so does the sight.
        let hidden = Look {
            in_view: false,
            ..look(0)
        };
        let unseen = perceive(&mut pipeline, Some(&mut sight), Detectors::HUD, &hidden);
        assert_eq!(unseen.world.hud.hp.failure_reason.as_deref(), NOT_RUN);
        assert!(!sight.sees_hud(frame.width(), frame.height()));
        // No layout yet: the detector rests still, and the sight looks for
        // the HUD itself.
        let first = perceive(&mut pipeline, Some(&mut sight), Detectors::HUD, &look(1));
        assert_eq!(first.world.hud.hp.failure_reason.as_deref(), NOT_RUN);
        assert!(sight.sees_hud(frame.width(), frame.height()));
        // From here the sight's bars answer and the detector rests. The
        // bars say nothing until a reading fits their tracks (where a fill
        // ends is seen; where its track ends is not).
        let second = perceive(&mut pipeline, Some(&mut sight), Detectors::HUD, &look(2));
        assert_eq!(second.world.hud.hp.failure_reason.as_deref(), NOT_RUN);
        assert!(second.obs.hp.is_none(), "{:?}", second.obs.hp);
        sight.verified(
            &frame,
            &crate::sight::teacher::HudValues {
                hp: Some((320, 400)),
                mp: Some((675, 1351)),
                exp_percent: Some(30.0),
                ..Default::default()
            },
        );
        let third = perceive(&mut pipeline, Some(&mut sight), Detectors::HUD, &look(3));
        assert_eq!(third.world.hud.hp.failure_reason.as_deref(), NOT_RUN);
        let hp = third.obs.hp.expect("the sight's HP");
        assert!((hp.percent - 80.0).abs() < 3.0, "{hp:?}");
        assert!(!hp.read);
        // The scene's detectors for Claude: they run, and the HUD's is still
        // the sight's to answer.
        let scene = Detectors {
            motion: true,
            dialog: true,
            panels: true,
            ..Detectors::HUD
        };
        let rich = perceive(&mut pipeline, Some(&mut sight), scene, &look(4));
        assert_eq!(rich.world.hud.hp.failure_reason.as_deref(), NOT_RUN);
        assert_ne!(rich.world.motion.failure_reason.as_deref(), NOT_RUN);
        assert_ne!(rich.world.icon_row.failure_reason.as_deref(), NOT_RUN);
        assert!(rich.obs.hp.is_some(), "the sight's HP still");
        // The preview wants the detector's picture whatever the sight sees.
        let shown = perceive(&mut pipeline, Some(&mut sight), Detectors::ALL, &look(5));
        assert_ne!(shown.world.hud.hp.failure_reason.as_deref(), NOT_RUN);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
