//! BPE tokenizer — how large language models turn text into tokens.
//!
//! A ~2.5 minute explainer in four acts, aimed at people with basic computer
//! literacy and no prior knowledge of the algorithm:
//!
//! 1. **Hook** — an LLM never sees words or letters; it sees tokens.
//! 2. **The two extremes** — character-level splits (tiny vocabulary, huge
//!    sequences) vs word-level splits (short sequences, unwieldy vocabulary,
//!    unknown words) lead to the subword middle ground.
//! 3. **Training** — the Byte Pair Encoding loop, shown on a tiny corpus:
//!    split every word into characters, count adjacent pairs, merge the most
//!    frequent pair, repeat. Four merges are shown in full (`lo`, `low`,
//!    `es`, `est`); the loop is then scaled up to a real vocabulary size.
//! 4. **Encoding & why it matters** — the learned merges are applied to the
//!    unseen word `slowest` (`s|low|est`), followed by real-world scale and
//!    the quirks that follow from tokenization.
//!
//! Every number on screen (pair counts, merge order, encoding stages) is
//! produced by a real BPE implementation in this file — nothing is
//! hand-animated. Structure follows `examples/agents/convolution_kernels`:
//! one scene, a shared `AnimStack` of long-lived `AnimSequence`s, each group
//! owning its full life cycle (fade in, optional recolor morphs, fade out).

use std::ops::Range;

use ranim::{
    anims::{
        fading::{FadeOut, FadingAnim},
        morph::MorphAnim,
    },
    color::{AlphaColor, Srgb, palettes::manim, rgb8},
    core::animation::eval::pure::Pure,
    glam::{DVec3, dvec3},
    items::vitem::{
        VItem,
        geometry::{Circle, Line, Rectangle},
        text::TextItem,
    },
    prelude::*,
    utils::rate_functions::smooth,
};

// MARK: BPE algorithm

type Pair = (String, String);

/// The tiny training corpus: (word, occurrences). Chosen so that the four
/// merges are each a unique maximum and read as meaningful pieces:
/// `l+o` → `lo`, `lo+w` → `low`, `e+s` → `es`, `es+t` → `est`.
const CORPUS: [(&str, u32); 7] = [
    ("low", 4),
    ("lower", 2),
    ("lowest", 2),
    ("loan", 1),
    ("lies", 1),
    ("newest", 3),
    ("widest", 1),
];

/// How many merges to show in full.
const N_MERGES: usize = 4;

/// One snapshot of the training state *before* a merge: the current token
/// rows (tokens per corpus word, with word multiplicity), the candidate
/// pair counts sorted by frequency, and the winning pair.
struct Stage {
    rows: Vec<(Vec<String>, u32)>,
    cands: Vec<(Pair, u32)>,
    winner: Pair,
}

fn split_chars(word: &str) -> Vec<String> {
    word.chars().map(|c| c.to_string()).collect()
}

/// Count all adjacent token pairs across the corpus, weighted by word
/// multiplicity. Ties keep first-encountered order (stable sort).
fn count_pairs(rows: &[(Vec<String>, u32)]) -> Vec<(Pair, u32)> {
    let mut order: Vec<Pair> = Vec::new();
    let mut counts: Vec<u32> = Vec::new();
    for (tokens, mult) in rows {
        for w in tokens.windows(2) {
            let pair = (w[0].clone(), w[1].clone());
            if let Some(i) = order.iter().position(|p| *p == pair) {
                counts[i] += mult;
            } else {
                order.push(pair);
                counts.push(*mult);
            }
        }
    }
    let mut idx: Vec<usize> = (0..order.len()).collect();
    idx.sort_by(|&a, &b| counts[b].cmp(&counts[a]));
    idx.into_iter()
        .map(|i| (order[i].clone(), counts[i]))
        .collect()
}

/// Merge every (left-to-right, non-overlapping) occurrence of `pair`.
fn merge_all(tokens: &mut Vec<String>, pair: &Pair) {
    let mut i = 0;
    while i + 1 < tokens.len() {
        if tokens[i] == pair.0 && tokens[i + 1] == pair.1 {
            tokens[i] = format!("{}{}", pair.0, pair.1);
            tokens.remove(i + 1);
        }
        i += 1;
    }
}

/// Run the training loop: `N_MERGES` snapshots before each merge, plus the
/// final rows after the last merge.
fn training() -> (Vec<Stage>, Vec<(Vec<String>, u32)>) {
    let mut rows: Vec<(Vec<String>, u32)> =
        CORPUS.iter().map(|(w, c)| (split_chars(w), *c)).collect();
    let mut stages = Vec::new();
    for _ in 0..N_MERGES {
        let cands = count_pairs(&rows);
        let winner = cands[0].0.clone();
        let mut merged = rows.clone();
        for (tokens, _) in &mut merged {
            merge_all(tokens, &winner);
        }
        stages.push(Stage {
            rows,
            cands,
            winner,
        });
        rows = merged;
    }
    (stages, rows)
}

/// Encode a word by applying the learned merges in order, returning the
/// token list after each rule (including the initial character split).
fn encode(word: &str, merges: &[Pair]) -> Vec<Vec<String>> {
    let mut tokens = split_chars(word);
    let mut stages = vec![tokens.clone()];
    for pair in merges {
        merge_all(&mut tokens, pair);
        stages.push(tokens.clone());
    }
    stages
}

// MARK: Colors & layout

