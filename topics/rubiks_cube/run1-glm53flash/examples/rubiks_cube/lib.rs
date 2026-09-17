//! Rubik's cube — a complete layer-by-layer solve, shown simultaneously on a
//! 3D cube and on a flat unfolded net.
//!
//! Every number on screen is real: the scramble, the solve moves and the
//! per-phase move counts all come from a layer-by-layer solver built into
//! this file, operating on the exact same sticker state that drives the
//! animation. The solver is:
//!
//! - stages 1–3 (white cross, bottom corners, middle edges): bidirectional
//!   BFS over an abstract tracked-piece state, so every insertion is
//!   near-optimal and provably preserves everything already solved (the
//!   solved pieces are part of the goal check);
//! - stage 4 (last layer): BFS over a small generator set (U turns plus the
//!   classic OLL/PLL algorithms), which keeps the searches tiny;
//! - the generator algorithms are verified by unit tests, and the whole
//!   solve is verified end-to-end by tests over random scrambles.
//!
//! The 3D cube is 26 [`MeshItem`] cubies (dark body + raised sticker quads);
//! a face turn is a custom [`Eval`] rotating the 9 cubies of the layer. The
//! unfolded net (cross layout) is 54 [`VItem`] squares whose sticker
//! permutation is computed with the same integer rotation math, so the two
//! views can never disagree.

use std::collections::{HashMap, HashSet};
use std::f64::consts::{FRAC_PI_2, PI, TAU};
use std::hash::{BuildHasherDefault, Hasher};
use std::sync::OnceLock;

use ranim::{
    anims::{fading::FadingAnim, morph::MorphAnim},
    color::{AlphaColor, Srgb, palettes::manim},
    core::components::rgba::Rgba,
    glam::{DAffine3, DVec3, dvec3},
    items::{
        mesh::MeshItem,
        vitem::{VItem, geometry::Square, svg::SvgItem, typst::typst_svg},
    },
    prelude::*,
    utils::rate_functions::{linear, smooth},
};

// MARK: Face model

/// Outward normal of each face in cubie grid coordinates. Faces are indexed
/// `U R F D L B` throughout this file.
///
/// The world frame is Z-up with `F = +X`, `R = +Y`, `U = +Z`; the camera
/// sits in the (+X, +Y, +Z) octant, so F (green, screen left), R (red,
/// screen right) and U (yellow, top) are visible.
const FACE_NORMALS: [IVec; 6] = [
    [0, 0, 1],  // U
    [0, 1, 0],  // R
    [1, 0, 0],  // F
    [0, 0, -1], // D
    [0, -1, 0], // L
    [-1, 0, 0], // B
];

/// In-face "right" axis of each face when viewed from outside (net layout).
const FACE_RIGHT: [IVec; 6] = [
    [0, 1, 0],  // U: right points to R
    [-1, 0, 0], // R: right points to B
    [0, 1, 0],  // F: right points to R
    [0, 1, 0],  // D: right points to R
    [1, 0, 0],  // L: right points to F
    [0, -1, 0], // B: right points to L
];

/// In-face "up" axis of each face when viewed from outside.
const FACE_UP: [IVec; 6] = [
    [-1, 0, 0], // U: up points to B
    [0, 0, 1],  // R: up points to U
    [0, 0, 1],  // F: up points to U
    [1, 0, 0],  // D: up points to F
    [0, 0, 1],  // L: up points to U
    [0, 0, 1],  // B: up points to U
];

/// Top-left block (column, row) of each face in the net cross layout:
/// `U` above the `L F R B` row, `D` below `F`.
const NET_BLOCKS: [(i32, i32); 6] = [
    (3, 0), // U
    (6, 3), // R
    (3, 3), // F
    (3, 6), // D
    (0, 3), // L
    (9, 3), // B
];

/// Standard color scheme with white on the bottom face (the layer-by-layer
/// method builds the white cross on D): U yellow, R red, F green, D white,
/// L orange, B blue.
const FACE_COLORS: [AlphaColor<Srgb>; 6] = [
    manim::YELLOW_C, // U
    manim::RED_C,    // R
    manim::GREEN_C,  // F
    manim::WHITE,    // D
    manim::ORANGE,   // L
    manim::BLUE_C,   // B
];

const FACE_LETTERS: [&str; 6] = ["U", "R", "F", "D", "L", "B"];

/// White is the color of the D face (face index 3): the first stage of the
/// solve builds the white cross there.
const WHITE: u8 = 3;
/// Yellow is the color of the U face (face index 0): the last layer.
const YELLOW: u8 = 0;

/// Integer vector in the cubie grid; components are in `{-1, 0, 1}` for
/// positions and are unit axis vectors for normals.
type IVec = [i32; 3];

fn idot(a: IVec, b: IVec) -> i32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn icross(a: IVec, b: IVec) -> IVec {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn iscale(v: IVec, s: i32) -> IVec {
    [v[0] * s, v[1] * s, v[2] * s]
}

fn iadd(a: IVec, b: IVec) -> IVec {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn to_dvec(v: IVec) -> DVec3 {
    dvec3(v[0] as f64, v[1] as f64, v[2] as f64)
}

/// Rotate `v` by `quarter` quarter-turns around the unit axis vector `axis`
/// (right-hand rule; `quarter` 3 means -90°).
fn rot90(v: IVec, axis: IVec, quarter: i32) -> IVec {
    let parallel = iscale(axis, idot(axis, v));
    match quarter.rem_euclid(4) {
        0 => v,
        // +90°: v' = axis × v + axis (axis · v)  (Rodrigues)
        1 => iadd(icross(axis, v), parallel),
        // 180°: v' = 2 axis (axis · v) - v
        2 => iadd(parallel, iadd(parallel, iscale(v, -1))),
        // -90°: v' = -axis × v + axis (axis · v)
        _ => iadd(iscale(icross(axis, v), -1), parallel),
    }
}

/// Grid position of sticker `idx` (row-major, viewed from outside) of `face`.
fn sticker_pos(face: usize, idx: usize) -> IVec {
    let (row, col) = ((idx / 3) as i32, (idx % 3) as i32);
    iadd(
        FACE_NORMALS[face],
        iadd(
            iscale(FACE_RIGHT[face], col - 1),
            iscale(FACE_UP[face], 1 - row),
        ),
    )
}

/// Inverse of [`sticker_pos`]: locate the (face, idx) of a sticker from its
/// grid position and (rotated) outward normal.
fn sticker_locate(pos: IVec, normal: IVec) -> (usize, usize) {
    let face = FACE_NORMALS
        .iter()
        .position(|&n| n == normal)
        .expect("normal must be a face normal");
    let col = (idot(pos, FACE_RIGHT[face]) + 1) as usize;
    let row = (1 - idot(pos, FACE_UP[face])) as usize;
    (face, row * 3 + col)
}

/// One face turn: `quarter` quarter-turns of the outer layer of `face`
/// (right-hand rule around the outward normal).
///
/// This is the engineer's convention: the cuber's clockwise `X` is
/// `(face, quarter = 3)`, `X'` is `(face, 1)` and `X2` is `(face, 2)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Move {
    face: u8,
    quarter: u8,
}

impl Move {
    fn inverse(self) -> Self {
        let quarter = match self.quarter {
            1 => 3,
            3 => 1,
            q => q,
        };
        Self {
            face: self.face,
            quarter,
        }
    }

    /// Rotation angle in radians, matching [`rot90`] for the same quarter.
    fn angle(self) -> f64 {
        match self.quarter {
            1 => FRAC_PI_2,
            2 => PI,
            _ => -FRAC_PI_2,
        }
    }
}

/// All 18 quarter/half turns, indexed `face * 3 + (quarter - 1)`.
const fn m(face: u8, quarter: u8) -> Move {
    Move { face, quarter }
}

const ALL_MOVES: [Move; 18] = [
    m(0, 1),
    m(0, 2),
    m(0, 3),
    m(1, 1),
    m(1, 2),
    m(1, 3),
    m(2, 1),
    m(2, 2),
    m(2, 3),
    m(3, 1),
    m(3, 2),
    m(3, 3),
    m(4, 1),
    m(4, 2),
    m(4, 3),
    m(5, 1),
    m(5, 2),
    m(5, 3),
];

fn move_index(mv: Move) -> usize {
    ALL_MOVES
        .iter()
        .position(|&m| m == mv)
        .expect("move must be one of the 18 base moves")
}

fn inverse_index(mi: usize) -> usize {
    move_index(ALL_MOVES[mi].inverse())
}

/// Cuber-notation algorithms translated into the engine convention
/// (`X` → quarter 3, `X'` → quarter 1, `X2` → quarter 2), used as last-layer
/// generators. Each is verified by the `ll_generators_behave` test.
mod algs {
    use super::Move;

    const fn m(face: u8, quarter: u8) -> Move {
        Move { face, quarter }
    }

    /// F R U R' U' F' — flips UF and UR in place; makes a yellow cross from
    /// the "line" case (repeated for L and dot cases).
    pub const OLL_CROSS: [Move; 6] = [m(2, 3), m(1, 3), m(0, 3), m(1, 1), m(0, 1), m(2, 1)];

    /// R U R' U R U2 R' — Sune (OLL 27): twists U corners while keeping all
    /// edges fixed.
    pub const SUNE: [Move; 7] = [
        m(1, 3),
        m(0, 3),
        m(1, 1),
        m(0, 3),
        m(1, 3),
        m(0, 2),
        m(1, 1),
    ];

    /// U R U' L' U R' U' L — Niklas: 3-cycles U corners, orientation and all
    /// edges preserved.
    ///
    /// NOTE: this textbook variant turned out to twist corners (caught by
    /// `ll_generators_behave`); the solver uses A_PERM below instead.
    #[allow(dead_code)]
    pub const NIKLAS: [Move; 8] = [
        m(0, 3),
        m(1, 3),
        m(0, 1),
        m(4, 1),
        m(0, 3),
        m(1, 1),
        m(0, 1),
        m(4, 3),
    ];

    /// R' F R' B2 R F' R' B2 R2 — Aa perm: orientation-preserving corner
    /// 3-cycle; all edges and both lower layers preserved.
    pub const A_PERM: [Move; 9] = [
        m(1, 1),
        m(2, 3),
        m(1, 1),
        m(5, 2),
        m(1, 3),
        m(2, 1),
        m(1, 1),
        m(5, 2),
        m(1, 2),
    ];

    /// R U' R U R U R U' R' U' R2 — Ua perm: 3-cycles U edges, corners fixed.
    pub const UA_PERM: [Move; 11] = [
        m(1, 3),
        m(0, 1),
        m(1, 3),
        m(0, 3),
        m(1, 3),
        m(0, 3),
        m(1, 3),
        m(0, 1),
        m(1, 1),
        m(0, 1),
        m(1, 2),
    ];
}

/// Logical cube state: `state[face][idx]` = color of that sticker, encoded
/// as the face index the sticker belongs to when solved.
type State = [[usize; 9]; 6];

fn solved_state() -> State {
    std::array::from_fn(|f| [f; 9])
}

/// Apply the sticker permutation of `mv` to the logical cube state.
fn apply_move(state: &mut State, mv: Move) {
    let axis = FACE_NORMALS[mv.face as usize];
    let old = *state;
    for (face, old_face) in old.iter().enumerate() {
        for (idx, &color) in old_face.iter().enumerate() {
            let pos = sticker_pos(face, idx);
            if idot(pos, axis) != 1 {
                continue; // sticker not in the turned layer
            }
            let pos = rot90(pos, axis, mv.quarter as i32);
            let normal = rot90(FACE_NORMALS[face], axis, mv.quarter as i32);
            let (f2, i2) = sticker_locate(pos, normal);
            state[f2][i2] = color;
        }
    }
}

/// Deterministic xorshift64-based scramble: `n` moves, never twice the same
/// face in a row.
fn scramble_moves(n: usize, seed: u64) -> Vec<Move> {
    let mut x = seed | 1;
    let mut next = move || {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        x
    };
    let mut moves = Vec::with_capacity(n);
    while moves.len() < n {
        let face = (next() % 6) as u8;
        if moves.last().is_some_and(|m: &Move| m.face == face) {
            continue;
        }
        let quarter = [1, 2, 3][(next() % 3) as usize];
        moves.push(Move { face, quarter });
    }
    moves
}

// MARK: Pieces and slots

/// A facelet id: `face * 9 + idx`.
type Facelet = usize;

/// The 12 edge slots, each holding the slot's two facelets ordered by
/// facelet id. Computed once from the grid geometry.
fn edge_slots() -> &'static Vec<[Facelet; 2]> {
    static SLOTS: OnceLock<Vec<[Facelet; 2]>> = OnceLock::new();
    SLOTS.get_or_init(|| {
        group_slots(2)
            .into_iter()
            .map(|mut fs| {
                fs.sort_unstable();
                [fs[0], fs[1]]
            })
            .collect()
    })
}

