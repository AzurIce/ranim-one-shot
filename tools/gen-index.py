#!/usr/bin/env python3
"""Generate all derived indexes from topics/*/*/meta.toml.

Outputs (idempotent — CI reruns this and fails on any diff):
  - README.md                    index section between the HTML markers
  - topics/<topic>/README.md     generated overview: runs table + the
                                 verbatim prompt
  - website/data/index.json      aggregate for the website

topics/*/*/meta.toml is the only input; never hand-edit what this writes.
"""

from __future__ import annotations

import json
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
MARK_START = "<!-- index:start -->"
MARK_END = "<!-- index:end -->"


def fmt_duration(seconds: int) -> str:
    if seconds <= 0:
        return "—"
    m, s = divmod(seconds, 60)
    return f"{seconds}s ({m}:{s:02d})" if m else f"{seconds}s"


def fmt_pin(pin: str) -> str:
    if not pin or set(pin) == {"0"}:
        return "—"
    return f"[`{pin[:8]}`](https://github.com/AzurIce/ranim/commit/{pin})"


def load_topics() -> dict[str, list[dict]]:
    topics: dict[str, list[dict]] = {}
    for meta_path in sorted(ROOT.glob("topics/*/*/meta.toml")):
        run_dir = meta_path.parent
        meta = tomllib.loads(meta_path.read_text())
        meta["_run_dir"] = run_dir.name
        topics.setdefault(run_dir.parent.name, []).append(meta)
    for runs in topics.values():
        def ordinal(m: dict) -> tuple:
            stem = m["_run_dir"].split("-", 1)[0]  # "run1", "run2", ...
            digits = "".join(ch for ch in stem if ch.isdigit())
            return (int(digits or 0), m["_run_dir"])
        runs.sort(key=ordinal)
    return topics


def run_row(topic: str, m: dict) -> str:
    model = m.get("model", {})
    run = m.get("run", {})
    delivery = m.get("delivery", {})
    proto = m.get("protocol", {}).get("ref", "—")
    video = delivery.get("video_ref", "")
    video_cell = f"[video]({video})" if video else "—"
    return (
        f"| [{m['_run_dir']}]({m['_run_dir']}/) "
        f"| {model.get('name', '—')} "
        f"| {run.get('date', '—')} "
        f"| {run.get('render_rounds', '—')} "
        f"| {fmt_duration(int(delivery.get('duration_s', 0)))} "
        f"| {fmt_pin(delivery.get('pin', ''))} "
        f"| {proto} "
        f"| {video_cell} |"
    )


def write_topic_readme(topic: str, runs: list[dict]) -> None:
    prompt_path = ROOT / "topics" / topic / "prompt.md"
    prompt = ""
    if prompt_path.exists():
        lines = prompt_path.read_text().splitlines()
        prompt = "\n".join(
            ln for ln in lines if not ln.startswith("<!--")
        ).strip()
    latest = runs[-1]
    description = latest.get("run", {}).get("description", "")

    lines = [
        f"# {topic}",
        "",
        description or f"One-shot runs for the `{topic}` topic.",
        "",
        "| Run | Model | Date | Rounds | Duration | Pin | Protocol | Video |",
        "|---|---|---|---|---|---|---|---|",
    ]
    lines += [run_row(topic, m) for m in runs]
    lines += ["", "## Original prompt", "", "Verbatim, never edited:", ""]
    if prompt:
        lines += [prompt]
    else:
        lines.append("*(missing: `topics/" + topic + "/prompt.md`)*")
    out = ROOT / "topics" / topic / "README.md"
    out.write_text("\n".join(lines).rstrip() + "\n")


def write_root_readme(topics: dict[str, list[dict]]) -> None:
    if not topics:
        body = "No runs yet — deliveries appear here as they merge to `main`."
    else:
        body = (
            "| Topic | Runs | Models | Pins |\n|---|---|---|---|\n"
            + "\n".join(
                f"| [{topic}](topics/{topic}/) "
                f"| {len(runs)} "
                f"| {', '.join(sorted({m.get('model', {}).get('name', '?') for m in runs}))} "
                f"| {', '.join(sorted({fmt_pin(m.get('delivery', {}).get('pin', '')) for m in runs}))} |"
                for topic, runs in topics.items()
            )
        )
    readme = ROOT / "README.md"
    text = readme.read_text()
    start = text.index(MARK_START) + len(MARK_START)
    end = text.index(MARK_END)
    text = text[:start] + "\n\n" + body + "\n\n" + text[end:]
    readme.write_text(text)


def write_site_data(topics: dict[str, list[dict]]) -> None:
    def scrub(m: dict) -> dict:
        return {k: v for k, v in m.items() if not k.startswith("_")} | {
            "run_dir": m["_run_dir"]
        }

    data = {
        "topics": [
            {
                "name": topic,
                "runs": [scrub(m) for m in runs],
            }
            for topic, runs in topics.items()
        ],
    }
    out = ROOT / "website" / "data"
    out.mkdir(parents=True, exist_ok=True)
    # tomllib yields date/datetime objects for unquoted TOML dates; str()
    # renders them as ISO-8601.
    (out / "index.json").write_text(
        json.dumps(data, ensure_ascii=False, indent=2, default=str) + "\n"
    )


def main() -> None:
    topics = load_topics()
    for topic, runs in topics.items():
        write_topic_readme(topic, runs)
    write_root_readme(topics)
    write_site_data(topics)
    total = sum(len(r) for r in topics.values())
    print(f"gen-index: {len(topics)} topics, {total} runs")


if __name__ == "__main__":
    main()
