# ranim-one-shot

Frozen one-shot explainer videos for [ranim](https://github.com/AzurIce/ranim),
produced as a controlled experiment: **one original prompt, one agent
delivery**. The rendered video delivered at the end of a run is the final
product — later API or rendering-pipeline changes in ranim do not (and must
not) alter it.

## How this repository works

- The **protocol** (conventions, methodology skill, prompts, tooling) lives
  on the `base` branch; every finalized protocol state is tagged
  `one-shot-base-v<N>`.
- A **run** = one agent one-shot: a worktree forked from the newest
  `one-shot-base-v*` tag, delivered as a frozen standalone cargo package at
  `topics/<topic>/run<N>-<modelslug>/` on `main`, with its own `Cargo.lock`
  and flake pinning the exact ranim rev that produced the render.
- `meta.toml` in each run is the single source of truth for model, harness,
  protocol, and delivery facts; all indexes below are generated from it.
- The pre-protocol pilots live on the `legacy` branch, frozen.

See [AGENTS.md](AGENTS.md) for the full contract, and
`skill/ranim-one-shot/SKILL.md` for the production methodology.

## Topics

<!-- index:start -->

No runs yet — deliveries appear here as they merge to `main`.

<!-- index:end -->

## Rendering a run

```bash
cd topics/<topic>/<run>
nix develop          # pinned nightly toolchain, ffmpeg, ranim-cli
ranim output <topic> --example <topic>   # extra features documented in the run README
```

Videos are hosted via [shadow](https://github.com/AzurIce/shadow)
(content-addressed objects); the links live in each run's `meta.toml`.