/// The 8 corner slots, each holding the slot's three facelets ordered by
/// facelet id.
fn corner_slots() -> &'static Vec<[Facelet; 3]> {
    static SLOTS: OnceLock<Vec<[Facelet; 3]>> = OnceLock::new();
    SLOTS.get_or_init(|| {
        group_slots(3)
            .into_iter()
            .map(|mut fs| {
                fs.sort_unstable();
                [fs[0], fs[1], fs[2]]
            })
            .collect()
    })
}

/// Group facelets by grid position; cubies with exactly `n` stickers.
fn group_slots(n: usize) -> Vec<Vec<Facelet>> {
    let mut by_pos: HashMap<IVec, Vec<Facelet>> = HashMap::new();
    for face in 0..6 {
        for idx in 0..9 {
            let pos = sticker_pos(face, idx);
            if pos.iter().filter(|&&c| c != 0).count() == n {
                by_pos.entry(pos).or_default().push(face * 9 + idx);
            }
        }
    }
    let mut groups: Vec<Vec<Facelet>> = by_pos.into_values().collect();
    groups.sort_unstable();
    groups
}

/// The 12 edge pieces as sorted color pairs (colors are face indices, since
/// the solved cube paints face `f` with color `f`).
fn edge_pieces() -> &'static Vec<(u8, u8)> {
    static PIECES: OnceLock<Vec<(u8, u8)>> = OnceLock::new();
    PIECES.get_or_init(|| {
        let mut pieces: Vec<(u8, u8)> = Vec::new();
        for a in 0..6u8 {
            for b in a + 1..6u8 {
                if idot(FACE_NORMALS[a as usize], FACE_NORMALS[b as usize]) == 0 {
                    pieces.push((a, b));
                }
            }
        }
        pieces
    })
}

/// The 8 corner pieces as sorted color triples (one color from each of the
/// three opposite pairs).
fn corner_pieces() -> &'static Vec<(u8, u8, u8)> {
    static PIECES: OnceLock<Vec<(u8, u8, u8)>> = OnceLock::new();
    PIECES.get_or_init(|| {
        let mut pieces = Vec::new();
        for &u in &[0u8, 3u8] {
            for &r in &[1u8, 4u8] {
                for &f in &[2u8, 5u8] {
                    let mut c = [u, r, f];
                    c.sort_unstable();
                    pieces.push((c[0], c[1], c[2]));
                }
            }
        }
        pieces
    })
}

/// Facelets of the edge piece with colors `(a, b)` in the solved cube.
fn edge_piece_facelets(a: u8, b: u8) -> [Facelet; 2] {
    let pos = iadd(FACE_NORMALS[a as usize], FACE_NORMALS[b as usize]);
    let fa = sticker_locate(pos, FACE_NORMALS[a as usize]);
    let fb = sticker_locate(pos, FACE_NORMALS[b as usize]);
    [fa.0 * 9 + fa.1, fb.0 * 9 + fb.1]
}

/// Facelets of the corner piece with colors `(a, b, c)` in the solved cube.
fn corner_piece_facelets(a: u8, b: u8, c: u8) -> [Facelet; 3] {
    let pos = iadd(
        FACE_NORMALS[a as usize],
        iadd(FACE_NORMALS[b as usize], FACE_NORMALS[c as usize]),
    );
    let mut fs: Vec<Facelet> = [a, b, c]
        .iter()
        .map(|&f| {
            let (fc, fi) = sticker_locate(pos, FACE_NORMALS[f as usize]);
            fc * 9 + fi
        })
        .collect();
    fs.sort_unstable();
    [fs[0], fs[1], fs[2]]
}

fn find_edge_slot(want: [Facelet; 2]) -> usize {
    edge_slots()
        .iter()
        .position(|s| *s == want)
        .expect("edge slot must exist")
}

fn find_corner_slot(want: [Facelet; 3]) -> usize {
    corner_slots()
        .iter()
        .position(|s| *s == want)
        .expect("corner slot must exist")
}

// MARK: Move tables

/// Per-move transition tables derived once from the sticker permutation.
struct MoveTables {
    /// Edge transition: `edge[m][slot][ori] -> [new_slot, new_ori]`.
    ///
    /// Orientation = which of the slot's facelets (in slot order) holds the
    /// piece's primary (smaller) color, so orientation 0 at the home slot is
    /// exactly the solved orientation.
    edge: Vec<[[[u8; 2]; 2]; 12]>,
    /// Corner transition: `corner[m][slot][ori] -> [new_slot, new_ori]`.
    corner: Vec<[[[u8; 2]; 3]; 8]>,
}

fn move_tables() -> &'static MoveTables {
    static TABLES: OnceLock<MoveTables> = OnceLock::new();
    TABLES.get_or_init(|| {
        // Facelet permutation per move: new[to] = old[from].
        let facelet_from: Vec<[u8; 54]> = ALL_MOVES
            .iter()
            .map(|&mv| {
                let mut lab = solved_state();
                for (f, row) in lab.iter_mut().enumerate() {
                    for (i, cell) in row.iter_mut().enumerate() {
                        *cell = f * 9 + i;
                    }
                }
                apply_move(&mut lab, mv);
                let mut perm = [0u8; 54];
                for f in 0..6 {
                    for i in 0..9 {
                        perm[f * 9 + i] = lab[f][i] as u8;
                    }
                }
                perm
            })
            .collect();

        // Invert once: `sticker_to[from]` = the facelet the sticker at
        // `from` moves to under the move.
        let sticker_to: Vec<[u8; 54]> = facelet_from
            .iter()
            .map(|perm| {
                let mut inv = [0u8; 54];
                for (to, &from) in perm.iter().enumerate() {
                    inv[from as usize] = to as u8;
                }
                inv
            })
            .collect();

        let mut edge = Vec::with_capacity(18);
        for perm in &sticker_to {
            let mut table = [[[0u8; 2]; 2]; 12];
            for (si, s) in edge_slots().iter().enumerate() {
                for (ori, &src) in s.iter().enumerate() {
                    let dest = perm[src] as Facelet;
                    let (sj, j) = edge_slots()
                        .iter()
                        .enumerate()
                        .find_map(|(sj, s2)| s2.iter().position(|&f| f == dest).map(|j| (sj, j)))
                        .expect("edge sticker must land in an edge slot");
                    table[si][ori] = [sj as u8, j as u8];
                }
            }
            edge.push(table);
        }

        let mut corner = Vec::with_capacity(18);
        for perm in &sticker_to {
            let mut table = [[[0u8; 2]; 3]; 8];
            for (si, s) in corner_slots().iter().enumerate() {
                for (ori, &src) in s.iter().enumerate() {
                    let dest = perm[src] as Facelet;
                    let (sj, j) = corner_slots()
                        .iter()
                        .enumerate()
                        .find_map(|(sj, s2)| s2.iter().position(|&f| f == dest).map(|j| (sj, j)))
                        .expect("corner sticker must land in a corner slot");
                    table[si][ori] = [sj as u8, j as u8];
                }
            }
            corner.push(table);
        }

        MoveTables { edge, corner }
    })
}

