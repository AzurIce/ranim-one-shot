//! Linux memory allocation — how a `malloc(100)` actually gets its bytes.
//!
//! A ~5.7 minute explainer in five acts, aimed at people with basic computer
//! literacy and no prior knowledge of kernel memory management:
//!
//! 1. **Hook** — a program calls `malloc`, gets a pointer, and nobody asks
//!    where the bytes came from. Memory = a huge array of numbered bytes;
//!    allocating = finding an empty stretch and handing back its address.
//! 2. **From segments to pages** — every process believes it owns all of
//!    memory (virtual addresses). Plan A, segments (base + limit), works
//!    until external fragmentation eats it (a computed first-fit demo where
//!    128 KB free cannot satisfy 80 KB). Plan B, paging: both spaces cut
//!    into fixed 4 KB pages, page tables translate, multi-level tables keep
//!    the bookkeeping affordable, TLB caches the walk.
//! 3. **The buddy allocator** — who owns the physical frames. A full buddy
//!    simulation (orders 0-4 over 16 pages) drives the animation: splits
//!    cascade down on allocation, `p ^ size` finds the buddy, frees cascade
//!    back up until the arena is whole again.
//! 4. **Slab** — the retail layer. One cache per kernel object type, pages
//!    cut into equal slots, freed slots handed back warm, `kmalloc` size
//!    classes round requests up.
//! 5. **The whole journey** — the supply chain of one `malloc(100)`:
//!    libc's stockpile, mmap on paper, and the page fault that finally
//!    summons a physical frame (demand paging). Recap cards close.
//!
//! Every number on screen (first-fit placements, address translation,
//! split/merge cascades, slot reuse) is produced by real implementations in
//! this file — nothing is hand-animated. Structure follows
//! `bpe_tokenizer`: one scene, a shared `AnimStack`, each group owning its
//! full life cycle (fade in, optional morph events, fade out).

use ranim::{
    anims::{fading::FadingAnim, morph::MorphAnim},
    color::{AlphaColor, Srgb, palettes::manim, rgb8},
    glam::{DVec3, dvec3},
    items::vitem::{
        VItem,
        geometry::{Line, Rectangle},
        text::TextItem,
    },
    prelude::*,
    utils::rate_functions::smooth,
};

// MARK: Segmentation — first-fit simulation

/// One operation of the fragmentation demo.
#[derive(Clone, Copy, Debug)]
enum SegOp {
    /// Allocate `size` KB for a block named `id`.
    Alloc(char, u32),
    /// Free the block named `id`.
    Free(char),
}

/// The outcome of one [`SegOp`] against the arena.
#[derive(Clone, Copy, Debug)]
struct SegStep {
    op: SegOp,
    /// Start offset in KB; `None` if the allocation failed.
    start: Option<u32>,
    len: u32,
}

/// First-fit allocation over a linear arena (units of KB). Free blocks are
/// the gaps between live blocks; the first gap that fits wins.
fn seg_first_fit(arena_kb: u32, ops: &[SegOp]) -> Vec<SegStep> {
    let mut live: Vec<(char, u32, u32)> = Vec::new(); // (id, start, len)
    let mut steps = Vec::new();
    for &op in ops {
        match op {
            SegOp::Alloc(id, len) => {
                // Collect gaps: [prev_end .. next_start], including the ends.
                let mut gaps: Vec<(u32, u32)> = Vec::new();
                let mut cursor = 0;
                for &(_, s, l) in &live {
                    if s > cursor {
                        gaps.push((cursor, s - cursor));
                    }
                    cursor = s + l;
                }
                if arena_kb > cursor {
                    gaps.push((cursor, arena_kb - cursor));
                }
                let hit = gaps.iter().find(|(_, glen)| *glen >= len).copied();
                let start = hit.map(|(s, _)| s);
                if let Some(s) = start {
                    live.push((id, s, len));
                    live.sort_by_key(|&(_, s, _)| s);
                }
                steps.push(SegStep { op, start, len });
            }
            SegOp::Free(id) => {
                let &(_, s, l) = live.iter().find(|&&(i, _, _)| i == id).unwrap();
                live.retain(|&(i, _, _)| i != id);
                steps.push(SegStep {
                    op,
                    start: Some(s),
                    len: l,
                });
            }
        }
    }
    steps
}

/// The demo timeline: fill, punch two holes, then ask for too much.
const SEG_OPS: [SegOp; 7] = [
    SegOp::Alloc('A', 64),
    SegOp::Alloc('B', 64),
    SegOp::Alloc('C', 64),
    SegOp::Alloc('D', 32),
    SegOp::Free('B'),
    SegOp::Free('D'),
    SegOp::Alloc('E', 80),
];
const SEG_ARENA_KB: u32 = 256;

// MARK: Paging — address translation & table math

const PAGE_SIZE: u32 = 1 << 12;
const DEMO_VA: u32 = 0x0040_1234;
const DEMO_VPN: u32 = DEMO_VA >> 12; // 0x00401
const DEMO_OFFSET: u32 = DEMO_VA & (PAGE_SIZE - 1); // 0x234
const DEMO_FRAME: u32 = 0x007;
const DEMO_PA: u32 = (DEMO_FRAME << 12) | DEMO_OFFSET; // 0x007234

/// A window of the page table shown next to the translation.
const DEMO_TABLE: [(u32, u32); 3] = [(0x00400, 0x002), (0x00401, 0x007), (0x00402, 0x00A)];

/// Cost of a flat single-level table for a 32-bit space.
const FLAT_ENTRIES: u32 = 1 << 20;
const FLAT_BYTES: u32 = FLAT_ENTRIES * 4; // 4 MiB per process

/// x86-64 multi-level geometry.
const LEVELS: u32 = 4;
const BITS_PER_LEVEL: u32 = 9;
const VA_BITS: u32 = LEVELS * BITS_PER_LEVEL + 12; // 48
const ENTRIES_PER_TABLE: u32 = 1 << BITS_PER_LEVEL; // 512
const TABLE_BYTES: u32 = ENTRIES_PER_TABLE * 8; // 4096 = one page

// MARK: Buddy allocator simulation

const B_MAX_ORDER: u8 = 4; // arena = 16 pages = 64 KB

#[derive(Clone, Copy, Debug, PartialEq)]
enum BOp {
    /// Allocate `order`: a block of `2^order` pages.
    Alloc(char, u8),
    /// Free the block named `id`.
    Free(char),
}

/// One step the buddy allocator takes, in order.
#[derive(Clone, Copy, Debug, PartialEq)]
enum BEvent {
    /// Block (`start`, `order`) splits into (`start`, `order-1`) and its
    /// buddy (`start + 2^(order-1)`, `order-1`).
    Split { start: u32, order: u8 },
    /// Block (`start`, `order`) is handed to the requester.
    Grant { start: u32, order: u8 },
    /// Block (`start`, `order`) is parked on its order's free list.
    ListAdd { start: u32, order: u8 },
    /// Block (`start`, `order`) and its buddy merge into
    /// (`start`, `order + 1`).
    Merge { start: u32, order: u8 },
}

/// Run the buddy allocator over `ops`, returning each op with its event log.
fn buddy_sim(ops: &[BOp]) -> Vec<(BOp, Vec<BEvent>)> {
    let mut free_lists: Vec<Vec<u32>> = vec![Vec::new(); (B_MAX_ORDER + 1) as usize];
    free_lists[B_MAX_ORDER as usize].push(0); // the whole arena starts free
    let mut live: Vec<(char, u32, u8)> = Vec::new();
    let mut out = Vec::new();

    for &op in ops {
        let mut events = Vec::new();
        match op {
            BOp::Alloc(id, order) => {
                // Smallest order >= `order` with a free block.
                let mut o = order;
                while free_lists[o as usize].is_empty() {
                    o += 1;
                }
                free_lists[o as usize].sort_unstable();
                let start = free_lists[o as usize].remove(0);
                // Split down the left spine until the right size.
                while o > order {
                    o -= 1;
                    events.push(BEvent::Split {
                        start,
                        order: o + 1,
                    });
                    let buddy = start + (1 << o);
                    free_lists[o as usize].push(buddy);
                    events.push(BEvent::ListAdd {
                        start: buddy,
                        order: o,
                    });
                }
                events.push(BEvent::Grant { start, order });
                live.push((id, start, order));
            }
            BOp::Free(id) => {
                let i = live.iter().position(|&(n, _, _)| n == id).unwrap();
                let (_, mut start, mut order) = live[i];
                live.remove(i);
                // Merge with the buddy while it is free.
                while order < B_MAX_ORDER {
                    let buddy = start ^ (1 << order);
                    let list = &mut free_lists[order as usize];
                    if let Some(j) = list.iter().position(|&s| s == buddy) {
                        list.remove(j);
                        start = start.min(buddy);
                        events.push(BEvent::Merge { start, order });
                        order += 1;
                    } else {
                        break;
                    }
                }
                free_lists[order as usize].push(start);
                events.push(BEvent::ListAdd { start, order });
            }
        }
        out.push((op, events));
    }
    out
}

/// The demo script: two split cascades down, then three frees that merge
/// all the way back to a single whole-arena block.
const B_OPS: [BOp; 6] = [
    BOp::Alloc('A', 1),
    BOp::Alloc('B', 0),
    BOp::Alloc('C', 2),
    BOp::Free('B'),
    BOp::Free('A'),
    BOp::Free('C'),
];

// MARK: Slab simulation

const SLOTS_PER_SLAB: usize = 4;

#[derive(Clone, Copy, Debug)]
enum SOp {
    Alloc(char),
    Free(char),
}

/// One slab operation's outcome.
#[derive(Clone, Copy, Debug)]
struct SStep {
    op: SOp,
    slab: usize,
    slot: usize,
    /// The slot served a freed object before — it is being reused warm.
    reused: bool,
}

/// One kmem-cache: slabs of equal slots. Allocation prefers the first
/// partial slab (lowest index, lowest free slot) and only grows when no
/// slab has a free slot.
fn slab_sim(ops: &[SOp]) -> Vec<SStep> {
    let mut slabs: Vec<[Option<char>; SLOTS_PER_SLAB]> = vec![[None; SLOTS_PER_SLAB]];
    let mut freed: Vec<(usize, usize)> = Vec::new(); // slots that served a free
    let mut live: Vec<(char, usize, usize)> = Vec::new();
    let mut steps = Vec::new();
    for &op in ops {
        match op {
            SOp::Alloc(id) => {
                let mut hit: Option<(usize, usize)> = None;
                'outer: for (si, slots) in slabs.iter().enumerate() {
                    for (li, slot) in slots.iter().enumerate() {
                        if slot.is_none() {
                            hit = Some((si, li));
                            break 'outer;
                        }
                    }
                }
                let (si, li) = hit.unwrap_or_else(|| {
                    slabs.push([None; SLOTS_PER_SLAB]);
                    (slabs.len() - 1, 0)
                });
                let reused = freed.contains(&(si, li));
                slabs[si][li] = Some(id);
                live.push((id, si, li));
                steps.push(SStep {
                    op,
                    slab: si,
                    slot: li,
                    reused,
                });
            }
            SOp::Free(id) => {
                let i = live.iter().position(|&(n, _, _)| n == id).unwrap();
                let (_, si, li) = live[i];
                live.remove(i);
                slabs[si][li] = None;
                freed.push((si, li));
                steps.push(SStep {
                    op,
                    slab: si,
                    slot: li,
                    reused: false,
                });
            }
        }
    }
    steps
}

