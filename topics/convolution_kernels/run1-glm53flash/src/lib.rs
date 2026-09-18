//! convolution_kernels 的真实实现层。
//!
//! 屏幕上出现的每一个数字都来自这里：程序生成的源图、标准 3x3 零填充
//! 卷积、显示约定与机制幕的完整算例。`cargo test` 断言这些行为，
//! 动画时间线由它们输出的数据驱动——动画只是数据的视图。

/// 图像边长，源图与所有卷积输出都是 `N * N`。
pub const N: usize = 10;

/// 竖直亮条的 x 坐标（列方向）。
const BAR_X: f64 = 2.1;
/// 水平亮条的 y 坐标（行方向）。
const BAR_Y: f64 = 2.1;
/// 亮圆盘的 (x, y, 半径)。
const DISC: (f64, f64, f64) = (6.5, 6.2, 1.7);

/// 程序生成的源图：水平渐变作平滑基底，叠一个亮圆盘、一条竖直亮条、
/// 一条水平亮条，clamp 到 [0, 1]。
///
/// 四种特征各有用途：平滑区域看模糊，圆盘看锐化与各向边缘，
/// 竖直条只响应 Sobel X，水平条只响应 Sobel Y。
pub fn source_image() -> Vec<f64> {
    let mut img = vec![0.0; N * N];
    for i in 0..N {
        for j in 0..N {
            let (x, y) = (j as f64, i as f64); // x = 列，y = 行，行 0 在顶部
            let mut v = 0.12 + 0.40 * x / (N - 1) as f64;
            let dist = ((x - DISC.0).powi(2) + (y - DISC.1).powi(2)).sqrt();
            if dist < DISC.2 {
                v += 0.38;
            }
            if (x - BAR_X).abs() < 0.5 {
                v += 0.30;
            }
            if (y - BAR_Y).abs() < 0.5 {
                v += 0.30;
            }
            img[i * N + j] = v.clamp(0.0, 1.0);
        }
    }
    img
}

/// 标准 3x3 卷积：zero padding，不归一化，保留符号。
/// `img` 为行优先的 `n * n` 图，`kernel` 为 `kernel[行][列]`。
pub fn convolve_n(img: &[f64], n: usize, kernel: &[[f64; 3]; 3]) -> Vec<f64> {
    let mut out = vec![0.0; n * n];
    for i in 0..n {
        for j in 0..n {
            let mut acc = 0.0;
            for (ki, krow) in kernel.iter().enumerate() {
                for (kj, &w) in krow.iter().enumerate() {
                    let y = i as i64 + ki as i64 - 1;
                    let x = j as i64 + kj as i64 - 1;
                    let inside = (0..n as i64).contains(&y) && (0..n as i64).contains(&x);
                    let v = if inside {
                        img[y as usize * n + x as usize]
                    } else {
                        0.0
                    };
                    acc += w * v;
                }
            }
            out[i * n + j] = acc;
        }
    }
    out
}

/// 对内置尺寸 [`N`] 的图像做卷积。
pub fn convolve(img: &[f64], kernel: &[[f64; 3]; 3]) -> Vec<f64> {
    convolve_n(img, N, kernel)
}

/// 屏幕显示约定：无符号核直接 clamp 到 [0, 1]；有符号核（边缘、Sobel）
/// 取响应的绝对值后 clamp。片头字幕与 README 均声明该约定。
pub fn display_value(raw: f64, signed: bool) -> f64 {
    let v = if signed { raw.abs() } else { raw };
    v.clamp(0.0, 1.0)
}

/// 四舍五入到 2 位小数（屏幕数字的精度）。
pub fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

/// 卷积窗口 (center_r, center_c) 覆盖的 9 个源值，按行优先。
pub fn window_values(img: &[f64], center_r: usize, center_c: usize) -> [f64; 9] {
    let mut vals = [0.0; 9];
    for (k, slot) in vals.iter_mut().enumerate() {
        let (di, dj) = ((k / 3) as i64 - 1, (k % 3) as i64 - 1);
        *slot = img[(center_r as i64 + di) as usize * N + (center_c as i64 + dj) as usize];
    }
    vals
}

/// 机制幕的完整算例：放大区中心像素用盒式模糊核计算。
///
/// 屏幕显示的 9 个乘积、它们的和、以及和除以 9 的结果都出自这里；
/// [`tests::worked_consistency`] 断言显示值与真实卷积输出一致。
pub struct Worked {
    /// 窗口中心的行。
    pub center_r: usize,
    /// 窗口中心的列。
    pub center_c: usize,
    /// 屏幕显示的 9 个乘积（源值四舍五入到 2 位小数），行优先。
    pub products: [f64; 9],
    /// 屏幕显示的乘积之和，即 `products` 的精确和再取 2 位小数。
    pub sum: f64,
    /// 屏幕显示的平均值，`sum / 9` 取 2 位小数。
    pub mean: f64,
}

