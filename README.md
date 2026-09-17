# ranim-one-shot

Frozen one-shot explainer videos for [ranim](https://github.com/AzurIce/ranim),
produced as a controlled experiment: **one original prompt, one agent
delivery**. The rendered video delivered at the end of a run is the final
product — later API or rendering-pipeline changes in ranim do not (and must
not) alter it.

## How this repository works

- The **protocol** (conventions, methodology skill, prompts, tooling) lives
  on the `base` branch; runs always fork from its tip, and its append-only
  history keeps every past protocol state reachable.
- A **run** = one agent one-shot: a worktree forked from `base`, delivered
  as a frozen standalone cargo package at
  `topics/<topic>/run<N>-<modelslug>/` on `main`, with its own `Cargo.lock`
  and flake pinning the exact ranim rev that produced the render.
- `meta.toml` in each run is the single source of truth for model, harness,
  protocol, and delivery facts; all indexes below are generated from it.
- The pre-protocol pilots live on the `legacy` branch, frozen.

See [AGENTS.md](AGENTS.md) for the full contract, and
`.agents/skills/ranim-one-shot/SKILL.md` for the production methodology.

## Topics

<!-- index:start -->

| Topic | Runs | Models | Pins |
|---|---|---|---|
| [rubiks_cube](topics/rubiks_cube/) | 1 | GLM-5.3-Flash | [`40d15be6`](https://github.com/AzurIce/ranim/commit/40d15be64edf5c04a78e2db75908e4a192a8e942) |

<!-- index:end -->

## Rendering a run

```bash
cd topics/<topic>/<run>
nix develop          # pinned nightly toolchain, ffmpeg, ranim-cli
ranim output <topic> --example <topic>   # extra features documented in the run README
```

Videos are hosted via [shadow](https://github.com/AzurIce/shadow)
(content-addressed objects); the links are derived from committed refs and
shown on the [website](https://azurice.github.io/ranim-one-shot/).