// MARK: Solver searches

/// A tracked piece: an edge or corner piece by index into the piece tables.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum PieceRef {
    Edge(usize),
    Corner(usize),
}

impl PieceRef {
    fn colors(self) -> Vec<u8> {
        match self {
            PieceRef::Edge(k) => {
                let (a, b) = edge_pieces()[k];
                vec![a, b]
            }
            PieceRef::Corner(k) => {
                let (a, b, c) = corner_pieces()[k];
                vec![a, b, c]
            }
        }
    }

    /// Home slot of the piece. Orientation 0 = the piece's primary (smallest)
    /// color on the home slot's first facelet, which is exactly the solved
    /// orientation.
    fn home_slot(self) -> usize {
        match self {
            PieceRef::Edge(k) => {
                let (a, b) = edge_pieces()[k];
                find_edge_slot(edge_piece_facelets(a, b))
            }
            PieceRef::Corner(k) => {
                let (a, b, c) = corner_pieces()[k];
                find_corner_slot(corner_piece_facelets(a, b, c))
            }
        }
    }
}

/// Goal for one tracked piece: sit in one of `slot_mask`'s slots (bit `s`
/// set = slot `s` allowed) with one of `ori_mask`'s orientations.
#[derive(Clone, Copy)]
struct Goal {
    slot_mask: u16,
    ori_mask: u8,
}

impl Goal {
    fn home_exact(piece: PieceRef) -> Self {
        Goal {
            slot_mask: 1 << piece.home_slot(),
            ori_mask: 0b001,
        }
    }

    fn in_slots(slot_mask: u16, any_ori: bool) -> Self {
        Goal {
            slot_mask,
            ori_mask: if any_ori { 0b111 } else { 0b001 },
        }
    }

    fn check(self, slot: u8, ori: u8) -> bool {
        self.slot_mask >> slot & 1 == 1 && self.ori_mask >> ori & 1 == 1
    }
}

/// Slot masks of the U-layer edge / corner slots.
fn u_edge_slots() -> u16 {
    edge_slots()
        .iter()
        .enumerate()
        .filter(|(_, s)| s.iter().any(|&f| f / 9 == 0))
        .fold(0u16, |acc, (si, _)| acc | 1 << si)
}

fn u_corner_slots() -> u16 {
    corner_slots()
        .iter()
        .enumerate()
        .filter(|(_, s)| s.iter().any(|&f| f / 9 == 0))
        .fold(0u16, |acc, (si, _)| acc | 1 << si)
}

/// Locate a piece in the state: (slot, ori), ori counted against the piece's
/// primary (smallest) color.
fn locate(state: &State, piece: PieceRef) -> (usize, u8) {
    let colors = piece.colors();
    let primary = colors.iter().copied().min().unwrap() as usize;
    let mut want = colors.iter().map(|&c| c as usize).collect::<Vec<_>>();
    want.sort_unstable();
    match piece {
        PieceRef::Edge(_) => {
            for (si, s) in edge_slots().iter().enumerate() {
                let mut have: Vec<usize> = s.iter().map(|&f| state[f / 9][f % 9]).collect();
                have.sort_unstable();
                if have == want {
                    let ori = u8::from(state[s[0] / 9][s[0] % 9] != primary);
                    return (si, ori);
                }
            }
        }
        PieceRef::Corner(_) => {
            for (si, s) in corner_slots().iter().enumerate() {
                let mut have: Vec<usize> = s.iter().map(|&f| state[f / 9][f % 9]).collect();
                have.sort_unstable();
                if have == want {
                    let ori = s
                        .iter()
                        .position(|&f| state[f / 9][f % 9] == primary)
                        .expect("primary color must be on one of the slot's facelets")
                        as u8;
                    return (si, ori);
                }
            }
        }
    }
    panic!("piece {piece:?} not found in state");
}

/// Pack the tracked pieces into a u64, 5 bits each.
fn pack(state: &State, refs: &[PieceRef]) -> u64 {
    let mut out = 0u64;
    for (k, &r) in refs.iter().enumerate() {
        let (slot, ori) = locate(state, r);
        let v = match r {
            PieceRef::Edge(_) => ((slot as u64) << 1) | ori as u64,
            PieceRef::Corner(_) => ((slot as u64) << 2) | ori as u64,
        };
        out |= v << (5 * k);
    }
    out
}

fn decode_piece(r: PieceRef, v: u64) -> (usize, usize) {
    match r {
        PieceRef::Edge(_) => ((v >> 1) as usize, (v & 1) as usize),
        PieceRef::Corner(_) => ((v >> 2) as usize, (v & 3) as usize),
    }
}

fn encode_piece(r: PieceRef, slot: usize, ori: usize) -> u64 {
    match r {
        PieceRef::Edge(_) => ((slot as u64) << 1) | ori as u64,
        PieceRef::Corner(_) => ((slot as u64) << 2) | ori as u64,
    }
}

fn goals_met(refs: &[PieceRef], goals: &[Goal], packed: u64) -> bool {
    refs.iter().enumerate().zip(goals).all(|((k, r), g)| {
        let (slot, ori) = decode_piece(*r, (packed >> (5 * k)) & 0x1f);
        g.check(slot as u8, ori as u8)
    })
}

/// Apply one move to a packed tracked state.
fn expand_packed(packed: u64, mi: usize, refs: &[PieceRef]) -> u64 {
    let tables = move_tables();
    let mut out = 0u64;
    for (k, r) in refs.iter().enumerate() {
        let (slot, ori) = decode_piece(*r, (packed >> (5 * k)) & 0x1f);
        let [s2, o2] = match r {
            PieceRef::Edge(_) => tables.edge[mi][slot][ori],
            PieceRef::Corner(_) => tables.corner[mi][slot][ori],
        };
        out |= encode_piece(*r, s2 as usize, o2 as usize) << (5 * k);
    }
    out
}

/// Multiplicative hasher for the solver's u64-keyed maps (SipHash dominates
/// the search otherwise).
#[derive(Default)]
struct FxHasher(u64);

impl Hasher for FxHasher {
    fn finish(&self) -> u64 {
        self.0
    }
    fn write(&mut self, _bytes: &[u8]) {
        unreachable!("solver maps are keyed by u64 only");
    }
    fn write_u64(&mut self, v: u64) {
        self.0 = self.0.rotate_left(5) ^ v;
        self.0 = self.0.wrapping_mul(0x517c_c1b7_2722_0a95);
    }
}

type FastMap<V> = HashMap<u64, V, BuildHasherDefault<FxHasher>>;
type FastSet = HashSet<u64, BuildHasherDefault<FxHasher>>;

/// Bidirectional BFS in tracked-piece space.
///
/// `start` is the current packed state; `goal_states` enumerates the (small)
/// goal set. Returns base-move indices. Both sides are depth-limited; the
/// union of the two bounds must cover the optimal solution length. `allowed`
/// filters the move set (insertions only need U turns plus the target slot's
/// side faces). Moves never repeat the same face consecutively (any shortest
/// path can be rewritten that way, so the pruning keeps the search complete).
fn bidir_bfs(
    start: u64,
    goal_states: &[u64],
    refs: &[PieceRef],
    fwd_depth: usize,
    bwd_depth: usize,
    allowed: &[usize],
) -> Option<Vec<usize>> {
    const NO_FACE: u8 = u8::MAX;
    if goal_states.contains(&start) {
        return Some(Vec::new());
    }

    // Backward: from the goal set, expanding by inverse moves. Each state
    // stores the full move path that leads from it to the goal.
    let mut bwd: FastMap<Vec<u8>> = FastMap::default();
    let mut frontier: Vec<(u64, u8)> = goal_states
        .iter()
        .filter(|g| bwd.insert(**g, Vec::new()).is_none())
        .map(|&g| (g, NO_FACE))
        .collect();
    for _ in 0..bwd_depth {
        let mut next = Vec::new();
        for &(t, last_face) in &frontier {
            for &mi in allowed {
                if mi as u8 / 3 == last_face {
                    continue;
                }
                let s = expand_packed(t, inverse_index(mi), refs);
                if bwd.contains_key(&s) {
                    continue;
                }
                let mut path = Vec::with_capacity(bwd[&t].len() + 1);
                path.push(mi as u8);
                path.extend_from_slice(&bwd[&t]);
                bwd.insert(s, path);
                next.push((s, mi as u8));
            }
        }
        frontier = next;
    }

    // Forward: layer by layer, checking the backward map on every new state.
    let mut fwd: FastMap<Vec<u8>> = FastMap::default();
    fwd.insert(start, Vec::new());
    frontier = vec![(start, NO_FACE)];
    for _ in 0..fwd_depth {
        let mut next = Vec::new();
        for &(t, last_face) in &frontier {
            for &mi in allowed {
                if mi as u8 / 3 == last_face {
                    continue;
                }
                let s = expand_packed(t, mi, refs);
                if fwd.contains_key(&s) {
                    continue;
                }
                let mut path = fwd[&t].clone();
                path.push(mi as u8);
                fwd.insert(s, path.clone());
                if let Some(tail) = bwd.get(&s) {
                    path.extend(tail.iter().copied());
                    return Some(path.into_iter().map(|m| m as usize).collect());
                }
                next.push((s, mi as u8));
            }
        }
        frontier = next;
    }
    None
}
/// Move indices for U turns plus quarter/half turns of `faces`.
fn u_plus_faces(faces: impl IntoIterator<Item = u8>) -> Vec<usize> {
    let mut out: Vec<usize> = (0..3).collect(); // U, U2, U'
    for face in faces {
        out.extend([
            face as usize * 3,
            face as usize * 3 + 1,
            face as usize * 3 + 2,
        ]);
    }
    out
}