/// The demo: fill slab 0, open slab 1, free two objects, watch their slots
/// get reused warm, then take one fresh slot.
const S_OPS: [SOp; 11] = [
    SOp::Alloc('a'),
    SOp::Alloc('b'),
    SOp::Alloc('c'),
    SOp::Alloc('d'),
    SOp::Alloc('e'),
    SOp::Alloc('f'),
    SOp::Free('b'),
    SOp::Free('e'),
    SOp::Alloc('g'),
    SOp::Alloc('h'),
    SOp::Alloc('i'),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn segmentation_hits_external_fragmentation() {
        let steps = seg_first_fit(SEG_ARENA_KB, &SEG_OPS);
        let starts: Vec<Option<u32>> = steps.iter().map(|s| s.start).collect();
        assert_eq!(
            starts,
            vec![
                Some(0),
                Some(64),
                Some(128),
                Some(192),
                Some(64),
                Some(192),
                None
            ]
        );
        // Free bytes: hole at 64 (64 KB) + 192..256 (64 KB) = 128 KB total.
        assert_eq!(steps[6].len, 80);
    }

    #[test]
    fn paging_translation_math() {
        assert_eq!(DEMO_VPN, 0x00401);
        assert_eq!(DEMO_OFFSET, 0x234);
        assert_eq!(DEMO_PA, 0x007_234);
        assert_eq!(FLAT_BYTES, 4 * 1024 * 1024);
        assert_eq!(VA_BITS, 48);
        assert_eq!(TABLE_BYTES, 4096);
    }

    #[test]
    fn buddy_cascade_matches_the_script() {
        let plan = buddy_sim(&B_OPS);
        let evs: Vec<Vec<BEvent>> = plan.iter().map(|(_, e)| e.clone()).collect();
        assert_eq!(
            evs[0],
            vec![
                BEvent::Split { start: 0, order: 4 },
                BEvent::ListAdd { start: 8, order: 3 },
                BEvent::Split { start: 0, order: 3 },
                BEvent::ListAdd { start: 4, order: 2 },
                BEvent::Split { start: 0, order: 2 },
                BEvent::ListAdd { start: 2, order: 1 },
                BEvent::Grant { start: 0, order: 1 },
            ]
        );
        assert_eq!(
            evs[1],
            vec![
                BEvent::Split { start: 2, order: 1 },
                BEvent::ListAdd { start: 3, order: 0 },
                BEvent::Grant { start: 2, order: 0 },
            ]
        );
        assert_eq!(evs[2], vec![BEvent::Grant { start: 4, order: 2 }]);
        assert_eq!(
            evs[3],
            vec![
                BEvent::Merge { start: 2, order: 0 },
                BEvent::ListAdd { start: 2, order: 1 },
            ]
        );
        assert_eq!(
            evs[4],
            vec![
                BEvent::Merge { start: 0, order: 1 },
                BEvent::ListAdd { start: 0, order: 2 },
            ]
        );
        assert_eq!(
            evs[5],
            vec![
                BEvent::Merge { start: 0, order: 2 },
                BEvent::Merge { start: 0, order: 3 },
                BEvent::ListAdd { start: 0, order: 4 },
            ]
        );
    }

    #[test]
    fn buddy_ends_whole() {
        let plan = buddy_sim(&B_OPS);
        // Replay the events to reconstruct the free lists.
        let mut lists: Vec<Vec<u32>> = vec![Vec::new(); 5];
        let mut take = |lists: &mut Vec<Vec<u32>>, order: u8, start: u32| {
            if let Some(i) = lists[order as usize].iter().position(|&s| s == start) {
                lists[order as usize].remove(i);
            }
        };
        for (_, events) in &plan {
            for e in events {
                match *e {
                    BEvent::Split { start, order } => take(&mut lists, order, start),
                    BEvent::Grant { start, order } => take(&mut lists, order, start),
                    BEvent::ListAdd { start, order } => lists[order as usize].push(start),
                    BEvent::Merge { start, order } => {
                        // Both halves leave their order's list.
                        take(&mut lists, order, start ^ (1 << order));
                        take(&mut lists, order, start);
                    }
                }
            }
        }
        assert_eq!(lists, vec![vec![], vec![], vec![], vec![], vec![0]]);
    }

    #[test]
    fn slab_reuses_warm_slots() {
        let steps = slab_sim(&S_OPS);
        let placements: Vec<(usize, usize)> = steps.iter().map(|s| (s.slab, s.slot)).collect();
        // a b c d fill slab 0; e f open slab 1; g reuses b's slot,
        // h reuses e's slot, i takes the next fresh slot.
        assert_eq!(
            placements,
            vec![
                (0, 0),
                (0, 1),
                (0, 2),
                (0, 3),
                (1, 0),
                (1, 1),
                (0, 1),
                (1, 0),
                (0, 1),
                (1, 0),
                (1, 2)
            ]
        );
        assert!(steps[8].reused && steps[9].reused && !steps[10].reused);
    }
}

// MARK: Colors

const TEXT_COL: AlphaColor<Srgb> = AlphaColor::WHITE;
const GREY: AlphaColor<Srgb> = manim::GREY_B;
const GREY_DIM: AlphaColor<Srgb> = manim::GREY_D;
const CHIP_FILL: AlphaColor<Srgb> = rgb8(0x26, 0x26, 0x2e);
const PANEL_FILL: AlphaColor<Srgb> = rgb8(0x1c, 0x1c, 0x24);
const GOOD: AlphaColor<Srgb> = manim::GREEN_C;
const BAD: AlphaColor<Srgb> = manim::RED_C;
const HILITE: AlphaColor<Srgb> = manim::GOLD_D;

/// Virtual memory / process A.
const VIRT: AlphaColor<Srgb> = manim::BLUE_C;
/// The second process.
const PROC2: AlphaColor<Srgb> = manim::PURPLE_C;
/// Physical memory.
const PHYS: AlphaColor<Srgb> = manim::GOLD_D;
/// Slab objects.
const SLAB: AlphaColor<Srgb> = manim::TEAL_C;
/// Free memory blocks.
const FREE_STROKE: AlphaColor<Srgb> = manim::GREY_C;
const FREE_FILL: AlphaColor<Srgb> = rgb8(0x23, 0x2b, 0x27);

// MARK: Layout helpers

/// Layout width of a text at `em` size, in world units.
fn text_width(text: &str, em: f64) -> f64 {
    TextItem::new(text, em).inline_length_em() * em
}

/// Single-line text as glyphs, ink-centered at `pos`.
fn text_vitems(text: &str, em: f64, pos: DVec3) -> Vec<VItem> {
    let mut vitems = Vec::<VItem>::from(TextItem::new(text, em));
    vitems.move_anchor_to(AabbPoint::CENTER, pos);
    vitems
}

/// Single-line text with its left edge at `x_left`, baseline at `y`.
fn text_left(text: &str, em: f64, x_left: f64, y: f64) -> Vec<VItem> {
    let mut vitems = Vec::<VItem>::from(TextItem::new(text, em));
    vitems.shift(dvec3(x_left, y, 0.0));
    vitems
}

/// A panel rectangle (dark fill, grey stroke) centered at `center`.
fn panel(center: DVec3, w: f64, h: f64, stroke: AlphaColor<Srgb>) -> VItem {
    let mut p = VItem::from(Rectangle::new(w, h));
    p.move_to(center);
    p.set_fill_color(PANEL_FILL).set_fill_opacity(0.6);
    p.set_stroke_color(stroke).set_stroke_width(0.02);
    p
}

/// A variable-width chip: box + glyphs, ink-centered at `center`.
fn chip(text: &str, em: f64, center: DVec3, stroke: AlphaColor<Srgb>) -> Vec<VItem> {
    let w = text_width(text, em) + 0.3;
    let mut box_item = VItem::from(Rectangle::new(w, em + 0.4));
    box_item.move_to(center);
    box_item.set_fill_color(CHIP_FILL).set_fill_opacity(0.92);
    box_item.set_stroke_color(stroke).set_stroke_width(0.022);
    let mut glyphs = Vec::<VItem>::from(TextItem::new(text, em));
    glyphs.move_to(center);
    glyphs.set_fill_color(TEXT_COL);
    let mut items = vec![box_item];
    items.extend(glyphs);
    items
}

/// An open arrowhead arrow: shaft + two flanks, all thin lines.
fn arrow_items(start: DVec3, end: DVec3, color: AlphaColor<Srgb>) -> Vec<VItem> {
    let d = (end - start).normalize_or_zero();
    let n = dvec3(-d.y, d.x, 0.0);
    let mut items = Vec::new();
    let mut shaft = VItem::from(Line::new(start, end - d * 0.05));
    shaft.set_stroke_color(color).set_stroke_width(0.02);
    items.push(shaft);
    for side in [-1.0, 1.0] {
        let flank = Line::new(end - d * 0.15 + n * side * 0.055, end - d * 0.01);
        let mut f = VItem::from(flank);
        f.set_stroke_color(color).set_stroke_width(0.02);
        items.push(f);
    }
    items
}

/// `n` stroke-only cells of a bar, left edge `x0`, vertically centered `y`.
fn cell_row(n: usize, x0: f64, y: f64, cw: f64, ch: f64, stroke: AlphaColor<Srgb>) -> Vec<VItem> {
    (0..n)
        .map(|i| {
            let mut c = VItem::from(Rectangle::new(cw - 0.02, ch - 0.02));
            c.move_to(dvec3(x0 + (i as f64 + 0.5) * cw, y, 0.0));
            c.set_fill_opacity(0.0);
            c.set_stroke_color(stroke).set_stroke_width(0.014);
            c
        })
        .collect()
}

/// Center x of cells `[a, b)` in such a row.
fn span_center_x(x0: f64, cw: f64, a: usize, b: usize) -> f64 {
    x0 + (a as f64 + (b - a) as f64 / 2.0) * cw
}

/// Width of cells `[a, b)`.
fn span_w(cw: f64, a: usize, b: usize) -> f64 {
    (b - a) as f64 * cw - 0.05
}

/// A filled rectangle over cells `[a, b)` of a bar (no label).
#[allow(clippy::too_many_arguments)]
fn span_rect(
    x0: f64,
    y: f64,
    cw: f64,
    ch: f64,
    a: usize,
    b: usize,
    fill: AlphaColor<Srgb>,
    fill_opacity: f32,
    stroke: AlphaColor<Srgb>,
) -> VItem {
    let mut r = VItem::from(Rectangle::new(span_w(cw, a, b), ch - 0.05));
    r.move_to(dvec3(span_center_x(x0, cw, a, b), y, 0.0));
    r.set_fill_color(fill).set_fill_opacity(fill_opacity);
    r.set_stroke_color(stroke).set_stroke_width(0.02);
    r
}

// MARK: Sequences

/// A paint applied to a group's items to produce its next look.
type Paint = Box<dyn Fn(&mut Vec<VItem>)>;

/// An event in a group's life: at `at`, morph the group into `paint(items)`.
struct Ev {
    at: f64,
    dur: f64,
    paint: Paint,
}

fn ev(at: f64, dur: f64, paint: impl Fn(&mut Vec<VItem>) + 'static) -> Ev {
    Ev {
        at,
        dur,
        paint: Box::new(paint),
    }
}

/// A group's full life: fade in (per-item `lag`), optional morph events,
/// fade out at `out_t`, then hold (invisible) to `TOTAL`.
fn life_seq(
    items: &mut Vec<VItem>,
    in_t: f64,
    out_t: f64,
    in_dur: f64,
    out_dur: f64,
    lag: f64,
    events: Vec<Ev>,
) -> AnimSequence {
    let mut s = AnimSequence::new();
    s.forward_to(in_t);
    s.push(
        items
            .iter_mut()
            .map(|it| it.fade_in().with_duration(in_dur))
            .into_lagged(lag),
    );
    for e in events {
        if e.at > in_t {
            s.hold_to(e.at);
            let mut dst = items.clone();
            (e.paint)(&mut dst);
            s.push(
                items
                    .morph_to(dst)
                    .with_duration(e.dur)
                    .with_rate_func(smooth),
            );
        }
    }
    if out_t < TOTAL {
        s.hold_to(out_t);
        s.push(
            items
                .iter_mut()
                .map(|it| it.fade_out().with_duration(out_dur))
                .into_lagged(0.0),
        );
    }
    s.hold_to(TOTAL);
    s
}

/// Fade-in/hold/fade-out caption.
fn caption(
    content: &mut AnimStack,
    text: &str,
    em: f64,
    pos: DVec3,
    color: AlphaColor<Srgb>,
    in_t: f64,
    out_t: f64,
) {
    let mut items = text_vitems(text, em, pos);
    items.set_fill_color(color);
    content.push(life_seq(&mut items, in_t, out_t, 0.5, 0.5, 0.0, vec![]));
}

/// Left-aligned log line with a colored accent.
#[allow(clippy::too_many_arguments)]
fn log_line(
    content: &mut AnimStack,
    text: &str,
    em: f64,
    x_left: f64,
    y: f64,
    color: AlphaColor<Srgb>,
    in_t: f64,
    out_t: f64,
) {
    let mut items = text_left(text, em, x_left, y);
    items.set_fill_color(color);
    content.push(life_seq(&mut items, in_t, out_t, 0.4, 0.4, 0.0, vec![]));
}

// MARK: Timeline