/// 以 (4, 5) 为窗口中心、盒式模糊核为例的完整算例。
pub fn worked_example() -> Worked {
    let img = source_image();
    let (center_r, center_c) = (4usize, 5usize);
    let products = window_values(&img, center_r, center_c).map(round2);
    let sum = round2(products.iter().sum::<f64>());
    let mean = round2(sum / 9.0);
    Worked {
        center_r,
        center_c,
        products,
        sum,
        mean,
    }
}

/// 一个待演示的卷积核：矩阵、名称与一句话解读。
pub struct KernelDef {
    /// 屏幕上的核名称。
    pub label: &'static str,
    /// 扫描完成后的解读字幕。
    pub caption: &'static str,
    /// 3x3 卷积核矩阵，`kernel[行][列]`。
    pub kernel: [[f64; 3]; 3],
    /// 响应有符号：屏幕显示绝对值。
    pub signed: bool,
}

/// 本片演示的五种常用卷积核。
pub const KERNELS: [KernelDef; 5] = [
    KernelDef {
        label: "盒式模糊",
        caption: "九个 1/9：取邻域平均，差异被抹平",
        kernel: [[1.0 / 9.0; 3]; 3],
        signed: false,
    },
    KernelDef {
        label: "锐化",
        caption: "中心 5、四邻 -1：与邻居的差距被放大",
        kernel: [[0.0, -1.0, 0.0], [-1.0, 5.0, -1.0], [0.0, -1.0, 0.0]],
        signed: false,
    },
    KernelDef {
        label: "边缘检测",
        caption: "周围 +1、中心 -8：平坦处归零，只剩变化",
        kernel: [[-1.0; 3], [-1.0, 8.0, -1.0], [-1.0; 3]],
        signed: true,
    },
    KernelDef {
        label: "Sobel X",
        caption: "左列减右列：只对竖直的边缘有响应",
        kernel: [[1.0, 0.0, -1.0], [2.0, 0.0, -2.0], [1.0, 0.0, -1.0]],
        signed: true,
    },
    KernelDef {
        label: "Sobel Y",
        caption: "上两行减下两行：只对水平的边缘有响应",
        kernel: [[-1.0, -2.0, -1.0], [0.0, 0.0, 0.0], [1.0, 2.0, 1.0]],
        signed: true,
    },
];