/// All 18 move indices.
fn all_move_indices() -> Vec<usize> {
    (0..ALL_MOVES.len()).collect()
}

/// The side faces (not U, not D) of the slot a piece currently occupies.
fn piece_slot_faces(state: &State, piece: PieceRef) -> Vec<u8> {
    let (slot, _) = locate(state, piece);
    let facelets: Vec<Facelet> = match piece {
        PieceRef::Edge(_) => edge_slots()[slot].to_vec(),
        PieceRef::Corner(_) => corner_slots()[slot].to_vec(),
    };
    let mut faces: Vec<u8> = facelets
        .iter()
        .map(|&f| (f / 9) as u8)
        .filter(|&f| f != 0 && f != WHITE)
        .collect();
    faces.sort_unstable();
    faces.dedup();
    faces
}

/// Enumerate the packed states satisfying the goals (cheap: only free pieces
/// contribute more than one state).
fn enumerate_goals(refs: &[PieceRef], goals: &[Goal]) -> Vec<u64> {
    let mut out = vec![0u64];
    for (k, r) in refs.iter().enumerate() {
        let slot_count = match r {
            PieceRef::Edge(_) => 12,
            PieceRef::Corner(_) => 8,
        };
        let ori_count = match r {
            PieceRef::Edge(_) => 2,
            PieceRef::Corner(_) => 3,
        };
        let mut next = Vec::new();
        for &base in &out {
            for slot in 0..slot_count {
                if goals[k].slot_mask >> slot & 1 == 0 {
                    continue;
                }
                for ori in 0..ori_count {
                    if goals[k].ori_mask >> ori & 1 == 0 {
                        continue;
                    }
                    next.push(base | (encode_piece(*r, slot, ori) << (5 * k)));
                }
            }
        }
        out = next;
    }
    out
}

/// Depth-escalating search wrapper over the given move set.
fn search(
    state: &State,
    refs: &[PieceRef],
    goals: &[Goal],
    max_total: usize,
    allowed: &[usize],
) -> Option<Vec<Move>> {
    let start = pack(state, refs);
    let goal_states = enumerate_goals(refs, goals);
    for (f, b) in [(3usize, 3usize), (4usize, 4usize), (5usize, 5usize)] {
        if f + b > max_total {
            break;
        }
        if let Some(path) = bidir_bfs(start, &goal_states, refs, f, b, allowed) {
            return Some(path.into_iter().map(|i| ALL_MOVES[i]).collect());
        }
    }
    None
}

/// BFS over a small generator set (each generator = a fixed move sequence).
fn gen_bfs(
    start_packed: u64,
    refs: &[PieceRef],
    goals: &[Goal],
    gens: &[Vec<Move>],
    max_depth: usize,
) -> Option<Vec<Move>> {
    if goals_met(refs, goals, start_packed) {
        return Some(Vec::new());
    }
    let mut visited: FastSet = FastSet::default();
    visited.insert(start_packed);
    let mut frontier: Vec<(u64, Vec<Move>)> = vec![(start_packed, Vec::new())];
    for _ in 0..max_depth {
        let mut next = Vec::new();
        for (packed, path) in &frontier {
            for algo in gens {
                let mut s = *packed;
                for &mv in algo {
                    s = expand_packed(s, move_index(mv), refs);
                }
                if visited.insert(s) {
                    let mut p = path.clone();
                    p.extend_from_slice(algo);
                    if goals_met(refs, goals, s) {
                        return Some(p);
                    }
                    next.push((s, p));
                }
            }
        }
        frontier = next;
        if frontier.is_empty() {
            break;
        }
    }
    None
}

// MARK: The layer-by-layer solver

/// Solve phases, in order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Cross,
    BottomCorners,
    MiddleEdges,
    LastCross,
    LastOrient,
    LastPermCorners,
    LastPermEdges,
}

impl Phase {
    fn title(self) -> &'static str {
        match self {
            Phase::Cross => "第 1 步 · 白色十字",
            Phase::BottomCorners => "第 2 步 · 底层角块",
            Phase::MiddleEdges => "第 3 步 · 中层棱块",
            Phase::LastCross => "第 4 步 · 顶层 黄色十字",
            Phase::LastOrient => "顶层 · 翻正角块",
            Phase::LastPermCorners => "顶层 · 角块归位",
            Phase::LastPermEdges => "顶层 · 棱块归位",
        }
    }
}

fn edge_piece_of(a: u8, b: u8) -> usize {
    let (lo, hi) = (a.min(b), a.max(b));
    edge_pieces()
        .iter()
        .position(|&(x, y)| (x, y) == (lo, hi))
        .expect("edge piece must exist")
}

fn corner_piece_of(a: u8, b: u8, c: u8) -> usize {
    let mut want = [a, b, c];
    want.sort_unstable();
    corner_pieces()
        .iter()
        .position(|&t| t == (want[0], want[1], want[2]))
        .expect("corner piece must exist")
}

fn white_edge_refs() -> Vec<PieceRef> {
    [(WHITE, 2), (WHITE, 1), (WHITE, 5), (WHITE, 4)]
        .into_iter()
        .map(|(a, b)| PieceRef::Edge(edge_piece_of(a, b)))
        .collect()
}

fn white_corner_refs() -> Vec<PieceRef> {
    [(WHITE, 2, 1), (WHITE, 1, 5), (WHITE, 5, 4), (WHITE, 4, 2)]
        .into_iter()
        .map(|(a, b, c)| PieceRef::Corner(corner_piece_of(a, b, c)))
        .collect()
}

fn middle_edge_refs() -> Vec<PieceRef> {
    [(2, 1), (1, 5), (5, 4), (4, 2)]
        .into_iter()
        .map(|(a, b)| PieceRef::Edge(edge_piece_of(a, b)))
        .collect()
}

fn yellow_edge_refs() -> Vec<PieceRef> {
    [(YELLOW, 2), (YELLOW, 1), (YELLOW, 5), (YELLOW, 4)]
        .into_iter()
        .map(|(a, b)| PieceRef::Edge(edge_piece_of(a, b)))
        .collect()
}

fn yellow_corner_refs() -> Vec<PieceRef> {
    [
        (YELLOW, 1, 2),
        (YELLOW, 1, 5),
        (YELLOW, 4, 5),
        (YELLOW, 2, 4),
    ]
    .into_iter()
    .map(|(a, b, c)| PieceRef::Corner(corner_piece_of(a, b, c)))
    .collect()
}

/// Is the piece currently in the U layer (its slot touches the U face)?
fn in_u_layer(state: &State, piece: PieceRef) -> bool {
    let (slot, _) = locate(state, piece);
    match piece {
        PieceRef::Edge(_) => u_edge_slots() >> slot & 1 == 1,
        PieceRef::Corner(_) => u_corner_slots() >> slot & 1 == 1,
    }
}

fn apply_all(state: &mut State, moves: &[Move]) {
    for &mv in moves {
        apply_move(state, mv);
    }
}

/// Place one piece at home (orientation 0) without disturbing `priors`,
/// ejecting it to the U layer first if it is buried in a lower layer.
///
/// Insertion and ejection only need U turns plus the slot's side faces (the
/// beginner-method move family), which keeps the searches tiny; if that
/// family ever fails, fall back to the full 18-move search.
fn place_piece(state: &mut State, piece: PieceRef, priors: &[PieceRef]) -> Vec<Move> {
    let mut total = Vec::new();
    for _ in 0..3 {
        let (slot, ori) = locate(state, piece);
        if slot == piece.home_slot() && ori == 0 {
            return total;
        }
        let mut refs = priors.to_vec();
        refs.push(piece);
        let mut goals: Vec<Goal> = priors.iter().map(|&p| Goal::home_exact(p)).collect();
        let in_u = in_u_layer(state, piece);
        if in_u {
            goals.push(Goal::home_exact(piece));
        } else {
            let u_mask = match piece {
                PieceRef::Edge(_) => u_edge_slots(),
                PieceRef::Corner(_) => u_corner_slots(),
            };
            goals.push(Goal::in_slots(u_mask, true));
        }

        // Restricted family: U turns + the relevant slot's side faces.
        let slot_faces = piece_slot_faces(state, piece);
        let allowed = u_plus_faces(slot_faces);
        let sol = search(state, &refs, &goals, 8, &allowed)
            .or_else(|| search(state, &refs, &goals, 10, &all_move_indices()))
            .unwrap_or_else(|| panic!("placing {piece:?} must be solvable"));
        apply_all(state, &sol);
        total.extend(sol);
    }
    let (slot, ori) = locate(state, piece);
    assert!(
        slot == piece.home_slot() && ori == 0,
        "failed to place {piece:?} (ended at slot {slot} ori {ori})"
    );
    total
}

/// All 20 movable pieces at home with orientation 0.
fn is_solved(state: &State) -> bool {
    (0..edge_pieces().len())
        .map(PieceRef::Edge)
        .chain((0..corner_pieces().len()).map(PieceRef::Corner))
        .all(|p| {
            let (slot, ori) = locate(state, p);
            slot == p.home_slot() && ori == 0
        })
}

