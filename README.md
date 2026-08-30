# ranim-one-shot

[examples/agents](https://github.com/AzurIce/ranim) 的 one-shot example 的
冻结仓库：每个 example 自成一个独立 cargo package，依赖钉死到渲染出成片时
所用的 ranim git rev。one-shot 的成片交付之后 example 就不再演进（ranim
后续 API 变化不影响它复现当年的成片），因此版本绑定到 commit 而不是分支。

## 结构

```text
ranim-one-shot/
├── flake.nix              # devShell：钉定 rev 的 ranim-cli + 对应 nightly 工具链
├── rust-toolchain.toml    # 供非 nix 的 rustup 用户对齐工具链
└── <example>/             # 每个example一个独立package（各自 Cargo.lock，互不影响）
    ├── Cargo.toml         # ranim = { git = ..., rev = <pin> }
    ├── Cargo.lock
    ├── README.md          # 效果图 / 原始 prompt / 设计与迭代记录
    ├── examples/<example>/lib.rs
    └── *.png              # 成片截图
```

各 package 之间**没有顶层 workspace**——不同 package 可以各自钉不同的
ranim rev，一个包坏了不影响其他包构建。

## 使用

```bash
# 进入钉定环境（含与 pin 同 rev 构建的 ranim-cli、nightly 工具链、ffmpeg）
nix develop

# 渲染某个 example（在仓库根目录或对应包目录下均可）
ranim output <scene> --example <example> -p <example>
```

不加 nix 时也可以直接 `cargo build`（需要能满足 `rust-toolchain.toml` 的
nightly 工具链），但渲染需要自备与 pin 一致的 `ranim-cli`。

## 绑定策略

- 每个 package 的 `Cargo.toml` 里 `ranim` 的 `rev` 即其 pin；`Cargo.lock`
  一并入库，保证依赖树完全可复现。
- pin 的选择原则：**成片是在哪个 ranim 代码状态下渲染的，就钉哪个 rev**，
  且该 rev 必须能从远端拉取（历史改写后被丢弃的 commit 不可作 pin）。
- 新增 example 时：建独立 package、钉当次渲染所用 rev；若 rev 与现有
  flake input 不同，在 `flake.nix` 里新增 input 与对应 devShell。

## Example 一览

| Example | ranim pin | 状态 |
|---|---|---|
| [bpe_tokenizer](bpe_tokenizer/) | `09d67d0f` | 已在 pin 上构建并渲染 |
| [convolution_kernels](convolution_kernels/) | `09d67d0f` | 已在 pin 上构建（成片为迁移前渲染） |
| [double_pendulum](double_pendulum/) | `09d67d0f` | 已在 pin 上构建（成片为迁移前渲染） |
| [rubiks_cube](rubiks_cube/) | `09d67d0f` | 已在 pin 上构建（成片为迁移前渲染） |

> 历史说明：double_pendulum / rubiks_cube / convolution_kernels 创建于
> 2026-08-18/19，当时对应的 main 提交（如 `d32ae2ee`）在 ranim main 历史
> 改写后已不可从远端拉取，故无法钉回原 rev；经内容比对，`09d67d0f` 的
> `packages/` 与各成片渲染时的代码状态一致（ranim-core 字节级相同），三个
> example 在该 pin 上均编译通过，成片视为仍然有效。
