//! Minimal scene skeleton for a ranim one-shot example.
//!
//! Abridged from the pilot `convolution_kernels` run (the shared-timeline
//! pattern: every item group owns one full-lifecycle `AnimSequence`, aligned
//! with `forward_to` / `hold_to`, all pushed into a single `AnimStack`).
//!
//! NOT a compiled artifact — API surface drifts between ranim revs. Copy the
//! *shape*, then verify against the pin you are building on (compare with a
//! merged run's source on main if in doubt).

use ranim::{
    anims::fading::{FadeOut, FadingAnim},
    color::palettes::manim,
    glam::dvec3,
    items::vitem::{VItem, geometry::Square, text::TextItem},
    prelude::*,
    utils::rate_functions::linear,
};

/// Act boundaries in seconds. Plan acts on paper first (see SKILL.md §1);
/// the timeline is derived from the beat list, not invented while coding.
const ACT0_T0: f64 = 0.0;
const ACT1_T0: f64 = 16.0;
const TOTAL: f64 = 120.0;

/// One full-lifecycle sequence for an item group: fade in at `in_t`,
/// (optional event morphs), fade out at `out_t`, hold to the end.
fn life_seq(items: Vec<VItem>, in_t: f64, out_t: f64) -> AnimSequence {
    let mut s = AnimSequence::new();
    s.forward_to(in_t);
    s.fade_in().with_duration(1.0).with_rate_func(linear);
    s.hold_to(out_t);
    s.play(items.fade_out().with_duration(1.0));
    s.hold_to(TOTAL);
    s
}

// MARK: real simulation
// Build the real implementation here (simulator / algorithm), assert its
// behavior with #[cfg(test)] tests, and drive the timeline from its output
// event sequence. No hand-written animation numbers.

#[scene]
#[output(dir = "./output/<topic>")]
fn topic_scene(r: &mut RanimScene) {
    let mut content = AnimStack::new();

    // Title / hook card (act 0)
    {
        let title: Vec<VItem> = TextItem::new("Hook")
            .with(|t| t.fill_color = manim::WHITE)
            .into();
        // ... position, build items, push life_seq(...)
    }

    // Per act: build item groups, push their sequences onto `content`.
    // Drive events from the simulation output (see SKILL.md §2).

    r.play(CameraFrame::default().show().with_duration(TOTAL));
    r.play(content);

    // Captures: preview.png is the densest "hero" frame; add 2-5 more at
    // key beats. Copy the produced files into the run directory on delivery.
    r.insert_time_mark(TOTAL * 0.5, TimeMark::Capture("preview.png".to_string()));
    r.insert_time_mark(TOTAL, TimeMark::Capture("end.png".to_string()));
}