/// Run the full layer-by-layer solve on a scrambled state, returning the
/// moves per phase. The state is left fully solved.
fn solve_layers(state: &mut State) -> Vec<(Phase, Vec<Move>)> {
    let mut out: Vec<(Phase, Vec<Move>)> = Vec::new();

    // Stage 1: white cross — all four white edges home simultaneously (the
    // cross is never more than 8 moves from any position).
    let cross = white_edge_refs();
    {
        let goals: Vec<Goal> = cross.iter().map(|&r| Goal::home_exact(r)).collect();
        let sol = search(state, &cross, &goals, 8, &all_move_indices())
            .expect("white cross must be solvable within 8 moves");
        apply_all(state, &sol);
        out.push((Phase::Cross, sol));
    }

    // Stage 2: bottom corners, one at a time.
    let corners = white_corner_refs();
    let mut placed: Vec<PieceRef> = Vec::new();
    for &cp in &corners {
        let mut priors = cross.clone();
        priors.extend(placed.iter().copied());
        let sol = place_piece(state, cp, &priors);
        out.push((Phase::BottomCorners, sol));
        placed.push(cp);
    }

    // Stage 3: middle edges, one at a time.
    let mut solved_mid: Vec<PieceRef> = Vec::new();
    for &me in &middle_edge_refs() {
        let mut priors = cross.clone();
        priors.extend(corners.iter().copied());
        priors.extend(solved_mid.iter().copied());
        let sol = place_piece(state, me, &priors);
        out.push((Phase::MiddleEdges, sol));
        solved_mid.push(me);
    }

    // Stage 4: last layer, via generator BFS. U turns plus one named
    // algorithm per sub-step; each goal is checked over the tracked pieces.
    let u_turns: [Vec<Move>; 3] = [
        vec![Move {
            face: 0,
            quarter: 1,
        }],
        vec![Move {
            face: 0,
            quarter: 3,
        }],
        vec![Move {
            face: 0,
            quarter: 2,
        }],
    ];

    let ll_step = |state: &mut State,
                   out: &mut Vec<(Phase, Vec<Move>)>,
                   refs: &[PieceRef],
                   goals: &[Goal],
                   gens: &[Vec<Move>],
                   depth: usize,
                   phase: Phase| {
        let sol = gen_bfs(pack(state, refs), refs, goals, gens, depth)
            .unwrap_or_else(|| panic!("{phase:?} must be reachable via generators"));
        apply_all(state, &sol);
        out.push((phase, sol));
    };

    // 4a: yellow cross — all four U edges oriented, staying in the U layer.
    // The U corners are tracked too: OLL cross permutes them within the U
    // layer, and the goal keeps them there for the later sub-steps.
    {
        let mut refs = yellow_edge_refs();
        refs.extend(yellow_corner_refs());
        let mut goals: Vec<Goal> = yellow_edge_refs()
            .iter()
            .map(|_| Goal::in_slots(u_edge_slots(), false))
            .collect();
        goals.extend(
            yellow_corner_refs()
                .iter()
                .map(|_| Goal::in_slots(u_corner_slots(), true)),
        );
        let mut gens: Vec<Vec<Move>> = u_turns.to_vec();
        gens.push(algs::OLL_CROSS.to_vec());
        ll_step(state, &mut out, &refs, &goals, &gens, 6, Phase::LastCross);
    }

    // 4b: orient corners — all yellow up, any U slot.
    {
        let refs = yellow_corner_refs();
        let goals: Vec<Goal> = refs
            .iter()
            .map(|_| Goal::in_slots(u_corner_slots(), false))
            .collect();
        let mut gens: Vec<Vec<Move>> = u_turns.to_vec();
        gens.push(algs::SUNE.to_vec());
        ll_step(state, &mut out, &refs, &goals, &gens, 8, Phase::LastOrient);
    }

    // 4c: permute corners home. The U edges are tracked too so the goal
    // keeps them inside the U layer (A perm preserves them regardless).
    {
        let mut refs = yellow_corner_refs();
        refs.extend(yellow_edge_refs());
        let mut goals: Vec<Goal> = yellow_corner_refs()
            .iter()
            .map(|&r| Goal::home_exact(r))
            .collect();
        goals.extend(
            yellow_edge_refs()
                .iter()
                .map(|_| Goal::in_slots(u_edge_slots(), true)),
        );
        let mut gens: Vec<Vec<Move>> = u_turns.to_vec();
        gens.push(algs::A_PERM.to_vec());
        ll_step(
            state,
            &mut out,
            &refs,
            &goals,
            &gens,
            6,
            Phase::LastPermCorners,
        );
    }

    // 4d: permute edges home. Corners are tracked and must end home too —
    // otherwise a trailing U turn could "solve" the edges while twisting
    // the corner layer out of alignment.
    {
        let mut refs = yellow_edge_refs();
        refs.extend(yellow_corner_refs());
        let mut goals: Vec<Goal> = yellow_edge_refs()
            .iter()
            .map(|&r| Goal::home_exact(r))
            .collect();
        goals.extend(yellow_corner_refs().iter().map(|&r| Goal::home_exact(r)));
        let mut gens: Vec<Vec<Move>> = u_turns.to_vec();
        gens.push(algs::UA_PERM.to_vec());
        ll_step(
            state,
            &mut out,
            &refs,
            &goals,
            &gens,
            6,
            Phase::LastPermEdges,
        );
    }

    assert!(is_solved(state), "solve must end with a solved cube");
    out
}

// MARK: Cubie meshes

/// Cubie body half-size; the gap between cubies reads as the dark frame.
const CUBIE_HALF: f64 = 0.46;
/// Sticker quad half-size.
const STICKER_HALF: f64 = 0.35;
/// How far the sticker quads sit above the cubie body.
const STICKER_LIFT: f64 = 0.005;

/// Append a double-sided quad (4 vertices, 4 triangles) to the mesh buffers.
fn push_quad(
    points: &mut Vec<DVec3>,
    colors: &mut Vec<Rgba>,
    indices: &mut Vec<u32>,
    corners: [DVec3; 4],
    color: Rgba,
) {
    let base = points.len() as u32;
    points.extend(corners);
    colors.extend([color; 4]);
    indices.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
    indices.extend([base + 2, base + 1, base, base + 3, base + 2, base]);
}

/// Build the mesh of the cubie at grid position `grid`: a dark body plus one
/// raised colored sticker quad per outward-facing side.
fn cubie_mesh(grid: IVec) -> MeshItem {
    let center = to_dvec(grid);
    let body: Rgba = manim::BLACK.into();

    let mut points = Vec::new();
    let mut colors = Vec::new();
    let mut indices = Vec::new();

    // The two in-plane axes for each world axis.
    let in_plane: [(IVec, IVec); 3] = [
        ([0, 1, 0], [0, 0, 1]),
        ([1, 0, 0], [0, 0, 1]),
        ([1, 0, 0], [0, 1, 0]),
    ];

    for axis_i in 0..3 {
        for &sign in &[-1i32, 1] {
            let mut normal = [0; 3];
            normal[axis_i] = sign;
            let n = to_dvec(normal);
            let (e1, e2) = in_plane[axis_i];
            let (e1, e2) = (to_dvec(e1), to_dvec(e2));

            // Body face.
            let c = center + n * CUBIE_HALF;
            push_quad(
                &mut points,
                &mut colors,
                &mut indices,
                [
                    c - e1 * CUBIE_HALF - e2 * CUBIE_HALF,
                    c + e1 * CUBIE_HALF - e2 * CUBIE_HALF,
                    c + e1 * CUBIE_HALF + e2 * CUBIE_HALF,
                    c - e1 * CUBIE_HALF + e2 * CUBIE_HALF,
                ],
                body,
            );

            // Sticker on outward-facing sides.
            if grid[axis_i] == sign {
                let face = FACE_NORMALS.iter().position(|&n| n == normal).unwrap();
                let color: Rgba = FACE_COLORS[face].into();
                let c = center + n * (CUBIE_HALF + STICKER_LIFT);
                push_quad(
                    &mut points,
                    &mut colors,
                    &mut indices,
                    [
                        c - e1 * STICKER_HALF - e2 * STICKER_HALF,
                        c + e1 * STICKER_HALF - e2 * STICKER_HALF,
                        c + e1 * STICKER_HALF + e2 * STICKER_HALF,
                        c - e1 * STICKER_HALF + e2 * STICKER_HALF,
                    ],
                    color,
                );
            }
        }
    }

    let mut mesh = MeshItem::from_indexed_vertices(points, indices);
    mesh.vertex_colors = colors.into();
    mesh
}

/// One cubie: its current mesh state, its logical grid position, and its own
/// animation sequence on the shared timeline.
struct Cubie {
    grid: IVec,
    mesh: Transformed<MeshItem, Rigid>,
    seq: AnimSequence,
}

/// One net sticker square with its own animation sequence.
struct NetSticker {
    item: VItem,
    seq: AnimSequence,
}

/// Rotate a cubie's mesh around `axis` (through the cube center at the
/// origin) by `angle * alpha` — the 3D counterpart of one face turn.
struct CubieTurn {
    src: Transformed<MeshItem, Rigid>,
    axis: DVec3,
    angle: f64,
}

impl Eval for CubieTurn {
    type Output = Transformed<MeshItem, Rigid>;

    fn eval_alpha(&self, alpha: f64) -> Self::Output {
        let mut out = self.src.clone();
        out.transform =
            Rigid::from_axis_angle(self.axis, self.angle * alpha).compose(&self.src.transform);
        out
    }
}

// MARK: Scene

/// Net sticker edge length.
const NET_SIZE: f64 = 0.38;
/// Fixed xorshift seed for the scramble (deterministic video). Chosen with
/// the `seed_scan::scan_seeds` tool for a photogenic solve where every
/// phase does visible work.
const SCRAMBLE_SEED: u64 = 0x7b;
/// Scramble length in HTM.
const SCRAMBLE_LEN: usize = 20;

/// Build a text group from a typst string, scaled to height `h`.
///
/// The svg is compiled directly instead of going through `TypstText`: at
/// this pin its constructor asserts the source's *byte* length against the
/// glyph count, which always fails for non-ASCII text (CJK included).
fn text_items(s: &str, h: f64, color: AlphaColor<Srgb>) -> Vec<VItem> {
    let mut items = Vec::<VItem>::from(SvgItem::new(typst_svg(s)));
    items.scale_to(ScaleHint::PorportionalY(h));
    for it in items.iter_mut() {
        it.set_fill_color(color);
        it.set_stroke_width(0.0);
    }
    items
}

/// The golden frame that follows the turned face across the net.
struct NetFrame {
    item: VItem,
    seq: AnimSequence,
}

