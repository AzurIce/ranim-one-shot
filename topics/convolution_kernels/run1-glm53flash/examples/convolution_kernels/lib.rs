//! convolution_kernels — 用几种常用卷积核对图像做卷积的讲解动画。
//!
//! 场景只做数据的视图：像素网格、扫描窗口与输出显现全部由
//! `convolution_kernels` 库里的真实卷积输出驱动。每个物件一条
//! 生命周期序列（fade in → 参与 → fade out → hold_to(TOTAL)），
//! 共享时钟对齐；扫描窗口与输出显现是两个自定义 Eval。

use ::convolution_kernels::{
    KERNELS, KernelDef, N, Worked, convolve, display_value, kernel_cell_text, source_image,
    worked_example,
};
use ranim::{
    anims::{fading::FadingAnim, morph::MorphAnim},
    color::{AlphaColor, Srgb, palettes::manim, rgb8},
    glam::{DVec3, dvec3},
    items::vitem::{
        VItem,
        geometry::{Rectangle, Square},
        text::{TextFont, TextItem},
    },
    prelude::*,
    utils::rate_functions::{linear, smooth},
};

// ---- 全局时间轴（秒）----
const T_HOOK: f64 = 0.0;
const T_NUMBERS: f64 = 8.0;
const T_MECH: f64 = 20.0;
const T_SCAN: f64 = 38.0;
const ACT_SCAN: f64 = 9.5;
const N_KERNEL_ACTS: f64 = 5.0;
const T_SUMMARY: f64 = T_SCAN + N_KERNEL_ACTS * ACT_SCAN;
const TOTAL: f64 = T_SUMMARY + 13.0;

const FONT_FAMILIES: [&str; 3] = ["HarmonyOS Sans SC", "Noto Sans CJK SC", "LXGW WenKai"];
const PIXEL_STROKE: AlphaColor<Srgb> = rgb8(16, 16, 22);

/// 各核的强调色，顺序与 [`KERNELS`] 一致。
const ACCENTS: [AlphaColor<Srgb>; 5] = [
    manim::BLUE_C,
    manim::GREEN_C,
    manim::RED_C,
    manim::GOLD_C,
    manim::TEAL_C,
];