/// 卷积核矩阵单元的屏幕文本：整数原样，1/9 写作分数，其余透传。
pub fn kernel_cell_text(v: f64) -> String {
    if v == 0.0 {
        "0".to_string()
    } else if (v - v.round()).abs() < 1e-9 {
        format!("{}", v.round() as i64)
    } else if (v * 9.0 - 1.0).abs() < 1e-9 {
        "1/9".to_string()
    } else {
        format!("{v}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EPS: f64 = 1e-9;

    #[test]
    fn source_image_range_and_features() {
        let img = source_image();
        assert_eq!(img.len(), N * N);
        assert!(img.iter().all(|v| (0.0..=1.0).contains(v)));
        // 底部一行只有水平渐变与竖直条：除竖直条右缘（j = 3）处
        // 的回落外，沿行严格单调递增。
        for j in 1..N {
            if j == 3 {
                continue;
            }
            assert!(img[9 * N + j] > img[9 * N + j - 1]);
        }
        // 圆盘中心比同列的平滑基底亮。
        let base = 0.12 + 0.40 * 6.0 / 9.0;
        assert!(img[6 * N + 6] > base + 0.3);
        // 竖直条与水平条的交点叠加了两个 +0.30。
        assert!(img[2 * N + 2] > 0.75);
    }

    #[test]
    fn convolve_identity_recovers_source() {
        let img = source_image();
        let identity = [[0.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 0.0]];
        let out = convolve(&img, &identity);
        assert!(out.iter().zip(&img).all(|(o, v)| (o - v).abs() < EPS));
    }

    #[test]
    fn convolve_box_keeps_constant_interior() {
        let img = vec![0.5; N * N];
        let out = convolve(&img, &KERNELS[0].kernel);
        // 内部远离边界处仍是 0.5；角点因 zero padding 只剩 4/9 权重。
        assert!((out[5 * N + 5] - 0.5).abs() < EPS);
        assert!((out[0] - 0.5 * 4.0 / 9.0).abs() < EPS);
    }

    #[test]
    fn convolve_hand_computed_spike() {
        // 3x3 图像，仅中心为 1。
        let img = vec![0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0];
        let box_out = convolve_n(&img, 3, &KERNELS[0].kernel);
        assert!(box_out.iter().all(|v| (v - 1.0 / 9.0).abs() < EPS));
        let lap = convolve_n(&img, 3, &KERNELS[2].kernel);
        assert!((lap[4] - 8.0).abs() < EPS, "中心响应 = +8");
        assert!((lap[0] - (-1.0)).abs() < EPS, "角点响应 = 来自中心的 -1");
    }

    #[test]
    fn laplacian_padding_artifact_is_real() {
        // 常数图的内部响应为 0，边界因 zero padding 出现强响应——
        // 视频中边缘检测输出四周的亮框即此伪影，属忠实呈现。
        let img = vec![0.8; N * N];
        let out = convolve(&img, &KERNELS[2].kernel);
        for i in 1..N - 1 {
            for j in 1..N - 1 {
                assert!(out[i * N + j].abs() < EPS);
            }
        }
        // 角点：中心权重 8 乘 0.8，减去 3 个界内邻居的 0.8。
        assert!((out[0] - 5.0 * 0.8).abs() < EPS);
        assert!(out[0] > 1.0);
    }

    #[test]
    fn sobel_responds_to_matching_direction_only() {
        // 竖直阶跃：左半亮。Sobel X 在交界处响应 4，Sobel Y 无响应。
        let mut img = vec![0.0; N * N];
        for i in 0..N {
            for j in 0..N {
                if j < 5 {
                    img[i * N + j] = 1.0;
                }
            }
        }
        let gx = convolve(&img, &KERNELS[3].kernel);
        let gy = convolve(&img, &KERNELS[4].kernel);
        assert!((gx[5 * N + 5] - 4.0).abs() < EPS);
        assert!(gy[5 * N + 5].abs() < EPS);
        // 水平阶跃：上半亮。Sobel Y 在交界处响应幅度 4。
        let mut img = vec![0.0; N * N];
        for i in 0..N {
            for j in 0..N {
                if i < 5 {
                    img[i * N + j] = 1.0;
                }
            }
        }
        let gx = convolve(&img, &KERNELS[3].kernel);
        let gy = convolve(&img, &KERNELS[4].kernel);
        assert!(gx[5 * N + 5].abs() < EPS);
        assert!((gy[5 * N + 5] - (-4.0)).abs() < EPS);
    }

    #[test]
    fn display_value_conventions() {
        assert!((display_value(0.7, false) - 0.7).abs() < EPS);
        assert_eq!(display_value(-0.5, false), 0.0, "无符号核不取绝对值");
        assert!((display_value(-0.5, true) - 0.5).abs() < EPS);
        assert_eq!(display_value(-3.2, true), 1.0, "绝对值超界后 clamp");
    }

    #[test]
    fn worked_consistency() {
        let img = source_image();
        let w = worked_example();
        let (r, c) = (w.center_r, w.center_c);
        // 显示的 9 个乘积 = 窗口内源值（2 位小数）。
        let vals = window_values(&img, r, c);
        for (p, v) in w.products.iter().zip(vals.iter()) {
            assert!((p - round2(*v)).abs() < EPS);
        }
        // 屏幕上的和 = 屏幕上 9 个数之和；平均 = 和 ÷ 9。
        assert!((w.sum - round2(w.products.iter().sum::<f64>())).abs() < EPS);
        assert!((w.mean - round2(w.sum / 9.0)).abs() < EPS);
        // 屏幕平均值与真实卷积输出一致到"显示精度"：屏幕数字按 2 位
        // 小数逐级舍入（乘积 → 和 → 平均），与精确均值最多差一个
        // 显示精度（0.01）。
        let out = convolve(&img, &KERNELS[0].kernel);
        assert!((out[r * N + c] - w.sum / 9.0).abs() < 0.0051);
        assert!((w.mean - out[r * N + c]).abs() < 0.0101);
    }

    #[test]
    fn all_kernels_produce_displayable_outputs() {
        let img = source_image();
        for k in KERNELS.iter() {
            let out = convolve(&img, &k.kernel);
            assert_eq!(out.len(), N * N);
            for raw in out.iter() {
                let v = display_value(*raw, k.signed);
                assert!((0.0..=1.0).contains(&v));
            }
        }
    }

    #[test]
    fn kernel_cell_text_matches_matrices() {
        let expected: [&[&str]; 5] = [
            &["1/9"; 9],
            &["0", "-1", "0", "-1", "5", "-1", "0", "-1", "0"],
            &["-1", "-1", "-1", "-1", "8", "-1", "-1", "-1", "-1"],
            &["1", "0", "-1", "2", "0", "-2", "1", "0", "-1"],
            &["-1", "-2", "-1", "0", "0", "0", "1", "2", "1"],
        ];
        for (k, exp) in KERNELS.iter().zip(expected.iter()) {
            let flat: Vec<String> = k
                .kernel
                .iter()
                .flatten()
                .map(|v| kernel_cell_text(*v))
                .collect();
            let exp: Vec<String> = exp.iter().map(|s| s.to_string()).collect();
            assert_eq!(flat, exp, "kernel {}", k.label);
        }
    }
}