/// The shared timeline: every item group owns one full-lifecycle
/// [`AnimSequence`]; the timeline helpers keep them all in lockstep.
struct Timeline {
    cubies: Vec<Cubie>,
    stickers: Vec<NetSticker>,
    /// Item groups advanced by the shared clock (face letters, net frame).
    extra: Vec<AnimSequence>,
    /// Text groups. Each carries its complete lifecycle (fade in, hold,
    /// fade out at absolute times) from the moment of creation, so the
    /// global clock must never append to them — appended dead time past
    /// their fade out would stretch the scene past the camera's end.
    texts: Vec<AnimSequence>,
    frame: Option<NetFrame>,
    state: State,
    clock: f64,
    /// Camera-aligned basis (screen right/up) and the net's center; every
    /// flat item (texts, net frame) is billboarded with them so they read
    /// correctly under the perspective camera.
    net_center: DVec3,
    right: DVec3,
    up: DVec3,
}

impl Timeline {
    fn hold(&mut self, secs: f64) {
        for c in &mut self.cubies {
            c.seq.hold(secs);
        }
        for s in &mut self.stickers {
            s.seq.hold(secs);
        }
        for e in &mut self.extra {
            e.hold(secs);
        }
        if let Some(f) = &mut self.frame {
            f.seq.hold(secs);
        }
        self.clock += secs;
    }

    /// Play one face turn on the 3D cube and the net, advancing the clock.
    fn turn(&mut self, mv: Move, dur: f64) {
        let axis_ivec = FACE_NORMALS[mv.face as usize];
        let axis = to_dvec(axis_ivec);
        let angle = mv.angle();

        for cubie in &mut self.cubies {
            if idot(cubie.grid, axis_ivec) == 1 {
                let anim = CubieTurn {
                    src: cubie.mesh.clone(),
                    axis,
                    angle,
                };
                cubie.seq.push(
                    anim.apply_to(&mut cubie.mesh)
                        .with_duration(dur)
                        .with_rate_func(smooth),
                );
                cubie.grid = rot90(cubie.grid, axis_ivec, mv.quarter as i32);
            } else {
                cubie.seq.hold(dur);
            }
        }

        let old = self.state;
        apply_move(&mut self.state, mv);
        for (i, sticker) in self.stickers.iter_mut().enumerate() {
            let (face, idx) = (i / 9, i % 9);
            if self.state[face][idx] == old[face][idx] {
                sticker.seq.hold(dur);
            } else {
                // Keep the old color through most of the turn, flip at the end.
                let color = FACE_COLORS[self.state[face][idx]];
                sticker.seq.hold(dur * 0.6);
                sticker.seq.push(
                    sticker
                        .item
                        .morph(move |it| {
                            it.set_fill_color(color);
                        })
                        .with_duration(dur * 0.4),
                );
            }
        }

        if let Some(f) = &mut self.frame {
            let center = net_block_center(mv.face as usize, self.net_center, self.right, self.up);
            f.seq.hold(dur * 0.4);
            f.seq.push(
                f.item
                    .morph(move |it| {
                        it.move_to(center);
                    })
                    .with_duration(dur * 0.6),
            );
        }

        for e in &mut self.extra {
            e.hold(dur);
        }
        self.clock += dur;
    }

    /// Add a text group that fades in now, holds for `life`, fades out.
    /// The glyphs are billboarded onto the camera-aligned plane so they read
    /// upright under the perspective camera.
    fn text(&mut self, s: &str, h: f64, pos: DVec3, color: AlphaColor<Srgb>, fade: f64, life: f64) {
        let mut items = text_items(s, h, color);
        items.apply(DAffine3::from_cols(
            self.right,
            self.up,
            self.right.cross(self.up),
            DVec3::ZERO,
        ));
        items.move_to(pos);
        let mut seq = AnimSequence::new();
        seq.forward_to(self.clock);
        seq.push(items.fade_in().with_duration(fade).with_rate_func(linear));
        seq.hold(life);
        seq.push(items.fade_out().with_duration(fade).with_rate_func(linear));
        self.texts.push(seq);
    }
}

/// Center of a face's 3×3 block on the net.
fn net_block_center(face: usize, net_center: DVec3, right: DVec3, up: DVec3) -> DVec3 {
    let (bc, br) = NET_BLOCKS[face];
    let x = (bc + 1) as f64 * NET_SIZE + NET_SIZE / 2.0 - 6.0 * NET_SIZE;
    let y = 4.5 * NET_SIZE - ((br + 1) as f64 * NET_SIZE + NET_SIZE / 2.0);
    net_center + right * x + up * y
}

