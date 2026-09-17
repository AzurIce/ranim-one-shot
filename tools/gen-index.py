#!/usr/bin/env python3
"""Generate all derived indexes and the website content from
topics/*/*/meta.toml plus committed shadow refs.

Outputs (idempotent — CI reruns this and fails on any diff):
  - README.md                     index section between the HTML markers
  - topics/<topic>/README.md      generated overview: runs table + the
                                  verbatim prompt
  - website/data/index.json       aggregate consumed by the home template
  - website/content/topics/...    Zola section + page files (topic/run)
  - website/static/runs/...       copies of each run's capture PNGs

topics/*/*/meta.toml and .shadow/refs/ are the only inputs; never
hand-edit what this writes.
"""

from __future__ import annotations

import json
import shutil
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SITE = ROOT / "website"
MARK_START = "<!-- index:start -->"
MARK_END = "<!-- index:end -->"

# Keep in sync with shadow.toml: <bucket>.<endpoint-host>/<name>/objects/...
SHADOW_URL_BASE = "https://azurice-shadow.tos-cn-beijing.volces.com/ranim-one-shot/objects"


def toml_str(s: str) -> str:
    """Escape a string as a TOML basic string (single line)."""
    out = (
        s.replace("\\", "\\\\")
        .replace('"', '\\"')
        .replace("\n", "\\n")
        .replace("\t", "\\t")
        .replace("\r", "\\r")
    )
    return f'"{out}"'


def fmt_duration(seconds: int) -> str:
    if seconds <= 0:
        return "—"
    m, s = divmod(seconds, 60)
    return f"{seconds}s ({m}:{s:02d})" if m else f"{seconds}s"


def proto_display(protocol: dict) -> str:
    """`base@<short-sha>` derived from the recorded fork commit."""
    commit = str(protocol.get("commit", ""))
    if commit:
        return f"base@{commit[:8]}"
    return str(protocol.get("ref", "")) or "—"


def fmt_pin(pin: str) -> str:
    if not pin or set(pin) == {"0"}:
        return "—"
    return f"[`{pin[:8]}`](https://github.com/AzurIce/ranim/commit/{pin})"


def load_topics() -> dict[str, list[dict]]:
    topics: dict[str, list[dict]] = {}
    for meta_path in sorted(ROOT.glob("topics/*/*/meta.toml")):
        run_dir = meta_path.parent
        topic = run_dir.parent.name
        meta = tomllib.loads(meta_path.read_text())
        meta["_run_dir"] = run_dir.name
        meta["_rel"] = str(run_dir.relative_to(ROOT))
        topics.setdefault(topic, []).append(meta)
    for runs in topics.values():
        def ordinal(m: dict) -> tuple:
            stem = m["_run_dir"].split("-", 1)[0]  # "run1", "run2", ...
            digits = "".join(ch for ch in stem if ch.isdigit())
            return (int(digits or 0), m["_run_dir"])
        runs.sort(key=ordinal)
    return topics


def shadow_urls(rel: str) -> dict[str, str]:
    """Map `<run rel>/output/<file>` -> content-addressed URL, derived from
    the committed .shadow/refs/ tree (one `<path>.ref` per published file)."""
    refs = ROOT / ".shadow" / "refs" / rel / "output"
    out: dict[str, str] = {}
    if refs.is_dir():
        for ref in sorted(refs.glob("*.ref")):
            data = tomllib.loads(ref.read_text())
            oid = str(data.get("oid", ""))
            kind, _, hexdigest = oid.partition(":")
            if kind != "sha256" or len(hexdigest) != 64:
                print(f"gen-index: WARNING unparsable oid in {ref}")
                continue
            url = f"{SHADOW_URL_BASE}/{kind}/{hexdigest[:2]}/{hexdigest[2:]}"
            out[f"{rel}/output/{ref.stem}"] = url
    return out


def run_video(rel: str, published: dict[str, str]) -> str:
    for path, url in sorted(published.items()):
        if path.endswith(".mp4"):
            return url
    return ""


def run_copies_captures(run_dir: Path, topic: str, run: str) -> list[str]:
    """Copy capture PNGs into the site and return their site-relative paths."""
    dest = SITE / "static" / "runs" / topic / run
    dest.mkdir(parents=True, exist_ok=True)
    copied = []
    for png in sorted(run_dir.glob("*.png")):
        shutil.copyfile(png, dest / png.name)
        copied.append(f"runs/{topic}/{run}/{png.name}")
    return copied


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


def write_topic_readme(topic: str, runs: list[dict], published: dict[str, str]) -> None:
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
    for m in runs:
        model = m.get("model", {})
        run = m.get("run", {})
        delivery = m.get("delivery", {})
        proto = proto_display(m.get("protocol", {}))
        rel = m["_rel"]
        video = run_video(rel, published)
        video_cell = f"[video]({video})" if video else "—"
        lines.append(
            f"| [{m['_run_dir']}]({m['_run_dir']}/) "
            f"| {model.get('name', '—')} "
            f"| {run.get('date', '—')} "
            f"| {run.get('render_rounds', '—')} "
            f"| {fmt_duration(int(delivery.get('duration_s', 0)))} "
            f"| {fmt_pin(delivery.get('pin', ''))} "
            f"| {proto} "
            f"| {video_cell} |"
        )
    lines += ["", "## Original prompt", "", "Verbatim, never edited:", ""]
    if prompt:
        lines += [prompt]
    else:
        lines.append(f"*(missing: `topics/{topic}/prompt.md`)*")
    out = ROOT / "topics" / topic / "README.md"
    out.write_text("\n".join(lines).rstrip() + "\n")