const TEXT_COL: AlphaColor<Srgb> = AlphaColor::WHITE;
const CHIP_FILL: AlphaColor<Srgb> = rgb8(0x26, 0x26, 0x2e);
const PANEL_FILL: AlphaColor<Srgb> = rgb8(0x1c, 0x1c, 0x24);
const PLAIN_STROKE: AlphaColor<Srgb> = manim::GREY_B;
const BAD_COL: AlphaColor<Srgb> = manim::RED_C;
const GOOD_COL: AlphaColor<Srgb> = manim::GREEN_C;
const HILITE: AlphaColor<Srgb> = manim::GOLD_D;
/// One accent color per learned merge: `lo`, `low`, `es`, `est`.
const MERGE_COL: [AlphaColor<Srgb>; N_MERGES] = [
    manim::BLUE_C,
    manim::YELLOW_C,
    manim::TEAL_C,
    manim::GREEN_C,
];

/// Stroke color for a token: learned tokens wear their merge's accent.
fn token_color(tok: &str) -> AlphaColor<Srgb> {
    match tok {
        "lo" => MERGE_COL[0],
        "low" => MERGE_COL[1],
        "es" => MERGE_COL[2],
        "est" => MERGE_COL[3],
        _ => PLAIN_STROKE,
    }
}

/// Layout width of a text at `em` size, in world units.
fn text_width(text: &str, em: f64) -> f64 {
    TextItem::new(text, em).inline_length_em() * em
}

/// Width of one token cell / gap between adjacent chips / baseline drop.
const CELL: f64 = 0.42;
const CHIP_GAP: f64 = 0.08;
const BASELINE_DROP: f64 = 0.3;

/// A box + glyphs for one token occupying `span` cells of `cell` width,
/// centered at `center`.
///
/// Glyphs sit by baseline (`TextItem` items have their baseline at the
/// local origin), which keeps rows of single letters optically aligned; all
/// corpus/encode content is descender-free. Word-level chips pass
/// `center_glyphs = true` to center the ink box instead.
fn chip_sized(
    text: &str,
    span: usize,
    center: DVec3,
    stroke: AlphaColor<Srgb>,
    em: f64,
    cell: f64,
    center_glyphs: bool,
) -> Vec<VItem> {
    let mut box_item = VItem::from(Rectangle::new(
        span as f64 * cell - CHIP_GAP,
        cell - CHIP_GAP,
    ));
    box_item.move_to(center);
    box_item.set_fill_color(CHIP_FILL).set_fill_opacity(0.92);
    box_item.set_stroke_color(stroke).set_stroke_width(0.022);
    let mut glyphs = Vec::<VItem>::from(TextItem::new(text, em));
    if center_glyphs {
        glyphs.move_to(center);
    } else {
        let w = text_width(text, em);
        glyphs.shift(dvec3(
            center.x - w / 2.0,
            center.y - em * BASELINE_DROP,
            0.0,
        ));
    }
    glyphs.set_fill_color(TEXT_COL);
    let mut items = vec![box_item];
    items.extend(glyphs);
    items
}

/// One corpus row: a chip per token, laid out left to right from `x_left`,
/// vertically centered at `y`. Returns the items plus, per token, the range
/// of items it occupies (box first, then glyphs).
fn chip_row(tokens: &[String], x_left: f64, y: f64, em: f64) -> (Vec<VItem>, Vec<Range<usize>>) {
    let mut items = Vec::new();
    let mut ranges = Vec::new();
    let mut x = x_left;
    for tok in tokens {
        let span = tok.chars().count().max(1);
        let center = dvec3(x + span as f64 * CELL / 2.0, y, 0.0);
        let chip = chip_sized(tok, span, center, token_color(tok), em, CELL, false);
        ranges.push(items.len()..items.len() + chip.len());
        items.extend(chip);
        x += span as f64 * CELL;
    }
    (items, ranges)
}

/// Single-line text as glyphs, ink-centered at `pos`.
fn text_vitems(text: &str, em: f64, pos: DVec3) -> Vec<VItem> {
    let mut vitems = Vec::<VItem>::from(TextItem::new(text, em));
    vitems.move_anchor_to(AabbPoint::CENTER, pos);
    vitems
}