#[scene(clear_color = "#101218")]
#[output(dir = "./output/rubiks_cube", fps = 60)]
fn rubiks_cube(r: &mut RanimScene) {
    // Perspective camera in the (+X, +Y, +Z) octant; the look-at target is
    // shifted along the screen-right axis so the cube sits on the left half
    // of the frame and the net on the right half.
    let phi = 62.0_f64.to_radians();
    let theta = 45.0_f64.to_radians();
    let distance = 12.0;
    let mut cam = CameraFrame::from_spherical(phi, theta, distance);
    let screen_right = cam.facing.cross(DVec3::Z).normalize();
    let screen_up = screen_right.cross(cam.facing).normalize();
    let target = screen_right * 2.1;
    cam.set_spherical(phi, theta, distance, target);
    cam.fovy = 0.62;

    // Real solve: scramble + layer-by-layer solver, up front.
    let scramble = scramble_moves(SCRAMBLE_LEN, SCRAMBLE_SEED);
    let mut work = solved_state();
    apply_all(&mut work, &scramble);
    let phases = solve_layers(&mut work);
    let solve_total: usize = phases.iter().map(|(_, ms)| ms.len()).sum();

    let net_center = target + screen_right * 4.35;
    // Long captions live on the frame's center axis so they never clip at
    // the right edge under the perspective camera.
    let top_center = target + screen_up * 2.9;
    let bottom_center = target - screen_up * 3.1;

    // 3D cube: 26 cubies centered at the origin.
    let cubies: Vec<Cubie> = (-1..=1)
        .flat_map(|x| {
            (-1..=1).flat_map(move |y| {
                (-1..=1).filter_map(move |z| {
                    if x == 0 && y == 0 && z == 0 {
                        None
                    } else {
                        Some(Cubie {
                            grid: [x, y, z],
                            mesh: cubie_mesh([x, y, z]).transformed(Rigid::IDENTITY),
                            seq: AnimSequence::new(),
                        })
                    }
                })
            })
        })
        .collect();

    // Flat net in a plane facing the camera, right of the cube.
    let mut stickers: Vec<NetSticker> = Vec::with_capacity(54);
    for face in 0..6 {
        let (bc, br) = NET_BLOCKS[face];
        for idx in 0..9 {
            let (row, col) = (idx / 3, idx % 3);
            let x = (bc + col) as f64 * NET_SIZE + NET_SIZE / 2.0 - 6.0 * NET_SIZE;
            let y = 4.5 * NET_SIZE - ((br + row) as f64 * NET_SIZE + NET_SIZE / 2.0);
            let pos = net_center + screen_right * x + screen_up * y;
            let mut item = VItem::from(Square::new(NET_SIZE * 0.94));
            item.apply(DAffine3::from_cols(screen_right, screen_up, DVec3::Z, pos));
            item.set_fill_color(FACE_COLORS[face]);
            item.set_stroke_color(manim::BLACK);
            item.set_stroke_width(0.015);
            stickers.push(NetSticker {
                item,
                seq: AnimSequence::new(),
            });
        }
    }

    let mut tl = Timeline {
        cubies,
        stickers,
        extra: Vec::new(),
        texts: Vec::new(),
        frame: None,
        state: solved_state(),
        clock: 0.0,
        net_center,
        right: screen_right,
        up: screen_up,
    };
    let mut captures: Vec<(f64, &str)> = Vec::new();

    // -- Act 0: hook -------------------------------------------------------
    tl.text(
        "魔方 · 从混乱到复原",
        0.72,
        top_center,
        manim::WHITE,
        1.0,
        7.5,
    );
    tl.text(
        "4.3 × 10¹⁹ 种可能状态 · 只有 1 种是复原",
        0.34,
        top_center - screen_up * 0.85,
        manim::GREY_B,
        1.0,
        7.5,
    );

    let intro = 1.2;
    for cubie in &mut tl.cubies {
        cubie.seq.push(cubie.mesh.show().with_duration(intro));
    }
    for sticker in &mut tl.stickers {
        sticker.seq.push(sticker.item.show().with_duration(intro));
    }
    tl.clock += intro;
    tl.hold(1.0);

    // Whole-cube spin (all cubies together, 360° = identity). The cubies
    // carry the spin themselves, so the remaining groups advance by hand —
    // a full `hold` here would double-advance the cubie sequences.
    let spin_dur = 5.0;
    for cubie in &mut tl.cubies {
        let anim = CubieTurn {
            src: cubie.mesh.clone(),
            axis: DVec3::Z,
            angle: TAU,
        };
        cubie.seq.push(
            anim.apply_to(&mut cubie.mesh)
                .with_duration(spin_dur)
                .with_rate_func(smooth),
        );
    }
    for sticker in &mut tl.stickers {
        sticker.seq.hold(spin_dur);
    }
    for e in &mut tl.extra {
        e.hold(spin_dur);
    }
    tl.clock += spin_dur;
    captures.push((tl.clock - 2.0, "hook.png"));
    tl.hold(1.5);

    // -- Act 1: anatomy ----------------------------------------------------
    tl.text(
        "认识魔方 · 26 个小块",
        0.5,
        top_center,
        manim::YELLOW_C,
        0.8,
        12.0,
    );

    // Face letters on the net centers (persist to the end).
    for (face, letter) in FACE_LETTERS.iter().enumerate() {
        let pos = net_block_center(face, net_center, screen_right, screen_up) - cam.facing * 0.02;
        let mut items = text_items(letter, NET_SIZE * 0.6, manim::BLACK);
        items.apply(DAffine3::from_cols(
            screen_right,
            screen_up,
            screen_right.cross(screen_up),
            DVec3::ZERO,
        ));
        items.move_to(pos);
        let mut seq = AnimSequence::new();
        seq.forward_to(tl.clock);
        seq.push(items.fade_in().with_duration(0.8).with_rate_func(linear));
        tl.extra.push(seq);
    }
    tl.hold(2.0);

    for (text, life) in [
        ("6 个中心块 · 定住每面的颜色，永不动", 2.4),
        ("12 条棱块 · 两色，位置和朝向都要对", 2.4),
        ("8 个角块 · 三色，最难缠的部分", 2.4),
    ] {
        tl.text(text, 0.36, bottom_center, manim::WHITE, 0.5, life);
        tl.hold(3.0);
    }
    tl.text(
        "策略：分层推进 · 每层完成后永不被破坏",
        0.42,
        bottom_center,
        manim::YELLOW_C,
        0.7,
        2.6,
    );
    tl.hold(3.4);
    captures.push((tl.clock - 1.0, "anatomy.png"));

    // -- Act 2: scramble ---------------------------------------------------
    tl.text(
        &format!("打乱 {SCRAMBLE_LEN} 步"),
        0.46,
        top_center,
        manim::WHITE,
        0.6,
        11.0,
    );
    tl.hold(1.2);

    // Highlight frame for the turned face; follows every turn from here on.
    {
        let start_center = net_block_center(
            scramble[0].face as usize,
            net_center,
            screen_right,
            screen_up,
        );
        let mut item = VItem::from(Square::new(NET_SIZE * 3.12));
        item.apply(DAffine3::from_cols(
            screen_right,
            screen_up,
            screen_right.cross(screen_up),
            DVec3::ZERO,
        ));
        item.move_to(start_center);
        item.set_stroke_color(manim::YELLOW_C);
        item.set_stroke_width(0.05);
        item.set_fill_opacity(0.0);
        let mut seq = AnimSequence::new();
        seq.forward_to(tl.clock);
        seq.push(item.fade_in().with_duration(0.5).with_rate_func(linear));
        tl.frame = Some(NetFrame { item, seq });
    }

    for &mv in &scramble {
        let dur = if mv.quarter == 2 { 0.7 } else { 0.55 };
        tl.turn(mv, dur);
    }
    captures.push((tl.clock, "scrambled.png"));
    tl.hold(1.6);

    // -- Act 3: solve ------------------------------------------------------
    let mut last_umbrella = -1i32;
    for (phase, moves) in &phases {
        let umbrella = match phase {
            Phase::Cross => 0,
            Phase::BottomCorners => 1,
            Phase::MiddleEdges => 2,
            Phase::LastCross => 3,
            _ => -1,
        };
        if umbrella >= 0 && umbrella != last_umbrella {
            let title = match umbrella {
                0 => "求解 · 第 1 步 白色十字",
                1 => "求解 · 第 2 步 底层角块",
                2 => "求解 · 第 3 步 中层棱块",
                _ => "求解 · 第 4 步 顶层",
            };
            tl.text(title, 0.55, top_center, manim::YELLOW_C, 0.7, 2.0);
            tl.hold(2.6);
            last_umbrella = umbrella;
        } else if umbrella < 0 {
            tl.text(phase.title(), 0.4, top_center, manim::GREY_B, 0.5, 1.2);
            tl.hold(1.6);
        }

        if *phase == Phase::MiddleEdges && captures.len() == 4 {
            // Hero frame: mid-solve, net and cube both busy.
            captures.push((tl.clock + 1.5, "preview.png"));
        }

        let dur = match phase {
            Phase::Cross => 0.62,
            Phase::BottomCorners => 0.5,
            Phase::MiddleEdges => 0.46,
            _ => 0.36,
        };
        for &mv in moves {
            tl.turn(mv, dur);
        }
        if *phase == Phase::Cross {
            captures.push((tl.clock, "cross.png"));
        }
        tl.hold(0.7);
    }

    // Retire the highlight frame once the solve is done.
    if let Some(f) = &mut tl.frame {
        f.seq
            .push(f.item.fade_out().with_duration(0.6).with_rate_func(linear));
    }

    // -- Act 4: outro ------------------------------------------------------
    let count = |p: Phase| -> usize {
        phases
            .iter()
            .filter(|(q, _)| *q == p)
            .map(|(_, ms)| ms.len())
            .sum()
    };
    let detail = format!(
        "十字 {} 步 · 底角 {} 步 · 中层 {} 步 · 顶层 {} 步",
        count(Phase::Cross),
        count(Phase::BottomCorners),
        count(Phase::MiddleEdges),
        count(Phase::LastCross)
            + count(Phase::LastOrient)
            + count(Phase::LastPermCorners)
            + count(Phase::LastPermEdges),
    );
    tl.text(
        &format!("打乱 {SCRAMBLE_LEN} 步 · 求解 {solve_total} 步 · 复原"),
        0.6,
        top_center,
        manim::YELLOW_C,
        0.8,
        4.7,
    );
    tl.hold(1.2);
    tl.text(
        &detail,
        0.36,
        top_center - screen_up * 0.78,
        manim::GREY_B,
        0.8,
        3.0,
    );
    tl.hold(1.2);
    tl.text(
        "分而治之：每一步都只动还没完成的部分",
        0.44,
        bottom_center,
        manim::WHITE,
        0.8,
        2.3,
    );
    tl.hold(2.6);
    captures.push((tl.clock - 0.35, "end.png"));
    // Tail pad: keeps the last rendered frame strictly inside the timeline.
    // With a duration that is an exact multiple of the frame time, the final
    // frame of `ranim output` lands on the timeline end where every cell
    // (including the camera) has already ended (D0002).
    tl.hold(0.13);

    // Compose everything on one timeline.
    let total = tl.clock;
    let end = total + 1.5;
    if cfg!(test) {
        eprintln!("[scene] timeline total = {total:.4}");
    }
    let mut content = AnimStack::new();
    for mut cubie in tl.cubies {
        cubie.seq.hold_to(end);
        content.push(cubie.seq);
    }
    for mut sticker in tl.stickers {
        sticker.seq.hold_to(end);
        content.push(sticker.seq);
    }
    for mut e in tl.extra {
        e.hold_to(end);
        content.push(e);
    }
    // Text sequences are already complete; extending them would only add
    // invisible dead time past the camera's end.
    for t in tl.texts {
        content.push(t);
    }
    if let Some(mut f) = tl.frame {
        f.seq.hold_to(end);
        content.push(f.seq);
    }

    // The tail renders as a short frozen ending beat on the solved cube.
    // The camera and the content share this end so nothing pops out of the
    // frame while the last texts are fading; the extra length also absorbs
    // the net frame's fade out, which can spill past `total`.
    r.play(cam.show().with_duration(end));
    r.play(content);

    // Marks must be inserted in timeline order for the capture pass.
    captures.sort_by(|a, b| a.0.total_cmp(&b.0));
    for (t, name) in captures {
        r.insert_time_mark(t.min(total - 0.2), TimeMark::Capture(name.to_string()));
    }
}