const A0_END: f64 = 20.0;
const A1_IN: f64 = 21.4;
const A2_IN: f64 = 120.8;
const A2_OUT: f64 = 218.4;
const A3_IN: f64 = 219.8;
const A4_IN: f64 = 296.8;
const OUT_T: f64 = 340.6;
const TOTAL: f64 = 343.0;

/// Act 2 (buddy): fixed op-line times; events step inside each op.
const B_OP_T: [f64; 6] = [145.4, 160.6, 166.6, 180.4, 185.8, 193.6];
const B_EV_STEP: f64 = 1.3;

/// Act 2 (buddy) bar geometry: 16 pages as cells.
const BB_N: usize = 16;
const BB_CELL: f64 = 0.52;
const BB_X0: f64 = -4.85;
const BB_Y: f64 = 1.15;
const BB_H: f64 = 0.64;

/// Act 2 free-list panel.
const FL_CHIP_X: f64 = 6.3;
const FL_TOP: f64 = 1.9;
const FL_DY: f64 = 0.74;

/// Act 1 paging beat geometry.
const PG_N: usize = 8;
const PG_CELL: f64 = 0.92;
const PG_X0: f64 = -6.5;
const PG_VY: f64 = 1.8;
const PG_VH: f64 = 0.78;
const PM_N: usize = 16;
const PM_CELL: f64 = 0.46;
const PM_Y: f64 = -1.8;
const PM_H: f64 = 0.62;
const PT_X: f64 = 5.3;
const PT_W: f64 = 3.0;
const PT_H: f64 = 4.3;
const PT_CY: f64 = 0.05;
const PT_ROW_DY: f64 = 0.82;
const PT_ROW0_Y: f64 = 1.35;

/// Act 1 fragmentation beat geometry: 16 cells of 16 KB.
const FG_N: usize = 16;
const FG_CELL: f64 = 0.66;
const FG_X0: f64 = -5.28;
const FG_Y: f64 = 1.05;
const FG_H: f64 = 0.7;
const FG_OP_T: [f64; 7] = [50.0, 51.8, 53.6, 55.4, 57.2, 59.0, 61.2];

/// Act 3 slab geometry.
const SL_X: [f64; 3] = [-4.5, 0.0, 4.5];
const SL_Y: f64 = 1.4;
const SL_W: f64 = 3.4;
const SL_H: f64 = 1.2;
const SL_SLOT: f64 = 0.72;

// MARK: Act 0 — hook

fn act0(content: &mut AnimStack) {
    // Title & subtitle make way for the question at 10.6.
    let mut title = text_vitems("Where Does Memory Come From?", 0.7, dvec3(0.0, 1.3, 0.0));
    content.push(life_seq(&mut title, 0.3, 10.4, 0.9, 0.6, 0.0, vec![]));
    caption(
        content,
        "the journey of a single allocation",
        0.34,
        dvec3(0.0, 0.6, 0.0),
        GREY,
        0.9,
        10.4,
    );

    // The one-liner every programmer has written.
    let mut code = text_vitems("buf = malloc(100)", 0.46, dvec3(0.0, -0.55, 0.0));
    content.push(life_seq(&mut code, 2.6, 10.4, 0.6, 0.5, 0.0, vec![]));
    let mut ptr = chip("0x7f3a2c000010", 0.3, dvec3(0.0, -1.6, 0.0), GREY);
    content.push(life_seq(
        &mut ptr,
        3.9,
        10.4,
        0.4,
        0.5,
        0.0,
        vec![ev(4.7, 0.4, move |items: &mut Vec<VItem>| {
            items.set_stroke_color(VIRT);
        })],
    ));
    caption(
        content,
        "all you get back is a number",
        0.3,
        dvec3(0.0, -2.55, 0.0),
        GREY,
        4.9,
        7.0,
    );
    caption(
        content,
        "programs do this thousands of times a second",
        0.3,
        dvec3(0.0, -2.55, 0.0),
        GREY,
        7.4,
        9.8,
    );

    // The question, then the mental model: memory is numbered bytes.
    caption(
        content,
        "but who actually found those bytes?",
        0.5,
        dvec3(0.0, 0.55, 0.0),
        TEXT_COL,
        10.8,
        15.8,
    );
    const MB_N: usize = 24;
    const MB_CELL: f64 = 0.5;
    const MB_X0: f64 = -6.0;
    const MB_Y: f64 = -1.35;
    let mut cells = cell_row(MB_N, MB_X0, MB_Y, MB_CELL, 0.55, GREY_DIM);
    content.push(life_seq(&mut cells, 11.4, 15.8, 0.4, 0.5, 0.012, vec![]));
    let mut zero = text_vitems("byte 0", 0.2, dvec3(MB_X0 + 0.45, MB_Y - 0.55, 0.0));
    zero.set_fill_color(GREY);
    content.push(life_seq(&mut zero, 11.8, 15.8, 0.3, 0.3, 0.0, vec![]));
    let mut last = text_vitems(
        "byte N",
        0.2,
        dvec3(MB_X0 + MB_N as f64 * MB_CELL - 0.45, MB_Y - 0.55, 0.0),
    );
    last.set_fill_color(GREY);
    content.push(life_seq(&mut last, 12.0, 15.8, 0.3, 0.3, 0.0, vec![]));
    let mut win = vec![span_rect(
        MB_X0, MB_Y, MB_CELL, 0.55, 9, 14, GOOD, 0.25, GOOD,
    )];
    content.push(life_seq(&mut win, 13.0, 15.8, 0.35, 0.4, 0.0, vec![]));
    caption(
        content,
        "allocating = find an empty stretch and hand back its address",
        0.3,
        dvec3(0.0, -2.55, 0.0),
        TEXT_COL,
        13.4,
        15.8,
    );

    // The supply chain, as a three-item table of contents.
    caption(
        content,
        "behind it: a supply chain",
        0.3,
        dvec3(0.0, 1.95, 0.0),
        GREY,
        16.2,
        A0_END,
    );
    for (i, (text, color)) in [
        ("1 · pages — translate and isolate", VIRT),
        ("2 · buddy — wholesale physical frames", PHYS),
        ("3 · slab — retail for small objects", SLAB),
    ]
    .into_iter()
    .enumerate()
    {
        let mut c = chip(text, 0.3, dvec3(0.0, 1.05 - i as f64 * 0.9, 0.0), color);
        content.push(life_seq(
            &mut c,
            16.5 + i as f64 * 0.3,
            A0_END,
            0.35,
            0.5,
            0.0,
            vec![],
        ));
    }
}

// MARK: Act 1 — from segments to pages

/// Process palette for the fragmentation demo blocks.
const SEG_COLS: [AlphaColor<Srgb>; 4] = [VIRT, SLAB, PROC2, manim::ORANGE];

/// The segment-translation example (all derived, never hand-computed).
const SEG_VBASE: u32 = 0x0040_0000;
const SEG_PBASE: u32 = 0x0020_0000;
const SEG_VA: u32 = 0x0040_2000;
const SEG_LIMIT_KB: u32 = 64;

fn hex(v: u32, digits: usize) -> String {
    format!("0x{:0width$x}", v, width = digits)
}

/// A filled rect between absolute x coordinates (for region maps).
fn region_rect(
    x_l: f64,
    x_r: f64,
    y: f64,
    h: f64,
    fill: AlphaColor<Srgb>,
    stroke: AlphaColor<Srgb>,
) -> VItem {
    let mut r = VItem::from(Rectangle::new(x_r - x_l - 0.04, h - 0.04));
    r.move_to(dvec3((x_l + x_r) / 2.0, y, 0.0));
    r.set_fill_color(fill).set_fill_opacity(0.3);
    r.set_stroke_color(stroke).set_stroke_width(0.02);
    r
}