/// A variable-width word chip (ink width + padding).
fn word_chip(text: &str, em: f64, center: DVec3, stroke: AlphaColor<Srgb>) -> Vec<VItem> {
    let w = text_width(text, em) + 0.3;
    let mut box_item = VItem::from(Rectangle::new(w, em + 0.42));
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

// MARK: Timeline

/// Act boundaries (seconds). Each act fades its content out at `*_END`.
const ACT0_END: f64 = 16.6;
const ACT1_IN: f64 = 17.6;
const ACT1_END: f64 = 42.0;
const ACT2_IN: f64 = 43.0;
const ACT2_END: f64 = 107.0;
const ACT3_IN: f64 = 108.6;
const ACT3_END: f64 = 127.4;
const ACT4_IN: f64 = 129.0;
const OUT_T: f64 = 155.4;
const TOTAL: f64 = 157.0;

/// Act 2 corpus panel.
const ROW_DY: f64 = 0.62;
const ROW_TOP: f64 = 1.86;
const ROW_X: f64 = -6.75;
const BADGE_X: f64 = -3.75;

/// Act 2 merge step: each of the four merges occupies `STEP` seconds.
const MERGE_T0: f64 = 50.4;
const STEP: f64 = 13.0;
const CHART_IN: f64 = 0.9;
const WINNER_AT: f64 = 3.6;
const WIN_CAP_AT: f64 = 4.1;
const MERGE_CAP_AT: f64 = 5.9;
const OLD_OUT: f64 = 6.2;
const NEW_IN: f64 = 6.55;
const VOCAB_AT: f64 = 7.7;
const CHART_OUT: f64 = 10.6;

/// Act 2 count chart.
const CHART_C: DVec3 = dvec3(0.35, 0.2, 0.0);
const CHART_DY: f64 = 0.46;
const CHART_ROWS: usize = 6;
const BAR_X: f64 = 1.0;
const BAR_MAX_W: f64 = 1.45;

/// Act 2 vocabulary panel.
const VOCAB_C: DVec3 = dvec3(5.05, 0.05, 0.0);
const VOCAB_W: f64 = 3.7;
const VOCAB_H: f64 = 4.6;

/// Act 3 encoding demo.
const ENC_CELL: f64 = 0.6;
const ENC_C: DVec3 = dvec3(1.7, 0.7, 0.0);
const RULES_X: f64 = -5.3;

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

/// A group's full life: fade in (per-item `lag`), optional recolor events,
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

/// Recolor the box of the chips forming the winning pair (the box is the
/// first item of each token's range; glyphs are left untouched).
fn hilite_pair_paint(
    winner: Pair,
    tokens: Vec<String>,
    ranges: Vec<Range<usize>>,
    color: AlphaColor<Srgb>,
) -> impl Fn(&mut Vec<VItem>) {
    move |items: &mut Vec<VItem>| {
        for (i, w) in tokens.windows(2).enumerate() {
            if w[0] == winner.0 && w[1] == winner.1 {
                for r in [&ranges[i], &ranges[i + 1]] {
                    items[r.start]
                        .set_stroke_color(color)
                        .set_stroke_width(0.03);
                }
            }
        }
    }
}

// MARK: Act 0 — hook

fn act0(content: &mut AnimStack) {
    // Title and subtitle make way for the question card at 11.4.
    let mut title = text_vitems("How LLMs Read Text", 0.85, dvec3(0.0, 1.0, 0.0));
    content.push(life_seq(&mut title, 0.2, 11.2, 0.9, 0.6, 0.0, vec![]));
    caption(
        content,
        "tokenization & byte pair encoding",
        0.34,
        dvec3(0.0, 0.25, 0.0),
        manim::GREY_B,
        0.8,
        11.2,
    );

    // The sentence, then one chip per token, each chip wearing its own color.
    let words = ["Tokenization", "is", "the", "first", "step"];
    let in_t = 3.4;
    let em = 0.4;
    let gap = 0.24;
    let widths: Vec<f64> = words.iter().map(|w| text_width(w, em) + 0.3).collect();
    let total: f64 = widths.iter().sum::<f64>() + gap * (words.len() - 1) as f64;
    let mut plain = Vec::new();
    let mut centers = Vec::new();
    let mut x_cur = -total / 2.0;
    for (i, w) in words.iter().enumerate() {
        let c = dvec3(x_cur + widths[i] / 2.0, -1.1, 0.0);
        plain.append(&mut text_vitems(w, em, c));
        centers.push(c);
        x_cur += widths[i] + gap;
    }
    plain.set_fill_color(TEXT_COL);
    content.push(life_seq(
        &mut plain,
        in_t,
        in_t + 1.6,
        0.6,
        0.4,
        0.0,
        vec![],
    ));

    for (i, w) in words.iter().enumerate() {
        let color = MERGE_COL[i % MERGE_COL.len()];
        let mut chip = word_chip(w, em, centers[i], PLAIN_STROKE);
        content.push(life_seq(
            &mut chip,
            in_t + 1.6,
            ACT0_END,
            0.3,
            0.6,
            0.0,
            vec![ev(
                5.2 + i as f64 * 0.35,
                0.35,
                move |items: &mut Vec<VItem>| {
                    items.set_stroke_color(color);
                },
            )],
        ));
    }

    caption(
        content,
        "the model only ever sees tokens",
        0.3,
        dvec3(0.0, -2.0, 0.0),
        manim::GREY_B,
        6.9,
        8.9,
    );
    caption(
        content,
        "each token is one entry in a fixed vocabulary",
        0.3,
        dvec3(0.0, -2.0, 0.0),
        manim::GREY_B,
        9.2,
        11.2,
    );

    caption(
        content,
        "How are the pieces chosen?",
        0.55,
        dvec3(0.0, 0.7, 0.0),
        TEXT_COL,
        11.6,
        ACT0_END,
    );
    caption(
        content,
        "Byte Pair Encoding (BPE)",
        0.5,
        dvec3(0.0, -0.2, 0.0),
        manim::YELLOW_C,
        13.4,
        ACT0_END,
    );
}

// MARK: Act 1 — two extremes

const A1_EM: f64 = 0.42;

/// `the newest score` as per-character chips (mode A) or word chips (mode B).
fn act1_sentence(mode_a: bool) -> Vec<VItem> {
    let words: [&str; 3] = ["the", "newest", "score"];
    let mut items = Vec::new();
    let cell = 0.5;
    let total = if mode_a {
        14.0 * cell + 2.0 * 0.35
    } else {
        words
            .iter()
            .map(|w| text_width(w, A1_EM) + 0.3)
            .sum::<f64>()
            + 2.0 * 0.4
    };
    let mut x = -total / 2.0;
    if mode_a {
        for w in words {
            for ch in w.chars() {
                let c = dvec3(x + cell / 2.0, 1.8, 0.0);
                items.extend(chip_sized(
                    &ch.to_string(),
                    1,
                    c,
                    PLAIN_STROKE,
                    0.34,
                    cell,
                    false,
                ));
                x += cell;
            }
            x += 0.35;
        }
    } else {
        for w in words {
            let cw = text_width(w, A1_EM) + 0.3;
            let c = dvec3(x + cw / 2.0, 1.8, 0.0);
            items.extend(word_chip(w, A1_EM, c, PLAIN_STROKE));
            x += cw + 0.4;
        }
    }
    items
}

fn act1(content: &mut AnimStack) {
    // Mode A: characters.
    caption(
        content,
        "Option A — one token per character",
        0.38,
        dvec3(0.0, 2.8, 0.0),
        TEXT_COL,
        ACT1_IN,
        24.8,
    );
    let mut chars = act1_sentence(true);
    content.push(life_seq(
        &mut chars,
        ACT1_IN + 0.6,
        24.8,
        0.5,
        0.5,
        0.012,
        vec![],
    ));
    caption(
        content,
        "vocabulary: \\~100 characters  ✓",
        0.3,
        dvec3(-4.6, 0.2, 0.0),
        GOOD_COL,
        19.0,
        24.8,
    );
    caption(
        content,
        "sequence: 14 tokens for one sentence  ✗",
        0.3,
        dvec3(4.4, 0.2, 0.0),
        BAD_COL,
        19.4,
        24.8,
    );
    // A long sequence: the strip of tokens runs off the screen.
    let mut strip = Vec::new();
    for i in 0..44 {
        let mut sq = VItem::from(Rectangle::new(0.14, 0.14));
        sq.move_to(dvec3(-6.9 + i as f64 * 0.32, -1.7, 0.0));
        sq.set_fill_color(manim::GREY_E).set_fill_opacity(0.8);
        sq.set_stroke_opacity(0.0);
        strip.push(sq);
    }
    content.push(life_seq(&mut strip, 21.8, 24.8, 0.4, 0.4, 0.008, vec![]));
    caption(
        content,
        "long sequences are slow and expensive",
        0.3,
        dvec3(0.0, -2.5, 0.0),
        BAD_COL,
        22.6,
        24.8,
    );

    // Mode B: words.
    caption(
        content,
        "Option B — one token per word",
        0.38,
        dvec3(0.0, 2.8, 0.0),
        TEXT_COL,
        25.6,
        34.6,
    );
    let mut words_items = act1_sentence(false);
    content.push(life_seq(
        &mut words_items,
        26.2,
        34.6,
        0.5,
        0.5,
        0.05,
        vec![],
    ));
    caption(
        content,
        "vocabulary: every word ever seen  ✗",
        0.3,
        dvec3(-4.6, 0.2, 0.0),
        BAD_COL,
        27.2,
        34.6,
    );
    caption(
        content,
        "sequence: 3 tokens  ✓",
        0.3,
        dvec3(4.4, 0.2, 0.0),
        GOOD_COL,
        27.0,
        34.6,
    );

    // The word vocabulary grows line by line inside a panel, then breaks on
    // unknown words.
    let mut voc_panel = VItem::from(Rectangle::new(4.2, 2.9));
    voc_panel.move_to(dvec3(4.6, -1.75, 0.0));
    voc_panel.set_fill_color(PANEL_FILL).set_fill_opacity(0.6);
    voc_panel
        .set_stroke_color(manim::GREY_B)
        .set_stroke_width(0.02);
    content.push(life_seq(
        &mut vec![voc_panel],
        27.4,
        34.6,
        0.5,
        0.5,
        0.0,
        vec![],
    ));
    let voc_lines = ["the", "newest", "score", "scores", "scoring", "scored", "…"];
    for (i, line) in voc_lines.iter().enumerate() {
        let mut items = text_vitems(line, 0.28, dvec3(4.6, -0.62 - i as f64 * 0.3, 0.0));
        items.set_fill_color(TEXT_COL);
        content.push(life_seq(
            &mut items,
            27.8 + i as f64 * 0.22,
            34.6,
            0.3,
            0.3,
            0.0,
            vec![],
        ));
    }
    caption(
        content,
        "500,000+ entries  ✗",
        0.3,
        dvec3(4.6, -0.62 - 7.0 * 0.3 - 0.23, 0.0),
        BAD_COL,
        30.0,
        34.6,
    );
    // An unseen word cannot be encoded.
    let mut unknown = word_chip("newset", 0.38, dvec3(-4.6, -1.6, 0.0), BAD_COL);
    content.push(life_seq(&mut unknown, 31.4, 34.6, 0.5, 0.5, 0.0, vec![]));
    caption(
        content,
        "✗ unknown — cannot be encoded",
        0.3,
        dvec3(-4.6, -2.35, 0.0),
        BAD_COL,
        31.9,
        34.6,
    );

    // Verdict: the subword middle ground, as a spectrum.
    let mut axis = Vec::new();
    let mut line = VItem::from(Line::new(dvec3(-4.5, -0.6, 0.0), dvec3(4.5, -0.6, 0.0)));
    line.set_stroke_color(manim::GREY_B).set_stroke_width(0.03);
    axis.push(line);
    let mut l_lab = text_vitems("characters", 0.32, dvec3(-4.5, -1.15, 0.0));
    l_lab.set_fill_color(manim::GREY_B);
    axis.append(&mut l_lab);
    let mut r_lab = text_vitems("words", 0.32, dvec3(4.5, -1.15, 0.0));
    r_lab.set_fill_color(manim::GREY_B);
    axis.append(&mut r_lab);
    content.push(life_seq(&mut axis, 35.2, ACT1_END, 0.6, 0.6, 0.0, vec![]));

    // The cursor slides from the character end to the subword middle, then
    // leaves with the act.
    let mut dot = VItem::from(Circle::new(0.09));
    dot.move_to(dvec3(-4.5, -0.6, 0.0));
    dot.set_fill_color(TEXT_COL).set_fill_opacity(1.0);
    dot.set_stroke_opacity(0.0);
    let mut seq = AnimSequence::new();
    seq.forward_to(35.2);
    seq.push(dot.fade_in().with_duration(0.5));
    seq.hold_to(36.4);
    let base = dot.clone();
    seq.push(
        Pure::new(move |alpha: f64| {
            let mut d = base.clone();
            d.move_to(dvec3(-4.5 + 4.5 * alpha, -0.6, 0.0));
            d
        })
        .with_duration(2.6)
        .with_rate_func(smooth),
    );
    seq.hold_to(ACT1_END);
    let mut end_dot = dot.clone();
    end_dot.move_to(dvec3(0.0, -0.6, 0.0));
    seq.push(FadeOut::new(end_dot).with_duration(0.6));
    seq.hold_to(TOTAL);
    content.push(seq);

    caption(
        content,
        "subwords: frequent pieces stay whole, rare ones fall apart",
        0.36,
        dvec3(0.0, 0.3, 0.0),
        manim::YELLOW_C,
        39.2,
        ACT1_END,
    );
    caption(
        content,
        "BPE learns the split points from data — let's train one.",
        0.32,
        dvec3(0.0, -1.9, 0.0),
        TEXT_COL,
        40.4,
        ACT1_END,
    );
}

// MARK: Act 2 — training

fn count_chart(cands: &[(Pair, u32)]) -> (Vec<VItem>, Vec<Range<usize>>) {
    let top: Vec<&(Pair, u32)> = cands.iter().take(CHART_ROWS).collect();
    let max = top.first().map(|(_, c)| *c).unwrap_or(1) as f64;
    let mut items = Vec::new();
    let mut ranges = Vec::new();
    let n = top.len() as f64;
    for (i, (pair, count)) in top.iter().enumerate() {
        let y = CHART_C.y + (n - 1.0) * CHART_DY / 2.0 - i as f64 * CHART_DY;
        let label = format!("{}+{}", pair.0, pair.1);
        let mut g = text_vitems(&label, 0.27, dvec3(CHART_C.x - 0.35, y, 0.0));
        g.set_fill_color(TEXT_COL);
        let start = items.len();
        items.append(&mut g);
        let bw = *count as f64 / max * BAR_MAX_W;
        let mut bar = VItem::from(Rectangle::new(bw, 0.24));
        bar.move_to(dvec3(BAR_X + bw / 2.0, y, 0.0));
        bar.set_fill_color(CHIP_FILL).set_fill_opacity(0.95);
        bar.set_stroke_color(manim::GREY_B).set_stroke_width(0.015);
        items.push(bar);
        let mut c = text_vitems(&count.to_string(), 0.27, dvec3(BAR_X + bw + 0.24, y, 0.0));
        c.set_fill_color(TEXT_COL);
        items.append(&mut c);
        ranges.push(start..items.len());
    }
    (items, ranges)
}

fn vocab_line(k: usize, pair: &Pair, y: f64) -> Vec<VItem> {
    let tok = format!("{}{}", pair.0, pair.1);
    let span = tok.chars().count();
    let mut items = chip_sized(
        &tok,
        span,
        dvec3(VOCAB_C.x - 1.45, y, 0.0),
        MERGE_COL[k],
        0.22,
        0.3,
        false,
    );
    let mut from = text_vitems(
        &format!("← {} + {}", pair.0, pair.1),
        0.24,
        dvec3(VOCAB_C.x + 0.1, y, 0.0),
    );
    from.set_fill_color(TEXT_COL);
    items.append(&mut from);
    let mut id = text_vitems(
        &format!("\\#{}", 256 + k),
        0.24,
        dvec3(VOCAB_C.x + 1.42, y, 0.0),
    );
    id.set_fill_color(manim::GREY_B);
    items.append(&mut id);
    items
}

fn act2(content: &mut AnimStack) {
    let (stages, final_rows) = training();

    caption(
        content,
        "Training — learn merges from a corpus",
        0.45,
        dvec3(0.0, 3.4, 0.0),
        TEXT_COL,
        ACT2_IN,
        ACT2_END,
    );

    // Corpus rows appear as whole words with multiplicity badges, then split
    // into per-character chips. The badges stay for the whole act. The word
    // chips are plain — nothing has been learned yet.
    for (i, (word, _count)) in CORPUS.iter().enumerate() {
        let y = ROW_TOP - i as f64 * ROW_DY;
        let span = word.chars().count();
        let mut row = chip_sized(
            word,
            span,
            dvec3(ROW_X + span as f64 * CELL / 2.0, y, 0.0),
            PLAIN_STROKE,
            0.34,
            CELL,
            false,
        );
        content.push(life_seq(
            &mut row,
            43.6 + i as f64 * 0.14,
            46.6,
            0.5,
            0.35,
            0.0,
            vec![],
        ));
    }
    for (i, (_, count)) in CORPUS.iter().enumerate() {
        let y = ROW_TOP - i as f64 * ROW_DY;
        let mut badge = text_vitems(&format!("×{count}"), 0.26, dvec3(BADGE_X, y, 0.0));
        badge.set_fill_color(manim::GREY_B);
        content.push(life_seq(&mut badge, 43.8, ACT2_END, 0.5, 0.6, 0.0, vec![]));
    }
    caption(
        content,
        "step 0 — split into characters",
        0.32,
        dvec3(-4.3, 2.7, 0.0),
        manim::GREY_B,
        45.8,
        49.4,
    );

    // Per-stage rows: stage k is shown from its reveal until merge k zips.
    // stage_in[0] is the character split; stage k (k ≥ 1) appears right
    // after merge k-1; the final stage holds to the act end.
    let stage_in: Vec<f64> = (0..=N_MERGES)
        .map(|k| {
            if k == 0 {
                46.8
            } else {
                MERGE_T0 + (k as f64 - 1.0) * STEP + NEW_IN
            }
        })
        .collect();
    let stage_out: Vec<f64> = (0..=N_MERGES)
        .map(|k| {
            if k < N_MERGES {
                MERGE_T0 + k as f64 * STEP + OLD_OUT
            } else {
                ACT2_END
            }
        })
        .collect();

    for (k, stage) in stages.iter().enumerate() {
        for (i, (tokens, _)) in stage.rows.iter().enumerate() {
            let y = ROW_TOP - i as f64 * ROW_DY;
            let (mut row, ranges) = chip_row(tokens, ROW_X, y, 0.3);
            // While this stage's merge is being counted, highlight the
            // winning pair's chips.
            let events = vec![ev(
                MERGE_T0 + k as f64 * STEP + WINNER_AT,
                0.4,
                hilite_pair_paint(stage.winner.clone(), tokens.clone(), ranges, HILITE),
            )];
            content.push(life_seq(
                &mut row,
                stage_in[k],
                stage_out[k],
                0.4,
                0.35,
                0.0,
                events,
            ));
        }
    }
    for (i, (tokens, _)) in final_rows.iter().enumerate() {
        let y = ROW_TOP - i as f64 * ROW_DY;
        let (mut row, _) = chip_row(tokens, ROW_X, y, 0.3);
        content.push(life_seq(
            &mut row,
            stage_in[N_MERGES],
            ACT2_END,
            0.4,
            0.6,
            0.0,
            vec![],
        ));
    }

    // Vocabulary panel.
    let mut panel = VItem::from(Rectangle::new(VOCAB_W, VOCAB_H));
    panel.move_to(VOCAB_C);
    panel.set_fill_color(PANEL_FILL).set_fill_opacity(0.6);
    panel.set_stroke_color(manim::GREY_B).set_stroke_width(0.02);
    content.push(life_seq(
        &mut vec![panel],
        48.2,
        ACT2_END,
        0.6,
        0.6,
        0.0,
        vec![],
    ));
    let mut header = text_vitems(
        "vocabulary",
        0.3,
        dvec3(VOCAB_C.x, VOCAB_C.y + VOCAB_H / 2.0 - 0.42, 0.0),
    );
    header.set_fill_color(TEXT_COL);
    content.push(life_seq(&mut header, 48.5, ACT2_END, 0.5, 0.5, 0.0, vec![]));
    let mut bytes_line = text_vitems(
        "256 byte tokens:  a b c … 0 9 …",
        0.24,
        dvec3(VOCAB_C.x, VOCAB_C.y + 1.55, 0.0),
    );
    bytes_line.set_fill_color(manim::GREY_B);
    content.push(life_seq(
        &mut bytes_line,
        48.8,
        ACT2_END,
        0.5,
        0.5,
        0.0,
        vec![],
    ));

    caption(
        content,
        "repeat: count pairs → merge the most frequent",
        0.32,
        dvec3(-2.6, 2.7, 0.0),
        manim::GREY_B,
        49.6,
        ACT2_END,
    );

    // The four merge steps.
    for (k, stage) in stages.iter().enumerate() {
        let t0 = MERGE_T0 + k as f64 * STEP;
        let color = MERGE_COL[k];
        let merged_text = format!("{}{}", stage.winner.0, stage.winner.1);

        // Count chart; the top row (the winner) lights up at WINNER_AT.
        let (mut chart, ranges) = count_chart(&stage.cands);
        let events = vec![ev(t0 + WINNER_AT, 0.5, move |items: &mut Vec<VItem>| {
            for (i, r) in ranges.iter().enumerate() {
                if i == 0 {
                    // Each row is [label glyphs…, bar, count glyphs…].
                    let bar = &mut items[r.start + r.len() - 2];
                    bar.set_fill_color(color).set_fill_opacity(0.85);
                    bar.set_stroke_color(color).set_stroke_width(0.02);
                } else {
                    for it in &mut items[r.clone()] {
                        it.set_opacity(0.35);
                    }
                }
            }
        })];
        content.push(life_seq(
            &mut chart,
            t0 + CHART_IN,
            t0 + CHART_OUT,
            0.4,
            0.4,
            0.05,
            events,
        ));

        let cap_y = CHART_C.y - CHART_ROWS as f64 * CHART_DY / 2.0 - 0.45;
        caption(
            content,
            &format!(
                "most frequent:  {} + {}  (×{})",
                stage.winner.0, stage.winner.1, stage.cands[0].1
            ),
            0.3,
            dvec3(CHART_C.x + 0.2, cap_y, 0.0),
            HILITE,
            t0 + WIN_CAP_AT,
            t0 + MERGE_CAP_AT - 0.05,
        );
        caption(
            content,
            &format!(
                "merge:  {} + {}  →  {}",
                stage.winner.0, stage.winner.1, merged_text
            ),
            0.3,
            dvec3(CHART_C.x + 0.2, cap_y, 0.0),
            color,
            t0 + MERGE_CAP_AT,
            t0 + CHART_OUT,
        );

        // The vocabulary gains the merged token.
        let y = VOCAB_C.y + 0.95 - k as f64 * 0.52;
        let mut line = vocab_line(k, &stage.winner, y);
        content.push(life_seq(
            &mut line,
            t0 + VOCAB_AT,
            ACT2_END,
            0.5,
            0.5,
            0.0,
            vec![],
        ));
    }

    // Scale-up: the loop keeps running.
    caption(
        content,
        "…and repeat, \\~50,000 more times",
        0.3,
        dvec3(VOCAB_C.x, VOCAB_C.y - 0.75, 0.0),
        manim::GREY_B,
        103.6,
        ACT2_END,
    );
    caption(
        content,
        "GPT-2: 256 bytes + 50,000 merges = 50,257 tokens",
        0.26,
        dvec3(VOCAB_C.x, VOCAB_C.y - 1.35, 0.0),
        manim::YELLOW_C,
        104.6,
        ACT2_END,
    );
}

// MARK: Act 3 — encoding

fn encode_row(tokens: &[String], center: DVec3) -> Vec<VItem> {
    let w: f64 = tokens
        .iter()
        .map(|t| t.chars().count().max(1) as f64 * ENC_CELL)
        .sum();
    let mut items = Vec::new();
    let mut x = center.x - w / 2.0;
    for tok in tokens {
        let span = tok.chars().count().max(1);
        items.extend(chip_sized(
            tok,
            span,
            dvec3(x + span as f64 * ENC_CELL / 2.0, center.y, 0.0),
            token_color(tok),
            0.4,
            ENC_CELL,
            false,
        ));
        x += span as f64 * ENC_CELL;
    }
    items
}

fn act3(content: &mut AnimStack, merges: &[Pair]) {
    caption(
        content,
        "Encoding — apply the learned merges",
        0.45,
        dvec3(0.0, 3.4, 0.0),
        TEXT_COL,
        ACT3_IN,
        ACT3_END,
    );

    // The learned merge rules, left panel; each rule flashes when applied.
    let mut header = text_vitems("learned merges", 0.32, dvec3(RULES_X, 2.2, 0.0));
    header.set_fill_color(TEXT_COL);
    content.push(life_seq(
        &mut header,
        ACT3_IN + 0.6,
        ACT3_END,
        0.5,
        0.5,
        0.0,
        vec![],
    ));
    for (k, pair) in merges.iter().enumerate() {
        let y = 1.5 - k as f64 * 0.62;
        let tok = format!("{}{}", pair.0, pair.1);
        let mut items = chip_sized(
            &tok,
            tok.chars().count(),
            dvec3(RULES_X - 0.85, y, 0.0),
            MERGE_COL[k],
            0.28,
            0.36,
            false,
        );
        let mut from = text_vitems(
            &format!("← {} + {}", pair.0, pair.1),
            0.26,
            dvec3(RULES_X + 0.55, y, 0.0),
        );
        from.set_fill_color(TEXT_COL);
        items.append(&mut from);
        let at = 112.4 + k as f64 * 3.6;
        content.push(life_seq(
            &mut items,
            ACT3_IN + 0.8 + k as f64 * 0.2,
            ACT3_END,
            0.5,
            0.5,
            0.0,
            // Flash only the rule chip's box; glyphs keep their fill.
            vec![ev(at, 0.4, move |its: &mut Vec<VItem>| {
                its[0].set_stroke_color(HILITE).set_stroke_width(0.03);
            })],
        ));
    }

    // The unseen word, encoded stage by stage.
    let word = "slowest";
    caption(
        content,
        "a word the model has never seen:",
        0.32,
        dvec3(ENC_C.x, 2.5, 0.0),
        manim::GREY_B,
        110.4,
        111.9,
    );
    let enc_stages = encode(word, merges);
    let rule_at = |k: usize| 112.4 + k as f64 * 3.6;
    for (k, tokens) in enc_stages.iter().enumerate() {
        let in_t = if k == 0 { 110.8 } else { rule_at(k - 1) + 0.45 };
        let out_t = if k < enc_stages.len() - 1 {
            rule_at(k) + 0.1
        } else {
            ACT3_END
        };
        let mut row = encode_row(tokens, ENC_C);
        content.push(life_seq(&mut row, in_t, out_t, 0.35, 0.3, 0.0, vec![]));
        if k > 0 {
            let pair = &merges[k - 1];
            let last = k == merges.len();
            let merged = format!("{}{}", pair.0, pair.1);
            caption(
                content,
                &format!("apply rule {k}:  {} + {}  →  {}", pair.0, pair.1, merged),
                0.32,
                dvec3(ENC_C.x, 2.5, 0.0),
                MERGE_COL[k - 1],
                rule_at(k - 1),
                if last {
                    rule_at(k - 1) + 2.2
                } else {
                    rule_at(k)
                },
            );
        }
    }
    caption(
        content,
        "never in the training data — still split into meaningful pieces",
        0.36,
        dvec3(ENC_C.x, -0.7, 0.0),
        manim::YELLOW_C,
        124.8,
        ACT3_END,
    );
    caption(
        content,
        "s  ·  low  ·  est   =   3 tokens",
        0.3,
        dvec3(ENC_C.x, -1.5, 0.0),
        TEXT_COL,
        125.4,
        ACT3_END,
    );
}

// MARK: Act 4 — scale & quirks

fn act4(content: &mut AnimStack) {
    caption(
        content,
        "at scale",
        0.4,
        dvec3(0.0, 2.9, 0.0),
        TEXT_COL,
        ACT4_IN,
        133.0,
    );
    let stats = [
        ("256 bytes + \\~50,000 merges", -4.4),
        ("1 token ≈ 4 characters", 0.0),
        ("no unknown words — ever", 4.4),
    ];
    for (i, (text, x)) in stats.iter().enumerate() {
        let mut chip = word_chip(text, 0.32, dvec3(*x, 2.0, 0.0), MERGE_COL[i]);
        content.push(life_seq(
            &mut chip,
            ACT4_IN + 0.5 + i as f64 * 0.3,
            133.0,
            0.5,
            0.5,
            0.0,
            vec![],
        ));
    }

    // A row of word chips for the quirk demos.
    fn quirk_chips(
        content: &mut AnimStack,
        chips: &[(&str, AlphaColor<Srgb>)],
        y: f64,
        in_t: f64,
        out_t: f64,
    ) {
        let em = 0.42;
        let widths: Vec<f64> = chips.iter().map(|(t, _)| text_width(t, em) + 0.3).collect();
        let total: f64 = widths.iter().sum::<f64>() + 0.3 * (chips.len() - 1) as f64;
        let mut x = -total / 2.0;
        for (i, (tok, color)) in chips.iter().enumerate() {
            let c = dvec3(x + widths[i] / 2.0, y, 0.0);
            x += widths[i] + 0.3;
            let mut chip = word_chip(tok, em, c, *color);
            content.push(life_seq(&mut chip, in_t, out_t, 0.5, 0.5, 0.0, vec![]));
        }
    }

    // Quirk A: letters hide inside tokens.
    quirk_chips(
        content,
        &[
            ("str", manim::BLUE_C),
            ("aw", manim::YELLOW_C),
            ("berry", manim::TEAL_C),
        ],
        0.6,
        133.6,
        138.2,
    );
    caption(
        content,
        "strawberry",
        0.3,
        dvec3(0.0, 1.4, 0.0),
        manim::GREY_B,
        133.4,
        138.2,
    );
    caption(
        content,
        "letters hide inside the pieces — \"how many r's?\" is hard to see",
        0.3,
        dvec3(0.0, -0.4, 0.0),
        TEXT_COL,
        134.4,
        138.2,
    );

    // Quirk B: numbers get chunked.
    quirk_chips(
        content,
        &[
            ("123", manim::BLUE_C),
            ("456", manim::YELLOW_C),
            ("7", manim::TEAL_C),
        ],
        0.6,
        138.8,
        143.4,
    );
    caption(
        content,
        "1234567",
        0.3,
        dvec3(0.0, 1.4, 0.0),
        manim::GREY_B,
        138.6,
        143.4,
    );
    caption(
        content,
        "digits are chunked — the model sees groups, not single digits",
        0.3,
        dvec3(0.0, -0.4, 0.0),
        TEXT_COL,
        139.6,
        143.4,
    );

    // Quirk C: unfamiliar scripts & emoji become long byte sequences.
    let mut strip = Vec::new();
    let widths = [0.3, 0.3, 0.9, 0.5, 0.3, 0.3, 0.7, 0.3, 0.3, 0.3, 0.5, 0.3];
    let total: f64 = widths.iter().sum::<f64>() + 0.14 * (widths.len() - 1) as f64;
    let mut x = -total / 2.0;
    for (i, w) in widths.iter().enumerate() {
        let mut chip = VItem::from(Rectangle::new(*w, 0.42));
        chip.move_to(dvec3(x + w / 2.0, 0.6, 0.0));
        chip.set_fill_color(CHIP_FILL).set_fill_opacity(0.92);
        chip.set_stroke_color(MERGE_COL[i % MERGE_COL.len()])
            .set_stroke_width(0.02);
        strip.push(chip);
        x += w + 0.14;
    }
    content.push(life_seq(&mut strip, 144.0, 147.8, 0.5, 0.5, 0.02, vec![]));
    caption(
        content,
        "unfamiliar scripts & emoji: each byte becomes its own token",
        0.3,
        dvec3(0.0, -0.4, 0.0),
        TEXT_COL,
        144.6,
        147.8,
    );

    // Recap & outro.
    caption(
        content,
        "count pairs → merge the most frequent → repeat",
        0.42,
        dvec3(0.0, -1.7, 0.0),
        manim::YELLOW_C,
        148.6,
        OUT_T,
    );
    caption(
        content,
        "Now you know how an LLM reads.",
        0.5,
        dvec3(0.0, -3.0, 0.0),
        TEXT_COL,
        152.0,
        OUT_T,
    );
}

// MARK: Scene

#[scene]
#[output(dir = "./output/agents/bpe_tokenizer")]
fn bpe_tokenizer(r: &mut RanimScene) {
    let merges: Vec<Pair> = {
        let (stages, _) = training();
        stages.iter().map(|s| s.winner.clone()).collect()
    };

    let mut content = AnimStack::new();
    act0(&mut content);
    act1(&mut content);
    act2(&mut content);
    act3(&mut content, &merges);
    act4(&mut content);

    r.play(CameraFrame::default().show().with_duration(TOTAL));
    r.play(content);

    // Captures: hook answer, merge-1 winner moment, slowest encoded, recap.
    r.insert_time_mark(15.0, TimeMark::Capture("hook.png".to_string()));
    r.insert_time_mark(
        MERGE_T0 + WINNER_AT + 1.2,
        TimeMark::Capture("preview.png".to_string()),
    );
    r.insert_time_mark(125.8, TimeMark::Capture("slowest.png".to_string()));
    r.insert_time_mark(150.5, TimeMark::Capture("recap.png".to_string()));
}