// MARK: Tests

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rot90_quarter_turns() {
        // +90° around Z maps F → R and R → B.
        assert_eq!(rot90([1, 0, 0], [0, 0, 1], 1), [0, 1, 0]);
        assert_eq!(rot90([0, 1, 0], [0, 0, 1], 1), [-1, 0, 0]);
        // 180° around Y maps F → B.
        assert_eq!(rot90([1, 0, 0], [0, 1, 0], 2), [-1, 0, 0]);
        // -90° around X maps U → R.
        assert_eq!(rot90([0, 0, 1], [1, 0, 0], 3), [0, 1, 0]);
    }

    #[test]
    fn move_inverse_is_identity() {
        for mv in ALL_MOVES {
            let mut st = solved_state();
            apply_move(&mut st, mv);
            apply_move(&mut st, mv.inverse());
            assert_eq!(st, solved_state(), "{mv:?} inverse must restore");
        }
    }

    #[test]
    fn cubing_u_turn_cycles_front_to_left() {
        // Cubing U (clockwise from the top) = quarter 3: front top row →
        // left face, and the U face itself is unchanged.
        let mut st = solved_state();
        apply_move(
            &mut st,
            Move {
                face: 0,
                quarter: 3,
            },
        );
        assert_eq!(st[0], [0; 9], "U face unchanged");
        for i in 0..3 {
            // The L face top row now holds what F had.
            assert_eq!(st[4][i], 2, "L top row {i} must come from F");
        }
    }

    #[test]
    fn centers_never_move() {
        let scramble = scramble_moves(30, 0xDEAD_BEEF);
        let mut st = solved_state();
        apply_all(&mut st, &scramble);
        for f in 0..6 {
            assert_eq!(st[f][4], f, "center of face {f} must stay");
        }
    }

    #[test]
    fn slots_and_pieces_consistent() {
        assert_eq!(edge_slots().len(), 12);
        assert_eq!(corner_slots().len(), 8);
        assert_eq!(edge_pieces().len(), 12);
        assert_eq!(corner_pieces().len(), 8);
        let st = solved_state();
        for k in 0..12 {
            let p = PieceRef::Edge(k);
            let (slot, ori) = locate(&st, p);
            assert_eq!(slot, p.home_slot(), "edge {k} must start home");
            assert_eq!(ori, 0, "edge {k} must start oriented");
        }
        for k in 0..8 {
            let p = PieceRef::Corner(k);
            let (slot, ori) = locate(&st, p);
            assert_eq!(slot, p.home_slot(), "corner {k} must start home");
            assert_eq!(ori, 0, "corner {k} must start oriented");
        }
    }

    /// All pieces below the last layer, home and oriented.
    fn lower_layers_solved(st: &State) -> bool {
        white_edge_refs()
            .iter()
            .chain(white_corner_refs().iter())
            .chain(middle_edge_refs().iter())
            .all(|&p| {
                let (slot, ori) = locate(st, p);
                slot == p.home_slot() && ori == 0
            })
    }

    #[test]
    fn ll_generators_behave() {
        // Each generator must preserve both lower layers, and do its named job.
        let oll = apply_gen(solved_state(), &algs::OLL_CROSS);
        assert!(lower_layers_solved(&oll));
        let flipped = yellow_edge_refs()
            .iter()
            .filter(|&&p| {
                let (_, ori) = locate(&oll, p);
                ori == 1
            })
            .count();
        assert_eq!(flipped, 2, "OLL cross must flip exactly two U edges");
        for &p in &yellow_corner_refs() {
            let (slot, _) = locate(&oll, p);
            assert!(
                u_corner_slots() >> slot & 1 == 1,
                "OLL cross must keep corners in the U layer"
            );
        }

        let sune = apply_gen(solved_state(), &algs::SUNE);
        assert!(lower_layers_solved(&sune));
        for &p in &yellow_edge_refs() {
            let (slot, ori) = locate(&sune, p);
            assert_eq!(ori, 0, "sune must keep U edges oriented");
            assert!(u_edge_slots() >> slot & 1 == 1, "sune keeps U edges in U");
        }

        let aperm = apply_gen(solved_state(), &algs::A_PERM);
        assert!(lower_layers_solved(&aperm));
        let home_corners = yellow_corner_refs()
            .iter()
            .filter(|&&p| {
                let (slot, _) = locate(&aperm, p);
                slot == p.home_slot()
            })
            .count();
        assert_eq!(home_corners, 1, "A perm must fix exactly one corner");
        for &p in &yellow_corner_refs() {
            let (_, ori) = locate(&aperm, p);
            assert_eq!(ori, 0, "A perm must preserve corner orientation");
        }

        let ua = apply_gen(solved_state(), &algs::UA_PERM);
        assert!(lower_layers_solved(&ua));
        for &p in &yellow_corner_refs() {
            let (slot, ori) = locate(&ua, p);
            assert_eq!(slot, p.home_slot(), "ua must keep corners in place");
            assert_eq!(ori, 0, "ua must keep corners oriented");
        }
        let home_edges = yellow_edge_refs()
            .iter()
            .filter(|&&p| {
                let (slot, _) = locate(&ua, p);
                slot == p.home_slot()
            })
            .count();
        assert_eq!(home_edges, 1, "ua must fix exactly one edge");
        for &p in &yellow_edge_refs() {
            let (_, ori) = locate(&ua, p);
            assert_eq!(ori, 0, "ua must preserve edge orientation");
        }
    }

    fn apply_gen(mut st: State, algo: &[Move]) -> State {
        apply_all(&mut st, algo);
        st
    }

    /// Assert the invariant of each phase as the solution is replayed.
    /// Cumulative stages are placed one piece per phase entry, so their
    /// full invariant only holds once the last entry of the stage played.
    #[derive(Default)]
    struct ReplayProgress {
        corners: usize,
        mids: usize,
    }

    fn assert_phase_invariants(st: &State, phase: Phase, prog: &mut ReplayProgress) {
        match phase {
            Phase::Cross => {
                for &p in &white_edge_refs() {
                    let (slot, ori) = locate(st, p);
                    assert_eq!(slot, p.home_slot(), "cross edge home after cross");
                    assert_eq!(ori, 0, "cross edge oriented after cross");
                }
            }
            Phase::BottomCorners => {
                prog.corners += 1;
                if prog.corners == white_corner_refs().len() {
                    for &p in white_edge_refs().iter().chain(white_corner_refs().iter()) {
                        let (slot, ori) = locate(st, p);
                        assert_eq!(slot, p.home_slot(), "first layer home after corners");
                        assert_eq!(ori, 0);
                    }
                }
            }
            Phase::MiddleEdges => {
                prog.mids += 1;
                if prog.mids == middle_edge_refs().len() {
                    assert!(lower_layers_solved(st), "two layers home after mid edges");
                }
            }
            Phase::LastPermEdges => {
                assert!(is_solved(st), "cube solved after the last phase");
            }
            _ => {}
        }
    }

    fn solve_and_check(seed: u64) {
        let scramble = scramble_moves(20, seed);
        let mut st = solved_state();
        apply_all(&mut st, &scramble);
        let phases = solve_layers(&mut st);

        // Replay and assert every phase's invariant.
        let mut replay = solved_state();
        apply_all(&mut replay, &scramble);
        let mut prog = ReplayProgress::default();
        for (phase, moves) in &phases {
            apply_all(&mut replay, moves);
            assert_phase_invariants(&replay, *phase, &mut prog);
        }

        // Sanity: cross optimal (≤ 8), every insertion bounded, non-trivial solve.
        assert!(phases[0].1.len() <= 8);
        let total: usize = phases.iter().map(|(_, ms)| ms.len()).sum();
        assert!(total > 15, "a real 20-move scramble needs a real solution");
    }

    #[test]
    fn packed_transitions_match_real_moves() {
        let mut st = solved_state();
        apply_all(&mut st, &scramble_moves(15, 7));
        let refs = [
            PieceRef::Edge(0),
            PieceRef::Edge(3),
            PieceRef::Edge(7),
            PieceRef::Corner(2),
            PieceRef::Corner(4),
        ];
        let p0 = pack(&st, &refs);
        for mi in 0..ALL_MOVES.len() {
            let mut st2 = st;
            apply_move(&mut st2, ALL_MOVES[mi]);
            assert_eq!(
                pack(&st2, &refs),
                expand_packed(p0, mi, &refs),
                "move {mi:?} must transform tracked pieces identically"
            );
        }
    }

    #[test]
    fn solver_end_to_end_debug() {
        for seed in [1, 2, 3] {
            solve_and_check(seed);
        }
    }

    #[test]
    #[cfg_attr(debug_assertions, ignore = "heavy: run under cargo test --release")]
    fn solver_end_to_end_release() {
        for seed in 10..40 {
            solve_and_check(seed);
        }
    }

    #[test]
    fn video_scramble_solves() {
        // The exact scramble used by the scene.
        let scramble = scramble_moves(SCRAMBLE_LEN, SCRAMBLE_SEED);
        let mut st = solved_state();
        apply_all(&mut st, &scramble);
        let phases = solve_layers(&mut st);
        assert!(is_solved(&st));
        let total: usize = phases.iter().map(|(_, ms)| ms.len()).sum();
        // Deterministic smoke numbers for the delivery notes.
        println!("video scramble solve: {total} moves");
        for (phase, moves) in &phases {
            println!("  {phase:?}: {} moves", moves.len());
        }
    }
}

#[cfg(test)]
mod seed_scan {
    use super::*;

    #[test]
    #[ignore = "manual tool: scan seeds for a photogenic solve"]
    fn scan_seeds() {
        for seed in [0x2026_0917, 1, 2, 3, 42, 77, 123, 2026, 9999, 31337] {
            let scramble = scramble_moves(20, seed);
            let mut st = solved_state();
            apply_all(&mut st, &scramble);
            let phases = solve_layers(&mut st);
            let total: usize = phases.iter().map(|(_, ms)| ms.len()).sum();
            let counts: Vec<usize> = phases.iter().map(|(_, ms)| ms.len()).collect();
            println!("seed {seed:#x}: total {total} phases {counts:?}");
        }
    }
}

#[cfg(test)]
mod capture_probe {
    use super::*;
    use ranim::core::core_item::CoreItem;

    /// Regression guard for the D0002 renderer panic: every sampled frame
    /// that still carries items must carry exactly one active CameraFrame.
    /// (A lifecycle sequence that outlives the camera cell — e.g. from dead
    /// time appended after its fade out — aborts the render at that frame.)
    #[test]
    fn every_rendered_frame_has_exactly_one_camera() {
        let mut scene = RanimScene::new();
        rubiks_cube(&mut scene);
        let sealed = scene.seal();
        assert!(
            sealed.total_secs() < 123.0,
            "scene stretched to {:.2}s — some sequence outlives the timeline",
            sealed.total_secs()
        );
        let total = sealed.total_secs();
        let mut evaluator = sealed.into_evaluator(120.0);
        let mut frame = Vec::new();
        evaluator.sample_at(total + 4.9, &mut frame);
        let mut camera = 0usize;
        let mut vitem = 0usize;
        let mut mesh = 0usize;
        for (_, item) in &frame {
            match item {
                CoreItem::CameraFrame(_) => camera += 1,
                CoreItem::MeshItem(_) => mesh += 1,
                CoreItem::VItem(_) => vitem += 1,
            }
        }
        println!(
            "[probe] at total+4.9: items={frame_len} camera={camera} vitem={vitem} mesh={mesh}",
            frame_len = frame.len()
        );
        let mut last_nonempty = (0.0f64, 0usize);
        let mut tt = 0.0f64;
        while tt <= total + 15.0 {
            let mut f2 = Vec::new();
            evaluator.sample_at(tt, &mut f2);
            if !f2.is_empty() {
                last_nonempty = (tt, f2.len());
            }
            tt += 0.1;
        }
        println!(
            "[probe] last non-empty frame at t={:.2} ({} items)",
            last_nonempty.0, last_nonempty.1
        );
        let mut frame = Vec::new();
        let mut t = 0.0f64;
        while t <= total {
            evaluator.sample_at(t, &mut frame);
            let cameras = frame
                .iter()
                .filter(|(_, item)| matches!(item, CoreItem::CameraFrame(_)))
                .count();
            assert!(
                !(frame.is_empty() && cameras == 0) || frame.is_empty(),
                "frame at {t:.2}: {} items with {} cameras",
                frame.len(),
                cameras
            );
            assert_eq!(
                cameras,
                if frame.is_empty() { 0 } else { 1 },
                "frame at {t:.2} must carry exactly one camera"
            );
            frame.clear();
            t += 0.25;
        }
    }
}