fn act1(content: &mut AnimStack) {
    // Beat 1 — the lie: every process owns all of it.
    caption(
        content,
        "every program is told the same lie",
        0.45,
        dvec3(0.0, 3.4, 0.0),
        TEXT_COL,
        A1_IN,
        36.6,
    );
    caption(
        content,
        "a private address space, all to itself",
        0.3,
        dvec3(0.0, 2.85, 0.0),
        GREY,
        21.9,
        25.2,
    );
    let mut panel_a = vec![panel(dvec3(-3.5, 1.35, 0.0), 5.8, 2.9, VIRT)];
    content.push(life_seq(&mut panel_a, 22.3, 36.2, 0.5, 0.6, 0.0, vec![]));
    let mut panel_b = vec![panel(dvec3(3.5, 1.35, 0.0), 5.8, 2.9, PROC2)];
    content.push(life_seq(&mut panel_b, 22.9, 36.2, 0.5, 0.6, 0.0, vec![]));
    let mut label_a = text_vitems("program A", 0.32, dvec3(-3.5, 2.42, 0.0));
    label_a.set_fill_color(VIRT);
    content.push(life_seq(&mut label_a, 22.5, 36.2, 0.4, 0.5, 0.0, vec![]));
    let mut label_b = text_vitems("program B", 0.32, dvec3(3.5, 2.42, 0.0));
    label_b.set_fill_color(PROC2);
    content.push(life_seq(&mut label_b, 23.1, 36.2, 0.4, 0.5, 0.0, vec![]));
    for (x, color, in_t) in [(-3.5, VIRT, 23.2), (3.5, PROC2, 23.7)] {
        let mut hint = text_vitems("where my bytes live", 0.24, dvec3(x, 1.7, 0.0));
        hint.set_fill_color(GREY);
        content.push(life_seq(&mut hint, in_t, 36.2, 0.35, 0.4, 0.0, vec![]));
        let mut c = chip("0x00400000", 0.36, dvec3(x, 1.05, 0.0), color);
        content.push(life_seq(
            &mut c,
            in_t + 0.25,
            36.2,
            0.35,
            0.5,
            0.0,
            vec![ev(25.3, 0.4, move |items: &mut Vec<VItem>| {
                items.set_stroke_color(HILITE);
            })],
        ));
    }
    caption(
        content,
        "both believe they own byte 0x00400000",
        0.3,
        dvec3(0.0, -0.9, 0.0),
        GREY,
        25.9,
        28.0,
    );
    caption(
        content,
        "they can't both be right — those are virtual addresses",
        0.32,
        dvec3(0.0, -0.9, 0.0),
        TEXT_COL,
        28.4,
        31.4,
    );
    let mut ram = vec![panel(dvec3(0.0, -2.1, 0.0), 11.4, 0.62, PHYS)];
    content.push(life_seq(&mut ram, 31.8, 36.2, 0.5, 0.6, 0.0, vec![]));
    let mut ram_label = text_vitems(
        "physical memory — the real thing",
        0.28,
        dvec3(0.0, -2.95, 0.0),
    );
    ram_label.set_fill_color(PHYS);
    content.push(life_seq(&mut ram_label, 32.1, 36.2, 0.4, 0.5, 0.0, vec![]));
    caption(
        content,
        "the lie needs a translator: virtual → physical",
        0.34,
        dvec3(0.0, -0.9, 0.0),
        TEXT_COL,
        32.7,
        36.2,
    );

    // Beat 2 — plan A: segments.
    caption(
        content,
        "plan A — cut memory into segments",
        0.45,
        dvec3(0.0, 3.4, 0.0),
        TEXT_COL,
        37.0,
        48.2,
    );
    let mut seg_panel = vec![panel(dvec3(-4.1, 1.3, 0.0), 5.0, 3.6, VIRT)];
    content.push(life_seq(&mut seg_panel, 37.4, 48.2, 0.5, 0.6, 0.0, vec![]));
    let mut seg_label = text_vitems("program A", 0.3, dvec3(-4.1, 2.72, 0.0));
    seg_label.set_fill_color(VIRT);
    content.push(life_seq(&mut seg_label, 37.5, 48.2, 0.4, 0.5, 0.0, vec![]));
    for (i, (name, color)) in [
        ("code · 64 KB", VIRT),
        ("data · 32 KB", SLAB),
        ("stack · 64 KB", PROC2),
    ]
    .into_iter()
    .enumerate()
    {
        let mut c = chip(name, 0.28, dvec3(-4.1, 2.1 - i as f64 * 0.75, 0.0), color);
        content.push(life_seq(
            &mut c,
            37.8 + i as f64 * 0.25,
            48.2,
            0.35,
            0.5,
            0.0,
            vec![],
        ));
    }
    // Physical bar with the three segments dropped anywhere.
    const SB_X0: f64 = -1.0;
    const SB_X1: f64 = 6.9;
    const SB_Y: f64 = 1.3;
    let mut bar = vec![panel(
        dvec3((SB_X0 + SB_X1) / 2.0, SB_Y, 0.0),
        SB_X1 - SB_X0,
        0.78,
        GREY,
    )];
    content.push(life_seq(&mut bar, 38.0, 48.2, 0.5, 0.6, 0.0, vec![]));
    let regions: [(f64, f64, AlphaColor<Srgb>); 3] =
        [(0.06, 0.32, VIRT), (0.46, 0.64, SLAB), (0.80, 0.97, PROC2)];
    for (i, (a, b, color)) in regions.into_iter().enumerate() {
        let mut r = vec![region_rect(
            SB_X0 + a * (SB_X1 - SB_X0),
            SB_X0 + b * (SB_X1 - SB_X0),
            SB_Y,
            0.78,
            color,
            color,
        )];
        content.push(life_seq(
            &mut r,
            38.6 + i as f64 * 0.25,
            48.2,
            0.35,
            0.5,
            0.0,
            vec![],
        ));
    }
    let mut bar_label = text_vitems(
        "physical memory",
        0.26,
        dvec3((SB_X0 + SB_X1) / 2.0, 0.55, 0.0),
    );
    bar_label.set_fill_color(GREY);
    content.push(life_seq(&mut bar_label, 38.7, 48.2, 0.4, 0.5, 0.0, vec![]));
    // Arrows from each segment chip to its region.
    let chip_y = [2.1, 1.35, 0.6];
    for (i, (a, _b, color)) in regions.into_iter().enumerate() {
        let start = dvec3(-1.55, chip_y[i], 0.0);
        let end = dvec3(SB_X0 + (a + 0.04) * (SB_X1 - SB_X0), SB_Y + 0.28, 0.0);
        let mut arrow = arrow_items(start, end, color);
        content.push(life_seq(
            &mut arrow,
            39.4 + i as f64 * 0.3,
            48.2,
            0.35,
            0.5,
            0.0,
            vec![],
        ));
    }
    caption(
        content,
        "a segment = base + limit — a variable-size stretch, placed anywhere",
        0.28,
        dvec3(2.9, -0.35, 0.0),
        GREY,
        39.2,
        43.0,
    );
    // One translation, computed live from the consts.
    let seg_off = SEG_VA - SEG_VBASE;
    let seg_pa = SEG_PBASE + seg_off;
    let mut tr_panel = vec![panel(dvec3(3.9, -1.9, 0.0), 6.6, 2.5, GREY)];
    content.push(life_seq(&mut tr_panel, 40.4, 48.2, 0.5, 0.6, 0.0, vec![]));
    for (i, (text, color, in_t)) in [
        (format!("the cpu wants {}", hex(SEG_VA, 8)), TEXT_COL, 40.9),
        (
            format!(
                "offset = {} - {} = {}",
                hex(SEG_VA, 8),
                hex(SEG_VBASE, 8),
                hex(seg_off, 4)
            ),
            GREY,
            41.8,
        ),
        (
            format!(
                "{} is below the {} KB limit ✓",
                hex(seg_off, 4),
                SEG_LIMIT_KB
            ),
            GREY,
            42.7,
        ),
        (
            format!(
                "physical = {} + {} = {}",
                hex(SEG_PBASE, 8),
                hex(seg_off, 4),
                hex(seg_pa, 8)
            ),
            TEXT_COL,
            43.6,
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let mut line = text_left(&text, 0.24, 1.0, -1.25 - i as f64 * 0.52);
        line.set_fill_color(color);
        content.push(life_seq(&mut line, in_t, 48.2, 0.4, 0.5, 0.0, vec![]));
    }
    caption(
        content,
        "one add + one compare per access — and isolation comes free",
        0.3,
        dvec3(0.0, -3.6, 0.0),
        TEXT_COL,
        44.8,
        47.2,
    );
    caption(
        content,
        "so what breaks?",
        0.4,
        dvec3(0.0, -3.6, 0.0),
        HILITE,
        47.6,
        49.2,
    );

    // Beat 3 — external fragmentation, computed by a real first-fit pass.
    caption(
        content,
        "the catch — memory shatters into holes",
        0.45,
        dvec3(0.0, 3.4, 0.0),
        TEXT_COL,
        48.6,
        69.4,
    );
    let mut cells = cell_row(FG_N, FG_X0, FG_Y, FG_CELL, FG_H, GREY_DIM);
    content.push(life_seq(&mut cells, 49.0, 69.4, 0.4, 0.5, 0.012, vec![]));
    let mut bar_label = text_vitems("physical memory · 256 KB", 0.26, dvec3(0.0, 1.78, 0.0));
    bar_label.set_fill_color(GREY);
    content.push(life_seq(&mut bar_label, 49.1, 69.4, 0.4, 0.5, 0.0, vec![]));
    for k in 0..=4 {
        let mut lbl = text_vitems(
            &if k == 4 {
                "256 KB".to_string()
            } else {
                format!("{}", k * 64)
            },
            0.2,
            dvec3(FG_X0 + k as f64 * 4.0 * FG_CELL, FG_Y - 0.62, 0.0),
        );
        lbl.set_fill_color(GREY);
        content.push(life_seq(&mut lbl, 49.3, 69.4, 0.3, 0.4, 0.0, vec![]));
    }
    let steps = seg_first_fit(SEG_ARENA_KB, &SEG_OPS);
    let kb_per_cell = SEG_ARENA_KB / FG_N as u32;
    for (i, step) in steps.iter().enumerate() {
        let t = FG_OP_T[i];
        match step.op {
            SegOp::Alloc(id, len) => {
                if let Some(start) = step.start {
                    let color = SEG_COLS[i.min(SEG_COLS.len() - 1)];
                    let a = (start / kb_per_cell) as usize;
                    let b = a + (len / kb_per_cell) as usize;
                    // Freed blocks (B, D) vanish when their free op runs.
                    let out_t = match id {
                        'B' => FG_OP_T[4] + 0.25,
                        'D' => FG_OP_T[5] + 0.25,
                        _ => 69.4,
                    };
                    let mut rect = vec![span_rect(
                        FG_X0, FG_Y, FG_CELL, FG_H, a, b, color, 0.3, color,
                    )];
                    content.push(life_seq(&mut rect, t, out_t, 0.35, 0.4, 0.0, vec![]));
                    let mut letter = text_vitems(
                        &id.to_string(),
                        0.34,
                        dvec3(span_center_x(FG_X0, FG_CELL, a, b), FG_Y, 0.0),
                    );
                    letter.set_fill_color(TEXT_COL);
                    content.push(life_seq(
                        &mut letter,
                        t + 0.15,
                        out_t,
                        0.3,
                        0.4,
                        0.0,
                        vec![],
                    ));
                    let end = start + len;
                    log_line(
                        content,
                        &format!("{id} = alloc({len} KB)  →  {start}-{end} KB"),
                        0.24,
                        -6.95,
                        -0.15 - i as f64 * 0.5,
                        GREY,
                        t,
                        69.4,
                    );
                } else {
                    log_line(
                        content,
                        &format!("{id} = alloc({len} KB)  →  ✗ no hole fits"),
                        0.24,
                        -6.95,
                        -0.15 - i as f64 * 0.5,
                        BAD,
                        t,
                        69.4,
                    );
                }
            }
            SegOp::Free(id) => {
                log_line(
                    content,
                    &format!("free({id})  →  a hole opens"),
                    0.24,
                    -6.95,
                    -0.15 - i as f64 * 0.5,
                    GREY,
                    t,
                    69.4,
                );
            }
        }
    }
    // Freed blocks leave stroke-only gold outlines where the holes are.
    for (i, step) in steps.iter().enumerate() {
        if let SegOp::Free(_) = step.op {
            let start = step.start.unwrap() / kb_per_cell;
            let a = start as usize;
            let b = a + (step.len / kb_per_cell) as usize;
            let mut hole = vec![span_rect(
                FG_X0, FG_Y, FG_CELL, FG_H, a, b, HILITE, 0.0, HILITE,
            )];
            for it in &mut hole {
                it.set_stroke_width(0.03);
            }
            let t = FG_OP_T[i] + 0.3;
            content.push(life_seq(&mut hole, t, 69.4, 0.35, 0.5, 0.0, vec![]));
        }
    }
    // Hole size labels + the failing request.
    caption(
        content,
        "128 KB free — yet one 80 KB stretch is impossible",
        0.32,
        dvec3(0.6, -3.6, 0.0),
        HILITE,
        62.6,
        66.8,
    );
    caption(
        content,
        "compaction would fix it — copying GBs mid-flight: too slow",
        0.3,
        dvec3(0.0, -3.6, 0.0),
        GREY,
        67.2,
        69.4,
    );

    // Beat 4 — plan B: paging.
    caption(
        content,
        "plan B — cut both spaces into fixed pages",
        0.45,
        dvec3(0.0, 3.4, 0.0),
        TEXT_COL,
        70.0,
        78.0,
    );
    caption(
        content,
        "4 KB each — one unit of accounting everywhere",
        0.3,
        dvec3(-2.8, 2.85, 0.0),
        GREY,
        70.4,
        74.2,
    );
    let mut v_label = text_vitems("one process's virtual space", 0.26, dvec3(-2.8, 2.42, 0.0));
    v_label.set_fill_color(VIRT);
    content.push(life_seq(&mut v_label, 70.8, 78.0, 0.4, 0.5, 0.0, vec![]));
    let mut v_cells = cell_row(PG_N, PG_X0, PG_VY, PG_CELL, PG_VH, VIRT);
    content.push(life_seq(&mut v_cells, 71.0, 78.0, 0.4, 0.5, 0.02, vec![]));
    for i in 0..PG_N {
        let mut n = text_vitems(
            &i.to_string(),
            0.2,
            dvec3(span_center_x(PG_X0, PG_CELL, i, i + 1), PG_VY, 0.0),
        );
        n.set_fill_color(TEXT_COL);
        content.push(life_seq(
            &mut n,
            71.3 + i as f64 * 0.05,
            78.0,
            0.25,
            0.3,
            0.0,
            vec![],
        ));
    }
    let mut p_label = text_vitems("physical memory", 0.26, dvec3(-2.8, -2.55, 0.0));
    p_label.set_fill_color(PHYS);
    content.push(life_seq(&mut p_label, 70.9, 78.0, 0.4, 0.5, 0.0, vec![]));
    let mut f_cells = cell_row(PM_N, PG_X0, PM_Y, PM_CELL, PM_H, PHYS);
    content.push(life_seq(&mut f_cells, 71.0, 78.0, 0.4, 0.5, 0.012, vec![]));
    for i in 0..PM_N {
        let mut n = text_vitems(
            &i.to_string(),
            0.15,
            dvec3(span_center_x(PG_X0, PM_CELL, i, i + 1), PM_Y, 0.0),
        );
        n.set_fill_color(GREY);
        content.push(life_seq(
            &mut n,
            71.3 + i as f64 * 0.03,
            78.0,
            0.25,
            0.3,
            0.0,
            vec![],
        ));
    }
    let page_map = [(0usize, 2u32), (1, 7), (2, 1), (3, 9)];
    let mut pt = vec![panel(dvec3(PT_X, PT_CY, 0.0), PT_W, PT_H, GREY)];
    content.push(life_seq(&mut pt, 71.6, 78.0, 0.5, 0.6, 0.0, vec![]));
    let mut pt_head = text_vitems(
        "page table",
        0.28,
        dvec3(PT_X, PT_CY + PT_H / 2.0 - 0.42, 0.0),
    );
    pt_head.set_fill_color(TEXT_COL);
    content.push(life_seq(&mut pt_head, 71.8, 78.0, 0.4, 0.5, 0.0, vec![]));
    for (i, (page, frame)) in page_map.into_iter().enumerate() {
        let row_y = PT_ROW0_Y - i as f64 * PT_ROW_DY;
        let mut row = text_vitems(
            &format!("page {page} → frame {frame}"),
            0.22,
            dvec3(PT_X, row_y, 0.0),
        );
        row.set_fill_color(TEXT_COL);
        content.push(life_seq(
            &mut row,
            72.2 + i as f64 * 0.15,
            78.0,
            0.3,
            0.4,
            0.0,
            vec![],
        ));
        let color = if i % 2 == 0 { VIRT } else { manim::BLUE_B };
        let mut a1 = arrow_items(
            dvec3(
                span_center_x(PG_X0, PG_CELL, page, page + 1),
                PG_VY - PG_VH / 2.0,
                0.0,
            ),
            dvec3(PT_X - 1.42, row_y, 0.0),
            color,
        );
        content.push(life_seq(
            &mut a1,
            73.4 + i as f64 * 0.2,
            78.0,
            0.3,
            0.4,
            0.0,
            vec![],
        ));
        let mut a2 = arrow_items(
            dvec3(PT_X + 1.42, row_y, 0.0),
            dvec3(
                span_center_x(PG_X0, PM_CELL, frame as usize, frame as usize + 1),
                PM_Y + PM_H / 2.0,
                0.0,
            ),
            PHYS,
        );
        content.push(life_seq(
            &mut a2,
            73.6 + i as f64 * 0.2,
            78.0,
            0.3,
            0.4,
            0.0,
            vec![],
        ));
    }
    caption(
        content,
        "any free frame works — out of order is fine",
        0.3,
        dvec3(-2.8, -0.4, 0.0),
        TEXT_COL,
        74.8,
        77.8,
    );

    // Beat 4b — one translation, up close.
    caption(
        content,
        "one translation, up close",
        0.45,
        dvec3(0.0, 3.4, 0.0),
        TEXT_COL,
        78.4,
        92.6,
    );
    let mut va = chip(
        &format!("virtual  {}", hex(DEMO_VA, 8)),
        0.4,
        dvec3(0.0, 2.3, 0.0),
        VIRT,
    );
    content.push(life_seq(&mut va, 78.8, 92.6, 0.4, 0.5, 0.0, vec![]));
    for side in [-1.0, 1.0] {
        let mut a = arrow_items(
            dvec3(side * 1.1, 1.95, 0.0),
            dvec3(side * 2.9, 1.55, 0.0),
            GREY_DIM,
        );
        content.push(life_seq(&mut a, 79.9, 92.6, 0.3, 0.4, 0.0, vec![]));
    }
    let mut vpn = chip(
        &format!("page number  {}", hex(DEMO_VPN, 5)),
        0.3,
        dvec3(-3.0, 1.2, 0.0),
        VIRT,
    );
    content.push(life_seq(&mut vpn, 80.2, 92.6, 0.35, 0.5, 0.0, vec![]));
    let mut off = chip(
        &format!("offset  {}", hex(DEMO_OFFSET, 3)),
        0.3,
        dvec3(3.0, 1.2, 0.0),
        GREY,
    );
    content.push(life_seq(&mut off, 80.2, 92.6, 0.35, 0.5, 0.0, vec![]));
    caption(
        content,
        "high bits pick a row of the table — low bits pass through",
        0.28,
        dvec3(0.0, 0.45, 0.0),
        GREY,
        81.2,
        84.0,
    );
    let mut tw = vec![panel(dvec3(-0.2, -1.1, 0.0), 6.8, 2.1, GREY)];
    content.push(life_seq(&mut tw, 83.6, 92.6, 0.5, 0.6, 0.0, vec![]));
    let mut hit_row = vec![{
        let mut r = VItem::from(Rectangle::new(6.5, 0.44));
        r.move_to(dvec3(-0.2, -1.1, 0.0));
        r.set_fill_color(HILITE).set_fill_opacity(0.14);
        r.set_stroke_color(HILITE).set_stroke_width(0.03);
        r
    }];
    content.push(life_seq(&mut hit_row, 85.4, 92.6, 0.35, 0.5, 0.0, vec![]));
    for (i, (page, frame)) in DEMO_TABLE.into_iter().enumerate() {
        let hit = page == DEMO_VPN;
        let mut row = text_left(
            &format!("{} → frame {}", hex(page, 5), hex(frame, 3)),
            0.26,
            -3.0,
            -0.55 - i as f64 * 0.55,
        );
        row.set_fill_color(if hit { HILITE } else { TEXT_COL });
        content.push(life_seq(
            &mut row,
            84.0 + i as f64 * 0.2,
            92.6,
            0.3,
            0.4,
            0.0,
            vec![],
        ));
    }
    let mut tw_head = text_vitems("page table", 0.24, dvec3(-0.2, -0.25, 0.0));
    tw_head.set_fill_color(GREY);
    content.push(life_seq(&mut tw_head, 83.8, 92.6, 0.35, 0.4, 0.0, vec![]));
    caption(
        content,
        "the row for page 0x00401 holds frame 0x007",
        0.28,
        dvec3(-0.2, -2.6, 0.0),
        GREY,
        85.8,
        88.0,
    );
    let mut pa = chip(
        &format!("physical  {}", hex(DEMO_PA, 6)),
        0.34,
        dvec3(5.15, -1.1, 0.0),
        PHYS,
    );
    content.push(life_seq(&mut pa, 87.8, 92.6, 0.4, 0.5, 0.0, vec![]));
    caption(
        content,
        "swap the frame in, keep the offset — the address is reborn",
        0.3,
        dvec3(-0.2, -2.6, 0.0),
        TEXT_COL,
        88.4,
        90.6,
    );
    caption(
        content,
        "the MMU does this on every access — and caches results in the TLB",
        0.28,
        dvec3(0.0, -3.35, 0.0),
        GREY,
        90.9,
        92.6,
    );

    // Beat 5 — the bill: flat tables are unaffordable.
    caption(
        content,
        "the price of the lie",
        0.45,
        dvec3(0.0, 3.4, 0.0),
        TEXT_COL,
        93.0,
        116.0,
    );
    let mut mp = vec![panel(dvec3(-3.4, 1.35, 0.0), 6.2, 2.9, GREY)];
    content.push(life_seq(&mut mp, 93.4, 116.0, 0.5, 0.6, 0.0, vec![]));
    for (i, (text, in_t)) in [
        ("32-bit space: 2^32 bytes", 93.9),
        ("4 KB pages → 2^32 / 2^12 = 2^20 rows", 94.7),
        (
            &format!("4 bytes each → {} MiB of table", FLAT_BYTES / 1024 / 1024),
            95.5,
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let mut line = text_left(text, 0.28, -6.2, 2.15 - i as f64 * 0.66);
        line.set_fill_color(TEXT_COL);
        content.push(life_seq(&mut line, in_t, 116.0, 0.4, 0.5, 0.0, vec![]));
    }
    caption(
        content,
        "4 MiB of bookkeeping per process — to maybe use 100 KB",
        0.3,
        dvec3(-2.2, -0.7, 0.0),
        HILITE,
        96.8,
        100.6,
    );
    // The multi-level tree.
    let mut tree = Vec::new();
    for k in 0..4usize {
        let mut s = VItem::from(Rectangle::new(0.85, 0.5));
        s.move_to(dvec3(3.0 + k as f64 * 1.05, 2.3, 0.0));
        if k < 2 {
            s.set_fill_color(VIRT).set_fill_opacity(0.3);
            s.set_stroke_color(VIRT).set_stroke_width(0.02);
        } else {
            s.set_fill_color(CHIP_FILL).set_fill_opacity(0.9);
            s.set_stroke_color(GREY_DIM).set_stroke_width(0.016);
        }
        tree.push(s);
    }
    for k in 0..3usize {
        let mut s = VItem::from(Rectangle::new(0.85, 0.5));
        s.move_to(dvec3(3.2 + k as f64 * 1.05, 1.3, 0.0));
        s.set_fill_color(VIRT).set_fill_opacity(0.3);
        s.set_stroke_color(VIRT).set_stroke_width(0.02);
        tree.push(s);
    }
    tree.push(VItem::from(Line::new(
        dvec3(3.42, 2.03, 0.0),
        dvec3(4.0, 1.58, 0.0),
    )));
    tree.push(VItem::from(Line::new(
        dvec3(4.68, 2.03, 0.0),
        dvec3(4.25, 1.58, 0.0),
    )));
    for it in tree.iter_mut().skip(4).take(2) {
        it.set_stroke_color(GREY).set_stroke_width(0.016);
    }
    content.push(life_seq(&mut tree, 97.2, 116.0, 0.4, 0.5, 0.06, vec![]));
    let mut l1 = text_vitems("level 1", 0.2, dvec3(2.25, 2.3, 0.0));
    l1.set_fill_color(GREY);
    content.push(life_seq(&mut l1, 97.6, 116.0, 0.3, 0.4, 0.0, vec![]));
    let mut l2 = text_vitems("level 2", 0.2, dvec3(2.25, 1.3, 0.0));
    l2.set_fill_color(GREY);
    content.push(life_seq(&mut l2, 98.6, 116.0, 0.3, 0.4, 0.0, vec![]));
    caption(
        content,
        "a tree — unused subtrees don't exist",
        0.28,
        dvec3(4.0, 0.55, 0.0),
        TEXT_COL,
        99.4,
        103.0,
    );
    for (i, (text, in_t)) in [
        (
            format!(
                "x86-64: {} levels × {} bits + 12 = {}-bit addresses",
                LEVELS, BITS_PER_LEVEL, VA_BITS
            ),
            103.6,
        ),
        (
            format!(
                "each table: {} × 8 B = {} KB — exactly one page",
                ENTRIES_PER_TABLE,
                TABLE_BYTES / 1024
            ),
            104.8,
        ),
        (
            "the TLB caches translations — the walk is the slow path".to_string(),
            106.0,
        ),
        (
            "large pages (2 MB, 1 GB) skip levels entirely".to_string(),
            107.2,
        ),
    ]
    .into_iter()
    .enumerate()
    {
        caption(
            content,
            &text,
            0.26,
            dvec3(-3.4, -1.55 - i as f64 * 0.56, 0.0),
            GREY,
            in_t,
            112.6,
        );
    }
    caption(
        content,
        "and your 100 bytes still occupy a whole page — half of it waste",
        0.3,
        dvec3(-3.4, -1.9, 0.0),
        GREY,
        113.2,
        116.0,
    );
    caption(
        content,
        "paging won — segments survive only as a compatibility fossil",
        0.34,
        dvec3(0.0, 0.3, 0.0),
        TEXT_COL,
        116.4,
        118.8,
    );
}

// MARK: Act 2 — the buddy allocator

/// Life of one buddy block (bar rectangle) across the demo.
struct BlkLife {
    key: (u32, u8),
    in_t: f64,
    out_t: f64,
    /// (time, requester) when the block was granted.
    grant: Option<(f64, char)>,
    /// time when a granted block was handed back.
    freed: Option<f64>,
    /// the block was split or merged away — lookups must skip it.
    consumed: bool,
}

/// Life of one free-list chip.
struct ChipLife {
    key: (u32, u8),
    in_t: f64,
    out_t: f64,
}

fn bb_cx(start: u32, order: u8) -> f64 {
    BB_X0 + (start as f64 + (1u32 << order) as f64 / 2.0) * BB_CELL
}

fn fl_row_y(order: u8) -> f64 {
    FL_TOP - (B_MAX_ORDER - order) as f64 * FL_DY
}

fn proc_color(id: char) -> AlphaColor<Srgb> {
    match id {
        'A' => VIRT,
        'B' => PROC2,
        'C' => SLAB,
        _ => GREY,
    }
}

fn act2(content: &mut AnimStack) {
    // Beat 0 — who owns the physical pages?
    caption(
        content,
        "who hands out the physical pages?",
        0.45,
        dvec3(0.0, 3.4, 0.0),
        TEXT_COL,
        A2_IN,
        134.2,
    );
    caption(
        content,
        "page tables keep promising frames — somebody must find them",
        0.3,
        dvec3(0.0, 2.8, 0.0),
        GREY,
        121.2,
        125.2,
    );
    for (i, (text, x)) in [
        ("a page fault: 1 frame", -4.4),
        ("an 8 KB buffer: 2 frames", 0.0),
        ("a 2 MB huge page: 512 frames", 4.4),
    ]
    .into_iter()
    .enumerate()
    {
        let mut c = chip(text, 0.28, dvec3(x, 0.9, 0.0), GREY);
        content.push(life_seq(
            &mut c,
            121.8 + i as f64 * 0.5,
            127.8,
            0.35,
            0.5,
            0.0,
            vec![],
        ));
    }
    caption(
        content,
        "requests of every size, all day long — and the holes must not come back",
        0.3,
        dvec3(0.0, -0.9, 0.0),
        HILITE,
        124.2,
        128.2,
    );
    caption(
        content,
        "the kernel's answer: the buddy allocator",
        0.42,
        dvec3(0.0, -2.3, 0.0),
        TEXT_COL,
        128.8,
        132.4,
    );
    caption(
        content,
        "one per memory zone · blocks of 2^k pages · k up to 10",
        0.28,
        dvec3(0.0, -3.2, 0.0),
        GREY,
        130.2,
        134.2,
    );

    // Beat 1 — the arena and its free lists.
    caption(
        content,
        "the wholesale layer — the buddy allocator",
        0.45,
        dvec3(0.0, 3.4, 0.0),
        TEXT_COL,
        134.6,
        A2_OUT,
    );
    let mut cells = cell_row(BB_N, BB_X0, BB_Y, BB_CELL, BB_H, GREY_DIM);
    content.push(life_seq(&mut cells, 135.0, 203.8, 0.4, 0.6, 0.01, vec![]));
    let mut bar_label = text_vitems(
        "physical memory · 16 pages (64 KB)",
        0.26,
        dvec3(-0.7, 1.85, 0.0),
    );
    bar_label.set_fill_color(GREY);
    content.push(life_seq(
        &mut bar_label,
        135.2,
        203.8,
        0.4,
        0.6,
        0.0,
        vec![],
    ));
    let mut fl_panel = vec![panel(dvec3(5.6, 0.35, 0.0), 2.9, 4.0, GREY)];
    content.push(life_seq(&mut fl_panel, 135.8, 203.8, 0.5, 0.6, 0.0, vec![]));
    let mut fl_head = text_vitems("free lists", 0.26, dvec3(5.6, 2.6, 0.0));
    fl_head.set_fill_color(TEXT_COL);
    content.push(life_seq(&mut fl_head, 136.0, 203.8, 0.4, 0.6, 0.0, vec![]));
    for o in 0..=B_MAX_ORDER {
        let mut lbl = text_left(&format!("order {o}"), 0.22, 4.45, fl_row_y(o));
        lbl.set_fill_color(GREY);
        content.push(life_seq(
            &mut lbl,
            136.2 + (B_MAX_ORDER - o) as f64 * 0.1,
            203.8,
            0.3,
            0.4,
            0.0,
            vec![],
        ));
    }
    caption(
        content,
        "free blocks are always a power of two — each size gets its own list",
        0.3,
        dvec3(-1.0, -0.1, 0.0),
        GREY,
        137.0,
        144.4,
    );

    // Walk the simulation and collect block / chip lifetimes.
    let plan = buddy_sim(&B_OPS);
    let mut blocks: Vec<BlkLife> = vec![BlkLife {
        key: (0, B_MAX_ORDER),
        in_t: 135.5,
        out_t: 203.8,
        grant: None,
        freed: None,
        consumed: false,
    }];
    let mut chips: Vec<ChipLife> = vec![ChipLife {
        key: (0, B_MAX_ORDER),
        in_t: 136.4,
        out_t: 203.8,
    }];
    let mut live: std::collections::HashMap<char, (u32, u8)> = std::collections::HashMap::new();
    let mut log_texts: Vec<String> = Vec::new();
    for (k, (op, events)) in plan.iter().enumerate() {
        let t0 = B_OP_T[k];
        let mut t = t0 + 1.0;
        // Free bookkeeping runs first: the merge events below consume the
        // freed block from `blocks`.
        if let BOp::Free(id) = op {
            let (s, o) = live[id];
            live.remove(id);
            let span = if o == 0 {
                format!("page {s}")
            } else {
                format!("pages {}-{}", s, s + (1 << o) - 1)
            };
            log_texts.push(format!("free {id}: {span}  (order {o})"));
            let i = blocks
                .iter()
                .position(|b| b.key == (s, o) && b.grant.map(|(_, n)| n) == Some(*id))
                .unwrap();
            blocks[i].freed = Some(t0 + 0.6);
        }
        for e in events {
            match *e {
                BEvent::Split { start, order } => {
                    let i = blocks
                        .iter()
                        .position(|b| !b.consumed && b.key == (start, order))
                        .unwrap();
                    blocks[i].out_t = t;
                    blocks[i].consumed = true;
                    let parent_key = blocks[i].key;
                    // Taking a block off its list kills its chip.
                    if let Some(ci) = chips.iter().position(|c| c.key == parent_key) {
                        chips[ci].out_t = t;
                        chips.remove(ci);
                    }
                    blocks.push(BlkLife {
                        key: (start, order - 1),
                        in_t: t + 0.05,
                        out_t: 203.8,
                        grant: None,
                        freed: None,
                        consumed: false,
                    });
                    blocks.push(BlkLife {
                        key: (start + (1 << (order - 1)), order - 1),
                        in_t: t + 0.05,
                        out_t: 203.8,
                        grant: None,
                        freed: None,
                        consumed: false,
                    });
                }
                BEvent::ListAdd { start, order } => {
                    chips.push(ChipLife {
                        key: (start, order),
                        in_t: t + 0.35,
                        out_t: 203.8,
                    });
                }
                BEvent::Grant { start, order } => {
                    let i = blocks
                        .iter()
                        .position(|b| !b.consumed && b.key == (start, order))
                        .unwrap();
                    blocks[i].grant = Some((
                        t,
                        match op {
                            BOp::Alloc(id, _) => *id,
                            _ => unreachable!(),
                        },
                    ));
                    if let Some(ci) = chips.iter().position(|c| c.key == (start, order)) {
                        chips[ci].out_t = t;
                        chips.remove(ci);
                    }
                }
                BEvent::Merge { start, order } => {
                    let buddy = start ^ (1 << order);
                    for key in [(start, order), (buddy, order)] {
                        let i = blocks
                            .iter()
                            .position(|b| !b.consumed && b.key == key)
                            .unwrap();
                        blocks[i].out_t = t;
                        blocks[i].consumed = true;
                        if let Some(ci) = chips.iter().position(|c| c.key == key) {
                            chips[ci].out_t = t;
                            chips.remove(ci);
                        }
                    }
                    blocks.push(BlkLife {
                        key: (start, order + 1),
                        in_t: t + 0.05,
                        out_t: 203.8,
                        grant: None,
                        freed: None,
                        consumed: false,
                    });
                }
            }
            t += B_EV_STEP;
        }
        match op {
            BOp::Alloc(id, order) => {
                let s = match events.iter().find(|e| matches!(e, BEvent::Grant { .. })) {
                    Some(BEvent::Grant { start, .. }) => *start,
                    _ => 0,
                };
                live.insert(*id, (s, *order));
                let span = if *order == 0 {
                    format!("page {s}")
                } else {
                    format!("pages {}-{}", s, s + (1 << order) - 1)
                };
                let noun = if *order == 0 {
                    "1 page".to_string()
                } else {
                    format!("{} pages", 1 << order)
                };
                log_texts.push(format!("{id} = alloc({noun})  →  {span}"));
            }
            BOp::Free(_) => {}
        }
    }
    // The dim log lines brighten as each op runs.
    for (k, text) in log_texts.iter().enumerate() {
        let mut line = text_left(text, 0.22, -6.95, -1.9 - k as f64 * 0.38);
        line.set_fill_color(GREY_DIM);
        content.push(life_seq(
            &mut line,
            145.0,
            203.8,
            0.4,
            0.6,
            0.0,
            vec![ev(B_OP_T[k] + 0.05, 0.3, move |items: &mut Vec<VItem>| {
                items.set_fill_color(TEXT_COL);
            })],
        ));
    }
    // Bar blocks.
    for b in &blocks {
        let (start, order) = b.key;
        let a = start as usize;
        let e = a + (1usize << order);
        let mut rect = vec![span_rect(
            BB_X0,
            BB_Y,
            BB_CELL,
            BB_H,
            a,
            e,
            FREE_FILL,
            0.9,
            FREE_STROKE,
        )];
        let mut events = Vec::new();
        if let Some((gt, id)) = b.grant {
            let c = proc_color(id);
            events.push(ev(gt, 0.35, move |items: &mut Vec<VItem>| {
                items.set_fill_color(c).set_fill_opacity(0.34);
                items.set_stroke_color(c).set_stroke_width(0.028);
            }));
        }
        if let Some(ft) = b.freed {
            events.push(ev(ft, 0.35, move |items: &mut Vec<VItem>| {
                items.set_fill_color(FREE_FILL).set_fill_opacity(0.9);
                items.set_stroke_color(FREE_STROKE).set_stroke_width(0.02);
            }));
        }
        content.push(life_seq(&mut rect, b.in_t, b.out_t, 0.3, 0.3, 0.0, events));
        // Labels: range → (name) → range again.
        let pos = dvec3(bb_cx(start, order), BB_Y, 0.0);
        let range = if order == 0 {
            format!("{start}")
        } else {
            format!("{}-{}", start, start + (1 << order) - 1)
        };
        if let Some((gt, id)) = b.grant {
            let mut rl = text_vitems(&range, 0.26, pos);
            rl.set_fill_color(GREY);
            content.push(life_seq(
                &mut rl,
                b.in_t + 0.15,
                gt + 0.05,
                0.25,
                0.25,
                0.0,
                vec![],
            ));
            let mut nl = text_vitems(&id.to_string(), 0.36, pos);
            nl.set_fill_color(proc_color(id));
            let nl_out = b.freed.unwrap_or(b.out_t);
            content.push(life_seq(&mut nl, gt + 0.2, nl_out, 0.2, 0.2, 0.0, vec![]));
            if let Some(ft) = b.freed {
                let mut rl2 = text_vitems(&range, 0.26, pos);
                rl2.set_fill_color(GREY);
                content.push(life_seq(
                    &mut rl2,
                    ft + 0.25,
                    b.out_t,
                    0.25,
                    0.3,
                    0.0,
                    vec![],
                ));
            }
        } else {
            let mut rl = text_vitems(&range, 0.26, pos);
            rl.set_fill_color(GREY);
            content.push(life_seq(
                &mut rl,
                b.in_t + 0.15,
                b.out_t,
                0.25,
                0.3,
                0.0,
                vec![],
            ));
        }
    }
    // Free-list chips.
    for c in &chips {
        let (start, order) = c.key;
        let text = if order == 0 {
            format!("{start}")
        } else {
            format!("{}-{}", start, start + (1 << order) - 1)
        };
        let mut chip = chip(
            &text,
            0.24,
            dvec3(FL_CHIP_X, fl_row_y(order), 0.0),
            FREE_STROKE,
        );
        content.push(life_seq(&mut chip, c.in_t, c.out_t, 0.3, 0.3, 0.0, vec![]));
    }
    // Final pulse: the arena back as one block.
    let mut pulse = vec![{
        let mut r = VItem::from(Rectangle::new(BB_N as f64 * BB_CELL, BB_H));
        r.move_to(dvec3(BB_X0 + BB_N as f64 * BB_CELL / 2.0, BB_Y, 0.0));
        r.set_fill_opacity(0.0);
        r.set_stroke_color(HILITE).set_stroke_width(0.035);
        r
    }];
    content.push(life_seq(&mut pulse, 198.0, 203.8, 0.5, 0.5, 0.0, vec![]));

    // Beat 2 — allocation cascades.
    caption(
        content,
        "no order-1 block? take a bigger one and keep halving",
        0.3,
        dvec3(-1.0, -0.55, 0.0),
        TEXT_COL,
        146.4,
        154.4,
    );
    caption(
        content,
        "the waiting halves park on their size's list",
        0.28,
        dvec3(-1.0, -0.55, 0.0),
        GREY,
        154.8,
        159.2,
    );
    caption(
        content,
        "down again — one more split for a single page",
        0.28,
        dvec3(-1.0, -0.55, 0.0),
        GREY,
        161.0,
        165.6,
    );
    caption(
        content,
        "C fits exactly: no split — straight off the list",
        0.28,
        dvec3(-1.0, -0.55, 0.0),
        GREY,
        167.2,
        171.2,
    );

    // Beat 3 — who is the buddy?
    caption(
        content,
        "who is a block's buddy?",
        0.34,
        dvec3(-1.0, -0.55, 0.0),
        TEXT_COL,
        171.8,
        176.4,
    );
    let mut formula = text_vitems(
        "buddy of p, size 2^k   =   p ^ 2^k",
        0.34,
        dvec3(-1.0, -1.35, 0.0),
    );
    formula.set_fill_color(HILITE);
    content.push(life_seq(&mut formula, 172.4, 177.4, 0.4, 0.4, 0.0, vec![]));
    let mut ex = text_vitems(
        "page 2 (0b0010) ^ 1 page = page 3 (0b0011)",
        0.3,
        dvec3(-1.0, -1.95, 0.0),
    );
    ex.set_fill_color(TEXT_COL);
    content.push(life_seq(&mut ex, 173.2, 177.4, 0.4, 0.4, 0.0, vec![]));

    // Beat 4 — free cascades.
    caption(
        content,
        "free → check the buddy → merge if free",
        0.3,
        dvec3(-1.0, -0.55, 0.0),
        TEXT_COL,
        180.8,
        183.0,
    );
    caption(
        content,
        "buddy 0-1 is still in use (A) — park here",
        0.28,
        dvec3(-1.0, -0.55, 0.0),
        GREY,
        183.2,
        185.4,
    );
    caption(
        content,
        "free A → merge with 2-3 → buddy 4-7 (C) in use: stop",
        0.28,
        dvec3(-1.0, -0.55, 0.0),
        GREY,
        186.6,
        190.6,
    );
    caption(
        content,
        "free C → two merges, all the way back up",
        0.3,
        dvec3(-1.0, -0.55, 0.0),
        TEXT_COL,
        195.6,
        200.4,
    );
    caption(
        content,
        "external fragmentation cannot survive here",
        0.34,
        dvec3(-1.0, -1.2, 0.0),
        HILITE,
        198.8,
        203.4,
    );

    // Beat 5 — strengths and the limit.
    for (i, (text, x)) in [
        ("one piece: 4 KB up to 4 MB", -3.4),
        ("per-zone, per-cpu lists: fast", 3.4),
    ]
    .into_iter()
    .enumerate()
    {
        let mut c = chip(text, 0.28, dvec3(x, 1.1, 0.0), PHYS);
        content.push(life_seq(
            &mut c,
            204.6 + i as f64 * 0.3,
            209.6,
            0.35,
            0.5,
            0.0,
            vec![],
        ));
    }
    caption(
        content,
        "but the smallest wholesale unit is a whole page",
        0.3,
        dvec3(0.0, -0.3, 0.0),
        GREY,
        205.6,
        209.6,
    );
    let mut need = chip("need: a 192 B dentry", 0.3, dvec3(-2.7, -1.5, 0.0), SLAB);
    content.push(life_seq(&mut need, 210.0, 214.4, 0.35, 0.5, 0.0, vec![]));
    let mut get = chip("get: a whole 4 KB page", 0.3, dvec3(2.7, -1.5, 0.0), PHYS);
    content.push(life_seq(&mut get, 210.2, 214.4, 0.35, 0.5, 0.0, vec![]));
    caption(
        content,
        "95% of it wasted — the kernel needs a retail layer",
        0.32,
        dvec3(0.0, -2.8, 0.0),
        BAD,
        211.0,
        215.0,
    );
    caption(
        content,
        "enter: slab",
        0.44,
        dvec3(0.0, -3.6, 0.0),
        TEXT_COL,
        214.6,
        217.4,
    );
}

// MARK: Act 3 — slab

/// Slot visuals: filled slot = rect + object letter.
fn slot_items(cx: f64, cy: f64, letter: &str, color: AlphaColor<Srgb>) -> Vec<VItem> {
    let mut r = VItem::from(Rectangle::new(SL_SLOT - 0.08, SL_SLOT - 0.08));
    r.move_to(dvec3(cx, cy, 0.0));
    r.set_fill_color(color).set_fill_opacity(0.5);
    r.set_stroke_color(color).set_stroke_width(0.022);
    let mut g = text_vitems(letter, 0.3, dvec3(cx, cy, 0.0));
    g.set_fill_color(TEXT_COL);
    let mut v = vec![r];
    v.extend(g);
    v
}

fn act3(content: &mut AnimStack) {
    // Beat 1 — the kernel breeds small objects.
    caption(
        content,
        "the retail layer — slab",
        0.45,
        dvec3(0.0, 3.4, 0.0),
        TEXT_COL,
        A3_IN,
        264.2,
    );
    for (i, name) in [
        "task\\_struct",
        "mm\\_struct",
        "vm\\_area\\_struct",
        "dentry",
        "filp",
    ]
    .into_iter()
    .enumerate()
    {
        let mut c = chip(name, 0.28, dvec3(-5.6 + i as f64 * 2.8, 2.0, 0.0), SLAB);
        content.push(life_seq(
            &mut c,
            220.4 + i as f64 * 0.22,
            226.4,
            0.35,
            0.5,
            0.0,
            vec![],
        ));
    }
    caption(
        content,
        "the kernel constantly breeds small objects — forks, opens, lookups",
        0.3,
        dvec3(0.0, 0.9, 0.0),
        GREY,
        221.8,
        226.4,
    );
    caption(
        content,
        "each lives briefly — and a twin is born microseconds later",
        0.3,
        dvec3(0.0, 0.9, 0.0),
        GREY,
        226.8,
        230.4,
    );
    caption(
        content,
        "buddy would burn a page per object — slab cuts one page into equal slots",
        0.3,
        dvec3(0.0, -0.5, 0.0),
        HILITE,
        228.6,
        232.6,
    );

    // Beat 2 — one cache, three slabs.
    caption(
        content,
        "cache: dentry · objects of 192 B",
        0.32,
        dvec3(0.0, 2.5, 0.0),
        SLAB,
        233.0,
        264.0,
    );
    for (si, &sx) in SL_X.iter().enumerate() {
        let mut box_p = vec![panel(dvec3(sx, SL_Y, 0.0), SL_W, SL_H, GREY)];
        content.push(life_seq(
            &mut box_p,
            233.6 + si as f64 * 0.2,
            264.0,
            0.4,
            0.5,
            0.0,
            vec![],
        ));
        let mut lbl = text_vitems(&format!("slab {si}"), 0.22, dvec3(sx, 0.6, 0.0));
        lbl.set_fill_color(GREY);
        content.push(life_seq(
            &mut lbl,
            233.8 + si as f64 * 0.2,
            264.0,
            0.3,
            0.4,
            0.0,
            vec![],
        ));
        for j in 0..SLOTS_PER_SLAB {
            let cx = sx + (j as f64 - 1.5) * SL_SLOT;
            let mut empty = vec![{
                let mut r = VItem::from(Rectangle::new(SL_SLOT - 0.08, SL_SLOT - 0.08));
                r.move_to(dvec3(cx, SL_Y, 0.0));
                r.set_fill_color(CHIP_FILL).set_fill_opacity(0.92);
                r.set_stroke_color(GREY_DIM).set_stroke_width(0.016);
                r
            }];
            content.push(life_seq(
                &mut empty,
                233.6 + si as f64 * 0.2 + j as f64 * 0.06,
                264.0,
                0.3,
                0.4,
                0.0,
                vec![],
            ));
        }
    }
    // Status chips (slab 0 flips from full to partial when b dies).
    let mut st0a = text_vitems("full", 0.24, dvec3(SL_X[0], 0.1, 0.0));
    st0a.set_fill_color(GREY);
    content.push(life_seq(&mut st0a, 235.6, 245.9, 0.3, 0.3, 0.0, vec![]));
    let mut st0b = text_vitems("partial", 0.24, dvec3(SL_X[0], 0.1, 0.0));
    st0b.set_fill_color(SLAB);
    content.push(life_seq(&mut st0b, 246.3, 264.0, 0.3, 0.4, 0.0, vec![]));
    let mut st1 = text_vitems("partial", 0.24, dvec3(SL_X[1], 0.1, 0.0));
    st1.set_fill_color(SLAB);
    content.push(life_seq(&mut st1, 235.9, 264.0, 0.3, 0.4, 0.0, vec![]));
    let mut st2 = text_vitems("empty", 0.24, dvec3(SL_X[2], 0.1, 0.0));
    st2.set_fill_color(GREY);
    content.push(life_seq(&mut st2, 236.2, 264.0, 0.3, 0.4, 0.0, vec![]));
    caption(
        content,
        "one cache per object type — a slab is one page cut into equal slots",
        0.3,
        dvec3(0.0, -0.85, 0.0),
        GREY,
        236.8,
        240.6,
    );
    caption(
        content,
        "allocation takes the first free slot of the first partial slab — no searching",
        0.3,
        dvec3(0.0, -0.85, 0.0),
        GREY,
        241.0,
        244.8,
    );

    // Beat 3 — the alloc/free cycle, driven by the simulation.
    let steps = slab_sim(&S_OPS);
    // Wall-clock time of each cycle step, by index in S_OPS (6-10).
    let step_t = |i: usize| match i {
        6 => 245.8,  // free b
        8 => 248.8,  // alloc g — reuses b's slot
        7 => 252.6,  // free e
        9 => 255.2,  // alloc h — reuses e's slot
        10 => 258.2, // alloc i — fresh slot
        _ => 0.0,
    };
    for (i, step) in steps.iter().enumerate() {
        let SOp::Alloc(id) = step.op else { continue };
        let cx = SL_X[step.slab] + (step.slot as f64 - 1.5) * SL_SLOT;
        let out_t = match id {
            'b' => 245.9,
            'e' => 252.7,
            _ => 264.0,
        };
        let (in_t, events) = if i < 6 {
            (234.2 + i as f64 * 0.18, Vec::new())
        } else {
            let base = step_t(i);
            let mut evs = Vec::new();
            if step.reused {
                evs.push(ev(base + 0.9, 0.3, move |items: &mut Vec<VItem>| {
                    items[0].set_stroke_color(HILITE).set_stroke_width(0.032);
                }));
                evs.push(ev(base + 2.0, 0.3, move |items: &mut Vec<VItem>| {
                    items[0].set_stroke_color(SLAB).set_stroke_width(0.022);
                }));
            }
            (base + 0.5, evs)
        };
        let mut slot = slot_items(cx, SL_Y, &id.to_string(), SLAB);
        content.push(life_seq(&mut slot, in_t, out_t, 0.3, 0.35, 0.0, events));
    }
    for (k, (text, color, t)) in [
        ("free b  →  slab 0, slot 1", GREY, 245.8),
        ("alloc g  →  slab 0, slot 1 — still warm", HILITE, 248.8),
        ("free e  →  slab 1, slot 0", GREY, 252.6),
        ("alloc h  →  slab 1, slot 0 — still warm", HILITE, 255.2),
        ("alloc i  →  slab 1, slot 2 — fresh", GREY, 258.2),
    ]
    .into_iter()
    .enumerate()
    {
        log_line(
            content,
            text,
            0.24,
            -6.95,
            -1.7 - k as f64 * 0.5,
            color,
            t,
            264.0,
        );
    }
    caption(
        content,
        "just-freed slots are handed back first — still warm",
        0.3,
        dvec3(0.0, -0.85, 0.0),
        HILITE,
        250.6,
        254.6,
    );
    caption(
        content,
        "no buddy round-trip for a 192 B object — one page serves about 20 of them",
        0.28,
        dvec3(0.0, -0.85, 0.0),
        GREY,
        258.8,
        263.6,
    );

    // Beat 4 — generic sizes: kmalloc.
    caption(
        content,
        "generic objects: kmalloc(n)",
        0.45,
        dvec3(0.0, 3.4, 0.0),
        TEXT_COL,
        264.4,
        276.2,
    );
    let sizes = [
        "32", "64", "96", "128", "192", "256", "512", "1K", "2K", "4K", "8K",
    ];
    let ruler: Vec<Vec<VItem>> = sizes
        .iter()
        .enumerate()
        .map(|(i, s)| chip(s, 0.26, dvec3(-5.5 + i as f64 * 1.1, 1.2, 0.0), GREY))
        .collect();
    for (i, mut c) in ruler.into_iter().enumerate() {
        content.push(life_seq(
            &mut c,
            264.8 + i as f64 * 0.04,
            276.2,
            0.3,
            0.5,
            0.0,
            vec![ev(267.6, 0.4, move |items: &mut Vec<VItem>| {
                if i == 3 {
                    items[0].set_stroke_color(HILITE).set_stroke_width(0.032);
                }
            })],
        ));
    }
    let mut req = chip("kmalloc(100)", 0.3, dvec3(0.5, -0.15, 0.0), VIRT);
    content.push(life_seq(&mut req, 266.4, 276.2, 0.35, 0.5, 0.0, vec![]));
    let mut req_arrow = arrow_items(dvec3(-0.85, -0.2, 0.0), dvec3(-2.05, 0.72, 0.0), GREY_DIM);
    content.push(life_seq(
        &mut req_arrow,
        267.4,
        276.2,
        0.3,
        0.4,
        0.0,
        vec![],
    ));
    caption(
        content,
        "round up to the next size class — O(1), a few bytes of waste",
        0.3,
        dvec3(0.0, -1.1, 0.0),
        TEXT_COL,
        268.4,
        272.4,
    );
    caption(
        content,
        "the size classes run on the same slab machinery",
        0.28,
        dvec3(0.0, -1.9, 0.0),
        GREY,
        270.6,
        274.6,
    );

    // Beat 5 — it is running right now.
    caption(
        content,
        "on a running Linux box",
        0.45,
        dvec3(0.0, 3.4, 0.0),
        TEXT_COL,
        276.6,
        292.4,
    );
    let mut rp = vec![panel(dvec3(0.0, -0.55, 0.0), 9.8, 3.4, GREY)];
    content.push(life_seq(&mut rp, 277.0, 286.0, 0.5, 0.6, 0.0, vec![]));
    let mut rh = text_vitems(
        "caches you would find in /proc/slabinfo",
        0.26,
        dvec3(0.0, 0.85, 0.0),
    );
    rh.set_fill_color(GREY);
    content.push(life_seq(&mut rh, 277.2, 286.0, 0.4, 0.5, 0.0, vec![]));
    for (i, name) in [
        "task\\_struct",
        "mm\\_struct",
        "vm\\_area\\_struct",
        "dentry",
        "ext4\\_inode\\_cache",
        "kmalloc-1k",
    ]
    .into_iter()
    .enumerate()
    {
        let col = i % 2;
        let row = i / 2;
        let mut n = text_vitems(
            name,
            0.28,
            dvec3(-2.4 + col as f64 * 4.8, 0.25 - row as f64 * 0.62, 0.0),
        );
        n.set_fill_color(TEXT_COL);
        content.push(life_seq(
            &mut n,
            277.8 + i as f64 * 0.15,
            286.0,
            0.3,
            0.4,
            0.0,
            vec![],
        ));
    }
    caption(
        content,
        "watch them breathe: run slabtop",
        0.3,
        dvec3(0.0, -2.9, 0.0),
        SLAB,
        279.0,
        283.2,
    );
    caption(
        content,
        "the idea dates to 1994 (SunOS) — Linux still runs on it (SLUB)",
        0.28,
        dvec3(0.0, -3.55, 0.0),
        GREY,
        281.2,
        285.8,
    );
    caption(
        content,
        "so far: pages translate · buddy supplies · slab recycles",
        0.36,
        dvec3(0.0, -0.6, 0.0),
        TEXT_COL,
        286.6,
        292.6,
    );
}

// MARK: Act 4 — the whole journey

fn act4(content: &mut AnimStack) {
    caption(
        content,
        "the whole journey — trace malloc(100)",
        0.45,
        dvec3(0.0, 3.4, 0.0),
        TEXT_COL,
        A4_IN,
        320.2,
    );
    let stations = [
        ("program", GREY),
        ("libc malloc", VIRT),
        ("brk · mmap", VIRT),
        ("page tables", PHYS),
        ("buddy", PHYS),
        ("physical RAM", PHYS),
    ];
    // Even spacing from the real chip widths, arrows between the edges.
    let widths: Vec<f64> = stations
        .iter()
        .map(|(name, _)| text_width(name, 0.24) + 0.3)
        .collect();
    let gap = 0.6;
    let total: f64 = widths.iter().sum::<f64>() + gap * (stations.len() - 1) as f64;
    let mut cursor = -total / 2.0;
    let mut centers = Vec::new();
    let mut edges = Vec::new();
    for w in &widths {
        centers.push(cursor + w / 2.0);
        cursor += w;
        edges.push(cursor);
        cursor += gap;
    }
    for (i, (name, color)) in stations.into_iter().enumerate() {
        let mut c = chip(name, 0.24, dvec3(centers[i], 2.1, 0.0), color);
        content.push(life_seq(
            &mut c,
            297.4 + i as f64 * 0.15,
            320.2,
            0.3,
            0.5,
            0.0,
            vec![],
        ));
    }
    for (i, &edge) in edges.iter().enumerate().take(stations.len() - 1) {
        let mut a = arrow_items(
            dvec3(edge + 0.08, 2.1, 0.0),
            dvec3(edge + gap - 0.08, 2.1, 0.0),
            GREY_DIM,
        );
        content.push(life_seq(
            &mut a,
            298.8 + i as f64 * 0.08,
            320.2,
            0.25,
            0.4,
            0.0,
            vec![],
        ));
    }

    // Step 1 — libc serves it from its own stockpile.
    caption(
        content,
        "1 · libc cuts 112 B from its own stockpile — the kernel never hears",
        0.3,
        dvec3(0.0, -0.55, 0.0),
        TEXT_COL,
        300.0,
        303.8,
    );
    let mut heap = cell_row(8, -2.2, -1.75, 0.55, 0.6, GREY_DIM);
    content.push(life_seq(&mut heap, 300.4, 304.0, 0.3, 0.4, 0.015, vec![]));
    let mut cut = vec![span_rect(-2.2, -1.75, 0.55, 0.6, 2, 4, GOOD, 0.4, GOOD)];
    content.push(life_seq(&mut cut, 300.9, 304.0, 0.3, 0.4, 0.0, vec![]));
    let mut heap_lbl = text_vitems(
        "the libc heap — already mapped",
        0.22,
        dvec3(0.0, -2.55, 0.0),
    );
    heap_lbl.set_fill_color(GREY);
    content.push(life_seq(&mut heap_lbl, 300.6, 304.0, 0.3, 0.4, 0.0, vec![]));

    // Step 2 — grow the address space on paper.
    caption(
        content,
        "2 · stockpile low? the kernel mmaps a fresh region — on paper only",
        0.3,
        dvec3(0.0, -0.55, 0.0),
        TEXT_COL,
        304.4,
        308.8,
    );
    let mut region = vec![region_rect(-2.9, 2.9, -1.75, 0.6, VIRT, VIRT)];
    content.push(life_seq(&mut region, 304.9, 308.8, 0.4, 0.5, 0.0, vec![]));
    let mut region_lbl = text_vitems("2 MiB of new virtual space", 0.22, dvec3(0.0, -2.55, 0.0));
    region_lbl.set_fill_color(VIRT);
    content.push(life_seq(
        &mut region_lbl,
        305.2,
        308.8,
        0.3,
        0.4,
        0.0,
        vec![],
    ));
    caption(
        content,
        "page-table rows reserved — but no RAM behind them yet",
        0.26,
        dvec3(0.0, -3.3, 0.0),
        GREY,
        306.4,
        308.8,
    );

    // Step 3 — the first write triggers the real delivery.
    caption(
        content,
        "3 · the first write → page fault → the real delivery",
        0.32,
        dvec3(0.0, -0.55, 0.0),
        HILITE,
        309.2,
        315.6,
    );
    let mut fault = chip("page fault!", 0.3, dvec3(-4.7, -1.55, 0.0), BAD);
    content.push(life_seq(&mut fault, 309.8, 315.8, 0.35, 0.5, 0.0, vec![]));
    let mut frame = chip("buddy: 1 frame", 0.3, dvec3(-4.7, -2.7, 0.0), PHYS);
    content.push(life_seq(&mut frame, 311.4, 315.8, 0.35, 0.5, 0.0, vec![]));
    let mut fault_arrow = arrow_items(dvec3(-4.7, -1.95, 0.0), dvec3(-4.7, -2.3, 0.0), GREY_DIM);
    content.push(life_seq(
        &mut fault_arrow,
        310.9,
        315.8,
        0.25,
        0.4,
        0.0,
        vec![],
    ));
    let mut table = chip("page table: row filled ✓", 0.3, dvec3(0.4, -2.7, 0.0), GOOD);
    content.push(life_seq(&mut table, 312.8, 315.8, 0.35, 0.5, 0.0, vec![]));
    let mut table_arrow = arrow_items(dvec3(-3.3, -2.7, 0.0), dvec3(-1.5, -2.7, 0.0), GREY_DIM);
    content.push(life_seq(
        &mut table_arrow,
        312.3,
        315.8,
        0.25,
        0.4,
        0.0,
        vec![],
    ));
    let mut land = chip("the byte lands in RAM", 0.3, dvec3(4.9, -2.7, 0.0), PHYS);
    content.push(life_seq(&mut land, 314.0, 315.8, 0.35, 0.5, 0.0, vec![]));
    let mut land_arrow = arrow_items(dvec3(2.35, -2.7, 0.0), dvec3(3.5, -2.7, 0.0), GREY_DIM);
    content.push(life_seq(
        &mut land_arrow,
        313.5,
        315.8,
        0.25,
        0.4,
        0.0,
        vec![],
    ));

    // The demand-paging punchline.
    caption(
        content,
        "malloc returned before any RAM existed",
        0.32,
        dvec3(0.0, 0.5, 0.0),
        HILITE,
        316.2,
        320.6,
    );
    caption(
        content,
        "demand paging: pay on first touch",
        0.28,
        dvec3(0.0, -0.1, 0.0),
        GREY,
        317.4,
        320.6,
    );

    // Recap cards.
    for (i, (title, l1, color)) in [
        ("pages", "translate · isolate", VIRT),
        ("buddy", "wholesale frames", PHYS),
        ("slab", "retail objects", SLAB),
    ]
    .into_iter()
    .enumerate()
    {
        let x = -4.7 + i as f64 * 4.7;
        let mut card = vec![panel(dvec3(x, 1.1, 0.0), 3.9, 2.4, color)];
        content.push(life_seq(
            &mut card,
            321.0 + i as f64 * 0.25,
            333.0,
            0.4,
            0.5,
            0.0,
            vec![],
        ));
        let mut t = text_vitems(title, 0.36, dvec3(x, 1.75, 0.0));
        t.set_fill_color(color);
        content.push(life_seq(
            &mut t,
            321.2 + i as f64 * 0.25,
            333.0,
            0.35,
            0.5,
            0.0,
            vec![],
        ));
        let mut a = text_vitems(l1, 0.26, dvec3(x, 1.15, 0.0));
        a.set_fill_color(TEXT_COL);
        content.push(life_seq(
            &mut a,
            321.4 + i as f64 * 0.25,
            333.0,
            0.35,
            0.5,
            0.0,
            vec![],
        ));
    }
    for (i, (text, _color)) in [
        ("4 KB accounting", VIRT),
        ("split and merge · no holes", PHYS),
        ("recycled warm", SLAB),
    ]
    .into_iter()
    .enumerate()
    {
        let x = -4.7 + i as f64 * 4.7;
        let mut a = text_vitems(text, 0.22, dvec3(x, 0.55, 0.0));
        a.set_fill_color(GREY);
        content.push(life_seq(
            &mut a,
            321.8 + i as f64 * 0.25,
            333.0,
            0.3,
            0.4,
            0.0,
            vec![],
        ));
    }
    // Outro.
    caption(
        content,
        "so next time malloc hands you a pointer…",
        0.36,
        dvec3(0.0, -1.6, 0.0),
        TEXT_COL,
        333.8,
        OUT_T,
    );
    caption(
        content,
        "…remember the supply chain that stayed out of your way.",
        0.36,
        dvec3(0.0, -2.6, 0.0),
        HILITE,
        336.2,
        OUT_T,
    );
}

// MARK: Scene

#[scene]
#[output(dir = "./output/linux_mem_alloc")]
fn linux_mem_alloc(r: &mut RanimScene) {
    let mut content = AnimStack::new();
    act0(&mut content);
    act1(&mut content);
    act2(&mut content);
    act3(&mut content);
    act4(&mut content);

    r.play(CameraFrame::default().show().with_duration(TOTAL));
    r.play(content);

    // Captures: hook TOC, translation, buddy cascade (preview), slab reuse,
    // journey delivery.
    r.insert_time_mark(17.8, TimeMark::Capture("hook.png".to_string()));
    r.insert_time_mark(86.5, TimeMark::Capture("paging.png".to_string()));
    r.insert_time_mark(201.0, TimeMark::Capture("preview.png".to_string()));
    r.insert_time_mark(251.5, TimeMark::Capture("slab.png".to_string()));
    r.insert_time_mark(314.6, TimeMark::Capture("journey.png".to_string()));
}
