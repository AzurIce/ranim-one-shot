# AGENTS.md — ranim-one-shot

This repository runs a controlled one-shot experiment: given **one original
prompt** and a **frozen production protocol**, an AI coding agent delivers a
complete explainer video built with [ranim](https://github.com/AzurIce/ranim).
Every delivery is frozen forever. Humans and AI agents alike follow this
file; the production *methodology* lives in `skill/ranim-one-shot/`, the
repository *contract* lives here.

## Branch topology

| Branch | Purpose |
|---|---|
| `base` | The **protocol line**: this file, `skill/`, `topics/*/prompt.md`, `schema/`, `tools/`, CI, and the root authoring flake. Never contains a run implementation, never bumps the root flake's ranim pin (pin bumps happen inside run branches). Every finalized protocol state is tagged `one-shot-base-v<N>`; runs start from the newest tag. |
| `main` | The **results line**: the protocol plus frozen deliveries under `topics/<topic>/run<N>-<modelslug>/`, and the generated indexes. |
| `legacy` | Pre-reform archive: five pilot one-shots produced before the protocol existed. Frozen; never merge from it — mine it for reference. |

`base` and `main` share their root commit, so run branches (which fork from a
`one-shot-base-v*` tag on `base`) merge back into `main` as normal three-way
merges. After a protocol change, `base` may be merged into `main` to keep
documentation current.

## Frozen-delivery discipline

A run directory under `topics/<topic>/run<N>-<modelslug>/` is **immutable
once merged to `main`**. Never edit its source, `Cargo.lock`, `flake.nix`,
`flake.lock`, captures, or `meta.toml` after delivery — if something is
broken, the fix is another run, not a patch.

- The `rev` on the `ranim` dependency in a run's `Cargo.toml` is **the code
  state that produced the delivered render**; `Cargo.toml`, `flake.nix` and
  `flake.lock` must pin the same rev (CI checks this per run). Pin only
  commits certain to be reachable from a branch of the ranim repository —
  a pre-reform history rewrite once orphaned the pilot pins (see `legacy`'s
  root README).
- Package name and `[[example]]` name always equal the **topic name**, so
  the render command is identical across every run of a topic.
- Rendered videos are never committed to git; they are published with
  [shadow](https://github.com/AzurIce/shadow) (see *Media*).

## Starting a run

```bash
git fetch --tags origin
git worktree add -b <topic>-run<N>-<modelslug> ../ros-<topic>-run<N> one-shot-base-v<N>
cd ../ros-<topic>-run<N>
```

1. Read `topics/<topic>/prompt.md`. It is verbatim and must never be edited.
2. Follow `skill/ranim-one-shot/SKILL.md` for methodology.
3. **First commit on the run branch**: if the topic needs a newer ranim,
   bump the root flake's `ranim` pin (and the matching nightly) — otherwise
   leave it untouched.
4. Reference older runs by reading them on `main` (structural patterns,
   pitfalls); do not merge them into the run branch.
5. The protocol is frozen for the duration of the run (next section).

Naming: topic slugs are lowercase `snake_case`; model slugs are lowercase
alphanumeric without dots (`glm53flash`, `kimik3`). `<N>` is the per-topic
run ordinal, starting at 1.

## Protocol freeze

Within a run branch, these paths must remain **byte-identical** to the
starting tag: `skill/`, `AGENTS.md`, `schema/`, `tools/`,
`topics/*/prompt.md`. CI enforces this on run-branch PRs. If the protocol
blocks you, do not edit it — record the problem in `meta.toml`
(`[protocol] skill_modified / notes`) and continue. Protocol revisions
happen **between runs**, as commits on `base` followed by a new
`one-shot-base-v<N>` tag.

## `meta.toml`

Every run ships `topics/<topic>/run<N>-<modelslug>/meta.toml`, following
`schema/meta.toml.example`. It is the single source of truth for model,
harness, protocol, run, and delivery facts; README indexes and the website
are generated from it by `tools/gen-index.py` and never hand-edited.

- Model identity is **self-reported** by the harness unless the maintainer
  verified it (`source = "verified"`).
- Record render rounds and wall time honestly — they are experiment data,
  not vanity metrics.
- `version` fields may be `"unknown"`; never invent values.

## Delivery checklist

- [ ] `cargo check` / `cargo clippy` / `cargo fmt` / `cargo test` clean
      (with the pin's toolchain, typst features included where used)
- [ ] Final full render: `ranim output <topic> --example <topic>`
      (plus any features, documented in the run README)
- [ ] Captures copied into the run directory; `preview.png` is the hero frame
- [ ] Run flake frozen: copy the root flake into the run directory, point
      its `ranim` input at the delivered rev, `nix flake lock` committed
- [ ] `meta.toml` complete (model, harness, protocol ref, run stats, delivery)
- [ ] Run README per `skill/ranim-one-shot/reference/run-readme-template.md`
- [ ] `shadow publish` for the mp4; ref committed; URL backfilled into
      `meta.toml` (`video_ref`)
- [ ] `python3 tools/gen-index.py` run; generated indexes refreshed
- [ ] CI green, including the protocol-freeze check

> Run flakes pinning a rev that **predates ranim#211** cannot use ranim's
> own `packages.ranim-cli` (it was broken back then) — in that case keep the
> local crane build, copying the flake shape from a `legacy` example.

## Media (shadow)

Rendered mp4 files live under the run's `output/` and are **not committed**.
At delivery: `shadow publish` uploads them as content-addressed objects and
writes small refs under `.shadow/refs/` (committed). Install with a pinned
rev — do not track moving HEAD:

```bash
cargo install --git https://github.com/AzurIce/shadow --rev ceaaac778ab433f6666957facbf68eeb7c58b918 --locked
```

Credentials live in `.env` (`TOS_ACCESS_KEY` / `TOS_SECRET_KEY`,
gitignored, never committed). **Never run `shadow free` in this
repository** — frozen artifacts are immutable and their URLs are permanent;
CI only runs `shadow check --remote`.

## Adding a topic

PR against **`base`** adding `topics/<t>/prompt.md` — the verbatim prompt,
never paraphrased. After merge, tag `one-shot-base-v<N+1>`; subsequent runs
start from the new tag.

## Language

`AGENTS.md`, root `README.md`, schema and CI comments: English.
`topics/*/prompt.md`: verbatim original wording (usually Chinese), never
edited or translated. Run READMEs: Chinese, following the template.
Commit messages: English, conventional style.
