# ranim-one-shot

Frozen one-shot examples for [ranim](https://github.com/AzurIce/ranim).

The one-shot convention: **one original prompt, one agent delivery**. The
rendered video delivered at the end of the task is the final product — later
API or rendering-pipeline changes in ranim do not (and must not) alter it.
That makes these examples a poor fit for the ranim repository's `examples/`,
which evolve with the main codebase; they live here instead.

## Layout

- Each example is a standalone cargo package (no top-level workspace) with
  its own committed `Cargo.lock`, pinning ranim to the exact git rev that
  produced the delivered render. One broken package never affects the others,
  and different packages may pin different revs.
- Each example carries its own flake (`flake.nix` + `flake.lock`) freezing
  the environment for that pin: the matching nightly toolchain, ffmpeg, and a
  `ranim-cli` built from the same pinned source — so any render stays
  reproducible at any time.
- The root flake is the authoring workspace for the *next* one-shot. On
  delivery, an example gets its own flake and freezes.
- CI keeps each example's Cargo.toml rev, flake rev and flake.lock in sync,
  checks that every flake still evaluates, and `cargo check`s every package
  against its pin.

Detailed archives per example (previews, the original prompt, design and
iteration notes, model and harness environment) live in each directory's
`README.md`.

## Examples

| Example | Description | Model | Date | ranim pin |
|---|---|---|---|---|
| [bpe_tokenizer](#bpe_tokenizer) | 157 s four-act explainer: how LLMs turn text into tokens with BPE — the full count → merge → repeat training loop on a mini corpus, plus encoding of unseen words | GLM-5.3-Flash (ZCode) | 2026-08-27 | [`09d67d0f`](https://github.com/AzurIce/ranim/commit/09d67d0f456c3124cc4e466f407369800f490845) |
| [convolution_kernels](#convolution_kernels) | Sliding-window 2D convolution on a 12x12 pixel grid: Identity / Box Blur / Sharpen / Edge Detect, computed pixel by pixel and compared side by side | Kimi K3 (kimi-code/k3) | 2026-08-19 | [`09d67d0f`](https://github.com/AzurIce/ranim/commit/09d67d0f456c3124cc4e466f407369800f490845) |
| [double_pendulum](#double_pendulum) | Three double pendulums whose initial angles differ by only 0.001 rad: perfectly overlapping at first, then exponentially diverging — sensitivity to initial conditions | Kimi K3 (kimi-code/k3) | 2026-08-18 | [`09d67d0f`](https://github.com/AzurIce/ranim/commit/09d67d0f456c3124cc4e466f407369800f490845) |
| [linux_mem_alloc](#linux_mem_alloc) | 343 s five-act explainer: where a malloc's bytes really come from — virtual memory, segmentation vs paging, the buddy allocator's split/merge cascades, slab's warm slot reuse, and the full supply chain of one allocation | GLM-5.3-Flash (ZCode) | 2026-09-15 | [`09d67d0f`](https://github.com/AzurIce/ranim/commit/09d67d0f456c3124cc4e466f407369800f490845) |
| [rubiks_cube](#rubiks_cube) | A 3x3x3 cube scrambled in 12 moves and solved in reverse, with the 3D cube and a live 2D net kept in sync | Kimi K3 (kimi-code/k3) | 2026-08-18 | [`09d67d0f`](https://github.com/AzurIce/ranim/commit/09d67d0f456c3124cc4e466f407369800f490845) |

### bpe_tokenizer

![bpe_tokenizer](bpe_tokenizer/preview.png)

"How LLMs Read Text" — a tokenizer explainer for a general audience. Starting
from "LLMs never read text letter by letter", it contrasts character-level and
word-level tokenization, walks the full BPE training loop on a 7-word mini
corpus (split into characters → count adjacent pairs → merge the most frequent
→ repeat, with all four merges `lo / low / es / est` shown on screen), applies
the learned rules to the unseen word `slowest` → `s | low | est`, and closes
with real-world scale (GPT-2's 50,257 tokens) and its consequences (counting
the r's in strawberry, digit chunking, more expensive scripts). Every count,
merge order and encoding phase on screen is computed by the example's real
BPE implementation — nothing is hand-written. 157 s @ 1080p60.

### convolution_kernels

![convolution_kernels](convolution_kernels/preview.png)

On a procedurally generated 12x12 grayscale image (gradient + bright disk +
diagonal stripes), four common 3x3 kernels (Identity / Box Blur / Sharpen /
Edge Detect) are demonstrated pixel by pixel: a yellow window slides across
the input in scan order, output pixels light up as they are computed, and the
four results are compared side by side at the end.

### linux_mem_alloc

![linux_mem_alloc](linux_mem_alloc/preview.png)

"Where Does Memory Come From?" — a kernel memory-management explainer for a
general audience, following one `malloc(100)` down the supply chain. It builds
the mental model (memory = numbered bytes), exposes the virtual-address lie,
fails segmentation on a computed external-fragmentation demo (128 KB free, an
80 KB request impossible), wins with paging (page tables, a bit-level
translation close-up, multi-level table math derived from constants), then
hands physical frames to a buddy allocator whose split/merge cascades are
driven by a real in-file simulation (`p ^ size` buddy lookup, frees merging
all the way back to one whole block), and finishes with slab (per-type caches,
warm slot reuse, kmalloc size classes) and a demand-paging payoff: malloc
returned long before any RAM existed. Every placement, translation and
cascade on screen comes from the example's real allocators, verified by unit
tests. 343 s @ 1080p60.

### double_pendulum

![double_pendulum](double_pendulum/preview.png)

Deterministic chaos: three identical double pendulums differ only in the
second arm's initial angle by 0.001 rad. Their trajectories overlap perfectly
for the first seconds, then the difference grows exponentially until they
fully separate — the equations are deterministic, yet long-term behavior is
unpredictable. RK4 integration, with trailing polylines visualizing the tip
trajectories.

### rubiks_cube

![rubiks_cube](rubiks_cube/preview.png)

A full 3x3x3 cube scrambled in 12 moves and solved in reverse order: the 3D
cube rotates while the 2D net on the right tracks the color state of every
face in real time.

## Usage

```bash
# Enter the frozen environment: pinned nightly toolchain, ffmpeg, and a
# ranim-cli built from the same pinned source.
cd double_pendulum
nix develop

# Render (inside the package directory; the exact command — extra
# --features, etc. — is documented in each example's README)
ranim output double_pendulum --example double_pendulum
```

Nix is not strictly required — `cargo build` works with any toolchain
satisfying the root `rust-toolchain.toml` — but rendering then needs a
`ranim-cli` matching the pin.

## Pinning policy

- The `rev` on the `ranim` dependency in each package's `Cargo.toml` is its
  pin; `Cargo.lock` is committed, so the dependency tree is fully
  reproducible. CI checks that Cargo.toml, flake.nix and flake.lock all pin
  the same rev.
- Rule: **pin the rev whose code state produced the delivered render**, and
  it must be fetchable from the remote — commits dropped by history rewrites
  cannot serve as pins.
- Migration note: double_pendulum / rubiks_cube / convolution_kernels were
  created on 2026-08-18/19; the then-main commit of the ranim repository is
  no longer fetchable after a history rewrite, so the original revs cannot be
  pinned. After content comparison, the pinned rev's `packages/` matches the
  code state the three renders were produced with (ranim-core is
  byte-identical); all four packages compile on this pin, and bpe_tokenizer
  additionally did a full re-render, pixel-identical to the original
  (PSNR = inf).
- The ranim repository's flake `packages.ranim-cli` is currently broken (its
  crane fileset omits the root crate's sources), so the flakes here build the
  CLI from the full pinned source tree themselves.

## Adding an example

1. Create `<example>/` as a standalone package: pin the ranim rev used for
   that render in `Cargo.toml`, put the scene source in
   `examples/<example>/lib.rs` (`[[example]]`, `crate-type = ["cdylib"]`),
   and add a `README.md` (preview, original prompt, design & iteration log,
   model and harness environment) plus final-frame screenshots.
2. Copy the flake from an existing example into the new directory, point its
   `ranim` input at the package's pin, and run `nix flake lock`.
3. Register the example in the table above.
