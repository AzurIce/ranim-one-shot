# ranim-one-shot

[ranim](https://github.com/AzurIce/ranim) 的 one-shot example 冻结仓库。

one-shot example 的约定是：**一个原始 prompt 对应一次 agent 任务交付**，交付
的成片就是最终产物——ranim 后续的 API 变化、渲染管线调整都不应该（也不会）
反映到已交付的成片上。因此它们不适合继续放在 ranim 主仓库的 `examples/`
里随主仓库演进，而是迁到这里：

- 每个 example 自成一个**独立 cargo package**（没有顶层 workspace），各自
  持有 `Cargo.lock`，把 ranim 依赖**钉死到渲染出成片时所用的 git rev**；
- 一个包坏了不影响其他包；不同包可以钉不同的 rev；
- 仓库 flake 提供与 pin 一致的完整环境（nightly 工具链、ffmpeg、以及用同一
  份钉定源码构建的 `ranim-cli`），保证任何时候都能复现当年的渲染。

## Example 列表

| Example | 一句话说明 | 模型 | 生成日期 | ranim pin |
|---|---|---|---|---|
| [bpe_tokenizer](#bpe_tokenizer) | 157 秒四幕科普视频：LLM 如何用 BPE 把文本变成 token，在迷你语料上完整演示「计数 → 合并 → 重复」训练循环与未见词编码 | GLM-5.3-Flash（ZCode） | 2026-08-27 | [`09d67d0f`](https://github.com/AzurIce/ranim/commit/09d67d0f456c3124cc4e466f407369800f490845) |
| [convolution_kernels](#convolution_kernels) | 在 12x12 像素网格上演示 Identity / Box Blur / Sharpen / Edge Detect 四种常用 3x3 卷积核的滑动窗口卷积过程与结果对比 | Kimi K3（kimi-code/k3） | 2026-08-19 | [`09d67d0f`](https://github.com/AzurIce/ranim/commit/09d67d0f456c3124cc4e466f407369800f490845) |
| [double_pendulum](#double_pendulum) | 三个初始角仅差 0.001 rad 的双摆从重叠到彻底分离，演示混沌的初值敏感性 | Kimi K3（kimi-code/k3） | 2026-08-18 | [`09d67d0f`](https://github.com/AzurIce/ranim/commit/09d67d0f456c3124cc4e466f407369800f490845) |
| [rubiks_cube](#rubiks_cube) | 三阶魔方「12 步打乱 → 逆序求解」全过程，3D 魔方与平面展开图同步更新 | Kimi K3（kimi-code/k3） | 2026-08-18 | [`09d67d0f`](https://github.com/AzurIce/ranim/commit/09d67d0f456c3124cc4e466f407369800f490845) |

各 example 的详细档案（效果图、原始 prompt、设计与实现思路、agent 用
ranim-cli 迭代的全过程记录、模型与 harness 环境）见各自目录下的 `README.md`。

### bpe_tokenizer

![bpe_tokenizer](bpe_tokenizer/preview.png)

「How LLMs Read Text」——面向零基础观众的 tokenizer 科普。从「LLM 从不逐字
母读文本」出发，先对比按字符切与按词切两个极端，然后在 7 词迷你语料上完整
走一遍 BPE 训练循环（拆字符 → 数相邻对 → 合并最高频对 → 重复，四个 merge
`lo / low / es / est` 全程展示），再把学到的规则应用到训练数据里没有的词
`slowest` 得到 `s | low | est`，最后落到真实规模（GPT-2 的 50,257 个
token）与实际后果（strawberry 数 r、数字分块、生僻文字更贵）。屏幕上所有
计数、合并顺序、编码阶段都由 example 内置的真实 BPE 实现计算，无手写数字。
157 s @ 1080p60。

### convolution_kernels

![convolution_kernels](convolution_kernels/preview.png)

在程序化生成的 12x12 灰度图（渐变 + 亮盘 + 斜条）上，逐像素演示四种常用
3x3 卷积核（Identity / Box Blur / Sharpen / Edge Detect）的滑动窗口计算，
黄色窗口按扫描序滑过输入，输出像素随计算逐个亮起，结尾并排对比四种结果。

### double_pendulum

![double_pendulum](double_pendulum/preview.png)

确定性混沌演示：三个完全相同的双摆仅第二杆初始角相差 0.001 rad，前几秒轨
迹完全重叠，随后差异被指数放大直至彻底分离——方程是确定性的，长期行为不可
预测。RK4 积分、拖尾折线可视化末端轨迹。

### rubiks_cube

![rubiks_cube](rubiks_cube/preview.png)

三阶魔方 12 步打乱后按逆序还原的全过程：3D 魔方同步旋转，右侧平面展开图实
时跟综每个面的颜色状态。

## 使用

```bash
# 进入钉定环境：nightly-2026-08-01 工具链 + ffmpeg +
# 与 pin 同源码构建的 ranim-cli
nix develop

# 渲染某个 example（在对应包目录下运行）
cd bpe_tokenizer
ranim output bpe_tokenizer --example bpe_tokenizer -p bpe_tokenizer
```

不用 nix 也可以 `cargo build`（需要满足 `rust-toolchain.toml` 的 nightly
工具链），但渲染需要自备与 pin 一致的 `ranim-cli`。

## 绑定策略

- 每个 package 的 `Cargo.toml` 里 `ranim` 的 `rev` 即其 pin，`Cargo.lock`
  入库，依赖树完全可复现。
- pin 的选择原则：**成片在哪个 ranim 代码状态下渲染，就钉哪个 rev**，且该
  rev 必须能从远端拉取——历史改写后被丢弃的 commit 不可作 pin。
- 迁移备注：double_pendulum / rubiks_cube / convolution_kernels 生成于
  2026-08-18/19，当时的 main 提交在 ranim 主仓库历史改写后已无法从远端拉
  取，故无法钉回原 rev；经内容比对，当前 pin 的 `packages/` 与三个成片渲
  染时的代码状态一致（ranim-core 字节级相同），四个包均在该 pin 上编译通
  过，bpe_tokenizer 另做了全片重渲染，与原成片逐像素一致（PSNR = inf）。
- ranim 主仓库 flake 的 `packages.ranim-cli` 目前构建失败（crane fileset
  遗漏根 crate 源码），本仓库 flake 自行从完整钉定源码树构建 CLI。

## 新增 example

1. 新建 `<example>/` 独立 package：`Cargo.toml` 里钉当次渲染所用的 ranim
   rev，`examples/<example>/lib.rs` 放场景源码（`[[example]]`，
   `crate-type = ["cdylib"]`），附 `README.md`（效果图 / 原始 prompt /
   设计与迭代记录 / 模型与 harness 环境）与成片截图。
2. 若 pin 与现有 flake input 不同，在 `flake.nix` 中新增 input 与对应
   devShell。
3. 在本 README 的 Example 列表中登记。