def write_site(topics: dict[str, list[dict]], published_all: dict[str, dict[str, str]]) -> None:
    # Regenerate from scratch so deleted runs/topics disappear too.
    for gen in ("content/topics", "static/runs", "data"):
        shutil.rmtree(SITE / gen, ignore_errors=True)
    (SITE / "data").mkdir(parents=True, exist_ok=True)

    site_topics = []
    for topic, runs in topics.items():
        latest = runs[-1]
        prompt_path = ROOT / "topics" / topic / "prompt.md"
        prompt = ""
        if prompt_path.exists():
            prompt = "\n".join(
                ln
                for ln in prompt_path.read_text().splitlines()
                if not ln.startswith("<!--")
            ).strip()
        description = latest.get("run", {}).get("description", "")

        section = SITE / "content" / "topics" / topic
        section.mkdir(parents=True, exist_ok=True)
        (section / "_index.md").write_text(
            "+++\n"
            f'title = "{topic}"\n'
            'template = "topic.html"\n'
            f"description = {toml_str(description)}\n"
            "[extra]\n"
            f"prompt = {toml_str(prompt)}\n"
            "+++\n"
        )

        site_runs = []
        for m in runs:
            run = m["_run_dir"]
            rel = m["_rel"]
            captures = run_copies_captures(ROOT / rel, topic, run)
            video = run_video(rel, published_all.get(rel, {}))
            model = m.get("model", {})
            runmeta = m.get("run", {})
            delivery = m.get("delivery", {})
            protocol = m.get("protocol", {})
            harness = m.get("harness", {})
            preview = next((c for c in captures if c.endswith("preview.png")), "")
            (section / f"{run}.md").write_text(
                "+++\n"
                f'title = "{run}"\n'
                'template = "run.html"\n'
                f"description = {toml_str(runmeta.get('description', ''))}\n"
                "[extra]\n"
                f"topic = {toml_str(topic)}\n"
                "[extra.model]\n"
                f"name = {toml_str(str(model.get('name', '')))}\n"
                f"id = {toml_str(str(model.get('id', '')))}\n"
                f"source = {toml_str(str(model.get('source', '')))}\n"
                "[extra.harness]\n"
                f"name = {toml_str(str(harness.get('name', '')))}\n"
                f"version = {toml_str(str(harness.get('version', '')))}\n"
                "[extra.protocol]\n"
                f"ref = {toml_str(proto_display(protocol))}\n"
                f"commit = {toml_str(str(protocol.get('commit', '')))}\n"
                f"skill_modified = {str(bool(protocol.get('skill_modified', False))).lower()}\n"
                "[extra.run]\n"
                f"date = {toml_str(str(runmeta.get('date', '')))}\n"
                f"render_rounds = {int(runmeta.get('render_rounds', 0) or 0)}\n"
                f"wall_time = {toml_str(str(runmeta.get('wall_time', '')))}\n"
                "[extra.delivery]\n"
                f"pin = {toml_str(str(delivery.get('pin', '')))}\n"
                f"duration_s = {int(delivery.get('duration_s', 0) or 0)}\n"
                f"duration_min = {round((delivery.get('duration_s', 0) or 0) / 60, 1)}\n"
                f"resolution = {toml_str(str(delivery.get('resolution', '')))}\n"
                f"tests = {int(delivery.get('tests', 0) or 0)}\n"
                f"video = {toml_str(video)}\n"
                f"previews = {json.dumps(captures)}\n"
                "+++\n"
            )
            site_runs.append(
                {
                    "run_dir": run,
                    "topic": topic,
                    "description": runmeta.get("description", ""),
                    "model": model.get("name", ""),
                    "date": str(runmeta.get("date", "")),
                    "render_rounds": runmeta.get("render_rounds", 0),
                    "duration_s": delivery.get("duration_s", 0),
                    "duration_min": round(
                        (delivery.get("duration_s", 0) or 0) / 60, 1
                    ),
                    "pin": delivery.get("pin", ""),
                    "protocol": proto_display(protocol),
                    "preview": preview,
                    "video": video,
                }
            )

        site_topics.append(
            {
                "name": topic,
                "description": description,
                "preview": next(
                    (r["preview"] for r in reversed(site_runs) if r["preview"]), ""
                ),
                "models": sorted({r["model"] for r in site_runs if r["model"]}),
                "run_count": len(site_runs),
                "run_word": "run" if len(site_runs) == 1 else "runs",
                "total_duration_min": round(
                    sum(r["duration_s"] or 0 for r in site_runs) / 60, 1
                ),
                "runs": site_runs,
            }
        )

    (SITE / "data" / "index.json").write_text(
        json.dumps({"topics": site_topics}, ensure_ascii=False, indent=2) + "\n"
    )


def main() -> None:
    topics = load_topics()
    published_all = {m["_rel"]: shadow_urls(m["_rel"]) for runs in topics.values() for m in runs}
    for topic, runs in topics.items():
        write_topic_readme(topic, runs, published_all)
    write_root_readme(topics)
    write_site(topics, published_all)
    total = sum(len(r) for r in topics.values())
    print(f"gen-index: {len(topics)} topics, {total} runs")


if __name__ == "__main__":
    main()