fn gray(v: f64) -> AlphaColor<Srgb> {
    let c = (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    rgb8(c, c, c)
}

/// 深色像素上用浅字，浅色像素上用深字。
fn ink_for(v: f64) -> AlphaColor<Srgb> {
    if v > 0.55 {
        rgb8(20, 20, 26)
    } else {
        rgb8(235, 235, 240)
    }
}

/// 单行文本 → 字形组（TextItem 内部走 typst，转出的 Vec 即组）。
fn text(s: &str, em: f64, color: AlphaColor<Srgb>) -> Vec<VItem> {
    Vec::<VItem>::from(
        TextItem::new(s, em)
            .with_font(TextFont::new(FONT_FAMILIES))
            .with(|t| {
                t.set_fill_color(color);
            }),
    )
}

/// 以 `base` 为网格中心时，N×N 网格第 (i, j) 格（行 0 在顶部）的格心。
fn cell_center(base: DVec3, pitch: f64, i: usize, j: usize) -> DVec3 {
    let half = (N as f64 - 1.0) / 2.0;
    dvec3(
        base.x + (j as f64 - half) * pitch,
        base.y - (i as f64 - half) * pitch,
        0.0,
    )
}

/// 5×5 网格（放大区）的格心。
fn cell_center5(base: DVec3, pitch: f64, i: usize, j: usize) -> DVec3 {
    let half = 2.0;
    dvec3(
        base.x + (j as f64 - half) * pitch,
        base.y - (i as f64 - half) * pitch,
        0.0,
    )
}

/// 3×3 网格（卷积核）的格心。
fn cell_center3(base: DVec3, pitch: f64, i: usize, j: usize) -> DVec3 {
    let half = 1.0;
    dvec3(
        base.x + (j as f64 - half) * pitch,
        base.y - (i as f64 - half) * pitch,
        0.0,
    )
}

/// 值填充的像素网格，网格中心在 `base`。
fn pixel_grid(vals: &[f64], base: DVec3, pitch: f64, size: f64, stroke_w: f32) -> Vec<VItem> {
    vals.iter()
        .enumerate()
        .map(|(idx, &v)| {
            VItem::from(Square::new(size)).with(|sq| {
                sq.set_fill_color(gray(v))
                    .set_stroke_color(PIXEL_STROKE)
                    .set_stroke_width(stroke_w)
                    .move_to(cell_center(base, pitch, idx / N, idx % N));
            })
        })
        .collect()
}

/// 只描边的空网格（输出格）。
fn outline_grid(
    n: usize,
    base: DVec3,
    pitch: f64,
    size: f64,
    color: AlphaColor<Srgb>,
    stroke_w: f32,
) -> Vec<VItem> {
    (0..n * n)
        .map(|idx| {
            VItem::from(Square::new(size)).with(|sq| {
                sq.set_stroke_color(color)
                    .set_stroke_width(stroke_w)
                    .move_to(cell_center5(base, pitch, idx / n, idx % n));
            })
        })
        .collect()
}

fn rect_at(center: DVec3, w: f64, h: f64, color: AlphaColor<Srgb>, stroke_w: f32) -> VItem {
    VItem::from(Rectangle::new(w, h)).with(|r| {
        r.set_stroke_color(color)
            .set_stroke_width(stroke_w)
            .move_to(center);
    })
}

/// 物件组的生命周期：t_in 淡入，t_out 淡出，首尾对齐共享时钟。
fn group_life(group: &[VItem], t_in: f64, t_out: f64) -> AnimSequence {
    let mut sq = AnimSequence::new();
    sq.hold_to(t_in);
    sq.push(
        group
            .to_vec()
            .fade_in()
            .with_duration(0.8)
            .with_rate_func(smooth),
    );
    sq.hold_to(t_out);
    sq.push(
        group
            .to_vec()
            .fade_out()
            .with_duration(0.8)
            .with_rate_func(smooth),
    );
    sq.hold_to(TOTAL);
    sq
}

/// Eval 叶子的生命周期：t0 开始播放，其余时间对齐共享时钟。
fn eval_life(anim: impl Unplaced + 'static, t0: f64) -> AnimSequence {
    let mut sq = AnimSequence::new();
    sq.hold_to(t0);
    sq.push(anim);
    sq.hold_to(TOTAL);
    sq
}

/// 1.0 → 0.0 的线性包络：t0 前全见，t1 后全隐。
fn envelope(t: f64, t0: f64, t1: f64) -> f64 {
    if t <= t0 {
        1.0
    } else if t >= t1 {
        0.0
    } else {
        1.0 - (t - t0) / (t1 - t0)
    }
}

/// 扫描输出网格：第 i 格在窗口扫过它的那一步淡入，整幕随包络淡出。
struct RevealEval {
    cells: Vec<VItem>,
    scan_t0: f64,
    step: f64,
    fade_t0: f64,
    fade_t1: f64,
    dur: f64,
}

impl Eval for RevealEval {
    type Output = Vec<VItem>;
    fn eval_alpha(&self, alpha: f64) -> Vec<VItem> {
        let t = alpha * self.dur;
        let g = envelope(t, self.fade_t0, self.fade_t1);
        let mut out = self.cells.clone();
        for (i, item) in out.iter_mut().enumerate() {
            let f = ((t - self.scan_t0) / self.step - i as f64).clamp(0.0, 1.0);
            item.set_opacity((f * g) as f32);
        }
        out
    }
}

/// 滑动窗口：淡入后停在 0 号格，扫描期按光栅序逐格吸附，随包络淡出。
struct WindowEval {
    rect: VItem,
    centers: Vec<DVec3>,
    in_t0: f64,
    in_t1: f64,
    scan_t0: f64,
    step: f64,
    fade_t0: f64,
    fade_t1: f64,
    dur: f64,
}

impl Eval for WindowEval {
    type Output = VItem;
    fn eval_alpha(&self, alpha: f64) -> VItem {
        let t = alpha * self.dur;
        let idx = ((((t - self.scan_t0) / self.step).floor() as i64)
            .clamp(0, self.centers.len() as i64 - 1)) as usize;
        let mut r = self.rect.clone();
        r.move_to(self.centers[idx]);
        let mut op = envelope(t, self.fade_t0, self.fade_t1);
        if t < self.in_t0 {
            op = 0.0;
        } else if t < self.in_t1 {
            op *= (t - self.in_t0) / (self.in_t1 - self.in_t0);
        }
        r.set_opacity(op as f32);
        r
    }
}

/// 序幕：源图亮相，抛出"一个 3×3 方阵，三种命运"的钩子。
fn act_hook(stack: &mut AnimStack) {
    let t = T_HOOK;
    let image = pixel_grid(&source_image(), dvec3(0.0, 0.55, 0.0), 0.55, 0.5, 0.008);
    // 三个词各自成组定位，再拼成一个组。
    let mut words: Vec<VItem> = Vec::new();
    for (i, (w, c)) in ["模糊", "锐利", "描边"]
        .iter()
        .zip([manim::BLUE_C, manim::GREEN_C, manim::RED_C])
        .enumerate()
    {
        let mut g = text(w, 0.62, c);
        g.move_to(dvec3((i as f64 - 1.0) * 2.8, -2.75, 0.0));
        words.extend(g);
    }
    let caption = text(
        "同一个 3×3 方阵，三种截然不同的效果",
        0.5,
        rgb8(225, 225, 232),
    )
    .with(|g| g.move_to(dvec3(0.0, 3.55, 0.0)).discard());
    let title = text(
        "卷积核：一个 3×3 方阵，如何改变一张图像",
        0.58,
        manim::WHITE,
    )
    .with(|g| g.move_to(dvec3(0.0, 0.0, 0.0)).discard());

    let mut image_sq = AnimSequence::new();
    image_sq.hold_to(t).push(
        image
            .clone()
            .fade_in()
            .with_duration(1.0)
            .with_rate_func(smooth),
    );
    image_sq.hold_to(t + 4.6);
    image_sq.push(
        image
            .clone()
            .fade_out()
            .with_duration(0.8)
            .with_rate_func(smooth),
    );
    image_sq.hold_to(TOTAL);

    let mut words_sq = AnimSequence::new();
    words_sq.hold_to(t + 1.4).push(
        words
            .clone()
            .fade_in()
            .with_duration(1.2)
            .with_rate_func(smooth),
    );
    words_sq.hold_to(t + 4.6);
    words_sq.push(
        words
            .clone()
            .fade_out()
            .with_duration(0.8)
            .with_rate_func(smooth),
    );
    words_sq.hold_to(TOTAL);

    let mut caption_sq = AnimSequence::new();
    caption_sq.hold_to(t + 3.2).push(
        caption
            .clone()
            .fade_in()
            .with_duration(1.0)
            .with_rate_func(smooth),
    );
    caption_sq.hold_to(t + 4.6);
    caption_sq.push(
        caption
            .clone()
            .fade_out()
            .with_duration(0.8)
            .with_rate_func(smooth),
    );
    caption_sq.hold_to(TOTAL);

    stack.extend(stack![image_sq, words_sq, caption_sq]);
    stack.push(group_life(&title, t + 5.6, t + 7.2));
}

/// 第一幕：图像 = 数字网格。放大 5×5 区域，把亮度值亮出来。
fn act_numbers(stack: &mut AnimStack, img: &[f64]) {
    let t = T_NUMBERS;
    let base = dvec3(-3.6, 0.0, 0.0);
    let (pitch, size) = (0.55, 0.5);
    let grid = pixel_grid(img, base, pitch, size, 0.008);
    let label = text("一张 10×10 的灰度图", 0.42, rgb8(210, 210, 220))
        .with(|g| g.move_to(dvec3(base.x, -3.3, 0.0)).discard());
    // 放大区：行 2..=6、列 3..=7。
    let (zr, zc) = (2usize, 3usize);
    let crop_vals: Vec<f64> = (zr..zr + 5)
        .flat_map(|i| (zc..zc + 5).map(move |j| img[i * N + j]))
        .collect();
    let highlight = rect_at(
        cell_center(base, pitch, zr + 2, zc + 2),
        5.0 * pitch,
        5.0 * pitch,
        manim::YELLOW_C,
        0.035,
    );
    let zoom_base = dvec3(3.9, 0.3, 0.0);
    let zoom = pixel_grid(&crop_vals, zoom_base, 0.98, 0.9, 0.008);
    let zoom_nums: Vec<VItem> = crop_vals
        .iter()
        .enumerate()
        .flat_map(|(idx, &v)| {
            text(&format!("{v:.2}"), 0.3, ink_for(v)).with(|g| {
                g.move_to(cell_center5(zoom_base, 0.98, idx / 5, idx % 5));
            })
        })
        .collect();
    let note = text("每个格子存一个 0 到 1 的亮度值", 0.42, rgb8(210, 210, 220))
        .with(|g| g.move_to(dvec3(zoom_base.x, -3.3, 0.0)).discard());
    let punch = text("所谓「处理图像」，就是对这些数做运算", 0.5, manim::WHITE)
        .with(|g| g.move_to(dvec3(0.0, 3.6, 0.0)).discard());

    stack.push(group_life(&grid, t, t + 11.2));
    stack.push(group_life(&label, t + 1.0, t + 11.2));
    stack.push(group_life(&[highlight], t + 2.2, t + 11.2));
    stack.push(group_life(&zoom, t + 2.6, t + 11.2));
    stack.push(group_life(&zoom_nums, t + 3.4, t + 11.2));
    stack.push(group_life(&note, t + 5.8, t + 7.4));
    stack.push(group_life(&punch, t + 7.6, t + 11.2));
}

/// 第二幕：卷积机制。以盒式模糊为例，完整算出一个输出像素，
/// 再滑动两次示意"对每个像素重复"。
fn act_mechanism(stack: &mut AnimStack, img: &[f64], worked: &Worked) {
    let t = T_MECH;
    let box_accent = ACCENTS[0];
    let (pitch, size) = (0.92, 0.86);
    let in_base = dvec3(-4.3, 0.2, 0.0);
    let out_base = dvec3(4.3, 0.2, 0.0);
    let k_base = dvec3(0.0, 1.55, 0.0);

    // 放大输入（行 2..=6、列 3..=7）：格子 + 数值。
    let (zr, zc) = (2usize, 3usize);
    let crop_vals: Vec<f64> = (zr..zr + 5)
        .flat_map(|i| (zc..zc + 5).map(move |j| img[i * N + j]))
        .collect();
    let in_grid = pixel_grid(&crop_vals, in_base, pitch, size, 0.008);
    let in_nums: Vec<VItem> = crop_vals
        .iter()
        .enumerate()
        .flat_map(|(idx, &v)| {
            text(&format!("{v:.2}"), 0.27, ink_for(v)).with(|g| {
                g.move_to(cell_center5(in_base, pitch, idx / 5, idx % 5))
                    .discard()
            })
        })
        .collect();

    // 输出网格：空格，等窗口逐个点亮。
    let out_raw = convolve(img, &KERNELS[0].kernel);
    let out_grid = outline_grid(5, out_base, pitch, size, rgb8(120, 120, 135), 0.014);
    let out_pos = |i: usize, j: usize| cell_center5(out_base, pitch, i, j);

    // 卷积核 3×3：格子 + 数值。
    let k_cells: Vec<VItem> = (0..9)
        .map(|c| {
            VItem::from(Square::new(0.68)).with(|sq| {
                sq.set_stroke_color(box_accent)
                    .set_fill_color(box_accent.with_alpha(0.14))
                    .set_stroke_width(0.02)
                    .move_to(cell_center3(k_base, 0.74, c / 3, c % 3));
            })
        })
        .collect();
    let k_nums: Vec<VItem> = KERNELS[0]
        .kernel
        .iter()
        .flatten()
        .enumerate()
        .flat_map(|(c, &v)| {
            text(&kernel_cell_text(v), 0.3, box_accent).with(|g| {
                g.move_to(cell_center3(k_base, 0.74, c / 3, c % 3))
                    .discard()
            })
        })
        .collect();

    let label_in = text("输入（放大）", 0.4, rgb8(210, 210, 220))
        .with(|g| g.move_to(dvec3(in_base.x, -2.95, 0.0)).discard());
    let label_out = text("输出", 0.4, rgb8(210, 210, 220))
        .with(|g| g.move_to(dvec3(out_base.x, -2.95, 0.0)).discard());
    let label_k =
        text("卷积核", 0.4, box_accent).with(|g| g.move_to(dvec3(k_base.x, 3.05, 0.0)).discard());

    // 算式：三行九个乘积 + 一行汇总，全部是真实数字。
    let p = worked.products;
    let row1 = format!("{:.2} + {:.2} + {:.2}", p[0], p[1], p[2]);
    let row2 = format!("+ {:.2} + {:.2} + {:.2}", p[3], p[4], p[5]);
    let row3 = format!("+ {:.2} + {:.2} + {:.2}", p[6], p[7], p[8]);
    let final_row = format!("= {:.2} ÷ 9 = {:.2}", worked.sum, worked.mean);
    let arith_rows: Vec<Vec<VItem>> = [
        text(&row1, 0.34, rgb8(225, 225, 232)),
        text(&row2, 0.34, rgb8(225, 225, 232)),
        text(&row3, 0.34, rgb8(225, 225, 232)),
        text(&final_row, 0.42, box_accent),
    ]
    .into_iter()
    .enumerate()
    .map(|(r, mut g)| {
        let y = [0.0, -0.6, -1.2, -1.9][r];
        g.move_to(dvec3(0.0, y, 0.0));
        g
    })
    .collect();

    let mut seqs: Vec<AnimSequence> = vec![
        group_life(&in_grid, t, t + 17.0),
        group_life(&in_nums, t + 0.2, t + 17.0),
        group_life(&out_grid, t + 0.3, t + 17.0),
        group_life(&k_cells, t + 0.6, t + 17.0),
        group_life(&k_nums, t + 1.4, t + 17.0),
        group_life(&label_in, t + 0.4, t + 17.0),
        group_life(&label_out, t + 0.5, t + 17.0),
        group_life(&label_k, t + 0.7, t + 17.0),
    ];
    let arith_times = [3.8, 4.6, 5.4, 6.6];
    for (row, t_in) in arith_rows.iter().zip(arith_times) {
        seqs.push(group_life(row, t + t_in, t + 17.0));
    }

    // 滑动窗口（黄色方框）：停在算例位 → 右滑一格 → 右上滑一格，
    // 始终不离开放大区。
    let worked_local = (worked.center_r - zr, worked.center_c - zc);
    let slide1 = (worked_local.0, worked_local.1 + 1);
    let slide2 = (worked_local.0 - 1, worked_local.1 + 1);
    let window = rect_at(
        cell_center5(in_base, pitch, worked_local.0, worked_local.1),
        3.0 * pitch,
        3.0 * pitch,
        manim::YELLOW_C,
        0.04,
    );
    let mut win_sq = AnimSequence::new();
    win_sq.hold_to(t + 2.8).push(
        window
            .clone()
            .fade_in()
            .with_duration(0.8)
            .with_rate_func(smooth),
    );
    win_sq.hold_to(t + 8.8);
    win_sq.push(
        window
            .clone()
            .morph(|w| {
                w.move_to(cell_center5(in_base, pitch, slide1.0, slide1.1))
                    .discard()
            })
            .with_duration(0.8)
            .with_rate_func(smooth),
    );
    win_sq.hold_to(t + 10.2);
    win_sq.push(
        window
            .clone()
            .morph(|w| {
                w.move_to(cell_center5(in_base, pitch, slide2.0, slide2.1))
                    .discard()
            })
            .with_duration(0.8)
            .with_rate_func(smooth),
    );
    win_sq.hold_to(t + 17.0);
    win_sq.push(
        window
            .clone()
            .fade_out()
            .with_duration(0.8)
            .with_rate_func(smooth),
    );
    win_sq.hold_to(TOTAL);
    seqs.push(win_sq);

    // 输出格点亮：算例格，以及窗口滑过的两格。填充值都是真实卷积输出。
    let fills: [(usize, usize, f64); 3] = [
        (worked_local.0, worked_local.1, 7.4),
        (slide1.0, slide1.1, 9.7),
        (slide2.0, slide2.1, 11.1),
    ];
    for (i, j, t_fill) in fills {
        let gr = zr + i;
        let gc = zc + j;
        let v = display_value(out_raw[gr * N + gc], false);
        let overlay = VItem::from(Square::new(size)).with(|sq| {
            sq.set_stroke_width(0.0).move_to(out_pos(i, j));
        });
        let mut sq = AnimSequence::new();
        sq.hold_to(t + t_fill);
        sq.push(
            overlay
                .clone()
                .morph(|cell| cell.set_fill_color(gray(v)).discard())
                .with_duration(0.6)
                .with_rate_func(smooth),
        );
        sq.hold_to(t + 17.0);
        sq.push(
            overlay
                .clone()
                .fade_out()
                .with_duration(0.8)
                .with_rate_func(smooth),
        );
        sq.hold_to(TOTAL);
        seqs.push(sq);
    }

    let caption = text(
        "窗口滑遍全图，每个像素都这样算；边界之外一律补 0",
        0.46,
        manim::WHITE,
    )
    .with(|g| g.move_to(dvec3(0.0, -3.65, 0.0)).discard());
    seqs.push(group_life(&caption, t + 11.8, t + 17.0));

    for sq in seqs {
        stack.push(sq);
    }
}

/// 核扫描幕：左输入、中核、右输出，窗口逐格扫过，输出逐格点亮。
fn act_kernel_scan(stack: &mut AnimStack, t0: f64, idx: usize) {
    let k: &KernelDef = &KERNELS[idx];
    let accent = ACCENTS[idx];
    let img = source_image();
    let out_raw = convolve(&img, &k.kernel);
    let out_vals: Vec<f64> = out_raw
        .iter()
        .map(|v| display_value(*v, k.signed))
        .collect();

    // 舞台参数（scan 期为本地时间，相对 t0）。
    let (pitch, size) = (0.52, 0.48);
    let in_base = dvec3(-4.35, 0.0, 0.0);
    let out_base = dvec3(4.35, 0.0, 0.0);
    let k_base = dvec3(0.0, 0.9, 0.0);
    let scan_t0 = 1.6;
    let step = 0.052;
    let fade_t0 = 8.7;
    let act_dur = ACT_SCAN;

    let in_grid = pixel_grid(&img, in_base, pitch, size, 0.007);
    let out_grid = pixel_grid(&out_vals, out_base, pitch, size, 0.007);
    let k_cells: Vec<VItem> = (0..9)
        .map(|c| {
            VItem::from(Square::new(0.6)).with(|sq| {
                sq.set_stroke_color(accent)
                    .set_fill_color(accent.with_alpha(0.14))
                    .set_stroke_width(0.02)
                    .move_to(cell_center3(k_base, 0.66, c / 3, c % 3));
            })
        })
        .collect();
    let k_nums: Vec<VItem> = k
        .kernel
        .iter()
        .flatten()
        .enumerate()
        .flat_map(|(c, &v)| {
            text(&kernel_cell_text(v), 0.26, accent).with(|g| {
                g.move_to(cell_center3(k_base, 0.66, c / 3, c % 3))
                    .discard()
            })
        })
        .collect();
    let label = text(k.label, 0.52, accent).with(|g| g.move_to(dvec3(0.0, 2.75, 0.0)).discard());
    let label_in = text("输入", 0.38, rgb8(200, 200, 210))
        .with(|g| g.move_to(dvec3(in_base.x, -3.05, 0.0)).discard());
    let label_out = text("输出", 0.38, rgb8(200, 200, 210))
        .with(|g| g.move_to(dvec3(out_base.x, -3.05, 0.0)).discard());
    let note: Vec<VItem> = if k.signed {
        text("灰度 = 响应的绝对值", 0.26, rgb8(170, 170, 185))
            .with(|g| g.move_to(dvec3(out_base.x, -3.5, 0.0)).discard())
    } else {
        Vec::new()
    };
    let caption =
        text(k.caption, 0.46, manim::WHITE).with(|g| g.move_to(dvec3(0.0, -3.7, 0.0)).discard());

    stack.push(group_life(&in_grid, t0, t0 + fade_t0));
    stack.push(group_life(&out_grid, t0, t0 + fade_t0));
    stack.push(group_life(&k_cells, t0 + 0.2, t0 + fade_t0));
    stack.push(group_life(&k_nums, t0 + 0.6, t0 + fade_t0));
    stack.push(group_life(&label, t0 + 0.1, t0 + fade_t0));
    stack.push(group_life(&label_in, t0 + 0.4, t0 + fade_t0));
    stack.push(group_life(&label_out, t0 + 0.4, t0 + fade_t0));
    if !note.is_empty() {
        stack.push(group_life(&note, t0 + 0.6, t0 + fade_t0));
    }
    stack.push(group_life(&caption, t0 + 7.0, t0 + fade_t0));

    let centers: Vec<DVec3> = (0..N * N)
        .map(|c| cell_center(in_base, pitch, c / N, c % N))
        .collect();
    let reveal = RevealEval {
        cells: out_grid,
        scan_t0,
        step,
        fade_t0,
        fade_t1: act_dur,
        dur: act_dur,
    };
    let window = WindowEval {
        rect: rect_at(
            DVec3::ZERO,
            3.0 * pitch,
            3.0 * pitch,
            manim::YELLOW_C,
            0.035,
        ),
        centers,
        in_t0: 0.6,
        in_t1: 1.2,
        scan_t0,
        step,
        fade_t0,
        fade_t1: act_dur,
        dur: act_dur,
    };
    stack.push(eval_life(
        reveal.with_duration(act_dur).with_rate_func(linear),
        t0,
    ));
    stack.push(eval_life(
        window.with_duration(act_dur).with_rate_func(linear),
        t0,
    ));
}

/// 终幕：输入与五种核结果的六联对比 + 点题。
fn act_summary(stack: &mut AnimStack, img: &[f64]) {
    let t = T_SUMMARY;
    let (pitch, size) = (0.24, 0.225);
    let cols = [-4.4, 0.0, 4.4];
    let rows_y = [1.45, -1.75];
    let mut panels: Vec<Vec<VItem>> = vec![pixel_grid(img, DVec3::ZERO, pitch, size, 0.005)];
    for k in KERNELS.iter() {
        let out = convolve(img, &k.kernel);
        let vals: Vec<f64> = out.iter().map(|v| display_value(*v, k.signed)).collect();
        panels.push(pixel_grid(&vals, DVec3::ZERO, pitch, size, 0.005));
    }
    let labels_src = ["输入", "盒式模糊", "锐化", "边缘检测", "Sobel X", "Sobel Y"];
    let mut groups: Vec<(Vec<VItem>, Vec<VItem>)> = Vec::new();
    for (p, panel) in panels.iter().enumerate() {
        let (col, row) = (p % 3, p / 3);
        let center = dvec3(cols[col], rows_y[row], 0.0);
        let placed: Vec<VItem> = panel
            .iter()
            .map(|sq| {
                let mut s = sq.clone();
                s.shift(center);
                s
            })
            .collect();
        let label_color = if p == 0 {
            rgb8(210, 210, 220)
        } else {
            ACCENTS[p - 1]
        };
        let label = text(labels_src[p], 0.34, label_color)
            .with(|g| g.move_to(dvec3(center.x, center.y - 1.42, 0.0)).discard());
        groups.push((placed, label));
    }
    let closing = text(
        "同一个滑动窗口，九个不同的权重——卷积核，就是看图的一种方式。",
        0.48,
        manim::WHITE,
    )
    .with(|g| g.move_to(dvec3(0.0, 3.55, 0.0)).discard());

    for (p, (panel, label)) in groups.iter().enumerate() {
        let t_in = t + p as f64 * 0.3;
        let mut sq = AnimSequence::new();
        sq.hold_to(t_in);
        sq.push(
            panel
                .clone()
                .fade_in()
                .with_duration(0.8)
                .with_rate_func(smooth),
        );
        sq.hold_to(t + 12.2);
        sq.push(
            panel
                .clone()
                .fade_out()
                .with_duration(0.8)
                .with_rate_func(smooth),
        );
        sq.hold_to(TOTAL);
        stack.push(sq);
        stack.push(group_life(label, t_in + 0.1, t + 12.2));
    }
    stack.push(group_life(&closing, t + 2.8, t + 12.2));
}

/// 全部 capture 时点（绝对秒）。
fn capture_marks() -> Vec<(f64, &'static str)> {
    vec![
        (T_NUMBERS + 9.5, "numbers.png"),
        (T_MECH + 8.0, "mechanism.png"),
        (T_SCAN + 4.2, "scan_box.png"),
        (T_SCAN + 2.0 * ACT_SCAN + 4.2, "scan_edge.png"),
        (T_SCAN + 3.0 * ACT_SCAN + 4.2, "scan_sobelx.png"),
        (T_SCAN + 4.0 * ACT_SCAN + 4.2, "scan_sobely.png"),
        (T_SUMMARY + 6.0, "preview.png"),
    ]
}

#[scene(clear_color = "#202028ff")]
#[output(dir = "./output/convolution_kernels")]
fn convolution_kernels(r: &mut RanimScene) {
    let img = source_image();
    let worked = worked_example();
    let mut stack = AnimStack::new();

    act_hook(&mut stack);
    act_numbers(&mut stack, &img);
    act_mechanism(&mut stack, &img, &worked);
    for idx in 0..KERNELS.len() {
        act_kernel_scan(&mut stack, T_SCAN + idx as f64 * ACT_SCAN, idx);
    }
    act_summary(&mut stack, &img);

    r.play(CameraFrame::default().show().with_duration(TOTAL));
    r.play(stack);
    for (t, name) in capture_marks() {
        r.insert_time_mark(t, TimeMark::Capture(name.to_string()));
    }
}
